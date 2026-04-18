//! Unified command history — Phase 16.2.
//!
//! Tracks all commands entered via the k7s CLI and TUI, persisted across
//! sessions to the XDG state directory as `command_history.json`.
//!
//! # Storage
//!
//! History is stored as a JSON array of [`HistoryEntry`] objects at
//! `~/.local/state/k7s/command_history.json`.  The file is rewritten on
//! each [`CommandHistory::push`] call.  The list is capped at
//! [`MAX_ENTRIES`]; oldest entries are evicted when the cap is exceeded.
//!
//! # Thread Safety
//!
//! `CommandHistory` is **not** `Sync` — it is intended to be owned by the
//! main application thread (CLI or TUI event loop) with exclusive access.
//! The history file should never be written concurrently from two processes;
//! last-writer wins if both the CLI and a TUI session happen to run at once.

use std::fs;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Maximum number of history entries to retain on disk.
pub const MAX_ENTRIES: usize = 1000;

/// Where a history entry originated.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum HistorySource {
    /// Command entered via the `k7s` CLI (e.g. `k7s get pods`).
    Cli,
    /// Action performed from inside the TUI (navigation, dialogs, etc.).
    Tui,
}

impl std::fmt::Display for HistorySource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Cli => write!(f, "cli"),
            Self::Tui => write!(f, "tui"),
        }
    }
}

/// A single entry in the unified command history.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HistoryEntry {
    /// Monotonically increasing sequence number (1-based).
    pub id: u64,
    /// Wall-clock time when the command was issued (UTC).
    pub timestamp: DateTime<Utc>,
    /// Where the command originated.
    pub source: HistorySource,
    /// Human-readable command string.
    ///
    /// For CLI commands this is the full `kubectl`-equivalent argument list,
    /// e.g. `"get pods -n default"`.  For TUI actions it describes the
    /// action, e.g. `"navigate:pods"` or `"delete:pod/my-pod (default)"`.
    pub command: String,
    /// Active Kubernetes context at the time, if known.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context: Option<String>,
    /// Active namespace at the time, if known.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub namespace: Option<String>,
    /// Whether the command completed successfully.
    pub success: bool,
}

/// Persistent, unified command history.
///
/// Loaded once at startup via [`CommandHistory::load`], entries are appended
/// in memory and flushed to disk after every [`push`][Self::push].
pub struct CommandHistory {
    entries: Vec<HistoryEntry>,
    path: PathBuf,
    next_id: u64,
}

impl CommandHistory {
    /// Load history from `state_dir/command_history.json`.
    ///
    /// Creates the directory hierarchy if it does not exist.
    /// Silently starts with an empty history when the file is absent,
    /// unreadable, or contains invalid JSON — a corrupt history file must
    /// never prevent k7s from starting.
    pub fn load(state_dir: &Path) -> Self {
        let path = state_dir.join("command_history.json");

        let entries: Vec<HistoryEntry> = fs::read(&path)
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default();

        let next_id = entries.iter().map(|e| e.id).max().unwrap_or(0) + 1;

        Self {
            entries,
            path,
            next_id,
        }
    }

    /// Create an empty in-memory history that is never persisted.
    ///
    /// Useful for unit tests and contexts where no state directory is
    /// available.
    pub fn in_memory() -> Self {
        Self {
            entries: Vec::new(),
            path: PathBuf::from("/dev/null"),
            next_id: 1,
        }
    }

    /// Append a new entry and persist to disk.
    ///
    /// Assigns the next sequence ID, records the current UTC timestamp, and
    /// trims the list to [`MAX_ENTRIES`] before saving.
    pub fn push(
        &mut self,
        source: HistorySource,
        command: impl Into<String>,
        context: Option<String>,
        namespace: Option<String>,
        success: bool,
    ) {
        let entry = HistoryEntry {
            id: self.next_id,
            timestamp: Utc::now(),
            source,
            command: command.into(),
            context,
            namespace,
            success,
        };
        self.next_id += 1;
        self.entries.push(entry);

        // Evict oldest entries when the cap is exceeded.
        if self.entries.len() > MAX_ENTRIES {
            let excess = self.entries.len() - MAX_ENTRIES;
            self.entries.drain(0..excess);
        }

        self.save();
    }

    /// Return all entries, oldest first.
    pub fn list(&self) -> &[HistoryEntry] {
        &self.entries
    }

    /// Return the most recent `limit` entries, **newest first**.
    pub fn recent(&self, limit: usize) -> Vec<&HistoryEntry> {
        self.entries.iter().rev().take(limit).collect()
    }

    /// Return the Nth-last entry (1-based, 1 = most recent).
    ///
    /// Returns `None` when there are fewer than `n` entries or `n == 0`.
    pub fn nth_last(&self, n: usize) -> Option<&HistoryEntry> {
        if n == 0 {
            return None;
        }
        let idx = self.entries.len().checked_sub(n)?;
        self.entries.get(idx)
    }

    /// The most recent entry, if any.  Equivalent to `nth_last(1)`.
    pub fn last(&self) -> Option<&HistoryEntry> {
        self.nth_last(1)
    }

    /// Number of history entries currently stored.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// `true` when the history contains no entries.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    // ─── Private ──────────────────────────────────────────────────────────────

    fn save(&self) {
        if let Some(parent) = self.path.parent() {
            if let Err(e) = fs::create_dir_all(parent) {
                tracing::warn!(
                    path = %parent.display(),
                    error = %e,
                    "could not create state dir for history"
                );
                return;
            }
        }

        match serde_json::to_vec_pretty(&self.entries) {
            Ok(bytes) => {
                if let Err(e) = fs::write(&self.path, &bytes) {
                    tracing::warn!(
                        path = %self.path.display(),
                        error = %e,
                        "could not write command history"
                    );
                }
            }
            Err(e) => {
                tracing::error!(error = %e, "could not serialise command history");
            }
        }
    }
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn tmp_history() -> (CommandHistory, TempDir) {
        let dir = TempDir::new().unwrap();
        let h = CommandHistory::load(dir.path());
        (h, dir)
    }

    fn push_cli(h: &mut CommandHistory, cmd: &str) {
        h.push(HistorySource::Cli, cmd, None, None, true);
    }

    // ── basic operations ──────────────────────────────────────────────────────

    #[test]
    fn empty_on_fresh_dir() {
        let (h, _dir) = tmp_history();
        assert!(h.is_empty());
        assert_eq!(h.len(), 0);
        assert!(h.last().is_none());
    }

    #[test]
    fn push_increases_len() {
        let (mut h, _dir) = tmp_history();
        push_cli(&mut h, "get pods");
        assert_eq!(h.len(), 1);
        push_cli(&mut h, "get nodes");
        assert_eq!(h.len(), 2);
    }

    #[test]
    fn ids_are_monotonically_increasing() {
        let (mut h, _dir) = tmp_history();
        push_cli(&mut h, "get pods");
        push_cli(&mut h, "get nodes");
        let ids: Vec<u64> = h.list().iter().map(|e| e.id).collect();
        assert_eq!(ids, vec![1, 2]);
    }

    #[test]
    fn last_returns_most_recent() {
        let (mut h, _dir) = tmp_history();
        push_cli(&mut h, "get pods");
        push_cli(&mut h, "get nodes");
        assert_eq!(h.last().unwrap().command, "get nodes");
    }

    #[test]
    fn nth_last_indexing() {
        let (mut h, _dir) = tmp_history();
        push_cli(&mut h, "a");
        push_cli(&mut h, "b");
        push_cli(&mut h, "c");
        assert_eq!(h.nth_last(1).unwrap().command, "c");
        assert_eq!(h.nth_last(2).unwrap().command, "b");
        assert_eq!(h.nth_last(3).unwrap().command, "a");
        assert!(h.nth_last(4).is_none());
        assert!(h.nth_last(0).is_none());
    }

    #[test]
    fn recent_returns_newest_first() {
        let (mut h, _dir) = tmp_history();
        for cmd in &["a", "b", "c", "d"] {
            push_cli(&mut h, cmd);
        }
        let recent: Vec<&str> = h.recent(3).iter().map(|e| e.command.as_str()).collect();
        assert_eq!(recent, vec!["d", "c", "b"]);
    }

    #[test]
    fn recent_with_limit_larger_than_entries() {
        let (mut h, _dir) = tmp_history();
        push_cli(&mut h, "x");
        assert_eq!(h.recent(100).len(), 1);
    }

    // ── eviction ──────────────────────────────────────────────────────────────

    #[test]
    fn entries_capped_at_max() {
        let (mut h, _dir) = tmp_history();
        for i in 0..=(MAX_ENTRIES + 5) {
            push_cli(&mut h, &format!("cmd-{i}"));
        }
        assert_eq!(h.len(), MAX_ENTRIES);
        // The oldest entry should have been evicted; the newest is still there.
        assert_eq!(
            h.last().unwrap().command,
            format!("cmd-{}", MAX_ENTRIES + 5)
        );
    }

    // ── persistence ───────────────────────────────────────────────────────────

    #[test]
    fn history_survives_reload() {
        let dir = TempDir::new().unwrap();
        {
            let mut h = CommandHistory::load(dir.path());
            push_cli(&mut h, "get pods");
            push_cli(&mut h, "describe pod my-pod");
        }
        // Re-load from the same directory.
        let h2 = CommandHistory::load(dir.path());
        assert_eq!(h2.len(), 2);
        assert_eq!(h2.nth_last(1).unwrap().command, "describe pod my-pod");
        assert_eq!(h2.nth_last(2).unwrap().command, "get pods");
    }

    #[test]
    fn corrupt_file_starts_empty() {
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join("command_history.json"), b"not valid json").unwrap();
        let h = CommandHistory::load(dir.path());
        assert!(h.is_empty());
    }

    // ── metadata fields ───────────────────────────────────────────────────────

    #[test]
    fn entry_stores_context_and_namespace() {
        let (mut h, _dir) = tmp_history();
        h.push(
            HistorySource::Cli,
            "get pods",
            Some("prod-ctx".into()),
            Some("kube-system".into()),
            true,
        );
        let e = h.last().unwrap();
        assert_eq!(e.context.as_deref(), Some("prod-ctx"));
        assert_eq!(e.namespace.as_deref(), Some("kube-system"));
        assert!(e.success);
    }

    #[test]
    fn failed_entry_records_success_false() {
        let (mut h, _dir) = tmp_history();
        h.push(HistorySource::Cli, "delete pod bad", None, None, false);
        assert!(!h.last().unwrap().success);
    }

    #[test]
    fn source_variants_serialise() {
        assert_eq!(HistorySource::Cli.to_string(), "cli");
        assert_eq!(HistorySource::Tui.to_string(), "tui");
    }
}
