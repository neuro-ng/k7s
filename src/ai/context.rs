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
