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
use ratatui::widgets::{
    Block, Borders, Paragraph, Scrollbar, ScrollbarOrientation, ScrollbarState,
};
use ratatui::Frame;

use crate::model::log::{LogItem, LogLevel, LogModel};

/// What the caller should do after a key press.
#[derive(Debug, Clone, PartialEq)]
pub enum LogAction {
    /// User pressed `q` / Esc (outside filter mode) — close the log view.
    Close,
    /// User cycled to a different container — re-stream logs for this container.
    SwitchContainer(String),
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
    /// Pod name (without container suffix).
    pub pod_name: String,
    /// All available container names for this pod.
    pub containers: Vec<String>,
    /// Index into `containers` for the currently selected container.
    pub container_idx: usize,
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
    /// Whether the container selector overlay is visible.
    selector_visible: bool,
}

impl LogView {
    /// Create a log view for a pod.
    ///
    /// `containers` is the list of container names; leave empty if not yet known.
    /// The first container in the list is selected by default.
    pub fn new(pod_name: impl Into<String>, containers: Vec<String>) -> Self {
        Self {
            pod_name: pod_name.into(),
            containers,
            container_idx: 0,
            model: LogModel::new(),
            scroll: 0,
            mode: InputMode::Normal,
            filter_input: String::new(),
            filter_error: None,
            selector_visible: false,
        }
    }

    /// The name of the currently selected container, if any.
    pub fn current_container(&self) -> Option<&str> {
        self.containers.get(self.container_idx).map(|s| s.as_str())
    }

    /// Derive a display title from pod name + current container.
    fn display_title(&self) -> String {
        match self.current_container() {
            Some(c) => format!("{} / {}", self.pod_name, c),
            None => self.pod_name.clone(),
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
        // Container selector overlay — consumes keys when visible.
        if self.selector_visible {
            return self.handle_selector_key(key);
        }

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
            // 'c' opens the container selector (only when multiple containers exist).
            KeyCode::Char('c') if self.containers.len() > 1 => {
                self.selector_visible = true;
            }
            KeyCode::Esc | KeyCode::Char('q') => return LogAction::Close,
            _ => {}
        }
        LogAction::None
    }

    fn handle_selector_key(&mut self, key: KeyEvent) -> LogAction {
        match key.code {
            KeyCode::Esc | KeyCode::Char('q') => {
                self.selector_visible = false;
            }
            KeyCode::Up | KeyCode::Char('k') => {
                if self.containers.is_empty() {
                    return LogAction::None;
                }
                self.container_idx = if self.container_idx == 0 {
                    self.containers.len() - 1
                } else {
                    self.container_idx - 1
                };
            }
            KeyCode::Down | KeyCode::Char('j') => {
                if !self.containers.is_empty() {
                    self.container_idx = (self.container_idx + 1) % self.containers.len();
                }
            }
            KeyCode::Enter => {
                self.selector_visible = false;
                if let Some(name) = self.current_container() {
                    let name = name.to_owned();
                    // Clear current log buffer — caller will re-stream.
                    self.model = LogModel::new();
                    self.scroll = 0;
                    return LogAction::SwitchContainer(name);
                }
            }
            _ => {}
        }
        LogAction::None
    }

    fn handle_filter_key(&mut self, key: KeyEvent) -> LogAction {
        match key.code {
            KeyCode::Enter => {
                let pattern = self.filter_input.trim().to_owned();
                match self.model.set_filter(if pattern.is_empty() {
                    None
                } else {
                    Some(&pattern)
                }) {
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
            KeyCode::Backspace => {
                self.filter_input.pop();
            }
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

        // Container selector overlay on top (when active).
        if self.selector_visible {
            self.render_container_selector(frame, area);
        }
    }

    fn render_container_selector(&self, frame: &mut Frame, area: Rect) {
        use ratatui::widgets::{Block, Borders, Clear, List, ListItem, ListState};

        if self.containers.is_empty() {
            return;
        }

        // Size: max 12 lines tall, 36 cols wide, anchored top-right.
        let height = (self.containers.len() as u16 + 2).min(14).min(area.height);
        let width = 38u16.min(area.width);
        let popup = Rect {
            x: area.x + area.width.saturating_sub(width),
            y: area.y,
            width,
            height,
        };

        frame.render_widget(ratatui::widgets::Clear, popup);

        let items: Vec<ListItem> = self
            .containers
            .iter()
            .enumerate()
            .map(|(i, name)| {
                let style = if i == self.container_idx {
                    Style::default()
                        .fg(Color::Yellow)
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default()
                };
                ListItem::new(format!(" {name} ")).style(style)
            })
            .collect();

        let block = Block::default()
            .title(" Containers  ↑↓ select  ⏎ switch  Esc cancel ")
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Cyan));

        let list = List::new(items).block(block);

        let mut state = ListState::default();
        state.select(Some(self.container_idx));
        frame.render_stateful_widget(list, popup, &mut state);
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
        let container_hint = if self.containers.len() > 1 {
            format!(" [c={}/{}]", self.container_idx + 1, self.containers.len())
        } else {
            String::new()
        };

        let block = Block::default()
            .title(format!(
                " Logs: {}{}{}{} ",
                self.display_title(),
                live_indicator,
                count_label,
                container_hint
            ))
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

        let hl_style = base_style
            .bg(Color::Yellow)
            .fg(Color::Black)
            .add_modifier(Modifier::BOLD);
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
                let cursor = if self.filter_input.is_empty() {
                    "█"
                } else {
                    ""
                };
                (
                    "Filter » ",
                    format!("{}{}", self.filter_input, cursor),
                    Color::Yellow,
                )
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
                Span::styled(
                    content,
                    Style::default()
                        .fg(Color::White)
                        .add_modifier(Modifier::BOLD),
                ),
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
        LogLevel::Warn => Style::default().fg(Color::Yellow),
        LogLevel::Info => Style::default().fg(Color::White),
        LogLevel::Debug => Style::default().fg(Color::DarkGray),
        LogLevel::Trace => Style::default().fg(Color::DarkGray),
        LogLevel::Unknown => Style::default(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_view() -> LogView {
        LogView::new("test-pod", vec!["app".to_owned(), "sidecar".to_owned()])
    }

    fn view_with_lines(lines: &[&str]) -> LogView {
        let mut v = make_view();
        for l in lines {
            v.push(*l, None);
        }
        v
    }

    #[test]
    fn push_increases_model_len() {
        let mut v = make_view();
        v.push("hello", None);
        v.push("world", None);
        assert_eq!(v.model.len(), 2);
    }

    #[test]
    fn close_on_q() {
        let mut v = make_view();
        let action = v.handle_key(KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE));
        assert_eq!(action, LogAction::Close);
    }

    #[test]
    fn close_on_esc_in_normal_mode() {
        let mut v = make_view();
        let action = v.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        assert_eq!(action, LogAction::Close);
    }

    #[test]
    fn enter_filter_mode_on_slash() {
        let mut v = make_view();
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
        // Esc inside filter mode clears the filter and returns to Normal.
        v.handle_key(KeyEvent::new(KeyCode::Char('/'), KeyModifiers::NONE));
        v.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        assert!(!v.model.is_filtered());
    }

    #[test]
    fn toggle_timestamps_with_t() {
        let mut v = make_view();
        assert!(!v.model.show_timestamps);
        v.handle_key(KeyEvent::new(KeyCode::Char('t'), KeyModifiers::NONE));
        assert!(v.model.show_timestamps);
    }

    #[test]
    fn container_selector_opens_with_c() {
        let mut v = make_view();
        assert!(!v.selector_visible);
        v.handle_key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::NONE));
        assert!(v.selector_visible);
    }

    #[test]
    fn container_selector_cycles_down() {
        let mut v = make_view(); // has ["app", "sidecar"]
        v.handle_key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::NONE));
        assert_eq!(v.container_idx, 0);
        v.handle_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
        assert_eq!(v.container_idx, 1);
    }

    #[test]
    fn container_selector_enter_emits_switch_action() {
        let mut v = make_view();
        v.handle_key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::NONE));
        // Select second container.
        v.handle_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
        let action = v.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert_eq!(action, LogAction::SwitchContainer("sidecar".to_owned()));
        assert!(!v.selector_visible);
    }

    #[test]
    fn current_container_returns_selected() {
        let v = make_view();
        assert_eq!(v.current_container(), Some("app"));
    }

    #[test]
    fn single_container_no_selector() {
        let mut v = LogView::new("pod", vec!["only".to_owned()]);
        // 'c' should NOT open the selector when only one container.
        v.handle_key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::NONE));
        assert!(!v.selector_visible);
    }
}
