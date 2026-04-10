//! Resource-specific action sets.
//!
//! Each resource type supports a different set of keyboard actions.  This
//! module defines what actions are available per GVR and is used by the
//! view registrar to populate the hints bar and key binding map.

use crate::client::Gvr;
use crate::client::gvr::well_known;

/// A single action that can be performed on a selected resource.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResourceAction {
    /// Key displayed in the hints bar (e.g. `"l"`).
    pub key: &'static str,
    /// Short description shown in the hints bar (e.g. `"Logs"`).
    pub label: &'static str,
    /// Machine-readable action identifier.
    pub id: ActionId,
}

/// All possible resource actions across all resource types.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ActionId {
    /// View describe output (`kubectl describe`).
    Describe,
    /// View YAML manifest.
    Yaml,
    /// Delete the resource.
    Delete,
    /// Stream container logs (pods only).
    Logs,
    /// Open a shell into a container (pods only).
    Shell,
    /// Scale replica count (deployments, statefulsets, replicasets).
    Scale,
    /// Trigger a rolling restart (deployments, statefulsets, daemonsets).
    Restart,
    /// Manually trigger a cronjob.
    Trigger,
    /// Cordon a node (nodes only).
    Cordon,
    /// Uncordon a node (nodes only).
    Uncordon,
}

impl ResourceAction {
    const fn new(key: &'static str, label: &'static str, id: ActionId) -> Self {
        Self { key, label, id }
    }
}

// ─── Common action sets ────────────────────────────────────────────────────────

const DESCRIBE: ResourceAction = ResourceAction::new("d", "Describe", ActionId::Describe);
const YAML:     ResourceAction = ResourceAction::new("y", "YAML",     ActionId::Yaml);
const DELETE:   ResourceAction = ResourceAction::new("Ctrl-d", "Delete",   ActionId::Delete);

// ─── Per-resource action tables ───────────────────────────────────────────────

/// Return the ordered list of actions available for a given GVR.
pub fn actions_for(gvr: &Gvr) -> Vec<ResourceAction> {
    match gvr {
        g if *g == well_known::pods() => vec![
            ResourceAction::new("l", "Logs",    ActionId::Logs),
            ResourceAction::new("s", "Shell",   ActionId::Shell),
            DESCRIBE,
            YAML,
            DELETE,
        ],
        g if *g == well_known::deployments() => vec![
            ResourceAction::new("s", "Scale",   ActionId::Scale),
            ResourceAction::new("r", "Restart", ActionId::Restart),
            DESCRIBE,
            YAML,
            DELETE,
        ],
        g if *g == well_known::stateful_sets() => vec![
            ResourceAction::new("s", "Scale",   ActionId::Scale),
            ResourceAction::new("r", "Restart", ActionId::Restart),
            DESCRIBE,
            YAML,
            DELETE,
        ],
        g if *g == well_known::daemon_sets() => vec![
            ResourceAction::new("r", "Restart", ActionId::Restart),
            DESCRIBE,
            YAML,
            DELETE,
        ],
        g if *g == well_known::replica_sets() => vec![
            ResourceAction::new("s", "Scale",   ActionId::Scale),
            DESCRIBE,
            YAML,
            DELETE,
        ],
        g if *g == well_known::cron_jobs() => vec![
            ResourceAction::new("t", "Trigger", ActionId::Trigger),
            DESCRIBE,
            YAML,
            DELETE,
        ],
        g if *g == well_known::nodes() => vec![
            ResourceAction::new("c", "Cordon",   ActionId::Cordon),
            ResourceAction::new("u", "Uncordon", ActionId::Uncordon),
            DESCRIBE,
            YAML,
        ],
        _ => vec![DESCRIBE, YAML, DELETE],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pods_have_logs_and_shell() {
        let actions = actions_for(&well_known::pods());
        assert!(actions.iter().any(|a| a.id == ActionId::Logs));
        assert!(actions.iter().any(|a| a.id == ActionId::Shell));
    }

    #[test]
    fn deployments_have_scale_and_restart() {
        let actions = actions_for(&well_known::deployments());
        assert!(actions.iter().any(|a| a.id == ActionId::Scale));
        assert!(actions.iter().any(|a| a.id == ActionId::Restart));
    }

    #[test]
    fn cronjobs_have_trigger() {
        let actions = actions_for(&well_known::cron_jobs());
        assert!(actions.iter().any(|a| a.id == ActionId::Trigger));
    }

    #[test]
    fn nodes_have_cordon_but_no_delete() {
        let actions = actions_for(&well_known::nodes());
        assert!(actions.iter().any(|a| a.id == ActionId::Cordon));
        assert!(actions.iter().any(|a| a.id == ActionId::Uncordon));
        assert!(!actions.iter().any(|a| a.id == ActionId::Delete));
    }

    #[test]
    fn unknown_gvr_has_describe_and_yaml() {
        let gvr = Gvr::new("custom.io", "v1alpha1", "widgets");
        let actions = actions_for(&gvr);
        assert!(actions.iter().any(|a| a.id == ActionId::Describe));
        assert!(actions.iter().any(|a| a.id == ActionId::Yaml));
    }
}
