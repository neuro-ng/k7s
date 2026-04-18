//! Key binding and action system.
//!
//! Each key press maps to an `Action` enum variant.  Views register handlers
//! for the actions they care about; unhandled actions fall through to the
//! parent view or are ignored.
//!
//! This separation keeps input handling and business logic decoupled.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

/// A semantic action triggered by a key press.
///
/// Using an enum instead of raw `KeyCode` means views declare intent,
/// not implementation — swapping key bindings only changes the `resolve()` fn.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    /// Quit k7s cleanly.
    Quit,
    /// Open the `:` command prompt.
    CommandPrompt,
    /// Open the `?` help view.
    Help,
    /// Describe the selected resource.
    Describe,
    /// Open the YAML view for the selected resource.
    ViewYaml,
    /// Open the log viewer for the selected resource.
    Logs,
    /// Delete the selected resource (with confirmation).
    Delete,
    /// Scale the selected resource.
    Scale,
    /// Restart the selected workload.
    Restart,
    /// Open a shell inside the selected pod.
    Shell,
    /// Port-forward the selected service/pod.
    PortForward,
    /// Navigate forward (confirm / enter).
    Enter,
    /// Navigate back (escape / previous view).
    Back,
    /// Go back one step in navigation history (`[`).
    HistoryBack,
    /// Go forward one step in navigation history (`]`).
    HistoryForward,
    /// Toggle to the last-visited resource (`-`).
    HistoryLast,
    /// Sort the table by the current column.
    SortColumn,
    /// Move cursor up.
    Up,
    /// Move cursor down.
    Down,
    /// Page up.
    PageUp,
    /// Page down.
    PageDown,
    /// Jump to top of list.
    Top,
    /// Jump to bottom of list.
    Bottom,
    /// Filter/search the current view.
    Filter,
    /// Refresh the current view immediately.
    Refresh,
    /// Copy selected resource name to clipboard.
    Copy,
    /// Toggle namespace scope (all vs current).
    ToggleAllNamespaces,
    /// Open the AI chat window.
    Chat,
    /// Ask the AI to analyse the selected resource.
    AiAnalyse,
    /// Set/update the container image for a workload.
    SetImage,
    /// Scan the selected image for vulnerabilities.
    VulnScan,
    /// An action that has no semantic mapping (unhandled key press).
    Unhandled(KeyCode),
}

/// Resolve a crossterm `KeyEvent` to a k7s `Action`.
///
/// All key bindings are defined here.  No magic, no config files for now —
/// add configurability in Phase 9.
pub fn resolve(event: &KeyEvent) -> Action {
    use KeyCode::*;
    use KeyModifiers as Mod;

    let ctrl = event.modifiers.contains(Mod::CONTROL);

    match (event.code, ctrl) {
        // Quit
        (Char('q'), false) | (Char('Q'), false) => Action::Quit,
        (Char('c'), true) => Action::Quit,

        // Navigation
        (Char(':'), false) => Action::CommandPrompt,
        (Char('?'), false) => Action::Help,
        (Enter, false) => Action::Enter,
        (Esc, false) => Action::Back,
        (Char('['), false) | (Backspace, false) => Action::HistoryBack,
        (Char(']'), false) => Action::HistoryForward,
        (Char('-'), false) => Action::HistoryLast,

        // Cursor movement
        (Up, false) | (Char('k'), false) => Action::Up,
        (Down, false) | (Char('j'), false) => Action::Down,
        (PageUp, false) | (Char('u'), true) => Action::PageUp,
        (PageDown, false) | (Char('d'), true) => Action::PageDown,
        (Home, false) | (Char('g'), false) => Action::Top,
        (End, false) | (Char('G'), false) => Action::Bottom,

        // Resource operations
        (Char('d'), false) => Action::Describe,
        (Char('y'), false) => Action::ViewYaml,
        (Char('l'), false) => Action::Logs,
        (Delete, false) | (Char('D'), false) => Action::Delete,
        (Char('s'), false) => Action::Scale,
        (Char('r'), false) => Action::Restart,
        (Char('e'), false) => Action::Shell,
        (Char('f'), false) => Action::PortForward,
        (Char('c'), false) => Action::Copy,
        (Char('i'), false) => Action::SetImage,
        (Char('v'), false) => Action::VulnScan,
        (Char('a'), false) => Action::ToggleAllNamespaces,

        // Utility
        (Char('/'), false) => Action::Filter,
        (F(5), false) => Action::Refresh,

        // AI
        (Char(' '), false) => Action::Chat, // space opens chat
        (Char('A'), false) => Action::AiAnalyse,

        (code, _) => Action::Unhandled(code),
    }
}

/// A hint shown in the key hints bar at the bottom of the screen.
#[derive(Debug, Clone)]
pub struct KeyHint {
    pub key: &'static str,
    pub description: &'static str,
}

/// Default hints shown when browsing a resource list.
pub const LIST_HINTS: &[KeyHint] = &[
    KeyHint {
        key: "↑↓/jk",
        description: "Move",
    },
    KeyHint {
        key: "⏎",
        description: "Select",
    },
    KeyHint {
        key: "[/]",
        description: "History",
    },
    KeyHint {
        key: "-",
        description: "Last",
    },
    KeyHint {
        key: "d",
        description: "Describe",
    },
    KeyHint {
        key: "l",
        description: "Logs",
    },
    KeyHint {
        key: "D",
        description: "Delete",
    },
    KeyHint {
        key: ":",
        description: "Command",
    },
    KeyHint {
        key: "?",
        description: "Help",
    },
    KeyHint {
        key: "q",
        description: "Quit",
    },
];

/// Format hints as a single status-bar string.
pub fn format_hints(hints: &[KeyHint]) -> String {
    hints
        .iter()
        .map(|h| format!("  {} {}", h.key, h.description))
        .collect::<Vec<_>>()
        .join("  ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyEventKind, KeyEventState};

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent {
            code,
            modifiers: KeyModifiers::NONE,
            kind: KeyEventKind::Press,
            state: KeyEventState::NONE,
        }
    }

    fn ctrl_key(code: KeyCode) -> KeyEvent {
        KeyEvent {
            code,
            modifiers: KeyModifiers::CONTROL,
            kind: KeyEventKind::Press,
            state: KeyEventState::NONE,
        }
    }

    #[test]
    fn q_resolves_to_quit() {
        assert_eq!(resolve(&key(KeyCode::Char('q'))), Action::Quit);
    }

    #[test]
    fn ctrl_c_resolves_to_quit() {
        assert_eq!(resolve(&ctrl_key(KeyCode::Char('c'))), Action::Quit);
    }

    #[test]
    fn colon_resolves_to_command_prompt() {
        assert_eq!(resolve(&key(KeyCode::Char(':'))), Action::CommandPrompt);
    }

    #[test]
    fn d_resolves_to_describe() {
        assert_eq!(resolve(&key(KeyCode::Char('d'))), Action::Describe);
    }

    #[test]
    fn bracket_resolves_to_history_back() {
        assert_eq!(resolve(&key(KeyCode::Char('['))), Action::HistoryBack);
    }

    #[test]
    fn close_bracket_resolves_to_history_forward() {
        assert_eq!(resolve(&key(KeyCode::Char(']'))), Action::HistoryForward);
    }

    #[test]
    fn dash_resolves_to_history_last() {
        assert_eq!(resolve(&key(KeyCode::Char('-'))), Action::HistoryLast);
    }

    #[test]
    fn esc_resolves_to_back() {
        assert_eq!(resolve(&key(KeyCode::Esc)), Action::Back);
    }

    #[test]
    fn unknown_key_is_unhandled() {
        assert!(matches!(
            resolve(&key(KeyCode::Char('X'))),
            Action::Unhandled(_)
        ));
    }

    #[test]
    fn format_hints_non_empty() {
        let s = format_hints(LIST_HINTS);
        assert!(s.contains("Quit"));
        assert!(s.contains("Describe"));
    }
}
