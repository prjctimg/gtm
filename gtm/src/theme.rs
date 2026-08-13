// Copyright (c) 2025 - present
// Author: prjctimg <prjctimg@outlook.com>
// NvChad-inspired color themes for the TUI, with TOML user-theme support.
//
// This is free software released under the GPL-3.0 license.

use ratatui::style::Color;
use std::borrow::Cow;

/// Central theme — all UI colors flow through here.
/// The TUI renders its own explicit `bg` behind everything.
#[derive(Clone, Copy)]
pub struct AppTheme {
    pub bg: Color,
    /// Main pane body fill (library/queue/lyrics content areas). Defaults to
    /// `bg`; user themes may set it independently to control the fill area.
    pub pane_bg: Color,
    pub picker_bg: Color,
    /// Floating-panel fill, one step darker/lighter than `picker_bg`.
    pub elevated_bg: Color,
    /// Quiet separator used for pane-edge rules and flat panel borders.
    pub muted_border: Color,
    pub fg: Color,
    pub fg_dim: Color,
    pub fg_bright: Color,
    pub accent: Color,
    pub error: Color,
    pub warning: Color,
    pub success: Color,
    pub selection_fg: Color,
    pub selection_bg: Color,
    pub border: Color,
    pub border_active: Color,
    pub volume_low: Color,
    pub volume_medium: Color,
    pub volume_high: Color,
    pub sidebar_active_border: Color,
}

impl AppTheme {
    pub fn volume_color(&self, volume: u8) -> Color {
        if volume > 85 {
            self.volume_high
        } else if volume > 50 {
            self.volume_medium
        } else {
            self.volume_low
        }
    }
}

/// A theme registry entry. `name` is `Cow<'static, str>` so built-ins borrow
/// `&'static str` literals while user-loaded themes own their names.
#[derive(Clone)]
pub struct ThemeEntry {
    pub name: Cow<'static, str>,
    pub light: bool,
    pub theme: AppTheme,
}

// ─── Color helpers ────────────────────────────────────────────────────

fn hex(c: u32) -> Color {
    Color::Rgb(
        ((c >> 16) & 0xFF) as u8,
        ((c >> 8) & 0xFF) as u8,
        (c & 0xFF) as u8,
    )
}

/// Parse a `#rrggbb` (or bare `rrggbb`) hex string into a `Color::Rgb`.
pub fn parse_color(s: &str) -> Result<Color, String> {
    let s = s.strip_prefix('#').unwrap_or(s);
    if s.len() != 6 {
        return Err(format!("expected 6 hex chars, got {s:?}"));
    }
    let r = u8::from_str_radix(&s[0..2], 16).map_err(|e| e.to_string())?;
    let g = u8::from_str_radix(&s[2..4], 16).map_err(|e| e.to_string())?;
    let b = u8::from_str_radix(&s[4..6], 16).map_err(|e| e.to_string())?;
    Ok(Color::Rgb(r, g, b))
}

// ─── Built-in presets ─────────────────────────────────────────────────

/// Chadrula — NvChad default
fn chadrula() -> AppTheme {
    AppTheme {
        bg: hex(0x24283b),
        pane_bg: hex(0x24283b),
        picker_bg: hex(0x1f2335),
        elevated_bg: hex(0x1a1e2e),
        muted_border: hex(0x3b4261),
        fg: hex(0xc0caf5),
        fg_dim: hex(0x565f89),
        fg_bright: hex(0xe0e6ff),
        accent: hex(0x7aa2f7),
        error: hex(0xf7768e),
        warning: hex(0xe0af68),
        success: hex(0x9ece6a),
        selection_fg: hex(0x24283b),
        selection_bg: hex(0x7aa2f7),
        border: hex(0x3b4261),
        border_active: hex(0x7aa2f7),
        volume_low: hex(0x9ece6a),
        volume_medium: hex(0xe0af68),
        volume_high: hex(0xf7768e),
        sidebar_active_border: hex(0x7aa2f7),
    }
}

/// One Dark — NvChad palette
fn one_dark() -> AppTheme {
    AppTheme {
        bg: hex(0x282c34),
        pane_bg: hex(0x282c34),
        picker_bg: hex(0x21252b),
        elevated_bg: hex(0x1c2026),
        muted_border: hex(0x3e4451),
        fg: hex(0xabb2bf),
        fg_dim: hex(0x5c6370),
        fg_bright: hex(0xe6e6e6),
        accent: hex(0x61afef),
        error: hex(0xe06c75),
        warning: hex(0xe5c07b),
        success: hex(0x98c379),
        selection_fg: hex(0x282c34),
        selection_bg: hex(0x61afef),
        border: hex(0x3e4451),
        border_active: hex(0x61afef),
        volume_low: hex(0x98c379),
        volume_medium: hex(0xe5c07b),
        volume_high: hex(0xe06c75),
        sidebar_active_border: hex(0x61afef),
    }
}

/// Tokyo Night — NvChad palette
fn tokyonight() -> AppTheme {
    AppTheme {
        bg: hex(0x1a1b26),
        pane_bg: hex(0x1a1b26),
        picker_bg: hex(0x16161e),
        elevated_bg: hex(0x12121a),
        muted_border: hex(0x292e42),
        fg: hex(0xa9b1d6),
        fg_dim: hex(0x565f89),
        fg_bright: hex(0xc0caf5),
        accent: hex(0x7aa2f7),
        error: hex(0xf7768e),
        warning: hex(0xff9e64),
        success: hex(0x9ece6a),
        selection_fg: hex(0x1a1b26),
        selection_bg: hex(0x7aa2f7),
        border: hex(0x292e42),
        border_active: hex(0x7aa2f7),
        volume_low: hex(0x9ece6a),
        volume_medium: hex(0xff9e64),
        volume_high: hex(0xf7768e),
        sidebar_active_border: hex(0x7aa2f7),
    }
}

/// Tokyo Night Storm — derived from Chadrula with a dimmer fg and a warmer
/// warning/volume-medium pair.
fn tokyonight_storm() -> AppTheme {
    let mut t = chadrula();
    t.fg = hex(0xa9b1d6);
    t.warning = hex(0xff9e64);
    t.volume_medium = hex(0xff9e64);
    t
}

/// Catppuccin Mocha — NvChad palette
fn catppuccin_mocha() -> AppTheme {
    AppTheme {
        bg: hex(0x1e1e2e),
        pane_bg: hex(0x1e1e2e),
        picker_bg: hex(0x181825),
        elevated_bg: hex(0x141422),
        muted_border: hex(0x313244),
        fg: hex(0xcdd6f4),
        fg_dim: hex(0x6c7086),
        fg_bright: hex(0xf5f5ff),
        accent: hex(0x89b4fa),
        error: hex(0xf38ba8),
        warning: hex(0xfab387),
        success: hex(0xa6e3a1),
        selection_fg: hex(0x1e1e2e),
        selection_bg: hex(0x89b4fa),
        border: hex(0x313244),
        border_active: hex(0x89b4fa),
        volume_low: hex(0xa6e3a1),
        volume_medium: hex(0xfab387),
        volume_high: hex(0xf38ba8),
        sidebar_active_border: hex(0x89b4fa),
    }
}

/// Gruvbox Dark — NvChad palette
fn gruvbox_dark() -> AppTheme {
    AppTheme {
        bg: hex(0x282828),
        pane_bg: hex(0x282828),
        picker_bg: hex(0x1d2021),
        elevated_bg: hex(0x181b1c),
        muted_border: hex(0x504945),
        fg: hex(0xebdbb2),
        fg_dim: hex(0x928374),
        fg_bright: hex(0xfbf1c7),
        accent: hex(0xd3869b),
        error: hex(0xfb4934),
        warning: hex(0xfe8019),
        success: hex(0xb8bb26),
        selection_fg: hex(0x282828),
        selection_bg: hex(0xd3869b),
        border: hex(0x504945),
        border_active: hex(0xd3869b),
        volume_low: hex(0xb8bb26),
        volume_medium: hex(0xfe8019),
        volume_high: hex(0xfb4934),
        sidebar_active_border: hex(0xd3869b),
    }
}

/// Nord — NvChad palette
fn nord() -> AppTheme {
    AppTheme {
        bg: hex(0x2e3440),
        pane_bg: hex(0x2e3440),
        picker_bg: hex(0x2b303b),
        elevated_bg: hex(0x262b35),
        muted_border: hex(0x3b4252),
        fg: hex(0xd8dee9),
        fg_dim: hex(0x4c566a),
        fg_bright: hex(0xeceff4),
        accent: hex(0x88c0d0),
        error: hex(0xbf616a),
        warning: hex(0xd08770),
        success: hex(0xa3be8c),
        selection_fg: hex(0x2e3440),
        selection_bg: hex(0x88c0d0),
        border: hex(0x3b4252),
        border_active: hex(0x88c0d0),
        volume_low: hex(0xa3be8c),
        volume_medium: hex(0xd08770),
        volume_high: hex(0xbf616a),
        sidebar_active_border: hex(0x88c0d0),
    }
}

/// Rose Pine — NvChad palette
fn rose_pine() -> AppTheme {
    AppTheme {
        bg: hex(0x191724),
        pane_bg: hex(0x191724),
        picker_bg: hex(0x11111b),
        elevated_bg: hex(0x0d0d16),
        muted_border: hex(0x26233a),
        fg: hex(0xe0def4),
        fg_dim: hex(0x6e6a86),
        fg_bright: hex(0xf0edf6),
        accent: hex(0xc4a7e7),
        error: hex(0xeb6f92),
        warning: hex(0xf6c177),
        success: hex(0x9ccfd8),
        selection_fg: hex(0x191724),
        selection_bg: hex(0xc4a7e7),
        border: hex(0x26233a),
        border_active: hex(0xc4a7e7),
        volume_low: hex(0x9ccfd8),
        volume_medium: hex(0xf6c177),
        volume_high: hex(0xeb6f92),
        sidebar_active_border: hex(0xc4a7e7),
    }
}

/// Everforest — NvChad palette
fn everforest() -> AppTheme {
    AppTheme {
        bg: hex(0x2d353b),
        pane_bg: hex(0x2d353b),
        picker_bg: hex(0x273036),
        elevated_bg: hex(0x232a30),
        muted_border: hex(0x414b52),
        fg: hex(0xd3c6aa),
        fg_dim: hex(0x7a8478),
        fg_bright: hex(0xeae4c9),
        accent: hex(0xa7c080),
        error: hex(0xe67e80),
        warning: hex(0xe69875),
        success: hex(0xa7c080),
        selection_fg: hex(0x2d353b),
        selection_bg: hex(0xa7c080),
        border: hex(0x414b52),
        border_active: hex(0xa7c080),
        volume_low: hex(0xa7c080),
        volume_medium: hex(0xe69875),
        volume_high: hex(0xe67e80),
        sidebar_active_border: hex(0xa7c080),
    }
}

/// Kanagawa — NvChad palette
fn kanagawa() -> AppTheme {
    AppTheme {
        bg: hex(0x1f1f28),
        pane_bg: hex(0x1f1f28),
        picker_bg: hex(0x181820),
        elevated_bg: hex(0x14141b),
        muted_border: hex(0x727169),
        fg: hex(0xdcd7ba),
        fg_dim: hex(0x727169),
        fg_bright: hex(0xc8c0b3),
        accent: hex(0x7e9cd8),
        error: hex(0xc34043),
        warning: hex(0xe6c384),
        success: hex(0x98bb6c),
        selection_fg: hex(0x1f1f28),
        selection_bg: hex(0x7e9cd8),
        border: hex(0x2d4f67),
        border_active: hex(0x7e9cd8),
        volume_low: hex(0x98bb6c),
        volume_medium: hex(0xe6c384),
        volume_high: hex(0xc34043),
        sidebar_active_border: hex(0x7e9cd8),
    }
}

/// Catppuccin Latte (light) — NvChad palette
fn catppuccin_latte() -> AppTheme {
    AppTheme {
        bg: hex(0xeff1f5),
        pane_bg: hex(0xeff1f5),
        picker_bg: hex(0xe6e9ef),
        elevated_bg: hex(0xdde1e9),
        muted_border: hex(0xccd0da),
        fg: hex(0x4c4f69),
        fg_dim: hex(0x9ca0b0),
        fg_bright: hex(0x1e1e2e),
        accent: hex(0x1e66f5),
        error: hex(0xd20f39),
        warning: hex(0xfe640b),
        success: hex(0x40a02b),
        selection_fg: hex(0xeff1f5),
        selection_bg: hex(0x1e66f5),
        border: hex(0xccd0da),
        border_active: hex(0x1e66f5),
        volume_low: hex(0x40a02b),
        volume_medium: hex(0xfe640b),
        volume_high: hex(0xd20f39),
        sidebar_active_border: hex(0x1e66f5),
    }
}

/// Kanagawa Lotus (light) — NvChad palette
fn kanagawa_lotus() -> AppTheme {
    AppTheme {
        bg: hex(0xf2ecbc),
        pane_bg: hex(0xf2ecbc),
        picker_bg: hex(0xeae4c9),
        elevated_bg: hex(0xe3dcc2),
        muted_border: hex(0xdcdbc4),
        fg: hex(0x545464),
        fg_dim: hex(0x949494),
        fg_bright: hex(0x434343),
        accent: hex(0x2d6a9f),
        error: hex(0xc84053),
        warning: hex(0xb47e2b),
        success: hex(0x6a9589),
        selection_fg: hex(0xf2ecbc),
        selection_bg: hex(0x2d6a9f),
        border: hex(0xdcdbc4),
        border_active: hex(0x2d6a9f),
        volume_low: hex(0x6a9589),
        volume_medium: hex(0xb47e2b),
        volume_high: hex(0xc84053),
        sidebar_active_border: hex(0x2d6a9f),
    }
}

/// Built-in theme table, constructed once and cached for the process lifetime.
pub fn builtin_themes() -> &'static [ThemeEntry] {
    static BUILTINS: std::sync::OnceLock<Vec<ThemeEntry>> = std::sync::OnceLock::new();
    BUILTINS
        .get_or_init(|| {
            vec![
                ThemeEntry {
                    name: Cow::Borrowed("Chadrula"),
                    light: false,
                    theme: chadrula(),
                },
                ThemeEntry {
                    name: Cow::Borrowed("One Dark"),
                    light: false,
                    theme: one_dark(),
                },
                ThemeEntry {
                    name: Cow::Borrowed("Tokyo Night"),
                    light: false,
                    theme: tokyonight(),
                },
                ThemeEntry {
                    name: Cow::Borrowed("Tokyo Night Storm"),
                    light: false,
                    theme: tokyonight_storm(),
                },
                ThemeEntry {
                    name: Cow::Borrowed("Catppuccin Mocha"),
                    light: false,
                    theme: catppuccin_mocha(),
                },
                ThemeEntry {
                    name: Cow::Borrowed("Catppuccin Latte"),
                    light: true,
                    theme: catppuccin_latte(),
                },
                ThemeEntry {
                    name: Cow::Borrowed("Gruvbox Dark"),
                    light: false,
                    theme: gruvbox_dark(),
                },
                ThemeEntry {
                    name: Cow::Borrowed("Nord"),
                    light: false,
                    theme: nord(),
                },
                ThemeEntry {
                    name: Cow::Borrowed("Rose Pine"),
                    light: false,
                    theme: rose_pine(),
                },
                ThemeEntry {
                    name: Cow::Borrowed("Everforest"),
                    light: false,
                    theme: everforest(),
                },
                ThemeEntry {
                    name: Cow::Borrowed("Kanagawa"),
                    light: false,
                    theme: kanagawa(),
                },
                ThemeEntry {
                    name: Cow::Borrowed("Kanagawa Lotus"),
                    light: true,
                    theme: kanagawa_lotus(),
                },
            ]
        })
        .as_slice()
}

// ─── User themes (TOML) ───────────────────────────────────────────────

/// `~/.config/gtm/themes/*.toml` directory.
pub fn user_themes_dir() -> std::path::PathBuf {
    let config = std::env::var("XDG_CONFIG_HOME")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| {
            let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".into());
            std::path::PathBuf::from(home).join(".config")
        });
    config.join("gtm").join("themes")
}

#[derive(serde::Deserialize)]
struct UserThemeFile {
    name: String,
    #[serde(default)]
    light: bool,
    bg: String,
    /// Optional for backward compatibility with theme TOMLs written before
    /// `pane_bg` existed; defaults to `bg`.
    #[serde(default)]
    pane_bg: Option<String>,
    picker_bg: String,
    /// Optional for backward compatibility with pre-C0 theme TOMLs.
    #[serde(default)]
    elevated_bg: Option<String>,
    /// Optional for backward compatibility with pre-C0 theme TOMLs.
    #[serde(default)]
    muted_border: Option<String>,
    fg: String,
    fg_dim: String,
    fg_bright: String,
    accent: String,
    error: String,
    warning: String,
    success: String,
    selection_fg: String,
    selection_bg: String,
    border: String,
    border_active: String,
    volume_low: String,
    volume_medium: String,
    volume_high: String,
    sidebar_active_border: String,
}

impl UserThemeFile {
    fn into_theme(self) -> Result<AppTheme, String> {
        Ok(AppTheme {
            bg: parse_color(&self.bg)?,
            pane_bg: parse_color(self.pane_bg.as_deref().unwrap_or(self.bg.as_str()))?,
            picker_bg: parse_color(&self.picker_bg)?,
            elevated_bg: parse_color(
                self.elevated_bg
                    .as_deref()
                    .unwrap_or(self.picker_bg.as_str()),
            )?,
            muted_border: parse_color(
                self.muted_border.as_deref().unwrap_or(self.border.as_str()),
            )?,
            fg: parse_color(&self.fg)?,
            fg_dim: parse_color(&self.fg_dim)?,
            fg_bright: parse_color(&self.fg_bright)?,
            accent: parse_color(&self.accent)?,
            error: parse_color(&self.error)?,
            warning: parse_color(&self.warning)?,
            success: parse_color(&self.success)?,
            selection_fg: parse_color(&self.selection_fg)?,
            selection_bg: parse_color(&self.selection_bg)?,
            border: parse_color(&self.border)?,
            border_active: parse_color(&self.border_active)?,
            volume_low: parse_color(&self.volume_low)?,
            volume_medium: parse_color(&self.volume_medium)?,
            volume_high: parse_color(&self.volume_high)?,
            sidebar_active_border: parse_color(&self.sidebar_active_border)?,
        })
    }
}

/// Load every `*.toml` theme file under [`user_themes_dir`]. Unparseable
/// files are silently skipped so a single corrupt file doesn't kill the TUI.
pub fn load_user_themes() -> Vec<ThemeEntry> {
    let dir = match std::fs::read_dir(user_themes_dir()) {
        Ok(d) => d,
        Err(_) => return Vec::new(),
    };
    let mut out = Vec::new();
    for entry in dir.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("toml") {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        let Ok(mut parsed) = toml::from_str::<UserThemeFile>(&text) else {
            continue;
        };
        let name = std::mem::take(&mut parsed.name);
        let declared_light = parsed.light;
        let Ok(theme) = parsed.into_theme() else {
            continue;
        };
        // Trust the explicit flag; fall back to a luminance heuristic so a
        // user-authored light theme without `light = true` still renders
        // correctly in the picker.
        let light = declared_light || theme_light(&theme);
        out.push(ThemeEntry {
            name: Cow::Owned(name),
            light,
            theme,
        });
    }
    out
}

/// Heuristic: classify a theme as light if its `bg` luminance is high.
/// Used as a fallback for user themes that omit the explicit `light` flag.
fn theme_light(t: &AppTheme) -> bool {
    if let Color::Rgb(r, g, b) = t.bg {
        0.299 * r as f64 + 0.587 * g as f64 + 0.114 * b as f64 > 128.0
    } else {
        false
    }
}

/// Built-in themes followed by user themes; user themes replace built-ins on
/// name collision so users can override defaults without deleting files.
pub fn merged_themes() -> Vec<ThemeEntry> {
    let mut v: Vec<ThemeEntry> = builtin_themes().to_vec();
    for ut in load_user_themes() {
        if let Some(existing) = v.iter_mut().find(|t| t.name == ut.name) {
            *existing = ut;
        } else {
            v.push(ut);
        }
    }
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_color_accepts_hash_hex() {
        assert_eq!(
            parse_color("#7aa2f7").unwrap(),
            Color::Rgb(0x7a, 0xa2, 0xf7)
        );
        assert_eq!(parse_color("7aa2f7").unwrap(), Color::Rgb(0x7a, 0xa2, 0xf7));
    }

    #[test]
    fn parse_color_rejects_bad_input() {
        assert!(parse_color("#abc").is_err());
        assert!(parse_color("zzzzzz").is_err());
        assert!(parse_color("").is_err());
    }

    #[test]
    fn builtin_themes_have_unique_names() {
        let names: Vec<&str> = builtin_themes().iter().map(|t| t.name.as_ref()).collect();
        let mut sorted = names.clone();
        sorted.sort();
        sorted.dedup();
        assert_eq!(sorted.len(), names.len(), "duplicate theme names");
    }

    #[test]
    fn tokyonight_storm_derives_from_chadrula() {
        let base = chadrula();
        let storm = tokyonight_storm();
        // Shared fields stay in sync with the base.
        assert_eq!(storm.bg, base.bg);
        assert_eq!(storm.accent, base.accent);
        assert_eq!(storm.success, base.success);
        // Overridden fields differ.
        assert_ne!(storm.fg, base.fg);
        assert_ne!(storm.warning, base.warning);
    }

    #[test]
    fn light_themes_are_flagged() {
        for t in builtin_themes() {
            let luma = match t.theme.bg {
                Color::Rgb(r, g, b) => 0.299 * r as f64 + 0.587 * g as f64 + 0.114 * b as f64,
                _ => 0.0,
            };
            let expected_light = luma > 128.0;
            assert_eq!(
                t.light, expected_light,
                "theme {} flagged light={} but bg luma is {:.0}",
                t.name, t.light, luma
            );
        }
    }

    #[test]
    fn user_theme_round_trip() {
        let toml = r##"
            name = "Test"
            light = true
            bg = "#eff1f5"
            picker_bg = "#e6e9ef"
            elevated_bg = "#dde1e9"
            muted_border = "#ccd0da"
            fg = "#4c4f69"
            fg_dim = "#9ca0b0"
            fg_bright = "#1e1e2e"
            accent = "#1e66f5"
            error = "#d20f39"
            warning = "#fe640b"
            success = "#40a02b"
            selection_fg = "#eff1f5"
            selection_bg = "#1e66f5"
            border = "#ccd0da"
            border_active = "#1e66f5"
            volume_low = "#40a02b"
            volume_medium = "#fe640b"
            volume_high = "#d20f39"
            sidebar_active_border = "#1e66f5"
        "##;
        let parsed: UserThemeFile = toml::from_str(toml).unwrap();
        let theme = parsed.into_theme().unwrap();
        assert_eq!(theme.bg, Color::Rgb(0xef, 0xf1, 0xf5));
        assert_eq!(theme.accent, Color::Rgb(0x1e, 0x66, 0xf5));
        assert_eq!(theme.elevated_bg, Color::Rgb(0xdd, 0xe1, 0xe9));
        assert_eq!(theme.muted_border, Color::Rgb(0xcc, 0xd0, 0xda));
    }

    #[test]
    fn user_theme_missing_c0_fields_fall_back() {
        // Pre-C0 TOML without elevated_bg/muted_border must still parse.
        let toml = r##"
            name = "Legacy"
            light = false
            bg = "#1a1b26"
            picker_bg = "#16161e"
            fg = "#a9b1d6"
            fg_dim = "#565f89"
            fg_bright = "#c0caf5"
            accent = "#7aa2f7"
            error = "#f7768e"
            warning = "#ff9e64"
            success = "#9ece6a"
            selection_fg = "#1a1b26"
            selection_bg = "#7aa2f7"
            border = "#292e42"
            border_active = "#7aa2f7"
            volume_low = "#9ece6a"
            volume_medium = "#ff9e64"
            volume_high = "#f7768e"
            sidebar_active_border = "#7aa2f7"
        "##;
        let parsed: UserThemeFile = toml::from_str(toml).unwrap();
        let theme = parsed.into_theme().unwrap();
        assert_eq!(theme.elevated_bg, theme.picker_bg);
        assert_eq!(theme.muted_border, theme.border);
    }

    #[test]
    fn merged_themes_replaces_on_collision() {
        let mut v: Vec<ThemeEntry> = builtin_themes().to_vec();
        let custom = ThemeEntry {
            name: Cow::Borrowed("Chadrula"),
            light: true,
            theme: chadrula(),
        };
        if let Some(existing) = v.iter_mut().find(|t| t.name == custom.name) {
            *existing = custom.clone();
        } else {
            v.push(custom);
        }
        assert_eq!(v.len(), builtin_themes().len());
        assert!(v.iter().find(|t| t.name == "Chadrula").unwrap().light);
    }
}
