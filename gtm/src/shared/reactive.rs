// Copyright (c) 2026
// Author: prjctimg <prjctimg@outlook.com>
// Palette extraction from cover art, shared by the daemon and the TUI
//
// This is free software released under the GPL-3.0 license.

//! Colors read out of a piece of artwork.
//!
//! Extraction is median-cut (`color-thief`) over a downscaled copy: the
//! dominant color becomes the primary, and the most hue-distant entry with
//! real chroma becomes the secondary so an accent never lands on the same hue
//! as the primary.
//!
//! This lives in `shared` rather than beside `derive_theme` because the daemon
//! is the one that extracts — it already holds the cover bytes for the current
//! track — and the TUI renders whatever palette arrives. `color-thief` reaches
//! the daemon through the `gtm` dependency, so it is not a second copy.

/// Colors extracted from cover art.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReactivePalette {
    /// Dominant color: drives `accent`, active borders, selection fill.
    pub primary: [u8; 3],
    /// Most hue-distant palette entry with meaningful population.
    pub secondary: [u8; 3],
    /// Runner-up, used for the third accent.
    pub tertiary: [u8; 3],
}

/// Extract a palette from encoded image bytes in any format the `image` crate
/// reads. `None` when decoding fails or the artwork yields no usable colors.
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

    // A near-monochrome cover yields no hue-distant entry, so the missing
    // accents are the primary rotated a third of a turn rather than grey.
    let fallback = |c: [u8; 3]| {
        let shifted = hue_shift(c, 0.33);
        [
            ((c[0] as f32 + shifted[0] as f32) / 2.0) as u8,
            ((c[1] as f32 + shifted[1] as f32) / 2.0) as u8,
            ((c[2] as f32 + shifted[2] as f32) / 2.0) as u8,
        ]
    };
    let secondary = secondary.unwrap_or_else(|| fallback(primary));
    let tertiary = tertiary.unwrap_or_else(|| fallback(secondary));

    Some(ReactivePalette {
        primary,
        secondary,
        tertiary,
    })
}

/// `#rrggbb` for one channel triple, the form the palette crosses the wire in.
pub fn to_hex(c: [u8; 3]) -> String {
    format!("#{:02x}{:02x}{:02x}", c[0], c[1], c[2])
}

/// The inverse of [`to_hex`]. `None` for anything that is not six hex digits, so
/// an empty or malformed field reads as "no palette" rather than as black.
pub fn from_hex(s: &str) -> Option<[u8; 3]> {
    let h = s.trim().strip_prefix('#')?;
    if h.len() != 6 {
        return None;
    }
    let mut out = [0u8; 3];
    for (i, slot) in out.iter_mut().enumerate() {
        *slot = u8::from_str_radix(&h[i * 2..i * 2 + 2], 16).ok()?;
    }
    Some(out)
}

/// Perceived luminance, 0–255.
pub fn luminance(c: &[u8; 3]) -> f32 {
    0.299 * c[0] as f32 + 0.587 * c[1] as f32 + 0.114 * c[2] as f32
}

/// RGB to HSV: hue in degrees, saturation and value in `0.0..=1.0`.
pub fn rgb_to_hsv(c: [u8; 3]) -> [f32; 3] {
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

/// HSV to RGB. Hue in degrees.
pub fn hsv_to_rgb(h: f32, s: f32, v: f32) -> [u8; 3] {
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

/// Circular hue distance in degrees, range `[0, 180]`.
pub fn hue_distance(a: f32, b: f32) -> f32 {
    let d = (a - b).abs() % 360.0;
    if d > 180.0 { 360.0 - d } else { d }
}

/// The same color rotated by `turns` of the hue circle, lifted in value so a
/// dark input stays visible as an accent.
pub fn hue_shift(c: [u8; 3], turns: f32) -> [u8; 3] {
    let [h, s, v] = rgb_to_hsv(c);
    hsv_to_rgb(
        (h + 360.0 * turns) % 360.0,
        s.max(0.25),
        (v * 1.15).min(1.0),
    )
}
