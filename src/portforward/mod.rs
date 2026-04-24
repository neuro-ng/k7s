//! Port-forward manager — Phase 8.3 / 8.5.
//!
//! Each port-forward runs `kubectl port-forward` as a background child process.
//! The manager owns the process handles and exposes list/add/remove operations
//! for the TUI port-forward list view.
//!
//! Using kubectl as the subprocess avoids requiring the `kube/ws` feature and
//! matches the behaviour users expect (same credentials, proxy settings, etc.).

use std::collections::HashMap;
use std::process::{Child, Command};
use std::sync::atomic::{AtomicU64, Ordering};

use tokio_util::sync::CancellationToken;

// ─── Forward ID ───────────────────────────────────────────────────────────────

/// Unique identifier for an active port-forward.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ForwardId(u64);

static NEXT_ID: AtomicU64 = AtomicU64::new(1);

impl ForwardId {
    fn new() -> Self {
        Self(NEXT_ID.fetch_add(1, Ordering::Relaxed))
    }
}

impl std::fmt::Display for ForwardId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "pf-{}", self.0)
    }
}

// ─── Status ───────────────────────────────────────────────────────────────────

/// Runtime status of a port-forward.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ForwardStatus {
    /// Process launched, forwarding traffic.
    Running,
    /// Stopped by the user.
    Stopped,
    /// Process exited unexpectedly.
    Failed(String),
}

impl ForwardStatus {
    pub fn as_str(&self) -> &str {
        match self {
            Self::Running => "running",
            Self::Stopped => "stopped",
            Self::Failed(_) => "failed",
        }
    }
}

// ─── ForwardEntry ─────────────────────────────────────────────────────────────

/// A single managed port-forward with its child process.
pub struct ForwardEntry {
    pub id: ForwardId,
    pub pod: String,
    pub namespace: String,
    /// Target port inside the pod.
    pub pod_port: u16,
    /// Local port on 127.0.0.1.
    pub local_port: u16,
    pub status: ForwardStatus,
    /// The running kubectl process (None after stop).
    child: Option<Child>,
}

impl ForwardEntry {
    /// Stop the port-forward by killing the child process.
    pub fn stop(&mut self) {
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
        self.status = ForwardStatus::Stopped;
        tracing::info!(id = %self.id, "port-forward stopped");
    }

    /// Poll whether the child process has exited unexpectedly.
    ///
    /// Returns `true` if the status changed (i.e. process died).
    pub fn poll(&mut self) -> bool {
        if self.status != ForwardStatus::Running {
            return false;
        }
        let Some(child) = &mut self.child else {
            return false;
        };
        match child.try_wait() {
            Ok(Some(exit)) => {
                let msg = format!("exited with code {:?}", exit.code());
                tracing::warn!(id = %self.id, %msg, "port-forward process died");
                self.status = ForwardStatus::Failed(msg);
                true
            }
            Ok(None) => false, // still running
            Err(e) => {
                self.status = ForwardStatus::Failed(e.to_string());
                true
            }
        }
    }

    /// One-line summary for the list view.
    pub fn display(&self) -> String {
        format!(
            "{}/{} {}:{} → 127.0.0.1:{}  [{}]",
            self.namespace,
            self.pod,
            self.pod,
            self.pod_port,
            self.local_port,
            self.status.as_str(),
        )
    }
}

impl Drop for ForwardEntry {
    fn drop(&mut self) {
        // Ensure the child is killed when the entry is removed.
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

// ─── Manager ──────────────────────────────────────────────────────────────────

/// Manages all active port-forwards.
pub struct PortForwardManager {
    forwards: HashMap<ForwardId, ForwardEntry>,
    /// Optional kubeconfig context forwarded to kubectl.
    pub context: Option<String>,
}

impl PortForwardManager {
    pub fn new() -> Self {
        Self {
            forwards: HashMap::new(),
            context: None,
        }
    }

    pub fn with_context(mut self, ctx: impl Into<String>) -> Self {
        self.context = Some(ctx.into());
        self
    }

    /// Start a new port-forward.
    ///
    /// Returns the assigned `ForwardId` on success, or an error string if
    /// `kubectl port-forward` could not be spawned.
    pub fn add(
        &mut self,
        namespace: impl Into<String>,
        pod: impl Into<String>,
        pod_port: u16,
        local_port: u16,
    ) -> anyhow::Result<ForwardId> {
        let ns = namespace.into();
        let pod = pod.into();

        let mut args = vec!["port-forward".to_owned()];

        if let Some(ctx) = &self.context {
            args.extend(["--context".to_owned(), ctx.clone()]);
        }

        args.extend([
            "-n".to_owned(),
            ns.clone(),
            pod.clone(),
            format!("{local_port}:{pod_port}"),
        ]);

        let child = Command::new("kubectl")
            .args(&args)
            // Suppress kubectl's stdout/stderr so it doesn't corrupt the TUI.
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .map_err(|e| anyhow::anyhow!("kubectl port-forward failed to start: {e}"))?;

        let id = ForwardId::new();

        tracing::info!(
            id = %id,
            namespace = %ns,
            pod = %pod,
            pod_port,
            local_port,
            "port-forward started"
        );

        self.forwards.insert(
            id,
            ForwardEntry {
                id,
                pod,
                namespace: ns,
                pod_port,
                local_port,
                status: ForwardStatus::Running,
                child: Some(child),
            },
        );

        Ok(id)
    }

    /// Stop and remove a forward by id.
    pub fn remove(&mut self, id: ForwardId) {
        if let Some(mut entry) = self.forwards.remove(&id) {
            entry.stop();
        }
    }

    /// Poll all running forwards for unexpected exits.
    ///
    /// Call this on each render tick to detect crashed forwards.
    pub fn poll_all(&mut self) {
        for entry in self.forwards.values_mut() {
            entry.poll();
        }
    }

    /// All forwards sorted by id (stable display order).
    pub fn all_sorted(&self) -> Vec<&ForwardEntry> {
        let mut v: Vec<_> = self.forwards.values().collect();
        v.sort_by_key(|e| e.id.0);
        v
    }

    /// Only running forwards.
    pub fn running(&self) -> Vec<&ForwardEntry> {
        self.forwards
            .values()
            .filter(|e| e.status == ForwardStatus::Running)
            .collect()
    }

    pub fn is_empty(&self) -> bool {
        self.forwards.is_empty()
    }
    pub fn len(&self) -> usize {
        self.forwards.len()
    }
}

impl Default for PortForwardManager {
    fn default() -> Self {
        Self::new()
    }
}

// ─── Snapshot for TUI rendering ──────────────────────────────────────────────

/// Immutable view of a forward entry — passed to the list renderer.
#[derive(Debug, Clone)]
pub struct ForwardSnapshot {
    pub id: ForwardId,
    pub namespace: String,
    pub pod: String,
    pub pod_port: u16,
    pub local_port: u16,
    pub status: String,
}

impl ForwardSnapshot {
    pub fn from_entry(e: &ForwardEntry) -> Self {
        Self {
            id: e.id,
            namespace: e.namespace.clone(),
            pod: e.pod.clone(),
            pod_port: e.pod_port,
            local_port: e.local_port,
            status: match &e.status {
                ForwardStatus::Running => "running".to_owned(),
                ForwardStatus::Stopped => "stopped".to_owned(),
                ForwardStatus::Failed(m) => format!("failed: {m}"),
            },
        }
    }
}

// ─── Port-forward list view (Phase 8.5) ──────────────────────────────────────

/// TUI list view for active port-forwards.
///
/// Renders a table of current forwards with their status.
pub struct PortForwardView {
    /// Snapshot of current forwards (refreshed each tick from the manager).
    pub entries: Vec<ForwardSnapshot>,
    /// Selected row index.
    pub selected: usize,
}

impl PortForwardView {
    pub fn new() -> Self {
        Self {
            entries: Vec::new(),
            selected: 0,
        }
    }

    /// Refresh from the manager state.
    pub fn refresh(&mut self, manager: &PortForwardManager) {
        self.entries = manager
            .all_sorted()
            .iter()
            .map(|e| ForwardSnapshot::from_entry(e))
            .collect();

        // Clamp selection.
        if !self.entries.is_empty() {
            self.selected = self.selected.min(self.entries.len() - 1);
        }
    }

    pub fn up(&mut self) {
        self.selected = self.selected.saturating_sub(1);
    }
    pub fn down(&mut self) {
        if !self.entries.is_empty() {
            self.selected = (self.selected + 1).min(self.entries.len() - 1);
        }
    }

    /// The currently selected forward id, if any.
    pub fn selected_id(&self) -> Option<ForwardId> {
        self.entries.get(self.selected).map(|e| e.id)
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

impl Default for PortForwardView {
    fn default() -> Self {
        Self::new()
    }
}

// ─── Fast-forwards — Phase 8.6 ───────────────────────────────────────────────

/// Annotation key k7s reads to auto-start port-forwards.
///
/// Supported annotation value formats:
///
/// | Value | Meaning |
/// |-------|---------|
/// | `"8080:80"` | local port 8080 → pod port 80 |
/// | `"8080"` | local port 8080 → pod port 8080 (same) |
/// | `"8080:80,9090:90"` | multiple forwards, comma-separated |
///
/// Example annotation on a pod or service:
/// ```yaml
/// metadata:
///   annotations:
///     k7s.io/portforward: "8080:80,9090:90"
/// ```
pub const FAST_FORWARD_ANNOTATION: &str = "k7s.io/portforward";

/// A single port-forward spec parsed from the fast-forward annotation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FastForwardSpec {
    /// Local port to bind on 127.0.0.1.
    pub local_port: u16,
    /// Target port inside the pod/service.
    pub pod_port: u16,
    /// Target pod or service name.
    pub target: String,
    /// Namespace.
    pub namespace: String,
}

/// Result of applying fast-forwards to a set of resources.
#[derive(Debug, Default)]
pub struct FastForwardResult {
    /// Forwards that were successfully started.
    pub started: Vec<FastForwardSpec>,
    /// Forwards that were skipped because an equivalent one is already running.
    pub skipped: Vec<FastForwardSpec>,
    /// Forwards that failed to start, with error messages.
    pub failed: Vec<(FastForwardSpec, String)>,
}

/// Parse a fast-forward annotation value into a list of [`FastForwardSpec`]s.
///
/// Returns an empty `Vec` if the value is empty or malformed.
///
/// ```
/// use k7s::portforward::parse_fast_forward_annotation;
/// let specs = parse_fast_forward_annotation("8080:80,9090:90", "nginx", "default");
/// assert_eq!(specs.len(), 2);
/// assert_eq!(specs[0].local_port, 8080);
/// assert_eq!(specs[0].pod_port, 80);
/// assert_eq!(specs[1].local_port, 9090);
/// assert_eq!(specs[1].pod_port, 90);
/// ```
pub fn parse_fast_forward_annotation(
    value: &str,
    target: &str,
    namespace: &str,
) -> Vec<FastForwardSpec> {
    value
        .split(',')
        .filter_map(|part| {
            let part = part.trim();
            if part.is_empty() {
                return None;
            }
            let (local_port, pod_port) = if let Some((l, p)) = part.split_once(':') {
                let l: u16 = l.trim().parse().ok()?;
                let p: u16 = p.trim().parse().ok()?;
                (l, p)
            } else {
                let port: u16 = part.parse().ok()?;
                (port, port)
            };
            Some(FastForwardSpec {
                local_port,
                pod_port,
                target: target.to_owned(),
                namespace: namespace.to_owned(),
            })
        })
        .collect()
}

impl PortForwardManager {
    /// Scan a list of pod/service metadata objects for the fast-forward
    /// annotation and start any requested port-forwards that are not already
    /// running.
    ///
    /// `resources` is a slice of `(name, namespace, annotations)` tuples
    /// extracted from the live resource list.  Pass the pod/service
    /// annotations map as a `HashMap<String, String>`.
    ///
    /// Returns a [`FastForwardResult`] summarising what was started, skipped,
    /// or failed.
    pub fn apply_fast_forwards(
        &mut self,
        resources: &[(String, String, std::collections::HashMap<String, String>)],
    ) -> FastForwardResult {
        let mut result = FastForwardResult::default();

        for (name, namespace, annotations) in resources {
            let Some(value) = annotations.get(FAST_FORWARD_ANNOTATION) else {
                continue;
            };

            let specs = parse_fast_forward_annotation(value, name, namespace);
            for spec in specs {
                // Skip if an identical forward is already running.
                let already_running = self.forwards.values().any(|e| {
                    e.status == ForwardStatus::Running
                        && e.pod == spec.target
                        && e.namespace == spec.namespace
                        && e.pod_port == spec.pod_port
                        && e.local_port == spec.local_port
                });

                if already_running {
                    tracing::debug!(
                        target = %spec.target,
                        namespace = %spec.namespace,
                        local = spec.local_port,
                        pod = spec.pod_port,
                        "fast-forward already running — skipping"
                    );
                    result.skipped.push(spec);
                    continue;
                }

                match self.add(
                    &spec.namespace,
                    &spec.target,
                    spec.pod_port,
                    spec.local_port,
                ) {
                    Ok(_id) => {
                        tracing::info!(
                            target = %spec.target,
                            namespace = %spec.namespace,
                            local = spec.local_port,
                            pod = spec.pod_port,
                            "fast-forward started"
                        );
                        result.started.push(spec);
                    }
                    Err(e) => {
                        tracing::warn!(
                            target = %spec.target,
                            namespace = %spec.namespace,
                            local = spec.local_port,
                            pod = spec.pod_port,
                            error = %e,
                            "fast-forward failed to start"
                        );
                        result.failed.push((spec, e.to_string()));
                    }
                }
            }
        }

        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn forward_id_unique() {
        assert_ne!(ForwardId::new(), ForwardId::new());
    }

    #[test]
    fn manager_starts_empty() {
        let m = PortForwardManager::new();
        assert!(m.is_empty());
    }

    #[test]
    fn snapshot_from_running_entry() {
        let entry = ForwardEntry {
            id: ForwardId(10),
            pod: "api".to_owned(),
            namespace: "prod".to_owned(),
            pod_port: 8080,
            local_port: 9090,
            status: ForwardStatus::Running,
            child: None,
        };
        let s = ForwardSnapshot::from_entry(&entry);
        assert_eq!(s.status, "running");
        assert_eq!(s.local_port, 9090);
    }

    #[test]
    fn pf_view_refresh_and_navigation() {
        let m = PortForwardManager::new();
        let mut view = PortForwardView::new();
        view.refresh(&m);
        assert!(view.is_empty());
        assert_eq!(view.selected_id(), None);
    }

    #[test]
    fn forward_status_as_str() {
        assert_eq!(ForwardStatus::Running.as_str(), "running");
        assert_eq!(ForwardStatus::Stopped.as_str(), "stopped");
        assert_eq!(ForwardStatus::Failed("x".into()).as_str(), "failed");
    }

    // ── Fast-forward annotation parsing ──────────────────────────────────────

    #[test]
    fn parse_single_port_pair() {
        let specs = parse_fast_forward_annotation("8080:80", "nginx", "default");
        assert_eq!(specs.len(), 1);
        assert_eq!(specs[0].local_port, 8080);
        assert_eq!(specs[0].pod_port, 80);
        assert_eq!(specs[0].target, "nginx");
        assert_eq!(specs[0].namespace, "default");
    }

    #[test]
    fn parse_same_port_shorthand() {
        let specs = parse_fast_forward_annotation("8080", "svc", "prod");
        assert_eq!(specs.len(), 1);
        assert_eq!(specs[0].local_port, 8080);
        assert_eq!(specs[0].pod_port, 8080);
    }

    #[test]
    fn parse_multiple_pairs_comma_separated() {
        let specs = parse_fast_forward_annotation("8080:80,9090:90", "api", "ns1");
        assert_eq!(specs.len(), 2);
        assert_eq!(specs[0].local_port, 8080);
        assert_eq!(specs[0].pod_port, 80);
        assert_eq!(specs[1].local_port, 9090);
        assert_eq!(specs[1].pod_port, 90);
    }

    #[test]
    fn parse_empty_annotation_returns_empty() {
        assert!(parse_fast_forward_annotation("", "x", "y").is_empty());
    }

    #[test]
    fn parse_malformed_skips_bad_entries() {
        // "notaport:80" should be skipped; "8080:80" should pass.
        let specs = parse_fast_forward_annotation("notaport:80,8080:80", "svc", "ns");
        assert_eq!(specs.len(), 1);
        assert_eq!(specs[0].local_port, 8080);
    }

    #[test]
    fn parse_whitespace_is_trimmed() {
        let specs = parse_fast_forward_annotation(" 8080 : 80 ", "svc", "ns");
        assert_eq!(specs.len(), 1);
        assert_eq!(specs[0].local_port, 8080);
        assert_eq!(specs[0].pod_port, 80);
    }
}
