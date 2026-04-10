use std::collections::HashMap;
use std::sync::Arc;

use kube::api::{ApiResource, DynamicObject, GroupVersionKind};
use kube::runtime::reflector;
use kube::runtime::reflector::store::Writer;
use kube::runtime::reflector::Store;
use kube::runtime::watcher;
use kube::runtime::WatchStreamExt;
use kube::{Api, Client};
use tokio::sync::RwLock;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

use crate::client::Gvr;

/// A running informer for one (GVR, namespace) pair.
///
/// Holds a read-only view of the local cache and the task driving it.
pub struct WatchHandle {
    /// Read-only access to the locally cached objects.
    pub store: Store<DynamicObject>,
    /// Background task driving the reflector. Abort via the factory's shutdown token.
    _task: JoinHandle<()>,
}

/// Unique key for a running watcher.
#[derive(Hash, Eq, PartialEq, Clone, Debug)]
struct WatchKey {
    gvr: String,
    namespace: Option<String>,
}

/// Manages a collection of running Kubernetes resource informers.
///
/// # Design
///
/// One `WatcherFactory` per connected cluster. Callers call `ensure()` to
/// start watching a resource type (idempotent). The resulting `Store` is
/// updated in the background by a dedicated tokio task.
///
/// Shutdown: drop the factory or call `shutdown()`. All background tasks are
/// cancelled via a `CancellationToken`.
pub struct WatcherFactory {
    client: Client,
    handles: Arc<RwLock<HashMap<WatchKey, WatchHandle>>>,
    shutdown: CancellationToken,
}

impl WatcherFactory {
    pub fn new(client: Client) -> Self {
        Self {
            client,
            handles: Arc::new(RwLock::new(HashMap::new())),
            shutdown: CancellationToken::new(),
        }
    }

    /// Ensure a watcher is running for this GVR + namespace.
    ///
    /// Idempotent: if a watcher is already running for this key, returns the
    /// existing store. Creates a new reflector task otherwise.
    pub async fn ensure(&self, gvr: &Gvr, namespace: Option<&str>) -> Store<DynamicObject> {
        let key = WatchKey {
            gvr: gvr.to_string(),
            namespace: namespace.map(str::to_owned),
        };

        // Fast path: already watching.
        {
            let handles = self.handles.read().await;
            if let Some(h) = handles.get(&key) {
                return h.store.clone();
            }
        }

        // Slow path: start a new informer.
        let mut handles = self.handles.write().await;
        // Double-check after acquiring write lock.
        if let Some(h) = handles.get(&key) {
            return h.store.clone();
        }

        let ar = gvr_to_api_resource(gvr);
        let api: Api<DynamicObject> = match &key.namespace {
            Some(ns) => Api::namespaced_with(self.client.clone(), ns, &ar),
            None => Api::all_with(self.client.clone(), &ar),
        };

        // DynamicObject's DynamicType is ApiResource (not Default), so we
        // can't use reflector::store() — construct Writer manually instead.
        let writer = Writer::<DynamicObject>::new(ar.clone());
        let store = writer.as_reader();
        let shutdown = self.shutdown.clone();
        let gvr_str = gvr.to_string();

        let task = tokio::spawn(async move {
            use futures::StreamExt;

            let stream = watcher(api, watcher::Config::default()).reflect(writer);
            tokio::pin!(stream);

            loop {
                tokio::select! {
                    _ = shutdown.cancelled() => {
                        tracing::debug!(gvr = %gvr_str, "watcher shutting down");
                        break;
                    }
                    event = stream.next() => {
                        match event {
                            None => {
                                tracing::warn!(gvr = %gvr_str, "watcher stream ended");
                                break;
                            }
                            Some(Err(e)) => {
                                tracing::warn!(gvr = %gvr_str, error = %e, "watcher error");
                                // kube-runtime watcher auto-reconnects; just continue.
                            }
                            Some(Ok(_)) => {} // store is updated by the reflector
                        }
                    }
                }
            }
        });

        let handle = WatchHandle {
            store: store.clone(),
            _task: task,
        };
        handles.insert(key, handle);
        store
    }

    /// Return the current store for a GVR + namespace, if a watcher is running.
    pub async fn store(&self, gvr: &Gvr, namespace: Option<&str>) -> Option<Store<DynamicObject>> {
        let key = WatchKey {
            gvr: gvr.to_string(),
            namespace: namespace.map(str::to_owned),
        };
        self.handles.read().await.get(&key).map(|h| h.store.clone())
    }

    /// Cancel all running watchers.
    pub fn shutdown(&self) {
        self.shutdown.cancel();
    }
}

impl Drop for WatcherFactory {
    fn drop(&mut self) {
        self.shutdown();
    }
}

/// Convert a k7s `Gvr` to kube's `ApiResource` for dynamic object access.
fn gvr_to_api_resource(gvr: &Gvr) -> ApiResource {
    let gvk = GroupVersionKind {
        group: gvr.group.clone(),
        version: gvr.version.clone(),
        kind: resource_to_kind(&gvr.resource),
    };
    ApiResource::from_gvk(&gvk)
}

/// Derive the PascalCase singular kind from a lowercase plural resource name.
///
/// Covers the common Kubernetes naming conventions. Special cases are listed
/// explicitly; the fallback strips a trailing `s`.
///
/// `pub(crate)` so `dao::generic` can reuse this without duplicating the mapping.
pub(crate) fn resource_to_kind(resource: &str) -> String {
    // Explicit overrides for irregular plurals.
    match resource {
        "endpoints" => return "Endpoints".to_owned(),
        "networkpolicies" => return "NetworkPolicy".to_owned(),
        "ingresses" => return "Ingress".to_owned(),
        "clusterroles" => return "ClusterRole".to_owned(),
        "clusterrolebindings" => return "ClusterRoleBinding".to_owned(),
        "rolebindings" => return "RoleBinding".to_owned(),
        "customresourcedefinitions" => return "CustomResourceDefinition".to_owned(),
        "persistentvolumes" => return "PersistentVolume".to_owned(),
        "persistentvolumeclaims" => return "PersistentVolumeClaim".to_owned(),
        "serviceaccounts" => return "ServiceAccount".to_owned(),
        "replicasets" => return "ReplicaSet".to_owned(),
        "statefulsets" => return "StatefulSet".to_owned(),
        "daemonsets" => return "DaemonSet".to_owned(),
        "deployments" => return "Deployment".to_owned(),
        "cronjobs" => return "CronJob".to_owned(),
        _ => {}
    }

    // Generic rule: strip trailing 's', then capitalise.
    let singular = if resource.ends_with('s') && !resource.ends_with("ss") {
        &resource[..resource.len() - 1]
    } else {
        resource
    };

    let mut chars = singular.chars();
    match chars.next() {
        None => String::new(),
        Some(c) => c.to_uppercase().collect::<String>() + chars.as_str(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resource_to_kind_standard() {
        assert_eq!(resource_to_kind("pods"), "Pod");
        assert_eq!(resource_to_kind("nodes"), "Node");
        assert_eq!(resource_to_kind("services"), "Service");
        assert_eq!(resource_to_kind("namespaces"), "Namespace");
        assert_eq!(resource_to_kind("secrets"), "Secret");
        assert_eq!(resource_to_kind("jobs"), "Job");
    }

    #[test]
    fn resource_to_kind_overrides() {
        assert_eq!(resource_to_kind("endpoints"), "Endpoints");
        assert_eq!(resource_to_kind("statefulsets"), "StatefulSet");
        assert_eq!(resource_to_kind("daemonsets"), "DaemonSet");
        assert_eq!(resource_to_kind("cronjobs"), "CronJob");
        assert_eq!(
            resource_to_kind("persistentvolumeclaims"),
            "PersistentVolumeClaim"
        );
        assert_eq!(
            resource_to_kind("customresourcedefinitions"),
            "CustomResourceDefinition"
        );
    }
}
