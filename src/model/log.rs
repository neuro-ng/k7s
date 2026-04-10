//! Log model — buffered, filterable log lines from one or more containers.
//!
//! # Design
//!
//! `LogItem` is a single parsed log line. `LogModel` owns a circular buffer of
//! `LogItem`s, applies an optional regex filter, and tracks which container
//! the lines came from (for multi-container merging).
//!
//! `LogModel` is pure data — the TUI widget (`LogView`) reads from it.  All
//! mutations go through `LogModel`; the widget never writes to it.

use std::collections::VecDeque;

use regex::Regex;

// ─── LogLevel ─────────────────────────────────────────────────────────────────

/// Detected log severity level.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogLevel {
    Error,
    Warn,
    Info,
    Debug,
    Trace,
    Unknown,
}

impl LogLevel {
    /// Detect a level from the raw line content.
    pub fn detect(line: &str) -> Self {
        let l = line.to_ascii_lowercase();
        if l.contains("error")
            || l.contains("err ")
            || l.contains(" err:")
            || l.contains("fatal")
            || l.contains("panic")
        {
            return Self::Error;
        }
        if l.contains("warn") {
            return Self::Warn;
        }
        if l.contains("debug") {
            return Self::Debug;
        }
        if l.contains("trace") {
            return Self::Trace;
        }
        if l.contains("info") {
            return Self::Info;
        }
        Self::Unknown
    }
}

// ─── LogItem ──────────────────────────────────────────────────────────────────

/// A single parsed log line.
#[derive(Debug, Clone)]
pub struct LogItem {
    /// The original raw line text (may include timestamp prefix if enabled).
    pub raw: String,
    /// Container name — populated when streaming multiple containers.
    pub container: Option<String>,
    /// Detected log level.
    pub level: LogLevel,
    /// Optional RFC 3339 timestamp prefix stripped from the raw line.
    pub timestamp: Option<String>,
    /// The message portion after stripping the timestamp.
    pub message: String,
}

impl LogItem {
    /// Parse a raw log line into a `LogItem`.
    ///
    /// If the line starts with an RFC 3339 timestamp (as emitted by the
    /// Kubernetes log API with `timestamps=true`), it is separated out.
    pub fn parse(raw: impl Into<String>, container: Option<String>) -> Self {
        let raw: String = raw.into();
        let (timestamp, message) = {
            let (ts, msg) = split_timestamp(&raw);
            (ts.map(str::to_owned), msg.to_owned())
        };
        let level = LogLevel::detect(&message);

        Self {
            raw,
            container,
            level,
            timestamp,
            message,
        }
    }

    /// Display text shown in the log view (container prefix + message).
    pub fn display(&self, show_timestamps: bool) -> String {
        let mut parts = Vec::new();
        if let Some(c) = &self.container {
            parts.push(format!("[{}]", c));
        }
        if show_timestamps {
            if let Some(ts) = &self.timestamp {
                parts.push(ts.clone());
            }
        }
        parts.push(self.message.clone());
        parts.join(" ")
    }
}

/// Try to split a leading RFC 3339 / k8s timestamp from a log line.
///
/// Kubernetes emits timestamps as `2024-01-15T12:34:56.789012345Z <message>`.
fn split_timestamp(line: &str) -> (Option<&str>, &str) {
    // A timestamp must start with a 4-digit year and contain 'T' and 'Z'.
    if line.len() < 20 {
        return (None, line);
    }
    let prefix = &line[..line.find(' ').unwrap_or(line.len())];
    if prefix.len() >= 20
        && prefix.chars().next().is_some_and(|c| c.is_ascii_digit())
        && prefix.contains('T')
        && (prefix.ends_with('Z') || prefix.contains('+'))
    {
        let rest = line[prefix.len()..].trim_start();
        return (Some(prefix), rest);
    }
    (None, line)
}

// ─── LogModel ─────────────────────────────────────────────────────────────────

/// Maximum log lines held in memory per model.
const DEFAULT_BUFFER_CAP: usize = 5_000;

/// Buffered, filterable log model.
///
/// Thread-safety: `LogModel` is `!Send` — it lives on the tokio task that
/// drives the TUI render loop. Lines are pushed from an async spawned task via
/// a `tokio::sync::mpsc` channel; the render task drains the channel into the
/// model each tick.
pub struct LogModel {
    /// Ring buffer — oldest lines are evicted when capacity is reached.
    lines: VecDeque<LogItem>,
    /// Maximum number of lines to keep.
    capacity: usize,
    /// Optional compiled regex filter.
    filter: Option<Regex>,
    /// Whether to show timestamps in the display.
    pub show_timestamps: bool,
    /// Whether the stream is actively receiving lines.
    pub streaming: bool,
    /// Containers currently being watched.
    pub containers: Vec<String>,
}

impl LogModel {
    pub fn new() -> Self {
        Self::with_capacity(DEFAULT_BUFFER_CAP)
    }

    pub fn with_capacity(cap: usize) -> Self {
        Self {
            lines: VecDeque::with_capacity(cap.min(DEFAULT_BUFFER_CAP)),
            capacity: cap,
            filter: None,
            show_timestamps: false,
            streaming: false,
            containers: Vec::new(),
        }
    }

    /// Push a new raw log line into the buffer.
    ///
    /// `container` is `None` for single-container pods or when merging is not needed.
    pub fn push(&mut self, raw: impl Into<String>, container: Option<String>) {
        if self.lines.len() == self.capacity {
            self.lines.pop_front();
        }
        self.lines.push_back(LogItem::parse(raw, container));
    }

    /// Total number of buffered lines (unfiltered).
    pub fn len(&self) -> usize {
        self.lines.len()
    }
    pub fn is_empty(&self) -> bool {
        self.lines.is_empty()
    }

    /// Set a regex filter. Pass `None` to clear.
    ///
    /// Returns `Err` if the pattern is invalid.
    pub fn set_filter(&mut self, pattern: Option<&str>) -> Result<(), regex::Error> {
        self.filter = match pattern {
            Some(p) if !p.is_empty() => Some(Regex::new(p)?),
            _ => None,
        };
        Ok(())
    }

    /// Current filter pattern string, if any.
    pub fn filter_pattern(&self) -> Option<&str> {
        self.filter.as_ref().map(|r| r.as_str())
    }

    /// Iterate over lines matching the current filter (or all lines if no filter).
    pub fn filtered_lines(&self) -> impl Iterator<Item = &LogItem> {
        self.lines.iter().filter(|item| match &self.filter {
            None => true,
            Some(re) => re.is_match(&item.raw),
        })
    }

    /// Filtered lines as a collected `Vec` (for indexed access in the view).
    pub fn visible_lines(&self) -> Vec<&LogItem> {
        self.filtered_lines().collect()
    }

    /// Whether a filter is currently active.
    pub fn is_filtered(&self) -> bool {
        self.filter.is_some()
    }

    /// Highlight regions within a line that match the filter regex.
    ///
    /// Returns a vec of `(start, end)` byte offsets within `text`.
    pub fn highlight_ranges(&self, text: &str) -> Vec<(usize, usize)> {
        match &self.filter {
            None => Vec::new(),
            Some(re) => re.find_iter(text).map(|m| (m.start(), m.end())).collect(),
        }
    }

    /// Clear all buffered lines (e.g. when switching containers).
    pub fn clear(&mut self) {
        self.lines.clear();
    }
}

impl Default for LogModel {
    fn default() -> Self {
        Self::new()
    }
}

// ─── Multi-container merging ───────────────────────────────────────────────────

/// Merge log lines from multiple containers into a single sorted stream.
///
/// Lines from each container are tagged with the container name.
/// When timestamps are available they are used for ordering; otherwise lines
/// are interleaved in arrival order.
///
/// In practice, `LogModel::push` with a `container` name is sufficient for
/// the streaming use-case. This function is provided for the case where all
/// lines are available at once (e.g. fetching historical logs for all containers).
pub fn merge_container_logs(container_lines: Vec<(String, Vec<String>)>) -> Vec<LogItem> {
    let mut all: Vec<LogItem> = container_lines
        .into_iter()
        .flat_map(|(container, lines)| {
            lines
                .into_iter()
                .map(move |raw| LogItem::parse(raw, Some(container.clone())))
        })
        .collect();

    // Sort by timestamp prefix when present; preserve insertion order otherwise.
    all.sort_by(|a, b| match (&a.timestamp, &b.timestamp) {
        (Some(ta), Some(tb)) => ta.cmp(tb),
        _ => std::cmp::Ordering::Equal,
    });

    all
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn push_and_len() {
        let mut m = LogModel::new();
        m.push("hello", None);
        m.push("world", None);
        assert_eq!(m.len(), 2);
    }

    #[test]
    fn capacity_evicts_oldest() {
        let mut m = LogModel::with_capacity(3);
        m.push("a", None);
        m.push("b", None);
        m.push("c", None);
        m.push("d", None); // evicts "a"
        assert_eq!(m.len(), 3);
        let msgs: Vec<_> = m
            .visible_lines()
            .iter()
            .map(|l| l.message.as_str())
            .collect();
        assert!(!msgs.contains(&"a"));
        assert!(msgs.contains(&"d"));
    }

    #[test]
    fn filter_narrows_visible_lines() {
        let mut m = LogModel::new();
        m.push("error: something went wrong", None);
        m.push("info: all good", None);
        m.push("error: another failure", None);
        m.set_filter(Some("error")).unwrap();
        assert_eq!(m.visible_lines().len(), 2);
    }

    #[test]
    fn clear_filter_shows_all_lines() {
        let mut m = LogModel::new();
        m.push("error: x", None);
        m.push("info: y", None);
        m.set_filter(Some("error")).unwrap();
        m.set_filter(None).unwrap();
        assert_eq!(m.visible_lines().len(), 2);
    }

    #[test]
    fn invalid_regex_returns_error() {
        let mut m = LogModel::new();
        assert!(m.set_filter(Some("[invalid")).is_err());
    }

    #[test]
    fn highlight_ranges_no_filter() {
        let m = LogModel::new();
        assert!(m.highlight_ranges("any text").is_empty());
    }

    #[test]
    fn highlight_ranges_with_filter() {
        let mut m = LogModel::new();
        m.set_filter(Some("err")).unwrap();
        let ranges = m.highlight_ranges("error here");
        assert!(!ranges.is_empty());
        assert_eq!(&"error here"[ranges[0].0..ranges[0].1], "err");
    }

    #[test]
    fn log_level_detection() {
        assert_eq!(LogLevel::detect("ERROR: disk full"), LogLevel::Error);
        assert_eq!(LogLevel::detect("WARN: low memory"), LogLevel::Warn);
        assert_eq!(LogLevel::detect("INFO: started"), LogLevel::Info);
        assert_eq!(LogLevel::detect("DEBUG: verbose output"), LogLevel::Debug);
        assert_eq!(LogLevel::detect("some plain text"), LogLevel::Unknown);
    }

    #[test]
    fn timestamp_splitting() {
        let (ts, msg) = split_timestamp("2024-01-15T12:34:56.000000000Z the message");
        assert!(ts.is_some());
        assert_eq!(msg, "the message");
    }

    #[test]
    fn no_timestamp_passthrough() {
        let (ts, msg) = split_timestamp("plain log line");
        assert!(ts.is_none());
        assert_eq!(msg, "plain log line");
    }

    #[test]
    fn log_item_display_with_container() {
        let item = LogItem::parse("hello", Some("app".to_owned()));
        assert!(item.display(false).contains("[app]"));
    }

    #[test]
    fn merge_container_logs_with_timestamps() {
        let lines = vec![
            (
                "web".to_owned(),
                vec!["2024-01-01T00:00:02Z msg-b".to_owned()],
            ),
            (
                "db".to_owned(),
                vec!["2024-01-01T00:00:01Z msg-a".to_owned()],
            ),
        ];
        let merged = merge_container_logs(lines);
        assert_eq!(merged.len(), 2);
        // "db" line has earlier timestamp so comes first.
        assert_eq!(merged[0].container.as_deref(), Some("db"));
        assert_eq!(merged[1].container.as_deref(), Some("web"));
    }
}
