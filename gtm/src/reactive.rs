// Copyright (c) 2026
// Author: prjctimg <prjctimg@outlook.com>
// Reactive theming: derive UI accent colors from the current track's cover
// art.
//
// This is free software released under the GPL-3.0 license.

//! Blending a cover-art palette into an [`AppTheme`].
//!
//! The palette itself is the daemon's: it extracts from the bytes it already
//! holds for the current track and broadcasts it as a
//! [`DaemonEvent::ReactivePalette`](crate::shared::ipc::DaemonEvent::ReactivePalette),
//! which arrives here over the same event stream as every other track fact.
//! Extraction and its color helpers live in [`crate::shared::reactive`]; this
//! module is only the part that is ratatui-shaped.

use crate::theme::{AppTheme, blend_colors};
use ratatui::style::Color;

// The palette type and the extraction helpers moved to `shared` when the daemon
// took over extraction. Re-exported so the client's call sites are unchanged and
// so there is one extractor, not two.
pub use crate::shared::reactive::{
    ReactivePalette, extract_palette, from_hex, hsv_to_rgb, hue_distance, hue_shift, luminance,
    rgb_to_hsv, to_hex,
};

/// Blend a reactive palette into a copy of `base`.  Backgrounds receive only
/// a faint wash of the dominant color so readability is preserved on both
/// dark and light themes; accents take the artwork colors nearly verbatim.
/// `intensity` (0.0–1.0) scales the ambient background wash — 0 disables it.
pub fn derive_theme(
    base: &AppTheme,
    pal: &ReactivePalette,
    light: bool,
    intensity: f32,
) -> AppTheme {
    let mut t = *base;
    let rgb = |c: [u8; 3]| Color::Rgb(c[0], c[1], c[2]);

    // Accents read best when nudged away from extreme luminance extremes:
    // darken near-white primaries on light themes, brighten near-black ones
    // on dark themes.
    let adjust = |c: [u8; 3]| -> Color {
        let l = luminance(&c);
        let target = if light { 70.0f32 } else { 170.0 };
        if (l - target).abs() < 40.0 {
            return rgb(c);
        }
        let t = ((target - l) / 255.0).clamp(-0.35, 0.35);
        if t >= 0.0 {
            blend_colors(rgb(c), Color::Rgb(255, 255, 255), t as f64)
        } else {
            blend_colors(rgb(c), Color::Rgb(0, 0, 0), (-t) as f64)
        }
    };

    let primary = adjust(pal.primary);
    let secondary = adjust(pal.secondary);
    let tertiary = adjust(pal.tertiary);

    t.accent = primary;
    t.secondary_accent = secondary;
    t.tertiary_accent = tertiary;
    t.border_active = blend_colors(base.border_active, primary, 0.55);
    t.sidebar_active_border = blend_colors(base.sidebar_active_border, primary, 0.7);
    t.notification_border = blend_colors(base.notification_border, primary, 0.5);
    let primary_raw = rgb(pal.primary);
    t.selection_bg = blend_colors(base.selection_bg, primary_raw, 0.4);
    // Ambient wash: pull the background toward the artwork so the whole
    // surface reacts to the current cover (Spotify-style). The strength is
    // user-tunable; 0 disables the wash entirely.
    let unified_bg = blend_colors(base.bg, primary_raw, intensity.clamp(0.0, 1.0) as f64);
    t.bg = unified_bg;
    t.pane_bg = unified_bg;
    t.elevated_bg = unified_bg;
    t.picker_bg = unified_bg;
    // Ensure fg_dim contrasts with the reactive bg for readability.
    // Target luminance difference of at least 60 from bg luminance.
    let bg_lum = luminance(&pal.primary);
    let target_dim_lum = if bg_lum > 128.0 {
        bg_lum - 60.0
    } else {
        bg_lum + 60.0
    };
    t.fg_dim = adjust_for_contrast(base.fg_dim, target_dim_lum);
    t.monochromatic = false;
    t
}

fn adjust_for_contrast(base_fg_dim: Color, target_lum: f32) -> Color {
    match base_fg_dim {
        Color::Rgb(r, g, b) => {
            let current_lum = 0.299 * r as f32 + 0.587 * g as f32 + 0.114 * b as f32;
            let diff = (target_lum - current_lum).abs();
            if diff >= 60.0 {
                base_fg_dim
            } else {
                // Adjust toward target luminance
                let factor = (target_lum - current_lum) / 255.0;
                let t = factor.clamp(-0.4, 0.4);
                if t >= 0.0 {
                    blend_colors(base_fg_dim, Color::Rgb(255, 255, 255), t as f64)
                } else {
                    blend_colors(base_fg_dim, Color::Rgb(0, 0, 0), (-t) as f64)
                }
            }
        }
        _ => base_fg_dim,
    }
}
