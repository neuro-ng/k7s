//! Help view — Phase 6.16.
//!
//! Displays a scrollable reference of all key bindings, grouped by context.
//! Opened with `?` in Browse mode; closed with `q` or `Esc`.
//!
//! # k9s Reference
//! `internal/view/help.go`

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph, Scrollbar, ScrollbarOrientation, ScrollbarState},
    Frame,
};

/// Action returned by [`HelpView::handle_key`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HelpAction {
    /// User closed the help view (`q` / `Esc`).
    Close,
    /// Key consumed but view stays open.
    None,
}

/// Scrollable key-binding reference overlay.
///
/// Renders as a centred modal that covers ~80% of the terminal.
pub struct HelpView {
    scroll: u16,
}

impl HelpView {
    pub fn new() -> Self {
        Self { scroll: 0 }
    }

    /// Handle a key event.
    pub fn handle_key(&mut self, key: &KeyEvent) -> HelpAction {
        match key.code {
            KeyCode::Char('q') | KeyCode::Esc => HelpAction::Close,
            KeyCode::Up | KeyCode::Char('k') => {
                self.scroll = self.scroll.saturating_sub(1);
                HelpAction::None
            }
            KeyCode::Down | KeyCode::Char('j') => {
                self.scroll = self.scroll.saturating_add(1);
                HelpAction::None
            }
            KeyCode::PageUp | KeyCode::Char('u') => {
                self.scroll = self.scroll.saturating_sub(10);
                HelpAction::None
            }
            KeyCode::PageDown | KeyCode::Char('d') => {
                self.scroll = self.scroll.saturating_add(10);
                HelpAction::None
            }
            KeyCode::Home | KeyCode::Char('g') => {
                self.scroll = 0;
                HelpAction::None
            }
            _ => HelpAction::None,
        }
    }

    /// Render the help overlay centred inside `area`.
    pub fn render(&mut self, frame: &mut Frame, area: Rect) {
        let popup = centred_rect(84, 88, area);
        frame.render_widget(Clear, popup);

        let block = Block::default()
            .borders(Borders::ALL)
            .title(Span::styled(
                " k7s — Key Bindings ",
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ))
            .title_alignment(Alignment::Center);

        let inner = block.inner(popup);
        frame.render_widget(block, popup);

        // Split inner: content (left) + scrollbar (right 1 col).
        let cols = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Min(0), Constraint::Length(1)])
            .split(inner);

        let lines = build_lines();
        let total = lines.len() as u16;

        // Clamp scroll.
        let visible = cols[0].height;
        self.scroll = self.scroll.min(total.saturating_sub(visible));

        let para = Paragraph::new(lines).scroll((self.scroll, 0));
        frame.render_widget(para, cols[0]);

        // Scrollbar.
        let mut sb_state = ScrollbarState::default()
            .content_length(total as usize)
            .position(self.scroll as usize);
        frame.render_stateful_widget(
            Scrollbar::new(ScrollbarOrientation::VerticalRight),
            cols[1],
            &mut sb_state,
        );
    }
}

impl Default for HelpView {
    fn default() -> Self {
        Self::new()
    }
}

// ─── Content ─────────────────────────────────────────────────────────────────

fn build_lines() -> Vec<Line<'static>> {
    let mut lines: Vec<Line<'static>> = Vec::new();

    // Helper closures.
    let section = |title: &'static str| -> Line<'static> {
        Line::from(vec![
            Span::raw("  "),
            Span::styled(
                title,
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            ),
        ])
    };
    let blank = || Line::raw("");
    let row = |key: &'static str, desc: &'static str| -> Line<'static> {
        Line::from(vec![
            Span::styled(format!("    {key:<18}"), Style::default().fg(Color::Cyan)),
            Span::raw(desc),
        ])
    };

    lines.push(blank());
    lines.push(section("Navigation"));
    lines.push(row("↑↓ / j k", "Move cursor up / down"));
    lines.push(row("PgUp / Ctrl-U", "Page up"));
    lines.push(row("PgDn / Ctrl-D", "Page down"));
    lines.push(row("g / Home", "Jump to top"));
    lines.push(row("G / End", "Jump to bottom"));
    lines.push(row("⏎ Enter", "Select / drill down"));
    lines.push(row("Esc", "Back / close overlay"));
    lines.push(blank());

    lines.push(section("History"));
    lines.push(row("[  or  Backspace", "Go back one step in history"));
    lines.push(row("]", "Go forward one step in history"));
    lines.push(row("-", "Toggle to last-visited resource"));
    lines.push(blank());

    lines.push(section("Resource Operations"));
    lines.push(row("d", "Describe selected resource"));
    lines.push(row("y", "View YAML of selected resource"));
    lines.push(row("l", "Stream logs (pods / containers)"));
    lines.push(row("e", "Open shell inside pod"));
    lines.push(row("f", "Port-forward service / pod"));
    lines.push(row("s", "Scale workload (replicas)"));
    lines.push(row("r", "Restart workload (rollout restart)"));
    lines.push(row("D  or  Delete", "Delete resource (with confirmation)"));
    lines.push(row("c", "Copy resource name to clipboard"));
    lines.push(row("a", "Toggle all-namespaces scope"));
    lines.push(blank());

    lines.push(section("Commands & Search"));
    lines.push(row(":", "Open command prompt"));
    lines.push(row("  :pods", "Navigate to Pod list"));
    lines.push(row("  :deploy", "Navigate to Deployment list"));
    lines.push(row("  :nodes", "Navigate to Node list"));
    lines.push(row("  :ns", "Navigate to Namespace list"));
    lines.push(row("  :svc", "Navigate to Service list"));
    lines.push(row("  :ctx", "Navigate to Context switcher"));
    lines.push(row("  :pulse", "Cluster overview dashboard"));
    lines.push(row("  :workload", "Aggregated workload view"));
    lines.push(row("  :crd", "Custom Resource Definitions"));
    lines.push(row("  :help", "Open this help screen"));
    lines.push(row("/", "Filter current view (regex)"));
    lines.push(row("F5", "Refresh current view immediately"));
    lines.push(blank());

    lines.push(section("AI Chat"));
    lines.push(row("Space", "Open AI chat window"));
    lines.push(row("A", "Ask AI to analyse selected resource"));
    lines.push(row("Esc  (in chat)", "Close chat window"));
    lines.push(blank());

    lines.push(section("Application"));
    lines.push(row("?", "This help screen"));
    lines.push(row("q  /  Ctrl-C", "Quit k7s"));
    lines.push(blank());

    lines.push(Line::from(Span::styled(
        "  Press q or Esc to close",
        Style::default().fg(Color::DarkGray),
    )));
    lines.push(blank());

    lines
}

// ─── Layout helper ────────────────────────────────────────────────────────────

/// Return a [`Rect`] centred in `area` with percentage width and percentage height.
fn centred_rect(percent_x: u16, percent_y: u16, area: Rect) -> Rect {
    let vert = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - percent_y.min(100)) / 2),
            Constraint::Percentage(percent_y.min(100)),
            Constraint::Percentage((100 - percent_y.min(100)) / 2),
        ])
        .split(area);

    let horiz = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x.min(100)) / 2),
            Constraint::Percentage(percent_x.min(100)),
            Constraint::Percentage((100 - percent_x.min(100)) / 2),
        ])
        .split(vert[1]);

    horiz[1]
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyEventKind, KeyEventState, KeyModifiers};

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent {
            code,
            modifiers: KeyModifiers::NONE,
            kind: KeyEventKind::Press,
            state: KeyEventState::NONE,
        }
    }

    #[test]
    fn q_closes_help() {
        let mut view = HelpView::new();
        assert_eq!(view.handle_key(&key(KeyCode::Char('q'))), HelpAction::Close);
    }

    #[test]
    fn esc_closes_help() {
        let mut view = HelpView::new();
        assert_eq!(view.handle_key(&key(KeyCode::Esc)), HelpAction::Close);
    }

    #[test]
    fn j_scrolls_down() {
        let mut view = HelpView::new();
        view.handle_key(&key(KeyCode::Char('j')));
        assert_eq!(view.scroll, 1);
    }

    #[test]
    fn k_scrolls_up_clamped_at_zero() {
        let mut view = HelpView::new();
        view.handle_key(&key(KeyCode::Char('k'))); // already at 0 — stays 0
        assert_eq!(view.scroll, 0);
    }

    #[test]
    fn g_resets_scroll() {
        let mut view = HelpView::new();
        view.scroll = 10;
        view.handle_key(&key(KeyCode::Char('g')));
        assert_eq!(view.scroll, 0);
    }

    #[test]
    fn build_lines_non_empty() {
        let lines = build_lines();
        assert!(!lines.is_empty());
        // Verify key sections are present in the content.
        let text: String = lines
            .iter()
            .flat_map(|l| l.spans.iter().map(|s| s.content.as_ref()))
            .collect::<Vec<_>>()
            .join("");
        assert!(text.contains("Navigation"));
        assert!(text.contains("History"));
        assert!(text.contains("AI Chat"));
    }
}
