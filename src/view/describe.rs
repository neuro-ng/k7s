//! Describe view — displays `kubectl describe`-style text in a scrollable pane.
//!
//! Used when the user presses `d` on a selected resource to see its full
//! description. Also handles YAML display (same widget, different content).

use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph, Scrollbar, ScrollbarOrientation, ScrollbarState};
use ratatui::Frame;

/// A scrollable read-only text view for describe / YAML output.
pub struct DescribeView {
    /// Display title shown in the border (e.g. "Pod: my-pod").
    pub title: String,
    /// The text content split into lines.
    lines: Vec<String>,
    /// Current vertical scroll offset (line index).
    scroll: usize,
}

impl DescribeView {
    pub fn new(title: impl Into<String>, content: impl Into<String>) -> Self {
        let lines: Vec<String> = content.into().lines().map(str::to_owned).collect();
        Self {
            title: title.into(),
            lines,
            scroll: 0,
        }
    }

    /// Replace the content and reset scroll to top.
    pub fn set_content(&mut self, content: impl Into<String>) {
        self.lines = content.into().lines().map(str::to_owned).collect();
        self.scroll = 0;
    }

    pub fn scroll_up(&mut self) {
        self.scroll = self.scroll.saturating_sub(1);
    }

    pub fn scroll_down(&mut self) {
        if self.scroll + 1 < self.lines.len() {
            self.scroll += 1;
        }
    }

    pub fn page_up(&mut self, amount: usize) {
        self.scroll = self.scroll.saturating_sub(amount);
    }

    pub fn page_down(&mut self, amount: usize) {
        let max = self.lines.len().saturating_sub(1);
        self.scroll = (self.scroll + amount).min(max);
    }

    pub fn top(&mut self) { self.scroll = 0; }

    pub fn bottom(&mut self) {
        self.scroll = self.lines.len().saturating_sub(1);
    }

    /// Number of content lines.
    pub fn line_count(&self) -> usize { self.lines.len() }

    /// Render into the given frame area.
    pub fn render(&self, frame: &mut Frame, area: Rect) {
        let block = Block::default()
            .title(format!(" {} ", self.title))
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Cyan));

        let inner = block.inner(area);

        // Build ratatui `Line`s from the text, applying minimal syntax colouring
        // (YAML keys and describe section headers get a distinct colour).
        let styled_lines: Vec<Line> = self
            .lines
            .iter()
            .skip(self.scroll)
            .take(inner.height as usize)
            .map(|l| style_line(l))
            .collect();

        let para = Paragraph::new(styled_lines).block(block);
        frame.render_widget(para, area);

        // Scrollbar on the right edge.
        if self.lines.len() > inner.height as usize {
            let mut scroll_state = ScrollbarState::new(self.lines.len())
                .position(self.scroll);
            let scrollbar = Scrollbar::new(ScrollbarOrientation::VerticalRight);
            frame.render_stateful_widget(scrollbar, area, &mut scroll_state);
        }
    }
}

/// Apply minimal syntax highlighting to a line of describe / YAML output.
fn style_line(line: &str) -> Line<'_> {
    // YAML keys: `key:` or `key: value`
    if let Some(colon_pos) = line.find(':') {
        let before = &line[..colon_pos];
        // Only colour if the prefix is all identifier-like characters (no leading spaces means top-level key).
        if !before.trim_start().is_empty() && before.chars().all(|c| c.is_alphanumeric() || c == '_' || c == '-' || c == ' ') {
            let key_style = Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD);
            let val_style = Style::default().fg(Color::White);
            return Line::from(vec![
                Span::styled(before.to_owned() + ":", key_style),
                Span::styled(line[colon_pos + 1..].to_owned(), val_style),
            ]);
        }
    }

    // Section headers in `kubectl describe` output (all-caps or title-case followed by colon).
    if line.ends_with(':') && !line.starts_with(' ') {
        return Line::from(Span::styled(
            line.to_owned(),
            Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD),
        ));
    }

    Line::from(Span::raw(line.to_owned()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_splits_lines() {
        let v = DescribeView::new("Test", "line1\nline2\nline3");
        assert_eq!(v.line_count(), 3);
    }

    #[test]
    fn scroll_down_bounded_by_content() {
        let mut v = DescribeView::new("Test", "a\nb\nc");
        v.scroll_down();
        v.scroll_down();
        v.scroll_down(); // should not exceed line_count - 1
        assert_eq!(v.scroll, 2);
    }

    #[test]
    fn scroll_up_does_not_underflow() {
        let mut v = DescribeView::new("Test", "a\nb");
        v.scroll_up(); // already at top
        assert_eq!(v.scroll, 0);
    }

    #[test]
    fn set_content_resets_scroll() {
        let mut v = DescribeView::new("Test", "a\nb\nc");
        v.scroll_down();
        assert_eq!(v.scroll, 1);
        v.set_content("x");
        assert_eq!(v.scroll, 0);
    }

    #[test]
    fn page_up_saturates_at_zero() {
        let mut v = DescribeView::new("Test", "a\nb\nc");
        v.page_up(100);
        assert_eq!(v.scroll, 0);
    }
}
