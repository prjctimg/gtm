use ratatui::style::Color;

/// Central theme — all UI colors flow through here.
pub struct AppTheme {
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
}

impl AppTheme {
    pub fn default() -> Self {
        cyberdeck()
    }

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

/// Cyberdeck TUI — default theme
fn cyberdeck() -> AppTheme {
    AppTheme {
        overlay_bg: hex(0x141313),
        fg: hex(0xe5e2e1),
        fg_dim: hex(0x353434),
        fg_bright: hex(0xc4c7c7),
        accent: hex(0x00e639),
        error: hex(0xffb4ab),
        warning: hex(0xffdad6),
        success: hex(0x00e639),
        selection_fg: hex(0x313030),
        selection_bg: hex(0x454747),
        border: hex(0x8e9192),
        border_active: hex(0xc8c6c5),
        volume_low: hex(0x00e639),
        volume_medium: hex(0xe5e2e1),
        volume_high: hex(0xffb4ab),
        tab_active_fg: hex(0x313030),
        tab_active_bg: hex(0xc8c6c5),
        tab_inactive_fg: hex(0x8e9192),
    }
}

/// Catppuccin Mocha
#[allow(dead_code)]
fn catppuccin_mocha() -> AppTheme {
    AppTheme {
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
    }
}

/// Catppuccin Macchiato
#[allow(dead_code)]
fn catppuccin_macchiato() -> AppTheme {
    AppTheme {
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
    }
}

/// Catppuccin Frappe
#[allow(dead_code)]
fn catppuccin_frappe() -> AppTheme {
    AppTheme {
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
    }
}

/// Catppuccin Latte (light)
#[allow(dead_code)]
fn catppuccin_latte() -> AppTheme {
    AppTheme {
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
    }
}

/// Tokyonight Night
#[allow(dead_code)]
fn tokyonight_night() -> AppTheme {
    AppTheme {
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
    }
}

/// Gruvbox Dark
#[allow(dead_code)]
fn gruvbox_dark() -> AppTheme {
    AppTheme {
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
    }
}

/// Ayu Dark
#[allow(dead_code)]
fn ayu_dark() -> AppTheme {
    AppTheme {
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
    }
}
