//! Cluster metadata record types — Phase 37.
//!
//! Three record types are appended to daily JSON-lines files:
//! - `Snapshot` — cluster state at session start
//! - `Issue` — detected problem (from expert scan or event watcher)
//! - `Interaction` — user action (delete, scale, port-forward, AI chat, …)

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

// ─── Top-level record enum ────────────────────────────────────────────────────

/// A single JSON-lines record in a daily cluster metadata file.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum MetadataRecord {
    Snapshot(SnapshotRecord),
    Issue(IssueRecord),
    Interaction(InteractionRecord),
}

impl MetadataRecord {
    /// UTC timestamp of this record.
    pub fn timestamp(&self) -> &DateTime<Utc> {
        match self {
            MetadataRecord::Snapshot(r) => &r.ts,
            MetadataRecord::Issue(r) => &r.ts,
            MetadataRecord::Interaction(r) => &r.ts,
        }
    }

    /// Short type label: `"snapshot"`, `"issue"`, or `"interaction"`.
    pub fn type_label(&self) -> &'static str {
        match self {
            MetadataRecord::Snapshot(_) => "snapshot",
            MetadataRecord::Issue(_) => "issue",
            MetadataRecord::Interaction(_) => "interaction",
        }
    }

    /// One-line human-readable summary for the TUI list.
    pub fn summary(&self) -> String {
        match self {
            MetadataRecord::Snapshot(r) => format!(
                "nodes {}/{} ready  deploys {}/{}  namespaces: {}",
                r.nodes.ready,
                r.nodes.total,
                r.workloads.running,
                r.workloads.deployments,
                r.namespaces.join(", ")
            ),
            MetadataRecord::Issue(r) => format!(
                "{} {}/{}: {}",
                r.kind, r.namespace, r.resource, r.message
            ),
            MetadataRecord::Interaction(r) => format!(
                "{} {}/{} → {}",
                r.action.label(),
                r.namespace,
                r.resource,
                r.outcome
            ),
        }
    }
}

// ─── Snapshot ─────────────────────────────────────────────────────────────────

/// Cluster state captured at the start of each k7s session.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SnapshotRecord {
    pub ts: DateTime<Utc>,
    pub nodes: NodeSummary,
    /// All namespace names visible at snapshot time.
    pub namespaces: Vec<String>,
    pub workloads: WorkloadSummary,
    pub server_version: String,
}

impl SnapshotRecord {
    pub fn new(
        nodes: NodeSummary,
        namespaces: Vec<String>,
        workloads: WorkloadSummary,
        server_version: impl Into<String>,
    ) -> Self {
        Self {
            ts: Utc::now(),
            nodes,
            namespaces,
            workloads,
            server_version: server_version.into(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct NodeSummary {
    pub total: u32,
    pub ready: u32,
    pub not_ready: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct WorkloadSummary {
    pub deployments: u32,
    pub running: u32,
    pub degraded: u32,
}

// ─── Issue ────────────────────────────────────────────────────────────────────

/// A detected cluster problem.  Written by the expert scanner and event watcher.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IssueRecord {
    pub ts: DateTime<Utc>,
    /// Issue kind: `"CrashLoopBackOff"`, `"OOMKilled"`, `"NodeNotReady"`, …
    pub kind: String,
    pub namespace: String,
    pub resource: String,
    pub resource_kind: String,
    pub message: String,
    /// Set when the issue is later observed to have resolved.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resolved_at: Option<DateTime<Utc>>,
}

impl IssueRecord {
    pub fn new(
        kind: impl Into<String>,
        namespace: impl Into<String>,
        resource: impl Into<String>,
        resource_kind: impl Into<String>,
        message: impl Into<String>,
    ) -> Self {
        Self {
            ts: Utc::now(),
            kind: kind.into(),
            namespace: namespace.into(),
            resource: resource.into(),
            resource_kind: resource_kind.into(),
            message: message.into(),
            resolved_at: None,
        }
    }

    /// Mark this issue as resolved at the current time.
    pub fn resolve(&mut self) {
        self.resolved_at = Some(Utc::now());
    }

    pub fn is_resolved(&self) -> bool {
        self.resolved_at.is_some()
    }

    /// Deduplication key: (kind, namespace, resource).
    pub fn dedup_key(&self) -> String {
        format!("{}:{}:{}", self.kind, self.namespace, self.resource)
    }
}

// ─── Interaction ──────────────────────────────────────────────────────────────

/// A user action performed via the TUI or CLI.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InteractionRecord {
    pub ts: DateTime<Utc>,
    pub action: InteractionAction,
    pub namespace: String,
    pub resource: String,
    pub resource_kind: String,
    /// `"success"` / `"failed"` / `"cancelled"`.
    pub outcome: String,
}

impl InteractionRecord {
    pub fn new(
        action: InteractionAction,
        namespace: impl Into<String>,
        resource: impl Into<String>,
        resource_kind: impl Into<String>,
        outcome: impl Into<String>,
    ) -> Self {
        Self {
            ts: Utc::now(),
            action,
            namespace: namespace.into(),
            resource: resource.into(),
            resource_kind: resource_kind.into(),
            outcome: outcome.into(),
        }
    }
}

/// The category of user action recorded.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "PascalCase")]
pub enum InteractionAction {
    DeletePod,
    RolloutRestart,
    ScaleDeployment,
    ViewLogs,
    PortForward,
    AiAnalyse,
    HelmRollback,
    HelmUninstall,
    VelaRestart,
    VelaResume,
    VelaRollback,
    VelaDelete,
    ExpertRemediation,
    Other,
}

impl InteractionAction {
    pub fn label(&self) -> &'static str {
        match self {
            InteractionAction::DeletePod => "DeletePod",
            InteractionAction::RolloutRestart => "RolloutRestart",
            InteractionAction::ScaleDeployment => "ScaleDeployment",
            InteractionAction::ViewLogs => "ViewLogs",
            InteractionAction::PortForward => "PortForward",
            InteractionAction::AiAnalyse => "AiAnalyse",
            InteractionAction::HelmRollback => "HelmRollback",
            InteractionAction::HelmUninstall => "HelmUninstall",
            InteractionAction::VelaRestart => "VelaRestart",
            InteractionAction::VelaResume => "VelaResume",
            InteractionAction::VelaRollback => "VelaRollback",
            InteractionAction::VelaDelete => "VelaDelete",
            InteractionAction::ExpertRemediation => "ExpertRemediation",
            InteractionAction::Other => "Other",
        }
    }
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snapshot_type_label() {
        let r = MetadataRecord::Snapshot(SnapshotRecord::new(
            NodeSummary { total: 3, ready: 3, not_ready: 0 },
            vec!["default".into()],
            WorkloadSummary { deployments: 2, running: 2, degraded: 0 },
            "1.30.2",
        ));
        assert_eq!(r.type_label(), "snapshot");
    }

    #[test]
    fn issue_type_label() {
        let r = MetadataRecord::Issue(IssueRecord::new(
            "CrashLoopBackOff", "prod", "api-server", "Pod", "exited 137",
        ));
        assert_eq!(r.type_label(), "issue");
    }

    #[test]
    fn interaction_type_label() {
        let r = MetadataRecord::Interaction(InteractionRecord::new(
            InteractionAction::DeletePod,
            "prod", "bad-pod", "Pod", "success",
        ));
        assert_eq!(r.type_label(), "interaction");
    }

    #[test]
    fn issue_resolve_sets_resolved_at() {
        let mut issue = IssueRecord::new("OOMKilled", "prod", "worker", "Pod", "OOM");
        assert!(!issue.is_resolved());
        issue.resolve();
        assert!(issue.is_resolved());
        assert!(issue.resolved_at.is_some());
    }

    #[test]
    fn issue_dedup_key_format() {
        let issue = IssueRecord::new("CrashLoopBackOff", "prod", "api", "Pod", "msg");
        assert_eq!(issue.dedup_key(), "CrashLoopBackOff:prod:api");
    }

    #[test]
    fn snapshot_summary_contains_node_counts() {
        let r = MetadataRecord::Snapshot(SnapshotRecord::new(
            NodeSummary { total: 5, ready: 4, not_ready: 1 },
            vec!["default".into(), "prod".into()],
            WorkloadSummary { deployments: 10, running: 9, degraded: 1 },
            "1.30.1",
        ));
        let s = r.summary();
        assert!(s.contains("4/5"), "should show ready/total: {s}");
    }

    #[test]
    fn serde_round_trip_snapshot() {
        let record = MetadataRecord::Snapshot(SnapshotRecord::new(
            NodeSummary { total: 2, ready: 2, not_ready: 0 },
            vec!["default".into()],
            WorkloadSummary { deployments: 3, running: 3, degraded: 0 },
            "1.29.0",
        ));
        let json = serde_json::to_string(&record).unwrap();
        let back: MetadataRecord = serde_json::from_str(&json).unwrap();
        assert_eq!(back.type_label(), "snapshot");
    }

    #[test]
    fn serde_round_trip_issue() {
        let record = MetadataRecord::Issue(IssueRecord::new(
            "OOMKilled", "ns", "pod-x", "Pod", "memory limit exceeded",
        ));
        let json = serde_json::to_string(&record).unwrap();
        let back: MetadataRecord = serde_json::from_str(&json).unwrap();
        assert_eq!(back.type_label(), "issue");
    }

    #[test]
    fn interaction_action_label() {
        assert_eq!(InteractionAction::DeletePod.label(), "DeletePod");
        assert_eq!(InteractionAction::AiAnalyse.label(), "AiAnalyse");
        assert_eq!(InteractionAction::ExpertRemediation.label(), "ExpertRemediation");
    }

    #[test]
    fn serde_round_trip_interaction() {
        let record = MetadataRecord::Interaction(InteractionRecord::new(
            InteractionAction::ScaleDeployment,
            "default", "my-deploy", "Deployment", "success",
        ));
        let json = serde_json::to_string(&record).unwrap();
        let back: MetadataRecord = serde_json::from_str(&json).unwrap();
        assert_eq!(back.type_label(), "interaction");
    }
}
