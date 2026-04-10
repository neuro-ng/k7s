//! Renderer for ConfigMaps (v1/configmaps).
//!
//! Shows key names only — values are never displayed (security boundary).

use ratatui::layout::Constraint;
use serde_json::Value;

use crate::client::gvr::well_known;
use crate::client::Gvr;
use crate::render::{age_from_obj, meta_name, meta_namespace, ColumnDef, RenderedRow, Renderer};

pub struct ConfigMapRenderer {
    gvr: Gvr,
    columns: Vec<ColumnDef>,
}

impl ConfigMapRenderer {
    pub fn new() -> Self {
        Self {
            gvr: well_known::config_maps(),
            columns: vec![
                ColumnDef::new("NAMESPACE", Constraint::Length(18)),
                ColumnDef::new("NAME", Constraint::Min(28)),
                ColumnDef::new("KEYS", Constraint::Length(6)),
                ColumnDef::new("KEY NAMES", Constraint::Min(40)),
                ColumnDef::new("AGE", Constraint::Length(6)),
            ],
        }
    }
}

impl Default for ConfigMapRenderer {
    fn default() -> Self {
        Self::new()
    }
}

impl Renderer for ConfigMapRenderer {
    fn gvr(&self) -> &Gvr {
        &self.gvr
    }
    fn columns(&self) -> &[ColumnDef] {
        &self.columns
    }

    fn render(&self, obj: &Value) -> RenderedRow {
        let namespace = meta_namespace(obj).to_owned();
        let name = meta_name(obj).to_owned();

        let (key_count, key_names) = extract_keys(obj);

        let (age, age_secs) = age_from_obj(obj);

        RenderedRow {
            cells: vec![namespace, name, key_count.to_string(), key_names, age],
            age_secs,
        }
    }
}

/// Extract key names from `data` and `binaryData` fields.
/// Values are intentionally never accessed.
fn extract_keys(obj: &Value) -> (usize, String) {
    let mut keys: Vec<String> = Vec::new();

    if let Some(data) = obj.get("data").and_then(|v| v.as_object()) {
        for k in data.keys() {
            keys.push(k.clone());
        }
    }
    if let Some(binary) = obj.get("binaryData").and_then(|v| v.as_object()) {
        for k in binary.keys() {
            if !keys.contains(k) {
                keys.push(k.clone());
            }
        }
    }

    keys.sort();
    let count = keys.len();

    // Truncate the display list if there are many keys.
    const MAX_DISPLAY: usize = 5;
    let display = if keys.len() > MAX_DISPLAY {
        let shown = keys[..MAX_DISPLAY].join(", ");
        format!("{shown}, +{}", keys.len() - MAX_DISPLAY)
    } else {
        keys.join(", ")
    };

    (count, display)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn render_configmap_keys_only() {
        let obj = json!({
            "metadata": { "name": "app-config", "namespace": "prod" },
            "data": {
                "database_url": "postgres://secret",
                "log_level": "info"
            }
        });
        let r = ConfigMapRenderer::new().render(&obj);
        assert_eq!(r.cells[0], "prod");
        assert_eq!(r.cells[1], "app-config");
        assert_eq!(r.cells[2], "2"); // key count
        assert!(r.cells[3].contains("database_url"), "key name must appear");
        assert!(r.cells[3].contains("log_level"), "key name must appear");
        // Values must NOT appear.
        assert!(
            !r.cells[3].contains("postgres://secret"),
            "values must not leak"
        );
        assert!(!r.cells[3].contains("info"), "values must not leak");
    }

    #[test]
    fn empty_configmap_shows_zero() {
        let obj = json!({ "metadata": { "name": "empty" } });
        let r = ConfigMapRenderer::new().render(&obj);
        assert_eq!(r.cells[2], "0");
        assert_eq!(r.cells[3], "");
    }

    #[test]
    fn many_keys_are_truncated() {
        let mut data = serde_json::Map::new();
        for i in 0..10 {
            data.insert(format!("key{i}"), Value::String("value".to_owned()));
        }
        let obj = json!({ "metadata": { "name": "big" }, "data": data });
        let r = ConfigMapRenderer::new().render(&obj);
        assert_eq!(r.cells[2], "10");
        assert!(
            r.cells[3].contains("+5"),
            "should truncate with +N: {}",
            r.cells[3]
        );
    }
}
