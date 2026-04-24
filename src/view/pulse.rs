//! Pulse view — Phase 6.20.
//!
//! Cluster-at-a-glance dashboard showing nodes, pods, deployments, events,
//! and namespace counts.  Opened with `:pulse`.
//!
//! The view renders as a grid of stat cards (like k9s's pulse view) using
//! ratatui blocks.  Data is fed via [`PulseView::update`] with a
//! [`ClusterSummary`] built from the current watcher store snapshots.
//!
//! # k9s Reference
//! `internal/view/pulse.go`

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
    Frame,
};

use crate::health::ClusterSummary;

/// Action returned by [`PulseView::handle_key`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PulseAction {
    /// User pressed `q` / `Esc` — close the pulse view.
    Close,
    /// `r` pressed — caller should trigger a data refresh.
    Refresh,
    /// Key consumed, no action.
    None,
}

/// The Pulse dashboard view.
pub struct PulseView {
    summary: ClusterSummary,
}

impl PulseView {
    pub fn new() -> Self {
        Self {
            summary: ClusterSummary::default(),
        }
    }

    /// Replace the displayed summary with fresh data.
    pub fn update(&mut self, summary: ClusterSummary) {
        self.summary = summary;
    }

    pub fn handle_key(&self, key: &KeyEvent) -> PulseAction {
        match key.code {
            KeyCode::Char('q') | KeyCode::Esc => PulseAction::Close,
            KeyCode::Char('r') => PulseAction::Refresh,
            _ => PulseAction::None,
        }
    }

    /// Render the pulse dashboard into `area`.
    pub fn render(&self, frame: &mut Frame, area: Rect) {
        // Outer block.
        let block = Block::default()
            .borders(Borders::ALL)
            .title(Span::styled(
                " Pulse — Cluster Overview ",
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ))
            .title_alignment(Alignment::Center);

        let inner = block.inner(area);
        frame.render_widget(block, area);

        // Three rows: nodes | pods | deployments / events / namespaces.
        let rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(5), // nodes row
                Constraint::Length(5), // pods row
                Constraint::Length(5), // misc row
                Constraint::Min(0),    // padding
            ])
            .split(inner);

        render_node_card(frame, &self.summary, rows[0]);
        render_pod_card(frame, &self.summary, rows[1]);
        render_misc_row(frame, &self.summary, rows[2]);
    }
}

impl Default for PulseView {
    fn default() -> Self {
        Self::new()
    }
}

// ─── Card renderers ───────────────────────────────────────────────────────────

fn render_node_card(frame: &mut Frame, s: &ClusterSummary, area: Rect) {
    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(33),
            Constraint::Percentage(33),
            Constraint::Percentage(34),
        ])
        .split(area);

    let ready_color = if s.nodes.not_ready > 0 {
        Color::Red
    } else {
        Color::Green
    };

    render_stat_card(
        frame,
        cols[0],
        "Nodes",
        &s.nodes.total.to_string(),
        Color::White,
    );
    render_stat_card(
        frame,
        cols[1],
        "Ready",
        &s.nodes.ready.to_string(),
        ready_color,
    );
    render_stat_card(
        frame,
        cols[2],
        "Not Ready",
        &s.nodes.not_ready.to_string(),
        ready_color,
    );
}

fn render_pod_card(frame: &mut Frame, s: &ClusterSummary, area: Rect) {
    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(25),
            Constraint::Percentage(25),
            Constraint::Percentage(25),
            Constraint::Percentage(25),
        ])
        .split(area);

    let fail_color = if s.pods.failed > 0 {
        Color::Red
    } else {
        Color::Green
    };

    render_stat_card(
        frame,
        cols[0],
        "Pods",
        &s.pods.total.to_string(),
        Color::White,
    );
    render_stat_card(
        frame,
        cols[1],
        "Running",
        &s.pods.running.to_string(),
        Color::Green,
    );
    render_stat_card(
        frame,
        cols[2],
        "Pending",
        &s.pods.pending.to_string(),
        Color::Yellow,
    );
    render_stat_card(
        frame,
        cols[3],
        "Failed",
        &s.pods.failed.to_string(),
        fail_color,
    );
}

fn render_misc_row(frame: &mut Frame, s: &ClusterSummary, area: Rect) {
    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(25),
            Constraint::Percentage(25),
            Constraint::Percentage(25),
            Constraint::Percentage(25),
        ])
        .split(area);

    let warn_color = if s.events_warn > 0 {
        Color::Yellow
    } else {
        Color::Green
    };
    let deploy_fail = if s.deployments.failed > 0 {
        Color::Red
    } else {
        Color::Green
    };

    render_stat_card(
        frame,
        cols[0],
        "Deployments",
        &s.deployments.total.to_string(),
        Color::White,
    );
    render_stat_card(
        frame,
        cols[1],
        "Deploy OK",
        &s.deployments.running.to_string(),
        Color::Green,
    );
    render_stat_card(
        frame,
        cols[2],
        "Namespaces",
        &s.namespaces.to_string(),
        Color::Cyan,
    );
    render_stat_card(
        frame,
        cols[3],
        "Warn Events",
        &s.events_warn.to_string(),
        warn_color,
    );
    let _ = deploy_fail; // suppress unused warning when there are no failed deployments
}

/// Render a single stat card with a title and a big number.
fn render_stat_card(frame: &mut Frame, area: Rect, title: &str, value: &str, value_color: Color) {
    let block = Block::default().borders(Borders::ALL).title(Span::styled(
        format!(" {title} "),
        Style::default().fg(Color::DarkGray),
    ));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    let para = Paragraph::new(vec![
        Line::raw(""),
        Line::from(Span::styled(
            value,
            Style::default()
                .fg(value_color)
                .add_modifier(Modifier::BOLD),
        )),
    ])
    .alignment(Alignment::Center);

    frame.render_widget(para, inner);
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::health::ClusterSummary;
    use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers};

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent {
            code,
            modifiers: KeyModifiers::NONE,
            kind: KeyEventKind::Press,
            state: KeyEventState::NONE,
        }
    }

    #[test]
    fn q_closes_pulse() {
        let view = PulseView::new();
        assert_eq!(
            view.handle_key(&key(KeyCode::Char('q'))),
            PulseAction::Close
        );
    }

    #[test]
    fn esc_closes_pulse() {
        let view = PulseView::new();
        assert_eq!(view.handle_key(&key(KeyCode::Esc)), PulseAction::Close);
    }

    #[test]
    fn other_key_is_none() {
        let view = PulseView::new();
        assert_eq!(view.handle_key(&key(KeyCode::Char('j'))), PulseAction::None);
    }

    #[test]
    fn update_replaces_summary() {
        let mut view = PulseView::new();
        let mut summary = ClusterSummary::new();
        summary.namespaces = 5;
        view.update(summary);
        assert_eq!(view.summary.namespaces, 5);
    }
}
