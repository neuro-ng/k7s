//! Alias renderer — lists all registered resource aliases.
//!
//! Column layout:
//! `RESOURCE · APIVERSION · NAMESPACED · ALIASES`
//!
//! Used by the `:alias` view.

use ratatui::layout::Constraint;
use serde_json::Value;

use crate::client::Gvr;
use crate::render::{ColumnDef, RenderedRow, Renderer};

pub struct AliasRenderer;

impl AliasRenderer {
    pub fn new() -> Self {
        Self
    }
}

impl Default for AliasRenderer {
    fn default() -> Self {
        Self::new()
    }
}

impl Renderer for AliasRenderer {
    fn gvr(&self) -> &Gvr {
        use std::sync::OnceLock;
        static GVR: OnceLock<Gvr> = OnceLock::new();
        GVR.get_or_init(|| Gvr::new("", "v1", "aliases"))
    }

    fn columns(&self) -> &[ColumnDef] {
        static COLS: &[ColumnDef] = &[
            ColumnDef::new("RESOURCE", Constraint::Min(20)),
            ColumnDef::new("APIVERSION", Constraint::Min(22)),
            ColumnDef::new("NAMESPACED", Constraint::Length(10)),
            ColumnDef::new("ALIASES", Constraint::Min(30)),
        ];
        COLS
    }

    fn render(&self, obj: &Value) -> RenderedRow {
        let resource = obj["resource"].as_str().unwrap_or("").to_owned();
        let apiversion = obj["apiversion"].as_str().unwrap_or("").to_owned();
        let namespaced = obj["namespaced"].as_str().unwrap_or("").to_owned();
        let aliases = obj["aliases"].as_str().unwrap_or("").to_owned();

        RenderedRow {
            cells: vec![resource, apiversion, namespaced, aliases],
            age_secs: 0,
        }
    }
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn renderer() -> AliasRenderer {
        AliasRenderer::new()
    }

    #[test]
    fn columns_count() {
        assert_eq!(renderer().columns().len(), 4);
        assert_eq!(renderer().columns()[0].name, "RESOURCE");
    }

    #[test]
    fn render_alias_row() {
        let obj = json!({
            "resource":   "Pods",
            "apiversion": "v1",
            "namespaced": "true",
            "aliases":    "po, pod, pods",
        });
        let row = renderer().render(&obj);
        assert_eq!(row.cells[0], "Pods");
        assert_eq!(row.cells[1], "v1");
        assert_eq!(row.cells[2], "true");
        assert_eq!(row.cells[3], "po, pod, pods");
    }

    #[test]
    fn render_missing_fields_gracefully() {
        let row = renderer().render(&json!({}));
        assert_eq!(row.cells.len(), 4);
        assert!(row.cells.iter().all(|c| c.is_empty()));
    }

    #[test]
    fn cluster_scoped_row() {
        let obj = json!({
            "resource":   "Nodes",
            "apiversion": "v1",
            "namespaced": "false",
            "aliases":    "no, node, nodes",
        });
        let row = renderer().render(&obj);
        assert_eq!(row.cells[2], "false");
    }
}
