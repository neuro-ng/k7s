//! Log view — fullscreen container log display with filter and scroll.
//!
//! # Layout
//!
//! ```text
//! ┌─ Logs: my-pod / app ────────────────────────── [LIVE] 142 lines ─┐
//! │ 2024-01-15T12:34:56Z INFO: server started                         │
//! │ 2024-01-15T12:34:57Z INFO: listening on :8080                     │
//! │ 2024-01-15T12:35:01Z ERROR: connection refused                    │
//! │ ...                                                                │
//! ├────────────────────────────────────────────────────────────────────┤
//! │ Filter: error                                      [Esc clear] [t] │
//! └────────────────────────────────────────────────────────────────────┘
//! ```
//!
//! Keys: `↑/↓` scroll, `PgUp/PgDn` page, `g/G` top/bottom, `t` toggle
//! timestamps, `/` enter filter, `Esc` clear filter, `q` close.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph, Scrollbar, ScrollbarOrientation, ScrollbarState};
use ratatui::Frame;

use crate::model::log::{LogItem, LogLevel, LogModel};

/// What the caller should do after a key press.
#[derive(Debug, Clone, PartialEq)]
pub enum LogAction {
    /// User pressed `q` / Esc (outside filter mode) — close the log view.
    Close,
    /// No action needed.
    None,
}

/// Input mode of the log view.
#[derive(Debug, Clone, PartialEq)]
enum InputMode {
    /// Normal navigation mode.
    Normal,
    /// User is typing a filter regex.
    Filter,
}

/// Fullscreen log view widget.
pub struct LogView {
    /// Display title (e.g. `"my-pod / app"`).
    pub title: String,
    /// The log data model.
    pub model: LogModel,
    /// Current scroll offset into the visible (filtered) lines.
    scroll: usize,
    /// Input mode.
    mode: InputMode,
    /// Filter text being typed.
    filter_input: String,
    /// Last filter error (shown to the user briefly).
    filter_error: Option<String>,
}

impl LogView {
    pub fn new(title: impl Into<String>) -> Self {
        Self {
            title:        title.into(),
            model:        LogModel::new(),
            scroll:       0,
            mode:         InputMode::Normal,
            filter_input: String::new(),
            filter_error: None,
        }
    }

    /// Push a new log line into the model.
    pub fn push(&mut self, line: impl Into<String>, container: Option<String>) {
        self.model.push(line, container);
        // Auto-scroll to bottom when streaming (scroll is at the tail).
        if self.model.streaming {
            self.scroll_to_bottom();
        }
    }

    /// Handle a key event. Returns the action the caller should take.
    pub fn handle_key(&mut self, key: KeyEvent) -> LogAction {
        match self.mode {
            InputMode::Filter => self.handle_filter_key(key),
            InputMode::Normal => self.handle_normal_key(key),
        }
    }

    fn handle_normal_key(&mut self, key: KeyEvent) -> LogAction {
        let visible = self.model.visible_lines().len();
        match key.code {
            KeyCode::Up | KeyCode::Char('k') => self.scroll_up(1),
            KeyCode::Down | KeyCode::Char('j') => self.scroll_down(1, visible),
            KeyCode::PageUp => self.scroll_up(20),
            KeyCode::PageDown => self.scroll_down(20, visible),
            KeyCode::Char('g') => self.scroll = 0,
            KeyCode::Char('G') => self.scroll_to_bottom(),
            KeyCode::Char('t') => {
                self.model.show_timestamps = !self.model.show_timestamps;
            }
            KeyCode::Char('/') => {
                self.mode = InputMode::Filter;
                self.filter_input = self.model.filter_pattern().unwrap_or("").to_owned();
                self.filter_error = None;
            }
            KeyCode::Esc | KeyCode::Char('q') => return LogAction::Close,
            _ => {}
        }
        LogAction::None
    }

    fn handle_filter_key(&mut self, key: KeyEvent) -> LogAction {
        match key.code {
            KeyCode::Enter => {
                let pattern = self.filter_input.trim().to_owned();
                match self.model.set_filter(if pattern.is_empty() { None } else { Some(&pattern) }) {
                    Ok(()) => {
                        self.filter_error = None;
                        self.scroll = 0;
                    }
                    Err(e) => {
                        self.filter_error = Some(format!("invalid regex: {e}"));
                    }
                }
                self.mode = InputMode::Normal;
            }
            KeyCode::Esc => {
                // Revert input and clear filter.
                let _ = self.model.set_filter(None);
                self.filter_input.clear();
                self.filter_error = None;
                self.mode = InputMode::Normal;
                self.scroll = 0;
            }
            KeyCode::Backspace => { self.filter_input.pop(); }
            KeyCode::Char(c) => {
                if !key.modifiers.contains(KeyModifiers::CONTROL) {
                    self.filter_input.push(c);
                }
            }
            _ => {}
        }
        LogAction::None
    }

    fn scroll_up(&mut self, n: usize) {
        self.scroll = self.scroll.saturating_sub(n);
    }

    fn scroll_down(&mut self, n: usize, visible: usize) {
        let max = visible.saturating_sub(1);
        self.scroll = (self.scroll + n).min(max);
    }

    fn scroll_to_bottom(&mut self) {
        let visible = self.model.visible_lines().len();
        self.scroll = visible.saturating_sub(1);
    }

    /// Render the log view into the frame.
    pub fn render(&self, frame: &mut Frame, area: Rect) {
        // Split area: log lines above, filter bar below.
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(3), Constraint::Length(3)])
            .split(area);

        self.render_log_pane(frame, chunks[0]);
        self.render_filter_bar(frame, chunks[1]);
    }

    fn render_log_pane(&self, frame: &mut Frame, area: Rect) {
        let live_indicator = if self.model.streaming { " [LIVE]" } else { "" };
        let visible = self.model.visible_lines();
        let total = self.model.len();
        let shown = visible.len();
        let count_label = if self.model.is_filtered() {
            format!(" {shown}/{total} lines")
        } else {
            format!(" {total} lines")
        };

        let block = Block::default()
            .title(format!(" Logs: {}{}{} ", self.title, live_indicator, count_label))
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Cyan));

        let inner = block.inner(area);
        let height = inner.height as usize;

        // Clamp scroll.
        let scroll = self.scroll.min(shown.saturating_sub(1));

        let lines: Vec<Line> = visible
            .iter()
            .skip(scroll)
            .take(height)
            .map(|item| self.render_log_line(item))
            .collect();

        let para = Paragraph::new(lines).block(block);
        frame.render_widget(para, area);

        // Scrollbar.
        if shown > height {
            let mut state = ScrollbarState::new(shown).position(scroll);
            let sb = Scrollbar::new(ScrollbarOrientation::VerticalRight);
            frame.render_stateful_widget(sb, area, &mut state);
        }
    }

    fn render_log_line<'a>(&self, item: &'a LogItem) -> Line<'a> {
        let text = item.display(self.model.show_timestamps);
        let base_style = level_style(item.level);

        // If there's an active filter, highlight matching spans.
        let ranges = self.model.highlight_ranges(&text);
        if ranges.is_empty() {
            return Line::from(Span::styled(text, base_style));
        }

        let hl_style = base_style.bg(Color::Yellow).fg(Color::Black).add_modifier(Modifier::BOLD);
        let mut spans = Vec::new();
        let mut pos = 0usize;

        for (start, end) in ranges {
            if pos < start {
                spans.push(Span::styled(text[pos..start].to_owned(), base_style));
            }
            spans.push(Span::styled(text[start..end].to_owned(), hl_style));
            pos = end;
        }
        if pos < text.len() {
            spans.push(Span::styled(text[pos..].to_owned(), base_style));
        }

        Line::from(spans)
    }

    fn render_filter_bar(&self, frame: &mut Frame, area: Rect) {
        let (label, content, border_color) = match &self.mode {
            InputMode::Filter => {
                let cursor = if self.filter_input.is_empty() { "█" } else { "" };
                ("Filter » ", format!("{}{}", self.filter_input, cursor), Color::Yellow)
            }
            InputMode::Normal => {
                let hint = if let Some(err) = &self.filter_error {
                    err.as_str()
                } else if self.model.is_filtered() {
                    self.model.filter_pattern().unwrap_or("")
                } else {
                    "/ to filter  ·  t timestamps  ·  g/G top/bottom  ·  q close"
                };
                ("", hint.to_owned(), Color::DarkGray)
            }
        };

        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(border_color));

        let text = if self.model.is_filtered() && self.mode == InputMode::Normal {
            Line::from(vec![
                Span::styled("Filter: ", Style::default().fg(Color::Yellow)),
                Span::styled(content, Style::default().fg(Color::White).add_modifier(Modifier::BOLD)),
                Span::raw("  (Esc to clear)"),
            ])
        } else {
            Line::from(vec![
                Span::styled(label.to_owned(), Style::default().fg(Color::Cyan)),
                Span::raw(content),
            ])
        };

        let para = Paragraph::new(vec![text]).block(block);
        frame.render_widget(para, area);
    }
}

/// Map a log level to a display style.
fn level_style(level: LogLevel) -> Style {
    match level {
        LogLevel::Error => Style::default().fg(Color::Red),
        LogLevel::Warn  => Style::default().fg(Color::Yellow),
        LogLevel::Info  => Style::default().fg(Color::White),
        LogLevel::Debug => Style::default().fg(Color::DarkGray),
        LogLevel::Trace => Style::default().fg(Color::DarkGray),
        LogLevel::Unknown => Style::default(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn view_with_lines(lines: &[&str]) -> LogView {
        let mut v = LogView::new("test-pod");
        for l in lines {
            v.push(*l, None);
        }
        v
    }

    #[test]
    fn push_increases_model_len() {
        let mut v = LogView::new("pod");
        v.push("hello", None);
        v.push("world", None);
        assert_eq!(v.model.len(), 2);
    }

    #[test]
    fn close_on_q() {
        let mut v = LogView::new("pod");
        let action = v.handle_key(KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE));
        assert_eq!(action, LogAction::Close);
    }

    #[test]
    fn close_on_esc_in_normal_mode() {
        let mut v = LogView::new("pod");
        let action = v.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        assert_eq!(action, LogAction::Close);
    }

    #[test]
    fn enter_filter_mode_on_slash() {
        let mut v = LogView::new("pod");
        v.handle_key(KeyEvent::new(KeyCode::Char('/'), KeyModifiers::NONE));
        assert_eq!(v.mode, InputMode::Filter);
    }

    #[test]
    fn filter_mode_esc_clears_filter() {
        let mut v = view_with_lines(&["error: x", "info: y"]);
        // Enter filter mode and set a pattern.
        v.handle_key(KeyEvent::new(KeyCode::Char('/'), KeyModifiers::NONE));
        for c in "error".chars() {
            v.handle_key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE));
        }
        v.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert!(v.model.is_filtered());
        // Now Esc in normal mode clears.
        // But first: Esc from normal mode closes the view, so we use the filter bar Esc instead.
        v.handle_key(KeyEvent::new(KeyCode::Char('/'), KeyModifiers::NONE));
        v.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        assert!(!v.model.is_filtered());
    }

    #[test]
    fn toggle_timestamps_with_t() {
        let mut v = LogView::new("pod");
        assert!(!v.model.show_timestamps);
        v.handle_key(KeyEvent::new(KeyCode::Char('t'), KeyModifiers::NONE));
        assert!(v.model.show_timestamps);
    }
}
