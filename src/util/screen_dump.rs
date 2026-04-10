//! Screen dump — Phase 10.2.
//!
//! Saves the current table contents to a plain-text or CSV file on disk.
//! The dump file is written to the k7s state directory by default, or to
//! any path the caller provides.
//!
//! # k9s Reference: `internal/view/screen_dump.go`, `internal/dao/screen_dump.go`

use std::fmt::Write as FmtWrite;
use std::path::{Path, PathBuf};

use crate::render::RenderedRow;

/// A single table snapshot ready to be dumped.
#[derive(Debug, Clone)]
pub struct TableSnapshot {
    /// Column header names (e.g. `["NAME", "READY", "STATUS"]`).
    pub headers: Vec<String>,
    /// All rows at the time of the snapshot.
    pub rows: Vec<RenderedRow>,
    /// Resource type label (e.g. `"pods"`).
    pub resource: String,
    /// Namespace filter active when the dump was taken.
    pub namespace: Option<String>,
}

/// Output format for the dump file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DumpFormat {
    /// Plain-text table with column padding.
    #[default]
    Text,
    /// Comma-separated values (RFC 4180).
    Csv,
}

impl DumpFormat {
    /// Infer format from a file extension.
    pub fn from_path(path: &Path) -> Self {
        match path.extension().and_then(|e| e.to_str()) {
            Some("csv") => DumpFormat::Csv,
            _ => DumpFormat::Text,
        }
    }
}

/// Write a `TableSnapshot` to `path` in the requested format.
///
/// The parent directory is created automatically if it does not exist.
pub fn dump(snapshot: &TableSnapshot, path: &Path, format: DumpFormat) -> anyhow::Result<()> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)?;
        }
    }

    let content = match format {
        DumpFormat::Text => render_text(snapshot),
        DumpFormat::Csv => render_csv(snapshot),
    };

    std::fs::write(path, content)?;
    Ok(())
}

/// Generate a timestamped default dump path inside `state_dir`.
///
/// Example: `~/.local/state/k7s/dumps/pods_2026-04-10T14-32-00.txt`
pub fn default_path(state_dir: &Path, resource: &str, format: DumpFormat) -> PathBuf {
    let ext = match format {
        DumpFormat::Text => "txt",
        DumpFormat::Csv => "csv",
    };
    // Use a filesystem-safe timestamp (replace `:` with `-`).
    let ts = chrono::Local::now().format("%Y-%m-%dT%H-%M-%S").to_string();
    state_dir
        .join("dumps")
        .join(format!("{resource}_{ts}.{ext}"))
}

// ─── Formatters ───────────────────────────────────────────────────────────────

fn render_text(snap: &TableSnapshot) -> String {
    let ncols = snap.headers.len();
    let mut widths: Vec<usize> = snap.headers.iter().map(|h| h.len()).collect();

    for row in &snap.rows {
        for (i, cell) in row.cells.iter().enumerate() {
            if i < ncols {
                widths[i] = widths[i].max(cell.len());
            }
        }
    }

    let mut out = String::new();

    write_padded_row(&mut out, &snap.headers, &widths);
    let sep: String = widths
        .iter()
        .map(|&w| "-".repeat(w))
        .collect::<Vec<_>>()
        .join("  ");
    out.push_str(&sep);
    out.push('\n');

    for row in &snap.rows {
        let cells: Vec<String> = (0..ncols)
            .map(|i| row.cells.get(i).cloned().unwrap_or_default())
            .collect();
        write_padded_row(&mut out, &cells, &widths);
    }

    out
}

fn write_padded_row(out: &mut String, cells: &[String], widths: &[usize]) {
    let parts: Vec<String> = cells
        .iter()
        .enumerate()
        .map(|(i, cell)| {
            let w = widths.get(i).copied().unwrap_or(cell.len());
            format!("{cell:<w$}")
        })
        .collect();
    writeln!(out, "{}", parts.join("  ")).unwrap();
}

fn render_csv(snap: &TableSnapshot) -> String {
    let mut out = String::new();
    writeln!(out, "{}", csv_row(&snap.headers)).unwrap();
    for row in &snap.rows {
        let cells: Vec<String> = (0..snap.headers.len())
            .map(|i| row.cells.get(i).cloned().unwrap_or_default())
            .collect();
        writeln!(out, "{}", csv_row(&cells)).unwrap();
    }
    out
}

fn csv_row(cells: &[String]) -> String {
    cells
        .iter()
        .map(|c| {
            if c.contains(',') || c.contains('"') || c.contains('\n') {
                format!("\"{}\"", c.replace('"', "\"\""))
            } else {
                c.clone()
            }
        })
        .collect::<Vec<_>>()
        .join(",")
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_snapshot() -> TableSnapshot {
        TableSnapshot {
            headers: vec!["NAME".into(), "STATUS".into(), "AGE".into()],
            rows: vec![
                RenderedRow {
                    cells: vec!["nginx".into(), "Running".into(), "5d".into()],
                    age_secs: 0,
                },
                RenderedRow {
                    cells: vec!["redis".into(), "Pending".into(), "1h".into()],
                    age_secs: 0,
                },
            ],
            resource: "pods".into(),
            namespace: Some("default".into()),
        }
    }

    #[test]
    fn text_dump_contains_headers() {
        let snap = sample_snapshot();
        let text = render_text(&snap);
        assert!(text.contains("NAME"));
        assert!(text.contains("nginx"));
        assert!(text.contains("Pending"));
    }

    #[test]
    fn csv_dump_correct_columns() {
        let snap = sample_snapshot();
        let csv = render_csv(&snap);
        let lines: Vec<&str> = csv.lines().collect();
        assert_eq!(lines[0], "NAME,STATUS,AGE");
        assert_eq!(lines[1], "nginx,Running,5d");
    }

    #[test]
    fn csv_escapes_commas() {
        let snap = TableSnapshot {
            headers: vec!["NAME".into()],
            rows: vec![RenderedRow {
                cells: vec!["foo,bar".into()],
                age_secs: 0,
            }],
            resource: "test".into(),
            namespace: None,
        };
        let csv = render_csv(&snap);
        assert!(csv.contains("\"foo,bar\""));
    }

    #[test]
    fn dump_to_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("out.csv");
        let snap = sample_snapshot();
        dump(&snap, &path, DumpFormat::Csv).unwrap();
        let content = std::fs::read_to_string(&path).unwrap();
        assert!(content.starts_with("NAME,STATUS,AGE\n"));
    }

    #[test]
    fn format_from_path() {
        assert_eq!(DumpFormat::from_path(Path::new("out.csv")), DumpFormat::Csv);
        assert_eq!(
            DumpFormat::from_path(Path::new("out.txt")),
            DumpFormat::Text
        );
        assert_eq!(DumpFormat::from_path(Path::new("out")), DumpFormat::Text);
    }

    #[test]
    fn default_path_contains_resource() {
        let path = default_path(Path::new("/state"), "pods", DumpFormat::Text);
        assert!(path.to_str().unwrap().contains("pods"));
        assert_eq!(path.extension().unwrap(), "txt");
    }
}
