//! Advanced log analysis — Phase 13.3, 13.5, 13.6.
//!
//! Three complementary capabilities that sit on top of the base log compressor:
//!
//! * [`summarise_stack_trace`]  — extract exception type + key frames (13.3)
//! * [`detect_temporal_patterns`] — detect error spikes / recurring intervals (13.5)
//! * [`SmartTruncator`]         — `"..."` truncation with on-demand expansion (13.6)
//!
//! All functions are pure (no I/O), fast (single pass), and safe to call
//! with arbitrarily large log buffers.

// ─── 13.3 Stack trace summarization ─────────────────────────────────────────

/// A summarised representation of a stack trace.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StackTraceSummary {
    /// The exception / error type and message (first line of the trace).
    pub exception: String,
    /// Key frames — the first few non-stdlib, non-runtime frames.
    pub frames: Vec<String>,
    /// Total number of frames seen (for context).
    pub total_frames: usize,
    /// Which language/runtime the trace looks like.
    pub runtime: TraceRuntime,
}

impl StackTraceSummary {
    /// Render as a compact string for LLM consumption.
    pub fn to_prompt_string(&self) -> String {
        let mut out = format!("[{}] {}\n", self.runtime.as_str(), self.exception);
        for frame in &self.frames {
            out.push_str(&format!("  at {frame}\n"));
        }
        if self.total_frames > self.frames.len() {
            out.push_str(&format!(
                "  ... ({} more frames)\n",
                self.total_frames - self.frames.len()
            ));
        }
        out
    }
}

/// Which runtime produced the stack trace.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TraceRuntime {
    Java,
    Python,
    Node,
    Go,
    Rust,
    Unknown,
}

impl TraceRuntime {
    pub fn as_str(self) -> &'static str {
        match self {
            TraceRuntime::Java    => "java",
            TraceRuntime::Python  => "python",
            TraceRuntime::Node    => "node",
            TraceRuntime::Go      => "go",
            TraceRuntime::Rust    => "rust",
            TraceRuntime::Unknown => "unknown",
        }
    }
}

/// Summarise a multi-line stack trace, keeping only the exception header and
/// the top `max_frames` user-space frames.
///
/// Accepts lines from a single trace block (already extracted from the full
/// log).  Returns `None` if the input does not look like a stack trace.
pub fn summarise_stack_trace(lines: &[&str], max_frames: usize) -> Option<StackTraceSummary> {
    if lines.is_empty() {
        return None;
    }

    let runtime = detect_runtime(lines);
    let exception = lines[0].trim().to_owned();

    // Is this actually a stack trace? Need at least one frame-looking line.
    let frame_lines: Vec<&str> = lines[1..]
        .iter()
        .filter(|l| is_frame_line(l, runtime))
        .copied()
        .collect();

    if frame_lines.is_empty() {
        return None;
    }

    let total_frames = frame_lines.len();
    let frames: Vec<String> = frame_lines
        .iter()
        .filter(|l| !is_stdlib_frame(l, runtime))
        .take(max_frames)
        .map(|l| clean_frame(l, runtime))
        .collect();

    Some(StackTraceSummary {
        exception,
        frames,
        total_frames,
        runtime,
    })
}

fn detect_runtime(lines: &[&str]) -> TraceRuntime {
    for line in lines.iter().take(5) {
        let l = line.trim();
        if l.starts_with("at ") && l.contains('(') && l.ends_with(')') {
            // "at com.example.Foo.bar(Foo.java:42)" — Java
            if l.contains(".java:") { return TraceRuntime::Java; }
            // "at Object.<anonymous> (file.js:10:5)" — Node
            if l.contains(".js:") { return TraceRuntime::Node; }
        }
        if l.starts_with("File \"") && l.contains(", line ") {
            return TraceRuntime::Python;
        }
        if l.starts_with("goroutine ") || (l.starts_with('\t') && l.contains(".go:")) {
            return TraceRuntime::Go;
        }
        if l.contains("::") && l.ends_with('>') {
            // "  0: std::panicking::begin_panic<...>" — Rust
            return TraceRuntime::Rust;
        }
    }
    TraceRuntime::Unknown
}

fn is_frame_line(line: &str, rt: TraceRuntime) -> bool {
    let l = line.trim();
    match rt {
        TraceRuntime::Java   => l.starts_with("at ") && l.contains('('),
        TraceRuntime::Python => l.starts_with("File \""),
        TraceRuntime::Node   => l.starts_with("at ") && l.contains('('),
        TraceRuntime::Go     => l.starts_with('\t') && l.contains(".go:"),
        TraceRuntime::Rust   => l.starts_with("  ") && (l.contains("::") || l.contains(".rs:")),
        TraceRuntime::Unknown => l.starts_with("at ") || l.starts_with('\t'),
    }
}

fn is_stdlib_frame(line: &str, rt: TraceRuntime) -> bool {
    let l = line.to_lowercase();
    match rt {
        TraceRuntime::Java => {
            l.contains("java.lang.") || l.contains("sun.reflect.") || l.contains("java.util.")
        }
        TraceRuntime::Python => {
            l.contains("/lib/python") || l.contains("site-packages/")
        }
        TraceRuntime::Node => {
            l.contains("node:internal") || l.contains("(node:") || l.contains("(timers.js")
        }
        TraceRuntime::Go => {
            l.contains("runtime/") || l.contains("testing/")
        }
        TraceRuntime::Rust => {
            l.contains("std::") || l.contains("core::") || l.contains("tokio::")
        }
        TraceRuntime::Unknown => false,
    }
}

fn clean_frame(line: &str, _rt: TraceRuntime) -> String {
    line.trim().to_owned()
}

// ─── 13.5 Temporal pattern detection ─────────────────────────────────────────

/// A detected temporal pattern in log lines.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TemporalPattern {
    /// Human-readable description, e.g. `"Errors spike every hour at :00"`.
    pub description: String,
    /// How many times the pattern was observed.
    pub occurrences: usize,
}

/// Detect recurring temporal patterns (spikes, intervals) in timestamped logs.
///
/// `lines` should be raw log lines; each line may start with an RFC 3339 or
/// common log format timestamp. Lines without a parseable timestamp are ignored.
///
/// Returns up to `max_patterns` detected patterns.
pub fn detect_temporal_patterns(lines: &[&str], max_patterns: usize) -> Vec<TemporalPattern> {
    if lines.is_empty() || max_patterns == 0 {
        return vec![];
    }

    // Collect (minute_of_hour, error_flag) tuples.
    let mut minute_errors: Vec<u8> = Vec::new();
    let mut hour_errors:   Vec<u8> = Vec::new();

    for line in lines {
        if let Some((minute, hour, is_error)) = parse_time_and_level(line) {
            if is_error {
                minute_errors.push(minute);
                hour_errors.push(hour);
            }
        }
    }

    if minute_errors.is_empty() {
        return vec![];
    }

    let mut patterns: Vec<TemporalPattern> = Vec::new();

    // Pattern 1 — errors cluster near the top of each minute (:00...:05).
    let near_top: usize = minute_errors.iter().filter(|&&m| m <= 5).count();
    let top_pct = near_top as f64 / minute_errors.len() as f64;
    if top_pct >= 0.6 && minute_errors.len() >= 3 {
        patterns.push(TemporalPattern {
            description: format!(
                "Errors spike at the top of each minute ({}% at :00–:05)",
                (top_pct * 100.0) as u8
            ),
            occurrences: near_top,
        });
    }

    // Pattern 2 — errors cluster near the top of each hour (:00 minute).
    let near_hour_top: usize = minute_errors.iter().filter(|&&m| m == 0).count();
    let hour_top_pct = near_hour_top as f64 / minute_errors.len() as f64;
    if hour_top_pct >= 0.4 && near_hour_top >= 2 {
        patterns.push(TemporalPattern {
            description: format!(
                "Errors spike at :00 of each hour ({}% of errors)",
                (hour_top_pct * 100.0) as u8
            ),
            occurrences: near_hour_top,
        });
    }

    // Pattern 3 — errors spread across multiple distinct hours (sustained).
    {
        let mut distinct_hours = hour_errors.clone();
        distinct_hours.sort_unstable();
        distinct_hours.dedup();
        if distinct_hours.len() >= 3 {
            patterns.push(TemporalPattern {
                description: format!(
                    "Errors are sustained across {} distinct hours",
                    distinct_hours.len()
                ),
                occurrences: minute_errors.len(),
            });
        }
    }

    patterns.truncate(max_patterns);
    patterns
}

/// Parse timestamp and log level from a log line.
///
/// Returns `(minute_of_hour, hour_of_day, is_error)` or `None`.
fn parse_time_and_level(line: &str) -> Option<(u8, u8, bool)> {
    // Try to find an ISO-8601 / RFC-3339 timestamp prefix like:
    //   2024-01-15T10:32:00Z
    //   2024-01-15 10:32:00
    let line = line.trim();

    // Need at least "YYYY-MM-DDThh:mm"
    if line.len() < 16 {
        return None;
    }

    let time_part = if line.as_bytes().get(10) == Some(&b'T') || line.as_bytes().get(10) == Some(&b' ') {
        &line[11..]
    } else {
        return None;
    };

    // time_part starts with "HH:MM"
    if time_part.len() < 5 {
        return None;
    }
    let hh = time_part[..2].parse::<u8>().ok()?;
    let colon = time_part.as_bytes().get(2)?;
    if *colon != b':' {
        return None;
    }
    let mm = time_part[3..5].parse::<u8>().ok()?;

    let lower = line.to_lowercase();
    let is_error = lower.contains("error") || lower.contains("err ") || lower.contains(" err:")
        || lower.contains("fatal") || lower.contains("panic");

    Some((mm, hh, is_error))
}

// ─── 13.6 Smart truncation ───────────────────────────────────────────────────

/// A truncated piece of text with a stable hash for later re-expansion.
#[derive(Debug, Clone)]
pub struct TruncatedText {
    /// The visible portion shown in the chat window or prompt.
    pub visible: String,
    /// Whether the text was actually truncated.
    pub was_truncated: bool,
    /// Stable hash of the *full* content — use with [`SmartTruncator::expand`].
    pub content_hash: u64,
}

/// Smart truncation engine with on-demand expansion.
///
/// Store the `SmartTruncator` alongside the chat session so prior truncations
/// can be expanded when the user asks a follow-up question.
///
/// # Usage
/// ```ignore
/// let truncator = SmartTruncator::new(500);
/// let t = truncator.truncate(&long_text);
/// // In the LLM prompt: include t.visible (short)
/// // On follow-up: if user asks "show me the full output":
/// if let Some(full) = truncator.expand(t.content_hash) { ... }
/// ```
pub struct SmartTruncator {
    /// Maximum number of characters in the visible portion.
    max_chars: usize,
    /// Stored full texts keyed by their hash. Bounded to avoid unbounded growth.
    store: std::collections::HashMap<u64, String>,
    /// Maximum number of stored expansions.
    max_stored: usize,
}

impl SmartTruncator {
    /// Create a truncator with the given character limit per truncation.
    pub fn new(max_chars: usize) -> Self {
        Self {
            max_chars,
            store: std::collections::HashMap::new(),
            max_stored: 128,
        }
    }

    /// Truncate `text` to `max_chars`, storing the full content for expansion.
    ///
    /// If `text` fits within the limit it is returned verbatim with
    /// `was_truncated = false`.
    pub fn truncate(&mut self, text: &str) -> TruncatedText {
        let hash = hash_str(text);

        if text.len() <= self.max_chars {
            return TruncatedText {
                visible: text.to_owned(),
                was_truncated: false,
                content_hash: hash,
            };
        }

        // Find the last whitespace boundary within the limit to avoid cutting words.
        let cutoff = self.max_chars.saturating_sub(3); // leave room for "..."
        let boundary = text[..cutoff]
            .rfind(char::is_whitespace)
            .unwrap_or(cutoff);

        let visible = format!("{}...", &text[..boundary]);

        // Store full text for later expansion (evict oldest if full).
        if self.store.len() >= self.max_stored {
            let oldest = self.store.keys().next().copied();
            if let Some(k) = oldest {
                self.store.remove(&k);
            }
        }
        self.store.insert(hash, text.to_owned());

        TruncatedText { visible, was_truncated: true, content_hash: hash }
    }

    /// Retrieve the full text for a previously truncated entry.
    ///
    /// Returns `None` if the hash is unknown (e.g. was evicted from the store).
    pub fn expand(&self, content_hash: u64) -> Option<&str> {
        self.store.get(&content_hash).map(|s| s.as_str())
    }

    /// Number of full texts currently stored.
    pub fn stored_count(&self) -> usize {
        self.store.len()
    }
}

/// FNV-1a 64-bit hash — fast, no dependencies.
fn hash_str(s: &str) -> u64 {
    const FNV_PRIME: u64 = 0x00000100_000001B3;
    const FNV_OFFSET: u64 = 0xcbf29ce4_84222325;
    s.bytes().fold(FNV_OFFSET, |acc, b| {
        (acc ^ b as u64).wrapping_mul(FNV_PRIME)
    })
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── Stack trace summarization ────────────────────────────────────────────

    const JAVA_TRACE: &[&str] = &[
        "java.lang.NullPointerException: Cannot invoke method foo()",
        "    at com.example.MyClass.doSomething(MyClass.java:42)",
        "    at com.example.MyClass.main(MyClass.java:10)",
        "    at java.lang.reflect.Method.invoke(Method.java:498)",
        "    at sun.reflect.NativeMethodAccessorImpl.invoke(NativeMethodAccessorImpl.java:62)",
    ];

    #[test]
    fn java_trace_detected() {
        let s = summarise_stack_trace(JAVA_TRACE, 3).unwrap();
        assert_eq!(s.runtime, TraceRuntime::Java);
        assert!(s.exception.contains("NullPointerException"));
    }

    #[test]
    fn java_trace_filters_stdlib_frames() {
        let s = summarise_stack_trace(JAVA_TRACE, 10).unwrap();
        // java.lang and sun.reflect frames should be filtered.
        assert!(s.frames.iter().all(|f| !f.contains("java.lang.reflect") && !f.contains("sun.reflect")));
        assert!(s.frames.iter().any(|f| f.contains("com.example")));
    }

    #[test]
    fn java_trace_respects_max_frames() {
        let s = summarise_stack_trace(JAVA_TRACE, 1).unwrap();
        assert_eq!(s.frames.len(), 1);
        assert!(s.total_frames >= 1);
    }

    #[test]
    fn no_frames_returns_none() {
        let lines = &["Just a plain error message with no frames"];
        assert!(summarise_stack_trace(lines, 5).is_none());
    }

    const PYTHON_TRACE: &[&str] = &[
        "Traceback (most recent call last):",
        "  File \"/app/main.py\", line 23, in <module>",
        "  File \"/app/service.py\", line 45, in handle",
        "  File \"/usr/lib/python3.9/json/__init__.py\", line 346, in loads",
        "ValueError: Expecting value: line 1 column 1 (char 0)",
    ];

    #[test]
    fn python_trace_detected() {
        let s = summarise_stack_trace(PYTHON_TRACE, 5).unwrap();
        assert_eq!(s.runtime, TraceRuntime::Python);
    }

    #[test]
    fn to_prompt_string_contains_exception() {
        let s = summarise_stack_trace(JAVA_TRACE, 2).unwrap();
        let out = s.to_prompt_string();
        assert!(out.contains("NullPointerException"));
        assert!(out.contains("[java]"));
    }

    // ── Temporal pattern detection ────────────────────────────────────────────

    fn error_lines_at_minute(minute: u8, count: usize) -> Vec<String> {
        (0..count)
            .map(|i| format!("2024-01-15T10:{minute:02}:{i:02}Z ERROR: something failed"))
            .collect()
    }

    #[test]
    fn detects_minute_spike() {
        let mut lines: Vec<String> = Vec::new();
        // Spike at :00 — 8 errors
        lines.extend(error_lines_at_minute(0, 8));
        // Scattered elsewhere — 2 errors
        lines.extend(error_lines_at_minute(30, 1));
        lines.extend(error_lines_at_minute(45, 1));

        let refs: Vec<&str> = lines.iter().map(|s| s.as_str()).collect();
        let patterns = detect_temporal_patterns(&refs, 3);
        assert!(!patterns.is_empty(), "should detect a pattern");
        let desc = patterns[0].description.to_lowercase();
        assert!(desc.contains(":00") || desc.contains("minute") || desc.contains("spike"), "desc: {desc}");
    }

    #[test]
    fn no_timestamps_returns_empty() {
        let lines = &["no timestamp here error", "also no timestamp warn"];
        let patterns = detect_temporal_patterns(lines, 5);
        assert!(patterns.is_empty());
    }

    #[test]
    fn max_patterns_respected() {
        let mut lines: Vec<String> = Vec::new();
        lines.extend(error_lines_at_minute(0, 10));
        lines.extend((1u8..=6).flat_map(|h| {
            (0..3).map(move |_| format!("2024-01-15T{h:02}:00:00Z ERROR: spike"))
        }));
        let refs: Vec<&str> = lines.iter().map(|s| s.as_str()).collect();
        let patterns = detect_temporal_patterns(&refs, 1);
        assert!(patterns.len() <= 1);
    }

    // ── Smart truncation ─────────────────────────────────────────────────────

    #[test]
    fn short_text_not_truncated() {
        let mut t = SmartTruncator::new(100);
        let result = t.truncate("hello world");
        assert!(!result.was_truncated);
        assert_eq!(result.visible, "hello world");
    }

    #[test]
    fn long_text_is_truncated_with_ellipsis() {
        let mut t = SmartTruncator::new(20);
        let long = "word ".repeat(20);
        let result = t.truncate(&long);
        assert!(result.was_truncated);
        assert!(result.visible.ends_with("..."));
        assert!(result.visible.len() <= 23); // max_chars + "..."
    }

    #[test]
    fn expand_returns_full_text() {
        let full = "word ".repeat(20);
        let mut t = SmartTruncator::new(20);
        let result = t.truncate(&full);
        assert!(result.was_truncated);
        let expanded = t.expand(result.content_hash).unwrap();
        assert_eq!(expanded, full.as_str());
    }

    #[test]
    fn unknown_hash_returns_none() {
        let t = SmartTruncator::new(100);
        assert!(t.expand(0xdeadbeef).is_none());
    }

    #[test]
    fn same_text_same_hash() {
        let text = "consistent text";
        assert_eq!(hash_str(text), hash_str(text));
    }

    #[test]
    fn different_text_different_hash() {
        assert_ne!(hash_str("foo"), hash_str("bar"));
    }
}
