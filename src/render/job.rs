use ratatui::layout::Constraint;
use serde_json::Value;

use crate::client::Gvr;
use crate::client::gvr::well_known;
use crate::render::{age_from_obj, meta_name, ColumnDef, RenderedRow, Renderer};

pub struct JobRenderer {
    gvr: Gvr,
    columns: Vec<ColumnDef>,
}

impl JobRenderer {
    pub fn new() -> Self {
        Self {
            gvr: well_known::jobs(),
            columns: vec![
                ColumnDef::new("NAME",        Constraint::Min(20)),
                ColumnDef::new("COMPLETIONS", Constraint::Length(12)),
                ColumnDef::new("DURATION",    Constraint::Length(9)),
                ColumnDef::new("AGE",         Constraint::Length(6)),
            ],
        }
    }
}

impl Default for JobRenderer {
    fn default() -> Self { Self::new() }
}

impl Renderer for JobRenderer {
    fn gvr(&self) -> &Gvr { &self.gvr }
    fn columns(&self) -> &[ColumnDef] { &self.columns }

    fn render(&self, obj: &Value) -> RenderedRow {
        let name        = meta_name(obj).to_owned();
        let completions = obj.pointer("/spec/completions").and_then(|v| v.as_i64()).unwrap_or(1);
        let succeeded   = obj.pointer("/status/succeeded").and_then(|v| v.as_i64()).unwrap_or(0);

        // Derive a duration from startTime → completionTime if available.
        let duration = job_duration(obj);
        let (age, age_secs) = age_from_obj(obj);

        RenderedRow {
            cells: vec![
                name,
                format!("{succeeded}/{completions}"),
                duration,
                age,
            ],
            age_secs,
        }
    }
}

/// Calculate the duration a job ran for.
///
/// Returns "-" if not yet complete, or the elapsed time as a compact string.
fn job_duration(obj: &Value) -> String {
    use chrono::DateTime;

    let start = obj
        .pointer("/status/startTime")
        .and_then(|v| v.as_str())
        .and_then(|s| DateTime::parse_from_rfc3339(s).ok());

    let end = obj
        .pointer("/status/completionTime")
        .and_then(|v| v.as_str())
        .and_then(|s| DateTime::parse_from_rfc3339(s).ok());

    match (start, end) {
        (Some(s), Some(e)) => {
            let secs = (e - s).num_seconds().max(0) as u64;
            crate::render::format_duration_secs(secs)
        }
        (Some(s), None) => {
            // Job still running — show elapsed so far.
            let now = chrono::Utc::now();
            let secs = now.signed_duration_since(s.with_timezone(&chrono::Utc))
                .num_seconds()
                .max(0) as u64;
            format!("{}+", crate::render::format_duration_secs(secs))
        }
        _ => "-".to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn render_completed_job() {
        let obj = json!({
            "metadata": {"name": "my-job"},
            "spec": {"completions": 1},
            "status": {"succeeded": 1}
        });
        let r = JobRenderer::new().render(&obj);
        assert_eq!(r.cells[0], "my-job");
        assert_eq!(r.cells[1], "1/1");
    }

    #[test]
    fn render_partial_job() {
        let obj = json!({
            "metadata": {"name": "batch-job"},
            "spec": {"completions": 5},
            "status": {"succeeded": 3}
        });
        let r = JobRenderer::new().render(&obj);
        assert_eq!(r.cells[1], "3/5");
    }
}
