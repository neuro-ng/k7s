//! Cluster health assessment — Phase 6.20 / 14.2.
//!
//! Provides the [`ClusterSummary`] data model that the Pulse view uses to
//! display a cluster-at-a-glance dashboard.  The summary is built from
//! pre-serialised resource snapshots (so it works with both live data and
//! tests without requiring a running cluster).
//!
//! Phase 14.2 adds the [`mem`] sub-module for process memory statistics.
//!
//! # k9s Reference
//! `internal/view/pulse.go`

pub mod mem;
pub use mem::HeapStats;

use serde_json::Value;

/// Counts of resources in a given state.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StateCounts {
    pub running: usize,
    pub pending: usize,
    pub failed: usize,
    pub unknown: usize,
    pub total: usize,
}

impl StateCounts {
    fn add_phase(&mut self, phase: &str) {
        self.total += 1;
        match phase {
            "Running" | "Active" | "Bound" => self.running += 1,
            "Pending" | "Terminating" => self.pending += 1,
            "Failed" | "Error" => self.failed += 1,
            _ => self.unknown += 1,
        }
    }
}

/// Node-level aggregates.
#[derive(Debug, Clone, Default)]
pub struct NodeSummary {
    pub ready: usize,
    pub not_ready: usize,
    pub total: usize,
}

/// A roll-up of the cluster's current state for display on the Pulse view.
#[derive(Debug, Clone, Default)]
pub struct ClusterSummary {
    pub pods: StateCounts,
    pub deployments: StateCounts,
    pub nodes: NodeSummary,
    pub namespaces: usize,
    pub events_warn: usize,
    pub events_total: usize,
}

impl ClusterSummary {
    pub fn new() -> Self {
        Self::default()
    }

    /// Accumulate a pod JSON value into the summary.
    pub fn add_pod(&mut self, obj: &Value) {
        let phase = obj
            .pointer("/status/phase")
            .and_then(|v| v.as_str())
            .unwrap_or("Unknown");
        self.pods.add_phase(phase);
    }

    /// Accumulate a deployment JSON value.
    pub fn add_deployment(&mut self, obj: &Value) {
        let available = obj
            .pointer("/status/availableReplicas")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        let desired = obj
            .pointer("/spec/replicas")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        self.deployments.total += 1;
        if desired == 0 || available >= desired {
            self.deployments.running += 1;
        } else if available == 0 {
            self.deployments.failed += 1;
        } else {
            self.deployments.pending += 1;
        }
    }

    /// Accumulate a node JSON value.
    pub fn add_node(&mut self, obj: &Value) {
        self.nodes.total += 1;
        let ready = node_is_ready(obj);
        if ready {
            self.nodes.ready += 1;
        } else {
            self.nodes.not_ready += 1;
        }
    }

    /// Count a namespace.
    pub fn add_namespace(&mut self) {
        self.namespaces += 1;
    }

    /// Accumulate an event JSON value.
    pub fn add_event(&mut self, obj: &Value) {
        self.events_total += 1;
        let ty = obj.pointer("/type").and_then(|v| v.as_str()).unwrap_or("");
        if ty == "Warning" {
            self.events_warn += 1;
        }
    }
}

/// True when the node has a `Ready=True` condition.
fn node_is_ready(obj: &Value) -> bool {
    let Some(conditions) = obj.pointer("/status/conditions").and_then(|v| v.as_array()) else {
        return false;
    };
    conditions.iter().any(|c| {
        c.get("type").and_then(|v| v.as_str()) == Some("Ready")
            && c.get("status").and_then(|v| v.as_str()) == Some("True")
    })
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn pod_phases_counted() {
        let mut s = ClusterSummary::new();
        s.add_pod(&json!({"status": {"phase": "Running"}}));
        s.add_pod(&json!({"status": {"phase": "Pending"}}));
        s.add_pod(&json!({"status": {"phase": "Failed"}}));
        assert_eq!(s.pods.running, 1);
        assert_eq!(s.pods.pending, 1);
        assert_eq!(s.pods.failed, 1);
        assert_eq!(s.pods.total, 3);
    }

    #[test]
    fn deployment_ready_counts_as_running() {
        let mut s = ClusterSummary::new();
        s.add_deployment(&json!({"spec": {"replicas": 3}, "status": {"availableReplicas": 3}}));
        assert_eq!(s.deployments.running, 1);
        assert_eq!(s.deployments.failed, 0);
    }

    #[test]
    fn deployment_degraded_counts_as_pending() {
        let mut s = ClusterSummary::new();
        s.add_deployment(&json!({"spec": {"replicas": 3}, "status": {"availableReplicas": 1}}));
        assert_eq!(s.deployments.pending, 1);
    }

    #[test]
    fn node_ready_state() {
        let mut s = ClusterSummary::new();
        let ready_node = json!({
            "status": {"conditions": [{"type": "Ready", "status": "True"}]}
        });
        let not_ready_node = json!({
            "status": {"conditions": [{"type": "Ready", "status": "False"}]}
        });
        s.add_node(&ready_node);
        s.add_node(&not_ready_node);
        assert_eq!(s.nodes.ready, 1);
        assert_eq!(s.nodes.not_ready, 1);
        assert_eq!(s.nodes.total, 2);
    }

    #[test]
    fn events_warning_counted() {
        let mut s = ClusterSummary::new();
        s.add_event(&json!({"type": "Warning", "reason": "OOMKill"}));
        s.add_event(&json!({"type": "Normal", "reason": "Started"}));
        assert_eq!(s.events_warn, 1);
        assert_eq!(s.events_total, 2);
    }
}
