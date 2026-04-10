//! Command prompt — the `:` input bar for resource navigation.
//!
//! Behaviour mirrors k9s's command input:
//! - `:pods` navigates to the pods view
//! - `:ns my-namespace` switches namespace
//! - `:ctx my-context` switches cluster context
//! - `Esc` or empty submit cancels the prompt

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

/// The state of the command prompt input field.
#[derive(Debug, Clone, Default)]
pub struct Prompt {
    /// The current buffer (what the user has typed after `:`).
    pub buffer: String,
    /// Whether the prompt is currently visible.
    pub active: bool,
}

/// What the caller should do after the user presses Enter.
#[derive(Debug, Clone, PartialEq)]
pub enum PromptSubmit {
    /// Navigate to a resource view. e.g. "pods", "po", "dp", "nodes".
    Navigate(String),
    /// Switch to a namespace. `:ns <name>` or `:ns -` for all.
    Namespace(Option<String>),
    /// Switch cluster context. `:ctx <name>`.
    Context(String),
    /// Filter the current view.
    Filter(String),
    /// User cancelled (empty submit or Esc).
    Cancel,
}

impl Prompt {
    pub fn new() -> Self {
        Self::default()
    }

    /// Activate the prompt (user pressed `:`).
    pub fn activate(&mut self) {
        self.active = true;
        self.buffer.clear();
    }

    /// Deactivate without submitting.
    pub fn cancel(&mut self) {
        self.active = false;
        self.buffer.clear();
    }

    /// Feed a key event to the prompt.
    ///
    /// Returns `Some(PromptSubmit)` when the user presses Enter or Esc.
    /// Returns `None` while the user is still typing.
    pub fn handle_key(&mut self, event: &KeyEvent) -> Option<PromptSubmit> {
        if !self.active {
            return None;
        }

        match event.code {
            KeyCode::Esc => {
                self.cancel();
                Some(PromptSubmit::Cancel)
            }
            KeyCode::Enter => {
                let cmd = self.buffer.trim().to_lowercase();
                self.active = false;
                self.buffer.clear();
                Some(parse_command(&cmd))
            }
            KeyCode::Backspace => {
                if event.modifiers.contains(KeyModifiers::CONTROL) {
                    // Ctrl+Backspace: clear to previous word.
                    let trimmed = self.buffer.trim_end();
                    let new_end = trimmed
                        .rfind(char::is_whitespace)
                        .map(|i| i + 1)
                        .unwrap_or(0);
                    self.buffer.truncate(new_end);
                } else {
                    self.buffer.pop();
                }
                None
            }
            KeyCode::Char(c) => {
                self.buffer.push(c);
                None
            }
            _ => None,
        }
    }

    /// The display string for the prompt bar.
    ///
    /// Returns `None` when the prompt is inactive.
    pub fn display(&self) -> Option<String> {
        if self.active {
            Some(format!(":{}", self.buffer))
        } else {
            None
        }
    }
}

/// Parse a submitted command string into a `PromptSubmit`.
fn parse_command(cmd: &str) -> PromptSubmit {
    if cmd.is_empty() {
        return PromptSubmit::Cancel;
    }

    let parts: Vec<&str> = cmd.splitn(2, char::is_whitespace).collect();
    let verb = parts[0];
    let arg = parts.get(1).map(|s| s.trim());

    match verb {
        "ns" | "namespace" => match arg {
            None | Some("") | Some("-") => PromptSubmit::Namespace(None), // all namespaces
            Some(ns) => PromptSubmit::Namespace(Some(ns.to_owned())),
        },
        "ctx" | "context" => match arg {
            Some(ctx) if !ctx.is_empty() => PromptSubmit::Context(ctx.to_owned()),
            _ => PromptSubmit::Cancel,
        },
        "filter" | "/" => match arg {
            Some(f) if !f.is_empty() => PromptSubmit::Filter(f.to_owned()),
            _ => PromptSubmit::Cancel,
        },
        // Anything else: treat as resource navigation.
        resource => PromptSubmit::Navigate(resource.to_owned()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyEventKind, KeyEventState};

    fn press(code: KeyCode) -> KeyEvent {
        KeyEvent {
            code,
            modifiers: KeyModifiers::NONE,
            kind: KeyEventKind::Press,
            state: KeyEventState::NONE,
        }
    }

    #[test]
    fn activate_clears_buffer() {
        let mut p = Prompt::new();
        p.buffer = "old".to_string();
        p.activate();
        assert!(p.buffer.is_empty());
        assert!(p.active);
    }

    #[test]
    fn typing_builds_buffer() {
        let mut p = Prompt::new();
        p.activate();
        p.handle_key(&press(KeyCode::Char('p')));
        p.handle_key(&press(KeyCode::Char('o')));
        p.handle_key(&press(KeyCode::Char('d')));
        assert_eq!(p.buffer, "pod");
    }

    #[test]
    fn enter_submits_navigate() {
        let mut p = Prompt::new();
        p.activate();
        for c in "pods".chars() {
            p.handle_key(&press(KeyCode::Char(c)));
        }
        let result = p.handle_key(&press(KeyCode::Enter));
        assert_eq!(result, Some(PromptSubmit::Navigate("pods".to_string())));
        assert!(!p.active);
    }

    #[test]
    fn esc_returns_cancel() {
        let mut p = Prompt::new();
        p.activate();
        let result = p.handle_key(&press(KeyCode::Esc));
        assert_eq!(result, Some(PromptSubmit::Cancel));
        assert!(!p.active);
    }

    #[test]
    fn parse_ns_command() {
        assert_eq!(
            parse_command("ns default"),
            PromptSubmit::Namespace(Some("default".to_owned()))
        );
        assert_eq!(parse_command("ns -"), PromptSubmit::Namespace(None));
        assert_eq!(parse_command("ns"), PromptSubmit::Namespace(None));
    }

    #[test]
    fn parse_ctx_command() {
        assert_eq!(
            parse_command("ctx prod"),
            PromptSubmit::Context("prod".to_owned())
        );
    }

    #[test]
    fn parse_empty_is_cancel() {
        assert_eq!(parse_command(""), PromptSubmit::Cancel);
    }

    #[test]
    fn display_shows_colon_prefix() {
        let mut p = Prompt::new();
        p.activate();
        p.buffer = "po".to_string();
        assert_eq!(p.display(), Some(":po".to_string()));
    }

    #[test]
    fn inactive_display_is_none() {
        let p = Prompt::new();
        assert_eq!(p.display(), None);
    }
}
