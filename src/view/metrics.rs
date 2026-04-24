//! Live metrics dashboard view — Phase 18.
//!
//! Renders CPU and memory sparklines for every pod and node whose metrics have
//! been ingested into the [`MetricsStore`].  Activated via the `:metrics`
//! (alias `:top`) command.
//!
//! Layout:
//!
//! ```text
//! ┌─────────────────────────────────────────────────────────┐
//! │  k7s · Live Metrics — ctx:my-cluster  [last poll: 3s]   │
//! ├───────────────────────┬─────────────────────────────────┤
//! │  NODES                │  PODS (top by CPU)              │
//! │  node-1  CPU ▁▂▃▅▆   │  ns/pod-a  CPU ▂▃▄▅▆  125m     │
//! │          MEM ▂▂▃▃▄   │            MEM ▁▁▂▂▃   64 Mi   │
//! │  node-2  CPU ▁▁▁▂▂   │  ns/pod-b  CPU ▁▁▁▂▂   45m     │
//! │          MEM ▃▃▃▄▄   │            MEM ▄▄▅▅▆  256 Mi   │
//! └───────────────────────┴─────────────────────────────────┘
//! ```
//!
//! # k9s Reference
//! No direct equivalent — this is a k7s-unique view.  Loosely inspired by
//! `internal/view/pulse.go` and k9s's top-like display.

use std::time::Instant;

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Sparkline},
    Frame,
};

use crate::metrics::{MetricsStore, HISTORY_LEN};

/// How many top pods to show in the dashboard.
const TOP_PODS: usize = 12;
/// How many nodes to show.
const TOP_NODES: usize = 8;

/// Actions that `MetricsView::handle_key` can return to the `App`.
#[derive(Debug, Clone)]
pub enum MetricsAction {
    /// User pressed Esc / `q` — close the metrics view.
    Close,
    /// User pressed `r` — trigger an immediate metrics refresh.
    Refresh,
}

/// Fullscreen metrics dashboard widget.
pub struct MetricsView {
    /// Scroll offset for the pod list.
    pub scroll: usize,
    /// Time of the last metrics snapshot ingested.
    pub last_poll: Option<Instant>,
    /// Whether we are waiting for the first poll.
    pub loading: bool,
}

impl MetricsView {
    pub fn new() -> Self {
        Self {
            scroll: 0,
            last_poll: None,
            loading: true,
        }
    }

    /// Notify the view that fresh metrics have arrived.
    pub fn on_metrics_updated(&mut self) {
        self.last_poll = Some(Instant::now());
        self.loading = false;
    }

    /// Handle a key event.  Returns `Some(action)` if the app should react.
    pub fn handle_key(&mut self, key: &KeyEvent) -> Option<MetricsAction> {
        match key.code {
            KeyCode::Esc | KeyCode::Char('q') => Some(MetricsAction::Close),
            KeyCode::Char('r') => Some(MetricsAction::Refresh),
            KeyCode::Down | KeyCode::Char('j') => {
                self.scroll = self.scroll.saturating_add(1);
                None
            }
            KeyCode::Up | KeyCode::Char('k') => {
                self.scroll = self.scroll.saturating_sub(1);
                None
            }
            _ => None,
        }
    }

    /// Draw the full metrics dashboard into `area`.
    pub fn draw(&self, frame: &mut Frame, area: Rect, store: &MetricsStore, ctx: &str) {
        // ── Header ─────────────────────────────────────────────────────────────
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(1), Constraint::Min(0)])
            .split(area);

        let poll_label = if self.loading {
            "waiting for first poll…".to_owned()
        } else if let Some(t) = self.last_poll {
            format!("last poll {}s ago", t.elapsed().as_secs())
        } else {
            "no data".to_owned()
        };

        let header_line = Line::from(vec![
            Span::styled(
                " k7s · Live Metrics",
                Style::default().add_modifier(Modifier::BOLD),
            ),
            Span::raw(format!(
                " — ctx:{ctx}  [{poll_label}]  ↑/↓ scroll · r refresh · Esc close"
            )),
        ]);
        frame.render_widget(Paragraph::new(header_line), chunks[0]);

        // ── Split: nodes left / pods right ─────────────────────────────────────
        let body = chunks[1];
        let cols = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(35), Constraint::Percentage(65)])
            .split(body);

        self.draw_nodes(frame, cols[0], store);
        self.draw_pods(frame, cols[1], store);
    }

    fn draw_nodes(&self, frame: &mut Frame, area: Rect, store: &MetricsStore) {
        let block = Block::default().borders(Borders::ALL).title(Span::styled(
            " NODES ",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ));
        let inner = block.inner(area);
        frame.render_widget(block, area);

        let nodes = store.top_nodes_by_cpu(TOP_NODES);
        if nodes.is_empty() {
            frame.render_widget(Paragraph::new(no_data_line()), inner);
            return;
        }

        // Each node occupies 3 rows: label row + CPU sparkline + mem sparkline.
        let rows_per_node = 3usize;
        let available_height = inner.height as usize;
        let visible = (available_height / rows_per_node).min(nodes.len());

        let constraints: Vec<Constraint> =
            std::iter::repeat(Constraint::Length(rows_per_node as u16))
                .take(visible)
                .chain(std::iter::once(Constraint::Min(0)))
                .collect();

        let node_areas = Layout::default()
            .direction(Direction::Vertical)
            .constraints(constraints)
            .split(inner);

        for (i, (key, hist)) in nodes.iter().enumerate().take(visible) {
            let latest = hist.latest();
            let cpu_m = latest
                .map(|s: &crate::metrics::MetricSample| s.cpu_m)
                .unwrap_or(0);
            let mem_ki = latest
                .map(|s: &crate::metrics::MetricSample| s.mem_ki)
                .unwrap_or(0);

            let label = format!(" {key}  cpu:{cpu_m}m  mem:{}", format_ki(mem_ki));

            render_resource_block(
                frame,
                node_areas[i],
                &label,
                &hist.cpu_sparkline(),
                &hist.mem_sparkline(),
                Color::Cyan,
            );
        }
    }

    fn draw_pods(&self, frame: &mut Frame, area: Rect, store: &MetricsStore) {
        let block = Block::default().borders(Borders::ALL).title(Span::styled(
            " PODS (top by CPU) ",
            Style::default()
                .fg(Color::Green)
                .add_modifier(Modifier::BOLD),
        ));
        let inner = block.inner(area);
        frame.render_widget(block, area);

        let pods = store.top_pods_by_cpu(TOP_PODS + self.scroll);
        // Apply scroll offset.
        let pods: Vec<_> = pods.into_iter().skip(self.scroll).collect();

        if pods.is_empty() {
            frame.render_widget(Paragraph::new(no_data_line()), inner);
            return;
        }

        let rows_per_pod = 3usize;
        let available_height = inner.height as usize;
        let visible = (available_height / rows_per_pod).min(pods.len());

        let constraints: Vec<Constraint> =
            std::iter::repeat(Constraint::Length(rows_per_pod as u16))
                .take(visible)
                .chain(std::iter::once(Constraint::Min(0)))
                .collect();

        let pod_areas = Layout::default()
            .direction(Direction::Vertical)
            .constraints(constraints)
            .split(inner);

        for (i, (key, hist)) in pods.iter().enumerate().take(visible) {
            let latest = hist.latest();
            let cpu_m = latest
                .map(|s: &crate::metrics::MetricSample| s.cpu_m)
                .unwrap_or(0);
            let mem_ki = latest
                .map(|s: &crate::metrics::MetricSample| s.mem_ki)
                .unwrap_or(0);

            let label = format!(" {key}  cpu:{cpu_m}m  mem:{}", format_ki(mem_ki));

            render_resource_block(
                frame,
                pod_areas[i],
                &label,
                &hist.cpu_sparkline(),
                &hist.mem_sparkline(),
                Color::Green,
            );
        }
    }
}

impl Default for MetricsView {
    fn default() -> Self {
        Self::new()
    }
}

// ─── Helpers ──────────────────────────────────────────────────────────────────

/// Render a single resource's label + CPU sparkline + memory sparkline into
/// a 3-row `area`.
fn render_resource_block(
    frame: &mut Frame,
    area: Rect,
    label: &str,
    cpu_data: &[u64],
    mem_data: &[u64],
    color: Color,
) {
    if area.height < 3 {
        return;
    }

    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(1),
        ])
        .split(area);

    // Label row
    frame.render_widget(
        Paragraph::new(Span::styled(
            label,
            Style::default().fg(color).add_modifier(Modifier::BOLD),
        )),
        rows[0],
    );

    // CPU sparkline
    let cpu_max = cpu_data.iter().copied().max().unwrap_or(1).max(1);
    let cpu_sparkline = Sparkline::default()
        .data(cpu_data)
        .max(cpu_max)
        .style(Style::default().fg(Color::Yellow))
        .block(Block::default().borders(Borders::NONE));
    frame.render_widget(cpu_sparkline, rows[1]);

    // Memory sparkline
    let mem_max = mem_data.iter().copied().max().unwrap_or(1).max(1);
    let mem_sparkline = Sparkline::default()
        .data(mem_data)
        .max(mem_max)
        .style(Style::default().fg(Color::Blue))
        .block(Block::default().borders(Borders::NONE));
    frame.render_widget(mem_sparkline, rows[2]);
}

/// Format kibibytes into a human-readable string.
pub fn format_ki(ki: u64) -> String {
    if ki == 0 {
        return "0 Ki".to_owned();
    }
    if ki < 1_024 {
        return format!("{ki} Ki");
    }
    if ki < 1_024 * 1_024 {
        return format!("{:.1} Mi", ki as f64 / 1_024.0);
    }
    format!("{:.2} Gi", ki as f64 / (1_024.0 * 1_024.0))
}

/// Placeholder line when the metrics store has no data yet.
fn no_data_line() -> Line<'static> {
    Line::from(Span::styled(
        " No metrics data — is metrics-server installed?",
        Style::default().fg(Color::DarkGray),
    ))
}

/// Suggested sparkline history length for display (may be narrower than
/// [`HISTORY_LEN`] depending on available width).
pub fn display_history(width: u16) -> usize {
    // Reserve ~40 chars for labels; each sparkline bar is 1 char wide.
    let usable = (width as usize).saturating_sub(40);
    usable.min(HISTORY_LEN)
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_ki_zero() {
        assert_eq!(format_ki(0), "0 Ki");
    }

    #[test]
    fn format_ki_kibibytes() {
        assert_eq!(format_ki(512), "512 Ki");
    }

    #[test]
    fn format_ki_mebibytes() {
        let s = format_ki(2048);
        assert!(s.ends_with("Mi"), "got {s}");
    }

    #[test]
    fn format_ki_gibibytes() {
        let s = format_ki(2 * 1024 * 1024);
        assert!(s.ends_with("Gi"), "got {s}");
    }

    #[test]
    fn display_history_clamps_to_history_len() {
        let h = display_history(200);
        assert!(h <= HISTORY_LEN);
    }

    #[test]
    fn display_history_narrow_terminal() {
        let h = display_history(20);
        assert_eq!(h, 0, "width < 40 should give 0 usable columns");
    }

    #[test]
    fn metrics_view_key_esc_closes() {
        let mut view = MetricsView::new();
        let key =
            crossterm::event::KeyEvent::new(KeyCode::Esc, crossterm::event::KeyModifiers::NONE);
        let action = view.handle_key(&key);
        assert!(matches!(action, Some(MetricsAction::Close)));
    }

    #[test]
    fn metrics_view_key_r_refreshes() {
        let mut view = MetricsView::new();
        let key = crossterm::event::KeyEvent::new(
            KeyCode::Char('r'),
            crossterm::event::KeyModifiers::NONE,
        );
        let action = view.handle_key(&key);
        assert!(matches!(action, Some(MetricsAction::Refresh)));
    }

    #[test]
    fn metrics_view_scroll() {
        let mut view = MetricsView::new();
        let down =
            crossterm::event::KeyEvent::new(KeyCode::Down, crossterm::event::KeyModifiers::NONE);
        view.handle_key(&down);
        assert_eq!(view.scroll, 1);
        let up = crossterm::event::KeyEvent::new(KeyCode::Up, crossterm::event::KeyModifiers::NONE);
        view.handle_key(&up);
        assert_eq!(view.scroll, 0);
    }

    #[test]
    fn on_metrics_updated_clears_loading() {
        let mut view = MetricsView::new();
        assert!(view.loading);
        view.on_metrics_updated();
        assert!(!view.loading);
        assert!(view.last_poll.is_some());
    }
}
