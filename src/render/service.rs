use ratatui::layout::Constraint;
use serde_json::Value;

use crate::client::Gvr;
use crate::client::gvr::well_known;
use crate::render::{age_from_obj, meta_name, ColumnDef, RenderedRow, Renderer};

pub struct ServiceRenderer {
    gvr: Gvr,
    columns: Vec<ColumnDef>,
}

impl ServiceRenderer {
    pub fn new() -> Self {
        Self {
            gvr: well_known::services(),
            columns: vec![
                ColumnDef::new("NAME",        Constraint::Min(20)),
                ColumnDef::new("TYPE",        Constraint::Length(12)),
                ColumnDef::new("CLUSTER-IP",  Constraint::Length(16)),
                ColumnDef::new("EXTERNAL-IP", Constraint::Length(16)),
                ColumnDef::new("PORT(S)",     Constraint::Min(14)),
                ColumnDef::new("AGE",         Constraint::Length(6)),
            ],
        }
    }
}

impl Default for ServiceRenderer {
    fn default() -> Self { Self::new() }
}

impl Renderer for ServiceRenderer {
    fn gvr(&self) -> &Gvr { &self.gvr }
    fn columns(&self) -> &[ColumnDef] { &self.columns }

    fn render(&self, obj: &Value) -> RenderedRow {
        let name = meta_name(obj).to_owned();
        let svc_type = obj.pointer("/spec/type")
            .and_then(|v| v.as_str())
            .unwrap_or("ClusterIP")
            .to_owned();
        let cluster_ip = obj.pointer("/spec/clusterIP")
            .and_then(|v| v.as_str())
            .unwrap_or("None")
            .to_owned();
        let external_ip = external_ips(obj);
        let ports = service_ports(obj);
        let (age, age_secs) = age_from_obj(obj);

        RenderedRow {
            cells: vec![name, svc_type, cluster_ip, external_ip, ports, age],
            age_secs,
        }
    }
}

fn external_ips(obj: &Value) -> String {
    // LoadBalancer ingress IPs/hostnames.
    if let Some(ingress) = obj.pointer("/status/loadBalancer/ingress").and_then(|v| v.as_array()) {
        let ips: Vec<&str> = ingress
            .iter()
            .filter_map(|i| {
                i.get("ip").or_else(|| i.get("hostname"))
                    .and_then(|v| v.as_str())
            })
            .collect();
        if !ips.is_empty() {
            return ips.join(",");
        }
    }
    // ExternalIPs field.
    if let Some(ext) = obj.pointer("/spec/externalIPs").and_then(|v| v.as_array()) {
        let ips: Vec<&str> = ext.iter().filter_map(|v| v.as_str()).collect();
        if !ips.is_empty() {
            return ips.join(",");
        }
    }
    "<none>".to_owned()
}

fn service_ports(obj: &Value) -> String {
    let ports = obj.pointer("/spec/ports").and_then(|v| v.as_array());
    match ports {
        None => "<none>".to_owned(),
        Some(arr) => {
            let formatted: Vec<String> = arr
                .iter()
                .map(|p| {
                    let port = p.get("port").and_then(|v| v.as_i64()).unwrap_or(0);
                    let protocol = p.get("protocol").and_then(|v| v.as_str()).unwrap_or("TCP");
                    let node_port = p.get("nodePort").and_then(|v| v.as_i64());
                    if let Some(np) = node_port {
                        format!("{port}:{np}/{protocol}")
                    } else {
                        format!("{port}/{protocol}")
                    }
                })
                .collect();
            formatted.join(",")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn render_clusterip_service() {
        let obj = json!({
            "metadata": { "name": "my-svc" },
            "spec": {
                "type": "ClusterIP",
                "clusterIP": "10.96.0.1",
                "ports": [{ "port": 80, "protocol": "TCP" }]
            }
        });
        let r = ServiceRenderer::new().render(&obj);
        assert_eq!(r.cells[0], "my-svc");
        assert_eq!(r.cells[1], "ClusterIP");
        assert_eq!(r.cells[2], "10.96.0.1");
        assert_eq!(r.cells[3], "<none>");
        assert_eq!(r.cells[4], "80/TCP");
    }

    #[test]
    fn render_loadbalancer_service() {
        let obj = json!({
            "metadata": { "name": "lb-svc" },
            "spec": {
                "type": "LoadBalancer",
                "clusterIP": "10.96.0.2",
                "ports": [{ "port": 443, "nodePort": 30443, "protocol": "TCP" }]
            },
            "status": {
                "loadBalancer": {
                    "ingress": [{ "ip": "1.2.3.4" }]
                }
            }
        });
        let r = ServiceRenderer::new().render(&obj);
        assert_eq!(r.cells[3], "1.2.3.4");
        assert_eq!(r.cells[4], "443:30443/TCP");
    }
}
