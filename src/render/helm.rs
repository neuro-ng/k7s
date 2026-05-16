//! Helm release renderer — Phase 10.5 / Phase 35.
//!
//! Converts `serde_json::Value` representations of `HelmRelease` and
//! `HelmHistoryEntry` into `RenderedRow` values suitable for the k7s table
//! widget.
//!
//! Helm releases are not native Kubernetes resources so these renderers are
//! standalone — they do NOT implement the `Renderer` trait (which requires a
//! `Gvr`).
//!
//! Release columns:  NAME · NAMESPACE · REVISION · STATUS · CHART · APP VERSION · UPDATED
//! History columns:  REV · STATUS · CHART · APP VERSION · UPDATED · DESCRIPTION

use serde_json::Value;

use crate::render::RenderedRow;

/// Column header names for the Helm release table.
pub fn headers() -> Vec<&'static str> {
    vec![
        "NAME",
        "NAMESPACE",
        "REVISION",
        "STATUS",
        "CHART",
        "APP VERSION",
        "UPDATED",
    ]
}

/// Render a single Helm release value into a table row.
pub fn render(obj: &Value) -> RenderedRow {
    let name = str_field(obj, "name");
    let namespace = str_field(obj, "namespace");
    let revision = str_field(obj, "revision");
    let status = str_field(obj, "status");
    let chart = str_field(obj, "chart");
    let app_version = str_field(obj, "app_version");
    let updated = format_updated(str_field(obj, "updated"));

    RenderedRow {
        cells: vec![
            name,
            namespace,
            revision,
            status,
            chart,
            app_version,
            updated,
        ],
        age_secs: 0,
    }
}

// ─── Helpers ─────────────────────────────────────────────────────────────────

fn str_field(obj: &Value, key: &str) -> String {
    obj.get(key)
        .and_then(|v| v.as_str())
        .unwrap_or("<none>")
        .to_string()
}

/// Truncate the Helm timestamp to a short form: `2026-04-10 12:00:00.999`.
fn format_updated(raw: String) -> String {
    // Helm format: "2026-04-10 12:00:00.123456789 +0000 UTC"
    // Keep the first two space-separated tokens.
    let parts: Vec<&str> = raw.splitn(3, ' ').collect();
    if parts.len() >= 2 {
        format!("{} {}", parts[0], parts[1])
    } else {
        raw
    }
}

// ─── History renderer ─────────────────────────────────────────────────────────

/// Column header names for the Helm history table.
pub fn history_headers() -> Vec<&'static str> {
    vec![
        "REV",
        "STATUS",
        "CHART",
        "APP VERSION",
        "UPDATED",
        "DESCRIPTION",
    ]
}

/// Render a single `HelmHistoryEntry` value into a table row.
///
/// Expected JSON keys: `revision` (u64), `status`, `chart`, `app_version`,
/// `updated`, `description`.
pub fn render_history(obj: &Value) -> RenderedRow {
    let revision = obj
        .get("revision")
        .and_then(|v| v.as_u64())
        .map(|n| n.to_string())
        .unwrap_or_else(|| "<none>".into());
    let status = str_field(obj, "status");
    let chart = str_field(obj, "chart");
    let app_version = str_field(obj, "app_version");
    let updated = format_updated(str_field(obj, "updated"));
    let description = str_field(obj, "description");

    RenderedRow {
        cells: vec![revision, status, chart, app_version, updated, description],
        age_secs: 0,
    }
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn make_release(status: &str) -> Value {
        json!({
            "name":        "my-app",
            "namespace":   "production",
            "revision":    "3",
            "updated":     "2026-04-10 12:00:00.000000000 +0000 UTC",
            "status":      status,
            "chart":       "my-app-1.2.3",
            "app_version": "1.2.3"
        })
    }

    #[test]
    fn headers_count() {
        assert_eq!(headers().len(), 7);
    }

    #[test]
    fn render_deployed() {
        let row = render(&make_release("deployed"));
        assert_eq!(row.cells[0], "my-app");
        assert_eq!(row.cells[3], "deployed");
    }

    #[test]
    fn format_updated_truncates() {
        let ts = "2026-04-10 12:00:00.999 +0000 UTC".to_string();
        assert_eq!(format_updated(ts), "2026-04-10 12:00:00.999");
    }

    #[test]
    fn missing_field_shows_none() {
        let row = render(&json!({}));
        assert!(row.cells.iter().all(|c| c == "<none>"));
    }

    #[test]
    fn history_headers_count() {
        assert_eq!(history_headers().len(), 6);
    }

    #[test]
    fn render_history_basic() {
        let obj = json!({
            "revision": 2,
            "status": "superseded",
            "chart": "my-app-1.2.2",
            "app_version": "1.2.2",
            "updated": "2026-04-09 10:00:00.000 +0000 UTC",
            "description": "Upgrade complete"
        });
        let row = render_history(&obj);
        assert_eq!(row.cells[0], "2");
        assert_eq!(row.cells[1], "superseded");
        assert_eq!(row.cells[5], "Upgrade complete");
    }

    #[test]
    fn render_history_missing_revision_shows_none() {
        let row = render_history(&json!({}));
        assert_eq!(row.cells[0], "<none>");
    }

    #[test]
    fn render_history_updated_truncated() {
        let obj = json!({
            "revision": 1,
            "status": "deployed",
            "chart": "app-1.0",
            "app_version": "1.0",
            "updated": "2026-01-01 08:00:00.000 +0000 UTC",
            "description": "Initial install"
        });
        let row = render_history(&obj);
        assert_eq!(row.cells[4], "2026-01-01 08:00:00.000");
    }
}
