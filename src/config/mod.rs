use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::ConfigError;

pub mod alias;
pub mod cluster;
pub mod hotkey;
pub mod plugin;
pub mod skin;

pub use alias::AliasConfig;
pub use cluster::{ClusterConfig, ClusterRegistry, FeatureGates};
pub use hotkey::HotkeyConfig;
pub use plugin::{Plugin, PluginConfig, PluginContext};
pub use skin::Skin;

/// XDG-compliant directory paths for k7s runtime files.
///
/// All paths follow the XDG Base Directory Specification.
/// Falls back to `~/.config/k7s` / `~/.local/share/k7s` / `~/.local/state/k7s`
/// when XDG env vars are not set.
#[derive(Debug, Clone)]
pub struct ConfigDirs {
    /// Primary config directory: `$XDG_CONFIG_HOME/k7s`
    pub config: PathBuf,
    /// Data directory: `$XDG_DATA_HOME/k7s`
    pub data: PathBuf,
    /// Logs / state directory: `$XDG_STATE_HOME/k7s`
    pub state: PathBuf,
}

impl ConfigDirs {
    /// Resolve XDG directories, creating them if absent.
    ///
    /// Returns `ConfigError::NoConfigDir` if the home directory cannot be determined.
    pub fn resolve() -> Result<Self, ConfigError> {
        let config = dirs::config_dir()
            .ok_or(ConfigError::NoConfigDir)?
            .join("k7s");

        let data = dirs::data_dir()
            .ok_or(ConfigError::NoConfigDir)?
            .join("k7s");

        let state = dirs::state_dir()
            .unwrap_or_else(|| {
                dirs::home_dir()
                    .unwrap_or_else(|| PathBuf::from("."))
                    .join(".local")
                    .join("state")
            })
            .join("k7s");

        Ok(Self {
            config,
            data,
            state,
        })
    }

    /// Path to the main config file: `$config_dir/config.yaml`
    pub fn config_file(&self) -> PathBuf {
        self.config.join("config.yaml")
    }

    /// Path to the audit log for sanitizer decisions.
    pub fn sanitizer_audit_log(&self) -> PathBuf {
        self.state.join("sanitizer-audit.log")
    }
}

/// Top-level application configuration, deserialized from `config.yaml`.
///
/// Follows the structure documented in CLAUDE.md.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Config {
    pub k7s: K7sConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct K7sConfig {
    /// Cluster resource refresh rate in seconds.
    pub refresh_rate: u32,
    /// When true, no mutating operations (delete, scale, edit) are permitted.
    pub read_only: bool,
    pub ui: UiConfig,
    pub logger: LoggerConfig,
    pub ai: AiConfig,
}

impl Default for K7sConfig {
    fn default() -> Self {
        Self {
            refresh_rate: 2,
            read_only: false,
            ui: UiConfig::default(),
            logger: LoggerConfig::default(),
            ai: AiConfig::default(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct UiConfig {
    pub skin: String,
    pub enable_mouse: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct LoggerConfig {
    /// Number of log lines to tail.
    pub tail: usize,
    /// Log buffer size.
    pub buffer: usize,
}

impl Default for LoggerConfig {
    fn default() -> Self {
        Self {
            tail: 200,
            buffer: 500,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct AiConfig {
    /// Provider: "antigravity" or "api"
    pub provider: String,
    /// API key — prefer the K7S_LLM_API_KEY env var over storing here.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub api_key: Option<String>,
    /// LLM API endpoint (for "api" provider).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub endpoint: Option<String>,
    pub token_budget: TokenBudgetConfig,
    pub sanitizer: SanitizerConfig,
}

impl Default for AiConfig {
    fn default() -> Self {
        Self {
            provider: "api".to_string(),
            api_key: None,
            endpoint: None,
            token_budget: TokenBudgetConfig::default(),
            sanitizer: SanitizerConfig::default(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct TokenBudgetConfig {
    pub max_per_session: u32,
    pub max_per_query: u32,
    pub warn_at: u32,
}

impl Default for TokenBudgetConfig {
    fn default() -> Self {
        Self {
            max_per_session: 100_000,
            max_per_query: 4_000,
            warn_at: 80_000,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct SanitizerConfig {
    /// Default-deny mode. Strongly recommended.
    pub strict_mode: bool,
    /// Write sanitization decisions to the audit log.
    pub audit_log: bool,
    /// Additional regex patterns to redact (beyond built-in rules).
    pub custom_patterns: Vec<String>,
}

impl Default for SanitizerConfig {
    fn default() -> Self {
        Self {
            strict_mode: true,
            audit_log: true,
            custom_patterns: Vec::new(),
        }
    }
}

/// Load configuration from disk.
///
/// Returns the default `Config` if no config file exists.
/// Returns an error if the file exists but cannot be parsed.
pub fn load(path: &Path) -> Result<Config, ConfigError> {
    if !path.exists() {
        tracing::debug!(path = %path.display(), "config file not found, using defaults");
        return Ok(Config::default());
    }

    let raw = std::fs::read_to_string(path).map_err(|source| ConfigError::Read {
        path: path.to_owned(),
        source,
    })?;

    serde_yaml::from_str(&raw).map_err(|source| ConfigError::Parse {
        path: path.to_owned(),
        source,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_is_strict_sanitizer() {
        let cfg = Config::default();
        assert!(cfg.k7s.ai.sanitizer.strict_mode);
        assert!(cfg.k7s.ai.sanitizer.audit_log);
    }

    #[test]
    fn default_config_is_not_readonly() {
        let cfg = Config::default();
        assert!(!cfg.k7s.read_only);
    }

    #[test]
    fn load_missing_file_returns_default() {
        let result = load(Path::new("/nonexistent/path/config.yaml"));
        assert!(result.is_ok());
    }

    #[test]
    fn load_valid_yaml() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.yaml");
        std::fs::write(
            &path,
            r#"
k7s:
  refreshRate: 5
  readOnly: true
"#,
        )
        .unwrap();
        let cfg = load(&path).unwrap();
        assert_eq!(cfg.k7s.refresh_rate, 5);
        assert!(cfg.k7s.read_only);
    }
}
