//! Async resource mutation operations — Phase 5.11.
//!
//! Thin helpers that build a dynamic `kube::Api` from a GVR and execute
//! delete / scale / restart against the live cluster.
//!
//! All functions take an owned `Client` so they can be sent to a background
//! tokio task without lifetime issues.

use kube::api::{ApiResource, DeleteParams, DynamicObject, Patch, PatchParams};
use kube::Api;
use kube::Client;
use serde_json::json;

use crate::client::Gvr;
use crate::watch::factory::resource_to_kind;

// ─── API resource construction ────────────────────────────────────────────────

/// Build a `kube::api::ApiResource` from a k7s GVR.
///
/// Constructs the plural / kind fields directly so the mapping is exact, rather
/// than relying on kube's automatic pluralisation heuristic.
pub fn api_resource_for(gvr: &Gvr) -> ApiResource {
    ApiResource {
        group: gvr.group.clone(),
        version: gvr.version.clone(),
        api_version: if gvr.group.is_empty() {
            gvr.version.clone()
        } else {
            format!("{}/{}", gvr.group, gvr.version)
        },
        kind: resource_to_kind(&gvr.resource),
        plural: gvr.resource.clone(),
    }
}

/// Build a typed `Api<DynamicObject>` from a GVR + optional namespace.
pub fn dynamic_api(client: Client, gvr: &Gvr, namespace: Option<&str>) -> Api<DynamicObject> {
    let ar = api_resource_for(gvr);
    match namespace {
        Some(ns) if !ns.is_empty() => Api::namespaced_with(client, ns, &ar),
        _ => Api::all_with(client, &ar),
    }
}

// ─── Delete ──────────────────────────────────────────────────────────────────

/// Delete a resource.  Returns an empty Ok on success.
pub async fn delete_resource(
    client: Client,
    gvr: &Gvr,
    namespace: Option<&str>,
    name: &str,
) -> anyhow::Result<()> {
    let api = dynamic_api(client, gvr, namespace);
    api.delete(name, &DeleteParams::default()).await?;
    Ok(())
}

// ─── Scale ───────────────────────────────────────────────────────────────────

/// Patch a workload's replica count.
///
/// Works for Deployments, StatefulSets, and ReplicaSets.
pub async fn scale_resource(
    client: Client,
    gvr: &Gvr,
    namespace: &str,
    name: &str,
    replicas: i32,
) -> anyhow::Result<()> {
    let api = dynamic_api(client, gvr, Some(namespace));
    let patch = json!({"spec": {"replicas": replicas}});
    api.patch(name, &PatchParams::apply("k7s"), &Patch::Merge(&patch))
        .await?;
    Ok(())
}

// ─── Restart ─────────────────────────────────────────────────────────────────

/// Trigger a rolling restart by annotating the pod template.
///
/// Equivalent to `kubectl rollout restart`.
/// Works for Deployments, StatefulSets, and DaemonSets.
pub async fn restart_resource(
    client: Client,
    gvr: &Gvr,
    namespace: &str,
    name: &str,
) -> anyhow::Result<()> {
    let api = dynamic_api(client, gvr, Some(namespace));
    let now = chrono::Utc::now().to_rfc3339();
    let patch = json!({
        "spec": {
            "template": {
                "metadata": {
                    "annotations": {
                        "kubectl.kubernetes.io/restartedAt": now
                    }
                }
            }
        }
    });
    api.patch(name, &PatchParams::apply("k7s"), &Patch::Merge(&patch))
        .await?;
    Ok(())
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn api_resource_core_group() {
        use crate::client::gvr::well_known;
        let ar = api_resource_for(&well_known::pods());
        assert_eq!(ar.group, "");
        assert_eq!(ar.version, "v1");
        assert_eq!(ar.plural, "pods");
        assert_eq!(ar.kind, "Pod");
        assert_eq!(ar.api_version, "v1");
    }

    #[test]
    fn api_resource_apps_group() {
        use crate::client::gvr::well_known;
        let ar = api_resource_for(&well_known::deployments());
        assert_eq!(ar.group, "apps");
        assert_eq!(ar.version, "v1");
        assert_eq!(ar.plural, "deployments");
        assert_eq!(ar.kind, "Deployment");
        assert_eq!(ar.api_version, "apps/v1");
    }

    #[test]
    fn api_resource_statefulsets() {
        use crate::client::gvr::well_known;
        let ar = api_resource_for(&well_known::stateful_sets());
        assert_eq!(ar.plural, "statefulsets");
        assert_eq!(ar.kind, "StatefulSet");
    }
}
