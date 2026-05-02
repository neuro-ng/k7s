//! Port-forward list renderer — Phase 29.
//!
//! Columns: ID · NAMESPACE · POD · PORTS · STATUS

use ratatui::layout::Constraint;
use serde_json::Value;

use crate::client::Gvr;
use crate::render::{ColumnDef, RenderedRow, Renderer};

pub struct PfRenderer {
    gvr: Gvr,
    columns: Vec<ColumnDef>,
}

impl PfRenderer {
    pub fn new() -> Self {
        Self {
            // Synthetic GVR — not a real Kubernetes resource.
            gvr: Gvr::new("", "v1", "portforwards"),
            columns: vec![
                ColumnDef::new("ID", Constraint::Length(8)),
                ColumnDef::new("NAMESPACE", Constraint::Min(14)),
                ColumnDef::new("POD", Constraint::Min(20)),
                ColumnDef::new("PORTS", Constraint::Length(14)),
                ColumnDef::new("STATUS", Constraint::Length(10)),
            ],
        }
    }
}

impl Default for PfRenderer {
    fn default() -> Self {
        Self::new()
    }
}

impl Renderer for PfRenderer {
    fn gvr(&self) -> &Gvr {
        &self.gvr
    }

    fn columns(&self) -> &[ColumnDef] {
        &self.columns
    }

    fn render(&self, obj: &Value) -> RenderedRow {
        let id = obj["id"].as_str().unwrap_or("").to_owned();
        let namespace = obj["namespace"].as_str().unwrap_or("").to_owned();
        let pod = obj["pod"].as_str().unwrap_or("").to_owned();
        let local_port = obj["local_port"].as_u64().unwrap_or(0);
        let pod_port = obj["pod_port"].as_u64().unwrap_or(0);
        let ports = format!("{local_port}→{pod_port}");
        let status = obj["status"].as_str().unwrap_or("").to_owned();

        RenderedRow {
            cells: vec![id, namespace, pod, ports, status],
            age_secs: 0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn columns_count() {
        assert_eq!(PfRenderer::new().columns().len(), 5);
    }

    #[test]
    fn render_running_forward() {
        let obj = json!({
            "id": "pf-1",
            "namespace": "default",
            "pod": "my-api-7f4c9",
            "local_port": 9090,
            "pod_port": 8080,
            "status": "running",
        });
        let row = PfRenderer::new().render(&obj);
        assert_eq!(row.cells[0], "pf-1");
        assert_eq!(row.cells[1], "default");
        assert_eq!(row.cells[2], "my-api-7f4c9");
        assert_eq!(row.cells[3], "9090→8080");
        assert_eq!(row.cells[4], "running");
    }

    #[test]
    fn render_failed_forward() {
        let obj = json!({
            "id": "pf-2",
            "namespace": "prod",
            "pod": "db-0",
            "local_port": 5432,
            "pod_port": 5432,
            "status": "failed",
        });
        let row = PfRenderer::new().render(&obj);
        assert_eq!(row.cells[4], "failed");
    }
}
