//! Per-cluster / per-context configuration — Phase 9.9 + 9.11.
//!
//! Stores context-specific overrides and feature gate settings that are
//! persisted to `~/.config/k7s/clusters/<context-name>.yaml`.
//!
//! # k9s Reference: `internal/config/data/`

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

// ─── Feature gates ────────────────────────────────────────────────────────────

/// Feature gate switches.
///
/// Each gate can be explicitly enabled/disabled per cluster, overriding the
/// global default.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct FeatureGates {
    /// Allow node-shell (spawns a privileged pod on the node).
    pub node_shell: bool,
    /// Enable image vulnerability scanning.
    pub image_scans: bool,
    /// Allow port-forwarding.
    pub port_forward: bool,
    /// Allow exec / attach into pods.
    pub pod_exec: bool,
    /// Show AI chat window.
    pub ai_chat: bool,
}

impl Default for FeatureGates {
    fn default() -> Self {
        Self {
            node_shell: false,
            image_scans: false,
            port_forward: true,
            pod_exec: true,
            ai_chat: true,
        }
    }
}

// ─── Per-cluster config ───────────────────────────────────────────────────────

/// Configuration overrides specific to a kubeconfig context.
///
/// Stored at `~/.config/k7s/clusters/<context-name>.yaml`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ClusterConfig {
    /// Display name for this cluster (overrides the context name in the header).
    pub display_name: Option<String>,
    /// Default namespace when entering the context.
    pub default_namespace: String,
    /// Feature gates specific to this cluster.
    pub features: FeatureGates,
    /// Per-namespace view preferences: namespace → last-used resource.
    pub namespace_prefs: HashMap<String, NamespacePrefs>,
    /// Whether to enforce read-only mode for this cluster.
    pub read_only: Option<bool>,
    /// Custom skin to use for this cluster (overrides global setting).
    pub skin: Option<String>,
}

impl Default for ClusterConfig {
    fn default() -> Self {
        Self {
            display_name: None,
            default_namespace: "default".to_owned(),
            features: FeatureGates::default(),
            namespace_prefs: HashMap::new(),
            read_only: None,
            skin: None,
        }
    }
}

/// Per-namespace view preferences.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct NamespacePrefs {
    /// Last-used resource alias in this namespace.
    pub active_resource: Option<String>,
    /// Whether to show all namespaces.
    pub all_namespaces: bool,
}

impl ClusterConfig {
    /// Load config for a given context name.
    ///
    /// Returns the default config if no file exists for this context.
    pub fn load(context: &str, config_dir: &Path) -> anyhow::Result<Self> {
        let path = Self::path_for(context, config_dir);
        if !path.exists() {
            return Ok(Self::default());
        }
        let raw = std::fs::read_to_string(&path)?;
        let cfg = serde_yaml::from_str(&raw)?;
        Ok(cfg)
    }

    /// Persist this config to disk.
    pub fn save(&self, context: &str, config_dir: &Path) -> anyhow::Result<()> {
        let path = Self::path_for(context, config_dir);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let yaml = serde_yaml::to_string(self)?;
        std::fs::write(&path, yaml)?;
        Ok(())
    }

    /// Update the active resource for a namespace and persist.
    pub fn set_active_resource(
        &mut self,
        namespace: &str,
        resource: &str,
        context: &str,
        config_dir: &Path,
    ) -> anyhow::Result<()> {
        let prefs = self
            .namespace_prefs
            .entry(namespace.to_owned())
            .or_default();
        prefs.active_resource = Some(resource.to_owned());
        self.save(context, config_dir)
    }

    /// Path to the cluster config file.
    pub fn path_for(context: &str, config_dir: &Path) -> PathBuf {
        // Sanitize context name to be safe as a filename.
        let safe_name: String = context
            .chars()
            .map(|c| {
                if c.is_alphanumeric() || c == '-' || c == '_' || c == '.' {
                    c
                } else {
                    '_'
                }
            })
            .collect();
        config_dir
            .join("clusters")
            .join(format!("{safe_name}.yaml"))
    }
}

// ─── Cluster config registry ──────────────────────────────────────────────────

/// Runtime cache of per-cluster configs, keyed by context name.
pub struct ClusterRegistry {
    configs: HashMap<String, ClusterConfig>,
    config_dir: PathBuf,
}

impl ClusterRegistry {
    pub fn new(config_dir: PathBuf) -> Self {
        Self {
            configs: HashMap::new(),
            config_dir,
        }
    }

    /// Get (or lazily load) config for a context.
    pub fn get(&mut self, context: &str) -> &ClusterConfig {
        if !self.configs.contains_key(context) {
            let cfg = ClusterConfig::load(context, &self.config_dir).unwrap_or_default();
            self.configs.insert(context.to_owned(), cfg);
        }
        self.configs.get(context).unwrap()
    }

    /// Get a mutable reference to the config for a context.
    pub fn get_mut(&mut self, context: &str) -> &mut ClusterConfig {
        if !self.configs.contains_key(context) {
            let cfg = ClusterConfig::load(context, &self.config_dir).unwrap_or_default();
            self.configs.insert(context.to_owned(), cfg);
        }
        self.configs.get_mut(context).unwrap()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_has_sensible_features() {
        let cfg = ClusterConfig::default();
        assert!(cfg.features.port_forward);
        assert!(cfg.features.pod_exec);
        assert!(cfg.features.ai_chat);
        assert!(!cfg.features.node_shell);
        assert!(!cfg.features.image_scans);
    }

    #[test]
    fn load_missing_returns_default() {
        let cfg = ClusterConfig::load("prod-ctx", Path::new("/nonexistent")).unwrap();
        assert_eq!(cfg.default_namespace, "default");
    }

    #[test]
    fn round_trip_persist() {
        let dir = tempfile::tempdir().unwrap();
        let mut cfg = ClusterConfig::default();
        cfg.display_name = Some("My Cluster".to_owned());
        cfg.features.node_shell = true;

        cfg.save("prod", dir.path()).unwrap();
        let loaded = ClusterConfig::load("prod", dir.path()).unwrap();
        assert_eq!(loaded.display_name.as_deref(), Some("My Cluster"));
        assert!(loaded.features.node_shell);
    }

    #[test]
    fn path_sanitizes_context_name() {
        let path = ClusterConfig::path_for("prod/cluster@east", Path::new("/cfg"));
        let filename = path.file_name().unwrap().to_str().unwrap();
        assert!(!filename.contains('/'));
        assert!(!filename.contains('@'));
    }

    #[test]
    fn set_active_resource_persists() {
        let dir = tempfile::tempdir().unwrap();
        let mut cfg = ClusterConfig::default();
        cfg.set_active_resource("kube-system", "pods", "my-ctx", dir.path())
            .unwrap();

        let loaded = ClusterConfig::load("my-ctx", dir.path()).unwrap();
        assert_eq!(
            loaded
                .namespace_prefs
                .get("kube-system")
                .and_then(|p| p.active_resource.as_deref()),
            Some("pods")
        );
    }

    #[test]
    fn registry_lazy_loads() {
        let dir = tempfile::tempdir().unwrap();
        let mut reg = ClusterRegistry::new(dir.path().to_path_buf());
        let cfg = reg.get("new-ctx");
        assert_eq!(cfg.default_namespace, "default");
    }
}
