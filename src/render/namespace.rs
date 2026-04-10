use ratatui::layout::Constraint;
use serde_json::Value;

use crate::client::Gvr;
use crate::client::gvr::well_known;
use crate::render::{age_from_obj, meta_name, ColumnDef, RenderedRow, Renderer};

pub struct NamespaceRenderer {
    gvr: Gvr,
    columns: Vec<ColumnDef>,
}

impl NamespaceRenderer {
    pub fn new() -> Self {
        Self {
            gvr: well_known::namespaces(),
            columns: vec![
                ColumnDef::new("NAME",   Constraint::Min(20)),
                ColumnDef::new("STATUS", Constraint::Length(10)),
                ColumnDef::new("AGE",    Constraint::Length(6)),
            ],
        }
    }
}

impl Default for NamespaceRenderer {
    fn default() -> Self { Self::new() }
}

impl Renderer for NamespaceRenderer {
    fn gvr(&self) -> &Gvr { &self.gvr }
    fn columns(&self) -> &[ColumnDef] { &self.columns }

    fn render(&self, obj: &Value) -> RenderedRow {
        let name   = meta_name(obj).to_owned();
        let status = obj.pointer("/status/phase").and_then(|v| v.as_str()).unwrap_or("Unknown").to_owned();
        let (age, age_secs) = age_from_obj(obj);

        RenderedRow {
            cells: vec![name, status, age],
            age_secs,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn render_active_namespace() {
        let obj = json!({
            "metadata": {"name": "production"},
            "status": {"phase": "Active"}
        });
        let r = NamespaceRenderer::new().render(&obj);
        assert_eq!(r.cells[0], "production");
        assert_eq!(r.cells[1], "Active");
    }
}
