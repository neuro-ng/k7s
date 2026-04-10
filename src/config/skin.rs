//! Skin / theme system — Phase 9.2.
//!
//! A `Skin` defines the colour palette for the entire TUI.  Skins are loaded
//! from `~/.config/k7s/skins/<name>.yaml` or fall back to one of the built-in
//! themes.
//!
//! # k9s Reference: `internal/config/styles.go`, `internal/color/`

use serde::{Deserialize, Serialize};

/// An ANSI terminal colour — stored as a hex string (`"#ff6b6b"`) or a named
/// ANSI colour (`"red"`, `"darkGray"`, etc.).
///
/// The TUI layer converts these to `ratatui::style::Color` at render time.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(untagged)]
pub enum SkinColor {
    /// A hex colour string: `"#ff6b6b"`.
    Hex(String),
    /// A named ratatui colour: `"red"`, `"green"`, `"darkGray"`, etc.
    Named(String),
}

impl SkinColor {
    /// Convert to a `ratatui::style::Color`.
    ///
    /// Unknown names fall back to `Color::Reset`.
    pub fn to_ratatui(&self) -> ratatui::style::Color {
        use ratatui::style::Color;
        let s = match self {
            SkinColor::Hex(h) | SkinColor::Named(h) => h.as_str(),
        };

        // Hex colour.
        if let Some(hex) = s.strip_prefix('#') {
            if hex.len() == 6 {
                if let (Ok(r), Ok(g), Ok(b)) = (
                    u8::from_str_radix(&hex[0..2], 16),
                    u8::from_str_radix(&hex[2..4], 16),
                    u8::from_str_radix(&hex[4..6], 16),
                ) {
                    return Color::Rgb(r, g, b);
                }
            }
        }

        // Named colour.
        match s.to_ascii_lowercase().as_str() {
            "black" => Color::Black,
            "red" => Color::Red,
            "green" => Color::Green,
            "yellow" => Color::Yellow,
            "blue" => Color::Blue,
            "magenta" => Color::Magenta,
            "cyan" => Color::Cyan,
            "gray" | "grey" | "white" => Color::Gray,
            "darkgray" | "darkgrey" => Color::DarkGray,
            "lightred" => Color::LightRed,
            "lightgreen" => Color::LightGreen,
            "lightyellow" => Color::LightYellow,
            "lightblue" => Color::LightBlue,
            "lightmagenta" => Color::LightMagenta,
            "lightcyan" => Color::LightCyan,
            "reset" => Color::Reset,
            _ => Color::Reset,
        }
    }
}

impl Default for SkinColor {
    fn default() -> Self {
        SkinColor::Named("reset".to_owned())
    }
}

/// Colour group for a TUI section.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ColorGroup {
    pub fg: SkinColor,
    pub bg: SkinColor,
}

/// Full skin definition.
///
/// Every section of the TUI has a named colour group.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Skin {
    /// Human-readable skin name.
    pub name: String,

    // ── Header / border ──────────────────────────────────────────────
    pub header: ColorGroup,
    pub border: ColorGroup,
    pub title: ColorGroup,

    // ── Table ────────────────────────────────────────────────────────
    pub table_header: ColorGroup,
    pub table_row: ColorGroup,
    pub table_sel: ColorGroup,   // selected row
    pub table_added: ColorGroup, // newly appeared row
    pub table_mod: ColorGroup,   // modified row
    pub table_del: ColorGroup,   // deleted (fading) row

    // ── Status / flash bar ───────────────────────────────────────────
    pub info: ColorGroup,
    pub warn: ColorGroup,
    pub error: ColorGroup,

    // ── Log view ─────────────────────────────────────────────────────
    pub log_info: ColorGroup,
    pub log_warn: ColorGroup,
    pub log_error: ColorGroup,
    pub log_debug: ColorGroup,

    // ── Prompt ───────────────────────────────────────────────────────
    pub prompt: ColorGroup,
    pub prompt_label: ColorGroup,

    // ── AI chat ──────────────────────────────────────────────────────
    pub chat_user: ColorGroup,
    pub chat_bot: ColorGroup,
    pub chat_system: ColorGroup,
}

impl Default for Skin {
    fn default() -> Self {
        Self::default_dark()
    }
}

impl Skin {
    /// The built-in dark theme (used when no skin is configured).
    pub fn default_dark() -> Self {
        Self {
            name: "dark".to_owned(),
            header: cg("#61afef", "#282c34"),
            border: cg("#4b5263", "reset"),
            title: cg("#e5c07b", "reset"),
            table_header: cg("#abb2bf", "#3e4452"),
            table_row: cg("#abb2bf", "reset"),
            table_sel: cg("#282c34", "#61afef"),
            table_added: cg("#98c379", "reset"),
            table_mod: cg("#e5c07b", "reset"),
            table_del: cg("#4b5263", "reset"),
            info: cg("#98c379", "reset"),
            warn: cg("#e5c07b", "reset"),
            error: cg("#e06c75", "reset"),
            log_info: cg("#abb2bf", "reset"),
            log_warn: cg("#e5c07b", "reset"),
            log_error: cg("#e06c75", "reset"),
            log_debug: cg("#5c6370", "reset"),
            prompt: cg("#abb2bf", "#3e4452"),
            prompt_label: cg("#61afef", "#3e4452"),
            chat_user: cg("#98c379", "reset"),
            chat_bot: cg("#61afef", "reset"),
            chat_system: cg("#5c6370", "reset"),
        }
    }

    /// Dracula theme.
    pub fn dracula() -> Self {
        Self {
            name: "dracula".to_owned(),
            header: cg("#bd93f9", "#282a36"),
            border: cg("#6272a4", "reset"),
            title: cg("#f1fa8c", "reset"),
            table_header: cg("#f8f8f2", "#44475a"),
            table_row: cg("#f8f8f2", "reset"),
            table_sel: cg("#282a36", "#bd93f9"),
            table_added: cg("#50fa7b", "reset"),
            table_mod: cg("#ffb86c", "reset"),
            table_del: cg("#6272a4", "reset"),
            info: cg("#50fa7b", "reset"),
            warn: cg("#ffb86c", "reset"),
            error: cg("#ff5555", "reset"),
            log_info: cg("#f8f8f2", "reset"),
            log_warn: cg("#ffb86c", "reset"),
            log_error: cg("#ff5555", "reset"),
            log_debug: cg("#6272a4", "reset"),
            prompt: cg("#f8f8f2", "#44475a"),
            prompt_label: cg("#bd93f9", "#44475a"),
            chat_user: cg("#50fa7b", "reset"),
            chat_bot: cg("#8be9fd", "reset"),
            chat_system: cg("#6272a4", "reset"),
        }
    }

    /// Monokai theme.
    pub fn monokai() -> Self {
        Self {
            name: "monokai".to_owned(),
            header: cg("#a6e22e", "#272822"),
            border: cg("#75715e", "reset"),
            title: cg("#e6db74", "reset"),
            table_header: cg("#f8f8f2", "#3e3d32"),
            table_row: cg("#f8f8f2", "reset"),
            table_sel: cg("#272822", "#a6e22e"),
            table_added: cg("#a6e22e", "reset"),
            table_mod: cg("#fd971f", "reset"),
            table_del: cg("#75715e", "reset"),
            info: cg("#a6e22e", "reset"),
            warn: cg("#fd971f", "reset"),
            error: cg("#f92672", "reset"),
            log_info: cg("#f8f8f2", "reset"),
            log_warn: cg("#fd971f", "reset"),
            log_error: cg("#f92672", "reset"),
            log_debug: cg("#75715e", "reset"),
            prompt: cg("#f8f8f2", "#3e3d32"),
            prompt_label: cg("#a6e22e", "#3e3d32"),
            chat_user: cg("#a6e22e", "reset"),
            chat_bot: cg("#66d9e8", "reset"),
            chat_system: cg("#75715e", "reset"),
        }
    }

    /// Load a skin by name from the built-in registry or a file.
    ///
    /// Built-in names: `"dark"` (default), `"dracula"`, `"monokai"`.
    /// File: `skins/<name>.yaml` relative to the config directory.
    pub fn load(name: &str, config_dir: &std::path::Path) -> Self {
        // Try built-ins first.
        match name {
            "dark" | "" => return Self::default_dark(),
            "dracula" => return Self::dracula(),
            "monokai" => return Self::monokai(),
            _ => {}
        }

        // Try loading from file.
        let path = config_dir.join("skins").join(format!("{name}.yaml"));
        if path.exists() {
            if let Ok(raw) = std::fs::read_to_string(&path) {
                if let Ok(skin) = serde_yaml::from_str::<Skin>(&raw) {
                    return skin;
                }
                tracing::warn!(path = %path.display(), "failed to parse skin file");
            }
        }

        tracing::debug!(name, "unknown skin, falling back to dark");
        Self::default_dark()
    }
}

/// Shorthand constructor for a `ColorGroup`.
fn cg(fg: &str, bg: &str) -> ColorGroup {
    ColorGroup {
        fg: SkinColor::Hex(fg.to_owned()),
        bg: if bg == "reset" {
            SkinColor::Named("reset".to_owned())
        } else {
            SkinColor::Hex(bg.to_owned())
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::style::Color;

    #[test]
    fn hex_to_ratatui() {
        let c = SkinColor::Hex("#ff0000".to_owned());
        assert_eq!(c.to_ratatui(), Color::Rgb(255, 0, 0));
    }

    #[test]
    fn named_to_ratatui() {
        assert_eq!(
            SkinColor::Named("cyan".to_owned()).to_ratatui(),
            Color::Cyan
        );
        assert_eq!(SkinColor::Named("red".to_owned()).to_ratatui(), Color::Red);
    }

    #[test]
    fn unknown_named_falls_back_to_reset() {
        assert_eq!(
            SkinColor::Named("sparkle".to_owned()).to_ratatui(),
            Color::Reset
        );
    }

    #[test]
    fn default_dark_is_dark() {
        let s = Skin::default_dark();
        assert_eq!(s.name, "dark");
    }

    #[test]
    fn dracula_skin() {
        let s = Skin::dracula();
        assert_eq!(s.name, "dracula");
    }

    #[test]
    fn load_builtin_skin() {
        let skin = Skin::load("dracula", std::path::Path::new("/nonexistent"));
        assert_eq!(skin.name, "dracula");
    }

    #[test]
    fn load_unknown_falls_back_to_dark() {
        let skin = Skin::load("neon-rainbow", std::path::Path::new("/nonexistent"));
        assert_eq!(skin.name, "dark");
    }

    #[test]
    fn load_from_file() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("skins")).unwrap();
        let content = "name: custom\nheader:\n  fg: \"#aabbcc\"\n  bg: reset\n";
        std::fs::write(dir.path().join("skins/custom.yaml"), content).unwrap();
        let skin = Skin::load("custom", dir.path());
        assert_eq!(skin.name, "custom");
    }
}
