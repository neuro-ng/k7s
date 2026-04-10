use ratatui::layout::Constraint;
use serde_json::Value;

use crate::client::Gvr;
use crate::client::gvr::well_known;
use crate::render::{age_from_obj, meta_name, ColumnDef, RenderedRow, Renderer};

pub struct CronJobRenderer {
    gvr: Gvr,
    columns: Vec<ColumnDef>,
}

impl CronJobRenderer {
    pub fn new() -> Self {
        Self {
            gvr: well_known::cron_jobs(),
            columns: vec![
                ColumnDef::new("NAME",         Constraint::Min(20)),
                ColumnDef::new("SCHEDULE",     Constraint::Length(16)),
                ColumnDef::new("SUSPEND",      Constraint::Length(8)),
                ColumnDef::new("ACTIVE",       Constraint::Length(7)),
                ColumnDef::new("LAST SCHEDULE",Constraint::Length(14)),
                ColumnDef::new("AGE",          Constraint::Length(6)),
            ],
        }
    }
}

impl Default for CronJobRenderer {
    fn default() -> Self { Self::new() }
}

impl Renderer for CronJobRenderer {
    fn gvr(&self) -> &Gvr { &self.gvr }
    fn columns(&self) -> &[ColumnDef] { &self.columns }

    fn render(&self, obj: &Value) -> RenderedRow {
        let name     = meta_name(obj).to_owned();
        let schedule = obj.pointer("/spec/schedule").and_then(|v| v.as_str()).unwrap_or("-").to_owned();
        let suspend  = obj.pointer("/spec/suspend").and_then(|v| v.as_bool()).unwrap_or(false);
        let active   = obj.pointer("/status/active")
            .and_then(|v| v.as_array())
            .map_or(0, |a| a.len());
        let last_schedule = last_schedule_age(obj);
        let (age, age_secs) = age_from_obj(obj);

        RenderedRow {
            cells: vec![
                name,
                schedule,
                if suspend { "true" } else { "false" }.to_owned(),
                active.to_string(),
                last_schedule,
                age,
            ],
            age_secs,
        }
    }
}

/// Format time since last schedule as a compact age string.
fn last_schedule_age(obj: &Value) -> String {
    use chrono::DateTime;

    obj.pointer("/status/lastScheduleTime")
        .and_then(|v| v.as_str())
        .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
        .map(|dt| {
            let now = chrono::Utc::now();
            let secs = now
                .signed_duration_since(dt.with_timezone(&chrono::Utc))
                .num_seconds()
                .max(0) as u64;
            crate::render::format_duration_secs(secs)
        })
        .unwrap_or_else(|| "<none>".to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn render_active_cronjob() {
        let obj = json!({
            "metadata": {"name": "my-cj"},
            "spec": {"schedule": "*/5 * * * *", "suspend": false},
            "status": {"active": [{}]}
        });
        let r = CronJobRenderer::new().render(&obj);
        assert_eq!(r.cells[0], "my-cj");
        assert_eq!(r.cells[1], "*/5 * * * *");
        assert_eq!(r.cells[2], "false");
        assert_eq!(r.cells[3], "1");
    }

    #[test]
    fn render_suspended_cronjob() {
        let obj = json!({
            "metadata": {"name": "paused"},
            "spec": {"schedule": "0 1 * * *", "suspend": true},
            "status": {}
        });
        let r = CronJobRenderer::new().render(&obj);
        assert_eq!(r.cells[2], "true");
    }
}
