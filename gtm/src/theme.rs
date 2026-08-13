// Copyright (c) 2025 - present
// Author: prjctimg <prjctimg@outlook.com>
// NvChad-inspired color themes for the TUI
//
// This is free software released under the GPL-3.0 license.

use ratatui::style::Color;

/// Central theme — all UI colors flow through here.
/// The TUI renders its own explicit `bg` behind everything.
#[derive(Clone, Copy)]
pub struct AppTheme {
    pub bg: Color,
    pub overlay_bg: Color,
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
    pub tab_active_fg: Color,
    pub tab_active_bg: Color,
    pub tab_inactive_fg: Color,
    pub tab_bar_bg: Color,
    pub sidebar_active_border: Color,
    // Syntax-highlighting-inspired colors repurposed as footer module accents
    pub syn_keyword: Color,
    pub syn_string: Color,
    pub syn_function: Color,
    pub syn_variable: Color,
    pub syn_comment: Color,
    pub syn_constant: Color,
    pub syn_type: Color,
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

// ─── Presets ──────────────────────────────────────────────────────────

fn hex(c: u32) -> Color {
    let r = ((c >> 16) & 0xFF) as u8;
    let g = ((c >> 8) & 0xFF) as u8;
    let b = (c & 0xFF) as u8;
    Color::Rgb(r, g, b)
}

/// Chadrula — NvChad default
fn chadrula() -> AppTheme {
    AppTheme {
        bg: hex(0x24283b),
        overlay_bg: hex(0x1f2335),
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
        tab_active_fg: hex(0x24283b),
        tab_active_bg: hex(0x7aa2f7),
        tab_inactive_fg: hex(0x7aa2f7),
        tab_bar_bg: hex(0x1f2335),
        sidebar_active_border: hex(0x7aa2f7),
        syn_keyword: hex(0xbb9af7),
        syn_string: hex(0x9ece6a),
        syn_function: hex(0x7aa2f7),
        syn_variable: hex(0xc0caf5),
        syn_comment: hex(0x565f89),
        syn_constant: hex(0xff9e64),
        syn_type: hex(0x2ac3de),
    }
}

/// One Dark — NvChad palette
fn one_dark() -> AppTheme {
    AppTheme {
        bg: hex(0x282c34),
        overlay_bg: hex(0x21252b),
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
        tab_active_fg: hex(0x282c34),
        tab_active_bg: hex(0x61afef),
        tab_inactive_fg: hex(0x61afef),
        tab_bar_bg: hex(0x21252b),
        sidebar_active_border: hex(0x61afef),
        syn_keyword: hex(0xc678dd),
        syn_string: hex(0x98c379),
        syn_function: hex(0x61afef),
        syn_variable: hex(0xe06c75),
        syn_comment: hex(0x5c6370),
        syn_constant: hex(0xd19a66),
        syn_type: hex(0xe5c07b),
    }
}

/// Tokyo Night — NvChad palette
fn tokyonight() -> AppTheme {
    AppTheme {
        bg: hex(0x1a1b26),
        overlay_bg: hex(0x16161e),
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
        tab_active_fg: hex(0x1a1b26),
        tab_active_bg: hex(0x7aa2f7),
        tab_inactive_fg: hex(0x7aa2f7),
        tab_bar_bg: hex(0x16161e),
        sidebar_active_border: hex(0x7aa2f7),
        syn_keyword: hex(0xbb9af7),
        syn_string: hex(0x9ece6a),
        syn_function: hex(0x7aa2f7),
        syn_variable: hex(0xc0caf5),
        syn_comment: hex(0x565f89),
        syn_constant: hex(0xff9e64),
        syn_type: hex(0x2ac3de),
    }
}

/// Catppuccin Mocha — NvChad palette
fn catppuccin_mocha() -> AppTheme {
    AppTheme {
        bg: hex(0x1e1e2e),
        overlay_bg: hex(0x181825),
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
        tab_active_fg: hex(0x1e1e2e),
        tab_active_bg: hex(0x89b4fa),
        tab_inactive_fg: hex(0x89b4fa),
        tab_bar_bg: hex(0x181825),
        sidebar_active_border: hex(0x89b4fa),
        syn_keyword: hex(0xcbafc7),
        syn_string: hex(0xa6e3a1),
        syn_function: hex(0x89b4fa),
        syn_variable: hex(0xf5c2e7),
        syn_comment: hex(0x6c7086),
        syn_constant: hex(0xfab387),
        syn_type: hex(0xf9e2af),
    }
}

/// Gruvbox Dark — NvChad palette
fn gruvbox_dark() -> AppTheme {
    AppTheme {
        bg: hex(0x282828),
        overlay_bg: hex(0x1d2021),
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
        tab_active_fg: hex(0x282828),
        tab_active_bg: hex(0xd3869b),
        tab_inactive_fg: hex(0xd3869b),
        tab_bar_bg: hex(0x1d2021),
        sidebar_active_border: hex(0xd3869b),
        syn_keyword: hex(0xfb4934),
        syn_string: hex(0xb8bb26),
        syn_function: hex(0xb8bb26),
        syn_variable: hex(0xebdbb2),
        syn_comment: hex(0x928374),
        syn_constant: hex(0xd3869b),
        syn_type: hex(0x83a598),
    }
}

/// Nord — NvChad palette
fn nord() -> AppTheme {
    AppTheme {
        bg: hex(0x2e3440),
        overlay_bg: hex(0x2b303b),
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
        tab_active_fg: hex(0x2e3440),
        tab_active_bg: hex(0x88c0d0),
        tab_inactive_fg: hex(0x88c0d0),
        tab_bar_bg: hex(0x2b303b),
        sidebar_active_border: hex(0x88c0d0),
        syn_keyword: hex(0x81a1c1),
        syn_string: hex(0xa3be8c),
        syn_function: hex(0x88c0d0),
        syn_variable: hex(0xd8dee9),
        syn_comment: hex(0x4c566a),
        syn_constant: hex(0xb48ead),
        syn_type: hex(0x8fbcbb),
    }
}

/// Rose Pine — NvChad palette
fn rose_pine() -> AppTheme {
    AppTheme {
        bg: hex(0x191724),
        overlay_bg: hex(0x11111b),
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
        tab_active_fg: hex(0x191724),
        tab_active_bg: hex(0xc4a7e7),
        tab_inactive_fg: hex(0xc4a7e7),
        tab_bar_bg: hex(0x11111b),
        sidebar_active_border: hex(0xc4a7e7),
        syn_keyword: hex(0xc4a7e7),
        syn_string: hex(0x9ccfd8),
        syn_function: hex(0xc4a7e7),
        syn_variable: hex(0xebbcba),
        syn_comment: hex(0x6e6a86),
        syn_constant: hex(0xf6c177),
        syn_type: hex(0xea9a97),
    }
}

/// Everforest — NvChad palette
fn everforest() -> AppTheme {
    AppTheme {
        bg: hex(0x2d353b),
        overlay_bg: hex(0x273036),
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
        tab_active_fg: hex(0x2d353b),
        tab_active_bg: hex(0xa7c080),
        tab_inactive_fg: hex(0xa7c080),
        tab_bar_bg: hex(0x273036),
        sidebar_active_border: hex(0xa7c080),
        syn_keyword: hex(0xd3c6aa),
        syn_string: hex(0xa7c080),
        syn_function: hex(0x7fbbb3),
        syn_variable: hex(0xd3c6aa),
        syn_comment: hex(0x7a8478),
        syn_constant: hex(0xdbcbb7),
        syn_type: hex(0xeaedc9),
    }
}

/// Kanagawa — NvChad palette
fn kanagawa() -> AppTheme {
    AppTheme {
        bg: hex(0x1f1f28),
        overlay_bg: hex(0x181820),
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
        tab_active_fg: hex(0x1f1f28),
        tab_active_bg: hex(0x7e9cd8),
        tab_inactive_fg: hex(0x7e9cd8),
        tab_bar_bg: hex(0x181820),
        sidebar_active_border: hex(0x7e9cd8),
        syn_keyword: hex(0xc47ea0),
        syn_string: hex(0x98bb6c),
        syn_function: hex(0x7e9cd8),
        syn_variable: hex(0x7e9cd8),
        syn_comment: hex(0x727169),
        syn_constant: hex(0xff9e64),
        syn_type: hex(0x7fb4ca),
    }
}

/// Catppuccin Latte (light) — NvChad palette
fn catppuccin_latte() -> AppTheme {
    AppTheme {
        bg: hex(0xeff1f5),
        overlay_bg: hex(0xe6e9ef),
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
        tab_active_fg: hex(0xeff1f5),
        tab_active_bg: hex(0x1e66f5),
        tab_inactive_fg: hex(0x1e66f5),
        tab_bar_bg: hex(0xe6e9ef),
        sidebar_active_border: hex(0x1e66f5),
        syn_keyword: hex(0x8839ef),
        syn_string: hex(0x40a02b),
        syn_function: hex(0x1e66f5),
        syn_variable: hex(0xea76cb),
        syn_comment: hex(0x9ca0b0),
        syn_constant: hex(0xfe640b),
        syn_type: hex(0x04a5e5),
    }
}

/// Tokyo Night Storm — NvChad palette
fn tokyonight_storm() -> AppTheme {
    AppTheme {
        bg: hex(0x24283b),
        overlay_bg: hex(0x1f2335),
        fg: hex(0xa9b1d6),
        fg_dim: hex(0x565f89),
        fg_bright: hex(0xc0caf5),
        accent: hex(0x7aa2f7),
        error: hex(0xf7768e),
        warning: hex(0xff9e64),
        success: hex(0x9ece6a),
        selection_fg: hex(0x24283b),
        selection_bg: hex(0x7aa2f7),
        border: hex(0x3b4261),
        border_active: hex(0x7aa2f7),
        volume_low: hex(0x9ece6a),
        volume_medium: hex(0xff9e64),
        volume_high: hex(0xf7768e),
        tab_active_fg: hex(0x24283b),
        tab_active_bg: hex(0x7aa2f7),
        tab_inactive_fg: hex(0x7aa2f7),
        tab_bar_bg: hex(0x1f2335),
        sidebar_active_border: hex(0x7aa2f7),
        syn_keyword: hex(0xbb9af7),
        syn_string: hex(0x9ece6a),
        syn_function: hex(0x7aa2f7),
        syn_variable: hex(0xc0caf5),
        syn_comment: hex(0x565f89),
        syn_constant: hex(0xff9e64),
        syn_type: hex(0x2ac3de),
    }
}

/// Kanagawa Lotus (light) — NvChad palette
fn kanagawa_lotus() -> AppTheme {
    AppTheme {
        bg: hex(0xf2ecbc),
        overlay_bg: hex(0xeae4c9),
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
        tab_active_fg: hex(0xf2ecbc),
        tab_active_bg: hex(0x2d6a9f),
        tab_inactive_fg: hex(0x2d6a9f),
        tab_bar_bg: hex(0xeae4c9),
        sidebar_active_border: hex(0x2d6a9f),
        syn_keyword: hex(0x8f4f8f),
        syn_string: hex(0x6a9589),
        syn_function: hex(0x2d6a9f),
        syn_variable: hex(0x396a6f),
        syn_comment: hex(0x949494),
        syn_constant: hex(0xb47e2b),
        syn_type: hex(0x396a6f),
    }
}

/// Theme registry — name + constructor
pub struct ThemeEntry {
    pub name: &'static str,
    pub builder: fn() -> AppTheme,
}

pub const THEMES: &[ThemeEntry] = &[
    ThemeEntry { name: "Chadrula", builder: chadrula },
    ThemeEntry { name: "One Dark", builder: one_dark },
    ThemeEntry { name: "Tokyo Night", builder: tokyonight },
    ThemeEntry { name: "Tokyo Night Storm", builder: tokyonight_storm },
    ThemeEntry { name: "Catppuccin Mocha", builder: catppuccin_mocha },
    ThemeEntry { name: "Catppuccin Latte", builder: catppuccin_latte },
    ThemeEntry { name: "Gruvbox Dark", builder: gruvbox_dark },
    ThemeEntry { name: "Nord", builder: nord },
    ThemeEntry { name: "Rose Pine", builder: rose_pine },
    ThemeEntry { name: "Everforest", builder: everforest },
    ThemeEntry { name: "Kanagawa", builder: kanagawa },
    ThemeEntry { name: "Kanagawa Lotus", builder: kanagawa_lotus },
];
