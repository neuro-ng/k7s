use ratatui::layout::Constraint;
use serde_json::Value;

use crate::client::gvr::well_known;
use crate::client::Gvr;
use crate::render::{age_from_obj, meta_name, ColumnDef, RenderedRow, Renderer};

pub struct NodeRenderer {
    gvr: Gvr,
    columns: Vec<ColumnDef>,
}

impl NodeRenderer {
    pub fn new() -> Self {
        Self {
            gvr: well_known::nodes(),
            columns: vec![
                ColumnDef::new("NAME", Constraint::Min(20)),
                ColumnDef::new("STATUS", Constraint::Length(10)),
                ColumnDef::new("ROLES", Constraint::Min(14)),
                ColumnDef::new("AGE", Constraint::Length(6)),
                ColumnDef::new("VERSION", Constraint::Length(12)),
            ],
        }
    }
}

impl Default for NodeRenderer {
    fn default() -> Self {
        Self::new()
    }
}

impl Renderer for NodeRenderer {
    fn gvr(&self) -> &Gvr {
        &self.gvr
    }
    fn columns(&self) -> &[ColumnDef] {
        &self.columns
    }

    fn render(&self, obj: &Value) -> RenderedRow {
        let name = meta_name(obj).to_owned();
        let status = node_status(obj);
        let roles = node_roles(obj);
        let (age, age_secs) = age_from_obj(obj);
        let version = obj
            .pointer("/status/nodeInfo/kubeletVersion")
            .and_then(|v| v.as_str())
            .unwrap_or("-")
            .to_owned();

        RenderedRow {
            cells: vec![name, status, roles, age, version],
            age_secs,
        }
    }
}

/// Determine node readiness from `conditions`.
fn node_status(obj: &Value) -> String {
    let conditions = obj.pointer("/status/conditions").and_then(|v| v.as_array());

    if let Some(conds) = conditions {
        // Check for specific problem conditions first.
        for c in conds {
            let type_ = c.get("type").and_then(|v| v.as_str()).unwrap_or("");
            let status = c.get("status").and_then(|v| v.as_str()).unwrap_or("False");
            match (type_, status) {
                ("MemoryPressure", "True") => return "MemoryPressure".to_owned(),
                ("DiskPressure", "True") => return "DiskPressure".to_owned(),
                ("PIDPressure", "True") => return "PIDPressure".to_owned(),
                ("NetworkUnavailable", "True") => return "NetworkUnavailable".to_owned(),
                _ => {}
            }
        }
        // Ready condition.
        for c in conds {
            if c.get("type").and_then(|v| v.as_str()) == Some("Ready") {
                return match c.get("status").and_then(|v| v.as_str()) {
                    Some("True") => "Ready".to_owned(),
                    Some("Unknown") => "Unknown".to_owned(),
                    _ => "NotReady".to_owned(),
                };
            }
        }
    }

    // Check if node is unschedulable (cordoned).
    if obj
        .pointer("/spec/unschedulable")
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
    {
        return "SchedulingDisabled".to_owned();
    }

    "Unknown".to_owned()
}

/// Derive node roles from `node-role.kubernetes.io/*` labels.
fn node_roles(obj: &Value) -> String {
    let labels = match obj.pointer("/metadata/labels").and_then(|v| v.as_object()) {
        Some(l) => l,
        None => return "<none>".to_owned(),
    };

    let mut roles: Vec<&str> = labels
        .keys()
        .filter_map(|k| k.strip_prefix("node-role.kubernetes.io/"))
        .collect();

    if roles.is_empty() {
        // Older clusters use a single label.
        if labels.contains_key("node-role.kubernetes.io/master") {
            roles.push("master");
        } else {
            return "<none>".to_owned();
        }
    }

    roles.sort_unstable();
    roles.join(",")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn render_ready_node() {
        let obj = json!({
            "metadata": {
                "name": "node-1",
                "creationTimestamp": "2020-01-01T00:00:00Z",
                "labels": { "node-role.kubernetes.io/control-plane": "" }
            },
            "status": {
                "conditions": [{ "type": "Ready", "status": "True" }],
                "nodeInfo": { "kubeletVersion": "v1.30.0" }
            }
        });
        let r = NodeRenderer::new().render(&obj);
        assert_eq!(r.cells[0], "node-1");
        assert_eq!(r.cells[1], "Ready");
        assert_eq!(r.cells[2], "control-plane");
        assert_eq!(r.cells[4], "v1.30.0");
    }

    #[test]
    fn node_roles_none() {
        let obj = json!({"metadata": {"labels": {}}});
        assert_eq!(node_roles(&obj), "<none>");
    }

    #[test]
    fn node_roles_multiple() {
        let obj = json!({
            "metadata": {
                "labels": {
                    "node-role.kubernetes.io/control-plane": "",
                    "node-role.kubernetes.io/etcd": ""
                }
            }
        });
        let roles = node_roles(&obj);
        assert!(roles.contains("control-plane"));
        assert!(roles.contains("etcd"));
    }

    #[test]
    fn node_status_notready() {
        let obj = json!({
            "status": {
                "conditions": [{ "type": "Ready", "status": "False" }]
            }
        });
        assert_eq!(node_status(&obj), "NotReady");
    }
}
