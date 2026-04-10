//! Renderer for Kubernetes Events (v1/events).

use ratatui::layout::Constraint;
use serde_json::Value;

use crate::client::gvr::well_known;
use crate::client::Gvr;
use crate::render::{age_from_obj, meta_name, meta_namespace, ColumnDef, RenderedRow, Renderer};

pub struct EventRenderer {
    gvr: Gvr,
    columns: Vec<ColumnDef>,
}

impl EventRenderer {
    pub fn new() -> Self {
        Self {
            gvr: well_known::events(),
            columns: vec![
                ColumnDef::new("NAMESPACE", Constraint::Length(18)),
                ColumnDef::new("NAME", Constraint::Min(30)),
                ColumnDef::new("TYPE", Constraint::Length(8)),
                ColumnDef::new("REASON", Constraint::Length(22)),
                ColumnDef::new("COUNT", Constraint::Length(6)),
                ColumnDef::new("OBJECT", Constraint::Min(20)),
                ColumnDef::new("MESSAGE", Constraint::Min(40)),
                ColumnDef::new("AGE", Constraint::Length(6)),
            ],
        }
    }
}

impl Default for EventRenderer {
    fn default() -> Self {
        Self::new()
    }
}

impl Renderer for EventRenderer {
    fn gvr(&self) -> &Gvr {
        &self.gvr
    }
    fn columns(&self) -> &[ColumnDef] {
        &self.columns
    }

    fn render(&self, obj: &Value) -> RenderedRow {
        let namespace = meta_namespace(obj).to_owned();
        let name = meta_name(obj).to_owned();

        let event_type = obj
            .pointer("/type")
            .and_then(|v| v.as_str())
            .unwrap_or("Normal")
            .to_owned();

        let reason = obj
            .pointer("/reason")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_owned();

        let count = obj
            .pointer("/count")
            .and_then(|v| v.as_u64())
            .unwrap_or(1)
            .to_string();

        // involvedObject: "kind/name"
        let involved_kind = obj
            .pointer("/involvedObject/kind")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let involved_name = obj
            .pointer("/involvedObject/name")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let object = if involved_kind.is_empty() {
            String::new()
        } else {
            format!("{}/{}", involved_kind, involved_name)
        };

        // Truncate long messages so the table stays readable.
        let message = obj
            .pointer("/message")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .chars()
            .take(120)
            .collect::<String>();

        let (age, age_secs) = age_from_obj(obj);

        RenderedRow {
            cells: vec![
                namespace, name, event_type, reason, count, object, message, age,
            ],
            age_secs,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn sample_event() -> Value {
        json!({
            "metadata": {
                "name": "my-pod.17c1234",
                "namespace": "default",
                "creationTimestamp": "2024-01-01T00:00:00Z"
            },
            "type": "Warning",
            "reason": "BackOff",
            "count": 47,
            "involvedObject": {
                "kind": "Pod",
                "name": "my-pod"
            },
            "message": "Back-off restarting failed container app in pod my-pod_default"
        })
    }

    #[test]
    fn render_event_columns() {
        let r = EventRenderer::new().render(&sample_event());
        assert_eq!(r.cells[0], "default", "namespace");
        assert_eq!(r.cells[2], "Warning", "type");
        assert_eq!(r.cells[3], "BackOff", "reason");
        assert_eq!(r.cells[4], "47", "count");
        assert_eq!(r.cells[5], "Pod/my-pod", "involvedObject");
        assert!(r.cells[6].contains("Back-off"), "message");
    }

    #[test]
    fn missing_optional_fields_dont_panic() {
        let sparse = json!({ "metadata": { "name": "ev" } });
        let r = EventRenderer::new().render(&sparse);
        assert_eq!(r.cells.len(), 8);
        assert_eq!(r.cells[2], "Normal"); // default type
        assert_eq!(r.cells[4], "1"); // default count
    }
}
