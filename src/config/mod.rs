use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::ConfigError;

pub mod alias;
pub mod cluster;
pub mod hotkey;
pub mod plugin;
pub mod skin;
pub mod views;
pub mod watcher;

pub use alias::AliasConfig;
pub use cluster::{ClusterConfig, ClusterRegistry, FeatureGates};
pub use hotkey::HotkeyConfig;
pub use plugin::{Plugin, PluginConfig, PluginContext};
pub use skin::Skin;
pub use views::{CustomColumnDef, CustomColumnRenderer, ResourceViewConfig, ViewsConfig};

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
    pub benchmark: BenchmarkConfig,
}

impl Default for K7sConfig {
    fn default() -> Self {
        Self {
            refresh_rate: 2,
            read_only: false,
            ui: UiConfig::default(),
            logger: LoggerConfig::default(),
            ai: AiConfig::default(),
            benchmark: BenchmarkConfig::default(),
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

/// HTTP benchmark configuration — Phase 9.8.
///
/// Controls how the built-in benchmark runner sends load against a target URL.
/// Corresponds to `benchmarkConfig` in `config.yaml`.
///
/// Example:
/// ```yaml
/// k7s:
///   benchmark:
///     concurrency: 50
///     totalRequests: 1000
///     timeoutMs: 5000
///     http2: false
///     method: GET
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct BenchmarkConfig {
    /// Number of concurrent workers sending requests.
    pub concurrency: u32,
    /// Total number of requests to send before stopping.
    ///
    /// Set to `0` to run for `duration_secs` instead.
    pub total_requests: u32,
    /// Run for this many seconds (used when `total_requests == 0`).
    pub duration_secs: u32,
    /// Per-request timeout in milliseconds.
    pub timeout_ms: u64,
    /// Force HTTP/2.
    pub http2: bool,
    /// HTTP method (GET, POST, PUT, etc.).
    pub method: String,
    /// Optional request body for POST/PUT.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub body: Option<String>,
    /// Additional request headers as `"Name: Value"` strings.
    pub headers: Vec<String>,
}

impl Default for BenchmarkConfig {
    fn default() -> Self {
        Self {
            concurrency: 10,
            total_requests: 200,
            duration_secs: 0,
            timeout_ms: 5_000,
            http2: false,
            method: "GET".to_owned(),
            body: None,
            headers: Vec::new(),
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
