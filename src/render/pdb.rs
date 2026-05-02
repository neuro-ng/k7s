//! PodDisruptionBudget renderer — Phase 28.
//!
//! Columns: NAME · MIN-AVAILABLE · MAX-UNAVAILABLE · ALLOWED-DISRUPTIONS · AGE

use ratatui::layout::Constraint;
use serde_json::Value;

use crate::client::gvr::well_known;
use crate::client::Gvr;
use crate::render::{age_from_obj, meta_name, ColumnDef, RenderedRow, Renderer};

pub struct PdbRenderer {
    gvr: Gvr,
    columns: Vec<ColumnDef>,
}

impl PdbRenderer {
    pub fn new() -> Self {
        Self {
            gvr: well_known::pod_disruption_budgets(),
            columns: vec![
                ColumnDef::new("NAME", Constraint::Min(20)),
                ColumnDef::new("MIN-AVAILABLE", Constraint::Length(14)),
                ColumnDef::new("MAX-UNAVAILABLE", Constraint::Length(16)),
                ColumnDef::new("ALLOWED-DISRUPTIONS", Constraint::Length(20)),
                ColumnDef::new("AGE", Constraint::Length(6)),
            ],
        }
    }
}

impl Default for PdbRenderer {
    fn default() -> Self {
        Self::new()
    }
}

impl Renderer for PdbRenderer {
    fn gvr(&self) -> &Gvr {
        &self.gvr
    }

    fn columns(&self) -> &[ColumnDef] {
        &self.columns
    }

    fn render(&self, obj: &Value) -> RenderedRow {
        let name = meta_name(obj).to_owned();

        let min_available = obj
            .pointer("/spec/minAvailable")
            .map(|v| v.to_string().trim_matches('"').to_owned())
            .unwrap_or_else(|| "N/A".to_owned());

        let max_unavailable = obj
            .pointer("/spec/maxUnavailable")
            .map(|v| v.to_string().trim_matches('"').to_owned())
            .unwrap_or_else(|| "N/A".to_owned());

        let allowed = obj
            .pointer("/status/disruptionsAllowed")
            .and_then(|v| v.as_u64())
            .map(|n| n.to_string())
            .unwrap_or_else(|| "0".to_owned());

        let (age, age_secs) = age_from_obj(obj);

        RenderedRow {
            cells: vec![name, min_available, max_unavailable, allowed, age],
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
        assert_eq!(PdbRenderer::new().columns().len(), 5);
    }

    #[test]
    fn render_pdb() {
        let obj = json!({
            "metadata": { "name": "web-pdb", "creationTimestamp": "2026-01-01T00:00:00Z" },
            "spec": { "minAvailable": 1, "maxUnavailable": null },
            "status": { "disruptionsAllowed": 2 },
        });
        let row = PdbRenderer::new().render(&obj);
        assert_eq!(row.cells[0], "web-pdb");
        assert_eq!(row.cells[1], "1");
        assert_eq!(row.cells[3], "2");
    }
}
