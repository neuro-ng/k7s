//! Resource owner navigation — Phase 10.10.
//!
//! Extracts `ownerReferences` from a Kubernetes resource JSON and resolves
//! them into navigation targets so the user can jump to the owning resource
//! (e.g. from a Pod to its ReplicaSet, or from a ReplicaSet to its Deployment).
//!
//! The function `resolve_owner` returns a list of [`OwnerRef`] values.
//! The TUI layer calls `browser_for_owner` to build the `BrowserView` for the
//! chosen owner.
//!
//! # k9s Reference: `internal/view/owner_extender.go`

use serde_json::Value;

use crate::client::Gvr;

// ─── Data types ───────────────────────────────────────────────────────────────

/// A resolved owner reference — enough information to navigate to the owner.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OwnerRef {
    /// Kubernetes `apiVersion` of the owner (e.g. `"apps/v1"`).
    pub api_version: String,
    /// Kubernetes `kind` of the owner (e.g. `"ReplicaSet"`).
    pub kind: String,
    /// Name of the owning resource.
    pub name: String,
    /// Namespace of the owning resource (empty for cluster-scoped owners).
    pub namespace: String,
    /// UID of the owner.
    pub uid: String,
    /// Whether the owner is a controller (as opposed to a weak reference).
    pub controller: bool,
}

impl OwnerRef {
    /// Convert to a display label, e.g. `"ReplicaSet/nginx-abc123"`.
    pub fn display_label(&self) -> String {
        format!("{}/{}", self.kind, self.name)
    }
}

// ─── Extraction ───────────────────────────────────────────────────────────────

/// Extract all `ownerReferences` from a raw Kubernetes resource JSON.
///
/// Returns an empty vec when the resource has no owner references.
pub fn resolve_owners(resource: &Value, namespace: &str) -> Vec<OwnerRef> {
    let refs = match resource
        .pointer("/metadata/ownerReferences")
        .and_then(|v| v.as_array())
    {
        Some(arr) => arr,
        None => return vec![],
    };

    refs.iter()
        .filter_map(|r| parse_owner_ref(r, namespace))
        .collect()
}

fn parse_owner_ref(r: &Value, namespace: &str) -> Option<OwnerRef> {
    let api_version = r["apiVersion"].as_str()?.to_string();
    let kind = r["kind"].as_str()?.to_string();
    let name = r["name"].as_str()?.to_string();
    let uid = r["uid"].as_str().unwrap_or("").to_string();
    let controller = r["controller"].as_bool().unwrap_or(false);

    Some(OwnerRef {
        api_version,
        kind,
        name,
        namespace: namespace.to_string(),
        uid,
        controller,
    })
}

// ─── GVR mapping ─────────────────────────────────────────────────────────────

/// Map a Kubernetes `kind` (case-insensitive) to a [`Gvr`].
///
/// Returns `None` for unknown or unsupported kinds.
pub fn gvr_for_kind(kind: &str, api_version: &str) -> Option<Gvr> {
    // Split "apps/v1" → group="apps", version="v1"
    // Split "v1"      → group="",     version="v1"
    let (group, version) = split_api_version(api_version);

    let resource = kind_to_resource(kind)?;
    Some(Gvr::new(group, version, resource))
}

fn split_api_version(av: &str) -> (&str, &str) {
    match av.split_once('/') {
        Some((g, v)) => (g, v),
        None => ("", av),
    }
}

/// Map a `kind` string to the plural resource name used in GVR.
fn kind_to_resource(kind: &str) -> Option<&'static str> {
    match kind {
        "Pod" => Some("pods"),
        "ReplicaSet" => Some("replicasets"),
        "Deployment" => Some("deployments"),
        "StatefulSet" => Some("statefulsets"),
        "DaemonSet" => Some("daemonsets"),
        "Job" => Some("jobs"),
        "CronJob" => Some("cronjobs"),
        "Service" => Some("services"),
        "Node" => Some("nodes"),
        "Namespace" => Some("namespaces"),
        "ConfigMap" => Some("configmaps"),
        "Secret" => Some("secrets"),
        "ServiceAccount" => Some("serviceaccounts"),
        "Role" => Some("roles"),
        "RoleBinding" => Some("rolebindings"),
        "ClusterRole" => Some("clusterroles"),
        "ClusterRoleBinding" => Some("clusterrolebindings"),
        "PersistentVolume" => Some("persistentvolumes"),
        "PersistentVolumeClaim" => Some("persistentvolumeclaims"),
        "Ingress" => Some("ingresses"),
        "HorizontalPodAutoscaler" => Some("horizontalpodautoscalers"),
        _ => None,
    }
}

// ─── Controller accessor ─────────────────────────────────────────────────────

/// Return only the *controller* owner (the one with `controller: true`),
/// if any.  Most resources have at most one controller owner.
pub fn controller_owner(owners: &[OwnerRef]) -> Option<&OwnerRef> {
    owners.iter().find(|o| o.controller)
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn pod_with_owner() -> Value {
        json!({
            "metadata": {
                "name": "nginx-abc123-xyz",
                "namespace": "default",
                "ownerReferences": [{
                    "apiVersion": "apps/v1",
                    "kind": "ReplicaSet",
                    "name": "nginx-abc123",
                    "uid": "aaa-bbb-ccc",
                    "controller": true,
                    "blockOwnerDeletion": true
                }]
            }
        })
    }

    #[test]
    fn extracts_owner_from_pod() {
        let owners = resolve_owners(&pod_with_owner(), "default");
        assert_eq!(owners.len(), 1);
        assert_eq!(owners[0].kind, "ReplicaSet");
        assert_eq!(owners[0].name, "nginx-abc123");
        assert!(owners[0].controller);
    }

    #[test]
    fn no_owners_returns_empty() {
        let resource = json!({ "metadata": { "name": "my-cm" } });
        assert!(resolve_owners(&resource, "default").is_empty());
    }

    #[test]
    fn controller_owner_finds_controller() {
        let owners = resolve_owners(&pod_with_owner(), "default");
        let ctrl = controller_owner(&owners);
        assert!(ctrl.is_some());
        assert_eq!(ctrl.unwrap().kind, "ReplicaSet");
    }

    #[test]
    fn gvr_for_replica_set() {
        let gvr = gvr_for_kind("ReplicaSet", "apps/v1").unwrap();
        assert_eq!(gvr.group, "apps");
        assert_eq!(gvr.version, "v1");
        assert_eq!(gvr.resource, "replicasets");
    }

    #[test]
    fn gvr_for_core_pod() {
        let gvr = gvr_for_kind("Pod", "v1").unwrap();
        assert_eq!(gvr.group, "");
        assert_eq!(gvr.version, "v1");
        assert_eq!(gvr.resource, "pods");
    }

    #[test]
    fn gvr_for_unknown_kind_is_none() {
        assert!(gvr_for_kind("FancyCRD", "custom.io/v1").is_none());
    }

    #[test]
    fn display_label() {
        let o = OwnerRef {
            api_version: "apps/v1".into(),
            kind: "Deployment".into(),
            name: "my-app".into(),
            namespace: "default".into(),
            uid: "123".into(),
            controller: true,
        };
        assert_eq!(o.display_label(), "Deployment/my-app");
    }

    #[test]
    fn multiple_owners_all_extracted() {
        let resource = json!({
            "metadata": {
                "ownerReferences": [
                    {
                        "apiVersion": "apps/v1",
                        "kind": "ReplicaSet",
                        "name": "rs-1",
                        "uid": "u1",
                        "controller": true
                    },
                    {
                        "apiVersion": "apps/v1",
                        "kind": "Deployment",
                        "name": "deploy-1",
                        "uid": "u2",
                        "controller": false
                    }
                ]
            }
        });
        let owners = resolve_owners(&resource, "default");
        assert_eq!(owners.len(), 2);
        assert_eq!(owners[0].name, "rs-1");
        assert_eq!(owners[1].name, "deploy-1");
    }
}
