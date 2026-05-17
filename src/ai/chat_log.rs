//! Chat history persistence — Phase 26.
//!
//! Saves AI chat sessions as JSON files under
//! `~/.local/state/k7s/chat_logs/`.  Sessions are capped at
//! [`MAX_CHAT_LOGS`] entries; the oldest files are pruned automatically.
//!
//! # Design
//!
//! Each session is a single `<id>.json` file.  IDs are nanosecond-epoch
//! strings (`chat_<u64_hex>`) which sort chronologically and guarantee
//! uniqueness within a single process.  The store is not thread-safe —
//! it is owned by the main TUI thread.

use std::cmp::Reverse;
use std::fs;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::ai::provider::Role;

/// Maximum number of chat sessions to retain on disk.
pub const MAX_CHAT_LOGS: usize = 50;

/// A single message persisted inside a [`ChatLog`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatLogMessage {
    pub role: Role,
    pub content: String,
    pub timestamp: DateTime<Utc>,
}

impl ChatLogMessage {
    pub fn new(role: Role, content: impl Into<String>) -> Self {
        Self {
            role,
            content: content.into(),
            timestamp: Utc::now(),
        }
    }
}

/// A persisted AI chat session.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatLog {
    /// Unique identifier — `chat_<nanoseconds-hex>`.
    pub id: String,
    /// When the session was first created.
    pub created_at: DateTime<Utc>,
    /// When the session was last updated.
    pub updated_at: DateTime<Utc>,
    /// Optional resource scope label (e.g. `"pods/web-abc  ·  default"`).
    pub context_label: Option<String>,
    /// All conversation turns (system messages excluded).
    pub messages: Vec<ChatLogMessage>,
    /// Accumulated token count for this session.
    pub tokens_used: u32,
}

impl ChatLog {
    /// Create a new empty log with a unique ID.
    pub fn new(context_label: Option<String>) -> Self {
        let now = Utc::now();
        // Nanosecond timestamp as hex gives a naturally sortable unique ID.
        let nanos = now
            .timestamp_nanos_opt()
            .unwrap_or(now.timestamp_millis() * 1_000_000);
        let id = format!("chat_{nanos:016x}");
        Self {
            id,
            created_at: now,
            updated_at: now,
            context_label,
            messages: Vec::new(),
            tokens_used: 0,
        }
    }

    /// Number of user + assistant messages (excludes system turns).
    pub fn message_count(&self) -> usize {
        self.messages
            .iter()
            .filter(|m| m.role != Role::System)
            .count()
    }

    /// Short human-readable date string for the browser column.
    pub fn date_label(&self) -> String {
        self.created_at.format("%Y-%m-%d %H:%M").to_string()
    }

    /// Render this session as a Markdown document suitable for export.
    ///
    /// System messages are excluded (they contain internal prompt scaffolding).
    /// Each user and assistant turn is labelled with a timestamp and separated
    /// by horizontal rules so the document is readable in any Markdown viewer.
    pub fn to_markdown(&self) -> String {
        let mut out = String::with_capacity(4096);

        // ── Header ─────────────────────────────────────────────────────────────
        out.push_str("# k7s AI Chat Export\n\n");
        if let Some(label) = &self.context_label {
            out.push_str(&format!("**Context:** {label}  \n"));
        }
        out.push_str(&format!(
            "**Date:** {}  \n",
            self.created_at.format("%Y-%m-%d %H:%M UTC")
        ));
        out.push_str(&format!("**Session:** {}  \n", self.id));
        if self.tokens_used > 0 {
            out.push_str(&format!("**Tokens used:** {}  \n", self.tokens_used));
        }
        out.push_str("\n---\n\n## Conversation\n\n");

        // ── Messages ───────────────────────────────────────────────────────────
        let turns: Vec<_> = self
            .messages
            .iter()
            .filter(|m| m.role != Role::System)
            .collect();

        if turns.is_empty() {
            out.push_str("*No messages in this session.*\n");
            return out;
        }

        for msg in turns {
            let label = match msg.role {
                Role::User => "**User**",
                Role::Assistant => "**Assistant**",
                Role::System => continue,
            };
            out.push_str(&format!(
                "{label} · {}\n\n{}\n\n---\n\n",
                msg.timestamp.format("%H:%M:%S"),
                msg.content.trim()
            ));
        }

        out
    }
}

/// Manages the on-disk chat log store.
pub struct ChatLogStore {
    dir: PathBuf,
}

impl ChatLogStore {
    /// Open (or create) the store at the given directory.
    pub fn open(dir: &Path) -> Self {
        let _ = fs::create_dir_all(dir);
        Self {
            dir: dir.to_owned(),
        }
    }

    /// Convenience constructor using the XDG state directory.
    pub fn default_store() -> Self {
        let dir = crate::config::ConfigDirs::resolve()
            .map(|d| d.state.join("chat_logs"))
            .unwrap_or_else(|_| PathBuf::from("/tmp/k7s_chat_logs"));
        Self::open(&dir)
    }

    fn log_path(&self, id: &str) -> PathBuf {
        self.dir.join(format!("{id}.json"))
    }

    /// Persist a [`ChatLog`] and prune old entries if over [`MAX_CHAT_LOGS`].
    pub fn save(&self, log: &mut ChatLog) {
        log.updated_at = Utc::now();
        let path = self.log_path(&log.id);
        if let Ok(json) = serde_json::to_string_pretty(log) {
            let _ = fs::write(&path, json);
        }
        self.prune();
    }

    /// Load a single [`ChatLog`] by ID.  Returns `None` on any error.
    pub fn load(&self, id: &str) -> Option<ChatLog> {
        let data = fs::read_to_string(self.log_path(id)).ok()?;
        serde_json::from_str(&data).ok()
    }

    /// List all persisted chat logs, sorted newest-first.
    pub fn list(&self) -> Vec<ChatLog> {
        let Ok(rd) = fs::read_dir(&self.dir) else {
            return Vec::new();
        };
        let mut logs: Vec<ChatLog> = rd
            .filter_map(|e| e.ok())
            .filter(|e| e.path().extension().map(|x| x == "json").unwrap_or(false))
            .filter_map(|e| {
                fs::read_to_string(e.path())
                    .ok()
                    .and_then(|s| serde_json::from_str::<ChatLog>(&s).ok())
            })
            .collect();
        logs.sort_by_key(|l| Reverse(l.created_at));
        logs
    }

    /// Delete a session by ID.
    pub fn delete(&self, id: &str) {
        let _ = fs::remove_file(self.log_path(id));
    }

    /// Remove oldest sessions until we are within [`MAX_CHAT_LOGS`].
    fn prune(&self) {
        let mut logs = self.list();
        // `list()` returns newest-first; keep first MAX_CHAT_LOGS, delete rest.
        if logs.len() > MAX_CHAT_LOGS {
            for old in logs.drain(MAX_CHAT_LOGS..) {
                self.delete(&old.id);
            }
        }
    }

    /// Return the most recent log, if any.
    pub fn most_recent(&self) -> Option<ChatLog> {
        self.list().into_iter().next()
    }
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn tmp_store() -> (TempDir, ChatLogStore) {
        let dir = TempDir::new().unwrap();
        let store = ChatLogStore::open(dir.path());
        (dir, store)
    }

    #[test]
    fn new_log_has_unique_id() {
        let a = ChatLog::new(None);
        let b = ChatLog::new(None);
        assert_ne!(a.id, b.id);
        assert!(a.id.starts_with("chat_"));
    }

    #[test]
    fn save_and_load_round_trip() {
        let (_dir, store) = tmp_store();
        let mut log = ChatLog::new(Some("pods/web-abc · default".to_owned()));
        log.messages.push(ChatLogMessage::new(Role::User, "Hello"));
        log.tokens_used = 42;
        let id = log.id.clone();
        store.save(&mut log);

        let loaded = store.load(&id).unwrap();
        assert_eq!(loaded.id, id);
        assert_eq!(loaded.tokens_used, 42);
        assert_eq!(loaded.messages.len(), 1);
        assert_eq!(
            loaded.context_label.as_deref(),
            Some("pods/web-abc · default")
        );
    }

    #[test]
    fn list_returns_newest_first() {
        let (_dir, store) = tmp_store();
        let mut a = ChatLog::new(None);
        let mut b = ChatLog::new(None);
        store.save(&mut a);
        // Ensure different nanosecond timestamps by tweaking updated_at.
        b.updated_at = Utc::now();
        store.save(&mut b);
        let list = store.list();
        assert_eq!(list.len(), 2);
        // Newest (b) should be first.
        assert_eq!(list[0].id, b.id);
    }

    #[test]
    fn delete_removes_file() {
        let (_dir, store) = tmp_store();
        let mut log = ChatLog::new(None);
        let id = log.id.clone();
        store.save(&mut log);
        assert!(store.load(&id).is_some());
        store.delete(&id);
        assert!(store.load(&id).is_none());
    }

    #[test]
    fn prune_enforces_max() {
        let (_dir, store) = tmp_store();
        for _ in 0..55 {
            let mut log = ChatLog::new(None);
            store.save(&mut log);
        }
        assert!(store.list().len() <= MAX_CHAT_LOGS);
    }

    #[test]
    fn message_count_excludes_system() {
        let mut log = ChatLog::new(None);
        log.messages.push(ChatLogMessage::new(Role::System, "sys"));
        log.messages.push(ChatLogMessage::new(Role::User, "q"));
        log.messages.push(ChatLogMessage::new(Role::Assistant, "a"));
        assert_eq!(log.message_count(), 2);
    }

    #[test]
    fn serde_round_trip() {
        let mut log = ChatLog::new(Some("test".to_owned()));
        log.messages.push(ChatLogMessage::new(Role::User, "hi"));
        let json = serde_json::to_string(&log).unwrap();
        let decoded: ChatLog = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded.id, log.id);
        assert_eq!(decoded.messages.len(), 1);
    }

    // ── to_markdown() tests ────────────────────────────────────────────────────

    #[test]
    fn markdown_has_header() {
        let log = ChatLog::new(None);
        let md = log.to_markdown();
        assert!(md.starts_with("# k7s AI Chat Export"));
    }

    #[test]
    fn markdown_includes_context_label() {
        let mut log = ChatLog::new(Some("pods/web-abc · default".to_owned()));
        log.tokens_used = 150;
        let md = log.to_markdown();
        assert!(md.contains("pods/web-abc · default"));
        assert!(md.contains("150"));
    }

    #[test]
    fn markdown_excludes_system_messages() {
        let mut log = ChatLog::new(None);
        log.messages
            .push(ChatLogMessage::new(Role::System, "Internal system prompt"));
        log.messages.push(ChatLogMessage::new(Role::User, "Hello?"));
        let md = log.to_markdown();
        assert!(!md.contains("Internal system prompt"));
        assert!(md.contains("Hello?"));
    }

    #[test]
    fn markdown_labels_user_and_assistant() {
        let mut log = ChatLog::new(None);
        log.messages
            .push(ChatLogMessage::new(Role::User, "What is wrong?"));
        log.messages.push(ChatLogMessage::new(
            Role::Assistant,
            "The pod is crashlooping.",
        ));
        let md = log.to_markdown();
        assert!(md.contains("**User**"));
        assert!(md.contains("**Assistant**"));
        assert!(md.contains("What is wrong?"));
        assert!(md.contains("The pod is crashlooping."));
    }

    #[test]
    fn markdown_empty_session_has_no_messages_notice() {
        let log = ChatLog::new(None);
        let md = log.to_markdown();
        assert!(md.contains("No messages"));
    }

    #[test]
    fn markdown_hides_tokens_when_zero() {
        let log = ChatLog::new(None);
        assert_eq!(log.tokens_used, 0);
        let md = log.to_markdown();
        assert!(!md.contains("Tokens used"));
    }
}
