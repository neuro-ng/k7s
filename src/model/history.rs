//! Navigation history — Phase 5.12.
//!
//! Tracks the sequence of resource types (aliases) the user has visited,
//! enabling `[` (back), `]` (forward), and `-` (last) navigation matching k9s behaviour.
//!
//! # Semantics
//!
//! The history is a bounded list of aliases with a cursor pointing at the
//! *current* position:
//!
//! ```text
//! entries: ["pods", "deployments", "nodes"]
//!                                    ↑ cursor = 2
//! ```
//!
//! * [`back`]    — move cursor left, return the entry at the new position.
//! * [`forward`] — move cursor right, return the entry at the new position.
//! * [`last`]    — toggle to the *previous* position (like `cd -` in shells).
//! * [`push`]    — add a new entry; entries beyond the cursor are discarded.
//!
//! # k9s Reference
//! `internal/model/history.go`

/// Maximum number of history entries to retain.
const MAX_ENTRIES: usize = 50;

/// Navigation history for resource-type aliases.
///
/// The cursor tracks the *currently displayed* entry.  Callers push an alias
/// each time the user navigates to a new resource type, and can step through
/// the list with [`back`][Self::back] / [`forward`][Self::forward].
#[derive(Debug, Default, Clone)]
pub struct NavHistory {
    /// Ordered list of visited resource aliases, oldest first.
    entries: Vec<String>,
    /// Index of the currently-displayed entry.  `None` when the list is empty.
    cursor: Option<usize>,
}

impl NavHistory {
    pub fn new() -> Self {
        Self::default()
    }

    /// Push a new alias onto the history.
    ///
    /// * If the alias is the same as the current entry it is *not* duplicated.
    /// * Any entries forward of the current cursor are discarded (like a browser).
    /// * The list is trimmed to [`MAX_ENTRIES`] oldest entries after insertion.
    pub fn push(&mut self, alias: impl Into<String>) {
        let alias = alias.into();

        // Skip if we'd create a duplicate at the cursor position.
        if let Some(cur) = self.cursor {
            if self.entries.get(cur) == Some(&alias) {
                return;
            }
        }

        // Discard everything after the cursor.
        if let Some(cur) = self.cursor {
            self.entries.truncate(cur + 1);
        }

        self.entries.push(alias);

        // Trim oldest entries if we've grown too large.
        if self.entries.len() > MAX_ENTRIES {
            let drop = self.entries.len() - MAX_ENTRIES;
            self.entries.drain(0..drop);
        }

        self.cursor = Some(self.entries.len() - 1);
    }

    /// Move one step backward in history.
    ///
    /// Returns the alias at the new cursor position, or `None` if already at
    /// the beginning of the history.
    pub fn back(&mut self) -> Option<&str> {
        let cur = self.cursor?;
        if cur == 0 {
            return None;
        }
        self.cursor = Some(cur - 1);
        self.entries.get(cur - 1).map(|s| s.as_str())
    }

    /// Move one step forward in history.
    ///
    /// Returns the alias at the new cursor position, or `None` if already at
    /// the most-recent entry.
    pub fn forward(&mut self) -> Option<&str> {
        let cur = self.cursor?;
        let next = cur + 1;
        if next >= self.entries.len() {
            return None;
        }
        self.cursor = Some(next);
        self.entries.get(next).map(|s| s.as_str())
    }

    /// Toggle to the *previous* cursor position (like `cd -`).
    ///
    /// On the first call after a `push` this is equivalent to [`back`][Self::back].
    /// Subsequent calls without an intervening `push` toggle between the two
    /// positions.  Returns `None` when there is no previous position.
    pub fn last(&mut self) -> Option<&str> {
        let cur = self.cursor?;
        if cur == 0 {
            return None;
        }
        self.cursor = Some(cur - 1);
        self.entries.get(cur - 1).map(|s| s.as_str())
    }

    /// Current alias (the entry at the cursor position), if any.
    pub fn current(&self) -> Option<&str> {
        self.cursor
            .and_then(|i| self.entries.get(i))
            .map(|s| s.as_str())
    }

    /// `true` when there is at least one entry behind the cursor.
    pub fn can_go_back(&self) -> bool {
        self.cursor.is_some_and(|i| i > 0)
    }

    /// `true` when there is at least one entry ahead of the cursor.
    pub fn can_go_forward(&self) -> bool {
        self.cursor.is_some_and(|i| i + 1 < self.entries.len())
    }

    /// Breadcrumb trail — the last few entries up to and including the cursor.
    ///
    /// Returns at most `max` entries, newest-first from the cursor position.
    pub fn trail(&self, max: usize) -> Vec<&str> {
        let Some(cur) = self.cursor else {
            return Vec::new();
        };
        let start = cur.saturating_sub(max - 1);
        self.entries[start..=cur]
            .iter()
            .map(|s| s.as_str())
            .collect()
    }
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn history_of(items: &[&str]) -> NavHistory {
        let mut h = NavHistory::new();
        for item in items {
            h.push(*item);
        }
        h
    }

    #[test]
    fn push_advances_cursor() {
        let h = history_of(&["pods", "nodes"]);
        assert_eq!(h.current(), Some("nodes"));
    }

    #[test]
    fn push_deduplicates_current() {
        let mut h = history_of(&["pods"]);
        h.push("pods"); // same as current — should be ignored
        assert_eq!(h.entries.len(), 1);
    }

    #[test]
    fn back_returns_previous() {
        let mut h = history_of(&["pods", "nodes"]);
        assert_eq!(h.back(), Some("pods"));
        assert_eq!(h.current(), Some("pods"));
    }

    #[test]
    fn back_at_start_returns_none() {
        let mut h = history_of(&["pods"]);
        assert_eq!(h.back(), None);
        assert_eq!(h.current(), Some("pods")); // cursor unchanged
    }

    #[test]
    fn forward_after_back() {
        let mut h = history_of(&["pods", "nodes", "deploys"]);
        h.back(); // → nodes
        h.back(); // → pods
        assert_eq!(h.forward(), Some("nodes"));
        assert_eq!(h.forward(), Some("deploys"));
    }

    #[test]
    fn forward_at_end_returns_none() {
        let mut h = history_of(&["pods", "nodes"]);
        assert_eq!(h.forward(), None);
    }

    #[test]
    fn push_discards_forward_history() {
        let mut h = history_of(&["pods", "nodes", "deploys"]);
        h.back(); // → nodes
        h.back(); // → pods
        h.push("services"); // discards [nodes, deploys] ahead of cursor
        assert_eq!(h.current(), Some("services"));
        assert_eq!(h.forward(), None);
        assert_eq!(h.back(), Some("pods"));
    }

    #[test]
    fn last_toggles_to_previous() {
        let mut h = history_of(&["pods", "nodes"]);
        assert_eq!(h.last(), Some("pods"));
    }

    #[test]
    fn last_at_start_returns_none() {
        let mut h = history_of(&["pods"]);
        assert_eq!(h.last(), None);
    }

    #[test]
    fn can_go_back_and_forward() {
        let mut h = history_of(&["pods", "nodes"]);
        assert!(h.can_go_back());
        assert!(!h.can_go_forward());
        h.back();
        assert!(!h.can_go_back());
        assert!(h.can_go_forward());
    }

    #[test]
    fn trail_returns_breadcrumbs() {
        let h = history_of(&["pods", "nodes", "deploys"]);
        let trail = h.trail(3);
        assert_eq!(trail, vec!["pods", "nodes", "deploys"]);
    }

    #[test]
    fn trail_capped_at_max() {
        let h = history_of(&["a", "b", "c", "d", "e"]);
        let trail = h.trail(3);
        assert_eq!(trail, vec!["c", "d", "e"]);
    }

    #[test]
    fn empty_history_trail_is_empty() {
        let h = NavHistory::new();
        assert!(h.trail(5).is_empty());
        assert!(!h.can_go_back());
        assert!(!h.can_go_forward());
    }

    #[test]
    fn history_trimmed_to_max_entries() {
        let mut h = NavHistory::new();
        for i in 0..=(MAX_ENTRIES + 5) {
            h.push(format!("resource{i}"));
        }
        assert_eq!(h.entries.len(), MAX_ENTRIES);
    }
}
