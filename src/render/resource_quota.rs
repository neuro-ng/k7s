//! ResourceQuota renderer — Phase 28.
//!
//! Columns: NAME · USED · HARD · AGE

use ratatui::layout::Constraint;
use serde_json::Value;

use crate::client::gvr::well_known;
use crate::client::Gvr;
use crate::render::{age_from_obj, meta_name, ColumnDef, RenderedRow, Renderer};

pub struct ResourceQuotaRenderer {
    gvr: Gvr,
    columns: Vec<ColumnDef>,
}

impl ResourceQuotaRenderer {
    pub fn new() -> Self {
        Self {
            gvr: well_known::resource_quotas(),
            columns: vec![
                ColumnDef::new("NAME", Constraint::Min(20)),
                ColumnDef::new("USED-PODS", Constraint::Length(10)),
                ColumnDef::new("HARD-PODS", Constraint::Length(10)),
                ColumnDef::new("CPU-USED", Constraint::Length(10)),
                ColumnDef::new("CPU-HARD", Constraint::Length(10)),
                ColumnDef::new("AGE", Constraint::Length(6)),
            ],
        }
    }
}

impl Default for ResourceQuotaRenderer {
    fn default() -> Self {
        Self::new()
    }
}

impl Renderer for ResourceQuotaRenderer {
    fn gvr(&self) -> &Gvr {
        &self.gvr
    }

    fn columns(&self) -> &[ColumnDef] {
        &self.columns
    }

    fn render(&self, obj: &Value) -> RenderedRow {
        let name = meta_name(obj).to_owned();

        let used_pods = obj
            .pointer("/status/used/pods")
            .and_then(|v| v.as_str())
            .unwrap_or("-")
            .to_owned();
        let hard_pods = obj
            .pointer("/status/hard/pods")
            .and_then(|v| v.as_str())
            .unwrap_or("-")
            .to_owned();
        let cpu_used = obj
            .pointer("/status/used/cpu")
            .or_else(|| obj.pointer("/status/used/requests.cpu"))
            .and_then(|v| v.as_str())
            .unwrap_or("-")
            .to_owned();
        let cpu_hard = obj
            .pointer("/status/hard/cpu")
            .or_else(|| obj.pointer("/status/hard/requests.cpu"))
            .and_then(|v| v.as_str())
            .unwrap_or("-")
            .to_owned();

        let (age, age_secs) = age_from_obj(obj);

        RenderedRow {
            cells: vec![name, used_pods, hard_pods, cpu_used, cpu_hard, age],
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
        assert_eq!(ResourceQuotaRenderer::new().columns().len(), 6);
    }

    #[test]
    fn render_quota() {
        let obj = json!({
            "metadata": { "name": "default-quota", "creationTimestamp": "2026-01-01T00:00:00Z" },
            "status": {
                "used":  { "pods": "3", "cpu": "500m" },
                "hard":  { "pods": "10", "cpu": "4" },
            }
        });
        let row = ResourceQuotaRenderer::new().render(&obj);
        assert_eq!(row.cells[0], "default-quota");
        assert_eq!(row.cells[1], "3");
        assert_eq!(row.cells[2], "10");
        assert_eq!(row.cells[3], "500m");
        assert_eq!(row.cells[4], "4");
    }
}
