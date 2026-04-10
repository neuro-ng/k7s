//! Alias configuration — Phase 9.4.
//!
//! Users can define short aliases for any resource type or command in
//! `~/.config/k7s/aliases.yaml`.  These are merged with the built-in
//! aliases from the DAO registry at startup.
//!
//! # k9s Reference: `internal/config/alias.go`

use std::collections::HashMap;
use std::path::Path;

use serde::{Deserialize, Serialize};

/// User-defined alias table.
///
/// Key: alias string (e.g. `"wl"`, `"mypod"`).
/// Value: the target resource or command (e.g. `"deployments"`, `":chat"`).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(transparent)]
pub struct AliasConfig {
    pub aliases: HashMap<String, String>,
}

impl AliasConfig {
    /// Load from an `aliases.yaml` file.
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

    /// Resolve a user alias to its target string.
    ///
    /// Lookup is case-insensitive.
    pub fn resolve(&self, alias: &str) -> Option<&str> {
        self.aliases.get(&alias.to_lowercase()).map(String::as_str)
    }

    /// Register a new alias (for runtime additions).
    pub fn insert(&mut self, alias: impl Into<String>, target: impl Into<String>) {
        self.aliases
            .insert(alias.into().to_lowercase(), target.into());
    }

    /// All aliases sorted for stable display.
    pub fn sorted(&self) -> Vec<(&str, &str)> {
        let mut v: Vec<_> = self
            .aliases
            .iter()
            .map(|(k, v)| (k.as_str(), v.as_str()))
            .collect();
        v.sort_by_key(|(k, _)| *k);
        v
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_on_missing_file() {
        let cfg = AliasConfig::load(Path::new("/nonexistent")).unwrap();
        assert!(cfg.aliases.is_empty());
    }

    #[test]
    fn load_valid_aliases() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("aliases.yaml");
        std::fs::write(&path, "wl: deployments\nkp: pods\n").unwrap();
        let cfg = AliasConfig::load(&path).unwrap();
        assert_eq!(cfg.resolve("wl"), Some("deployments"));
        assert_eq!(cfg.resolve("kp"), Some("pods"));
    }

    #[test]
    fn resolve_is_case_insensitive() {
        let mut cfg = AliasConfig::default();
        cfg.insert("WL", "deployments");
        assert_eq!(cfg.resolve("wl"), Some("deployments"));
        assert_eq!(cfg.resolve("WL"), Some("deployments"));
    }

    #[test]
    fn unknown_alias_returns_none() {
        let cfg = AliasConfig::default();
        assert_eq!(cfg.resolve("xyz"), None);
    }

    #[test]
    fn sorted_is_alphabetical() {
        let mut cfg = AliasConfig::default();
        cfg.insert("z", "last");
        cfg.insert("a", "first");
        let keys: Vec<_> = cfg.sorted().iter().map(|(k, _)| *k).collect();
        assert_eq!(keys, vec!["a", "z"]);
    }
}
