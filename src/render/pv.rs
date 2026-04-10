//! Renderers for PersistentVolumes and PersistentVolumeClaims.
//!
//! Both resource types show capacity, access modes, storage class, and phase.
//! Connection strings and mount paths are structural data — no secret material.

use ratatui::layout::Constraint;
use serde_json::Value;

use crate::client::gvr::well_known;
use crate::client::Gvr;
use crate::render::{age_from_obj, meta_name, meta_namespace, ColumnDef, RenderedRow, Renderer};

// ─── PersistentVolume ─────────────────────────────────────────────────────────

pub struct PvRenderer {
    gvr: Gvr,
    columns: Vec<ColumnDef>,
}

impl PvRenderer {
    pub fn new() -> Self {
        Self {
            gvr: well_known::persistent_volumes(),
            columns: vec![
                ColumnDef::new("NAME", Constraint::Min(24)),
                ColumnDef::new("CAPACITY", Constraint::Length(10)),
                ColumnDef::new("ACCESS MODES", Constraint::Length(14)),
                ColumnDef::new("RECLAIM POLICY", Constraint::Length(14)),
                ColumnDef::new("STATUS", Constraint::Length(10)),
                ColumnDef::new("STORAGE CLASS", Constraint::Min(16)),
                ColumnDef::new("AGE", Constraint::Length(6)),
            ],
        }
    }
}

impl Default for PvRenderer {
    fn default() -> Self {
        Self::new()
    }
}

impl Renderer for PvRenderer {
    fn gvr(&self) -> &Gvr {
        &self.gvr
    }
    fn columns(&self) -> &[ColumnDef] {
        &self.columns
    }

    fn render(&self, obj: &Value) -> RenderedRow {
        let name = meta_name(obj).to_owned();
        let capacity = pv_capacity(obj);
        let access_modes = access_modes(obj, "/spec/accessModes");
        let reclaim = obj
            .pointer("/spec/persistentVolumeReclaimPolicy")
            .and_then(|v| v.as_str())
            .unwrap_or("Retain")
            .to_owned();
        let status = obj
            .pointer("/status/phase")
            .and_then(|v| v.as_str())
            .unwrap_or("Unknown")
            .to_owned();
        let storage_class = obj
            .pointer("/spec/storageClassName")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_owned();
        let (age, age_secs) = age_from_obj(obj);

        RenderedRow {
            cells: vec![
                name,
                capacity,
                access_modes,
                reclaim,
                status,
                storage_class,
                age,
            ],
            age_secs,
        }
    }
}

// ─── PersistentVolumeClaim ────────────────────────────────────────────────────

pub struct PvcRenderer {
    gvr: Gvr,
    columns: Vec<ColumnDef>,
}

impl PvcRenderer {
    pub fn new() -> Self {
        Self {
            gvr: well_known::persistent_volume_claims(),
            columns: vec![
                ColumnDef::new("NAMESPACE", Constraint::Length(18)),
                ColumnDef::new("NAME", Constraint::Min(24)),
                ColumnDef::new("STATUS", Constraint::Length(10)),
                ColumnDef::new("VOLUME", Constraint::Min(20)),
                ColumnDef::new("CAPACITY", Constraint::Length(10)),
                ColumnDef::new("ACCESS MODES", Constraint::Length(14)),
                ColumnDef::new("STORAGE CLASS", Constraint::Min(16)),
                ColumnDef::new("AGE", Constraint::Length(6)),
            ],
        }
    }
}

impl Default for PvcRenderer {
    fn default() -> Self {
        Self::new()
    }
}

impl Renderer for PvcRenderer {
    fn gvr(&self) -> &Gvr {
        &self.gvr
    }
    fn columns(&self) -> &[ColumnDef] {
        &self.columns
    }

    fn render(&self, obj: &Value) -> RenderedRow {
        let namespace = meta_namespace(obj).to_owned();
        let name = meta_name(obj).to_owned();
        let status = obj
            .pointer("/status/phase")
            .and_then(|v| v.as_str())
            .unwrap_or("Pending")
            .to_owned();
        let volume = obj
            .pointer("/spec/volumeName")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_owned();
        let capacity = obj
            .pointer("/status/capacity/storage")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_owned();
        let access_modes = access_modes(obj, "/spec/accessModes");
        let storage_class = obj
            .pointer("/spec/storageClassName")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_owned();
        let (age, age_secs) = age_from_obj(obj);

        RenderedRow {
            cells: vec![
                namespace,
                name,
                status,
                volume,
                capacity,
                access_modes,
                storage_class,
                age,
            ],
            age_secs,
        }
    }
}

// ─── Helpers ─────────────────────────────────────────────────────────────────

/// Read capacity from `spec.capacity.storage`.
fn pv_capacity(obj: &Value) -> String {
    obj.pointer("/spec/capacity/storage")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_owned()
}

/// Abbreviate access modes to their short forms (RWO, ROX, RWX, RWOP).
fn access_modes(obj: &Value, path: &str) -> String {
    let modes = match obj.pointer(path).and_then(|v| v.as_array()) {
        Some(m) => m,
        None => return String::new(),
    };
    modes
        .iter()
        .filter_map(|v| v.as_str())
        .map(|m| match m {
            "ReadWriteOnce" => "RWO",
            "ReadOnlyMany" => "ROX",
            "ReadWriteMany" => "RWX",
            "ReadWriteOncePod" => "RWOP",
            other => other,
        })
        .collect::<Vec<_>>()
        .join(",")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn render_pv() {
        let obj = json!({
            "metadata": { "name": "pv-0001" },
            "spec": {
                "capacity": { "storage": "10Gi" },
                "accessModes": ["ReadWriteOnce"],
                "persistentVolumeReclaimPolicy": "Retain",
                "storageClassName": "standard"
            },
            "status": { "phase": "Available" }
        });
        let r = PvRenderer::new().render(&obj);
        assert_eq!(r.cells[0], "pv-0001");
        assert_eq!(r.cells[1], "10Gi");
        assert_eq!(r.cells[2], "RWO");
        assert_eq!(r.cells[3], "Retain");
        assert_eq!(r.cells[4], "Available");
        assert_eq!(r.cells[5], "standard");
    }

    #[test]
    fn render_pvc() {
        let obj = json!({
            "metadata": { "name": "data-0", "namespace": "default" },
            "spec": {
                "volumeName": "pv-0001",
                "accessModes": ["ReadWriteOnce"],
                "storageClassName": "standard"
            },
            "status": {
                "phase": "Bound",
                "capacity": { "storage": "10Gi" }
            }
        });
        let r = PvcRenderer::new().render(&obj);
        assert_eq!(r.cells[0], "default");
        assert_eq!(r.cells[1], "data-0");
        assert_eq!(r.cells[2], "Bound");
        assert_eq!(r.cells[3], "pv-0001");
        assert_eq!(r.cells[4], "10Gi");
        assert_eq!(r.cells[5], "RWO");
    }

    #[test]
    fn access_modes_abbreviations() {
        let obj = json!({
            "metadata": { "name": "pv" },
            "spec": {
                "accessModes": ["ReadWriteOnce", "ReadOnlyMany", "ReadWriteMany"]
            }
        });
        let r = PvRenderer::new().render(&obj);
        assert_eq!(r.cells[2], "RWO,ROX,RWX");
    }
}
