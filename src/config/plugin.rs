//! Plugin system — Phase 9.5.
//!
//! Plugins are external commands that can be invoked from the resource browser.
//! They are defined in `~/.config/k7s/plugins.yaml` and appear in the hints
//! bar when the triggering resource type is active.
//!
//! # k9s Reference: `internal/config/plugin.go`
//!
//! # Example plugins.yaml
//!
//! ```yaml
//! k-forward:
//!   shortCut: "Shift-F"
//!   description: "Port-Forward"
//!   scopes:
//!     - pods
//!   command: kubectl
//!   args:
//!     - port-forward
//!     - $NAME
//!     - "8080:8080"
//!     - "-n"
//!     - $NAMESPACE
//!   background: false
//!   confirm: false
//!
//! annotate-restart:
//!   shortCut: "Ctrl-R"
//!   description: "Restart"
//!   scopes:
//!     - deployments
//!   command: kubectl
//!   args:
//!     - rollout
//!     - restart
//!     - deployment/$NAME
//!     - "-n"
//!     - $NAMESPACE
//!   background: true
//!   confirm: true
//! ```
//!
//! ## Template variables
//!
//! | Variable | Expansion |
//! |----------|-----------|
//! | `$NAME` | Selected resource name |
//! | `$NAMESPACE` | Resource namespace |
//! | `$CONTEXT` | Current kubeconfig context |
//! | `$CLUSTER` | Current cluster name |

use std::collections::HashMap;
use std::path::Path;
use std::process::Command;

use crossterm::event::{KeyCode, KeyModifiers};
use serde::{Deserialize, Serialize};

/// A single plugin definition.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Plugin {
    /// Key binding string shown in the hints bar (e.g. `"Shift-F"`).
    pub short_cut: String,
    /// Human-readable description for the hints bar.
    pub description: String,
    /// Resource types (GVR aliases) this plugin applies to.
    /// Use `["all"]` to apply everywhere.
    pub scopes: Vec<String>,
    /// The executable to run.
    pub command: String,
    /// Arguments passed to the command (template variables expanded).
    #[serde(default)]
    pub args: Vec<String>,
    /// When true, run the command in the background (don't suspend TUI).
    #[serde(default)]
    pub background: bool,
    /// When true, prompt for confirmation before running.
    #[serde(default)]
    pub confirm: bool,
}

impl Plugin {
    /// Return true if this plugin's shortcut matches the given crossterm key event.
    pub fn matches_key(&self, event: &crossterm::event::KeyEvent) -> bool {
        if let Some((code, mods)) = parse_shortcut(&self.short_cut) {
            event.code == code && event.modifiers == mods
        } else {
            false
        }
    }

    /// Check if this plugin applies to a given resource type (by alias).
    pub fn applies_to(&self, scope: &str) -> bool {
        self.scopes
            .iter()
            .any(|s| s == "all" || s.eq_ignore_ascii_case(scope))
    }

    /// Expand template variables in the args list.
    ///
    /// Returns the expanded arg list ready to pass to `Command::args()`.
    pub fn expand_args(&self, ctx: &PluginContext) -> Vec<String> {
        self.args.iter().map(|arg| ctx.expand(arg)).collect()
    }

    /// Run the plugin.
    ///
    /// If `background` is true, the process is spawned and detached (TUI
    /// continues running).  If false, the TUI **must** have already disabled
    /// raw mode before calling this.
    pub fn run(&self, ctx: &PluginContext) -> anyhow::Result<PluginResult> {
        let args = self.expand_args(ctx);
        tracing::info!(
            plugin = %ctx.plugin_name,
            command = %self.command,
            ?args,
            background = self.background,
            "running plugin"
        );

        if self.background {
            let _child = Command::new(&self.command)
                .args(&args)
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .spawn()
                .map_err(|e| {
                    anyhow::anyhow!("plugin '{}' failed to spawn: {e}", ctx.plugin_name)
                })?;

            // Detach — we don't wait for it.
            Ok(PluginResult::Background)
        } else {
            let status = Command::new(&self.command)
                .args(&args)
                .status()
                .map_err(|e| anyhow::anyhow!("plugin '{}' failed to run: {e}", ctx.plugin_name))?;

            Ok(PluginResult::Foreground {
                exit_code: status.code(),
            })
        }
    }
}

/// Template expansion context for a plugin invocation.
#[derive(Debug, Clone)]
pub struct PluginContext {
    pub plugin_name: String,
    pub name: String,
    pub namespace: String,
    pub context: String,
    pub cluster: String,
}

impl PluginContext {
    /// Expand template variables in a single string.
    ///
    /// Longer variable names are replaced first to prevent `$NAME` from
    /// consuming the prefix of `$NAMESPACE`.
    pub fn expand(&self, s: &str) -> String {
        s.replace("$NAMESPACE", &self.namespace)
            .replace("$CONTEXT", &self.context)
            .replace("$CLUSTER", &self.cluster)
            .replace("$NAME", &self.name)
    }
}

/// Outcome of running a plugin.
#[derive(Debug)]
pub enum PluginResult {
    /// Plugin ran in foreground (TUI was suspended).
    Foreground { exit_code: Option<i32> },
    /// Plugin was spawned in background.
    Background,
}

/// Full plugin configuration loaded from `plugins.yaml`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(transparent)]
pub struct PluginConfig {
    pub plugins: HashMap<String, Plugin>,
}

impl PluginConfig {
    /// Load from a `plugins.yaml` file.
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

    /// Plugins applicable to a given resource scope, sorted by short_cut.
    pub fn for_scope(&self, scope: &str) -> Vec<(&str, &Plugin)> {
        let mut v: Vec<_> = self
            .plugins
            .iter()
            .filter(|(_, p)| p.applies_to(scope))
            .map(|(name, p)| (name.as_str(), p))
            .collect();
        v.sort_by_key(|(_, p)| p.short_cut.as_str());
        v
    }

    /// Look up a plugin by name.
    pub fn get(&self, name: &str) -> Option<&Plugin> {
        self.plugins.get(name)
    }
}

/// Parse a plugin `shortCut` string into a crossterm `(KeyCode, KeyModifiers)` pair.
///
/// Supported formats:
/// * `"f"` → `(Char('f'), NONE)`
/// * `"F"` / `"Shift-F"` / `"Shift-f"` → `(Char('F'), SHIFT)` — uppercase char implies Shift
/// * `"Ctrl-R"` / `"Ctrl-r"` → `(Char('r'), CONTROL)`
/// * `"Ctrl-Shift-F"` → `(Char('F'), CONTROL | SHIFT)`
/// * `"F5"` → `(F(5), NONE)`
/// * `"Delete"` → `(Delete, NONE)`
/// * `"Enter"` → `(Enter, NONE)`
/// * `"Esc"` → `(Esc, NONE)`
pub fn parse_shortcut(s: &str) -> Option<(KeyCode, KeyModifiers)> {
    let mut mods = KeyModifiers::NONE;
    let mut rest = s.trim();
    // Track whether any explicit modifier prefix was stripped.
    // When true, the case of the final character does NOT imply Shift —
    // e.g. `"Ctrl-R"` means Ctrl+r, not Ctrl+Shift+r.
    let mut explicit_prefix = false;

    // Strip modifier prefixes (case-insensitive).
    loop {
        if let Some(r) = rest.strip_prefix("Ctrl-").or_else(|| rest.strip_prefix("ctrl-")) {
            mods |= KeyModifiers::CONTROL;
            explicit_prefix = true;
            rest = r;
        } else if let Some(r) =
            rest.strip_prefix("Shift-").or_else(|| rest.strip_prefix("shift-"))
        {
            mods |= KeyModifiers::SHIFT;
            explicit_prefix = true;
            rest = r;
        } else if let Some(r) = rest.strip_prefix("Alt-").or_else(|| rest.strip_prefix("alt-")) {
            mods |= KeyModifiers::ALT;
            explicit_prefix = true;
            rest = r;
        } else {
            break;
        }
    }

    // Named keys.
    let code = match rest {
        "Enter" | "enter" => KeyCode::Enter,
        "Esc" | "esc" | "Escape" | "escape" => KeyCode::Esc,
        "Delete" | "delete" | "Del" | "del" => KeyCode::Delete,
        "Backspace" | "backspace" => KeyCode::Backspace,
        "Tab" | "tab" => KeyCode::Tab,
        "Space" | "space" => KeyCode::Char(' '),
        "Up" | "up" => KeyCode::Up,
        "Down" | "down" => KeyCode::Down,
        "Left" | "left" => KeyCode::Left,
        "Right" | "right" => KeyCode::Right,
        "Home" | "home" => KeyCode::Home,
        "End" | "end" => KeyCode::End,
        "PageUp" | "pageup" | "PgUp" | "pgup" => KeyCode::PageUp,
        "PageDown" | "pagedown" | "PgDown" | "pgdown" => KeyCode::PageDown,
        _ => {
            // F-key: "F1" … "F12"
            if let Some(num_str) = rest.strip_prefix('F').or_else(|| rest.strip_prefix('f')) {
                if let Ok(n) = num_str.parse::<u8>() {
                    KeyCode::F(n)
                } else {
                    // Single character — uppercase implies Shift only when no
                    // explicit modifier prefix (e.g. `Ctrl-`) was present.
                    let mut chars = rest.chars();
                    let ch = chars.next()?;
                    if chars.next().is_some() {
                        return None; // More than one char, unrecognised.
                    }
                    if ch.is_uppercase() && !explicit_prefix {
                        mods |= KeyModifiers::SHIFT;
                    }
                    KeyCode::Char(ch.to_lowercase().next().unwrap_or(ch))
                }
            } else {
                let mut chars = rest.chars();
                let ch = chars.next()?;
                if chars.next().is_some() {
                    return None;
                }
                if ch.is_uppercase() && !explicit_prefix {
                    mods |= KeyModifiers::SHIFT;
                }
                KeyCode::Char(ch.to_lowercase().next().unwrap_or(ch))
            }
        }
    };

    Some((code, mods))
}

#[cfg(test)]
mod tests {
    use super::*;

    const YAML: &str = r#"
k-forward:
  shortCut: "Shift-F"
  description: "Port-Forward"
  scopes:
    - pods
  command: kubectl
  args:
    - port-forward
    - "$NAME"
    - "8080:8080"
    - "-n"
    - "$NAMESPACE"
  background: false
  confirm: false
restart:
  shortCut: "Ctrl-R"
  description: "Restart"
  scopes:
    - deployments
    - all
  command: kubectl
  args:
    - rollout
    - restart
    - "deployment/$NAME"
  background: true
  confirm: true
"#;

    fn load_test_config() -> PluginConfig {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("plugins.yaml");
        std::fs::write(&path, YAML).unwrap();
        PluginConfig::load(&path).unwrap()
    }

    #[test]
    fn empty_on_missing_file() {
        let cfg = PluginConfig::load(Path::new("/nonexistent")).unwrap();
        assert!(cfg.plugins.is_empty());
    }

    #[test]
    fn load_plugins() {
        let cfg = load_test_config();
        assert_eq!(cfg.plugins.len(), 2);
        let pf = cfg.get("k-forward").unwrap();
        assert_eq!(pf.short_cut, "Shift-F");
        assert!(!pf.background);
    }

    #[test]
    fn scope_filter_pods() {
        let cfg = load_test_config();
        let scoped = cfg.for_scope("pods");
        // k-forward (pods) + restart (all) should both match.
        assert_eq!(scoped.len(), 2);
    }

    #[test]
    fn scope_filter_deployments() {
        let cfg = load_test_config();
        let scoped = cfg.for_scope("deployments");
        // Only restart applies to deployments + all.
        assert_eq!(scoped.len(), 1);
        assert_eq!(scoped[0].0, "restart");
    }

    #[test]
    fn template_expansion() {
        let ctx = PluginContext {
            plugin_name: "test".into(),
            name: "my-pod".into(),
            namespace: "default".into(),
            context: "prod".into(),
            cluster: "prod-cluster".into(),
        };
        assert_eq!(ctx.expand("$NAME in $NAMESPACE"), "my-pod in default");
        assert_eq!(ctx.expand("ctx=$CONTEXT"), "ctx=prod");
    }

    #[test]
    fn expand_args() {
        let cfg = load_test_config();
        let pf = cfg.get("k-forward").unwrap();
        let ctx = PluginContext {
            plugin_name: "k-forward".into(),
            name: "api-pod".into(),
            namespace: "prod".into(),
            context: "".into(),
            cluster: "".into(),
        };
        let args = pf.expand_args(&ctx);
        assert!(args.contains(&"api-pod".to_owned()));
        assert!(args.contains(&"prod".to_owned()));
    }

    // ── parse_shortcut tests ──────────────────────────────────────────────────

    #[test]
    fn parse_lowercase_char() {
        let (code, mods) = parse_shortcut("f").unwrap();
        assert_eq!(code, KeyCode::Char('f'));
        assert_eq!(mods, KeyModifiers::NONE);
    }

    #[test]
    fn parse_uppercase_char_implies_shift() {
        let (code, mods) = parse_shortcut("F").unwrap();
        assert_eq!(code, KeyCode::Char('f'));
        assert!(mods.contains(KeyModifiers::SHIFT));
    }

    #[test]
    fn parse_shift_prefix() {
        let (code, mods) = parse_shortcut("Shift-F").unwrap();
        assert_eq!(code, KeyCode::Char('f'));
        assert!(mods.contains(KeyModifiers::SHIFT));
    }

    #[test]
    fn parse_ctrl_prefix() {
        let (code, mods) = parse_shortcut("Ctrl-R").unwrap();
        assert_eq!(code, KeyCode::Char('r'));
        assert!(mods.contains(KeyModifiers::CONTROL));
    }

    #[test]
    fn parse_ctrl_shift_combination() {
        let (code, mods) = parse_shortcut("Ctrl-Shift-F").unwrap();
        assert_eq!(code, KeyCode::Char('f'));
        assert!(mods.contains(KeyModifiers::CONTROL));
        assert!(mods.contains(KeyModifiers::SHIFT));
    }

    #[test]
    fn parse_function_key() {
        let (code, mods) = parse_shortcut("F5").unwrap();
        assert_eq!(code, KeyCode::F(5));
        assert_eq!(mods, KeyModifiers::NONE);
    }

    #[test]
    fn parse_named_keys() {
        assert_eq!(parse_shortcut("Enter").unwrap().0, KeyCode::Enter);
        assert_eq!(parse_shortcut("Esc").unwrap().0, KeyCode::Esc);
        assert_eq!(parse_shortcut("Delete").unwrap().0, KeyCode::Delete);
        assert_eq!(parse_shortcut("Tab").unwrap().0, KeyCode::Tab);
    }

    #[test]
    fn parse_invalid_returns_none() {
        assert!(parse_shortcut("").is_none());
        assert!(parse_shortcut("xyz").is_none()); // >1 char, not a named key
    }

    #[test]
    fn matches_key_shift_f() {
        use crossterm::event::{KeyEventKind, KeyEventState};
        let cfg = load_test_config();
        let plugin = cfg.get("k-forward").unwrap();
        let key = crossterm::event::KeyEvent {
            code: KeyCode::Char('f'),
            modifiers: KeyModifiers::SHIFT,
            kind: KeyEventKind::Press,
            state: KeyEventState::NONE,
        };
        assert!(plugin.matches_key(&key));
    }

    #[test]
    fn matches_key_ctrl_r() {
        use crossterm::event::{KeyEventKind, KeyEventState};
        let cfg = load_test_config();
        let plugin = cfg.get("restart").unwrap();
        let key = crossterm::event::KeyEvent {
            code: KeyCode::Char('r'),
            modifiers: KeyModifiers::CONTROL,
            kind: KeyEventKind::Press,
            state: KeyEventState::NONE,
        };
        assert!(plugin.matches_key(&key));
    }

    #[test]
    fn does_not_match_wrong_key() {
        use crossterm::event::{KeyEventKind, KeyEventState};
        let cfg = load_test_config();
        let plugin = cfg.get("k-forward").unwrap();
        let key = crossterm::event::KeyEvent {
            code: KeyCode::Char('x'),
            modifiers: KeyModifiers::NONE,
            kind: KeyEventKind::Press,
            state: KeyEventState::NONE,
        };
        assert!(!plugin.matches_key(&key));
    }
}
