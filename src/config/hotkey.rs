//! Hotkey configuration — Phase 9.3.
//!
//! Users can bind any action to a custom key sequence via
//! `~/.config/k7s/hotkeys.yaml`.
//!
//! # k9s Reference: `internal/config/hotkey.go`

use std::collections::HashMap;
use std::path::Path;

use serde::{Deserialize, Serialize};

/// A single hotkey binding.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HotkeyBinding {
    /// The action to perform (e.g. `"viewPods"`, `"chat"`, `"quit"`).
    pub action:       String,
    /// Optional description shown in the hints bar.
    pub description:  Option<String>,
}

/// The full hotkey configuration loaded from `hotkeys.yaml`.
///
/// Keys are key names (`"F1"`, `"ctrl-k"`, etc.).
/// Values describe the action to bind.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(transparent)]
pub struct HotkeyConfig {
    pub bindings: HashMap<String, HotkeyBinding>,
}

impl HotkeyConfig {
    /// Load from a `hotkeys.yaml` file.
    ///
    /// Returns an empty config if the file does not exist.
    pub fn load(path: &Path) -> anyhow::Result<Self> {
        if !path.exists() {
            return Ok(Self::default());
        }
        let raw = std::fs::read_to_string(path)?;
        let cfg = serde_yaml::from_str(&raw)?;
        Ok(cfg)
    }

    /// Resolve the action bound to a key string, if any.
    pub fn action_for(&self, key: &str) -> Option<&HotkeyBinding> {
        self.bindings.get(key)
    }

    /// All bound keys, sorted for stable display.
    pub fn sorted_keys(&self) -> Vec<(&str, &HotkeyBinding)> {
        let mut v: Vec<_> = self.bindings.iter().map(|(k, v)| (k.as_str(), v)).collect();
        v.sort_by_key(|(k, _)| *k);
        v
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_config_on_missing_file() {
        let cfg = HotkeyConfig::load(Path::new("/nonexistent/hotkeys.yaml")).unwrap();
        assert!(cfg.bindings.is_empty());
    }

    #[test]
    fn load_valid_hotkeys() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("hotkeys.yaml");
        std::fs::write(&path, r#"
F1:
  action: viewPods
  description: "Open pod list"
ctrl-k:
  action: chat
"#).unwrap();
        let cfg = HotkeyConfig::load(&path).unwrap();
        assert_eq!(cfg.bindings.len(), 2);
        let b = cfg.action_for("F1").unwrap();
        assert_eq!(b.action, "viewPods");
        assert_eq!(b.description.as_deref(), Some("Open pod list"));
    }

    #[test]
    fn sorted_keys_are_alphabetical() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("hotkeys.yaml");
        std::fs::write(&path, r#"
z-key:
  action: last
a-key:
  action: first
"#).unwrap();
        let cfg = HotkeyConfig::load(&path).unwrap();
        let keys: Vec<_> = cfg.sorted_keys().into_iter().map(|(k, _)| k).collect();
        assert_eq!(keys, vec!["a-key", "z-key"]);
    }
}
