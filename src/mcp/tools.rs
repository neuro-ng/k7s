//! MCP tool implementations — Phase 40.
//!
//! Each function registers one Kubernetes tool on the MCP server.  All tool
//! responses pass through the sanitizer before being returned to the client.

use std::sync::Arc;

use anyhow::Context as _;
use k8s_openapi::api::apps::v1::Deployment;
use k8s_openapi::api::core::v1::Pod;
use kube::api::LogParams;
use kube::Api;
use rs_fast_mcp::error::FastMCPError;
use rs_fast_mcp::server::core::FastMCPServer;
use rs_fast_mcp::tools::tool::{Tool, ToolResult};
use serde_json::{json, Value};

use crate::client::gvr::well_known;
use crate::client::Gvr;
use crate::dao::deployment::DeploymentDao;
use crate::dao::event::list_events;
use crate::dao::generic::GenericDao;
use crate::dao::namespace::NamespaceDao;
use crate::dao::traits::Accessor;
use crate::health::build_cluster_summary;
use crate::meta::history::summarise;
use crate::meta::MetadataStore;
use crate::metrics::{MetricSample, MetricsClient, MetricsSnapshot};
use crate::sanitizer::{self, log_compressor};

use super::server::{anyhow_to_mcp, json_result, make_handler, text_result, McpState};

// ─── Registration ─────────────────────────────────────────────────────────────

/// Register all tools on the MCP server core.
///
/// Read-only tools are always registered.  Mutating tools are only registered
/// when `state.allow_mutations` is `true`.
pub fn register_all(core: &FastMCPServer, state: Arc<McpState>) -> Result<(), FastMCPError> {
    core.add_tool(list_resources(Arc::clone(&state)))?;
    core.add_tool(get_pod_logs(Arc::clone(&state)))?;
    core.add_tool(describe_resource(Arc::clone(&state)))?;
    core.add_tool(get_events(Arc::clone(&state)))?;
    core.add_tool(cluster_health(Arc::clone(&state)))?;
    core.add_tool(get_metrics(Arc::clone(&state)))?;
    core.add_tool(list_namespaces(Arc::clone(&state)))?;
    core.add_tool(get_cluster_history(Arc::clone(&state)))?;

    if state.allow_mutations {
        core.add_tool(scale_deployment(Arc::clone(&state)))?;
        core.add_tool(rollout_restart(Arc::clone(&state)))?;
    }

    Ok(())
}

// ─── Tool definitions ─────────────────────────────────────────────────────────

fn list_resources(state: Arc<McpState>) -> Tool {
    Tool::new(
        "k8s_list_resources",
        "List Kubernetes resources of a given type in a namespace. \
         Returns sanitized metadata only — no secrets or credentials.",
    )
    .add_parameter(
        "resource",
        "string",
        "Resource type, e.g. pods, deployments, services",
    )
    .add_parameter(
        "namespace",
        "string",
        "Namespace to query (empty string = all namespaces)",
    )
    .with_handler(make_handler(state, |s, args| async move {
        let resource = str_arg(&args, "resource").unwrap_or("pods");
        let namespace = str_arg(&args, "namespace").filter(|n| !n.is_empty());

        let gvr = gvr_from_resource(resource);
        let dao = GenericDao::new(gvr.clone());
        let items = dao
            .list(&s.client, namespace)
            .await
            .map_err(anyhow_to_mcp)?;

        let mut sanitized = Vec::new();
        for item in items {
            let raw = item.data;
            let name = raw
                .get("metadata")
                .and_then(|m: &Value| m.get("name"))
                .and_then(|n: &Value| n.as_str())
                .unwrap_or("<unknown>")
                .to_string();
            let ns = raw
                .get("metadata")
                .and_then(|m: &Value| m.get("namespace"))
                .and_then(|n: &Value| n.as_str())
                .map(|s: &str| s.to_string());

            if let Ok(safe) = sanitizer::sanitize(&gvr, ns.as_deref(), &name, raw, &s.sanitizer_cfg)
            {
                sanitized.push(safe.fields);
            }
        }

        let count = sanitized.len();
        Ok(json_result(json!({"items": sanitized, "count": count})))
    }))
}

fn get_pod_logs(state: Arc<McpState>) -> Tool {
    Tool::new(
        "k8s_get_pod_logs",
        "Fetch logs for a pod, compressed and deduplicated for token efficiency. \
         Raw log bytes are never returned — only pattern-summarised output.",
    )
    .add_parameter("pod", "string", "Pod name")
    .add_parameter("namespace", "string", "Namespace the pod lives in")
    .add_parameter(
        "container",
        "string",
        "Container name (optional for single-container pods)",
    )
    .add_parameter("tail", "integer", "Number of lines to tail (default 200)")
    .with_handler(make_handler(state, |s, args| async move {
        let pod = require_str(&args, "pod")?;
        let namespace = require_str(&args, "namespace")?;
        let container = str_arg(&args, "container");
        let tail = args.get("tail").and_then(|v| v.as_i64()).unwrap_or(200);

        let api: Api<Pod> = Api::namespaced(s.client.clone(), namespace);
        let raw_logs = api
            .logs(
                pod,
                &LogParams {
                    container: container.map(str::to_string),
                    tail_lines: Some(tail),
                    ..Default::default()
                },
            )
            .await
            .map_err(|e| FastMCPError::new(e.to_string()))?;

        let lines: Vec<String> = raw_logs.lines().map(str::to_owned).collect();
        let compressed = log_compressor::compress(&lines, 500);
        let summary = format!(
            "Log summary for {pod} ({namespace}):\n\
             Lines processed: {}\n\
             Unique patterns: {}\n\
             Level distribution: {}\n\n{}",
            compressed.stats.total_lines,
            compressed.stats.unique_patterns,
            compressed.stats.level_distribution(),
            compressed.to_prompt_string(),
        );

        Ok(text_result(summary))
    }))
}

fn describe_resource(state: Arc<McpState>) -> Tool {
    Tool::new(
        "k8s_describe_resource",
        "Describe a Kubernetes resource (equivalent to kubectl describe). \
         Output is redacted to remove any secrets or credentials.",
    )
    .add_parameter(
        "resource",
        "string",
        "Resource type, e.g. pod, deployment, node",
    )
    .add_parameter("name", "string", "Resource name")
    .add_parameter(
        "namespace",
        "string",
        "Namespace (leave empty for cluster-scoped resources)",
    )
    .with_handler(make_handler(state, |s, args| async move {
        let resource = require_str(&args, "resource")?;
        let name = require_str(&args, "name")?;
        let namespace = str_arg(&args, "namespace");

        let mut cmd = tokio::process::Command::new("kubectl");
        cmd.args(["describe", resource, name]);
        if let Some(ns) = namespace {
            cmd.args(["-n", ns]);
        }

        let output = cmd
            .output()
            .await
            .map_err(|e| FastMCPError::new(format!("kubectl not found: {e}")))?;

        let raw = String::from_utf8_lossy(&output.stdout).to_string();
        let redactor = crate::sanitizer::Redactor::new(&s.sanitizer_cfg.custom_patterns)
            .map_err(|e| FastMCPError::new(e.to_string()))?;
        let safe = redactor.redact_str(&raw);

        Ok(text_result(safe))
    }))
}

fn get_events(state: Arc<McpState>) -> Tool {
    Tool::new(
        "k8s_get_events",
        "List recent Kubernetes events for a namespace or specific resource. \
         Capped at 20 entries; annotation values are redacted.",
    )
    .add_parameter(
        "namespace",
        "string",
        "Namespace to query (empty = all namespaces)",
    )
    .add_parameter(
        "resource_name",
        "string",
        "Optional: filter events for this resource name",
    )
    .with_handler(make_handler(state, |s, args| async move {
        let namespace = str_arg(&args, "namespace").filter(|n| !n.is_empty());
        let resource_name = str_arg(&args, "resource_name");

        let all_events = list_events(&s.client, namespace)
            .await
            .map_err(anyhow_to_mcp)?;

        let filtered: Vec<Value> = all_events
            .into_iter()
            .filter(|e| {
                resource_name.map_or(true, |rn| {
                    e.get("involvedObject")
                        .and_then(|o| o.get("name"))
                        .and_then(|n| n.as_str())
                        == Some(rn)
                })
            })
            .take(20)
            .map(sanitize_event)
            .collect();

        Ok(json_result(
            json!({"events": filtered, "count": filtered.len()}),
        ))
    }))
}

fn cluster_health(state: Arc<McpState>) -> Tool {
    Tool::new(
        "k8s_cluster_health",
        "Return a structured cluster health summary: node conditions, \
         pod phase distribution, degraded workloads, and recent warning events.",
    )
    .with_handler(make_handler(state, |s, _args| async move {
        let summary = build_cluster_summary(&s.client, None).await;
        let v = json!({
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
        Ok(json_result(v))
    }))
}

fn get_metrics(state: Arc<McpState>) -> Tool {
    Tool::new(
        "k8s_get_metrics",
        "Return current CPU and memory usage from the metrics-server. \
         Shows top-10 pods by CPU and all node metrics.",
    )
    .with_handler(make_handler(state, |s, _args| async move {
        let mc = MetricsClient::new(s.client.clone());
        let snapshot: MetricsSnapshot = mc.fetch().await;

        let mut pod_list: Vec<(&String, &MetricSample)> = snapshot.pods.iter().collect();
        pod_list.sort_by(|a, b| b.1.cpu_m.cmp(&a.1.cpu_m));

        let pods: Vec<Value> = pod_list
            .into_iter()
            .take(10)
            .map(|(name, m)| {
                json!({
                    "name": name,
                    "cpu_millicores": m.cpu_m,
                    "mem_kibibytes": m.mem_ki,
                })
            })
            .collect();

        let nodes: Vec<Value> = snapshot
            .nodes
            .iter()
            .map(|(name, m)| {
                json!({
                    "name": name,
                    "cpu_millicores": m.cpu_m,
                    "mem_kibibytes": m.mem_ki,
                })
            })
            .collect();

        Ok(json_result(json!({
            "pods": pods,
            "nodes": nodes,
        })))
    }))
}

fn list_namespaces(state: Arc<McpState>) -> Tool {
    Tool::new(
        "k8s_list_namespaces",
        "List all accessible Kubernetes namespaces.",
    )
    .with_handler(make_handler(state, |s, _args| async move {
        let dao = NamespaceDao::new();
        let names = dao.list_names(&s.client).await.map_err(anyhow_to_mcp)?;
        Ok(json_result(json!({"namespaces": names})))
    }))
}

fn get_cluster_history(state: Arc<McpState>) -> Tool {
    Tool::new(
        "k8s_get_cluster_history",
        "Return a summary of recent cluster issues, operator actions, and workload drift \
         from the k7s metadata journal.",
    )
    .add_parameter(
        "days",
        "integer",
        "Number of days of history to include (default 7)",
    )
    .with_handler(make_handler(state, |s, args| async move {
        let days = args
            .get("days")
            .and_then(|v| v.as_u64())
            .map(|d| d.min(90) as u8)
            .unwrap_or(7);

        let Some(store) = MetadataStore::new(&s.meta_context) else {
            return Ok(text_result(format!(
                "No cluster metadata journal found for context '{}'.",
                s.meta_context
            )));
        };

        let records = store.load_recent(days);
        let history = summarise(&records, days, &s.meta_context);
        Ok(text_result(history.to_context_block()))
    }))
}

fn scale_deployment(state: Arc<McpState>) -> Tool {
    Tool::new(
        "k8s_scale_deployment",
        "Scale a Kubernetes Deployment to the specified number of replicas. \
         Only available when allowMutations is enabled in config.",
    )
    .add_parameter("name", "string", "Deployment name")
    .add_parameter("namespace", "string", "Namespace")
    .add_parameter("replicas", "integer", "Desired replica count")
    .with_handler(make_handler(state, |s, args| async move {
        if !s.allow_mutations {
            return Err(FastMCPError::new(
                "Mutating tools are disabled. Set allowMutations: true in config.".to_string(),
            ));
        }

        let name = require_str(&args, "name")?;
        let namespace = require_str(&args, "namespace")?;
        let replicas = args
            .get("replicas")
            .and_then(|v| v.as_u64())
            .ok_or_else(|| FastMCPError::new("replicas must be an integer".to_string()))?
            as u32;

        let api: Api<Deployment> = Api::namespaced(s.client.clone(), namespace);
        let patch = json!({"spec": {"replicas": replicas}});
        api.patch_scale(
            name,
            &kube::api::PatchParams::default(),
            &kube::api::Patch::Merge(patch),
        )
        .await
        .map_err(|e| FastMCPError::new(e.to_string()))?;

        Ok(text_result(format!(
            "Deployment {name} scaled to {replicas} replicas in {namespace}."
        )))
    }))
}

fn rollout_restart(state: Arc<McpState>) -> Tool {
    Tool::new(
        "k8s_rollout_restart",
        "Trigger a rolling restart of a Deployment, StatefulSet, or DaemonSet. \
         Only available when allowMutations is enabled in config.",
    )
    .add_parameter(
        "resource",
        "string",
        "Resource kind: deployment, statefulset, or daemonset",
    )
    .add_parameter("name", "string", "Resource name")
    .add_parameter("namespace", "string", "Namespace")
    .with_handler(make_handler(state, |s, args| async move {
        if !s.allow_mutations {
            return Err(FastMCPError::new(
                "Mutating tools are disabled. Set allowMutations: true in config.".to_string(),
            ));
        }

        let resource = require_str(&args, "resource")?;
        let name = require_str(&args, "name")?;
        let namespace = require_str(&args, "namespace")?;

        let target = format!("{resource}/{name}");
        let output = tokio::process::Command::new("kubectl")
            .args(["rollout", "restart", &target, "-n", namespace])
            .output()
            .await
            .map_err(|e| FastMCPError::new(format!("kubectl not found: {e}")))?;

        if output.status.success() {
            Ok(text_result(format!(
                "Rollout restart triggered for {target} in {namespace}."
            )))
        } else {
            Err(FastMCPError::new(
                String::from_utf8_lossy(&output.stderr).to_string(),
            ))
        }
    }))
}

// ─── Helpers ─────────────────────────────────────────────────────────────────

/// Extract a string argument from the args JSON object (returns `None` if missing or empty).
fn str_arg<'a>(args: &'a Value, key: &str) -> Option<&'a str> {
    args.get(key)
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
}

/// Extract a required string argument, returning a `FastMCPError` when absent.
fn require_str<'a>(args: &'a Value, key: &str) -> Result<&'a str, FastMCPError> {
    str_arg(args, key)
        .ok_or_else(|| FastMCPError::new(format!("required parameter '{key}' is missing or empty")))
}

/// Map a user-provided resource string to a `Gvr`.
fn gvr_from_resource(resource: &str) -> Gvr {
    match resource.to_lowercase().trim_end_matches('s') {
        "pod" => well_known::pods(),
        "deployment" | "deploy" => well_known::deployments(),
        "statefulset" | "sts" => well_known::stateful_sets(),
        "daemonset" | "ds" => well_known::daemon_sets(),
        "replicaset" | "rs" => well_known::replica_sets(),
        "service" | "svc" => well_known::services(),
        "node" | "no" => well_known::nodes(),
        "namespace" | "ns" => well_known::namespaces(),
        "configmap" | "cm" => well_known::config_maps(),
        "secret" => well_known::secrets(),
        "event" | "ev" => well_known::events(),
        "persistentvolume" | "pv" => well_known::persistent_volumes(),
        "persistentvolumeclaim" | "pvc" => well_known::persistent_volume_claims(),
        "ingress" | "ing" => well_known::ingresses(),
        "cronjob" | "cj" => well_known::cron_jobs(),
        "job" => well_known::jobs(),
        _ => Gvr::core("v1", resource),
    }
}

/// Strip sensitive fields from a raw event value before returning to MCP client.
fn sanitize_event(mut e: Value) -> Value {
    if let Some(obj) = e.as_object_mut() {
        obj.remove("managedFields");
        if let Some(meta) = obj.get_mut("metadata") {
            if let Some(m) = meta.as_object_mut() {
                m.remove("managedFields");
                m.remove("annotations"); // annotation values may contain secrets
            }
        }
    }
    e
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gvr_from_resource_pods() {
        let gvr = gvr_from_resource("pods");
        assert_eq!(gvr.resource, well_known::pods().resource);
    }

    #[test]
    fn gvr_from_resource_deployments() {
        let gvr = gvr_from_resource("deployments");
        assert_eq!(gvr.resource, well_known::deployments().resource);
    }

    #[test]
    fn gvr_from_resource_unknown_falls_back() {
        let gvr = gvr_from_resource("widgetfoos");
        assert_eq!(gvr.resource, "widgetfoos");
    }

    #[test]
    fn str_arg_returns_none_for_empty_string() {
        let args = serde_json::json!({"key": ""});
        assert!(str_arg(&args, "key").is_none());
    }

    #[test]
    fn str_arg_returns_none_for_missing_key() {
        let args = serde_json::json!({});
        assert!(str_arg(&args, "key").is_none());
    }

    #[test]
    fn require_str_errors_on_missing_key() {
        let args = serde_json::json!({});
        assert!(require_str(&args, "name").is_err());
    }

    #[test]
    fn sanitize_event_removes_managed_fields() {
        let e = serde_json::json!({
            "metadata": {"name": "e1", "managedFields": [{"manager": "kubectl"}], "annotations": {"k": "v"}},
            "managedFields": []
        });
        let safe = sanitize_event(e);
        assert!(safe.get("managedFields").is_none());
        assert!(safe["metadata"].get("managedFields").is_none());
        assert!(safe["metadata"].get("annotations").is_none());
    }
}
