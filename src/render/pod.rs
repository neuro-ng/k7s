use ratatui::layout::Constraint;
use serde_json::Value;

use crate::client::gvr::well_known;
use crate::client::Gvr;
use crate::render::{age_from_obj, meta_name, ColumnDef, RenderedRow, Renderer};

pub struct PodRenderer {
    gvr: Gvr,
    columns: Vec<ColumnDef>,
}

impl PodRenderer {
    pub fn new() -> Self {
        Self {
            gvr: well_known::pods(),
            columns: vec![
                ColumnDef::new("NAME", Constraint::Min(20)),
                ColumnDef::new("READY", Constraint::Length(7)),
                ColumnDef::new("STATUS", Constraint::Min(12)),
                ColumnDef::new("RESTARTS", Constraint::Length(9)),
                ColumnDef::new("IP", Constraint::Length(16)),
                ColumnDef::new("NODE", Constraint::Min(12)),
                ColumnDef::new("AGE", Constraint::Length(6)),
            ],
        }
    }
}

impl Default for PodRenderer {
    fn default() -> Self {
        Self::new()
    }
}

impl Renderer for PodRenderer {
    fn gvr(&self) -> &Gvr {
        &self.gvr
    }
    fn columns(&self) -> &[ColumnDef] {
        &self.columns
    }

    fn render(&self, obj: &Value) -> RenderedRow {
        let name = meta_name(obj).to_owned();
        let ready = pod_ready(obj);
        let status = pod_status(obj);
        let restarts = pod_restarts(obj).to_string();
        let ip = obj
            .pointer("/status/podIP")
            .and_then(|v| v.as_str())
            .unwrap_or("-")
            .to_owned();
        let node = obj
            .pointer("/spec/nodeName")
            .and_then(|v| v.as_str())
            .unwrap_or("-")
            .to_owned();
        let (age, age_secs) = age_from_obj(obj);

        RenderedRow {
            cells: vec![name, ready, status, restarts, ip, node, age],
            age_secs,
        }
    }
}

/// `N/M` ready containers.
fn pod_ready(obj: &Value) -> String {
    let cs = obj
        .pointer("/status/containerStatuses")
        .and_then(|v| v.as_array());

    match cs {
        None => "0/0".to_owned(),
        Some(arr) => {
            let total = arr.len();
            let ready = arr
                .iter()
                .filter(|c| c.get("ready").and_then(|v| v.as_bool()).unwrap_or(false))
                .count();
            format!("{ready}/{total}")
        }
    }
}

/// Derive a meaningful pod status string — mirrors `kubectl get pods` STATUS column.
fn pod_status(obj: &Value) -> String {
    // Terminating: has a deletionTimestamp.
    if obj.pointer("/metadata/deletionTimestamp").is_some() {
        return "Terminating".to_owned();
    }

    // Init containers: if any are failing/waiting, surface that first.
    if let Some(init_cs) = obj
        .pointer("/status/initContainerStatuses")
        .and_then(|v| v.as_array())
    {
        for c in init_cs {
            if let Some(reason) = container_state_reason(c) {
                return reason;
            }
        }
    }

    // Regular container states.
    if let Some(cs) = obj
        .pointer("/status/containerStatuses")
        .and_then(|v| v.as_array())
    {
        for c in cs {
            if let Some(reason) = container_state_reason(c) {
                return reason;
            }
        }
    }

    // Fall back to pod phase.
    obj.pointer("/status/phase")
        .and_then(|v| v.as_str())
        .unwrap_or("Pending")
        .to_owned()
}

fn container_state_reason(container_status: &Value) -> Option<String> {
    let state = container_status.get("state")?;

    // Waiting with a reason is always surfaced (CrashLoopBackOff, ImagePullBackOff, etc.)
    if let Some(reason) = state.pointer("/waiting/reason").and_then(|v| v.as_str()) {
        return Some(reason.to_owned());
    }

    // Terminated with non-zero exit code or non-Completed reason.
    if let Some(term) = state.get("terminated") {
        let reason = term
            .get("reason")
            .and_then(|v| v.as_str())
            .unwrap_or("Terminated");
        if reason != "Completed" {
            return Some(reason.to_owned());
        }
        let exit = term.get("exitCode").and_then(|v| v.as_i64()).unwrap_or(0);
        if exit != 0 {
            return Some(format!("Error({})", exit));
        }
    }

    None
}

/// Sum of restart counts across all containers.
fn pod_restarts(obj: &Value) -> i64 {
    obj.pointer("/status/containerStatuses")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|c| c.get("restartCount").and_then(|v| v.as_i64()))
                .sum()
        })
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn render_running_pod() {
        let obj = json!({
            "metadata": {
                "name": "my-pod",
                "creationTimestamp": "2020-01-01T00:00:00Z"
            },
            "spec": { "nodeName": "node-1" },
            "status": {
                "phase": "Running",
                "podIP": "10.0.0.1",
                "containerStatuses": [{
                    "name": "app",
                    "ready": true,
                    "restartCount": 2,
                    "state": { "running": {} }
                }]
            }
        });
        let r = PodRenderer::new().render(&obj);
        assert_eq!(r.cells[0], "my-pod");
        assert_eq!(r.cells[1], "1/1");
        assert_eq!(r.cells[2], "Running");
        assert_eq!(r.cells[3], "2");
        assert_eq!(r.cells[4], "10.0.0.1");
    }

    #[test]
    fn render_crashloop_pod() {
        let obj = json!({
            "metadata": { "name": "bad-pod" },
            "status": {
                "phase": "Running",
                "containerStatuses": [{
                    "name": "app",
                    "ready": false,
                    "restartCount": 15,
                    "state": {
                        "waiting": { "reason": "CrashLoopBackOff" }
                    }
                }]
            }
        });
        let r = PodRenderer::new().render(&obj);
        assert_eq!(r.cells[2], "CrashLoopBackOff");
        assert_eq!(r.cells[3], "15");
    }

    #[test]
    fn render_terminating_pod() {
        let obj = json!({
            "metadata": {
                "name": "dying-pod",
                "deletionTimestamp": "2024-01-01T00:00:00Z"
            },
            "status": { "phase": "Running" }
        });
        let r = PodRenderer::new().render(&obj);
        assert_eq!(r.cells[2], "Terminating");
    }

    #[test]
    fn pod_ready_no_containers() {
        let obj = json!({"status": {}});
        assert_eq!(pod_ready(&obj), "0/0");
    }
}
