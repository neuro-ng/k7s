//! Container renderer — Phase 4.9.
//!
//! Converts a [`ContainerInfo::to_json()`] value into a display row.
//!
//! Column layout:
//! `KIND · NAME · IMAGE · READY · STATE · RESTARTS · CPU-REQ · MEM-REQ`
//!
//! # k9s Reference
//! `internal/render/container.go`

use ratatui::layout::Constraint;
use serde_json::Value;

use crate::client::Gvr;
use crate::render::{ColumnDef, RenderedRow, Renderer};

pub struct ContainerRenderer;

impl ContainerRenderer {
    pub fn new() -> Self {
        Self
    }
}

impl Default for ContainerRenderer {
    fn default() -> Self {
        Self::new()
    }
}

impl Renderer for ContainerRenderer {
    fn gvr(&self) -> &Gvr {
        use std::sync::OnceLock;
        static GVR: OnceLock<Gvr> = OnceLock::new();
        GVR.get_or_init(crate::dao::container::container_gvr)
    }

    fn columns(&self) -> &[ColumnDef] {
        static COLS: &[ColumnDef] = &[
            ColumnDef::new("T", Constraint::Length(2)),
            ColumnDef::new("NAME", Constraint::Min(16)),
            ColumnDef::new("IMAGE", Constraint::Min(28)),
            ColumnDef::new("READY", Constraint::Length(6)),
            ColumnDef::new("STATE", Constraint::Min(12)),
            ColumnDef::new("RESTARTS", Constraint::Length(9)),
            ColumnDef::new("CPU-REQ", Constraint::Length(9)),
            ColumnDef::new("MEM-REQ", Constraint::Length(9)),
        ];
        COLS
    }

    fn render(&self, obj: &Value) -> RenderedRow {
        let kind = obj["kind"].as_str().unwrap_or("").to_owned();
        let name = obj["name"].as_str().unwrap_or("").to_owned();
        let image = obj["image"].as_str().unwrap_or("").to_owned();
        let ready = obj["ready"].as_str().unwrap_or("false").to_owned();
        let state = obj["state"].as_str().unwrap_or("").to_owned();
        let restarts = obj["restarts"].as_u64().unwrap_or(0).to_string();
        let cpu_req = obj["cpu_request"].as_str().unwrap_or("").to_owned();
        let mem_req = obj["mem_request"].as_str().unwrap_or("").to_owned();

        // Shorten image to last component after `/` to keep it readable.
        let short_image = image.rsplit('/').next().unwrap_or(&image).to_owned();

        RenderedRow {
            cells: vec![
                kind,
                name,
                short_image,
                ready,
                state,
                restarts,
                cpu_req,
                mem_req,
            ],
            age_secs: 0,
        }
    }
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn renderer() -> ContainerRenderer {
        ContainerRenderer::new()
    }

    #[test]
    fn columns_count() {
        assert_eq!(renderer().columns().len(), 8);
        assert_eq!(renderer().columns()[0].name, "T");
    }

    #[test]
    fn render_running_container() {
        let obj = json!({
            "kind": "C",
            "name": "app",
            "image": "registry.example.com/myorg/nginx:latest",
            "ready": "true",
            "state": "Running",
            "restarts": 0u64,
            "cpu_request": "100m",
            "mem_request": "128Mi"
        });
        let row = renderer().render(&obj);
        assert_eq!(row.cells[0], "C");
        assert_eq!(row.cells[1], "app");
        // Image should be shortened to last path component.
        assert_eq!(row.cells[2], "nginx:latest");
        assert_eq!(row.cells[3], "true");
        assert_eq!(row.cells[4], "Running");
        assert_eq!(row.cells[5], "0");
        assert_eq!(row.cells[6], "100m");
        assert_eq!(row.cells[7], "128Mi");
    }

    #[test]
    fn render_init_container() {
        let obj = json!({
            "kind": "I",
            "name": "init-db",
            "image": "alpine:3.18",
            "ready": "true",
            "state": "Completed",
            "restarts": 0u64,
            "cpu_request": "",
            "mem_request": ""
        });
        let row = renderer().render(&obj);
        assert_eq!(row.cells[0], "I");
        assert_eq!(row.cells[4], "Completed");
    }

    #[test]
    fn render_missing_fields_gracefully() {
        let row = renderer().render(&json!({}));
        assert_eq!(row.cells.len(), 8);
    }

    #[test]
    fn image_without_slash_unchanged() {
        let obj = json!({
            "kind": "C", "name": "x", "image": "nginx",
            "ready": "true", "state": "Running",
            "restarts": 0u64, "cpu_request": "", "mem_request": ""
        });
        let row = renderer().render(&obj);
        assert_eq!(row.cells[2], "nginx");
    }
}
