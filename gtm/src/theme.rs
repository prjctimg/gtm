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

/// Default Cyberdeck TUI
fn cyberdeck() -> AppTheme {
    AppTheme {
        bg: hex(0x141313),
        overlay_bg: hex(0x141313),
        fg: hex(0xe5e2e1),
        fg_dim: hex(0x6e6d6d),
        fg_bright: hex(0xc4c7c7),
        accent: hex(0x00e639),
        error: hex(0xffb4ab),
        warning: hex(0xffdad6),
        success: hex(0x00e639),
        selection_fg: hex(0xb3b3b3),
        selection_bg: hex(0x3a3a3a),
        border: hex(0x8e9192),
        border_active: hex(0xc8c6c5),
        volume_low: hex(0x00e639),
        volume_medium: hex(0xe5e2e1),
        volume_high: hex(0xffb4ab),
        tab_active_fg: hex(0x313030),
        tab_active_bg: hex(0xc8c6c5),
        tab_inactive_fg: hex(0x8e9192),
        tab_bar_bg: hex(0x141313),
        sidebar_active_border: hex(0x00e639),
    }
}

/// Catppuccin Mocha — OpenCode palette
#[allow(dead_code)]
fn catppuccin_mocha() -> AppTheme {
    AppTheme {
        bg: hex(0x1e1e2e),
        overlay_bg: hex(0x1e1e2e),
        fg: hex(0xcdd6f4),
        fg_dim: hex(0x6c7086),
        fg_bright: hex(0xffffff),
        accent: hex(0x89b4fa),
        error: hex(0xf38ba8),
        warning: hex(0xfab387),
        success: hex(0xa6e3a1),
        selection_fg: hex(0x1e1e2e),
        selection_bg: hex(0x89b4fa),
        border: hex(0x585b70),
        border_active: hex(0x89b4fa),
        volume_low: hex(0xa6e3a1),
        volume_medium: hex(0xfab387),
        volume_high: hex(0xf38ba8),
        tab_active_fg: hex(0x1e1e2e),
        tab_active_bg: hex(0x89b4fa),
        tab_inactive_fg: hex(0x89b4fa),
        tab_bar_bg: hex(0x1e1e2e),
        sidebar_active_border: hex(0x89b4fa),
    }
}

/// Catppuccin Macchiato — OpenCode palette
#[allow(dead_code)]
fn catppuccin_macchiato() -> AppTheme {
    AppTheme {
        bg: hex(0x24273a),
        overlay_bg: hex(0x24273a),
        fg: hex(0xcad3f5),
        fg_dim: hex(0x6e738d),
        fg_bright: hex(0xffffff),
        accent: hex(0x8aadf4),
        error: hex(0xed8796),
        warning: hex(0xf5a97f),
        success: hex(0xa6da95),
        selection_fg: hex(0x24273a),
        selection_bg: hex(0x8aadf4),
        border: hex(0x5b6078),
        border_active: hex(0x8aadf4),
        volume_low: hex(0xa6da95),
        volume_medium: hex(0xf5a97f),
        volume_high: hex(0xed8796),
        tab_active_fg: hex(0x24273a),
        tab_active_bg: hex(0x8aadf4),
        tab_inactive_fg: hex(0x8aadf4),
        tab_bar_bg: hex(0x24273a),
        sidebar_active_border: hex(0x8aadf4),
    }
}

/// Catppuccin Frappe — OpenCode palette
#[allow(dead_code)]
fn catppuccin_frappe() -> AppTheme {
    AppTheme {
        bg: hex(0x303446),
        overlay_bg: hex(0x303446),
        fg: hex(0xc6d0f5),
        fg_dim: hex(0x737994),
        fg_bright: hex(0xffffff),
        accent: hex(0x8caaee),
        error: hex(0xe78284),
        warning: hex(0xef9f76),
        success: hex(0xa6d189),
        selection_fg: hex(0x303446),
        selection_bg: hex(0x8caaee),
        border: hex(0x626880),
        border_active: hex(0x8caaee),
        volume_low: hex(0xa6d189),
        volume_medium: hex(0xef9f76),
        volume_high: hex(0xe78284),
        tab_active_fg: hex(0x303446),
        tab_active_bg: hex(0x8caaee),
        tab_inactive_fg: hex(0x8caaee),
        tab_bar_bg: hex(0x303446),
        sidebar_active_border: hex(0x8caaee),
    }
}

/// Catppuccin Latte (light) — OpenCode palette
#[allow(dead_code)]
fn catppuccin_latte() -> AppTheme {
    AppTheme {
        bg: hex(0xeff1f5),
        overlay_bg: hex(0xeff1f5),
        fg: hex(0x4c4f69),
        fg_dim: hex(0x9ca0b0),
        fg_bright: hex(0x000000),
        accent: hex(0x1e66f5),
        error: hex(0xd20f39),
        warning: hex(0xfe640b),
        success: hex(0x40a02b),
        selection_fg: hex(0xeff1f5),
        selection_bg: hex(0x1e66f5),
        border: hex(0xbcc0cc),
        border_active: hex(0x1e66f5),
        volume_low: hex(0x40a02b),
        volume_medium: hex(0xfe640b),
        volume_high: hex(0xd20f39),
        tab_active_fg: hex(0xeff1f5),
        tab_active_bg: hex(0x1e66f5),
        tab_inactive_fg: hex(0x1e66f5),
        tab_bar_bg: hex(0xeff1f5),
        sidebar_active_border: hex(0x1e66f5),
    }
}

/// Tokyonight Night — OpenCode palette
#[allow(dead_code)]
fn tokyonight_night() -> AppTheme {
    AppTheme {
        bg: hex(0x1a1b26),
        overlay_bg: hex(0x1a1b26),
        fg: hex(0xa9b1d6),
        fg_dim: hex(0x565f89),
        fg_bright: hex(0xc0caf5),
        accent: hex(0x7aa2f7),
        error: hex(0xf7768e),
        warning: hex(0xff9e64),
        success: hex(0x9ece6a),
        selection_fg: hex(0x1a1b26),
        selection_bg: hex(0x7aa2f7),
        border: hex(0x565f89),
        border_active: hex(0x7aa2f7),
        volume_low: hex(0x9ece6a),
        volume_medium: hex(0xff9e64),
        volume_high: hex(0xf7768e),
        tab_active_fg: hex(0x1a1b26),
        tab_active_bg: hex(0x7aa2f7),
        tab_inactive_fg: hex(0x7aa2f7),
        tab_bar_bg: hex(0x1a1b26),
        sidebar_active_border: hex(0x7aa2f7),
    }
}

/// Tokyonight Day (light) — OpenCode palette
#[allow(dead_code)]
fn tokyonight_day() -> AppTheme {
    AppTheme {
        bg: hex(0xe1e2e7),
        overlay_bg: hex(0xe1e2e7),
        fg: hex(0x3760bf),
        fg_dim: hex(0xa1a6c5),
        fg_bright: hex(0x1a1b26),
        accent: hex(0x2e7de9),
        error: hex(0xf52a65),
        warning: hex(0xb15c00),
        success: hex(0x587539),
        selection_fg: hex(0xe1e2e7),
        selection_bg: hex(0x2e7de9),
        border: hex(0xa1a6c5),
        border_active: hex(0x2e7de9),
        volume_low: hex(0x587539),
        volume_medium: hex(0xb15c00),
        volume_high: hex(0xf52a65),
        tab_active_fg: hex(0xe1e2e7),
        tab_active_bg: hex(0x2e7de9),
        tab_inactive_fg: hex(0x2e7de9),
        tab_bar_bg: hex(0xe1e2e7),
        sidebar_active_border: hex(0x2e7de9),
    }
}

/// Gruvbox Dark — OpenCode palette
#[allow(dead_code)]
fn gruvbox_dark() -> AppTheme {
    AppTheme {
        bg: hex(0x282828),
        overlay_bg: hex(0x282828),
        fg: hex(0xebdbb2),
        fg_dim: hex(0x928374),
        fg_bright: hex(0xebdbb2),
        accent: hex(0x83a598),
        error: hex(0xfb4934),
        warning: hex(0xfe8019),
        success: hex(0xb8bb26),
        selection_fg: hex(0x282828),
        selection_bg: hex(0x83a598),
        border: hex(0x665c54),
        border_active: hex(0x83a598),
        volume_low: hex(0xb8bb26),
        volume_medium: hex(0xfe8019),
        volume_high: hex(0xfb4934),
        tab_active_fg: hex(0x282828),
        tab_active_bg: hex(0x83a598),
        tab_inactive_fg: hex(0x83a598),
        tab_bar_bg: hex(0x282828),
        sidebar_active_border: hex(0x83a598),
    }
}

/// Gruvbox Light — OpenCode palette
#[allow(dead_code)]
fn gruvbox_light() -> AppTheme {
    AppTheme {
        bg: hex(0xfbf1c7),
        overlay_bg: hex(0xfbf1c7),
        fg: hex(0x3c3836),
        fg_dim: hex(0x928374),
        fg_bright: hex(0x282828),
        accent: hex(0x458588),
        error: hex(0x9d0006),
        warning: hex(0xd65d0e),
        success: hex(0x98971a),
        selection_fg: hex(0xfbf1c7),
        selection_bg: hex(0x458588),
        border: hex(0xbdae93),
        border_active: hex(0x458588),
        volume_low: hex(0x98971a),
        volume_medium: hex(0xd65d0e),
        volume_high: hex(0x9d0006),
        tab_active_fg: hex(0xfbf1c7),
        tab_active_bg: hex(0x458588),
        tab_inactive_fg: hex(0x458588),
        tab_bar_bg: hex(0xfbf1c7),
        sidebar_active_border: hex(0x458588),
    }
}

/// Ayu Dark — OpenCode palette
#[allow(dead_code)]
fn ayu_dark() -> AppTheme {
    AppTheme {
        bg: hex(0x0a0e14),
        overlay_bg: hex(0x0a0e14),
        fg: hex(0xb3b1ad),
        fg_dim: hex(0x6e6c66),
        fg_bright: hex(0xffffff),
        accent: hex(0xff8f40),
        error: hex(0xff3333),
        warning: hex(0xff8f40),
        success: hex(0x7ec8a0),
        selection_fg: hex(0x0a0e14),
        selection_bg: hex(0xff8f40),
        border: hex(0x3d424d),
        border_active: hex(0xff8f40),
        volume_low: hex(0x7ec8a0),
        volume_medium: hex(0xff8f40),
        volume_high: hex(0xff3333),
        tab_active_fg: hex(0x0a0e14),
        tab_active_bg: hex(0xff8f40),
        tab_inactive_fg: hex(0xff8f40),
        tab_bar_bg: hex(0x0a0e14),
        sidebar_active_border: hex(0xff8f40),
    }
}

/// Ayu Light — OpenCode palette
#[allow(dead_code)]
fn ayu_light() -> AppTheme {
    AppTheme {
        bg: hex(0xfafafa),
        overlay_bg: hex(0xfafafa),
        fg: hex(0x5c6166),
        fg_dim: hex(0xa0a0a0),
        fg_bright: hex(0x1a1f29),
        accent: hex(0xff8a3c),
        error: hex(0xf51818),
        warning: hex(0xf07100),
        success: hex(0x7bac3e),
        selection_fg: hex(0xfafafa),
        selection_bg: hex(0xff8a3c),
        border: hex(0xd7dae0),
        border_active: hex(0xff8a3c),
        volume_low: hex(0x7bac3e),
        volume_medium: hex(0xf07100),
        volume_high: hex(0xf51818),
        tab_active_fg: hex(0xfafafa),
        tab_active_bg: hex(0xff8a3c),
        tab_inactive_fg: hex(0xff8a3c),
        tab_bar_bg: hex(0xfafafa),
        sidebar_active_border: hex(0xff8a3c),
    }
}

/// ── NEW: OpenCode-inspired themes ──

/// Everforest Dark — OpenCode palette
#[allow(dead_code)]
fn everforest() -> AppTheme {
    AppTheme {
        bg: hex(0x2d353b),
        overlay_bg: hex(0x2d353b),
        fg: hex(0xd3c6aa),
        fg_dim: hex(0x7a8478),
        fg_bright: hex(0xffffff),
        accent: hex(0xa7c080),
        error: hex(0xe67e80),
        warning: hex(0xe69875),
        success: hex(0x83c092),
        selection_fg: hex(0x2d353b),
        selection_bg: hex(0xa7c080),
        border: hex(0x4f5b57),
        border_active: hex(0xa7c080),
        volume_low: hex(0x83c092),
        volume_medium: hex(0xe69875),
        volume_high: hex(0xe67e80),
        tab_active_fg: hex(0x2d353b),
        tab_active_bg: hex(0xa7c080),
        tab_inactive_fg: hex(0xa7c080),
        tab_bar_bg: hex(0x2d353b),
        sidebar_active_border: hex(0xa7c080),
    }
}

/// Kanagawa — OpenCode palette
#[allow(dead_code)]
fn kanagawa() -> AppTheme {
    AppTheme {
        bg: hex(0x1f1f28),
        overlay_bg: hex(0x1f1f28),
        fg: hex(0xdcd7ba),
        fg_dim: hex(0x727169),
        fg_bright: hex(0xffffff),
        accent: hex(0x7e9cd8),
        error: hex(0xe82424),
        warning: hex(0xd7a657),
        success: hex(0x98bb6c),
        selection_fg: hex(0x1f1f28),
        selection_bg: hex(0x7e9cd8),
        border: hex(0x54546d),
        border_active: hex(0x7e9cd8),
        volume_low: hex(0x98bb6c),
        volume_medium: hex(0xd7a657),
        volume_high: hex(0xe82424),
        tab_active_fg: hex(0x1f1f28),
        tab_active_bg: hex(0x7e9cd8),
        tab_inactive_fg: hex(0x7e9cd8),
        tab_bar_bg: hex(0x1f1f28),
        sidebar_active_border: hex(0x7e9cd8),
    }
}

/// Nord — OpenCode palette
#[allow(dead_code)]
fn nord() -> AppTheme {
    AppTheme {
        bg: hex(0x2e3440),
        overlay_bg: hex(0x2e3440),
        fg: hex(0xe5e9f0),
        fg_dim: hex(0x616e88),
        fg_bright: hex(0xffffff),
        accent: hex(0x88c0d0),
        error: hex(0xbf616a),
        warning: hex(0xd08770),
        success: hex(0xa3be8c),
        selection_fg: hex(0x2e3440),
        selection_bg: hex(0x88c0d0),
        border: hex(0x434c5e),
        border_active: hex(0x88c0d0),
        volume_low: hex(0xa3be8c),
        volume_medium: hex(0xd08770),
        volume_high: hex(0xbf616a),
        tab_active_fg: hex(0x2e3440),
        tab_active_bg: hex(0x88c0d0),
        tab_inactive_fg: hex(0x88c0d0),
        tab_bar_bg: hex(0x2e3440),
        sidebar_active_border: hex(0x88c0d0),
    }
}

/// Matrix — hacker green-on-black
#[allow(dead_code)]
fn matrix() -> AppTheme {
    AppTheme {
        bg: hex(0x0a0e0a),
        overlay_bg: hex(0x0a0e0a),
        fg: hex(0x62ff94),
        fg_dim: hex(0x3a7a4a),
        fg_bright: hex(0xaeffc7),
        accent: hex(0x2eff6a),
        error: hex(0xff4b4b),
        warning: hex(0xe6ff57),
        success: hex(0x62ff94),
        selection_fg: hex(0x0a0e0a),
        selection_bg: hex(0x2eff6a),
        border: hex(0x1a4a2a),
        border_active: hex(0x2eff6a),
        volume_low: hex(0x62ff94),
        volume_medium: hex(0xe6ff57),
        volume_high: hex(0xff4b4b),
        tab_active_fg: hex(0x0a0e0a),
        tab_active_bg: hex(0x2eff6a),
        tab_inactive_fg: hex(0x2eff6a),
        tab_bar_bg: hex(0x0a0e0a),
        sidebar_active_border: hex(0x2eff6a),
    }
}

/// One Dark — OpenCode palette
#[allow(dead_code)]
fn one_dark() -> AppTheme {
    AppTheme {
        bg: hex(0x282c34),
        overlay_bg: hex(0x282c34),
        fg: hex(0xabb2bf),
        fg_dim: hex(0x5c6370),
        fg_bright: hex(0xffffff),
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
        tab_bar_bg: hex(0x282c34),
        sidebar_active_border: hex(0x61afef),
    }
}

/// Theme registry — name + constructor
pub struct ThemeEntry {
    pub name: &'static str,
    pub builder: fn() -> AppTheme,
}

pub const THEMES: &[ThemeEntry] = &[
    ThemeEntry { name: "Cyberdeck", builder: cyberdeck },
    ThemeEntry { name: "Catppuccin Mocha", builder: catppuccin_mocha },
    ThemeEntry { name: "Catppuccin Macchiato", builder: catppuccin_macchiato },
    ThemeEntry { name: "Catppuccin Frappe", builder: catppuccin_frappe },
    ThemeEntry { name: "Catppuccin Latte", builder: catppuccin_latte },
    ThemeEntry { name: "Tokyonight Night", builder: tokyonight_night },
    ThemeEntry { name: "Tokyonight Day", builder: tokyonight_day },
    ThemeEntry { name: "Gruvbox Dark", builder: gruvbox_dark },
    ThemeEntry { name: "Gruvbox Light", builder: gruvbox_light },
    ThemeEntry { name: "Ayu Dark", builder: ayu_dark },
    ThemeEntry { name: "Ayu Light", builder: ayu_light },
    // OpenCode-inspired additions
    ThemeEntry { name: "Everforest", builder: everforest },
    ThemeEntry { name: "Kanagawa", builder: kanagawa },
    ThemeEntry { name: "Nord", builder: nord },
    ThemeEntry { name: "Matrix", builder: matrix },
    ThemeEntry { name: "One Dark", builder: one_dark },
];
