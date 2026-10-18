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

use crate::shared::theme::{derive, role_rgb_static};
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
    // The ramp is not computed here any more. `shared::theme::derive` is the
    // one derivation in this workspace and it is also what the daemon sends to
    // every other client, so the TUI and an editor cannot end up with different
    // colours from the same artwork.
    let roles = derive(pal, !light);
    let role = |name: &str| -> Color {
        match role_rgb_static(&roles, name) {
            Some([r, g, b]) => Color::Rgb(r, g, b),
            None => base.fg,
        }
    };

    let mut t = *base;
    let rgb = |c: [u8; 3]| Color::Rgb(c[0], c[1], c[2]);

    let primary = role("accent");
    t.accent = primary;
    t.secondary_accent = role("accent2");
    t.tertiary_accent = role("accent3");
    t.border_active = blend_colors(base.border_active, primary, 0.55);
    t.sidebar_active_border = blend_colors(base.sidebar_active_border, primary, 0.7);
    t.notification_border = blend_colors(base.notification_border, primary, 0.5);
    // The selection fill is the raw artwork colour rather than the adjusted
    // accent: the derived accent is pulled toward legible, which is right for
    // a glyph and wrong for a block of background behind one.
    t.selection_bg = blend_colors(base.selection_bg, rgb(pal.primary), 0.4);

    // Ambient wash, the TUI's own: pull the background toward the artwork so the
    // whole surface reacts to the current cover. The strength is user-tunable
    // and 0 disables it. Editor clients have no equivalent knob, which is why
    // `derive` does not take one and this layer does.
    let wash = intensity.clamp(0.0, 1.0) as f64;
    let unified_bg = blend_colors(base.bg, rgb(pal.primary), wash);
    t.bg = unified_bg;
    t.pane_bg = unified_bg;
    t.elevated_bg = unified_bg;
    t.picker_bg = unified_bg;

    // The dim foreground comes from the derivation, which already moved it off
    // the background it will be drawn on -- the wash shifts `bg` again after
    // `derive` ran, so the contrast is checked here rather than assumed.
    t.fg_dim = match (role("fg_dim"), unified_bg) {
        (Color::Rgb(r, g, b), Color::Rgb(bg_r, bg_g, bg_b)) => {
            adjust_for_contrast(Color::Rgb(r, g, b), luminance(&[bg_r, bg_g, bg_b]))
        }
        (dim, _) => dim,
    };
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
