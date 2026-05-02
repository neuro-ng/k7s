//! EndpointSlice renderer — Phase 28.
//!
//! Columns: NAME · ADDRESSTYPE · PORTS · ENDPOINTS · AGE

use ratatui::layout::Constraint;
use serde_json::Value;

use crate::client::gvr::well_known;
use crate::client::Gvr;
use crate::render::{age_from_obj, meta_name, ColumnDef, RenderedRow, Renderer};

pub struct EndpointSliceRenderer {
    gvr: Gvr,
    columns: Vec<ColumnDef>,
}

impl EndpointSliceRenderer {
    pub fn new() -> Self {
        Self {
            gvr: well_known::endpoint_slices(),
            columns: vec![
                ColumnDef::new("NAME", Constraint::Min(22)),
                ColumnDef::new("ADDRESSTYPE", Constraint::Length(12)),
                ColumnDef::new("PORTS", Constraint::Length(12)),
                ColumnDef::new("ENDPOINTS", Constraint::Min(30)),
                ColumnDef::new("AGE", Constraint::Length(6)),
            ],
        }
    }
}

impl Default for EndpointSliceRenderer {
    fn default() -> Self {
        Self::new()
    }
}

impl Renderer for EndpointSliceRenderer {
    fn gvr(&self) -> &Gvr {
        &self.gvr
    }

    fn columns(&self) -> &[ColumnDef] {
        &self.columns
    }

    fn render(&self, obj: &Value) -> RenderedRow {
        let name = meta_name(obj).to_owned();

        let addr_type = obj["addressType"].as_str().unwrap_or("IPv4").to_owned();

        let ports: String = obj["ports"]
            .as_array()
            .map(|arr| {
                arr.iter()
                    .filter_map(|p| p["port"].as_u64())
                    .map(|p| p.to_string())
                    .collect::<Vec<_>>()
                    .join(",")
            })
            .unwrap_or_else(|| "<unset>".to_owned());

        let endpoints: String = obj["endpoints"]
            .as_array()
            .map(|arr| {
                let addrs: Vec<&str> = arr
                    .iter()
                    .flat_map(|ep| {
                        ep["addresses"]
                            .as_array()
                            .map(|a| a.iter().filter_map(|v| v.as_str()).collect::<Vec<_>>())
                            .unwrap_or_default()
                    })
                    .collect();
                if addrs.len() > 4 {
                    format!("{},{},… +{}", addrs[0], addrs[1], addrs.len() - 2)
                } else {
                    addrs.join(",")
                }
            })
            .unwrap_or_else(|| "<none>".to_owned());

        let (age, age_secs) = age_from_obj(obj);

        RenderedRow {
            cells: vec![name, addr_type, ports, endpoints, age],
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
        assert_eq!(EndpointSliceRenderer::new().columns().len(), 5);
    }

    #[test]
    fn render_endpoint_slice() {
        let obj = json!({
            "metadata": { "name": "my-svc-abc12", "creationTimestamp": "2026-01-01T00:00:00Z" },
            "addressType": "IPv4",
            "ports": [{ "port": 8080 }, { "port": 9090 }],
            "endpoints": [
                { "addresses": ["10.0.0.1", "10.0.0.2"] }
            ],
        });
        let row = EndpointSliceRenderer::new().render(&obj);
        assert_eq!(row.cells[0], "my-svc-abc12");
        assert_eq!(row.cells[1], "IPv4");
        assert_eq!(row.cells[2], "8080,9090");
        assert!(row.cells[3].contains("10.0.0.1"));
    }
}
