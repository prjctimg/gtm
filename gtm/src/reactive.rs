// Copyright (c) 2026 - present
// Author: prjctimg <prjctimg@outlook.com>
// Reactive theming: derive UI accent colors from the current track's cover
// art.  Palette extraction uses median-cut (color-thief) over a downscaled
// copy of the artwork; the dominant color becomes the accent and a
// hue-distant companion becomes the secondary accent.
//
// This is free software released under the GPL-3.0 license.

use crate::theme::{blend_colors, AppTheme};
use ratatui::style::Color;

/// Colors extracted from cover art, ready to be blended into an [`AppTheme`].
#[derive(Debug, Clone, Copy)]
pub struct ReactivePalette {
    /// Dominant color: drives `accent`, active borders, selection fill.
    pub primary: [u8; 3],
    /// Most hue-distant palette entry with meaningful population.
    pub secondary: [u8; 3],
    /// Runner-up used for `tertiary_accent`.
    pub tertiary: [u8; 3],
}

/// Extract a [`ReactivePalette`] from encoded image bytes (any format
/// supported by the `image` crate).  Returns `None` when decoding fails or
/// the artwork yields no usable colors.
pub fn extract_palette(bytes: &[u8]) -> Option<ReactivePalette> {
    let img = image::load_from_memory(bytes).ok()?;
    let small = img.resize(64, 64, image::imageops::FilterType::Triangle);
    let rgba = small.to_rgba8();
    let (w, h) = (rgba.width() as usize, rgba.height() as usize);

    // color-thief expects packed samples and rejects fully transparent or
    // single-color images; drop near-transparent pixels up front.
    let mut pixels = Vec::with_capacity(w * h * 3);
    for px in rgba.pixels() {
        if px.0[3] > 32 {
            pixels.extend_from_slice(&px.0[..3]);
        }
    }
    if pixels.len() < 3 {
        return None;
    }

    let palette = match color_thief::get_palette(&pixels, color_thief::ColorFormat::Rgb, 10, 8) {
        Ok(p) if !p.is_empty() => p,
        _ => return None,
    };

    let rgb = |c: &color_thief::Color| [c.r, c.g, c.b];
    let primary = rgb(&palette[0]);

    // Secondary: maximize hue distance from the primary while keeping at
    // least ~15% of its saturation so muddy grays don't hijack accents.
    let primary_hsv = rgb_to_hsv(primary);
    let mut secondary = None;
    let mut tertiary = None;
    let mut best_score = -1.0f32;
    for c in palette.iter().skip(1) {
        let v = rgb(c);
        let hsv = rgb_to_hsv(v);
        let hue_gap = hue_distance(primary_hsv[0], hsv[0]);
        // Score rewards hue separation and chroma; population order is the
        // tie-breaker because color-thief emits entries by dominance.
        let score = hue_gap + 2.0 * hsv[1];
        if score > best_score && hsv[1] > 0.12 {
            best_score = score;
            tertiary = secondary.take();
            secondary = Some(v);
        } else if tertiary.is_none() {
            tertiary = Some(v);
        }
    }

    let fallback_mix = |c: [u8; 3], t: f32| -> [u8; 3] {
        let shifted = hue_shift(c, 0.33);
        [
            (c[0] as f32 * (1.0 - t) + shifted[0] as f32 * t) as u8,
            (c[1] as f32 * (1.0 - t) + shifted[1] as f32 * t) as u8,
            (c[2] as f32 * (1.0 - t) + shifted[2] as f32 * t) as u8,
        ]
    };
    let secondary = secondary.unwrap_or_else(|| fallback_mix(primary, 1.0));
    let tertiary = tertiary.unwrap_or_else(|| fallback_mix(secondary, 1.0));

    Some(ReactivePalette {
        primary,
        secondary,
        tertiary,
    })
}

/// Blend a reactive palette into a copy of `base`.  Backgrounds receive only
/// a faint wash of the dominant color so readability is preserved on both
/// dark and light themes; accents take the artwork colors nearly verbatim.
pub fn derive_theme(base: &AppTheme, pal: &ReactivePalette, light: bool) -> AppTheme {
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
    // Ambient wash: keep it subtle so text contrast is untouched.
    t.bg = blend_colors(base.bg, primary_raw, 0.05);
    t.pane_bg = blend_colors(base.pane_bg, primary_raw, 0.04);
    t.elevated_bg = blend_colors(base.elevated_bg, primary_raw, 0.08);
    t.picker_bg = blend_colors(base.picker_bg, primary_raw, 0.08);
    t.monochromatic = false;
    t
}

fn luminance(c: &[u8; 3]) -> f32 {
    0.299 * c[0] as f32 + 0.587 * c[1] as f32 + 0.114 * c[2] as f32
}
fn rgb_to_hsv(c: [u8; 3]) -> [f32; 3] {
    let r = c[0] as f32 / 255.0;
    let g = c[1] as f32 / 255.0;
    let b = c[2] as f32 / 255.0;
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let d = max - min;
    let h = if d <= 0.0 {
        0.0
    } else if (max - r).abs() < f32::EPSILON {
        ((g - b) / d).rem_euclid(6.0)
    } else if (max - g).abs() < f32::EPSILON {
        (b - r) / d + 2.0
    } else {
        (r - g) / d + 4.0
    } * 60.0;
    let s = if max <= 0.0 { 0.0 } else { d / max };
    [h, s, max]
}

/// Circular hue distance in degrees, range `[0, 180]`.
fn hue_distance(a: f32, b: f32) -> f32 {
    let d = (a - b).abs() % 360.0;
    if d > 180.0 {
        360.0 - d
    } else {
        d
    }
}

fn hue_shift(c: [u8; 3], turns: f32) -> [u8; 3] {
    let [h, s, v] = rgb_to_hsv(c);
    hsv_to_rgb(
        (h + 360.0 * turns) % 360.0,
        s.max(0.25),
        (v * 1.15).min(1.0),
    )
}

fn hsv_to_rgb(h: f32, s: f32, v: f32) -> [u8; 3] {
    let c = v * s;
    let x = c * (1.0 - ((h / 60.0) % 2.0 - 1.0).abs());
    let m = v - c;
    let (r, g, b) = match h as u32 {
        0..=59 => (c, x, 0.0),
        60..=119 => (x, c, 0.0),
        120..=179 => (0.0, c, x),
        180..=239 => (0.0, x, c),
        240..=299 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    [
        ((r + m) * 255.0) as u8,
        ((g + m) * 255.0) as u8,
        ((b + m) * 255.0) as u8,
    ]
}
