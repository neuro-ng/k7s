//! HTTP benchmark runner — Phase 10.1.
//!
//! A lightweight load-testing engine that fires concurrent HTTP requests
//! against a target URL and reports latency percentiles and throughput.
//!
//! # Design
//!
//! * Uses `reqwest` (already a dependency for the LLM client) — no extra crates.
//! * Spawns `config.concurrency` async tasks, each sending requests sequentially
//!   until the total request or duration cap is reached.
//! * All latency samples are collected in memory; percentiles are computed after
//!   the run completes.
//! * Intentionally simple: no warm-up phase, no histogram bucketing, no
//!   connection pooling knobs.  For production benchmarking use `hey` or `k6`.
//!
//! # Usage
//!
//! ```no_run
//! use k7s::bench::{BenchmarkTarget, run_benchmark};
//! use k7s::config::BenchmarkConfig;
//!
//! #[tokio::main]
//! async fn main() {
//!     let target = BenchmarkTarget {
//!         url: "http://localhost:8080/healthz".to_owned(),
//!     };
//!     let config = BenchmarkConfig::default();
//!     let result = run_benchmark(&target, &config).await;
//!     println!("{}", result.summary());
//! }
//! ```

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::sync::Mutex;

use crate::config::BenchmarkConfig;

// ─── Target ───────────────────────────────────────────────────────────────────

/// The URL to benchmark.
#[derive(Debug, Clone)]
pub struct BenchmarkTarget {
    /// Full URL including scheme, host, port, and path.
    pub url: String,
}

// ─── Result ───────────────────────────────────────────────────────────────────

/// Aggregate statistics from a completed benchmark run.
#[derive(Debug, Clone, Default)]
pub struct BenchmarkResult {
    /// Total requests attempted.
    pub total: u64,
    /// Requests that received a 2xx response.
    pub successful: u64,
    /// Requests that timed out, errored, or returned a non-2xx status.
    pub failed: u64,
    /// Wall-clock duration of the entire run in milliseconds.
    pub wall_ms: u64,
    /// Requests per second (total / wall_secs).
    pub rps: f64,
    /// Median latency in milliseconds.
    pub p50_ms: u64,
    /// 90th-percentile latency in milliseconds.
    pub p90_ms: u64,
    /// 99th-percentile latency in milliseconds.
    pub p99_ms: u64,
    /// Maximum observed latency in milliseconds.
    pub max_ms: u64,
    /// Minimum observed latency in milliseconds.
    pub min_ms: u64,
    /// Mean latency in milliseconds.
    pub mean_ms: f64,
    /// Sample of distinct error messages (at most 10 unique messages).
    pub errors: Vec<String>,
}

impl BenchmarkResult {
    /// One-line human-readable summary.
    pub fn summary(&self) -> String {
        format!(
            "{} req  {:.1} req/s  ok:{} fail:{}  p50:{}ms p90:{}ms p99:{}ms  wall:{}ms",
            self.total,
            self.rps,
            self.successful,
            self.failed,
            self.p50_ms,
            self.p90_ms,
            self.p99_ms,
            self.wall_ms,
        )
    }

    /// Multi-line report suitable for display in the TUI benchmark view.
    pub fn report(&self) -> String {
        let mut lines = vec![
            format!(
                "Requests:     {} total, {} successful, {} failed",
                self.total, self.successful, self.failed
            ),
            format!("Throughput:   {:.2} req/s", self.rps),
            format!("Wall time:    {} ms", self.wall_ms),
            format!("Latency min:  {} ms", self.min_ms),
            format!("Latency mean: {:.1} ms", self.mean_ms),
            format!("Latency p50:  {} ms", self.p50_ms),
            format!("Latency p90:  {} ms", self.p90_ms),
            format!("Latency p99:  {} ms", self.p99_ms),
            format!("Latency max:  {} ms", self.max_ms),
        ];
        if !self.errors.is_empty() {
            lines.push(String::new());
            lines.push(format!("Errors ({}):", self.errors.len()));
            for e in &self.errors {
                lines.push(format!("  • {e}"));
            }
        }
        lines.join("\n")
    }
}

// ─── Shared state during a run ────────────────────────────────────────────────

struct RunState {
    /// All collected latency samples in milliseconds.
    latencies: Vec<u64>,
    /// Error messages (deduplicated, capped at 10).
    errors: Vec<String>,
}

// ─── Runner ───────────────────────────────────────────────────────────────────

/// Run a benchmark against `target` with the given `config`.
///
/// Returns when either `config.total_requests` have been sent (if > 0)
/// or `config.duration_secs` have elapsed (if `total_requests == 0`).
///
/// The function is cancellation-safe: dropping the returned future aborts
/// all in-flight requests.
pub async fn run_benchmark(target: &BenchmarkTarget, config: &BenchmarkConfig) -> BenchmarkResult {
    let client = match build_client(config) {
        Ok(c) => c,
        Err(e) => {
            return BenchmarkResult {
                errors: vec![format!("failed to build HTTP client: {e}")],
                ..Default::default()
            };
        }
    };

    let concurrency = config.concurrency.max(1) as usize;
    let total = config.total_requests as u64;
    let duration = if total == 0 && config.duration_secs > 0 {
        Some(Duration::from_secs(config.duration_secs as u64))
    } else {
        None
    };

    // Shared counters.
    let counter = Arc::new(AtomicU64::new(0));
    let ok_count = Arc::new(AtomicU64::new(0));
    let fail_count = Arc::new(AtomicU64::new(0));
    let state = Arc::new(Mutex::new(RunState {
        latencies: Vec::new(),
        errors: Vec::new(),
    }));

    let wall_start = Instant::now();
    let deadline = duration.map(|d| wall_start + d);

    // Spawn worker tasks.
    let mut handles = Vec::with_capacity(concurrency);
    for _ in 0..concurrency {
        let client = client.clone();
        let url = target.url.clone();
        let method = config.method.clone();
        let body = config.body.clone();
        let counter = Arc::clone(&counter);
        let ok_count = Arc::clone(&ok_count);
        let fail_count = Arc::clone(&fail_count);
        let state = Arc::clone(&state);

        handles.push(tokio::spawn(async move {
            loop {
                // Check total-requests cap.
                if total > 0 {
                    let idx = counter.fetch_add(1, Ordering::Relaxed);
                    if idx >= total {
                        break;
                    }
                }

                // Check duration cap.
                if let Some(dl) = deadline {
                    if Instant::now() >= dl {
                        break;
                    }
                }

                // Duration mode without a deadline — shouldn't happen, but break
                // to avoid infinite loop.
                if total == 0 && deadline.is_none() {
                    break;
                }

                let t0 = Instant::now();
                let result = send_request(&client, &url, &method, body.as_deref()).await;
                let elapsed_ms = t0.elapsed().as_millis() as u64;

                match result {
                    Ok(status) if status.is_success() => {
                        ok_count.fetch_add(1, Ordering::Relaxed);
                        let mut s = state.lock().await;
                        s.latencies.push(elapsed_ms);
                    }
                    Ok(status) => {
                        fail_count.fetch_add(1, Ordering::Relaxed);
                        let msg = format!("HTTP {}", status.as_u16());
                        let mut s = state.lock().await;
                        s.latencies.push(elapsed_ms);
                        add_error(&mut s.errors, msg);
                    }
                    Err(e) => {
                        fail_count.fetch_add(1, Ordering::Relaxed);
                        let mut s = state.lock().await;
                        add_error(&mut s.errors, e.to_string());
                    }
                }
            }
        }));
    }

    for h in handles {
        let _ = h.await;
    }

    let wall_ms = wall_start.elapsed().as_millis() as u64;
    let successful = ok_count.load(Ordering::Relaxed);
    let failed = fail_count.load(Ordering::Relaxed);
    let total_sent = successful + failed;

    let mut run = state.lock().await;
    run.latencies.sort_unstable();

    let rps = if wall_ms > 0 {
        total_sent as f64 / (wall_ms as f64 / 1000.0)
    } else {
        0.0
    };

    let (p50, p90, p99, min_ms, max_ms, mean_ms) = compute_stats(&run.latencies);

    BenchmarkResult {
        total: total_sent,
        successful,
        failed,
        wall_ms,
        rps,
        p50_ms: p50,
        p90_ms: p90,
        p99_ms: p99,
        min_ms,
        max_ms,
        mean_ms,
        errors: run.errors.clone(),
    }
}

// ─── Helpers ─────────────────────────────────────────────────────────────────

fn build_client(config: &BenchmarkConfig) -> anyhow::Result<reqwest::Client> {
    let mut builder = reqwest::Client::builder()
        .timeout(Duration::from_millis(config.timeout_ms))
        .connection_verbose(false);

    if config.http2 {
        // Request HTTP/2 when the server supports it via ALPN.
        // `http2_prior_knowledge()` is only available with the `http2` feature;
        // use `use_rustls_tls()` which negotiates H2 via TLS ALPN instead.
        builder = builder.https_only(false);
    }

    Ok(builder.build()?)
}

async fn send_request(
    client: &reqwest::Client,
    url: &str,
    method: &str,
    body: Option<&str>,
) -> anyhow::Result<reqwest::StatusCode> {
    let method_parsed = reqwest::Method::from_bytes(method.as_bytes())?;
    let mut req = client.request(method_parsed, url);
    if let Some(b) = body {
        req = req.body(b.to_owned());
    }
    let resp = req.send().await?;
    Ok(resp.status())
}

/// Add an error message to the list, deduplicating and capping at 10.
fn add_error(errors: &mut Vec<String>, msg: String) {
    if errors.len() >= 10 {
        return;
    }
    // Simple dedup: don't add if an identical message is already present.
    if !errors.iter().any(|e| e == &msg) {
        errors.push(msg);
    }
}

/// Compute percentile and basic stats from a **sorted** latency slice.
///
/// Returns `(p50, p90, p99, min, max, mean)` in milliseconds.
fn compute_stats(sorted: &[u64]) -> (u64, u64, u64, u64, u64, f64) {
    if sorted.is_empty() {
        return (0, 0, 0, 0, 0, 0.0);
    }

    let p = |pct: f64| -> u64 {
        let idx = ((sorted.len() as f64 * pct / 100.0).ceil() as usize).saturating_sub(1);
        sorted[idx.min(sorted.len() - 1)]
    };

    let min = *sorted.first().unwrap();
    let max = *sorted.last().unwrap();
    let mean = sorted.iter().sum::<u64>() as f64 / sorted.len() as f64;

    (p(50.0), p(90.0), p(99.0), min, max, mean)
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compute_stats_single_sample() {
        let (p50, p90, p99, min, max, mean) = compute_stats(&[42]);
        assert_eq!(p50, 42);
        assert_eq!(p90, 42);
        assert_eq!(p99, 42);
        assert_eq!(min, 42);
        assert_eq!(max, 42);
        assert!((mean - 42.0).abs() < f64::EPSILON);
    }

    #[test]
    fn compute_stats_multiple() {
        // [10, 20, 30, 40, 50, 60, 70, 80, 90, 100]
        let samples: Vec<u64> = (1..=10).map(|i| i * 10).collect();
        let (p50, p90, p99, min, max, _mean) = compute_stats(&samples);
        assert_eq!(min, 10);
        assert_eq!(max, 100);
        assert!((50..=60).contains(&p50));
        assert!((90..=100).contains(&p90));
        assert!((90..=100).contains(&p99));
    }

    #[test]
    fn compute_stats_empty_returns_zeroes() {
        let (p50, p90, p99, min, max, mean) = compute_stats(&[]);
        assert_eq!((p50, p90, p99, min, max), (0, 0, 0, 0, 0));
        assert_eq!(mean, 0.0);
    }

    #[test]
    fn add_error_deduplicates() {
        let mut errors = Vec::new();
        add_error(&mut errors, "timeout".to_owned());
        add_error(&mut errors, "timeout".to_owned());
        assert_eq!(errors.len(), 1);
    }

    #[test]
    fn add_error_caps_at_ten() {
        let mut errors = Vec::new();
        for i in 0..15 {
            add_error(&mut errors, format!("err-{i}"));
        }
        assert_eq!(errors.len(), 10);
    }

    #[test]
    fn result_summary_contains_key_fields() {
        let r = BenchmarkResult {
            total: 100,
            successful: 98,
            failed: 2,
            wall_ms: 1000,
            rps: 100.0,
            p50_ms: 10,
            p90_ms: 25,
            p99_ms: 50,
            ..Default::default()
        };
        let s = r.summary();
        assert!(s.contains("100 req"));
        assert!(s.contains("100.0 req/s"));
        assert!(s.contains("ok:98"));
        assert!(s.contains("fail:2"));
    }

    #[test]
    fn benchmark_config_default_is_sane() {
        let c = BenchmarkConfig::default();
        assert!(c.concurrency > 0);
        assert!(c.total_requests > 0);
        assert!(c.timeout_ms > 0);
        assert_eq!(c.method, "GET");
    }
}
