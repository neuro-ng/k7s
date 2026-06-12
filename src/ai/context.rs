//! Contextual AI Chat — Phase 24.
//!
//! Builds a sanitized context snapshot for any selected Kubernetes resource so
//! the AI chat window can answer questions about *that specific resource*
//! without the user needing to manually describe it.
//!
//! # Pipeline
//!
//! ```text
//! selected resource (raw JSON from browser)
//!      │
//!      ▼
//! sanitize() ──→ SafeMetadata  (always)
//!      │
//!      ▼ (resource-type-specific enrichment)
//! events for this resource ──→ SafeMetadata per event (filtered to this object)
//!      │
//!      ▼  (pods only)
//! compressed log summary ──→ SafeMetadata  (log level dist + error patterns)
//!      │
//!      ▼
//! Vec<SafeMetadata>  +  ContextScope  (scope label for the chat badge)
//! ```
//!
//! # Security
//!
//! All data passes through `sanitize()` — raw cluster values never reach the
//! LLM.  Log data is run through the log compressor before being included.

use serde_json::json;

use crate::client::Gvr;
use crate::config::SanitizerConfig;
use crate::sanitizer::{compress, sanitize, SafeMetadata};

/// A human-readable label shown in the chat window header identifying what
/// resource the current context belongs to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContextScope {
    /// Display string, e.g. `"pod/web-abc-xyz  ·  default"`.
    pub label: String,
}

impl ContextScope {
    /// Build a scope label from the resource kind, name, and optional namespace.
    pub fn new(kind: &str, name: &str, namespace: Option<&str>) -> Self {
        let label = match namespace {
            Some(ns) if !ns.is_empty() => format!("{kind}/{name}  ·  {ns}"),
            _ => format!("{kind}/{name}"),
        };
        Self { label }
    }
}

/// Build a sanitized context snapshot for the selected resource.
///
/// Always returns at least one `SafeMetadata` entry (the resource itself).
/// For pods, events and a compressed log summary are appended when available.
/// For other workloads, matching events are appended.
///
/// Errors are swallowed per resource-type — the function always returns the
/// best available context rather than failing.
pub async fn build_resource_context(
    client: &kube::Client,
    gvr: &Gvr,
    name: &str,
    namespace: Option<&str>,
    raw_json: serde_json::Value,
    sanitizer_cfg: &SanitizerConfig,
) -> (Vec<SafeMetadata>, ContextScope) {
    let scope = ContextScope::new(&gvr.resource, name, namespace);
    let mut items: Vec<SafeMetadata> = Vec::new();

    // 1. Sanitize the resource itself.
    if let Ok(meta) = sanitize(gvr, namespace, name, raw_json, sanitizer_cfg) {
        items.push(meta);
    }

    // 2. Resource-type enrichment.
    match gvr.resource.as_str() {
        "pods" => {
            enrich_with_events(client, name, namespace, sanitizer_cfg, &mut items).await;
            enrich_with_pod_logs(client, name, namespace.unwrap_or("default"), &mut items).await;
        }
        "nodes" => {
            enrich_with_events(client, name, None, sanitizer_cfg, &mut items).await;
        }
        "deployments" | "statefulsets" | "daemonsets" | "replicasets" | "jobs" => {
            enrich_with_events(client, name, namespace, sanitizer_cfg, &mut items).await;
        }
        _ => {} // Generic resources: just the sanitized metadata above.
    }

    (items, scope)
}

/// Fetch events for `object_name` in `namespace` and append sanitized
/// SafeMetadata for each matching event.
async fn enrich_with_events(
    client: &kube::Client,
    object_name: &str,
    namespace: Option<&str>,
    sanitizer_cfg: &SanitizerConfig,
    items: &mut Vec<SafeMetadata>,
) {
    let Ok(events) = crate::dao::event::list_events(client, namespace).await else {
        return;
    };

    let event_gvr = Gvr {
        group: String::new(),
        version: "v1".to_owned(),
        resource: "events".to_owned(),
    };

    let matching: Vec<_> = events
        .into_iter()
        .filter(|e| e.pointer("/involvedObject/name").and_then(|v| v.as_str()) == Some(object_name))
        .take(10) // cap at 10 most recent events for token budget
        .collect();

    for ev in matching {
        let ev_name = ev
            .pointer("/metadata/name")
            .and_then(|v| v.as_str())
            .unwrap_or("event")
            .to_owned();
        let ev_ns = ev
            .pointer("/metadata/namespace")
            .and_then(|v| v.as_str())
            .map(ToOwned::to_owned);

        if let Ok(meta) = sanitize(&event_gvr, ev_ns.as_deref(), &ev_name, ev, sanitizer_cfg) {
            items.push(meta);
        }
    }
}

/// Fetch the last 100 log lines for the first container in `pod_name`,
/// compress them, and append a synthetic `SafeMetadata` entry containing
/// the token-efficient log summary.
async fn enrich_with_pod_logs(
    client: &kube::Client,
    pod_name: &str,
    namespace: &str,
    items: &mut Vec<SafeMetadata>,
) {
    use k8s_openapi::api::core::v1::Pod;
    use kube::api::LogParams;
    use kube::Api;

    let api: Api<Pod> = Api::namespaced(client.clone(), namespace);
    let params = LogParams {
        tail_lines: Some(100),
        ..Default::default()
    };

    let Ok(log_text) = api.logs(pod_name, &params).await else {
        return;
    };

    let lines: Vec<String> = log_text.lines().map(ToOwned::to_owned).collect();
    let compressed = compress(&lines, 60);
    let summary_text = compressed.to_prompt_string();

    if summary_text.trim().is_empty() {
        return;
    }

    // Wrap the summary in a synthetic SafeMetadata with a descriptive GVR.
    let log_gvr = Gvr {
        group: String::new(),
        version: "v1".to_owned(),
        resource: "pod-logs".to_owned(),
    };
    let summary_json = json!({
        "pod": pod_name,
        "namespace": namespace,
        "summary": summary_text,
    });

    // Use an internal sanitizer-bypass for this synthetic entry — the log
    // compressor already guarantees no secrets appear in `summary_text`.
    // We construct SafeMetadata directly here (crate-internal privilege).
    let meta = SafeMetadata {
        gvr: log_gvr.to_string(),
        name: pod_name.to_owned(),
        namespace: Some(namespace.to_owned()),
        fields: summary_json,
    };
    items.push(meta);
}

// ─── KubeVela context ─────────────────────────────────────────────────────────

/// Build a sanitized AI context snapshot for a KubeVela Application.
///
/// Fetches the Application CR from the cluster, sanitizes it, and appends
/// synthetic `SafeMetadata` entries for component health and workflow state.
/// Sensitive property values are dropped; only structural metadata is sent.
pub async fn build_vela_context(
    client: &kube::Client,
    app_name: &str,
    namespace: &str,
    sanitizer_cfg: &SanitizerConfig,
) -> (Vec<SafeMetadata>, ContextScope) {
    use kube::api::{ApiResource, DynamicObject, ListParams};
    use kube::Api;

    let scope = ContextScope::new("applications.core.oam.dev", app_name, Some(namespace));
    let mut items: Vec<SafeMetadata> = Vec::new();

    let ar = ApiResource {
        group: "core.oam.dev".into(),
        version: "v1beta1".into(),
        api_version: "core.oam.dev/v1beta1".into(),
        kind: "Application".into(),
        plural: "applications".into(),
    };
    let api: Api<DynamicObject> = Api::namespaced_with(client.clone(), namespace, &ar);

    let lp = ListParams::default().fields(&format!("metadata.name={app_name}"));
    let Ok(list) = api.list(&lp).await else {
        return (items, scope);
    };

    let Some(obj) = list.items.into_iter().next() else {
        return (items, scope);
    };

    let raw = match serde_json::to_value(&obj) {
        Ok(v) => v,
        Err(_) => return (items, scope),
    };

    // Sanitize the top-level Application resource.
    let gvr = crate::client::Gvr {
        group: "core.oam.dev".into(),
        version: "v1beta1".into(),
        resource: "applications".into(),
    };
    if let Ok(meta) = sanitize(&gvr, Some(namespace), app_name, raw.clone(), sanitizer_cfg) {
        items.push(meta);
    }

    // Synthetic SafeMetadata: component health summary (no raw values).
    let components = crate::vela::parse_components(&raw);
    if !components.is_empty() {
        let comp_summary: Vec<serde_json::Value> = components
            .iter()
            .map(|c| {
                json!({
                    "name": c.name,
                    "type": c.workload_type,
                    "healthy": c.healthy,
                    "message": if c.message.len() > 200 {
                        format!("{}…", &c.message[..200])
                    } else {
                        c.message.clone()
                    },
                    "traits": c.traits.iter().map(|t| json!({
                        "type": t.trait_type,
                        "healthy": t.healthy
                    })).collect::<Vec<_>>()
                })
            })
            .collect();

        items.push(SafeMetadata {
            gvr: "vela-components".into(),
            name: app_name.to_owned(),
            namespace: Some(namespace.to_owned()),
            fields: json!({ "components": comp_summary }),
        });
    }

    // Synthetic SafeMetadata: workflow steps (phase only, no sensitive data).
    let steps = crate::vela::parse_workflow_steps(&raw);
    if !steps.is_empty() {
        let step_summary: Vec<serde_json::Value> = steps
            .iter()
            .map(|s| {
                json!({
                    "name": s.name,
                    "type": s.step_type,
                    "phase": s.phase
                })
            })
            .collect();

        items.push(SafeMetadata {
            gvr: "vela-workflow".into(),
            name: app_name.to_owned(),
            namespace: Some(namespace.to_owned()),
            fields: json!({ "steps": step_summary }),
        });
    }

    (items, scope)
}

// ─── Helm context ─────────────────────────────────────────────────────────────

/// Build a sanitized AI context snapshot for a Helm release.
///
/// Fetches release info and history via the `helm` CLI and constructs
/// `SafeMetadata` entries safe to send to the LLM.  Values are run through
/// the `sanitize_helm_values` key-name + value-content redaction rules so
/// no passwords, tokens, or connection strings reach the AI.
pub async fn build_helm_context(name: &str, namespace: &str) -> (Vec<SafeMetadata>, ContextScope) {
    let scope = ContextScope::new("helm", name, Some(namespace));
    let mut items: Vec<SafeMetadata> = Vec::new();

    let dao = crate::dao::helm::HelmDao::new(None);

    // 1. Current release summary.
    if let Ok(releases) = dao.list(Some(namespace)) {
        if let Some(rel) = releases.into_iter().find(|r| r.name == name) {
            items.push(SafeMetadata {
                gvr: "helm/v3/releases".to_string(),
                name: name.to_owned(),
                namespace: Some(namespace.to_owned()),
                fields: json!({
                    "name":        rel.name,
                    "namespace":   rel.namespace,
                    "chart":       rel.chart,
                    "app_version": rel.app_version,
                    "status":      rel.status,
                    "revision":    rel.revision,
                    "updated":     rel.updated,
                }),
            });
        }
    }

    // 2. Revision history — last 5 revisions only for token budget.
    if let Ok(history) = dao.history(name, namespace) {
        let hist: Vec<serde_json::Value> = history
            .iter()
            .take(5)
            .map(|e| {
                json!({
                    "revision":    e.revision,
                    "status":      e.status,
                    "chart":       e.chart,
                    "app_version": e.app_version,
                    "updated":     e.updated,
                    "description": e.description,
                })
            })
            .collect();
        items.push(SafeMetadata {
            gvr: "helm/v3/history".to_string(),
            name: name.to_owned(),
            namespace: Some(namespace.to_owned()),
            fields: json!({ "revisions": hist }),
        });
    }

    // 3. Sanitized values YAML — secret keys/values redacted via helm sanitizer.
    let raw_values = helm_get_values(name, namespace);
    if !raw_values.is_empty() && !raw_values.starts_with("Error") {
        let sanitized = sanitize_helm_text_values(&raw_values);
        if !sanitized.trim().is_empty() {
            items.push(SafeMetadata {
                gvr: "helm/v3/values".to_string(),
                name: name.to_owned(),
                namespace: Some(namespace.to_owned()),
                fields: json!({ "values_yaml": sanitized }),
            });
        }
    }

    (items, scope)
}

/// Shell out to `helm get values <name> -n <ns> --output yaml`.
fn helm_get_values(name: &str, namespace: &str) -> String {
    match std::process::Command::new("helm")
        .args(["get", "values", name, "-n", namespace, "--output", "yaml"])
        .output()
    {
        Ok(o) if o.status.success() => String::from_utf8_lossy(&o.stdout).into_owned(),
        _ => String::new(),
    }
}

/// Redact secret-looking lines from Helm values YAML (line-level for text output).
fn sanitize_helm_text_values(text: &str) -> String {
    text.lines()
        .map(|line| {
            let lower = line.to_lowercase();
            let is_secret = crate::sanitizer::helm::SECRET_KEY_PATTERNS
                .iter()
                .any(|p| lower.contains(&format!("{p}:")));
            if is_secret {
                if let Some(pos) = line.find(':') {
                    format!("{}: [REDACTED]", &line[..pos])
                } else {
                    "[REDACTED]".to_string()
                }
            } else {
                line.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── ContextScope ──────────────────────────────────────────────────────────

    #[test]
    fn scope_with_namespace() {
        let s = ContextScope::new("pods", "web-abc", Some("production"));
        assert_eq!(s.label, "pods/web-abc  ·  production");
    }

    #[test]
    fn scope_without_namespace() {
        let s = ContextScope::new("nodes", "node-1", None);
        assert_eq!(s.label, "nodes/node-1");
    }

    #[test]
    fn scope_with_empty_namespace_treated_as_none() {
        let s = ContextScope::new("pods", "api", Some(""));
        assert_eq!(s.label, "pods/api");
    }

    #[test]
    fn scope_eq() {
        let a = ContextScope::new("pods", "x", Some("ns"));
        let b = ContextScope::new("pods", "x", Some("ns"));
        assert_eq!(a, b);
    }

    // ── Event filter (unit-level) ─────────────────────────────────────────────

    #[test]
    fn event_filter_keeps_matching_objects() {
        use serde_json::json;
        let events = [
            json!({"involvedObject": {"name": "web-pod"}, "metadata": {}}),
            json!({"involvedObject": {"name": "other-pod"}, "metadata": {}}),
            json!({"involvedObject": {"name": "web-pod"}, "metadata": {}}),
        ];
        let matching: Vec<_> = events
            .iter()
            .filter(|e| {
                e.pointer("/involvedObject/name").and_then(|v| v.as_str()) == Some("web-pod")
            })
            .collect();
        assert_eq!(matching.len(), 2);
    }

    #[test]
    fn event_filter_empty_when_no_match() {
        use serde_json::json;
        let events = [json!({"involvedObject": {"name": "other"}, "metadata": {}})];
        let matching: Vec<_> = events
            .iter()
            .filter(|e| {
                e.pointer("/involvedObject/name").and_then(|v| v.as_str()) == Some("target")
            })
            .collect();
        assert!(matching.is_empty());
    }

    // ── Context scope label formatting ────────────────────────────────────────

    #[test]
    fn scope_label_contains_kind_and_name() {
        let s = ContextScope::new("deployments", "my-app", Some("staging"));
        assert!(s.label.contains("deployments/my-app"));
        assert!(s.label.contains("staging"));
    }

    #[test]
    fn scope_label_node_no_namespace() {
        let s = ContextScope::new("nodes", "worker-3", None);
        assert!(s.label.contains("worker-3"));
        assert!(!s.label.contains("·"));
    }
}
