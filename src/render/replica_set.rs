use ratatui::layout::Constraint;
use serde_json::Value;

use crate::client::gvr::well_known;
use crate::client::Gvr;
use crate::render::{age_from_obj, meta_name, ColumnDef, RenderedRow, Renderer};

pub struct ReplicaSetRenderer {
    gvr: Gvr,
    columns: Vec<ColumnDef>,
}

impl ReplicaSetRenderer {
    pub fn new() -> Self {
        Self {
            gvr: well_known::replica_sets(),
            columns: vec![
                ColumnDef::new("NAME", Constraint::Min(20)),
                ColumnDef::new("DESIRED", Constraint::Length(8)),
                ColumnDef::new("CURRENT", Constraint::Length(8)),
                ColumnDef::new("READY", Constraint::Length(6)),
                ColumnDef::new("AGE", Constraint::Length(6)),
            ],
        }
    }
}

impl Default for ReplicaSetRenderer {
    fn default() -> Self {
        Self::new()
    }
}

impl Renderer for ReplicaSetRenderer {
    fn gvr(&self) -> &Gvr {
        &self.gvr
    }
    fn columns(&self) -> &[ColumnDef] {
        &self.columns
    }

    fn render(&self, obj: &Value) -> RenderedRow {
        let name = meta_name(obj).to_owned();
        let desired = obj
            .pointer("/spec/replicas")
            .and_then(|v| v.as_i64())
            .unwrap_or(0);
        let current = obj
            .pointer("/status/replicas")
            .and_then(|v| v.as_i64())
            .unwrap_or(0);
        let ready = obj
            .pointer("/status/readyReplicas")
            .and_then(|v| v.as_i64())
            .unwrap_or(0);
        let (age, age_secs) = age_from_obj(obj);

        RenderedRow {
            cells: vec![
                name,
                desired.to_string(),
                current.to_string(),
                ready.to_string(),
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
    fn render_replicaset() {
        let obj = json!({
            "metadata": {"name": "my-rs"},
            "spec": {"replicas": 2},
            "status": {"replicas": 2, "readyReplicas": 2}
        });
        let r = ReplicaSetRenderer::new().render(&obj);
        assert_eq!(r.cells[0], "my-rs");
        assert_eq!(r.cells[1], "2"); // desired
        assert_eq!(r.cells[3], "2"); // ready
    }
}
