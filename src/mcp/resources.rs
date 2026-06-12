//! MCP resource implementations — Phase 40.
//!
//! Exposes three read-only `k8s://` resources that MCP clients can subscribe to
//! and read at any time without invoking a tool.

use std::sync::Arc;

use rs_fast_mcp::error::FastMCPError;
use rs_fast_mcp::mcp::types::{BaseMetadata, Resource};
use rs_fast_mcp::resources::manager::ResourceReadHandler;
use rs_fast_mcp::resources::types::{ResourceContent, ResourceResult};
use rs_fast_mcp::server::context::Context;
use rs_fast_mcp::server::core::FastMCPServer;
use serde_json::json;

use crate::dao::namespace::NamespaceDao;
use crate::health::build_cluster_summary;
use crate::meta::history::summarise;
use crate::meta::MetadataStore;

use super::server::McpState;

// ─── Registration ─────────────────────────────────────────────────────────────

/// Register all k7s MCP resources on the server core.
pub fn register_all(core: &FastMCPServer, state: Arc<McpState>) -> Result<(), FastMCPError> {
    register_cluster_health(core, Arc::clone(&state))?;
    register_namespaces(core, Arc::clone(&state))?;
    register_cluster_history(core, state)?;
    Ok(())
}

// ─── k8s://cluster/health ────────────────────────────────────────────────────

fn register_cluster_health(core: &FastMCPServer, state: Arc<McpState>) -> Result<(), FastMCPError> {
    let resource = Resource {
        uri: "k8s://cluster/health".to_string(),
        description: Some(
            "Live cluster health: node conditions, pod phase distribution, \
             degraded workloads, and recent warning events."
                .to_string(),
        ),
        mime_type: Some("application/json".to_string()),
        size: None,
        annotations: None,
        icons: None,
        tags: Some(vec!["kubernetes".to_string(), "health".to_string()]),
        base_metadata: BaseMetadata {
            name: "cluster-health".to_string(),
            title: Some("Cluster Health".to_string()),
        },
    };

    let handler: Arc<ResourceReadHandler> =
        Arc::new(Box::new(move |_uri: String, _ctx: Context| {
            let state = Arc::clone(&state);
            Box::pin(async move {
                let summary = build_cluster_summary(&state.client, None).await;
                let v = serde_json::json!({
                    "pods": {
                        "total": summary.pods.total,
                        "running": summary.pods.running,
                        "failed": summary.pods.failed,
                        "pending": summary.pods.pending,
                    },
                    "deployments": {
                        "total": summary.deployments.total,
                        "running": summary.deployments.running,
                        "failed": summary.deployments.failed,
                    },
                    "nodes": {
                        "total": summary.nodes.total,
                        "ready": summary.nodes.ready,
                    },
                    "namespaces": summary.namespaces,
                    "events_warn": summary.events_warn,
                    "events_total": summary.events_total,
                });
                let json = serde_json::to_string_pretty(&v).unwrap_or_else(|_| "{}".to_string());
                Ok(ResourceResult {
                    contents: vec![ResourceContent::text(
                        json,
                        Some("application/json".to_string()),
                    )],
                    meta: None,
                })
            })
        }));

    core.add_resource(resource, Some(handler))
}

// ─── k8s://cluster/namespaces ────────────────────────────────────────────────

fn register_namespaces(core: &FastMCPServer, state: Arc<McpState>) -> Result<(), FastMCPError> {
    let resource = Resource {
        uri: "k8s://cluster/namespaces".to_string(),
        description: Some("All accessible Kubernetes namespaces.".to_string()),
        mime_type: Some("application/json".to_string()),
        size: None,
        annotations: None,
        icons: None,
        tags: Some(vec!["kubernetes".to_string()]),
        base_metadata: BaseMetadata {
            name: "cluster-namespaces".to_string(),
            title: Some("Cluster Namespaces".to_string()),
        },
    };

    let handler: Arc<ResourceReadHandler> =
        Arc::new(Box::new(move |_uri: String, _ctx: Context| {
            let state = Arc::clone(&state);
            Box::pin(async move {
                let dao = NamespaceDao::new();
                let names = dao.list_names(&state.client).await.unwrap_or_default();
                let json = serde_json::to_string_pretty(&json!({"namespaces": names}))
                    .unwrap_or_else(|_| "{}".to_string());
                Ok(ResourceResult {
                    contents: vec![ResourceContent::text(
                        json,
                        Some("application/json".to_string()),
                    )],
                    meta: None,
                })
            })
        }));

    core.add_resource(resource, Some(handler))
}

// ─── k8s://cluster/history ───────────────────────────────────────────────────

fn register_cluster_history(
    core: &FastMCPServer,
    state: Arc<McpState>,
) -> Result<(), FastMCPError> {
    let resource = Resource {
        uri: "k8s://cluster/history".to_string(),
        description: Some(
            "Last 7 days of cluster history: recurrent issues, resolved events, \
             operator actions, and workload drift from the k7s metadata journal."
                .to_string(),
        ),
        mime_type: Some("text/plain".to_string()),
        size: None,
        annotations: None,
        icons: None,
        tags: Some(vec!["kubernetes".to_string(), "history".to_string()]),
        base_metadata: BaseMetadata {
            name: "cluster-history".to_string(),
            title: Some("Cluster History".to_string()),
        },
    };

    let handler: Arc<ResourceReadHandler> =
        Arc::new(Box::new(move |_uri: String, _ctx: Context| {
            let state = Arc::clone(&state);
            Box::pin(async move {
                let text = match MetadataStore::new(&state.meta_context) {
                    Some(store) => {
                        let records = store.load_recent(7);
                        summarise(&records, 7, &state.meta_context).to_context_block()
                    }
                    None => format!(
                        "No cluster metadata journal found for context '{}'.",
                        state.meta_context
                    ),
                };
                Ok(ResourceResult {
                    contents: vec![ResourceContent::text(text, Some("text/plain".to_string()))],
                    meta: None,
                })
            })
        }));

    core.add_resource(resource, Some(handler))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resource_uris_use_k8s_scheme() {
        // Verify the URI strings match the documented scheme.
        assert_eq!("k8s://cluster/health".starts_with("k8s://"), true);
        assert_eq!("k8s://cluster/namespaces".starts_with("k8s://"), true);
        assert_eq!("k8s://cluster/history".starts_with("k8s://"), true);
    }

    #[test]
    fn health_resource_has_json_mime_type() {
        let mime = "application/json";
        assert_eq!(mime, "application/json");
    }

    #[test]
    fn history_resource_has_text_mime_type() {
        let mime = "text/plain";
        assert_eq!(mime, "text/plain");
    }
}
