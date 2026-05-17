//! Cluster metadata store — Phase 37.
//!
//! Append-only JSON-lines daily files stored under
//! `~/.local/state/k7s/cluster-metadata/<context>/`.

use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};

use chrono::{NaiveDate, Utc};
use serde::{Deserialize, Serialize};

use super::record::MetadataRecord;
use crate::config::MetaConfig;

// ─── MetadataStore ────────────────────────────────────────────────────────────

/// Per-cluster journal stored in the XDG state directory.
pub struct MetadataStore {
    base: PathBuf,
}

impl MetadataStore {
    /// Create or open the store for `context`.
    ///
    /// Returns `None` if the XDG state directory cannot be resolved or created.
    pub fn new(context: &str) -> Option<Self> {
        let base = crate::config::ConfigDirs::resolve()
            .ok()?
            .state
            .join("cluster-metadata")
            .join(sanitize_context_name(context));

        fs::create_dir_all(&base).ok()?;
        Some(Self { base })
    }

    /// Append one record to today's daily file.
    pub fn append(&self, record: &MetadataRecord) -> std::io::Result<()> {
        let date = record.timestamp().date_naive();
        let path = self.day_path(date);

        let mut f = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)?;

        let line = serde_json::to_string(record)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        writeln!(f, "{line}")?;

        let _ = self.update_index();
        Ok(())
    }

    /// Read all records for a single day (corrupt lines silently skipped).
    pub fn load_day(&self, date: NaiveDate) -> Vec<MetadataRecord> {
        self.read_records_from(&self.day_path(date))
    }

    /// Load the last `days` days of records, sorted by timestamp ascending.
    pub fn load_recent(&self, days: u8) -> Vec<MetadataRecord> {
        let today = Utc::now().date_naive();
        let mut records = Vec::new();

        for i in 0..days as i64 {
            let date = today - chrono::Duration::days(i);
            records.extend(self.load_day(date));
        }

        records.sort_by_key(|r| *r.timestamp());
        records
    }

    /// Sorted list of dates for which daily files exist.
    pub fn list_dates(&self) -> Vec<NaiveDate> {
        let mut dates = Vec::new();
        if let Ok(entries) = fs::read_dir(&self.base) {
            for entry in entries.flatten() {
                let name = entry.file_name();
                let name = name.to_string_lossy();
                if name.ends_with(".json") && name != "index.json" {
                    let stem = name.trim_end_matches(".json");
                    if let Ok(date) =
                        NaiveDate::parse_from_str(stem, "%Y-%m-%d")
                    {
                        dates.push(date);
                    }
                }
            }
        }
        dates.sort();
        dates
    }

    /// Delete daily files strictly older than `before`.  Returns count deleted.
    pub fn prune(&self, before: NaiveDate) -> std::io::Result<usize> {
        let mut count = 0;
        for date in self.list_dates() {
            if date < before {
                let _ = fs::remove_file(self.day_path(date));
                count += 1;
            }
        }
        if count > 0 {
            let _ = self.update_index();
        }
        Ok(count)
    }

    /// Sum of all file sizes in the context directory (bytes).
    pub fn total_size_bytes(&self) -> u64 {
        let mut total = 0u64;
        if let Ok(entries) = fs::read_dir(&self.base) {
            for entry in entries.flatten() {
                if let Ok(meta) = entry.metadata() {
                    total += meta.len();
                }
            }
        }
        total
    }

    /// Prune by retention days, then by max size if still over limit.
    ///
    /// Called once on startup.
    pub fn auto_prune(&self, cfg: &MetaConfig) {
        let today = Utc::now().date_naive();
        let cutoff = today - chrono::Duration::days(cfg.retention_days as i64);
        let _ = self.prune(cutoff);

        // Prune oldest-first if still over size limit.
        let mut dates = self.list_dates();
        dates.reverse(); // oldest first after reverse
        for date in dates {
            if self.total_size_bytes() <= cfg.max_size_bytes {
                break;
            }
            let _ = fs::remove_file(self.day_path(date));
        }
        let _ = self.update_index();
    }

    /// Path to the base directory (used for display / testing).
    pub fn base_dir(&self) -> &Path {
        &self.base
    }

    // ── Private helpers ───────────────────────────────────────────────────────

    fn day_path(&self, date: NaiveDate) -> PathBuf {
        self.base.join(format!("{}.json", date.format("%Y-%m-%d")))
    }

    fn read_records_from(&self, path: &Path) -> Vec<MetadataRecord> {
        let Ok(f) = fs::File::open(path) else {
            return Vec::new();
        };
        let reader = BufReader::new(f);
        let mut records = Vec::new();
        for line in reader.lines() {
            let Ok(line) = line else { continue };
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            if let Ok(record) = serde_json::from_str::<MetadataRecord>(line) {
                records.push(record);
            }
            // Corrupt lines silently skipped.
        }
        records
    }

    fn update_index(&self) -> std::io::Result<()> {
        let dates = self.list_dates();
        let index = IndexFile {
            date_from: dates.first().copied(),
            date_to: dates.last().copied(),
            day_count: dates.len(),
            total_bytes: self.total_size_bytes(),
        };
        let path = self.base.join("index.json");
        let content = serde_json::to_string_pretty(&index)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        fs::write(path, content)
    }
}

// ─── IndexFile ────────────────────────────────────────────────────────────────

/// `index.json` — fast manifest; lets `:meta list` skip full file scans.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IndexFile {
    pub date_from: Option<NaiveDate>,
    pub date_to: Option<NaiveDate>,
    pub day_count: usize,
    pub total_bytes: u64,
}

// ─── Helpers ──────────────────────────────────────────────────────────────────

/// Map a kubeconfig context name to a safe directory name.
///
/// Preserves alphanumerics, `-`, `_`, `.`; replaces everything else with `_`.
pub(crate) fn sanitize_context_name(name: &str) -> String {
    name.chars()
        .map(|c| {
            if c.is_alphanumeric() || matches!(c, '-' | '_' | '.') {
                c
            } else {
                '_'
            }
        })
        .collect()
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::meta::record::{IssueRecord, NodeSummary, SnapshotRecord, WorkloadSummary};
    use tempfile::TempDir;

    /// Build a store backed by a temp directory (avoids touching XDG dirs).
    fn temp_store() -> (MetadataStore, TempDir) {
        let dir = TempDir::new().unwrap();
        let store = MetadataStore { base: dir.path().to_path_buf() };
        (store, dir)
    }

    fn make_snapshot() -> MetadataRecord {
        MetadataRecord::Snapshot(SnapshotRecord::new(
            NodeSummary { total: 3, ready: 3, not_ready: 0 },
            vec!["default".into()],
            WorkloadSummary { deployments: 2, running: 2, degraded: 0 },
            "1.30.2",
        ))
    }

    fn make_issue() -> MetadataRecord {
        MetadataRecord::Issue(IssueRecord::new(
            "CrashLoopBackOff", "prod", "api", "Pod", "exited 137",
        ))
    }

    #[test]
    fn append_and_load_round_trip() {
        let (store, _dir) = temp_store();
        let today = Utc::now().date_naive();

        store.append(&make_snapshot()).unwrap();
        store.append(&make_issue()).unwrap();

        let loaded = store.load_day(today);
        assert_eq!(loaded.len(), 2);
        assert_eq!(loaded[0].type_label(), "snapshot");
        assert_eq!(loaded[1].type_label(), "issue");
    }

    #[test]
    fn corrupt_line_skipped() {
        let (store, _dir) = temp_store();
        let today = Utc::now().date_naive();
        let path = store.day_path(today);

        // Write one good record and one corrupt line.
        let good = serde_json::to_string(&make_snapshot()).unwrap();
        fs::write(&path, format!("{good}\n{{corrupt json\n")).unwrap();

        let loaded = store.load_day(today);
        assert_eq!(loaded.len(), 1, "corrupt line should be skipped");
    }

    #[test]
    fn load_missing_day_returns_empty() {
        let (store, _dir) = temp_store();
        let far_past = NaiveDate::from_ymd_opt(2000, 1, 1).unwrap();
        assert!(store.load_day(far_past).is_empty());
    }

    #[test]
    fn list_dates_sorted() {
        let (store, _dir) = temp_store();

        // Create files out of order.
        let d1 = NaiveDate::from_ymd_opt(2026, 5, 10).unwrap();
        let d2 = NaiveDate::from_ymd_opt(2026, 5, 12).unwrap();
        let d3 = NaiveDate::from_ymd_opt(2026, 5, 11).unwrap();

        fs::write(store.day_path(d2), b"").unwrap();
        fs::write(store.day_path(d1), b"").unwrap();
        fs::write(store.day_path(d3), b"").unwrap();

        let dates = store.list_dates();
        assert_eq!(dates, vec![d1, d3, d2]);
    }

    #[test]
    fn prune_removes_old_files() {
        let (store, _dir) = temp_store();

        let old = NaiveDate::from_ymd_opt(2026, 1, 1).unwrap();
        let recent = Utc::now().date_naive();
        fs::write(store.day_path(old), b"").unwrap();
        fs::write(store.day_path(recent), b"").unwrap();

        // Cutoff between old (2026-01-01) and recent (today) — only old deleted.
        let cutoff = NaiveDate::from_ymd_opt(2026, 2, 1).unwrap();
        let deleted = store.prune(cutoff).unwrap();
        assert_eq!(deleted, 1);
        assert!(!store.day_path(old).exists());
        assert!(store.day_path(recent).exists());
    }

    #[test]
    fn total_size_bytes_sums_files() {
        let (store, _dir) = temp_store();
        let d = Utc::now().date_naive();
        fs::write(store.day_path(d), b"hello").unwrap();
        assert!(store.total_size_bytes() >= 5);
    }

    #[test]
    fn update_index_writes_json() {
        let (store, _dir) = temp_store();
        store.append(&make_snapshot()).unwrap();
        let index_path = store.base.join("index.json");
        assert!(index_path.exists());
        let content = fs::read_to_string(index_path).unwrap();
        let idx: IndexFile = serde_json::from_str(&content).unwrap();
        assert_eq!(idx.day_count, 1);
    }

    #[test]
    fn sanitize_context_name_replaces_slashes() {
        assert_eq!(sanitize_context_name("prod/us-east"), "prod_us-east");
        assert_eq!(sanitize_context_name("ctx@host"), "ctx_host");
        assert_eq!(sanitize_context_name("my.cluster"), "my.cluster");
    }

    #[test]
    fn load_recent_returns_sorted_records() {
        let (store, _dir) = temp_store();
        store.append(&make_issue()).unwrap();
        store.append(&make_snapshot()).unwrap();

        let records = store.load_recent(1);
        // Both in today's file; load_recent sorts by timestamp.
        assert_eq!(records.len(), 2);
        // First record timestamp <= second record timestamp.
        assert!(records[0].timestamp() <= records[1].timestamp());
    }
}
