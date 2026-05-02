//! HorizontalPodAutoscaler renderer — Phase 28.
//!
//! Columns: NAME · REFERENCE · TARGETS · MINPODS · MAXPODS · REPLICAS · AGE

use ratatui::layout::Constraint;
use serde_json::Value;

use crate::client::gvr::well_known;
use crate::client::Gvr;
use crate::render::{age_from_obj, meta_name, ColumnDef, RenderedRow, Renderer};

pub struct HpaRenderer {
    gvr: Gvr,
    columns: Vec<ColumnDef>,
}

impl HpaRenderer {
    pub fn new() -> Self {
        Self {
            gvr: well_known::horizontal_pod_autoscalers(),
            columns: vec![
                ColumnDef::new("NAME", Constraint::Min(20)),
                ColumnDef::new("REFERENCE", Constraint::Min(22)),
                ColumnDef::new("TARGETS", Constraint::Length(16)),
                ColumnDef::new("MINPODS", Constraint::Length(8)),
                ColumnDef::new("MAXPODS", Constraint::Length(8)),
                ColumnDef::new("REPLICAS", Constraint::Length(9)),
                ColumnDef::new("AGE", Constraint::Length(6)),
            ],
        }
    }
}

impl Default for HpaRenderer {
    fn default() -> Self {
        Self::new()
    }
}

impl Renderer for HpaRenderer {
    fn gvr(&self) -> &Gvr {
        &self.gvr
    }

    fn columns(&self) -> &[ColumnDef] {
        &self.columns
    }

    fn render(&self, obj: &Value) -> RenderedRow {
        let name = meta_name(obj).to_owned();

        let ref_kind = obj
            .pointer("/spec/scaleTargetRef/kind")
            .and_then(|v| v.as_str())
            .unwrap_or("Unknown");
        let ref_name = obj
            .pointer("/spec/scaleTargetRef/name")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let reference = format!("{ref_kind}/{ref_name}");

        // Current / desired CPU target from status.currentMetrics if available.
        let targets = obj
            .pointer("/status/currentMetrics")
            .and_then(|v| v.as_array())
            .and_then(|arr| arr.first())
            .and_then(|m| {
                let current = m
                    .pointer("/resource/current/averageUtilization")
                    .and_then(|v| v.as_u64())
                    .map(|n| format!("{n}%"))?;
                let desired = obj
                    .pointer("/spec/metrics/0/resource/target/averageUtilization")
                    .and_then(|v| v.as_u64())
                    .map(|n| format!("{n}%"))
                    .unwrap_or_else(|| "<unknown>".to_owned());
                Some(format!("{current}/{desired}"))
            })
            .unwrap_or_else(|| "<unknown>/<unknown>".to_owned());

        let min = obj
            .pointer("/spec/minReplicas")
            .and_then(|v| v.as_u64())
            .map(|n| n.to_string())
            .unwrap_or_else(|| "1".to_owned());
        let max = obj
            .pointer("/spec/maxReplicas")
            .and_then(|v| v.as_u64())
            .map(|n| n.to_string())
            .unwrap_or_default();
        let replicas = obj
            .pointer("/status/currentReplicas")
            .and_then(|v| v.as_u64())
            .map(|n| n.to_string())
            .unwrap_or_else(|| "0".to_owned());

        let (age, age_secs) = age_from_obj(obj);

        RenderedRow {
            cells: vec![name, reference, targets, min, max, replicas, age],
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
        assert_eq!(HpaRenderer::new().columns().len(), 7);
    }

    #[test]
    fn render_basic_hpa() {
        let obj = json!({
            "metadata": { "name": "web-hpa", "creationTimestamp": "2026-01-01T00:00:00Z" },
            "spec": {
                "scaleTargetRef": { "kind": "Deployment", "name": "web" },
                "minReplicas": 2,
                "maxReplicas": 10,
            },
            "status": { "currentReplicas": 3 },
        });
        let row = HpaRenderer::new().render(&obj);
        assert_eq!(row.cells[0], "web-hpa");
        assert_eq!(row.cells[1], "Deployment/web");
        assert_eq!(row.cells[3], "2");
        assert_eq!(row.cells[4], "10");
        assert_eq!(row.cells[5], "3");
    }
}
