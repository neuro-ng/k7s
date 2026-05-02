//! Chat log browser renderer — Phase 26.
//!
//! Column layout: `DATE · CONTEXT · MESSAGES · TOKENS`
//!
//! Used by the `:chats` view to display persisted AI chat sessions.

use ratatui::layout::Constraint;
use serde_json::Value;

use crate::client::Gvr;
use crate::render::{ColumnDef, RenderedRow, Renderer};

pub struct ChatLogRenderer;

impl ChatLogRenderer {
    pub fn new() -> Self {
        Self
    }
}

impl Default for ChatLogRenderer {
    fn default() -> Self {
        Self::new()
    }
}

impl Renderer for ChatLogRenderer {
    fn gvr(&self) -> &Gvr {
        use std::sync::OnceLock;
        static GVR: OnceLock<Gvr> = OnceLock::new();
        GVR.get_or_init(|| Gvr::new("", "v1", "chats"))
    }

    fn columns(&self) -> &[ColumnDef] {
        static COLS: &[ColumnDef] = &[
            ColumnDef::new("DATE", Constraint::Length(17)),
            ColumnDef::new("CONTEXT", Constraint::Min(24)),
            ColumnDef::new("MSGS", Constraint::Length(6)),
            ColumnDef::new("TOKENS", Constraint::Length(8)),
        ];
        COLS
    }

    fn render(&self, obj: &Value) -> RenderedRow {
        let date = obj["date"].as_str().unwrap_or("").to_owned();
        let context = obj["context"].as_str().unwrap_or("(no context)").to_owned();
        let msgs = obj["msgs"].as_str().unwrap_or("0").to_owned();
        let tokens = obj["tokens"].as_str().unwrap_or("0").to_owned();

        RenderedRow {
            cells: vec![date, context, msgs, tokens],
            age_secs: 0,
        }
    }
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn renderer() -> ChatLogRenderer {
        ChatLogRenderer::new()
    }

    #[test]
    fn columns_count() {
        assert_eq!(renderer().columns().len(), 4);
        assert_eq!(renderer().columns()[0].name, "DATE");
    }

    #[test]
    fn render_full_row() {
        let obj = json!({
            "date": "2026-04-27 12:00",
            "context": "pods/web-abc · default",
            "msgs": "4",
            "tokens": "512",
        });
        let row = renderer().render(&obj);
        assert_eq!(row.cells[0], "2026-04-27 12:00");
        assert_eq!(row.cells[1], "pods/web-abc · default");
        assert_eq!(row.cells[2], "4");
        assert_eq!(row.cells[3], "512");
    }

    #[test]
    fn render_missing_fields() {
        let row = renderer().render(&json!({}));
        assert_eq!(row.cells[1], "(no context)");
    }
}
