//! KubeVela kube-API client — Phase 36.
//!
//! All functions query the Kubernetes API directly using `kube::api::Api<DynamicObject>`.
//! No `vela` binary is required for reads.

use kube::api::{ApiResource, DynamicObject, ListParams};
use kube::Api;

use super::{VelaApplication, VelaDefinition, VelaRevision};

// ─── OAM API constants ────────────────────────────────────────────────────────

const GROUP: &str = "core.oam.dev";
const VERSION: &str = "v1beta1";

fn app_resource() -> ApiResource {
    ApiResource {
        group: GROUP.into(),
        version: VERSION.into(),
        api_version: format!("{GROUP}/{VERSION}"),
        kind: "Application".into(),
        plural: "applications".into(),
    }
}

fn revision_resource() -> ApiResource {
    ApiResource {
        group: GROUP.into(),
        version: VERSION.into(),
        api_version: format!("{GROUP}/{VERSION}"),
        kind: "ApplicationRevision".into(),
        plural: "applicationrevisions".into(),
    }
}

fn def_resource(plural: &str, kind: &str) -> ApiResource {
    ApiResource {
        group: GROUP.into(),
        version: VERSION.into(),
        api_version: format!("{GROUP}/{VERSION}"),
        kind: kind.into(),
        plural: plural.into(),
    }
}

// ─── Public API ───────────────────────────────────────────────────────────────

/// Check whether KubeVela is installed by listing `applications.core.oam.dev`.
///
/// Returns `true` if the CRD exists and the API server responds.
/// Returns `false` on any error (CRD absent, permissions, etc.).
pub async fn is_installed(client: &kube::Client) -> bool {
    let ar = app_resource();
    let api: Api<DynamicObject> = Api::all_with(client.clone(), &ar);
    api.list(&ListParams::default().limit(1)).await.is_ok()
}

/// List all Application CRs, optionally scoped to a namespace.
///
/// Silently returns an empty `Vec` on errors (callers show "not installed" hint).
pub async fn list_apps(
    client: &kube::Client,
    namespace: Option<&str>,
) -> Vec<VelaApplication> {
    let ar = app_resource();
    let api: Api<DynamicObject> = match namespace {
        Some(ns) if !ns.is_empty() => Api::namespaced_with(client.clone(), ns, &ar),
        _ => Api::all_with(client.clone(), &ar),
    };

    let Ok(list) = api.list(&ListParams::default()).await else {
        return Vec::new();
    };

    list.items
        .into_iter()
        .filter_map(parse_app_from_dynamic)
        .collect()
}

/// List all `ApplicationRevision` CRs for a given application.
pub async fn list_revisions(
    client: &kube::Client,
    app_name: &str,
    namespace: &str,
) -> Vec<VelaRevision> {
    let ar = revision_resource();
    let api: Api<DynamicObject> = if namespace.is_empty() {
        Api::all_with(client.clone(), &ar)
    } else {
        Api::namespaced_with(client.clone(), namespace, &ar)
    };

    // Label selector: filter to revisions belonging to this application.
    let lp = ListParams::default()
        .labels(&format!("app.oam.dev/name={app_name}"));
    let Ok(list) = api.list(&lp).await else {
        return Vec::new();
    };

    let mut revisions: Vec<VelaRevision> = list
        .items
        .into_iter()
        .map(parse_revision_from_dynamic)
        .collect();

    revisions.sort_by_key(|r| r.revision);
    revisions
}

/// List all definitions of the given type.
///
/// `def_type` should be one of `"component"`, `"trait"`, `"workflowstep"`, `"policy"`.
pub async fn list_definitions(
    client: &kube::Client,
    def_type: &str,
) -> Vec<VelaDefinition> {
    let (plural, kind, label) = match def_type.to_lowercase().as_str() {
        "trait" => ("traitdefinitions", "TraitDefinition", "Trait"),
        "workflowstep" | "workflow" => (
            "workflowstepdefinitions",
            "WorkflowStepDefinition",
            "WorkflowStep",
        ),
        "policy" => ("policydefinitions", "PolicyDefinition", "Policy"),
        _ => ("componentdefinitions", "ComponentDefinition", "Component"),
    };
    let ar = def_resource(plural, kind);
    let api: Api<DynamicObject> = Api::all_with(client.clone(), &ar);

    let Ok(list) = api.list(&ListParams::default()).await else {
        return Vec::new();
    };

    list.items
        .into_iter()
        .map(|obj| {
            let name = obj.metadata.name.clone().unwrap_or_default();
            let description = obj
                .metadata
                .annotations
                .as_ref()
                .and_then(|a| a.get("definition.oam.dev/description"))
                .cloned()
                .unwrap_or_default();
            VelaDefinition {
                name,
                def_type: label.into(),
                description,
            }
        })
        .collect()
}

// ─── Parsers ─────────────────────────────────────────────────────────────────

fn parse_app_from_dynamic(obj: DynamicObject) -> Option<VelaApplication> {
    let name = obj.metadata.name.clone()?;
    let namespace = obj.metadata.namespace.clone().unwrap_or_default();

    let raw = serde_json::to_value(&obj).ok()?;

    let status = raw
        .pointer("/data/status/status")
        .or_else(|| raw.pointer("/status/status"))
        .and_then(|v| v.as_str())
        .unwrap_or("unknown")
        .to_string();

    let workflow_status = raw
        .pointer("/data/status/workflow/status")
        .or_else(|| raw.pointer("/status/workflow/status"))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    let component_count = raw
        .pointer("/data/spec/components")
        .or_else(|| raw.pointer("/spec/components"))
        .and_then(|v| v.as_array())
        .map(|a| a.len())
        .unwrap_or(0);

    let age_secs = obj
        .metadata
        .creation_timestamp
        .as_ref()
        .map(|ts| {
            let created = ts.0;
            let now = chrono::Utc::now();
            (now - created).num_seconds().max(0) as u64
        })
        .unwrap_or(0);

    Some(VelaApplication {
        name,
        namespace,
        status,
        component_count,
        workflow_status,
        age_secs,
        raw,
    })
}

fn parse_revision_from_dynamic(obj: DynamicObject) -> VelaRevision {
    let name = obj.metadata.name.clone().unwrap_or_default();

    // Extract revision number from name suffix (e.g. "my-app-v3" → 3)
    // or from label "app.oam.dev/appRevision".
    let revision = obj
        .metadata
        .labels
        .as_ref()
        .and_then(|l| l.get("app.oam.dev/appRevision"))
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or_else(|| {
            name.rsplit_once("-v")
                .and_then(|(_, n)| n.parse::<u64>().ok())
                .unwrap_or(0)
        });

    let deploy_time = obj
        .metadata
        .creation_timestamp
        .as_ref()
        .map(|ts| ts.0.format("%Y-%m-%d %H:%M").to_string())
        .unwrap_or_default();

    let raw = serde_json::to_value(&obj).unwrap_or_default();
    let succeeded = raw
        .pointer("/status/succeeded")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    VelaRevision {
        name,
        revision,
        deploy_time,
        status: if succeeded {
            "succeeded".into()
        } else {
            "unknown".into()
        },
    }
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn app_resource_has_correct_fields() {
        let ar = app_resource();
        assert_eq!(ar.group, "core.oam.dev");
        assert_eq!(ar.version, "v1beta1");
        assert_eq!(ar.plural, "applications");
        assert_eq!(ar.kind, "Application");
    }

    #[test]
    fn revision_resource_plural() {
        let ar = revision_resource();
        assert_eq!(ar.plural, "applicationrevisions");
    }

    #[test]
    fn def_resource_component_defaults() {
        let ar = def_resource("componentdefinitions", "ComponentDefinition");
        assert_eq!(ar.plural, "componentdefinitions");
        assert_eq!(ar.kind, "ComponentDefinition");
    }

    #[test]
    fn revision_number_parsed_from_name() {
        let obj = DynamicObject {
            types: None,
            metadata: kube::api::ObjectMeta {
                name: Some("my-app-v7".into()),
                ..Default::default()
            },
            data: serde_json::Value::Null,
        };
        let rev = parse_revision_from_dynamic(obj);
        assert_eq!(rev.revision, 7);
    }

    #[test]
    fn revision_number_from_label_preferred() {
        let mut labels = std::collections::BTreeMap::new();
        labels.insert(
            "app.oam.dev/appRevision".to_string(),
            "12".to_string(),
        );
        let obj = DynamicObject {
            types: None,
            metadata: kube::api::ObjectMeta {
                name: Some("my-app-v5".into()),
                labels: Some(labels),
                ..Default::default()
            },
            data: serde_json::Value::Null,
        };
        let rev = parse_revision_from_dynamic(obj);
        // Label "12" should override the name-derived "5".
        assert_eq!(rev.revision, 12);
    }
}
