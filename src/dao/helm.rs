//! Helm chart DAO — Phase 10.5.
//!
//! Provides read and lifecycle operations for Helm releases by shelling out
//! to the `helm` CLI.  This avoids depending on the Helm SDK and works with
//! any Helm version that supports `--output json`.
//!
//! # k9s Reference: `internal/dao/helm_chart.go`, `internal/dao/helm_history.go`

use serde::{Deserialize, Serialize};
use std::process::Command;
use thiserror::Error;

// ─── Error ────────────────────────────────────────────────────────────────────

#[derive(Debug, Error)]
pub enum HelmError {
    #[error("helm CLI not found — install helm and ensure it is on $PATH")]
    NotFound,
    #[error("helm command failed: {0}")]
    CommandFailed(String),
    #[error("failed to parse helm output: {0}")]
    Parse(#[from] serde_json::Error),
    #[error("I/O error running helm: {0}")]
    Io(#[from] std::io::Error),
}

// ─── Data structures ──────────────────────────────────────────────────────────

/// A deployed Helm release.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HelmRelease {
    pub name:        String,
    pub namespace:   String,
    pub revision:    String,
    pub updated:     String,
    pub status:      String,
    pub chart:       String,
    pub app_version: String,
}

/// A single entry in `helm history`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HelmHistoryEntry {
    pub revision:    u64,
    pub updated:     String,
    pub status:      String,
    pub chart:       String,
    pub app_version: String,
    pub description: String,
}

// ─── DAO ─────────────────────────────────────────────────────────────────────

/// DAO for Helm release operations.
pub struct HelmDao {
    /// `--kube-context` value to pass to helm; `None` uses the active context.
    pub context: Option<String>,
}

impl HelmDao {
    pub fn new(context: Option<String>) -> Self {
        Self { context }
    }

    // ── Queries ───────────────────────────────────────────────────────────────

    /// List all releases in `namespace`, or across all namespaces when `None`.
    pub fn list(&self, namespace: Option<&str>) -> Result<Vec<HelmRelease>, HelmError> {
        let mut cmd = self.base_cmd();
        cmd.args(["list", "--output", "json"]);
        if let Some(ns) = namespace {
            cmd.args(["-n", ns]);
        } else {
            cmd.arg("--all-namespaces");
        }

        let output = run_cmd(cmd)?;
        let releases: Vec<HelmRelease> = serde_json::from_str(&output)?;
        Ok(releases)
    }

    /// Return the revision history of a release.
    pub fn history(&self, name: &str, namespace: &str) -> Result<Vec<HelmHistoryEntry>, HelmError> {
        let mut cmd = self.base_cmd();
        cmd.args(["history", name, "--output", "json", "-n", namespace]);

        let output = run_cmd(cmd)?;
        let entries: Vec<HelmHistoryEntry> = serde_json::from_str(&output)?;
        Ok(entries)
    }

    // ── Mutations ─────────────────────────────────────────────────────────────

    /// Uninstall (delete) a release.
    pub fn delete(&self, name: &str, namespace: &str) -> Result<(), HelmError> {
        let mut cmd = self.base_cmd();
        cmd.args(["uninstall", name, "-n", namespace]);
        run_cmd(cmd)?;
        Ok(())
    }

    /// Roll back a release to `revision` (0 means previous revision).
    pub fn rollback(&self, name: &str, namespace: &str, revision: u64) -> Result<(), HelmError> {
        let rev = if revision == 0 {
            String::new()
        } else {
            revision.to_string()
        };
        let mut cmd = self.base_cmd();
        cmd.args(["rollback", name]);
        if !rev.is_empty() {
            cmd.arg(&rev);
        }
        cmd.args(["-n", namespace]);
        run_cmd(cmd)?;
        Ok(())
    }

    // ── Helpers ───────────────────────────────────────────────────────────────

    fn base_cmd(&self) -> Command {
        let mut cmd = Command::new("helm");
        if let Some(ctx) = &self.context {
            cmd.args(["--kube-context", ctx]);
        }
        cmd
    }
}

/// Run a `Command`, capture stdout, return `Err` on non-zero exit.
fn run_cmd(mut cmd: Command) -> Result<String, HelmError> {
    let output = cmd.output().map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            HelmError::NotFound
        } else {
            HelmError::Io(e)
        }
    })?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(HelmError::CommandFailed(stderr));
    }

    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn helm_release_deserializes() {
        let json = r#"[{
            "name": "my-app",
            "namespace": "production",
            "revision": "3",
            "updated": "2026-04-10 12:00:00.000000000 +0000 UTC",
            "status": "deployed",
            "chart": "my-app-1.2.3",
            "app_version": "1.2.3"
        }]"#;

        let releases: Vec<HelmRelease> = serde_json::from_str(json).unwrap();
        assert_eq!(releases.len(), 1);
        assert_eq!(releases[0].name, "my-app");
        assert_eq!(releases[0].status, "deployed");
    }

    #[test]
    fn helm_history_entry_deserializes() {
        let json = r#"[{
            "revision": 2,
            "updated": "2026-04-09 10:00:00.000000000 +0000 UTC",
            "status": "superseded",
            "chart": "my-app-1.2.2",
            "app_version": "1.2.2",
            "description": "Upgrade complete"
        }]"#;

        let entries: Vec<HelmHistoryEntry> = serde_json::from_str(json).unwrap();
        assert_eq!(entries[0].revision, 2);
        assert_eq!(entries[0].status, "superseded");
    }

    #[test]
    fn helm_not_found_when_missing() {
        // Use a definitely-absent binary name to trigger NotFound.
        let mut cmd = Command::new("helm_definitely_not_installed_k7s_test");
        let mut cmd = cmd;
        cmd.arg("version");
        let result = run_cmd(cmd);
        assert!(matches!(result, Err(HelmError::NotFound)));
    }
}
