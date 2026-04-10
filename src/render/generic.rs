//! Generic renderer for CRDs and resource types without a dedicated renderer.
//!
//! Displays: NAME, NAMESPACE (if namespaced), AGE.
//! For resources with a `status.phase` or `status.ready` field, those are also shown.

use ratatui::layout::Constraint;
use serde_json::Value;

use crate::client::Gvr;
use crate::render::{age_from_obj, meta_name, meta_namespace, ColumnDef, RenderedRow, Renderer};

pub struct GenericRenderer {
    gvr: Gvr,
    columns: Vec<ColumnDef>,
    namespaced: bool,
}

impl GenericRenderer {
    pub fn new(gvr: Gvr, namespaced: bool) -> Self {
        let mut columns = vec![ColumnDef::new("NAME", Constraint::Min(24))];
        if namespaced {
            columns.push(ColumnDef::new("NAMESPACE", Constraint::Min(14)));
        }
        columns.push(ColumnDef::new("STATUS", Constraint::Min(10)));
        columns.push(ColumnDef::new("AGE", Constraint::Length(6)));
        Self { gvr, columns, namespaced }
    }
}

impl Renderer for GenericRenderer {
    fn gvr(&self) -> &Gvr { &self.gvr }
    fn columns(&self) -> &[ColumnDef] { &self.columns }

    fn render(&self, obj: &Value) -> RenderedRow {
        let name = meta_name(obj).to_owned();
        let status = generic_status(obj);
        let (age, age_secs) = age_from_obj(obj);

        let cells = if self.namespaced {
            let ns = meta_namespace(obj).to_owned();
            vec![name, ns, status, age]
        } else {
            vec![name, status, age]
        };

        RenderedRow { cells, age_secs }
    }
}

/// Best-effort status extraction for unknown resource types.
fn generic_status(obj: &Value) -> String {
    // Try common status fields in order of preference.
    if let Some(phase) = obj.pointer("/status/phase").and_then(|v| v.as_str()) {
        return phase.to_owned();
    }

    // Ready condition pattern used by many CRDs.
    if let Some(conditions) = obj.pointer("/status/conditions").and_then(|v| v.as_array()) {
        for c in conditions {
            if c.get("type").and_then(|v| v.as_str()) == Some("Ready") {
                return match c.get("status").and_then(|v| v.as_str()) {
                    Some("True") => "Ready".to_owned(),
                    Some("False") => {
                        c.get("reason")
                            .and_then(|v| v.as_str())
                            .unwrap_or("NotReady")
                            .to_owned()
                    }
                    _ => "Unknown".to_owned(),
                };
            }
        }
    }

    if let Some(ready) = obj.pointer("/status/ready").and_then(|v| v.as_bool()) {
        return if ready { "Ready".to_owned() } else { "NotReady".to_owned() };
    }

    "-".to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::gvr::well_known;
    use serde_json::json;

    #[test]
    fn generic_render_namespaced() {
        let gvr = well_known::config_maps();
        let renderer = GenericRenderer::new(gvr, true);
        let obj = json!({
            "metadata": {
                "name": "my-cm",
                "namespace": "default",
                "creationTimestamp": "2020-01-01T00:00:00Z"
            }
        });
        let r = renderer.render(&obj);
        assert_eq!(r.cells[0], "my-cm");
        assert_eq!(r.cells[1], "default");
    }

    #[test]
    fn generic_status_from_phase() {
        let obj = json!({"status": {"phase": "Active"}});
        assert_eq!(generic_status(&obj), "Active");
    }

    #[test]
    fn generic_status_from_conditions() {
        let obj = json!({
            "status": {
                "conditions": [{ "type": "Ready", "status": "True" }]
            }
        });
        assert_eq!(generic_status(&obj), "Ready");
    }

    #[test]
    fn generic_status_fallback() {
        let obj = json!({"status": {}});
        assert_eq!(generic_status(&obj), "-");
    }
}
