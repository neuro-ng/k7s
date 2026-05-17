//! Helm Manager TUI views — Phase 35.
//!
//! A self-contained multi-pane Helm release manager:
//!
//! | Sub-view | Trigger | Description |
//! |----------|---------|-------------|
//! | `Releases` | `:helm` / `:hr` | All releases from `helm list` |
//! | `History`  | `Enter` on release | All revisions for a release |
//! | `Text`     | `v` / `m` / `n`   | Values / manifest / notes (scrollable) |
//!
//! Mutating operations (uninstall, rollback) return `HelmAction` variants so
//! the caller (`App`) can display a confirm dialog before shelling out.

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::layout::Constraint;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::widgets::{
    Block, Borders, Cell, Paragraph, Row, Scrollbar, ScrollbarOrientation, ScrollbarState, Table,
    TableState, Wrap,
};
use ratatui::Frame;

use crate::dao::helm::{HelmDao, HelmHistoryEntry, HelmRelease};

// ─── Sub-view ─────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq)]
enum SubView {
    Releases,
    History,
    /// Reused for values, manifest, and notes — content differs by `text_title`.
    Text,
}

// ─── Actions ──────────────────────────────────────────────────────────────────

/// What the calling `App` should do after a key press.
#[derive(Debug, Clone, PartialEq)]
pub enum HelmAction {
    /// User exited the top-level releases view — close Helm mode.
    Close,
    /// Reload the release list from `helm list`.
    Refresh,
    /// Open AI analysis for the selected release.
    AiAnalyse { name: String, namespace: String },
    /// Uninstall the selected release (caller shows a confirm dialog first).
    Uninstall { name: String, namespace: String },
    /// Rollback to a specific revision (caller shows a confirm dialog first).
    Rollback {
        name: String,
        namespace: String,
        revision: u64,
    },
    /// No action needed.
    None,
}

// ─── HelmView ─────────────────────────────────────────────────────────────────

/// Full Helm manager TUI view with sub-pane navigation.
pub struct HelmView {
    sub: SubView,
    /// All releases loaded from `helm list`.
    releases: Vec<HelmRelease>,
    /// Table selection state for the release browser.
    release_table: TableState,
    /// The name of the release whose history is currently shown.
    history_release: String,
    /// History entries for the currently selected release.
    history: Vec<HelmHistoryEntry>,
    /// Table selection state for the history list.
    history_table: TableState,
    /// Title of the scrollable text pane (values / manifest / notes).
    text_title: String,
    /// Content of the scrollable text pane.
    text_content: String,
    /// Vertical scroll offset for the text pane.
    text_scroll: usize,
    /// Error or transient status shown at the bottom.
    pub status: Option<String>,
}

impl HelmView {
    pub fn new() -> Self {
        Self {
            sub: SubView::Releases,
            releases: Vec::new(),
            release_table: TableState::default(),
            history_release: String::new(),
            history: Vec::new(),
            history_table: TableState::default(),
            text_title: String::new(),
            text_content: String::new(),
            text_scroll: 0,
            status: None,
        }
    }

    /// Load all releases from the `helm` binary.  Call once after construction.
    pub fn load_releases(&mut self, context: Option<&str>) {
        let dao = HelmDao::new(context.map(str::to_string));
        match dao.list(None) {
            Ok(releases) => {
                self.releases = releases;
                if !self.releases.is_empty() && self.release_table.selected().is_none() {
                    self.release_table.select(Some(0));
                }
                self.status = None;
            }
            Err(e) => {
                self.releases.clear();
                self.status = Some(format!("helm list: {e}"));
            }
        }
    }

    /// The currently selected release (if any).
    pub fn selected_release(&self) -> Option<&HelmRelease> {
        self.release_table
            .selected()
            .and_then(|i| self.releases.get(i))
    }

    /// The currently selected history entry (if any).
    fn selected_history_entry(&self) -> Option<&HelmHistoryEntry> {
        self.history_table
            .selected()
            .and_then(|i| self.history.get(i))
    }

    /// Feed a key event.  Returns an `HelmAction` for caller processing.
    pub fn handle_key(&mut self, event: &KeyEvent) -> HelmAction {
        match self.sub {
            SubView::Releases => self.handle_releases_key(event),
            SubView::History => self.handle_history_key(event),
            SubView::Text => self.handle_text_key(event),
        }
    }

    fn handle_releases_key(&mut self, event: &KeyEvent) -> HelmAction {
        match event.code {
            KeyCode::Char('q') | KeyCode::Esc => return HelmAction::Close,

            KeyCode::Up | KeyCode::Char('k') => {
                let i = self.release_table.selected().unwrap_or(0);
                if i > 0 {
                    self.release_table.select(Some(i - 1));
                }
            }
            KeyCode::Down | KeyCode::Char('j') => {
                let cur = self.release_table.selected().unwrap_or(0);
                let next = cur + 1;
                if next < self.releases.len() {
                    self.release_table.select(Some(next));
                }
            }
            KeyCode::Home | KeyCode::Char('g') if !self.releases.is_empty() => {
                self.release_table.select(Some(0));
            }
            KeyCode::End | KeyCode::Char('G') if !self.releases.is_empty() => {
                self.release_table.select(Some(self.releases.len() - 1));
            }

            // Enter — open history view.
            KeyCode::Enter => {
                if let Some(r) = self.selected_release() {
                    let name = r.name.clone();
                    let ns = r.namespace.clone();
                    let dao = HelmDao::new(None);
                    match dao.history(&name, &ns) {
                        Ok(h) => {
                            self.history_release = name;
                            self.history = h;
                            self.history_table = TableState::default();
                            if !self.history.is_empty() {
                                // Select latest revision.
                                self.history_table.select(Some(self.history.len() - 1));
                            }
                            self.sub = SubView::History;
                            self.status = None;
                        }
                        Err(e) => {
                            self.status = Some(format!("helm history: {e}"));
                        }
                    }
                }
            }

            // v — values view.
            KeyCode::Char('v') => {
                if let Some(r) = self.selected_release() {
                    let name = r.name.clone();
                    let ns = r.namespace.clone();
                    let content = run_helm_get("values", &name, &ns, &["--output", "yaml"]);
                    let sanitized = sanitize_text_output(content);
                    self.open_text(format!(" Helm Values — {name} · {ns} "), sanitized);
                }
            }

            // m — manifest view.
            KeyCode::Char('m') => {
                if let Some(r) = self.selected_release() {
                    let name = r.name.clone();
                    let ns = r.namespace.clone();
                    let content = run_helm_get("manifest", &name, &ns, &[]);
                    self.open_text(format!(" Helm Manifest — {name} · {ns} "), content);
                }
            }

            // n — notes view.
            KeyCode::Char('n') => {
                if let Some(r) = self.selected_release() {
                    let name = r.name.clone();
                    let ns = r.namespace.clone();
                    let content = run_helm_get("notes", &name, &ns, &[]);
                    self.open_text(format!(" Helm Notes — {name} · {ns} "), content);
                }
            }

            // r / F5 — refresh.
            KeyCode::Char('r') | KeyCode::F(5) => return HelmAction::Refresh,

            // D — uninstall (caller confirms).
            KeyCode::Char('D') => {
                if let Some(r) = self.selected_release() {
                    return HelmAction::Uninstall {
                        name: r.name.clone(),
                        namespace: r.namespace.clone(),
                    };
                }
            }

            // A — AI analyse.
            KeyCode::Char('A') => {
                if let Some(r) = self.selected_release() {
                    return HelmAction::AiAnalyse {
                        name: r.name.clone(),
                        namespace: r.namespace.clone(),
                    };
                }
            }

            _ => {}
        }
        HelmAction::None
    }

    fn handle_history_key(&mut self, event: &KeyEvent) -> HelmAction {
        match event.code {
            KeyCode::Esc | KeyCode::Char('q') => {
                self.sub = SubView::Releases;
            }
            KeyCode::Up | KeyCode::Char('k') => {
                let i = self.history_table.selected().unwrap_or(0);
                if i > 0 {
                    self.history_table.select(Some(i - 1));
                }
            }
            KeyCode::Down | KeyCode::Char('j') => {
                let cur = self.history_table.selected().unwrap_or(0);
                let next = cur + 1;
                if next < self.history.len() {
                    self.history_table.select(Some(next));
                }
            }

            // Enter / r — rollback to selected revision (caller confirms).
            KeyCode::Enter | KeyCode::Char('r') => {
                if let Some(e) = self.selected_history_entry() {
                    let rel = self.history_release.clone();
                    // We don't store namespace in HelmHistoryEntry; re-resolve from releases.
                    let ns = self
                        .releases
                        .iter()
                        .find(|r| r.name == rel)
                        .map(|r| r.namespace.clone())
                        .unwrap_or_default();
                    return HelmAction::Rollback {
                        name: rel,
                        namespace: ns,
                        revision: e.revision,
                    };
                }
            }

            // v — values for the current release (per-revision values need
            // `helm get values --revision N` which we expose as a full text pane).
            KeyCode::Char('v') => {
                if let Some(e) = self.selected_history_entry() {
                    let rev = e.revision.to_string();
                    let name = self.history_release.clone();
                    let ns = self
                        .releases
                        .iter()
                        .find(|r| r.name == name)
                        .map(|r| r.namespace.clone())
                        .unwrap_or_default();
                    let content = run_helm_get(
                        "values",
                        &name,
                        &ns,
                        &["--revision", &rev, "--output", "yaml"],
                    );
                    let sanitized = sanitize_text_output(content);
                    self.open_text(
                        format!(" Helm Values — {name} rev {rev} · {ns} "),
                        sanitized,
                    );
                }
            }

            _ => {}
        }
        HelmAction::None
    }

    fn handle_text_key(&mut self, event: &KeyEvent) -> HelmAction {
        match event.code {
            KeyCode::Esc | KeyCode::Char('q') => {
                self.sub = SubView::Releases;
            }
            KeyCode::Up | KeyCode::Char('k') => {
                self.text_scroll = self.text_scroll.saturating_sub(1);
            }
            KeyCode::Down | KeyCode::Char('j') => {
                self.text_scroll += 1;
            }
            KeyCode::PageUp => {
                self.text_scroll = self.text_scroll.saturating_sub(10);
            }
            KeyCode::PageDown => {
                self.text_scroll += 10;
            }
            KeyCode::Home | KeyCode::Char('g') => {
                self.text_scroll = 0;
            }
            KeyCode::End | KeyCode::Char('G') => {
                self.text_scroll = usize::MAX / 2;
            }
            _ => {}
        }
        HelmAction::None
    }

    fn open_text(&mut self, title: String, content: String) {
        self.text_title = title;
        self.text_content = content;
        self.text_scroll = 0;
        self.sub = SubView::Text;
    }

    /// Render the Helm view into the given area.
    pub fn render(&mut self, frame: &mut Frame, area: Rect) {
        match &self.sub {
            SubView::Releases => self.render_releases(frame, area),
            SubView::History => self.render_history(frame, area),
            SubView::Text => self.render_text(frame, area),
        }
    }

    fn render_releases(&mut self, frame: &mut Frame, area: Rect) {
        let selected_style = Style::default()
            .add_modifier(Modifier::BOLD)
            .bg(Color::DarkGray);
        let header_style = Style::default()
            .fg(Color::Yellow)
            .add_modifier(Modifier::BOLD | Modifier::UNDERLINED);

        let headers = [
            "NAME",
            "NAMESPACE",
            "CHART",
            "APP VER",
            "STATUS",
            "REV",
            "UPDATED",
        ];
        let header_row =
            Row::new(headers.iter().map(|h| Cell::from(*h).style(header_style))).height(1);

        let rows: Vec<Row> = self
            .releases
            .iter()
            .map(|r| {
                let status_style = status_color(&r.status);
                Row::new(vec![
                    Cell::from(r.name.clone()),
                    Cell::from(r.namespace.clone()),
                    Cell::from(r.chart.clone()),
                    Cell::from(r.app_version.clone()),
                    Cell::from(r.status.clone()).style(status_style),
                    Cell::from(r.revision.clone()),
                    Cell::from(format_updated(&r.updated)),
                ])
            })
            .collect();

        let widths = [
            Constraint::Percentage(20),
            Constraint::Percentage(14),
            Constraint::Percentage(20),
            Constraint::Percentage(9),
            Constraint::Percentage(11),
            Constraint::Length(5),
            Constraint::Percentage(19),
        ];

        let title = if self.releases.is_empty() {
            " Helm Releases (no releases found — run `helm install` first) ".to_string()
        } else {
            format!(" Helm Releases ({}) ", self.releases.len())
        };

        let table = Table::new(rows, widths)
            .header(header_row)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(title)
                    .title_style(
                        Style::default()
                            .fg(Color::Cyan)
                            .add_modifier(Modifier::BOLD),
                    ),
            )
            .row_highlight_style(selected_style)
            .highlight_symbol("▶ ");

        frame.render_stateful_widget(table, area, &mut self.release_table);

        if let Some(msg) = &self.status {
            let hint_area = Rect {
                x: area.x + 1,
                y: area.y + area.height.saturating_sub(1),
                width: area.width.saturating_sub(2),
                height: 1,
            };
            frame.render_widget(
                Paragraph::new(msg.as_str()).style(Style::default().fg(Color::Red)),
                hint_area,
            );
        }
    }

    fn render_history(&mut self, frame: &mut Frame, area: Rect) {
        let selected_style = Style::default()
            .add_modifier(Modifier::BOLD)
            .bg(Color::DarkGray);
        let header_style = Style::default()
            .fg(Color::Yellow)
            .add_modifier(Modifier::BOLD | Modifier::UNDERLINED);

        let headers = [
            "REV",
            "STATUS",
            "CHART",
            "APP VER",
            "UPDATED",
            "DESCRIPTION",
        ];
        let header_row =
            Row::new(headers.iter().map(|h| Cell::from(*h).style(header_style))).height(1);

        let rows: Vec<Row> = self
            .history
            .iter()
            .map(|e| {
                let status_style = status_color(&e.status);
                Row::new(vec![
                    Cell::from(e.revision.to_string()),
                    Cell::from(e.status.clone()).style(status_style),
                    Cell::from(e.chart.clone()),
                    Cell::from(e.app_version.clone()),
                    Cell::from(format_updated(&e.updated)),
                    Cell::from(e.description.clone()),
                ])
            })
            .collect();

        let widths = [
            Constraint::Length(5),
            Constraint::Percentage(12),
            Constraint::Percentage(22),
            Constraint::Percentage(10),
            Constraint::Percentage(20),
            Constraint::Percentage(29),
        ];

        let title = format!(" Helm History — {} ", self.history_release);

        let table = Table::new(rows, widths)
            .header(header_row)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(title)
                    .title_style(
                        Style::default()
                            .fg(Color::Cyan)
                            .add_modifier(Modifier::BOLD),
                    ),
            )
            .row_highlight_style(selected_style)
            .highlight_symbol("▶ ");

        frame.render_stateful_widget(table, area, &mut self.history_table);
    }

    fn render_text(&mut self, frame: &mut Frame, area: Rect) {
        let lines: Vec<ratatui::text::Line> = self
            .text_content
            .lines()
            .map(ratatui::text::Line::raw)
            .collect();
        let total = lines.len();
        let visible = area.height.saturating_sub(2) as usize;
        let max_scroll = total.saturating_sub(visible);
        self.text_scroll = self.text_scroll.min(max_scroll);

        let para = Paragraph::new(lines)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(self.text_title.clone())
                    .title_style(
                        Style::default()
                            .fg(Color::Cyan)
                            .add_modifier(Modifier::BOLD),
                    ),
            )
            .scroll((self.text_scroll as u16, 0))
            .wrap(Wrap { trim: false });

        frame.render_widget(para, area);

        if total > visible {
            let mut sb = ScrollbarState::new(max_scroll).position(self.text_scroll);
            frame.render_stateful_widget(
                Scrollbar::new(ScrollbarOrientation::VerticalRight),
                area,
                &mut sb,
            );
        }
    }
}

impl Default for HelmView {
    fn default() -> Self {
        Self::new()
    }
}

// ─── Helpers ──────────────────────────────────────────────────────────────────

/// Color-code a Helm release/revision status string.
pub fn status_color(status: &str) -> Style {
    match status.to_lowercase().as_str() {
        "deployed" => Style::default().fg(Color::Green),
        "failed" => Style::default().fg(Color::Red),
        s if s.starts_with("pending") => Style::default().fg(Color::Yellow),
        "uninstalling" => Style::default().fg(Color::Yellow),
        "superseded" => Style::default().fg(Color::DarkGray),
        _ => Style::default(),
    }
}

/// Truncate a Helm timestamp to the date + time tokens, dropping timezone.
fn format_updated(raw: &str) -> String {
    let parts: Vec<&str> = raw.splitn(3, ' ').collect();
    if parts.len() >= 2 {
        format!("{} {}", parts[0], parts[1])
    } else {
        raw.to_string()
    }
}

/// Run `helm get <subcommand> <release> -n <namespace> [extra_args...]`.
fn run_helm_get(subcommand: &str, name: &str, namespace: &str, extra: &[&str]) -> String {
    let mut cmd = std::process::Command::new("helm");
    cmd.args(["get", subcommand, name, "-n", namespace]);
    cmd.args(extra);
    match cmd.output() {
        Ok(o) if o.status.success() => String::from_utf8_lossy(&o.stdout).into_owned(),
        Ok(o) => format!("Error: {}", String::from_utf8_lossy(&o.stderr).trim()),
        Err(e) => format!("Error running helm: {e}"),
    }
}

/// Strip any line from `helm get values` output that looks like a secret.
///
/// Helm YAML values are already sanitized by the key/value rules when fed to
/// the AI, but this provides a best-effort redaction for the on-screen display
/// so secrets don't appear in the TUI at all.
fn sanitize_text_output(text: String) -> String {
    text.lines()
        .map(|line| {
            // If the line contains "password:", "token:", "secret:", etc., redact the value.
            let lower = line.to_lowercase();
            let is_secret_line = crate::sanitizer::helm::SECRET_KEY_PATTERNS
                .iter()
                .any(|p| lower.contains(&format!("{p}:")));
            if is_secret_line {
                // Keep the key, redact the value: "  password: [REDACTED]"
                if let Some(colon_pos) = line.find(':') {
                    format!("{}: [REDACTED]", &line[..colon_pos])
                } else {
                    "[REDACTED]".to_string()
                }
            } else {
                line.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyEventKind, KeyEventState, KeyModifiers};

    fn press(code: KeyCode) -> KeyEvent {
        KeyEvent {
            code,
            modifiers: KeyModifiers::NONE,
            kind: KeyEventKind::Press,
            state: KeyEventState::NONE,
        }
    }

    #[test]
    fn new_view_starts_at_releases() {
        let v = HelmView::new();
        assert_eq!(v.sub, SubView::Releases);
    }

    #[test]
    fn esc_closes_from_releases() {
        let mut v = HelmView::new();
        assert_eq!(v.handle_key(&press(KeyCode::Esc)), HelmAction::Close);
    }

    #[test]
    fn q_closes_from_releases() {
        let mut v = HelmView::new();
        assert_eq!(v.handle_key(&press(KeyCode::Char('q'))), HelmAction::Close);
    }

    #[test]
    fn esc_from_history_returns_to_releases() {
        let mut v = HelmView::new();
        v.sub = SubView::History;
        v.handle_key(&press(KeyCode::Esc));
        assert_eq!(v.sub, SubView::Releases);
    }

    #[test]
    fn esc_from_text_returns_to_releases() {
        let mut v = HelmView::new();
        v.sub = SubView::Text;
        v.handle_key(&press(KeyCode::Esc));
        assert_eq!(v.sub, SubView::Releases);
    }

    #[test]
    fn status_color_deployed_is_green() {
        assert_eq!(status_color("deployed").fg, Some(Color::Green));
    }

    #[test]
    fn status_color_failed_is_red() {
        assert_eq!(status_color("failed").fg, Some(Color::Red));
    }

    #[test]
    fn status_color_pending_install_is_yellow() {
        assert_eq!(status_color("pending-install").fg, Some(Color::Yellow));
    }

    #[test]
    fn status_color_superseded_is_dark_gray() {
        assert_eq!(status_color("superseded").fg, Some(Color::DarkGray));
    }

    #[test]
    fn format_updated_truncates_timezone() {
        assert_eq!(
            format_updated("2026-04-10 12:00:00.999 +0000 UTC"),
            "2026-04-10 12:00:00.999"
        );
    }

    #[test]
    fn format_updated_short_value_unchanged() {
        assert_eq!(format_updated("today"), "today");
    }

    #[test]
    fn refresh_action_on_r() {
        let mut v = HelmView::new();
        assert_eq!(
            v.handle_key(&press(KeyCode::Char('r'))),
            HelmAction::Refresh
        );
    }

    #[test]
    fn down_does_not_go_past_end_of_empty_list() {
        let mut v = HelmView::new();
        v.handle_key(&press(KeyCode::Down));
        assert_eq!(v.release_table.selected(), None);
    }

    #[test]
    fn sanitize_text_output_redacts_password_line() {
        let text = "replicas: 3\npassword: hunter2\nhost: localhost".to_string();
        let out = sanitize_text_output(text);
        assert!(out.contains("password: [REDACTED]"));
        assert!(!out.contains("hunter2"));
        assert!(out.contains("replicas: 3"));
        assert!(out.contains("host: localhost"));
    }
}
