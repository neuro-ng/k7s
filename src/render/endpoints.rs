//! Endpoints renderer — Phase 28.
//!
//! Columns: NAME · ENDPOINTS · AGE

use ratatui::layout::Constraint;
use serde_json::Value;

use crate::client::gvr::well_known;
use crate::client::Gvr;
use crate::render::{age_from_obj, meta_name, ColumnDef, RenderedRow, Renderer};

pub struct EndpointsRenderer {
    gvr: Gvr,
    columns: Vec<ColumnDef>,
}

impl EndpointsRenderer {
    pub fn new() -> Self {
        Self {
            gvr: well_known::endpoints(),
            columns: vec![
                ColumnDef::new("NAME", Constraint::Min(20)),
                ColumnDef::new("ENDPOINTS", Constraint::Min(40)),
                ColumnDef::new("AGE", Constraint::Length(6)),
            ],
        }
    }
}

impl Default for EndpointsRenderer {
    fn default() -> Self {
        Self::new()
    }
}

impl Renderer for EndpointsRenderer {
    fn gvr(&self) -> &Gvr {
        &self.gvr
    }

    fn columns(&self) -> &[ColumnDef] {
        &self.columns
    }

    fn render(&self, obj: &Value) -> RenderedRow {
        let name = meta_name(obj).to_owned();

        let endpoints: String = obj
            .pointer("/subsets")
            .and_then(|v| v.as_array())
            .map(|subsets| {
                let mut addrs: Vec<String> = Vec::new();
                for subset in subsets {
                    let ips: Vec<&str> = subset
                        .pointer("/addresses")
                        .and_then(|v| v.as_array())
                        .map(|arr| arr.iter().filter_map(|a| a["ip"].as_str()).collect())
                        .unwrap_or_default();

                    let ports: Vec<u64> = subset
                        .pointer("/ports")
                        .and_then(|v| v.as_array())
                        .map(|arr| arr.iter().filter_map(|p| p["port"].as_u64()).collect())
                        .unwrap_or_default();

                    for ip in &ips {
                        for port in &ports {
                            addrs.push(format!("{ip}:{port}"));
                        }
                    }
                }
                if addrs.is_empty() {
                    "<none>".to_owned()
                } else if addrs.len() > 3 {
                    format!("{},{},… +{}", addrs[0], addrs[1], addrs.len() - 2)
                } else {
                    addrs.join(",")
                }
            })
            .unwrap_or_else(|| "<none>".to_owned());

        let (age, age_secs) = age_from_obj(obj);

        RenderedRow {
            cells: vec![name, endpoints, age],
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
        assert_eq!(EndpointsRenderer::new().columns().len(), 3);
    }

    #[test]
    fn render_endpoints() {
        let obj = json!({
            "metadata": { "name": "my-svc", "creationTimestamp": "2026-01-01T00:00:00Z" },
            "subsets": [{
                "addresses": [{ "ip": "10.0.0.1" }, { "ip": "10.0.0.2" }],
                "ports": [{ "port": 8080 }]
            }]
        });
        let row = EndpointsRenderer::new().render(&obj);
        assert_eq!(row.cells[0], "my-svc");
        assert!(row.cells[1].contains("10.0.0.1:8080"));
        assert!(row.cells[1].contains("10.0.0.2:8080"));
    }

    #[test]
    fn render_no_endpoints() {
        let obj = json!({
            "metadata": { "name": "empty-svc", "creationTimestamp": "2026-01-01T00:00:00Z" },
        });
        let row = EndpointsRenderer::new().render(&obj);
        assert_eq!(row.cells[1], "<none>");
    }
}
