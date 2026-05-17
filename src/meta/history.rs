//! Cluster history summariser — Phase 37.
//!
//! Aggregates raw `MetadataRecord` slices into a `ClusterHistory` summary
//! suitable for injecting into the LLM context window (~300 tokens).

use std::collections::HashMap;

use super::record::{InteractionAction, MetadataRecord, SnapshotRecord, WorkloadSummary};

// ─── ClusterHistory ───────────────────────────────────────────────────────────

/// Human-readable cluster history summary derived from recent daily records.
#[derive(Debug, Default)]
pub struct ClusterHistory {
    /// Recurring issues: (description string, count).
    pub recurrent_issues: Vec<(String, usize)>,
    /// Issues that appeared and were later resolved.
    pub resolved_issues: Vec<String>,
    /// Operator actions: (action label, count).
    pub operator_actions: Vec<(String, usize)>,
    /// Change in deployment count from first to last snapshot (positive = growth).
    pub deployment_drift: Option<i64>,
    /// Change in node count from first to last snapshot.
    pub node_drift: Option<i64>,
    /// Days of history represented.
    pub days: u8,
    /// Cluster context name.
    pub context: String,
}

impl ClusterHistory {
    /// Render a ~300-token context block for LLM injection.
    pub fn to_context_block(&self) -> String {
        if self.is_empty() {
            return format!(
                "[Cluster History — {} — no records in last {} days]\n",
                self.context, self.days
            );
        }

        let mut lines = vec![format!(
            "[Cluster History — {} — last {} days]",
            self.context, self.days
        )];

        if !self.recurrent_issues.is_empty() {
            let issues: Vec<String> = self
                .recurrent_issues
                .iter()
                .take(5)
                .map(|(desc, count)| format!("{desc} (×{count})"))
                .collect();
            lines.push(format!("• Recurrent issues:  {}", issues.join(", ")));
        }

        if !self.resolved_issues.is_empty() {
            let resolved: Vec<String> = self.resolved_issues.iter().take(3).cloned().collect();
            lines.push(format!("• Resolved:          {}", resolved.join(", ")));
        }

        if !self.operator_actions.is_empty() {
            let actions: Vec<String> = self
                .operator_actions
                .iter()
                .take(6)
                .map(|(label, count)| format!("{label} ×{count}"))
                .collect();
            lines.push(format!("• Operator actions:  {}", actions.join(", ")));
        }

        if let Some(drift) = self.deployment_drift {
            let sign = if drift >= 0 { "+" } else { "" };
            lines.push(format!("• Workload drift:    deployments {sign}{drift}"));
        }

        if let Some(drift) = self.node_drift {
            if drift != 0 {
                let sign = if drift >= 0 { "+" } else { "" };
                lines.push(format!("• Node drift:        {sign}{drift}"));
            } else {
                lines.push("• Nodes:             steady".to_string());
            }
        }

        lines.join("\n") + "\n"
    }

    /// True when no meaningful history data is present.
    pub fn is_empty(&self) -> bool {
        self.recurrent_issues.is_empty()
            && self.resolved_issues.is_empty()
            && self.operator_actions.is_empty()
            && self.deployment_drift.is_none()
    }
}

// ─── Summarise ────────────────────────────────────────────────────────────────

/// Build a `ClusterHistory` from `records` spanning the last `days` days.
pub fn summarise(records: &[MetadataRecord], days: u8, context: &str) -> ClusterHistory {
    let mut issue_counts: HashMap<String, usize> = HashMap::new();
    let mut resolved: Vec<String> = Vec::new();
    let mut action_counts: HashMap<String, usize> = HashMap::new();
    let mut snapshots: Vec<&SnapshotRecord> = Vec::new();

    for record in records {
        match record {
            MetadataRecord::Issue(issue) => {
                if issue.is_resolved() {
                    resolved.push(format!(
                        "{} in {}/{} (resolved)",
                        issue.kind, issue.namespace, issue.resource
                    ));
                } else {
                    *issue_counts.entry(issue.dedup_key()).or_insert(0) += 1;
                }
            }
            MetadataRecord::Interaction(interaction) => {
                *action_counts
                    .entry(interaction.action.label().to_string())
                    .or_insert(0) += 1;
            }
            MetadataRecord::Snapshot(snap) => {
                snapshots.push(snap);
            }
        }
    }

    // Build recurrent issues list, sorted by count descending.
    let mut recurrent: Vec<(String, usize)> = issue_counts
        .into_iter()
        .filter(|(_, count)| *count >= 1)
        .map(|(key, count)| {
            // Convert "kind:namespace:resource" back to a readable label.
            let label = key.replace(':', " in ");
            (label, count)
        })
        .collect();
    recurrent.sort_by(|a, b| b.1.cmp(&a.1));

    // Build operator action summary.
    let mut actions: Vec<(String, usize)> = action_counts.into_iter().collect();
    actions.sort_by(|a, b| b.1.cmp(&a.1));

    // Drift: compare first and last snapshot.
    let (deployment_drift, node_drift) = compute_drift(&snapshots);

    // Deduplicate resolved issues.
    resolved.dedup();
    resolved.truncate(10);

    ClusterHistory {
        recurrent_issues: recurrent,
        resolved_issues: resolved,
        operator_actions: actions,
        deployment_drift,
        node_drift,
        days,
        context: context.to_string(),
    }
}

fn compute_drift(snapshots: &[&SnapshotRecord]) -> (Option<i64>, Option<i64>) {
    if snapshots.len() < 2 {
        return (None, None);
    }
    let first = snapshots[0];
    let last = snapshots[snapshots.len() - 1];

    let deploy_drift = last.workloads.deployments as i64 - first.workloads.deployments as i64;
    let node_drift = last.nodes.total as i64 - first.nodes.total as i64;

    (Some(deploy_drift), Some(node_drift))
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::meta::record::{
        InteractionRecord, IssueRecord, NodeSummary, SnapshotRecord, WorkloadSummary,
    };

    fn snap(deploys: u32, nodes: u32) -> MetadataRecord {
        MetadataRecord::Snapshot(SnapshotRecord::new(
            NodeSummary {
                total: nodes,
                ready: nodes,
                not_ready: 0,
            },
            vec!["default".into()],
            WorkloadSummary {
                deployments: deploys,
                running: deploys,
                degraded: 0,
            },
            "1.30.0",
        ))
    }

    fn issue(kind: &str, ns: &str, resource: &str) -> MetadataRecord {
        MetadataRecord::Issue(IssueRecord::new(kind, ns, resource, "Pod", "msg"))
    }

    fn resolved_issue(kind: &str, ns: &str, resource: &str) -> MetadataRecord {
        let mut r = IssueRecord::new(kind, ns, resource, "Pod", "msg");
        r.resolve();
        MetadataRecord::Issue(r)
    }

    fn interact(action: InteractionAction) -> MetadataRecord {
        MetadataRecord::Interaction(InteractionRecord::new(
            action,
            "prod",
            "deploy-x",
            "Deployment",
            "success",
        ))
    }

    #[test]
    fn empty_records_produces_empty_history() {
        let h = summarise(&[], 7, "prod");
        assert!(h.is_empty());
    }

    #[test]
    fn recurrent_issue_counted() {
        let records = vec![
            issue("CrashLoopBackOff", "prod", "api"),
            issue("CrashLoopBackOff", "prod", "api"),
        ];
        let h = summarise(&records, 7, "test");
        assert_eq!(h.recurrent_issues.len(), 1);
        assert_eq!(h.recurrent_issues[0].1, 2);
    }

    #[test]
    fn resolved_issue_separated() {
        let records = vec![resolved_issue("OOMKilled", "prod", "worker")];
        let h = summarise(&records, 7, "test");
        assert!(h.recurrent_issues.is_empty());
        assert_eq!(h.resolved_issues.len(), 1);
        assert!(h.resolved_issues[0].contains("OOMKilled"));
    }

    #[test]
    fn operator_actions_counted() {
        let records = vec![
            interact(InteractionAction::DeletePod),
            interact(InteractionAction::DeletePod),
            interact(InteractionAction::ScaleDeployment),
        ];
        let h = summarise(&records, 7, "test");
        let delete = h.operator_actions.iter().find(|(l, _)| l == "DeletePod");
        assert_eq!(delete.unwrap().1, 2);
    }

    #[test]
    fn deployment_drift_computed() {
        let records = vec![snap(10, 3), snap(13, 3)];
        let h = summarise(&records, 7, "test");
        assert_eq!(h.deployment_drift, Some(3));
        assert_eq!(h.node_drift, Some(0));
    }

    #[test]
    fn single_snapshot_no_drift() {
        let records = vec![snap(5, 2)];
        let h = summarise(&records, 7, "test");
        assert_eq!(h.deployment_drift, None);
    }

    #[test]
    fn to_context_block_contains_header() {
        let mut records = vec![snap(5, 3), snap(4, 3)];
        records.push(issue("CrashLoopBackOff", "prod", "api"));
        let h = summarise(&records, 7, "mycluster");
        let block = h.to_context_block();
        assert!(
            block.contains("mycluster"),
            "header should contain context: {block}"
        );
        assert!(
            block.contains("Recurrent"),
            "should mention issues: {block}"
        );
        assert!(
            block.contains("−1") || block.contains("-1"),
            "drift should appear: {block}"
        );
    }

    #[test]
    fn empty_history_to_context_block_says_no_records() {
        let h = summarise(&[], 7, "ctx");
        let block = h.to_context_block();
        assert!(block.contains("no records"), "{block}");
    }

    #[test]
    fn context_block_caps_issue_list() {
        // 10 distinct issues → only 5 shown in context block.
        let records: Vec<MetadataRecord> = (0..10)
            .map(|i| issue("CrashLoopBackOff", "prod", &format!("pod-{i}")))
            .collect();
        let h = summarise(&records, 7, "test");
        let block = h.to_context_block();
        // Count occurrences of "×" to verify at most 5 issues listed.
        let count = block.matches('×').count();
        assert!(count <= 5, "should show at most 5 issues: {block}");
    }
}
