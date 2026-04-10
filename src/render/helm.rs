//! Helm release renderer — Phase 10.5.
//!
//! Converts a `serde_json::Value` serialised from a `HelmRelease` into a
//! `RenderedRow` suitable for the k7s table widget.
//!
//! Helm releases are not native Kubernetes resources so this renderer is
//! standalone — it does NOT implement the `Renderer` trait (which requires a
//! `Gvr`).
//!
//! Columns: NAME · NAMESPACE · REVISION · STATUS · CHART · APP VERSION · UPDATED

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
}
