use ratatui::layout::Constraint;
use serde_json::Value;

use crate::client::Gvr;
use crate::client::gvr::well_known;
use crate::render::{age_from_obj, meta_name, ColumnDef, RenderedRow, Renderer};

pub struct StatefulSetRenderer {
    gvr: Gvr,
    columns: Vec<ColumnDef>,
}

impl StatefulSetRenderer {
    pub fn new() -> Self {
        Self {
            gvr: well_known::stateful_sets(),
            columns: vec![
                ColumnDef::new("NAME",    Constraint::Min(20)),
                ColumnDef::new("READY",   Constraint::Length(8)),
                ColumnDef::new("AGE",     Constraint::Length(6)),
            ],
        }
    }
}

impl Default for StatefulSetRenderer {
    fn default() -> Self { Self::new() }
}

impl Renderer for StatefulSetRenderer {
    fn gvr(&self) -> &Gvr { &self.gvr }
    fn columns(&self) -> &[ColumnDef] { &self.columns }

    fn render(&self, obj: &Value) -> RenderedRow {
        let name = meta_name(obj).to_owned();
        let desired = obj.pointer("/spec/replicas").and_then(|v| v.as_i64()).unwrap_or(1);
        let ready   = obj.pointer("/status/readyReplicas").and_then(|v| v.as_i64()).unwrap_or(0);
        let (age, age_secs) = age_from_obj(obj);

        RenderedRow {
            cells: vec![name, format!("{ready}/{desired}"), age],
            age_secs,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn render_statefulset() {
        let obj = json!({
            "metadata": {"name": "my-sts"},
            "spec": {"replicas": 3},
            "status": {"readyReplicas": 2, "replicas": 3}
        });
        let r = StatefulSetRenderer::new().render(&obj);
        assert_eq!(r.cells[0], "my-sts");
        assert_eq!(r.cells[1], "2/3");
    }
}
