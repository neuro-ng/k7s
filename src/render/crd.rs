//! CRD renderer — Phase 2.12 / 6.13.
//!
//! Displays the list of Custom Resource Definitions on the cluster.
//! Column layout: NAME · GROUP · VERSION · SCOPE · ESTABLISHED · AGE
//!
//! # k9s Reference
//! `internal/render/crd.go`

use ratatui::layout::Constraint;
use serde_json::Value;

use crate::client::Gvr;
use crate::render::{age_from_obj, meta_name, ColumnDef, RenderedRow, Renderer};

pub struct CrdRenderer;

impl CrdRenderer {
    pub fn new() -> Self {
        Self
    }
}

impl Default for CrdRenderer {
    fn default() -> Self {
        Self::new()
    }
}

impl Renderer for CrdRenderer {
    fn gvr(&self) -> &Gvr {
        use std::sync::OnceLock;
        static GVR: OnceLock<Gvr> = OnceLock::new();
        GVR.get_or_init(crate::client::gvr::well_known::custom_resource_definitions)
    }

    fn columns(&self) -> &[ColumnDef] {
        static COLS: &[ColumnDef] = &[
            ColumnDef::new("NAME", Constraint::Min(40)),
            ColumnDef::new("GROUP", Constraint::Min(22)),
            ColumnDef::new("VERSION", Constraint::Length(10)),
            ColumnDef::new("SCOPE", Constraint::Length(10)),
            ColumnDef::new("ESTABLISHED", Constraint::Length(12)),
            ColumnDef::new("AGE", Constraint::Length(6)),
        ];
        COLS
    }

    fn render(&self, obj: &Value) -> RenderedRow {
        let name = meta_name(obj).to_owned();

        // spec.group
        let group = obj
            .pointer("/spec/group")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_owned();

        // spec.versions[0].name or spec.version (deprecated)
        let version = obj
            .pointer("/spec/versions/0/name")
            .and_then(|v| v.as_str())
            .or_else(|| obj.pointer("/spec/version").and_then(|v| v.as_str()))
            .unwrap_or("")
            .to_owned();

        // spec.scope: "Namespaced" or "Cluster"
        let scope = obj
            .pointer("/spec/scope")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_owned();

        // status.conditions: find type=Established
        let established = crd_established(obj);

        let (age, age_secs) = age_from_obj(obj);

        RenderedRow {
            cells: vec![name, group, version, scope, established, age],
            age_secs,
        }
    }
}

/// Extract the Established condition status from CRD status.
fn crd_established(obj: &Value) -> String {
    let Some(conditions) = obj.pointer("/status/conditions").and_then(|v| v.as_array()) else {
        return "-".to_owned();
    };
    for c in conditions {
        if c.get("type").and_then(|v| v.as_str()) == Some("Established") {
            return match c.get("status").and_then(|v| v.as_str()) {
                Some("True") => "true".to_owned(),
                Some("False") => "false".to_owned(),
                _ => "-".to_owned(),
            };
        }
    }
    "-".to_owned()
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn renderer() -> CrdRenderer {
        CrdRenderer::new()
    }

    #[test]
    fn columns_count() {
        assert_eq!(renderer().columns().len(), 6);
    }

    #[test]
    fn render_full_crd() {
        let obj = json!({
            "metadata": {
                "name": "foos.example.com",
                "creationTimestamp": "2024-01-01T00:00:00Z"
            },
            "spec": {
                "group": "example.com",
                "versions": [{"name": "v1alpha1"}],
                "scope": "Namespaced"
            },
            "status": {
                "conditions": [{"type": "Established", "status": "True"}]
            }
        });
        let row = renderer().render(&obj);
        assert_eq!(row.cells[0], "foos.example.com");
        assert_eq!(row.cells[1], "example.com");
        assert_eq!(row.cells[2], "v1alpha1");
        assert_eq!(row.cells[3], "Namespaced");
        assert_eq!(row.cells[4], "true");
    }

    #[test]
    fn render_cluster_scoped_crd() {
        let obj = json!({
            "metadata": {"name": "bars.acme.io"},
            "spec": {
                "group": "acme.io",
                "versions": [{"name": "v1"}],
                "scope": "Cluster"
            },
            "status": {}
        });
        let row = renderer().render(&obj);
        assert_eq!(row.cells[3], "Cluster");
        assert_eq!(row.cells[4], "-");
    }

    #[test]
    fn render_missing_fields_gracefully() {
        let row = renderer().render(&json!({"metadata": {"name": "x"}}));
        assert_eq!(row.cells.len(), 6);
    }
}
