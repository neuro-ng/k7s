use ratatui::layout::Constraint;
use serde_json::Value;

use crate::client::gvr::well_known;
use crate::client::Gvr;
use crate::render::{age_from_obj, meta_name, ColumnDef, RenderedRow, Renderer};

pub struct DaemonSetRenderer {
    gvr: Gvr,
    columns: Vec<ColumnDef>,
}

impl DaemonSetRenderer {
    pub fn new() -> Self {
        Self {
            gvr: well_known::daemon_sets(),
            columns: vec![
                ColumnDef::new("NAME", Constraint::Min(20)),
                ColumnDef::new("DESIRED", Constraint::Length(8)),
                ColumnDef::new("CURRENT", Constraint::Length(8)),
                ColumnDef::new("READY", Constraint::Length(6)),
                ColumnDef::new("UP-TO-DATE", Constraint::Length(11)),
                ColumnDef::new("AVAILABLE", Constraint::Length(10)),
                ColumnDef::new("AGE", Constraint::Length(6)),
            ],
        }
    }
}

impl Default for DaemonSetRenderer {
    fn default() -> Self {
        Self::new()
    }
}

impl Renderer for DaemonSetRenderer {
    fn gvr(&self) -> &Gvr {
        &self.gvr
    }
    fn columns(&self) -> &[ColumnDef] {
        &self.columns
    }

    fn render(&self, obj: &Value) -> RenderedRow {
        let name = meta_name(obj).to_owned();
        let desired = obj
            .pointer("/status/desiredNumberScheduled")
            .and_then(|v| v.as_i64())
            .unwrap_or(0);
        let current = obj
            .pointer("/status/currentNumberScheduled")
            .and_then(|v| v.as_i64())
            .unwrap_or(0);
        let ready = obj
            .pointer("/status/numberReady")
            .and_then(|v| v.as_i64())
            .unwrap_or(0);
        let updated = obj
            .pointer("/status/updatedNumberScheduled")
            .and_then(|v| v.as_i64())
            .unwrap_or(0);
        let available = obj
            .pointer("/status/numberAvailable")
            .and_then(|v| v.as_i64())
            .unwrap_or(0);
        let (age, age_secs) = age_from_obj(obj);

        RenderedRow {
            cells: vec![
                name,
                desired.to_string(),
                current.to_string(),
                ready.to_string(),
                updated.to_string(),
                available.to_string(),
                age,
            ],
            age_secs,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn render_daemonset() {
        let obj = json!({
            "metadata": {"name": "my-ds"},
            "status": {
                "desiredNumberScheduled": 3,
                "currentNumberScheduled": 3,
                "numberReady": 3,
                "updatedNumberScheduled": 3,
                "numberAvailable": 3
            }
        });
        let r = DaemonSetRenderer::new().render(&obj);
        assert_eq!(r.cells[0], "my-ds");
        assert_eq!(r.cells[1], "3"); // desired
        assert_eq!(r.cells[3], "3"); // ready
    }
}
