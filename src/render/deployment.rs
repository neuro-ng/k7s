use ratatui::layout::Constraint;
use serde_json::Value;

use crate::client::gvr::well_known;
use crate::client::Gvr;
use crate::render::{age_from_obj, meta_name, ColumnDef, RenderedRow, Renderer};

pub struct DeploymentRenderer {
    gvr: Gvr,
    columns: Vec<ColumnDef>,
}

impl DeploymentRenderer {
    pub fn new() -> Self {
        Self {
            gvr: well_known::deployments(),
            columns: vec![
                ColumnDef::new("NAME", Constraint::Min(20)),
                ColumnDef::new("READY", Constraint::Length(8)),
                ColumnDef::new("UP-TO-DATE", Constraint::Length(11)),
                ColumnDef::new("AVAILABLE", Constraint::Length(10)),
                ColumnDef::new("AGE", Constraint::Length(6)),
            ],
        }
    }
}

impl Default for DeploymentRenderer {
    fn default() -> Self {
        Self::new()
    }
}

impl Renderer for DeploymentRenderer {
    fn gvr(&self) -> &Gvr {
        &self.gvr
    }
    fn columns(&self) -> &[ColumnDef] {
        &self.columns
    }

    fn render(&self, obj: &Value) -> RenderedRow {
        let name = meta_name(obj).to_owned();
        let desired = obj
            .pointer("/spec/replicas")
            .and_then(|v| v.as_i64())
            .unwrap_or(1);
        let ready = obj
            .pointer("/status/readyReplicas")
            .and_then(|v| v.as_i64())
            .unwrap_or(0);
        let updated = obj
            .pointer("/status/updatedReplicas")
            .and_then(|v| v.as_i64())
            .unwrap_or(0);
        let available = obj
            .pointer("/status/availableReplicas")
            .and_then(|v| v.as_i64())
            .unwrap_or(0);
        let (age, age_secs) = age_from_obj(obj);

        RenderedRow {
            cells: vec![
                name,
                format!("{ready}/{desired}"),
                updated.to_string(),
                available.to_string(),
                age,
            ],
            age_secs,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn render_healthy_deployment() {
        let obj = json!({
            "metadata": { "name": "my-deploy", "creationTimestamp": "2020-01-01T00:00:00Z" },
            "spec": { "replicas": 3 },
            "status": {
                "readyReplicas": 3,
                "updatedReplicas": 3,
                "availableReplicas": 3
            }
        });
        let r = DeploymentRenderer::new().render(&obj);
        assert_eq!(r.cells[0], "my-deploy");
        assert_eq!(r.cells[1], "3/3");
        assert_eq!(r.cells[2], "3");
        assert_eq!(r.cells[3], "3");
    }

    #[test]
    fn render_degraded_deployment() {
        let obj = json!({
            "metadata": { "name": "bad-deploy" },
            "spec": { "replicas": 5 },
            "status": { "readyReplicas": 2, "updatedReplicas": 5, "availableReplicas": 2 }
        });
        let r = DeploymentRenderer::new().render(&obj);
        assert_eq!(r.cells[1], "2/5");
    }
}
