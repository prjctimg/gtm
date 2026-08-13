use ratatui::style::Color;

/// Central theme — all UI colors flow through here.
pub struct AppTheme {
    // Backgrounds
    pub bg: Color,
    pub overlay_bg: Color,
    pub tab_bar_bg: Color,
    pub footer_bg: Color,

    // Foregrounds
    pub fg: Color,
    pub fg_dim: Color,
    pub fg_bright: Color,

    // Accent
    pub accent: Color,
    pub accent_bg: Color,
    pub error: Color,
    pub warning: Color,
    pub success: Color,
    pub info: Color,

    // Selection
    pub selection_fg: Color,
    pub selection_bg: Color,

    // Borders
    pub border: Color,
    pub border_active: Color,

    // Volume level
    pub volume_low: Color,
    pub volume_medium: Color,
    pub volume_high: Color,

    // Progress bar
    pub progress_filled: Color,
    pub progress_empty: Color,
    pub progress_head: Color,

    // Tab bar
    pub tab_active_fg: Color,
    pub tab_active_bg: Color,
    pub tab_inactive_fg: Color,
}

impl AppTheme {
    pub fn default() -> Self {
        catppuccin_mocha()
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

/// Catppuccin Mocha
fn catppuccin_mocha() -> AppTheme {
    AppTheme {
        bg: hex(0x1e1e2e),
        overlay_bg: hex(0x1e1e2e),
        tab_bar_bg: hex(0x181825),
        footer_bg: hex(0x181825),
        fg: hex(0xcdd6f4),
        fg_dim: hex(0x6c7086),
        fg_bright: hex(0xffffff),
        accent: hex(0x89b4fa),
        accent_bg: hex(0x45475a),
        error: hex(0xf38ba8),
        warning: hex(0xfab387),
        success: hex(0xa6e3a1),
        info: hex(0x74c7ec),
        selection_fg: hex(0x1e1e2e),
        selection_bg: hex(0x89b4fa),
        border: hex(0x585b70),
        border_active: hex(0x89b4fa),
        volume_low: hex(0xa6e3a1),
        volume_medium: hex(0xfab387),
        volume_high: hex(0xf38ba8),
        progress_filled: hex(0x89b4fa),
        progress_empty: hex(0x45475a),
        progress_head: hex(0xf5c2e7),
        tab_active_fg: hex(0x1e1e2e),
        tab_active_bg: hex(0x89b4fa),
        tab_inactive_fg: hex(0x89b4fa),
    }
}

/// Catppuccin Macchiato
#[allow(dead_code)]
fn catppuccin_macchiato() -> AppTheme {
    AppTheme {
        bg: hex(0x24273a),
        overlay_bg: hex(0x24273a),
        tab_bar_bg: hex(0x1e2030),
        footer_bg: hex(0x1e2030),
        fg: hex(0xcad3f5),
        fg_dim: hex(0x6e738d),
        fg_bright: hex(0xffffff),
        accent: hex(0x8aadf4),
        accent_bg: hex(0x494d64),
        error: hex(0xed8796),
        warning: hex(0xf5a97f),
        success: hex(0xa6da95),
        info: hex(0x7dc4e4),
        selection_fg: hex(0x24273a),
        selection_bg: hex(0x8aadf4),
        border: hex(0x5b6078),
        border_active: hex(0x8aadf4),
        volume_low: hex(0xa6da95),
        volume_medium: hex(0xf5a97f),
        volume_high: hex(0xed8796),
        progress_filled: hex(0x8aadf4),
        progress_empty: hex(0x494d64),
        progress_head: hex(0xf4dbd6),
        tab_active_fg: hex(0x24273a),
        tab_active_bg: hex(0x8aadf4),
        tab_inactive_fg: hex(0x8aadf4),
    }
}

/// Catppuccin Frappe
#[allow(dead_code)]
fn catppuccin_frappe() -> AppTheme {
    AppTheme {
        bg: hex(0x303446),
        overlay_bg: hex(0x303446),
        tab_bar_bg: hex(0x292c3c),
        footer_bg: hex(0x292c3c),
        fg: hex(0xc6d0f5),
        fg_dim: hex(0x737994),
        fg_bright: hex(0xffffff),
        accent: hex(0x8caaee),
        accent_bg: hex(0x51576d),
        error: hex(0xe78284),
        warning: hex(0xef9f76),
        success: hex(0xa6d189),
        info: hex(0x85c1dc),
        selection_fg: hex(0x303446),
        selection_bg: hex(0x8caaee),
        border: hex(0x626880),
        border_active: hex(0x8caaee),
        volume_low: hex(0xa6d189),
        volume_medium: hex(0xef9f76),
        volume_high: hex(0xe78284),
        progress_filled: hex(0x8caaee),
        progress_empty: hex(0x51576d),
        progress_head: hex(0xf4b8e4),
        tab_active_fg: hex(0x303446),
        tab_active_bg: hex(0x8caaee),
        tab_inactive_fg: hex(0x8caaee),
    }
}

/// Catppuccin Latte (light)
#[allow(dead_code)]
fn catppuccin_latte() -> AppTheme {
    AppTheme {
        bg: hex(0xeff1f5),
        overlay_bg: hex(0xeff1f5),
        tab_bar_bg: hex(0xe6e9ef),
        footer_bg: hex(0xe6e9ef),
        fg: hex(0x4c4f69),
        fg_dim: hex(0x9ca0b0),
        fg_bright: hex(0x000000),
        accent: hex(0x1e66f5),
        accent_bg: hex(0xccd0da),
        error: hex(0xd20f39),
        warning: hex(0xfe640b),
        success: hex(0x40a02b),
        info: hex(0x04a5e5),
        selection_fg: hex(0xeff1f5),
        selection_bg: hex(0x1e66f5),
        border: hex(0xbcc0cc),
        border_active: hex(0x1e66f5),
        volume_low: hex(0x40a02b),
        volume_medium: hex(0xfe640b),
        volume_high: hex(0xd20f39),
        progress_filled: hex(0x1e66f5),
        progress_empty: hex(0xccd0da),
        progress_head: hex(0xea76cb),
        tab_active_fg: hex(0xeff1f5),
        tab_active_bg: hex(0x1e66f5),
        tab_inactive_fg: hex(0x1e66f5),
    }
}

/// Tokyonight Night
#[allow(dead_code)]
fn tokyonight_night() -> AppTheme {
    AppTheme {
        bg: hex(0x1a1b26),
        overlay_bg: hex(0x1a1b26),
        tab_bar_bg: hex(0x16161e),
        footer_bg: hex(0x16161e),
        fg: hex(0xa9b1d6),
        fg_dim: hex(0x565f89),
        fg_bright: hex(0xc0caf5),
        accent: hex(0x7aa2f7),
        accent_bg: hex(0x3b4261),
        error: hex(0xf7768e),
        warning: hex(0xff9e64),
        success: hex(0x9ece6a),
        info: hex(0x2ac3de),
        selection_fg: hex(0x1a1b26),
        selection_bg: hex(0x7aa2f7),
        border: hex(0x565f89),
        border_active: hex(0x7aa2f7),
        volume_low: hex(0x9ece6a),
        volume_medium: hex(0xff9e64),
        volume_high: hex(0xf7768e),
        progress_filled: hex(0x7aa2f7),
        progress_empty: hex(0x3b4261),
        progress_head: hex(0xbb9af7),
        tab_active_fg: hex(0x1a1b26),
        tab_active_bg: hex(0x7aa2f7),
        tab_inactive_fg: hex(0x7aa2f7),
    }
}

/// Gruvbox Dark
#[allow(dead_code)]
fn gruvbox_dark() -> AppTheme {
    AppTheme {
        bg: hex(0x282828),
        overlay_bg: hex(0x282828),
        tab_bar_bg: hex(0x1d2021),
        footer_bg: hex(0x1d2021),
        fg: hex(0xebdbb2),
        fg_dim: hex(0x928374),
        fg_bright: hex(0xebdbb2),
        accent: hex(0x83a598),
        accent_bg: hex(0x504945),
        error: hex(0xfb4934),
        warning: hex(0xfe8019),
        success: hex(0xb8bb26),
        info: hex(0x83a598),
        selection_fg: hex(0x282828),
        selection_bg: hex(0x83a598),
        border: hex(0x665c54),
        border_active: hex(0x83a598),
        volume_low: hex(0xb8bb26),
        volume_medium: hex(0xfe8019),
        volume_high: hex(0xfb4934),
        progress_filled: hex(0x83a598),
        progress_empty: hex(0x504945),
        progress_head: hex(0xd3869b),
        tab_active_fg: hex(0x282828),
        tab_active_bg: hex(0x83a598),
        tab_inactive_fg: hex(0x83a598),
    }
}

/// Ayu Dark
#[allow(dead_code)]
fn ayu_dark() -> AppTheme {
    AppTheme {
        bg: hex(0x0a0e14),
        overlay_bg: hex(0x0a0e14),
        tab_bar_bg: hex(0x000000),
        footer_bg: hex(0x000000),
        fg: hex(0xb3b1ad),
        fg_dim: hex(0x6e6c66),
        fg_bright: hex(0xffffff),
        accent: hex(0xff8f40),
        accent_bg: hex(0x333333),
        error: hex(0xff3333),
        warning: hex(0xff8f40),
        success: hex(0x7ec8a0),
        info: hex(0x5ccfe6),
        selection_fg: hex(0x0a0e14),
        selection_bg: hex(0xff8f40),
        border: hex(0x3d424d),
        border_active: hex(0xff8f40),
        volume_low: hex(0x7ec8a0),
        volume_medium: hex(0xff8f40),
        volume_high: hex(0xff3333),
        progress_filled: hex(0xff8f40),
        progress_empty: hex(0x333333),
        progress_head: hex(0xd2a6ff),
        tab_active_fg: hex(0x0a0e14),
        tab_active_bg: hex(0xff8f40),
        tab_inactive_fg: hex(0xff8f40),
    }
}
