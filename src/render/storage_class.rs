//! StorageClass renderer — Phase 28.
//!
//! Columns: NAME · PROVISIONER · RECLAIM-POLICY · VOLUME-BINDING · ALLOW-EXPAND · AGE

use ratatui::layout::Constraint;
use serde_json::Value;

use crate::client::gvr::well_known;
use crate::client::Gvr;
use crate::render::{age_from_obj, meta_name, ColumnDef, RenderedRow, Renderer};

pub struct StorageClassRenderer {
    gvr: Gvr,
    columns: Vec<ColumnDef>,
}

impl StorageClassRenderer {
    pub fn new() -> Self {
        Self {
            gvr: well_known::storage_classes(),
            columns: vec![
                ColumnDef::new("NAME", Constraint::Min(18)),
                ColumnDef::new("PROVISIONER", Constraint::Min(26)),
                ColumnDef::new("RECLAIM", Constraint::Length(10)),
                ColumnDef::new("BINDING", Constraint::Length(16)),
                ColumnDef::new("EXPAND", Constraint::Length(7)),
                ColumnDef::new("AGE", Constraint::Length(6)),
            ],
        }
    }
}

impl Default for StorageClassRenderer {
    fn default() -> Self {
        Self::new()
    }
}

impl Renderer for StorageClassRenderer {
    fn gvr(&self) -> &Gvr {
        &self.gvr
    }

    fn columns(&self) -> &[ColumnDef] {
        &self.columns
    }

    fn render(&self, obj: &Value) -> RenderedRow {
        let name = meta_name(obj).to_owned();

        let provisioner = obj["provisioner"].as_str().unwrap_or("").to_owned();
        let reclaim = obj["reclaimPolicy"].as_str().unwrap_or("Delete").to_owned();
        let binding = obj["volumeBindingMode"]
            .as_str()
            .unwrap_or("Immediate")
            .to_owned();
        let expand = obj["allowVolumeExpansion"]
            .as_bool()
            .map(|b| if b { "true" } else { "false" })
            .unwrap_or("false")
            .to_owned();

        let (age, age_secs) = age_from_obj(obj);

        RenderedRow {
            cells: vec![name, provisioner, reclaim, binding, expand, age],
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
        assert_eq!(StorageClassRenderer::new().columns().len(), 6);
    }

    #[test]
    fn render_storage_class() {
        let obj = json!({
            "metadata": { "name": "standard", "creationTimestamp": "2026-01-01T00:00:00Z" },
            "provisioner": "kubernetes.io/no-provisioner",
            "reclaimPolicy": "Retain",
            "volumeBindingMode": "WaitForFirstConsumer",
            "allowVolumeExpansion": true,
        });
        let row = StorageClassRenderer::new().render(&obj);
        assert_eq!(row.cells[0], "standard");
        assert_eq!(row.cells[1], "kubernetes.io/no-provisioner");
        assert_eq!(row.cells[2], "Retain");
        assert_eq!(row.cells[3], "WaitForFirstConsumer");
        assert_eq!(row.cells[4], "true");
    }
}
