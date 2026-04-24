//! Live resource metrics — Phase 18.
//!
//! Fetches CPU and memory usage from the `metrics.k8s.io/v1beta1` API and
//! stores a rolling history so the TUI can render sparklines.
//!
//! # Architecture
//!
//! ```text
//! background task
//!   MetricsPoller::run()  ──mpsc──►  App  ──►  MetricsStore
//!                                               └─ sparkline data
//! ```
//!
//! The poller fires on `poll_interval` (default 30 s), fetches PodMetrics and
//! NodeMetrics from the cluster, and sends them as a `MetricsSnapshot` via an
//! mpsc channel.  The App thread receives the snapshot and feeds it into the
//! `MetricsStore` which keeps the last `HISTORY_LEN` samples per resource.
//!
//! # k9s Reference
//! `internal/client/metrics.go`, `internal/model/pulse.go`

use std::collections::{HashMap, VecDeque};
use std::time::Duration;

use kube::api::{ApiResource, DynamicObject, ListParams};
use kube::Api;
use serde_json::Value;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

/// Number of historical samples retained per resource.
pub const HISTORY_LEN: usize = 60;

/// Default polling interval.
pub const DEFAULT_POLL_INTERVAL: Duration = Duration::from_secs(30);

// ─── Data types ───────────────────────────────────────────────────────────────

/// A single CPU + memory reading for one resource.
#[derive(Debug, Clone, Default)]
pub struct MetricSample {
    /// CPU usage in millicores.
    pub cpu_m: u64,
    /// Memory usage in kibibytes.
    pub mem_ki: u64,
}

/// Per-resource metrics history — a ring buffer of samples.
#[derive(Debug, Clone, Default)]
pub struct ResourceHistory {
    pub samples: VecDeque<MetricSample>,
}

impl ResourceHistory {
    fn push(&mut self, sample: MetricSample) {
        if self.samples.len() >= HISTORY_LEN {
            self.samples.pop_front();
        }
        self.samples.push_back(sample);
    }

    /// Latest sample (most recent).
    pub fn latest(&self) -> Option<&MetricSample> {
        self.samples.back()
    }

    /// CPU history as a `Vec<u64>` for ratatui's `Sparkline` (oldest → newest).
    pub fn cpu_sparkline(&self) -> Vec<u64> {
        self.samples.iter().map(|s| s.cpu_m).collect()
    }

    /// Memory history (Ki) as a `Vec<u64>` for ratatui's `Sparkline`.
    pub fn mem_sparkline(&self) -> Vec<u64> {
        self.samples.iter().map(|s| s.mem_ki).collect()
    }
}

/// A batch of PodMetrics and NodeMetrics fetched in one poll cycle.
#[derive(Debug, Clone, Default)]
pub struct MetricsSnapshot {
    /// Map from `"namespace/pod-name"` to aggregated usage across all containers.
    pub pods: HashMap<String, MetricSample>,
    /// Map from `"node-name"` to usage.
    pub nodes: HashMap<String, MetricSample>,
}

// ─── MetricsStore ─────────────────────────────────────────────────────────────

/// In-memory time-series store for pod and node metrics.
///
/// Updated by the App each time a `MetricsSnapshot` arrives from the background
/// poller.  The stored histories are consumed by [`crate::view::metrics::MetricsView`].
#[derive(Debug, Default)]
pub struct MetricsStore {
    pub pods: HashMap<String, ResourceHistory>,
    pub nodes: HashMap<String, ResourceHistory>,
}

impl MetricsStore {
    pub fn new() -> Self {
        Self::default()
    }

    /// Ingest a new snapshot — append one sample to each resource's history.
    pub fn ingest(&mut self, snapshot: &MetricsSnapshot) {
        for (key, sample) in &snapshot.pods {
            self.pods
                .entry(key.clone())
                .or_default()
                .push(sample.clone());
        }
        for (key, sample) in &snapshot.nodes {
            self.nodes
                .entry(key.clone())
                .or_default()
                .push(sample.clone());
        }
    }

    /// Top N pods sorted by latest CPU usage (descending).
    pub fn top_pods_by_cpu(&self, n: usize) -> Vec<(&str, &ResourceHistory)> {
        let mut entries: Vec<_> = self.pods.iter().map(|(k, h)| (k.as_str(), h)).collect();
        entries.sort_by(|a, b| {
            let a_cpu = a.1.latest().map(|s| s.cpu_m).unwrap_or(0);
            let b_cpu = b.1.latest().map(|s| s.cpu_m).unwrap_or(0);
            b_cpu.cmp(&a_cpu)
        });
        entries.truncate(n);
        entries
    }

    /// Top N nodes sorted by latest CPU usage (descending).
    pub fn top_nodes_by_cpu(&self, n: usize) -> Vec<(&str, &ResourceHistory)> {
        let mut entries: Vec<_> = self.nodes.iter().map(|(k, h)| (k.as_str(), h)).collect();
        entries.sort_by(|a, b| {
            let a_cpu = a.1.latest().map(|s| s.cpu_m).unwrap_or(0);
            let b_cpu = b.1.latest().map(|s| s.cpu_m).unwrap_or(0);
            b_cpu.cmp(&a_cpu)
        });
        entries.truncate(n);
        entries
    }
}

// ─── MetricsClient ────────────────────────────────────────────────────────────

/// Fetches pod and node metrics from the `metrics.k8s.io/v1beta1` API.
pub struct MetricsClient {
    client: kube::Client,
}

impl MetricsClient {
    pub fn new(client: kube::Client) -> Self {
        Self { client }
    }

    /// Fetch a full metrics snapshot.  Returns an empty snapshot if the
    /// metrics-server is unavailable or the API call fails.
    pub async fn fetch(&self) -> MetricsSnapshot {
        let mut snapshot = MetricsSnapshot::default();

        match self.fetch_pod_metrics().await {
            Ok(pods) => snapshot.pods = pods,
            Err(e) => tracing::debug!(error = %e, "pod metrics fetch failed"),
        }

        match self.fetch_node_metrics().await {
            Ok(nodes) => snapshot.nodes = nodes,
            Err(e) => tracing::debug!(error = %e, "node metrics fetch failed"),
        }

        snapshot
    }

    async fn fetch_pod_metrics(&self) -> Result<HashMap<String, MetricSample>, anyhow::Error> {
        let ar = ApiResource {
            group: "metrics.k8s.io".into(),
            version: "v1beta1".into(),
            api_version: "metrics.k8s.io/v1beta1".into(),
            kind: "PodMetrics".into(),
            plural: "pods".into(),
        };
        let api: Api<DynamicObject> = Api::all_with(self.client.clone(), &ar);
        let list = api.list(&ListParams::default()).await?;

        let mut result = HashMap::new();
        for item in list.items {
            let ns = item
                .metadata
                .namespace
                .clone()
                .unwrap_or_else(|| "default".into());
            let name = item.metadata.name.clone().unwrap_or_default();
            let key = format!("{ns}/{name}");
            let sample = aggregate_containers(&item.data);
            result.insert(key, sample);
        }
        Ok(result)
    }

    async fn fetch_node_metrics(&self) -> Result<HashMap<String, MetricSample>, anyhow::Error> {
        let ar = ApiResource {
            group: "metrics.k8s.io".into(),
            version: "v1beta1".into(),
            api_version: "metrics.k8s.io/v1beta1".into(),
            kind: "NodeMetrics".into(),
            plural: "nodes".into(),
        };
        let api: Api<DynamicObject> = Api::all_with(self.client.clone(), &ar);
        let list = api.list(&ListParams::default()).await?;

        let mut result = HashMap::new();
        for item in list.items {
            let name = item.metadata.name.clone().unwrap_or_default();
            let sample = parse_usage(&item.data["usage"]);
            result.insert(name, sample);
        }
        Ok(result)
    }
}

// ─── Background poller ───────────────────────────────────────────────────────

/// Spawns a background tokio task that polls metrics on `interval` and sends
/// `MetricsSnapshot` values to `tx`.  The task exits when `cancel` is triggered.
pub fn spawn_metrics_poller(
    client: kube::Client,
    tx: mpsc::Sender<MetricsSnapshot>,
    interval: Duration,
    cancel: CancellationToken,
) {
    tokio::spawn(async move {
        let metrics_client = MetricsClient::new(client);
        let mut ticker = tokio::time::interval(interval);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

        loop {
            tokio::select! {
                _ = cancel.cancelled() => {
                    tracing::debug!("metrics poller stopped");
                    break;
                }
                _ = ticker.tick() => {
                    let snapshot = metrics_client.fetch().await;
                    tracing::debug!(
                        pods = snapshot.pods.len(),
                        nodes = snapshot.nodes.len(),
                        "metrics snapshot fetched"
                    );
                    if tx.send(snapshot).await.is_err() {
                        break; // receiver dropped
                    }
                }
            }
        }
    });
}

// ─── CPU / memory parsing helpers ────────────────────────────────────────────

/// Aggregate container usage fields from a PodMetrics `DynamicObject.data`.
fn aggregate_containers(data: &Value) -> MetricSample {
    let mut total_cpu_m: u64 = 0;
    let mut total_mem_ki: u64 = 0;

    if let Some(containers) = data.get("containers").and_then(|v| v.as_array()) {
        for c in containers {
            let usage = &c["usage"];
            let s = parse_usage(usage);
            total_cpu_m += s.cpu_m;
            total_mem_ki += s.mem_ki;
        }
    }

    MetricSample {
        cpu_m: total_cpu_m,
        mem_ki: total_mem_ki,
    }
}

/// Parse a Kubernetes `{"cpu": "100m", "memory": "256Mi"}` usage object.
fn parse_usage(usage: &Value) -> MetricSample {
    let cpu_str = usage.get("cpu").and_then(|v| v.as_str()).unwrap_or("0");
    let mem_str = usage.get("memory").and_then(|v| v.as_str()).unwrap_or("0");

    MetricSample {
        cpu_m: parse_cpu_to_millicores(cpu_str),
        mem_ki: parse_memory_to_ki(mem_str),
    }
}

/// Parse a Kubernetes CPU quantity string to millicores.
///
/// Examples: `"100m"` → 100, `"1"` → 1000, `"500n"` → 0 (nano-cores, rounded).
pub fn parse_cpu_to_millicores(s: &str) -> u64 {
    if s.is_empty() || s == "0" {
        return 0;
    }
    if let Some(rest) = s.strip_suffix('m') {
        return rest.parse::<u64>().unwrap_or(0);
    }
    if let Some(rest) = s.strip_suffix('n') {
        // nano-cores → millicores (÷ 1_000_000)
        return rest.parse::<u64>().unwrap_or(0) / 1_000_000;
    }
    if let Some(rest) = s.strip_suffix('u') {
        // micro-cores → millicores (÷ 1_000)
        return rest.parse::<u64>().unwrap_or(0) / 1_000;
    }
    // Bare number = whole cores
    s.parse::<u64>().unwrap_or(0) * 1_000
}

/// Parse a Kubernetes memory quantity string to kibibytes.
///
/// Examples: `"256Mi"` → 262144, `"1Gi"` → 1048576, `"512k"` → 512.
pub fn parse_memory_to_ki(s: &str) -> u64 {
    if s.is_empty() || s == "0" {
        return 0;
    }
    // Binary suffixes (powers of 1024)
    for (suffix, shift) in &[("Ki", 0u32), ("Mi", 10), ("Gi", 20), ("Ti", 30)] {
        if let Some(rest) = s.strip_suffix(suffix) {
            return rest.parse::<u64>().unwrap_or(0) << shift;
        }
    }
    // Decimal suffixes
    for (suffix, mul) in &[("k", 1u64), ("M", 1_000), ("G", 1_000_000)] {
        if let Some(rest) = s.strip_suffix(suffix) {
            let bytes = rest.parse::<u64>().unwrap_or(0) * mul * 1_000;
            return bytes / 1_024;
        }
    }
    // Bare bytes
    s.parse::<u64>().unwrap_or(0) / 1_024
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    // ── CPU parsing ──────────────────────────────────────────────────────────

    #[test]
    fn cpu_millicores_suffix() {
        assert_eq!(parse_cpu_to_millicores("100m"), 100);
        assert_eq!(parse_cpu_to_millicores("500m"), 500);
    }

    #[test]
    fn cpu_whole_cores() {
        assert_eq!(parse_cpu_to_millicores("1"), 1000);
        assert_eq!(parse_cpu_to_millicores("2"), 2000);
    }

    #[test]
    fn cpu_nanocores() {
        assert_eq!(parse_cpu_to_millicores("500000000n"), 500);
    }

    #[test]
    fn cpu_zero() {
        assert_eq!(parse_cpu_to_millicores("0"), 0);
        assert_eq!(parse_cpu_to_millicores(""), 0);
    }

    // ── Memory parsing ───────────────────────────────────────────────────────

    #[test]
    fn mem_mebibytes() {
        assert_eq!(parse_memory_to_ki("256Mi"), 256 * 1024);
        assert_eq!(parse_memory_to_ki("1Mi"), 1024);
    }

    #[test]
    fn mem_gibibytes() {
        assert_eq!(parse_memory_to_ki("1Gi"), 1024 * 1024);
    }

    #[test]
    fn mem_kibibytes() {
        assert_eq!(parse_memory_to_ki("4096Ki"), 4096);
    }

    #[test]
    fn mem_zero() {
        assert_eq!(parse_memory_to_ki("0"), 0);
    }

    // ── MetricsStore ─────────────────────────────────────────────────────────

    #[test]
    fn store_ingest_and_retrieve() {
        let mut store = MetricsStore::new();
        let mut snap = MetricsSnapshot::default();
        snap.pods.insert(
            "default/my-pod".into(),
            MetricSample {
                cpu_m: 200,
                mem_ki: 51200,
            },
        );
        store.ingest(&snap);

        let hist = store.pods.get("default/my-pod").unwrap();
        assert_eq!(hist.latest().unwrap().cpu_m, 200);
        assert_eq!(hist.latest().unwrap().mem_ki, 51200);
    }

    #[test]
    fn store_ring_buffer_caps_at_history_len() {
        let mut store = MetricsStore::new();
        for i in 0..=(HISTORY_LEN + 5) as u64 {
            let mut snap = MetricsSnapshot::default();
            snap.pods.insert(
                "ns/p".into(),
                MetricSample {
                    cpu_m: i,
                    mem_ki: i,
                },
            );
            store.ingest(&snap);
        }
        let hist = store.pods.get("ns/p").unwrap();
        assert_eq!(hist.samples.len(), HISTORY_LEN);
        // Oldest values should have been evicted; latest is the last pushed.
        assert_eq!(hist.latest().unwrap().cpu_m, (HISTORY_LEN + 5) as u64);
    }

    #[test]
    fn store_top_pods_by_cpu() {
        let mut store = MetricsStore::new();
        for (name, cpu) in &[("ns/a", 100u64), ("ns/b", 500), ("ns/c", 50)] {
            let mut snap = MetricsSnapshot::default();
            snap.pods.insert(
                name.to_string(),
                MetricSample {
                    cpu_m: *cpu,
                    mem_ki: 0,
                },
            );
            store.ingest(&snap);
        }
        let top = store.top_pods_by_cpu(2);
        assert_eq!(top[0].0, "ns/b");
        assert_eq!(top[1].0, "ns/a");
    }

    #[test]
    fn sparkline_data_order() {
        let mut hist = ResourceHistory::default();
        for i in 0..5u64 {
            hist.push(MetricSample {
                cpu_m: i * 10,
                mem_ki: 0,
            });
        }
        let data = hist.cpu_sparkline();
        assert_eq!(data, vec![0, 10, 20, 30, 40]);
    }

    // ── Usage parsing ────────────────────────────────────────────────────────

    #[test]
    fn parse_usage_fields() {
        let v = json!({"cpu": "250m", "memory": "128Mi"});
        let s = parse_usage(&v);
        assert_eq!(s.cpu_m, 250);
        assert_eq!(s.mem_ki, 128 * 1024);
    }

    #[test]
    fn aggregate_containers_sums_all() {
        let data = json!({
            "containers": [
                {"usage": {"cpu": "100m", "memory": "64Mi"}},
                {"usage": {"cpu": "200m", "memory": "128Mi"}}
            ]
        });
        let s = aggregate_containers(&data);
        assert_eq!(s.cpu_m, 300);
        assert_eq!(s.mem_ki, (64 + 128) * 1024);
    }
}
