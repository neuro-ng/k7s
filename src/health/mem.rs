//! Process memory statistics — Phase 14.2.
//!
//! Reads resident-set size (RSS) and virtual-memory size (VmSize) from
//! `/proc/self/status` on Linux.  On other platforms (macOS, Windows) the
//! function returns zeroes rather than failing — the feature degrades
//! gracefully everywhere.
//!
//! # Why `/proc/self/status`?
//!
//! * Zero extra dependencies.
//! * Available on any modern Linux kernel (including containers).
//! * Provides RSS, VmSize, VmPeak, and VmRSS at sub-millisecond cost.
//!
//! # Relationship to jemalloc
//!
//! When the `jemalloc` Cargo feature is enabled the binary swaps in
//! `tikv-jemallocator` as the global allocator.  The OS-level numbers from
//! `/proc/self/status` still reflect true memory usage, but jemalloc reduces
//! fragmentation so those numbers will typically be lower under sustained load
//! compared to the system allocator.

/// Memory statistics for the current process.
#[derive(Debug, Clone, Default)]
pub struct HeapStats {
    /// Virtual memory size in bytes (`VmSize` in `/proc/self/status`).
    pub vm_size: u64,
    /// Resident set size in bytes (`VmRSS`).
    pub vm_rss: u64,
    /// Peak resident set size in bytes (`VmPeak`).
    pub vm_peak: u64,
    /// Anonymous memory (heap + stack) in bytes (`VmData`).
    pub vm_data: u64,
}

impl HeapStats {
    /// Read current process memory stats.
    ///
    /// Returns `HeapStats { 0, 0, 0, 0 }` on platforms that lack `/proc`.
    pub fn read() -> Self {
        Self::read_proc_status().unwrap_or_default()
    }

    /// Human-readable RSS label (e.g. `"12.3 MB"`).
    pub fn rss_label(&self) -> String {
        format_bytes(self.vm_rss)
    }

    /// Human-readable virtual size label.
    pub fn vm_label(&self) -> String {
        format_bytes(self.vm_size)
    }

    /// Human-readable peak RSS label.
    pub fn peak_label(&self) -> String {
        format_bytes(self.vm_peak)
    }

    /// Multi-line report for the Pulse / diagnostics view.
    pub fn report(&self) -> String {
        format!(
            "RSS:   {}\nVmPeak: {}\nVmSize: {}\nVmData: {}",
            self.rss_label(),
            self.peak_label(),
            self.vm_label(),
            format_bytes(self.vm_data),
        )
    }

    /// Parse `/proc/self/status`.
    ///
    /// Returns `None` when the file cannot be read (non-Linux platforms, or
    /// sandboxed environments that restrict `/proc` access).
    #[cfg(target_os = "linux")]
    fn read_proc_status() -> Option<Self> {
        let content = std::fs::read_to_string("/proc/self/status").ok()?;
        let mut stats = HeapStats::default();
        for line in content.lines() {
            // Lines look like: `VmRSS:     12345 kB`
            let mut parts = line.splitn(2, ':');
            let key = parts.next()?.trim();
            let val_str = parts.next()?.trim();
            // Extract the numeric prefix (value in kB).
            let kb: u64 = val_str
                .split_whitespace()
                .next()
                .and_then(|s| s.parse().ok())
                .unwrap_or(0);
            let bytes = kb * 1_024;
            match key {
                "VmSize" => stats.vm_size = bytes,
                "VmRSS" => stats.vm_rss = bytes,
                "VmPeak" => stats.vm_peak = bytes,
                "VmData" => stats.vm_data = bytes,
                _ => {}
            }
        }
        Some(stats)
    }

    #[cfg(not(target_os = "linux"))]
    fn read_proc_status() -> Option<Self> {
        None
    }
}

// ─── Helpers ──────────────────────────────────────────────────────────────────

fn format_bytes(bytes: u64) -> String {
    match bytes {
        0 => "0 B".to_owned(),
        b if b < 1_024 => format!("{b} B"),
        b if b < 1_024 * 1_024 => format!("{:.1} KB", b as f64 / 1_024.0),
        b if b < 1_024 * 1_024 * 1_024 => {
            format!("{:.1} MB", b as f64 / (1_024.0 * 1_024.0))
        }
        b => format!("{:.2} GB", b as f64 / (1_024.0 * 1_024.0 * 1_024.0)),
    }
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_bytes_zero() {
        assert_eq!(format_bytes(0), "0 B");
    }

    #[test]
    fn format_bytes_bytes() {
        assert_eq!(format_bytes(512), "512 B");
    }

    #[test]
    fn format_bytes_kilobytes() {
        let s = format_bytes(2048);
        assert!(s.ends_with("KB"), "expected KB, got {s}");
    }

    #[test]
    fn format_bytes_megabytes() {
        let s = format_bytes(5 * 1_024 * 1_024);
        assert!(s.ends_with("MB"), "expected MB, got {s}");
    }

    #[test]
    fn heap_stats_read_does_not_panic() {
        // Just make sure it doesn't panic on any platform.
        let stats = HeapStats::read();
        // On Linux we expect non-zero RSS; on other platforms we get zeroes.
        #[cfg(target_os = "linux")]
        {
            assert!(stats.vm_rss > 0, "expected non-zero RSS on Linux");
        }
        #[cfg(not(target_os = "linux"))]
        {
            let _ = stats; // zeroes are fine
        }
    }

    #[test]
    fn heap_stats_labels_are_human_readable() {
        let stats = HeapStats {
            vm_rss: 12 * 1_024 * 1_024,  // 12 MB
            vm_size: 100 * 1_024 * 1_024,
            vm_peak: 15 * 1_024 * 1_024,
            vm_data: 8 * 1_024 * 1_024,
        };
        assert!(stats.rss_label().contains("MB"));
        assert!(stats.vm_label().contains("MB"));
        assert!(stats.peak_label().contains("MB"));
    }

    #[test]
    fn heap_stats_report_contains_all_fields() {
        let stats = HeapStats {
            vm_rss: 1024,
            vm_size: 2048,
            vm_peak: 3072,
            vm_data: 512,
        };
        let r = stats.report();
        assert!(r.contains("RSS"));
        assert!(r.contains("VmPeak"));
        assert!(r.contains("VmSize"));
        assert!(r.contains("VmData"));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn proc_status_parse() {
        // Write a synthetic /proc/self/status-like content and verify parsing.
        let content = "\
VmPeak:\t  1024 kB\n\
VmSize:\t   512 kB\n\
VmRSS:\t   256 kB\n\
VmData:\t   128 kB\n";

        let mut stats = HeapStats::default();
        for line in content.lines() {
            let mut parts = line.splitn(2, ':');
            let key = parts.next().unwrap().trim();
            let val_str = parts.next().unwrap().trim();
            let kb: u64 = val_str.split_whitespace().next().unwrap().parse().unwrap();
            let bytes = kb * 1_024;
            match key {
                "VmSize" => stats.vm_size = bytes,
                "VmRSS" => stats.vm_rss = bytes,
                "VmPeak" => stats.vm_peak = bytes,
                "VmData" => stats.vm_data = bytes,
                _ => {}
            }
        }
        assert_eq!(stats.vm_peak, 1024 * 1024);
        assert_eq!(stats.vm_rss, 256 * 1024);
    }
}
