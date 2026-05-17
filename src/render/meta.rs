//! Cluster metadata record renderer — Phase 37.
//!
//! Standalone renderer for `MetadataRecord` rows in the `ClusterMetaView`.

use ratatui::style::{Color, Style};

use crate::meta::MetadataRecord;
use crate::render::RenderedRow;

/// Column headers for the metadata record table.
pub fn headers() -> Vec<&'static str> {
    vec!["TIME", "TYPE", "SUMMARY"]
}

/// Render a single `MetadataRecord` into a table row.
pub fn render(record: &MetadataRecord) -> RenderedRow {
    let time = record.timestamp().format("%H:%M:%S").to_string();
    let type_label = record.type_label().to_string();
    let summary = record.summary();

    RenderedRow {
        cells: vec![time, type_label, summary],
        age_secs: 0,
    }
}

/// Render a date-index row (for the left-hand date pane).
pub fn date_headers() -> Vec<&'static str> {
    vec!["DATE", "RECORDS"]
}

/// Colour for a record type label.
pub fn type_color(type_label: &str) -> Style {
    match type_label {
        "snapshot" => Style::default().fg(Color::Blue),
        "issue" => Style::default().fg(Color::Red),
        "interaction" => Style::default().fg(Color::Cyan),
        _ => Style::default().fg(Color::White),
    }
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::meta::record::{
        InteractionAction, InteractionRecord, IssueRecord, NodeSummary, SnapshotRecord,
        WorkloadSummary,
    };
    use ratatui::style::Color;

    fn make_snapshot() -> MetadataRecord {
        MetadataRecord::Snapshot(SnapshotRecord::new(
            NodeSummary { total: 3, ready: 3, not_ready: 0 },
            vec!["default".into()],
            WorkloadSummary { deployments: 2, running: 2, degraded: 0 },
            "1.30.2",
        ))
    }

    fn make_issue() -> MetadataRecord {
        MetadataRecord::Issue(IssueRecord::new(
            "CrashLoopBackOff", "prod", "api", "Pod", "exited 137",
        ))
    }

    fn make_interaction() -> MetadataRecord {
        MetadataRecord::Interaction(InteractionRecord::new(
            InteractionAction::DeletePod,
            "prod", "bad-pod", "Pod", "success",
        ))
    }

    #[test]
    fn headers_count() {
        assert_eq!(headers().len(), 3);
    }

    #[test]
    fn render_snapshot_type_cell() {
        let row = render(&make_snapshot());
        assert_eq!(row.cells[1], "snapshot");
        assert_eq!(row.cells.len(), 3);
    }

    #[test]
    fn render_issue_type_cell() {
        let row = render(&make_issue());
        assert_eq!(row.cells[1], "issue");
        assert!(row.cells[2].contains("CrashLoopBackOff"));
    }

    #[test]
    fn render_interaction_type_cell() {
        let row = render(&make_interaction());
        assert_eq!(row.cells[1], "interaction");
        assert!(row.cells[2].contains("DeletePod"));
    }

    #[test]
    fn type_color_snapshot_is_blue() {
        assert_eq!(type_color("snapshot").fg, Some(Color::Blue));
    }

    #[test]
    fn type_color_issue_is_red() {
        assert_eq!(type_color("issue").fg, Some(Color::Red));
    }

    #[test]
    fn type_color_interaction_is_cyan() {
        assert_eq!(type_color("interaction").fg, Some(Color::Cyan));
    }

    #[test]
    fn time_cell_format() {
        let row = render(&make_snapshot());
        // Time cell should look like HH:MM:SS.
        assert_eq!(row.cells[0].len(), 8);
        assert!(row.cells[0].contains(':'));
    }
}
