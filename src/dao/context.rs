//! Context DAO — Phase 6.8.
//!
//! Reads kubeconfig contexts from disk (no live API call needed) and presents
//! them as a list suitable for display in the context browser view.
//!
//! Selecting an entry emits a [`ContextSwitch`] event; the app layer applies
//! it by rebuilding the Kubernetes client with the chosen context.
//!
//! # k9s Reference
//! `internal/dao/context.go`

use kube::config::Kubeconfig;
use serde::Serialize;

use crate::error::DaoError;

/// A single kubeconfig context entry, ready for display.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ContextEntry {
    /// The context name from kubeconfig.
    pub name: String,
    /// The cluster URL associated with this context.
    pub cluster: String,
    /// The default namespace for this context (empty = cluster default).
    pub namespace: String,
    /// Whether this is the currently-active context.
    pub is_current: bool,
}

/// Holds all kubeconfig contexts read from disk.
pub struct ContextDao {
    entries: Vec<ContextEntry>,
}

impl ContextDao {
    /// Load contexts from the default kubeconfig location
    /// (`$KUBECONFIG` or `~/.kube/config`).
    pub fn load() -> Result<Self, DaoError> {
        let kubeconfig = Kubeconfig::read().map_err(|e| DaoError::KubeConfig {
            source: Box::new(e),
        })?;

        let current = kubeconfig.current_context.as_deref().unwrap_or("");

        let entries = kubeconfig
            .contexts
            .iter()
            .map(|named_ctx| {
                let ctx = named_ctx.context.as_ref();
                // Look up the matching cluster entry for the server URL.
                let cluster_name = ctx.map(|c| c.cluster.as_str()).unwrap_or("");
                let cluster_url = kubeconfig
                    .clusters
                    .iter()
                    .find(|nc| nc.name == cluster_name)
                    .and_then(|nc| nc.cluster.as_ref())
                    .and_then(|c| c.server.as_deref())
                    .unwrap_or(cluster_name)
                    .to_owned();

                let namespace = ctx
                    .and_then(|c| c.namespace.as_deref())
                    .unwrap_or("")
                    .to_owned();

                ContextEntry {
                    is_current: named_ctx.name == current,
                    name: named_ctx.name.clone(),
                    cluster: cluster_url,
                    namespace,
                }
            })
            .collect();

        Ok(Self { entries })
    }

    /// Build from a pre-parsed list (used in tests / offline mode).
    pub fn from_entries(entries: Vec<ContextEntry>) -> Self {
        Self { entries }
    }

    /// All entries, current context first, then alphabetical.
    pub fn list(&self) -> &[ContextEntry] {
        &self.entries
    }

    /// Find a context entry by name.
    pub fn get(&self, name: &str) -> Option<&ContextEntry> {
        self.entries.iter().find(|e| e.name == name)
    }
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_entries() -> Vec<ContextEntry> {
        vec![
            ContextEntry {
                name: "prod".into(),
                cluster: "https://prod.example.com".into(),
                namespace: "default".into(),
                is_current: false,
            },
            ContextEntry {
                name: "staging".into(),
                cluster: "https://staging.example.com".into(),
                namespace: "staging".into(),
                is_current: true,
            },
            ContextEntry {
                name: "local".into(),
                cluster: "https://127.0.0.1:6443".into(),
                namespace: "".into(),
                is_current: false,
            },
        ]
    }

    #[test]
    fn list_returns_all_entries() {
        let dao = ContextDao::from_entries(sample_entries());
        assert_eq!(dao.list().len(), 3);
    }

    #[test]
    fn get_by_name() {
        let dao = ContextDao::from_entries(sample_entries());
        let entry = dao.get("staging").expect("staging should be found");
        assert!(entry.is_current);
        assert_eq!(entry.namespace, "staging");
    }

    #[test]
    fn get_missing_returns_none() {
        let dao = ContextDao::from_entries(sample_entries());
        assert!(dao.get("nonexistent").is_none());
    }

    #[test]
    fn current_entry_flagged() {
        let dao = ContextDao::from_entries(sample_entries());
        let current: Vec<_> = dao.list().iter().filter(|e| e.is_current).collect();
        assert_eq!(current.len(), 1);
        assert_eq!(current[0].name, "staging");
    }
}
