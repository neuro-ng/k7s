//! Image vulnerability scanning — Phase 10.4.
//!
//! Wraps the `trivy` CLI to scan container images and parse the JSON output
//! into structured [`VulReport`] / [`VulEntry`] data.  The results are
//! displayed in an interactive TUI table (see [`ImgScanView`]).
//!
//! # Design
//!
//! * Calls `trivy image --format json --quiet <image>` as a child process.
//! * Parses the compact Trivy JSON schema (Results array, Vulnerabilities list).
//! * Severity ordering: Critical > High > Medium > Low > Unknown.
//! * The view sorts entries by severity descending for fast triage.
//! * Trivy must be installed separately; if it is not found the view shows a
//!   clear error message rather than panicking.
//!
//! # Usage (library)
//!
//! ```no_run
//! use k7s::vul::VulnerabilityScanner;
//!
//! #[tokio::main]
//! async fn main() {
//!     let scanner = VulnerabilityScanner::new();
//!     match scanner.scan("nginx:latest").await {
//!         Ok(report) => println!("{} CVEs found", report.entries.len()),
//!         Err(e) => eprintln!("scan failed: {e}"),
//!     }
//! }
//! ```

use std::process::Stdio;
use std::time::SystemTime;

use serde::Deserialize;

// ─── Severity ────────────────────────────────────────────────────────────────

/// CVE severity level (Trivy naming convention).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Default)]
pub enum Severity {
    #[default]
    Unknown,
    Low,
    Medium,
    High,
    Critical,
}

impl Severity {
    /// Short label used in table cells.
    pub fn label(&self) -> &'static str {
        match self {
            Self::Critical => "CRIT",
            Self::High => "HIGH",
            Self::Medium => "MED",
            Self::Low => "LOW",
            Self::Unknown => "UNK",
        }
    }

    /// Terminal colour for the severity badge.
    pub fn color(&self) -> ratatui::style::Color {
        use ratatui::style::Color;
        match self {
            Self::Critical => Color::Red,
            Self::High => Color::LightRed,
            Self::Medium => Color::Yellow,
            Self::Low => Color::LightBlue,
            Self::Unknown => Color::DarkGray,
        }
    }

    fn from_str(s: &str) -> Self {
        match s.to_uppercase().as_str() {
            "CRITICAL" => Self::Critical,
            "HIGH" => Self::High,
            "MEDIUM" => Self::Medium,
            "LOW" => Self::Low,
            _ => Self::Unknown,
        }
    }
}

// ─── Report types ─────────────────────────────────────────────────────────────

/// A single vulnerability finding.
#[derive(Debug, Clone)]
pub struct VulEntry {
    /// CVE identifier (e.g. `CVE-2023-12345`).
    pub id: String,
    /// Affected package name.
    pub pkg: String,
    /// Installed version.
    pub version: String,
    /// Version in which the issue is fixed, if known.
    pub fixed_version: String,
    /// Severity classification.
    pub severity: Severity,
    /// Short description / title.
    pub title: String,
}

/// Aggregated scan result for one image.
#[derive(Debug, Clone, Default)]
pub struct VulReport {
    /// Image reference that was scanned.
    pub image: String,
    /// Wall-clock time when the scan completed.
    pub scan_time: Option<SystemTime>,
    /// All vulnerability findings, sorted by severity descending.
    pub entries: Vec<VulEntry>,
    /// Set when the scan failed (trivy not found, image pull error, etc.).
    pub error: Option<String>,
}

impl VulReport {
    /// Count entries by severity.
    pub fn count_by_severity(&self) -> (usize, usize, usize, usize) {
        let crit = self.entries.iter().filter(|e| e.severity == Severity::Critical).count();
        let high = self.entries.iter().filter(|e| e.severity == Severity::High).count();
        let med  = self.entries.iter().filter(|e| e.severity == Severity::Medium).count();
        let low  = self.entries.iter().filter(|e| e.severity == Severity::Low).count();
        (crit, high, med, low)
    }

    /// One-line summary suitable for flash messages.
    pub fn summary(&self) -> String {
        if let Some(ref e) = self.error {
            return format!("Scan failed: {e}");
        }
        let (c, h, m, l) = self.count_by_severity();
        format!(
            "{}: {} CVEs  CRIT:{c} HIGH:{h} MED:{m} LOW:{l}",
            self.image, self.entries.len()
        )
    }
}

// ─── Trivy JSON schema (internal) ────────────────────────────────────────────

#[derive(Deserialize)]
struct TrivyRoot {
    #[serde(rename = "Results", default)]
    results: Vec<TrivyResult>,
}

#[derive(Deserialize)]
struct TrivyResult {
    #[serde(rename = "Vulnerabilities", default)]
    vulnerabilities: Vec<TrivyVuln>,
}

#[derive(Deserialize)]
struct TrivyVuln {
    #[serde(rename = "VulnerabilityID", default)]
    id: String,
    #[serde(rename = "PkgName", default)]
    pkg: String,
    #[serde(rename = "InstalledVersion", default)]
    version: String,
    #[serde(rename = "FixedVersion", default)]
    fixed_version: String,
    #[serde(rename = "Severity", default)]
    severity: String,
    #[serde(rename = "Title", default)]
    title: String,
}

// ─── Scanner ─────────────────────────────────────────────────────────────────

/// Runs `trivy image` and returns a parsed [`VulReport`].
#[derive(Debug, Clone, Default)]
pub struct VulnerabilityScanner;

impl VulnerabilityScanner {
    pub fn new() -> Self {
        Self
    }

    /// Scan `image` synchronously (blocks until trivy exits).
    ///
    /// Returns `Ok(VulReport)` even when trivy reports zero findings.
    /// Returns `Err` only if the process could not be spawned.
    pub async fn scan(&self, image: &str) -> anyhow::Result<VulReport> {
        let output = tokio::process::Command::new("trivy")
            .args(["image", "--format", "json", "--quiet", image])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .await;

        let output = match output {
            Ok(o) => o,
            Err(e) => {
                let msg = if e.kind() == std::io::ErrorKind::NotFound {
                    "trivy not found — install it from https://github.com/aquasecurity/trivy"
                        .to_owned()
                } else {
                    format!("failed to run trivy: {e}")
                };
                return Ok(VulReport {
                    image: image.to_owned(),
                    scan_time: Some(SystemTime::now()),
                    error: Some(msg),
                    ..Default::default()
                });
            }
        };

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr).to_string();
            let msg = stderr
                .lines()
                .next()
                .unwrap_or("trivy exited with non-zero status")
                .to_owned();
            return Ok(VulReport {
                image: image.to_owned(),
                scan_time: Some(SystemTime::now()),
                error: Some(msg),
                ..Default::default()
            });
        }

        let root: TrivyRoot = serde_json::from_slice(&output.stdout).unwrap_or(TrivyRoot {
            results: vec![],
        });

        let mut entries: Vec<VulEntry> = root
            .results
            .into_iter()
            .flat_map(|r| r.vulnerabilities)
            .map(|v| VulEntry {
                id: v.id,
                pkg: v.pkg,
                version: v.version,
                fixed_version: v.fixed_version,
                severity: Severity::from_str(&v.severity),
                title: v.title,
            })
            .collect();

        // Sort by severity descending (Critical first).
        entries.sort_by(|a, b| b.severity.cmp(&a.severity));

        Ok(VulReport {
            image: image.to_owned(),
            scan_time: Some(SystemTime::now()),
            entries,
            error: None,
        })
    }
}

// ─── TUI view ─────────────────────────────────────────────────────────────────

use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Cell, Paragraph, Row, Table, TableState};
use ratatui::Frame;

/// Action returned by [`ImgScanView::handle_key`].
#[derive(Debug, Clone, PartialEq)]
pub enum ImgScanAction {
    /// User closed the view (Esc / q).
    Close,
    /// No action taken.
    None,
}

/// TUI view that displays a [`VulReport`] as a scrollable table.
pub struct ImgScanView {
    pub report: VulReport,
    table_state: TableState,
}

impl ImgScanView {
    pub fn new(report: VulReport) -> Self {
        let mut table_state = TableState::default();
        if !report.entries.is_empty() {
            table_state.select(Some(0));
        }
        Self { report, table_state }
    }

    /// Replace the current report (e.g. after a fresh scan).
    pub fn update(&mut self, report: VulReport) {
        let sel = if report.entries.is_empty() { None } else { Some(0) };
        self.report = report;
        self.table_state.select(sel);
    }

    pub fn handle_key(&mut self, key: &crossterm::event::KeyEvent) -> ImgScanAction {
        use crossterm::event::KeyCode;
        match key.code {
            KeyCode::Esc | KeyCode::Char('q') => ImgScanAction::Close,
            KeyCode::Down | KeyCode::Char('j') => {
                let len = self.report.entries.len();
                if len > 0 {
                    let next = self.table_state.selected().map(|s| (s + 1).min(len - 1)).unwrap_or(0);
                    self.table_state.select(Some(next));
                }
                ImgScanAction::None
            }
            KeyCode::Up | KeyCode::Char('k') => {
                let next = self.table_state.selected().map(|s| s.saturating_sub(1)).unwrap_or(0);
                self.table_state.select(Some(next));
                ImgScanAction::None
            }
            _ => ImgScanAction::None,
        }
    }

    pub fn render(&mut self, frame: &mut Frame, area: Rect) {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(3), // summary bar
                Constraint::Min(0),    // table
                Constraint::Length(1), // footer hints
            ])
            .split(area);

        self.render_summary(frame, chunks[0]);
        self.render_table(frame, chunks[1]);
        self.render_hints(frame, chunks[2]);
    }

    fn render_summary(&self, frame: &mut Frame, area: Rect) {
        let text = if let Some(ref e) = self.report.error {
            Line::from(vec![
                Span::styled("  Scan error: ", Style::default().fg(Color::Red).add_modifier(Modifier::BOLD)),
                Span::raw(e.as_str()),
            ])
        } else {
            let (c, h, m, l) = self.report.count_by_severity();
            Line::from(vec![
                Span::styled("  ", Style::default()),
                Span::styled(&self.report.image, Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
                Span::raw("  "),
                Span::styled(format!("CRIT:{c}"), Style::default().fg(Color::Red)),
                Span::raw("  "),
                Span::styled(format!("HIGH:{h}"), Style::default().fg(Color::LightRed)),
                Span::raw("  "),
                Span::styled(format!("MED:{m}"), Style::default().fg(Color::Yellow)),
                Span::raw("  "),
                Span::styled(format!("LOW:{l}"), Style::default().fg(Color::LightBlue)),
                Span::raw(format!("  total:{}", self.report.entries.len())),
            ])
        };
        let p = Paragraph::new(text)
            .block(Block::default().title(" Vulnerability Scan ").borders(Borders::ALL));
        frame.render_widget(p, area);
    }

    fn render_table(&mut self, frame: &mut Frame, area: Rect) {
        let header = Row::new(vec![
            Cell::from("SEV").style(Style::default().add_modifier(Modifier::BOLD)),
            Cell::from("CVE ID").style(Style::default().add_modifier(Modifier::BOLD)),
            Cell::from("PACKAGE").style(Style::default().add_modifier(Modifier::BOLD)),
            Cell::from("VERSION").style(Style::default().add_modifier(Modifier::BOLD)),
            Cell::from("FIXED").style(Style::default().add_modifier(Modifier::BOLD)),
            Cell::from("TITLE").style(Style::default().add_modifier(Modifier::BOLD)),
        ])
        .style(Style::default().fg(Color::White))
        .height(1);

        let rows: Vec<Row> = self
            .report
            .entries
            .iter()
            .map(|e| {
                let sev_cell = Cell::from(e.severity.label())
                    .style(Style::default().fg(e.severity.color()).add_modifier(Modifier::BOLD));
                Row::new(vec![
                    sev_cell,
                    Cell::from(e.id.as_str()),
                    Cell::from(e.pkg.as_str()),
                    Cell::from(e.version.as_str()),
                    Cell::from(e.fixed_version.as_str()),
                    Cell::from(e.title.as_str()),
                ])
            })
            .collect();

        let widths = [
            Constraint::Length(5),
            Constraint::Length(18),
            Constraint::Length(20),
            Constraint::Length(14),
            Constraint::Length(14),
            Constraint::Min(20),
        ];

        let table = Table::new(rows, widths)
            .header(header)
            .block(Block::default().borders(Borders::ALL).title(" CVEs "))
            .row_highlight_style(Style::default().bg(Color::DarkGray).add_modifier(Modifier::BOLD))
            .highlight_symbol("▶ ");

        frame.render_stateful_widget(table, area, &mut self.table_state);
    }

    fn render_hints(&self, frame: &mut Frame, area: Rect) {
        let hints = Paragraph::new("  ↑/↓ navigate   q/Esc close")
            .style(Style::default().fg(Color::DarkGray));
        frame.render_widget(hints, area);
    }
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn severity_ordering() {
        assert!(Severity::Critical > Severity::High);
        assert!(Severity::High > Severity::Medium);
        assert!(Severity::Medium > Severity::Low);
        assert!(Severity::Low > Severity::Unknown);
    }

    #[test]
    fn severity_from_str_case_insensitive() {
        assert_eq!(Severity::from_str("critical"), Severity::Critical);
        assert_eq!(Severity::from_str("HIGH"), Severity::High);
        assert_eq!(Severity::from_str("unknown_value"), Severity::Unknown);
    }

    #[test]
    fn vul_report_count_by_severity() {
        let entries = vec![
            VulEntry {
                id: "CVE-1".into(),
                pkg: "openssl".into(),
                version: "1.0".into(),
                fixed_version: "1.1".into(),
                severity: Severity::Critical,
                title: "RCE".into(),
            },
            VulEntry {
                id: "CVE-2".into(),
                pkg: "zlib".into(),
                version: "1.2".into(),
                fixed_version: String::new(),
                severity: Severity::High,
                title: "Overflow".into(),
            },
            VulEntry {
                id: "CVE-3".into(),
                pkg: "curl".into(),
                version: "7.0".into(),
                fixed_version: "7.1".into(),
                severity: Severity::Medium,
                title: "SSRF".into(),
            },
        ];
        let report = VulReport {
            image: "nginx:latest".into(),
            scan_time: None,
            entries,
            error: None,
        };
        let (c, h, m, l) = report.count_by_severity();
        assert_eq!(c, 1);
        assert_eq!(h, 1);
        assert_eq!(m, 1);
        assert_eq!(l, 0);
    }

    #[test]
    fn vul_report_summary_no_error() {
        let report = VulReport {
            image: "alpine:3.18".into(),
            entries: vec![],
            error: None,
            scan_time: None,
        };
        let s = report.summary();
        assert!(s.contains("alpine:3.18"));
        assert!(s.contains("CRIT:0"));
    }

    #[test]
    fn vul_report_summary_with_error() {
        let report = VulReport {
            image: "bad:image".into(),
            error: Some("trivy not found".into()),
            entries: vec![],
            scan_time: None,
        };
        let s = report.summary();
        assert!(s.contains("Scan failed"));
    }

    #[test]
    fn parse_trivy_json() {
        let json = r#"{"Results":[{"Vulnerabilities":[{"VulnerabilityID":"CVE-2021-1234","PkgName":"openssl","InstalledVersion":"1.0","FixedVersion":"1.1","Severity":"HIGH","Title":"Buffer overflow"}]}]}"#;
        let root: TrivyRoot = serde_json::from_str(json).unwrap();
        assert_eq!(root.results.len(), 1);
        let v = &root.results[0].vulnerabilities[0];
        assert_eq!(v.id, "CVE-2021-1234");
        assert_eq!(v.severity, "HIGH");
    }

    #[test]
    fn parse_empty_trivy_json() {
        let json = r#"{"Results":[]}"#;
        let root: TrivyRoot = serde_json::from_str(json).unwrap();
        assert_eq!(root.results.len(), 0);
    }
}
