//! Log compression engine for token-efficient LLM context.
//!
//! Raw container logs are extremely repetitive. Sending them verbatim to
//! an LLM wastes tokens and hits context limits quickly. This module
//! compresses logs to a small, high-signal summary:
//!
//! ```text
//! [2024-01-15T10:00:00Z] ERROR: connection refused (×847 in 5m)
//! [2024-01-15T10:05:00Z] INFO: reconnecting to database (×12 in 5m)
//! Log level distribution: ERR:67%, WARN:8%, INFO:25%
//! Total: 10,234 lines → 42 unique patterns (95.8% compression)
//! ```

use std::collections::HashMap;

/// A deduplicated log entry with occurrence count.
#[derive(Debug, Clone)]
pub struct CompressedLine {
    /// The representative line (first or most recent occurrence).
    pub line: String,
    /// Total count of identical or similar lines.
    pub count: u64,
    /// Detected log level.
    pub level: LogLevel,
}

/// Coarse log level parsed from a log line prefix.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LogLevel {
    Error,
    Warn,
    Info,
    Debug,
    Unknown,
}

impl LogLevel {
    fn as_str(self) -> &'static str {
        match self {
            Self::Error => "ERR",
            Self::Warn => "WARN",
            Self::Info => "INFO",
            Self::Debug => "DEBUG",
            Self::Unknown => "?",
        }
    }
}

/// Statistics about the compression run.
#[derive(Debug, Clone)]
pub struct CompressionStats {
    pub total_lines: u64,
    pub unique_patterns: usize,
    pub level_counts: HashMap<LogLevel, u64>,
}

impl CompressionStats {
    /// Compression ratio as a percentage (0–100).
    pub fn compression_pct(&self) -> f64 {
        if self.total_lines == 0 {
            return 0.0;
        }
        (1.0 - self.unique_patterns as f64 / self.total_lines as f64) * 100.0
    }

    /// Human-readable level distribution string, e.g. `"ERR:12%, WARN:23%, INFO:65%"`.
    pub fn level_distribution(&self) -> String {
        if self.total_lines == 0 {
            return "N/A".to_owned();
        }
        let mut parts: Vec<String> = [
            LogLevel::Error,
            LogLevel::Warn,
            LogLevel::Info,
            LogLevel::Debug,
        ]
        .iter()
        .filter_map(|&lvl| {
            let count = self.level_counts.get(&lvl).copied().unwrap_or(0);
            if count == 0 {
                return None;
            }
            let pct = (count as f64 / self.total_lines as f64 * 100.0) as u64;
            Some(format!("{}:{}%", lvl.as_str(), pct))
        })
        .collect();
        if parts.is_empty() {
            parts.push("?:100%".to_owned());
        }
        parts.join(", ")
    }
}

/// Result of a compression run.
#[derive(Debug, Clone)]
pub struct CompressedLog {
    pub lines: Vec<CompressedLine>,
    pub stats: CompressionStats,
}

impl CompressedLog {
    /// Render the compressed log as a compact string suitable for an LLM prompt.
    ///
    /// Uses the `"[message] (×N times)"` format for repeated lines.
    pub fn to_prompt_string(&self) -> String {
        let mut out = String::new();

        for entry in &self.lines {
            if entry.count == 1 {
                out.push_str(&entry.line);
            } else {
                out.push_str(&format!("{} (×{} times)", entry.line, entry.count));
            }
            out.push('\n');
        }

        // Append summary stats.
        out.push_str(&format!(
            "\n--- Log summary: {} lines → {} patterns ({:.1}% compression) | {}\n",
            self.stats.total_lines,
            self.stats.unique_patterns,
            self.stats.compression_pct(),
            self.stats.level_distribution(),
        ));

        out
    }
}

/// Compress a stream of log lines for LLM consumption.
///
/// Strategy:
/// 1. Normalize lines by stripping variable tokens (timestamps, hex IDs,
///    numbers in certain positions) to form a canonical "pattern".
/// 2. Deduplicate by pattern, counting occurrences.
/// 3. Keep the first occurrence as the representative line.
/// 4. Limit output to `max_unique` distinct patterns.
pub fn compress(lines: &[String], max_unique: usize) -> CompressedLog {
    let mut pattern_map: indexmap::IndexMap<String, CompressedLine> = indexmap::IndexMap::new();
    let mut level_counts: HashMap<LogLevel, u64> = HashMap::new();
    let mut total = 0u64;

    for line in lines {
        if line.trim().is_empty() {
            continue;
        }
        total += 1;

        let level = detect_level(line);
        *level_counts.entry(level).or_insert(0) += 1;

        let pattern = normalize(line);

        if let Some(entry) = pattern_map.get_mut(&pattern) {
            entry.count += 1;
        } else if pattern_map.len() < max_unique {
            pattern_map.insert(
                pattern,
                CompressedLine {
                    line: line.clone(),
                    count: 1,
                    level,
                },
            );
        }
        // Lines beyond max_unique are counted in total but not stored as patterns.
        // This is captured in CompressionStats.
    }

    let unique = pattern_map.len();
    let lines_out: Vec<CompressedLine> = pattern_map.into_values().collect();

    CompressedLog {
        lines: lines_out,
        stats: CompressionStats {
            total_lines: total,
            unique_patterns: unique,
            level_counts,
        },
    }
}

/// Detect the log level from a line prefix.
///
/// Recognises common formats: `[ERROR]`, `ERROR:`, `WARN`, `level=error`, etc.
fn detect_level(line: &str) -> LogLevel {
    let lower = line.to_lowercase();

    // level=error, level=warn, etc. (structured log style)
    if let Some(pos) = lower.find("level=") {
        let after = &lower[pos + 6..];
        return level_from_word(after);
    }

    // "ERROR:", "[ERROR]", " ERROR " etc.
    for word in lower.split_whitespace().take(4) {
        let word = word.trim_matches(|c: char| !c.is_alphanumeric());
        match word {
            "error" | "err" | "fatal" | "critical" | "crit" => return LogLevel::Error,
            "warning" | "warn" | "wrn" => return LogLevel::Warn,
            "info" | "inf" | "information" => return LogLevel::Info,
            "debug" | "dbg" | "trace" => return LogLevel::Debug,
            _ => {}
        }
    }

    LogLevel::Unknown
}

fn level_from_word(s: &str) -> LogLevel {
    let word = s
        .split_whitespace()
        .next()
        .unwrap_or("")
        .trim_matches(|c: char| !c.is_alphanumeric());
    match word {
        "error" | "err" | "fatal" => LogLevel::Error,
        "warning" | "warn" => LogLevel::Warn,
        "info" => LogLevel::Info,
        "debug" | "trace" => LogLevel::Debug,
        _ => LogLevel::Unknown,
    }
}

/// Normalize a log line by replacing variable tokens with placeholders.
///
/// This makes lines that differ only in timestamps, request IDs, etc.
/// hash to the same pattern.
fn normalize(line: &str) -> String {
    let mut result = String::with_capacity(line.len());
    let mut chars = line.chars().peekable();

    while let Some(c) = chars.next() {
        // Check for `0x…` hex prefix before the general digit case.
        if c == '0' && chars.peek() == Some(&'x') {
            chars.next(); // consume 'x'
            while chars.peek().map(|c| c.is_ascii_hexdigit()).unwrap_or(false) {
                chars.next();
            }
            result.push_str("<hex>");
            continue;
        }

        match c {
            // Replace runs of digits (timestamps, ports, counts, etc.).
            '0'..='9' => {
                let mut digits = String::new();
                digits.push(c);
                while chars.peek().map(|c| c.is_ascii_digit()).unwrap_or(false) {
                    digits.push(chars.next().unwrap());
                }
                // Heuristic: ≥4 consecutive digits look like a timestamp component.
                if digits.len() >= 4 {
                    result.push_str("<N>");
                } else {
                    result.push_str(&digits);
                }
            }
            // Keep everything else verbatim.
            _ => result.push(c),
        }
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compress_deduplicates_identical_lines() {
        let lines: Vec<String> = vec!["ERROR: connection refused".to_owned(); 100];
        let result = compress(&lines, 50);
        assert_eq!(result.stats.total_lines, 100);
        assert_eq!(result.lines.len(), 1);
        assert_eq!(result.lines[0].count, 100);
    }

    #[test]
    fn compress_respects_max_unique() {
        let lines: Vec<String> = (0..200).map(|i| format!("line {i}")).collect();
        let result = compress(&lines, 50);
        assert!(result.lines.len() <= 50, "should cap at max_unique");
        assert_eq!(result.stats.total_lines, 200);
    }

    #[test]
    fn compression_pct_100_identical() {
        let lines: Vec<String> = vec!["same line".to_owned(); 1000];
        let result = compress(&lines, 100);
        assert!(result.stats.compression_pct() > 99.0);
    }

    #[test]
    fn level_detection_error() {
        assert_eq!(detect_level("ERROR: something went wrong"), LogLevel::Error);
        assert_eq!(detect_level("[ERROR] failed to connect"), LogLevel::Error);
        assert_eq!(detect_level("level=error msg=timeout"), LogLevel::Error);
    }

    #[test]
    fn level_detection_warn() {
        assert_eq!(detect_level("WARN: disk space low"), LogLevel::Warn);
        assert_eq!(detect_level("WARNING retrying"), LogLevel::Warn);
    }

    #[test]
    fn level_detection_info() {
        assert_eq!(detect_level("INFO: server started"), LogLevel::Info);
    }

    #[test]
    fn to_prompt_string_includes_summary() {
        let lines = vec!["ERROR: crash".to_owned(); 50];
        let result = compress(&lines, 10);
        let s = result.to_prompt_string();
        assert!(s.contains("50 lines"), "should mention total: {s}");
        assert!(s.contains("×50 times"), "should show count: {s}");
        assert!(s.contains("ERR:"), "should show level dist: {s}");
    }

    #[test]
    fn level_distribution_percentages() {
        let lines = vec![
            "ERROR: bad".to_owned(),
            "ERROR: also bad".to_owned(),
            "INFO: ok".to_owned(),
            "INFO: also ok".to_owned(),
            "INFO: fine".to_owned(),
            "INFO: fine2".to_owned(),
        ];
        let result = compress(&lines, 20);
        let dist = result.stats.level_distribution();
        assert!(dist.contains("ERR:"), "dist: {dist}");
        assert!(dist.contains("INFO:"), "dist: {dist}");
    }
}
