//! Context renderer — Phase 6.8.
//!
//! Converts a serialized [`ContextEntry`] (from the kubeconfig context DAO)
//! into a display row.
//!
//! Column layout mirrors k9s context view:
//! `NAME  CLUSTER  NAMESPACE  CURRENT`
//!
//! # k9s Reference
//! `internal/render/ctx.go`

use ratatui::layout::Constraint;
use serde_json::Value;

use crate::client::Gvr;
use crate::render::{ColumnDef, RenderedRow, Renderer};

/// A synthetic GVR representing kubeconfig contexts (not a real K8s resource).
pub fn context_gvr() -> Gvr {
    Gvr {
        group: "".to_owned(),
        version: "v1".to_owned(),
        resource: "contexts".to_owned(),
    }
}

/// Renders a kubeconfig context entry into a table row.
///
/// Expects the JSON produced by
/// `serde_json::to_value(&ContextEntry { name, cluster, namespace, is_current })`.
pub struct ContextRenderer;

impl ContextRenderer {
    pub fn new() -> Self {
        Self
    }
}

impl Default for ContextRenderer {
    fn default() -> Self {
        Self::new()
    }
}

impl Renderer for ContextRenderer {
    fn gvr(&self) -> &Gvr {
        // The renderer is stateless; the GVR is constructed on demand.
        // We return a 'static reference by leaking a one-time allocation.
        // (This pattern is acceptable for a handful of synthetic GVRs.)
        use std::sync::OnceLock;
        static GVR: OnceLock<Gvr> = OnceLock::new();
        GVR.get_or_init(context_gvr)
    }

    fn columns(&self) -> &[ColumnDef] {
        static COLS: &[ColumnDef] = &[
            ColumnDef::new("NAME", Constraint::Min(20)),
            ColumnDef::new("CLUSTER", Constraint::Min(35)),
            ColumnDef::new("NAMESPACE", Constraint::Min(15)),
            ColumnDef::new("CURRENT", Constraint::Length(8)),
        ];
        COLS
    }

    fn render(&self, obj: &Value) -> RenderedRow {
        let name = obj["name"].as_str().unwrap_or("").to_owned();
        let cluster = obj["cluster"].as_str().unwrap_or("").to_owned();
        let namespace = obj["namespace"].as_str().unwrap_or("").to_owned();
        let current = obj["is_current"].as_bool().unwrap_or(false);

        let current_marker = if current { "✓" } else { "" }.to_owned();

        RenderedRow {
            cells: vec![name, cluster, namespace, current_marker],
            age_secs: 0,
        }
    }
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn renderer() -> ContextRenderer {
        ContextRenderer::new()
    }

    #[test]
    fn columns_count() {
        assert_eq!(renderer().columns().len(), 4);
    }

    #[test]
    fn render_current_context() {
        let obj = json!({
            "name": "prod",
            "cluster": "https://prod.example.com",
            "namespace": "default",
            "is_current": true
        });
        let row = renderer().render(&obj);
        assert_eq!(row.cells[0], "prod");
        assert_eq!(row.cells[1], "https://prod.example.com");
        assert_eq!(row.cells[2], "default");
        assert_eq!(row.cells[3], "✓");
    }

    #[test]
    fn render_non_current_context() {
        let obj = json!({
            "name": "staging",
            "cluster": "https://staging.example.com",
            "namespace": "",
            "is_current": false
        });
        let row = renderer().render(&obj);
        assert_eq!(row.cells[3], "");
    }

    #[test]
    fn render_missing_fields_gracefully() {
        let row = renderer().render(&json!({}));
        assert_eq!(row.cells.len(), 4);
        assert!(row.cells[3].is_empty());
    }
}
