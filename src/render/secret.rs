//! Renderer for Secrets (v1/secrets).
//!
//! Shows secret type and the count of keys — **no values, no base64 data**.
//! This is a strict security boundary: the actual secret data never reaches
//! the display layer.

use ratatui::layout::Constraint;
use serde_json::Value;

use crate::client::gvr::well_known;
use crate::client::Gvr;
use crate::render::{age_from_obj, meta_name, meta_namespace, ColumnDef, RenderedRow, Renderer};

pub struct SecretRenderer {
    gvr: Gvr,
    columns: Vec<ColumnDef>,
}

impl SecretRenderer {
    pub fn new() -> Self {
        Self {
            gvr: well_known::secrets(),
            columns: vec![
                ColumnDef::new("NAMESPACE", Constraint::Length(18)),
                ColumnDef::new("NAME", Constraint::Min(30)),
                ColumnDef::new("TYPE", Constraint::Min(28)),
                ColumnDef::new("KEYS", Constraint::Length(6)),
                ColumnDef::new("AGE", Constraint::Length(6)),
            ],
        }
    }
}

impl Default for SecretRenderer {
    fn default() -> Self {
        Self::new()
    }
}

impl Renderer for SecretRenderer {
    fn gvr(&self) -> &Gvr {
        &self.gvr
    }
    fn columns(&self) -> &[ColumnDef] {
        &self.columns
    }

    fn render(&self, obj: &Value) -> RenderedRow {
        let namespace = meta_namespace(obj).to_owned();
        let name = meta_name(obj).to_owned();

        let secret_type = obj
            .get("type")
            .and_then(|v| v.as_str())
            .unwrap_or("Opaque")
            .to_owned();

        // Count keys across `data` and `stringData` — never read values.
        let key_count = count_secret_keys(obj);

        let (age, age_secs) = age_from_obj(obj);

        RenderedRow {
            cells: vec![namespace, name, secret_type, key_count.to_string(), age],
            age_secs,
        }
    }
}

/// Count keys in `data` + `stringData` without touching any values.
fn count_secret_keys(obj: &Value) -> usize {
    let mut count = 0;
    if let Some(data) = obj.get("data").and_then(|v| v.as_object()) {
        count += data.len();
    }
    if let Some(sd) = obj.get("stringData").and_then(|v| v.as_object()) {
        count += sd.len();
    }
    count
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn render_opaque_secret() {
        let obj = json!({
            "metadata": { "name": "db-creds", "namespace": "prod" },
            "type": "Opaque",
            "data": {
                "username": "YWRtaW4=",
                "password": "c2VjcmV0"
            }
        });
        let r = SecretRenderer::new().render(&obj);
        assert_eq!(r.cells[0], "prod");
        assert_eq!(r.cells[1], "db-creds");
        assert_eq!(r.cells[2], "Opaque");
        assert_eq!(r.cells[3], "2");
        // Values must NOT appear anywhere.
        let all = r.cells.join(" ");
        assert!(!all.contains("YWRtaW4="), "base64 value must not leak");
        assert!(!all.contains("c2VjcmV0"), "base64 value must not leak");
        assert!(!all.contains("admin"), "decoded value must not leak");
        assert!(!all.contains("secret"), "decoded value must not leak");
    }

    #[test]
    fn render_tls_secret() {
        let obj = json!({
            "metadata": { "name": "my-tls" },
            "type": "kubernetes.io/tls",
            "data": { "tls.crt": "...", "tls.key": "..." }
        });
        let r = SecretRenderer::new().render(&obj);
        assert_eq!(r.cells[2], "kubernetes.io/tls");
        assert_eq!(r.cells[3], "2");
    }

    #[test]
    fn render_empty_secret() {
        let obj = json!({ "metadata": { "name": "empty" } });
        let r = SecretRenderer::new().render(&obj);
        assert_eq!(r.cells[2], "Opaque");
        assert_eq!(r.cells[3], "0");
    }
}
