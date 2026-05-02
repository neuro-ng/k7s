//! LimitRange renderer — Phase 28.
//!
//! Columns: NAME · AGE
//!
//! LimitRange contents are complex (per-type limits); details belong in describe.

use ratatui::layout::Constraint;
use serde_json::Value;

use crate::client::gvr::well_known;
use crate::client::Gvr;
use crate::render::{age_from_obj, meta_name, ColumnDef, RenderedRow, Renderer};

pub struct LimitRangeRenderer {
    gvr: Gvr,
    columns: Vec<ColumnDef>,
}

impl LimitRangeRenderer {
    pub fn new() -> Self {
        Self {
            gvr: well_known::limit_ranges(),
            columns: vec![
                ColumnDef::new("NAME", Constraint::Min(30)),
                ColumnDef::new("LIMITS", Constraint::Min(20)),
                ColumnDef::new("AGE", Constraint::Length(6)),
            ],
        }
    }
}

impl Default for LimitRangeRenderer {
    fn default() -> Self {
        Self::new()
    }
}

impl Renderer for LimitRangeRenderer {
    fn gvr(&self) -> &Gvr {
        &self.gvr
    }

    fn columns(&self) -> &[ColumnDef] {
        &self.columns
    }

    fn render(&self, obj: &Value) -> RenderedRow {
        let name = meta_name(obj).to_owned();

        // Summarise limit count by type.
        let limits = obj
            .pointer("/spec/limits")
            .and_then(|v| v.as_array())
            .map(|arr| {
                let types: Vec<&str> = arr.iter().filter_map(|l| l["type"].as_str()).collect();
                types.join(", ")
            })
            .unwrap_or_default();

        let (age, age_secs) = age_from_obj(obj);

        RenderedRow {
            cells: vec![name, limits, age],
            age_secs,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn columns_count() {
        assert_eq!(LimitRangeRenderer::new().columns().len(), 3);
    }

    #[test]
    fn render_limit_range() {
        let obj = json!({
            "metadata": { "name": "default-limits", "creationTimestamp": "2026-01-01T00:00:00Z" },
            "spec": {
                "limits": [
                    { "type": "Container", "max": { "cpu": "2" }, "min": { "cpu": "100m" } },
                    { "type": "Pod" }
                ]
            }
        });
        let row = LimitRangeRenderer::new().render(&obj);
        assert_eq!(row.cells[0], "default-limits");
        assert!(row.cells[1].contains("Container"));
        assert!(row.cells[1].contains("Pod"));
    }
}
