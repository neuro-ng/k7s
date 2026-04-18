//! Command prompt — the `:` input bar for resource navigation.
//!
//! Behaviour mirrors k9s's command input:
//! - `:pods` navigates to the pods view
//! - `:ns my-namespace` switches namespace
//! - `:ctx my-context` switches cluster context
//! - `Tab` cycles through fuzzy-matched suggestions
//! - `Esc` or empty submit cancels the prompt
//!
//! # Phase 5.14 — Fuzzy Autocomplete
//!
//! Callers populate the prompt with a candidate list via [`Prompt::set_candidates`].
//! As the user types, the prompt computes fuzzy matches and exposes them via
//! [`Prompt::suggestions`].  `Tab` advances to the next suggestion and inserts it;
//! `Shift+Tab` goes backwards.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::util::fuzzy_match;

/// The state of the command prompt input field.
#[derive(Debug, Clone, Default)]
pub struct Prompt {
    /// The current buffer (what the user has typed after `:`).
    pub buffer: String,
    /// Whether the prompt is currently visible.
    pub active: bool,
    /// Full candidate list for autocompletion (resource aliases + built-in commands).
    candidates: Vec<String>,
    /// Current fuzzy-matched suggestions in best-first order.
    pub suggestions: Vec<String>,
    /// Index into `suggestions` for the Tab-cycle cursor. `None` = not cycling.
    suggestion_idx: Option<usize>,
    /// The user's raw input before Tab-cycling started (so we can restore it on Esc).
    pre_tab_buffer: Option<String>,
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
    /// Re-run the Nth-last command from unified history (1-based, 1 = most recent).
    ///
    /// Triggered by `:retry [N]` or `!!` (equivalent to `:retry 1`).
    Retry(usize),
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
        self.suggestion_idx = None;
        self.pre_tab_buffer = None;
        self.refresh_suggestions();
    }

    /// Deactivate without submitting.
    pub fn cancel(&mut self) {
        // Restore the pre-tab buffer if the user Esc-ed out of Tab-cycling.
        if let Some(orig) = self.pre_tab_buffer.take() {
            self.buffer = orig;
        }
        self.active = false;
        self.buffer.clear();
        self.suggestions.clear();
        self.suggestion_idx = None;
    }

    /// Populate the candidate list used for fuzzy autocomplete.
    ///
    /// Call this once at startup (or whenever the registry changes) with all
    /// known resource aliases.  The prompt keeps its own copy so it can filter
    /// at input time without re-querying the registry on every key press.
    pub fn set_candidates(&mut self, candidates: Vec<String>) {
        self.candidates = candidates;
        self.refresh_suggestions();
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
                self.suggestions.clear();
                self.suggestion_idx = None;
                self.pre_tab_buffer = None;
                Some(parse_command(&cmd))
            }
            // Tab — advance to next suggestion.
            KeyCode::Tab => {
                self.tab_complete(false);
                None
            }
            // BackTab (Shift+Tab) — go to previous suggestion.
            KeyCode::BackTab => {
                self.tab_complete(true);
                None
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
                // Any edit resets Tab-cycling.
                self.suggestion_idx = None;
                self.pre_tab_buffer = None;
                self.refresh_suggestions();
                None
            }
            KeyCode::Char(c) => {
                self.buffer.push(c);
                // Any new character resets Tab-cycling.
                self.suggestion_idx = None;
                self.pre_tab_buffer = None;
                self.refresh_suggestions();
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

    /// Formatted suggestion hints for the footer bar.
    ///
    /// Returns a short string like `"  pods  po  pod  deployments  …"` showing
    /// the top suggestions (up to `max`), with the currently-selected one
    /// highlighted via `[brackets]`.
    pub fn suggestion_hint(&self, max: usize) -> String {
        if self.suggestions.is_empty() {
            return String::new();
        }
        self.suggestions
            .iter()
            .take(max)
            .enumerate()
            .map(|(i, s)| {
                if Some(i) == self.suggestion_idx {
                    format!("  [{s}]")
                } else {
                    format!("  {s}")
                }
            })
            .collect::<Vec<_>>()
            .join("")
    }

    // ─── Private helpers ──────────────────────────────────────────────────────

    /// Recompute `suggestions` based on the current buffer.
    fn refresh_suggestions(&mut self) {
        // Extract the "verb" part of the buffer (the first token) for matching.
        // For multi-word commands like "ns default" we only match on "ns".
        let query = self.buffer.split_whitespace().next().unwrap_or("");

        let refs: Vec<&str> = self.candidates.iter().map(|s| s.as_str()).collect();
        self.suggestions = fuzzy_match(query, &refs)
            .into_iter()
            .map(|m| m.candidate.to_owned())
            .collect();
    }

    /// Advance (or retreat) through the suggestion list via Tab / Shift+Tab.
    fn tab_complete(&mut self, reverse: bool) {
        if self.suggestions.is_empty() {
            return;
        }

        // Snapshot the raw input before the first Tab press.
        if self.suggestion_idx.is_none() {
            self.pre_tab_buffer = Some(self.buffer.clone());
        }

        let n = self.suggestions.len();
        let next = match self.suggestion_idx {
            None => {
                if reverse {
                    n.saturating_sub(1)
                } else {
                    0
                }
            }
            Some(i) => {
                if reverse {
                    i.checked_sub(1).unwrap_or(n - 1)
                } else {
                    (i + 1) % n
                }
            }
        };

        self.suggestion_idx = Some(next);
        self.buffer = self.suggestions[next].clone();
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
            _ => PromptSubmit::Navigate("ctx".to_owned()), // bare `:ctx` → open context view
        },
        "filter" | "/" => match arg {
            Some(f) if !f.is_empty() => PromptSubmit::Filter(f.to_owned()),
            _ => PromptSubmit::Cancel,
        },
        // Retry: `:retry [N]`, `:!!`, or `!!`
        "retry" | "!!" => {
            let n = arg
                .and_then(|s| s.parse::<usize>().ok())
                .unwrap_or(1)
                .max(1);
            PromptSubmit::Retry(n)
        }
        // Shell-history style `!N` shorthand (e.g. `:!3` → retry 3rd-last).
        s if s.starts_with('!') => {
            let n = s[1..].parse::<usize>().unwrap_or(1).max(1);
            PromptSubmit::Retry(n)
        }
        // Anything else: treat as resource navigation.
        resource => PromptSubmit::Navigate(resource.to_owned()),
    }
}

// ─── Tests ────────────────────────────────────────────────────────────────────

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

    fn prompt_with_candidates() -> Prompt {
        let mut p = Prompt::new();
        p.set_candidates(vec![
            "pods".into(),
            "pod".into(),
            "deployments".into(),
            "deploy".into(),
            "nodes".into(),
            "namespaces".into(),
            "ns".into(),
            "services".into(),
            "svc".into(),
            "ctx".into(),
            "context".into(),
        ]);
        p
    }

    // ── Basic input ───────────────────────────────────────────────────────────

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
    fn parse_ctx_with_arg() {
        assert_eq!(
            parse_command("ctx prod"),
            PromptSubmit::Context("prod".to_owned())
        );
    }

    #[test]
    fn parse_ctx_bare_opens_view() {
        assert_eq!(
            parse_command("ctx"),
            PromptSubmit::Navigate("ctx".to_owned())
        );
    }

    #[test]
    fn parse_empty_is_cancel() {
        assert_eq!(parse_command(""), PromptSubmit::Cancel);
    }

    #[test]
    fn parse_retry_bare() {
        assert_eq!(parse_command("retry"), PromptSubmit::Retry(1));
        assert_eq!(parse_command("!!"), PromptSubmit::Retry(1));
    }

    #[test]
    fn parse_retry_with_n() {
        assert_eq!(parse_command("retry 3"), PromptSubmit::Retry(3));
        assert_eq!(parse_command("!! 5"), PromptSubmit::Retry(5));
    }

    #[test]
    fn parse_bang_n_shorthand() {
        assert_eq!(parse_command("!2"), PromptSubmit::Retry(2));
        assert_eq!(parse_command("!10"), PromptSubmit::Retry(10));
    }

    #[test]
    fn parse_retry_zero_clamps_to_one() {
        // 0 is not a valid position; clamp to 1.
        assert_eq!(parse_command("retry 0"), PromptSubmit::Retry(1));
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

    // ── Autocomplete ──────────────────────────────────────────────────────────

    #[test]
    fn suggestions_populate_on_activate() {
        let mut p = prompt_with_candidates();
        p.activate();
        // Empty buffer → all candidates visible.
        assert!(!p.suggestions.is_empty());
    }

    #[test]
    fn typing_filters_suggestions() {
        let mut p = prompt_with_candidates();
        p.activate();
        p.handle_key(&press(KeyCode::Char('p')));
        p.handle_key(&press(KeyCode::Char('o')));
        // "po" should match at least "pod" and "pods" and they should rank first.
        assert!(!p.suggestions.is_empty());
        // The top two results (prefix matches) must start with "po".
        let top: Vec<_> = p.suggestions.iter().take(2).collect();
        assert!(top.iter().all(|s| s.starts_with("po")));
    }

    #[test]
    fn tab_inserts_first_suggestion() {
        let mut p = prompt_with_candidates();
        p.activate();
        p.handle_key(&press(KeyCode::Char('p')));
        p.handle_key(&press(KeyCode::Char('o')));
        p.handle_key(&press(KeyCode::Tab));
        // Buffer should now be the first suggestion.
        assert!(!p.buffer.is_empty());
        assert!(p.suggestion_idx == Some(0));
    }

    #[test]
    fn tab_cycles_suggestions() {
        let mut p = prompt_with_candidates();
        p.activate();
        p.handle_key(&press(KeyCode::Char('p')));
        p.handle_key(&press(KeyCode::Tab));
        let first = p.buffer.clone();
        p.handle_key(&press(KeyCode::Tab));
        let second = p.buffer.clone();
        // Two consecutive Tabs should give different results (if ≥2 matches).
        if p.suggestions.len() >= 2 {
            assert_ne!(first, second);
        }
    }

    #[test]
    fn typing_after_tab_resets_cycle() {
        let mut p = prompt_with_candidates();
        p.activate();
        p.handle_key(&press(KeyCode::Char('p')));
        p.handle_key(&press(KeyCode::Tab));
        assert!(p.suggestion_idx.is_some());
        // Typing a new character resets the cycle.
        p.handle_key(&press(KeyCode::Char('x')));
        assert!(p.suggestion_idx.is_none());
    }

    #[test]
    fn esc_during_tab_cycle_cancels() {
        let mut p = prompt_with_candidates();
        p.activate();
        p.handle_key(&press(KeyCode::Char('p')));
        p.handle_key(&press(KeyCode::Tab));
        // Esc should cancel and clear everything.
        let result = p.handle_key(&press(KeyCode::Esc));
        assert_eq!(result, Some(PromptSubmit::Cancel));
        assert!(!p.active);
    }

    #[test]
    fn suggestion_hint_brackets_selected() {
        let mut p = prompt_with_candidates();
        p.activate();
        p.handle_key(&press(KeyCode::Char('p')));
        p.handle_key(&press(KeyCode::Tab));
        let hint = p.suggestion_hint(5);
        // The selected suggestion should appear in brackets.
        assert!(hint.contains('['));
    }

    #[test]
    fn backtab_goes_backward() {
        let mut p = prompt_with_candidates();
        p.activate();
        // Get at least 2 suggestions.
        p.handle_key(&press(KeyCode::Tab)); // → index 0
        p.handle_key(&press(KeyCode::Tab)); // → index 1
        let fwd = p.suggestion_idx.unwrap();
        p.handle_key(&press(KeyCode::BackTab)); // → index 0 again
        assert_eq!(p.suggestion_idx, Some(fwd - 1));
    }
}
