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
            Self::Running      => "running",
            Self::Stopped      => "stopped",
            Self::Failed(_)    => "failed",
        }
    }
}

// ─── ForwardEntry ─────────────────────────────────────────────────────────────

/// A single managed port-forward with its child process.
pub struct ForwardEntry {
    pub id:         ForwardId,
    pub pod:        String,
    pub namespace:  String,
    /// Target port inside the pod.
    pub pod_port:   u16,
    /// Local port on 127.0.0.1.
    pub local_port: u16,
    pub status:     ForwardStatus,
    /// The running kubectl process (None after stop).
    child:          Option<Child>,
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
            Ok(None) => false,   // still running
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
            self.namespace, self.pod,
            self.pod, self.pod_port,
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
            context:  None,
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
        namespace:  impl Into<String>,
        pod:        impl Into<String>,
        pod_port:   u16,
        local_port: u16,
    ) -> anyhow::Result<ForwardId> {
        let ns  = namespace.into();
        let pod = pod.into();

        let mut args = vec!["port-forward".to_owned()];

        if let Some(ctx) = &self.context {
            args.extend(["--context".to_owned(), ctx.clone()]);
        }

        args.extend([
            "-n".to_owned(), ns.clone(),
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

        self.forwards.insert(id, ForwardEntry {
            id,
            pod,
            namespace: ns,
            pod_port,
            local_port,
            status: ForwardStatus::Running,
            child:  Some(child),
        });

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

    pub fn is_empty(&self) -> bool { self.forwards.is_empty() }
    pub fn len(&self)      -> usize { self.forwards.len() }
}

impl Default for PortForwardManager {
    fn default() -> Self { Self::new() }
}

// ─── Snapshot for TUI rendering ──────────────────────────────────────────────

/// Immutable view of a forward entry — passed to the list renderer.
#[derive(Debug, Clone)]
pub struct ForwardSnapshot {
    pub id:         ForwardId,
    pub namespace:  String,
    pub pod:        String,
    pub pod_port:   u16,
    pub local_port: u16,
    pub status:     String,
}

impl ForwardSnapshot {
    pub fn from_entry(e: &ForwardEntry) -> Self {
        Self {
            id:         e.id,
            namespace:  e.namespace.clone(),
            pod:        e.pod.clone(),
            pod_port:   e.pod_port,
            local_port: e.local_port,
            status:     match &e.status {
                ForwardStatus::Running   => "running".to_owned(),
                ForwardStatus::Stopped   => "stopped".to_owned(),
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
    pub entries:  Vec<ForwardSnapshot>,
    /// Selected row index.
    pub selected: usize,
}

impl PortForwardView {
    pub fn new() -> Self {
        Self { entries: Vec::new(), selected: 0 }
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

    pub fn up(&mut self)   { self.selected = self.selected.saturating_sub(1); }
    pub fn down(&mut self) {
        if !self.entries.is_empty() {
            self.selected = (self.selected + 1).min(self.entries.len() - 1);
        }
    }

    /// The currently selected forward id, if any.
    pub fn selected_id(&self) -> Option<ForwardId> {
        self.entries.get(self.selected).map(|e| e.id)
    }

    pub fn is_empty(&self) -> bool { self.entries.is_empty() }
}

impl Default for PortForwardView {
    fn default() -> Self { Self::new() }
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
            id:         ForwardId(10),
            pod:        "api".to_owned(),
            namespace:  "prod".to_owned(),
            pod_port:   8080,
            local_port: 9090,
            status:     ForwardStatus::Running,
            child:      None,
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
        assert_eq!(ForwardStatus::Running.as_str(),         "running");
        assert_eq!(ForwardStatus::Stopped.as_str(),         "stopped");
        assert_eq!(ForwardStatus::Failed("x".into()).as_str(), "failed");
    }
}
