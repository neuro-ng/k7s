//! AI chat window widget.
//!
//! Renders a split-pane view:
//!   - Top: scrollable conversation history
//!   - Bottom: single-line input field
//!   - Right edge: token usage bar
//!
//! The widget is deliberately pure-display: it owns the input buffer and
//! scroll state, but the actual AI call is dispatched by the view layer.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph, Scrollbar, ScrollbarOrientation, ScrollbarState, Wrap};
use ratatui::Frame;

use crate::ai::provider::Role;

/// A displayable message in the chat window.
#[derive(Debug, Clone)]
pub struct ChatMessage {
    pub role:    Role,
    pub content: String,
}

/// What the caller should do after a key press.
#[derive(Debug, Clone, PartialEq)]
pub enum ChatAction {
    /// Submit the current input to the AI.
    Submit(String),
    /// User pressed Esc / q — close the chat window.
    Close,
    /// No action needed.
    None,
}

/// State for the AI chat widget.
pub struct ChatWidget {
    /// Conversation messages for display.
    pub messages: Vec<ChatMessage>,
    /// Current input buffer.
    pub input:    String,
    /// Vertical scroll offset for the message list.
    scroll:       usize,
    /// Token usage (0–100 percent) for the progress bar.
    pub token_pct: u8,
    /// Whether the AI is currently generating a response.
    pub loading:  bool,
}

impl ChatWidget {
    pub fn new() -> Self {
        Self {
            messages:  Vec::new(),
            input:     String::new(),
            scroll:    0,
            token_pct: 0,
            loading:   false,
        }
    }

    /// Push a message into the conversation.
    pub fn push_message(&mut self, role: Role, content: String) {
        self.messages.push(ChatMessage { role, content });
        self.scroll_to_bottom();
    }

    /// Feed a key press to the widget.
    pub fn handle_key(&mut self, event: &KeyEvent) -> ChatAction {
        match event.code {
            KeyCode::Esc => ChatAction::Close,

            KeyCode::Enter => {
                if self.input.trim().is_empty() {
                    return ChatAction::None;
                }
                let msg = std::mem::take(&mut self.input);
                ChatAction::Submit(msg)
            }

            KeyCode::Backspace => {
                if event.modifiers.contains(KeyModifiers::CONTROL) {
                    // Ctrl+Backspace: delete last word.
                    let trimmed = self.input.trim_end_matches(char::is_whitespace);
                    let end = trimmed.rfind(char::is_whitespace).map(|i| i + 1).unwrap_or(0);
                    self.input.truncate(end);
                } else {
                    self.input.pop();
                }
                ChatAction::None
            }

            KeyCode::Char(c) => {
                self.input.push(c);
                ChatAction::None
            }

            // Scroll message list.
            KeyCode::Up   | KeyCode::PageUp   => { self.scroll = self.scroll.saturating_sub(1); ChatAction::None }
            KeyCode::Down | KeyCode::PageDown => { self.scroll += 1; ChatAction::None }
            KeyCode::Home => { self.scroll = 0; ChatAction::None }
            KeyCode::End  => { self.scroll_to_bottom(); ChatAction::None }

            _ => ChatAction::None,
        }
    }

    fn scroll_to_bottom(&mut self) {
        // Set a large value; clamping happens during render.
        self.scroll = usize::MAX / 2;
    }

    /// Render the chat window into the given area.
    pub fn render(&mut self, frame: &mut Frame, area: Rect) {
        // Layout: [message pane | token bar] / [input bar]
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Min(3),    // messages
                Constraint::Length(3), // input
            ])
            .split(area);

        let top_chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Min(0), Constraint::Length(4)])
            .split(chunks[0]);

        self.render_messages(frame, top_chunks[0]);
        self.render_token_bar(frame, top_chunks[1]);
        self.render_input(frame, chunks[1]);
    }

    fn render_messages(&mut self, frame: &mut Frame, area: Rect) {
        let mut lines: Vec<Line> = Vec::new();

        for msg in &self.messages {
            let (prefix, style) = match msg.role {
                Role::User      => ("You  › ", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
                Role::Assistant => ("k7s  › ", Style::default().fg(Color::Green)),
                Role::System    => ("sys  › ", Style::default().fg(Color::DarkGray)),
            };

            // First line gets the role prefix.
            let content_lines: Vec<&str> = msg.content.lines().collect();
            for (i, content_line) in content_lines.iter().enumerate() {
                let p = if i == 0 { prefix } else { "       " };
                lines.push(Line::from(vec![
                    Span::styled(p, style),
                    Span::raw(content_line.to_owned()),
                ]));
            }
            lines.push(Line::raw(""));
        }

        if self.loading {
            lines.push(Line::from(vec![
                Span::styled("k7s  › ", Style::default().fg(Color::Green)),
                Span::styled("▋", Style::default().fg(Color::Green).add_modifier(Modifier::SLOW_BLINK)),
            ]));
        }

        let total_lines = lines.len();
        let visible = area.height.saturating_sub(2) as usize; // minus borders

        // Clamp scroll.
        let max_scroll = total_lines.saturating_sub(visible);
        self.scroll = self.scroll.min(max_scroll);

        let paragraph = Paragraph::new(lines)
            .block(Block::default().borders(Borders::ALL).title(" AI Chat "))
            .scroll((self.scroll as u16, 0))
            .wrap(Wrap { trim: false });

        frame.render_widget(paragraph, area);

        // Scrollbar.
        if total_lines > visible {
            let mut sb_state = ScrollbarState::new(max_scroll).position(self.scroll);
            let sb = Scrollbar::new(ScrollbarOrientation::VerticalRight);
            frame.render_stateful_widget(sb, area, &mut sb_state);
        }
    }

    fn render_token_bar(&self, frame: &mut Frame, area: Rect) {
        let filled = ((self.token_pct as usize) * area.height as usize) / 100;
        let color = if self.token_pct >= 90 {
            Color::Red
        } else if self.token_pct >= 70 {
            Color::Yellow
        } else {
            Color::Green
        };

        let mut lines: Vec<Line> = Vec::new();
        for i in (0..area.height as usize).rev() {
            let c = if i < filled { "█" } else { "░" };
            lines.push(Line::from(Span::styled(c, Style::default().fg(color))));
        }

        let bar = Paragraph::new(lines)
            .block(Block::default().borders(Borders::LEFT).title("T"));
        frame.render_widget(bar, area);
    }

    fn render_input(&self, frame: &mut Frame, area: Rect) {
        let cursor = if self.loading { "…" } else { "█" };
        let prompt = Paragraph::new(format!(" › {}{}",
            self.input,
            cursor,
        ))
        .style(Style::default().fg(Color::White))
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(if self.loading { " Thinking… " } else { " Message (Enter to send, Esc to close) " }),
        );
        frame.render_widget(prompt, area);
    }
}

impl Default for ChatWidget {
    fn default() -> Self { Self::new() }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyEventKind, KeyEventState};

    fn press(code: KeyCode) -> KeyEvent {
        KeyEvent {
            code,
            modifiers: KeyModifiers::NONE,
            kind:  KeyEventKind::Press,
            state: KeyEventState::NONE,
        }
    }

    #[test]
    fn typing_builds_input() {
        let mut w = ChatWidget::new();
        w.handle_key(&press(KeyCode::Char('h')));
        w.handle_key(&press(KeyCode::Char('i')));
        assert_eq!(w.input, "hi");
    }

    #[test]
    fn enter_submits_and_clears() {
        let mut w = ChatWidget::new();
        w.input = "hello".to_owned();
        let action = w.handle_key(&press(KeyCode::Enter));
        assert_eq!(action, ChatAction::Submit("hello".to_owned()));
        assert!(w.input.is_empty());
    }

    #[test]
    fn enter_empty_is_noop() {
        let mut w = ChatWidget::new();
        let action = w.handle_key(&press(KeyCode::Enter));
        assert_eq!(action, ChatAction::None);
    }

    #[test]
    fn esc_closes() {
        let mut w = ChatWidget::new();
        assert_eq!(w.handle_key(&press(KeyCode::Esc)), ChatAction::Close);
    }

    #[test]
    fn push_message_appends() {
        let mut w = ChatWidget::new();
        w.push_message(Role::User, "test".to_owned());
        assert_eq!(w.messages.len(), 1);
    }
}
