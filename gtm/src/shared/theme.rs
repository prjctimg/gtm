// Copyright (c) 2026
// Author: prjctimg <prjctimg@outlook.com>
//
// This is free software released under the GPL-3.0 license.

//! Reactive tokens: the derived colours, named by what they are *for*.
//!
//! `REACTIVE_THEME.md` moved palette *extraction* into the daemon.
//! `REACTIVE_TOKENS.md` — this — moves *derivation* as well, so a client maps
//! role names to whatever its own surface needs instead of running the same ramp
//! maths in a third language and arriving at a third answer.
//!
//! No ratatui here, deliberately. `derive` is a pure function from a palette
//! and a polarity to a set of named hex colours, which is what makes it
//! shareable; the TUI's `derive_theme` is the one caller that happens to speak
//! `AppTheme`.

use std::collections::BTreeMap;

use crate::shared::reactive::{ReactivePalette, from_hex, luminance, to_hex};

/// Token-set version.
///
/// Bumped when a role is renamed or removed. A client compares it against what
/// it was built for and falls back if it is older. Roles are never added without
/// a bump, because a client that receives a name it does not know has to decide
/// what to do with it, and "ignore" is the only answer that is always safe.
pub const THEME_VER: u8 = 1;

/// A resolved ramp: every colour a client needs, by role.
///
/// `BTreeMap` rather than `HashMap` so the serialised order is stable. A map
/// that reorders itself between two runs produces different frames of the same
/// logical value, which is exactly the kind of thing that makes a diff useless.
pub type Roles = BTreeMap<&'static str, String>;

/// Every role this build produces, in the groups §5.1 of the spec gives.
///
/// Asserted against `derive`'s output by a test, so a role added to one and not
/// the other cannot ship.
pub const ROLE_NAMES: &[&str] = &[
    // Surface
    "bg0",
    "bg1",
    "bg2",
    "bg3",
    "bg_flat", // Foreground
    "fg",
    "fg_dim",
    "fg_bright", // Accent
    "accent",
    "accent2",
    "accent3", // Line
    "line",
    "line_focus", // Selection
    "sel_bg",
    "sel_fg", // Cursor
    "cursor",
    "cursor_line",
    "cursor_line_nr", // Search
    "search_bg",
    "search_fg", // Syntax
    "comment",
    "string",
    "keyword",
    "function",
    "type",
    "variable",
    "constant",
    "number",
    "boolean",
    "operator",
    "punctuation",
    "identifier", // Semantic
    "error",
    "warning",
    "info",
    "hint",
    "success", // Diff
    "diff_add",
    "diff_delete",
    "diff_change",
    "diff_text", // Misc
    "heading",
    "link_url",
];

/// One polarity's base surface, before the artwork is mixed in.
///
/// Fixed rather than derived: these are the greys a theme is made of, and a
/// client's own surface defaults have to start from the same numbers or two
/// clients disagree about what "dark" means.
const DARK_SURFACE: [([u8; 3], f32); 4] = [
    ([0x0d, 0x0d, 0x12], 0.0),  // bg0
    ([0x15, 0x15, 0x1c], 0.35), // bg1
    ([0x1d, 0x1d, 0x26], 0.6),  // bg2
    ([0x26, 0x26, 0x31], 0.85), // bg3
];

const LIGHT_SURFACE: [([u8; 3], f32); 4] = [
    ([0xf6, 0xf4, 0xf1], 0.0),  // bg0
    ([0xec, 0xe9, 0xe5], 0.35), // bg1
    ([0xe2, 0xde, 0xd9], 0.6),  // bg2
    ([0xd8, 0xd3, 0xcd], 0.85), // bg3
];

/// Build the full role set for one polarity.
///
/// Pure: no globals, no I/O, no ratatui. The daemon calls it twice per palette
/// — once per polarity — and the client calls it too, which is the point: the
/// ramp is computed once per build rather than once per client.
///
/// The two decisions here that are judgement rather than arithmetic:
///
/// - **Accents are pulled away from the luminance extremes.** An artwork's
///   dominant colour is frequently near-white or near-black, and a near-white
///   accent on a dark surface is invisible and a near-black one on a light
///   surface is worse. The pull is bounded, so a mid-tone artwork is left alone.
/// - **Syntax and semantic roles are fixed-hue, not artwork-driven.** They are
///   diagnostics, and a diagnostic that changes colour with the album art is one
///   the eye cannot learn. They are tinted toward the artwork slightly and no
///   further. This is a deliberate refusal of "more reactive", stated in §9 of
///   the spec.
pub fn derive(pal: &ReactivePalette, dark: bool) -> Roles {
    let surface = if dark { &DARK_SURFACE } else { &LIGHT_SURFACE };
    let accent = adjust(pal.primary, dark);
    let accent2 = adjust(pal.secondary, dark);
    let accent3 = adjust(pal.tertiary, dark);
    let bg0 = mix(surface[0].0, pal.primary, if dark { 0.06 } else { 0.05 });

    let fg = if dark {
        [0xe6, 0xe6, 0xee]
    } else {
        [0x1a, 0x1a, 0x1f]
    };
    let fg_dim = contrast(fg, bg0, 0.55);
    let fg_bright = if dark {
        [0xff, 0xff, 0xff]
    } else {
        [0x00, 0x00, 0x00]
    };
    let line = mix(surface[1].0, pal.primary, 0.25);
    let line_focus = mix(line, accent, 0.6);

    let sel_bg = mix(surface[2].0, pal.primary, 0.3);
    let sel_fg = if luminance(&sel_bg) > 140.0 {
        [0x10, 0x10, 0x14]
    } else {
        [0xf2, 0xf2, 0xf6]
    };
    let cursor_line = mix(bg0, pal.primary, if dark { 0.10 } else { 0.08 });
    let cursor_line_nr = mix(surface[0].0, pal.primary, if dark { 0.06 } else { 0.05 });

    // Fixed diagnostic hues, nudged toward the artwork only enough that the
    // palette does not read as foreign next to them.
    let tint = |base: [u8; 3]| -> [u8; 3] { mix(base, pal.primary, 0.12) };

    let mut r: Roles = Roles::new();
    let mut set = |k: &'static str, v: [u8; 3]| {
        r.insert(k, to_hex(v));
    };

    // Surface
    set("bg0", bg0);
    for (i, k) in ["bg1", "bg2", "bg3"].iter().enumerate() {
        set(
            k,
            mix(
                surface[i + 1].0,
                pal.primary,
                (surface[i + 1].1 * 0.35) as f64,
            ),
        );
    }
    set("bg_flat", surface[0].0);

    // Foreground
    set("fg", fg);
    set("fg_dim", fg_dim);
    set("fg_bright", fg_bright);

    // Accent
    set("accent", accent);
    set("accent2", accent2);
    set("accent3", accent3);

    // Line
    set("line", line);
    set("line_focus", line_focus);

    // Selection
    set("sel_bg", sel_bg);
    set("sel_fg", sel_fg);

    // Cursor
    set("cursor", accent);
    set("cursor_line", cursor_line);
    set("cursor_line_nr", cursor_line_nr);

    // Search
    set("search_bg", mix(surface[2].0, [0xff, 0xd5, 0x4f], 0.35));
    set(
        "search_fg",
        if dark {
            [0x1a, 0x1a, 0x10]
        } else {
            [0x20, 0x18, 0x00]
        },
    );

    // Syntax — fixed hues, lightly tinted.
    set(
        "comment",
        tint(if dark {
            [0x6b, 0x6b, 0x7a]
        } else {
            [0x77, 0x77, 0x82]
        }),
    );
    set(
        "string",
        tint(if dark {
            [0x9c, 0xd0, 0x8a]
        } else {
            [0x2f, 0x7a, 0x2a]
        }),
    );
    set(
        "keyword",
        tint(if dark {
            [0xc9, 0x9b, 0xe8]
        } else {
            [0x7b, 0x2f, 0xa8]
        }),
    );
    set(
        "function",
        tint(if dark {
            [0x82, 0xc8, 0xf0]
        } else {
            [0x1a, 0x62, 0xa8]
        }),
    );
    set(
        "type",
        tint(if dark {
            [0x7f, 0xd6, 0xd0]
        } else {
            [0x0d, 0x6f, 0x6a]
        }),
    );
    set(
        "variable",
        tint(if dark {
            [0xe6, 0xe6, 0xee]
        } else {
            [0x1a, 0x1a, 0x1f]
        }),
    );
    set(
        "constant",
        tint(if dark {
            [0xf0, 0xb0, 0x76]
        } else {
            [0xa8, 0x5a, 0x10]
        }),
    );
    set(
        "number",
        tint(if dark {
            [0xf0, 0xb0, 0x76]
        } else {
            [0xa8, 0x5a, 0x10]
        }),
    );
    set(
        "boolean",
        tint(if dark {
            [0xf0, 0x9a, 0xc0]
        } else {
            [0xa8, 0x1a, 0x60]
        }),
    );
    set(
        "operator",
        tint(if dark {
            [0xa8, 0xb0, 0xc0]
        } else {
            [0x50, 0x58, 0x66]
        }),
    );
    set(
        "punctuation",
        tint(if dark {
            [0x9a, 0x9a, 0xa8]
        } else {
            [0x58, 0x58, 0x64]
        }),
    );
    set(
        "identifier",
        tint(if dark {
            [0xe6, 0xe6, 0xee]
        } else {
            [0x1a, 0x1a, 0x1f]
        }),
    );

    // Semantic
    set("error", tint([0xf0, 0x6a, 0x6a]));
    set("warning", tint([0xe0, 0xa0, 0x40]));
    set("info", tint([0x70, 0xb0, 0xf0]));
    set("hint", tint([0x70, 0xd0, 0xa0]));
    set("success", tint([0x80, 0xc8, 0x70]));

    // Diff
    set(
        "diff_add",
        tint(if dark {
            [0x4a, 0x8a, 0x5a]
        } else {
            [0x2f, 0x7a, 0x3a]
        }),
    );
    set(
        "diff_delete",
        tint(if dark {
            [0x9a, 0x4a, 0x4a]
        } else {
            [0x9a, 0x2f, 0x2f]
        }),
    );
    set(
        "diff_change",
        tint(if dark {
            [0x8a, 0x74, 0x40]
        } else {
            [0x7a, 0x5a, 0x20]
        }),
    );
    set(
        "diff_text",
        tint(if dark {
            [0x6a, 0x6a, 0x9a]
        } else {
            [0x3a, 0x3a, 0x7a]
        }),
    );

    // Misc
    set(
        "heading",
        tint(if dark {
            [0x9c, 0xc0, 0xf0]
        } else {
            [0x1a, 0x4a, 0x8a]
        }),
    );
    set(
        "link_url",
        tint(if dark {
            [0x6a, 0xb0, 0xd0]
        } else {
            [0x1a, 0x5a, 0x8a]
        }),
    );

    r
}

/// The role set as it crosses the wire: both polarities, versioned.
///
/// Both at once because the pulse socket is a broadcast and per-client polarity
/// is not available without a protocol change. Sending both is the cheap fix,
/// at a real 2x payload cost the spec states in §9.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ReactiveTheme {
    /// Token-set version. See [`THEME_VER`].
    pub ver: u8,
    pub dark: BTreeMap<String, String>,
    pub light: BTreeMap<String, String>,
}

impl ReactiveTheme {
    /// Derive both polarities from one palette.
    ///
    /// `None` when the palette has nothing in it — the daemon emits an empty
    /// `reactive_palette` in that case and there is no ramp to build.
    pub fn from_palette(pal: &ReactivePalette) -> Option<Self> {
        if pal.primary == [0, 0, 0] && pal.secondary == [0, 0, 0] && pal.tertiary == [0, 0, 0] {
            return None;
        }
        let widen = |m: Roles| -> BTreeMap<String, String> {
            m.into_iter().map(|(k, v)| (k.to_string(), v)).collect()
        };
        Some(Self {
            ver: THEME_VER,
            dark: widen(derive(pal, true)),
            light: widen(derive(pal, false)),
        })
    }

    /// The roles for one polarity, or `None` if this build cannot supply them.
    ///
    /// The `None` is the whole compatibility story for a client: an unknown
    /// version, or a map missing a role it needs, and the client falls back to
    /// deriving locally or to a static palette. Silently returning a partial map
    /// would leave it rendering half-themed, which is worse than not themed.
    pub fn roles(&self, dark: bool) -> Option<&BTreeMap<String, String>> {
        if self.ver != THEME_VER {
            return None;
        }
        let m = if dark { &self.dark } else { &self.light };
        if ROLE_NAMES.iter().any(|r| !m.contains_key(*r)) {
            return None;
        }
        Some(m)
    }
}

/// Linear blend of `a` toward `b` by `t`.
fn mix(a: [u8; 3], b: [u8; 3], t: f64) -> [u8; 3] {
    let t = t.clamp(0.0, 1.0);
    let f = |x: u8, y: u8| (x as f64 + (y as f64 - x as f64) * t).round() as u8;
    [f(a[0], b[0]), f(a[1], b[1]), f(a[2], b[2])]
}

/// Pull an accent away from the luminance extremes, bounded.
///
/// An artwork's dominant colour is very often near-white or near-black. On a
/// dark surface a near-white accent is fine and a near-black one is invisible;
/// on a light surface it is the reverse. The band around the target is left
/// alone, so a mid-tone artwork is not touched at all.
fn adjust(c: [u8; 3], dark: bool) -> [u8; 3] {
    let l = luminance(&c);
    let target = if dark { 170.0f32 } else { 70.0f32 };
    if (l - target).abs() < 40.0 {
        return c;
    }
    let t = ((target - l) / 255.0).clamp(-0.35, 0.35);
    if t >= 0.0 {
        mix(c, [255, 255, 255], t as f64)
    } else {
        mix(c, [0, 0, 0], -t as f64)
    }
}

/// Move `fg` along the luminance axis until it is `amount` of the way to the
/// contrast the surface allows, in whichever direction that is.
fn contrast(fg: [u8; 3], bg: [u8; 3], amount: f32) -> [u8; 3] {
    let bg_l = luminance(&bg);
    let target = if bg_l > 128.0 {
        bg_l * (1.0 - amount)
    } else {
        // On a dark surface, "dimmer than the foreground" means closer to the
        // background, which means brighter in absolute terms.
        bg_l + (255.0 - bg_l) * amount
    };
    let l = luminance(&fg);
    if (l - target).abs() < 8.0 {
        return fg;
    }
    let t = ((target - l) / 255.0).clamp(-0.4, 0.4);
    if t >= 0.0 {
        mix(fg, [255, 255, 255], t as f64)
    } else {
        mix(fg, [0, 0, 0], -t as f64)
    }
}

/// A role value as RGB, for a caller that wants components rather than hex.
pub fn role_rgb(roles: &BTreeMap<String, String>, name: &str) -> Option<[u8; 3]> {
    roles.get(name).and_then(|h| from_hex(h))
}

/// The same lookup against a [`Roles`] map, whose keys are `&'static str`.
///
/// Two functions rather than one generic one: `BTreeMap::get` takes the map's own
/// key type, so a single signature would need `Borrow<str>`, and the two maps do
/// not share one.
pub fn role_rgb_static(roles: &Roles, name: &str) -> Option<[u8; 3]> {
    roles.get(name).and_then(|h| from_hex(h))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn palette() -> ReactivePalette {
        ReactivePalette {
            primary: [0xff, 0xff, 0xff],
            secondary: [0x10, 0x20, 0xc0],
            tertiary: [0xc0, 0x30, 0x10],
        }
    }

    /// Every declared role is produced, for both polarities.
    ///
    /// The failure this catches is the one a client cannot recover from: a role
    /// the spec promises that `derive` does not emit is a role every client has
    /// to fall back for.
    #[test]
    fn every_declared_role_is_produced() {
        for dark in [true, false] {
            let roles = derive(&palette(), dark);
            for name in ROLE_NAMES {
                assert!(roles.contains_key(name), "dark={dark}: missing role {name}");
            }
            assert_eq!(
                roles.len(),
                ROLE_NAMES.len(),
                "dark={dark}: {} roles for {} declared",
                roles.len(),
                ROLE_NAMES.len()
            );
        }
    }

    /// Every value is a parseable hex colour.
    ///
    /// A malformed hex is a silent theme failure on the other side of the wire:
    /// the client gets a string, cannot decode it, and substitutes whatever it
    /// likes. Cheap to check here, invisible otherwise.
    #[test]
    fn every_value_is_hex() {
        for dark in [true, false] {
            for (k, v) in derive(&palette(), dark) {
                assert_eq!(v.len(), 7, "{k} = {v:?} is not #rrggbb");
                assert!(v.starts_with('#'), "{k} = {v:?} has no hash");
                assert!(from_hex(&v).is_some(), "{k} = {v:?} does not parse back");
            }
        }
    }

    /// Foreground has to be readable on the surface it is drawn on.
    ///
    /// A contrast ratio below 3 is the point where text stops being legible at
    /// small sizes on a terminal. This is the property the whole ramp exists to
    /// protect, and a theme that fails it looks fine in a colour picker and is
    /// unreadable in use.
    #[test]
    fn fg_contrasts_with_the_surface() {
        for dark in [true, false] {
            let roles = derive(&palette(), dark);
            let ratio = |a: [u8; 3], b: [u8; 3]| -> f32 {
                let (la, lb) = (luminance(&a), luminance(&b));
                let (hi, lo) = if la > lb { (la, lb) } else { (lb, la) };
                (hi + 0.05) / (lo + 0.05)
            };
            let bg = role_rgb_static(&roles, "bg0").unwrap();
            for name in ["fg", "fg_bright"] {
                let fg = role_rgb_static(&roles, name).unwrap();
                let r = ratio(fg, bg);
                assert!(r >= 3.0, "dark={dark}: {name} contrast {r:.2} on bg0");
            }
            // The dim role is dim by design, so it gets a floor rather than the
            // same target as full-strength text.
            let dim = role_rgb_static(&roles, "fg_dim").unwrap();
            let r = ratio(dim, bg);
            assert!(r >= 2.0, "dark={dark}: fg_dim contrast {r:.2} on bg0");
        }
    }

    /// Accents must stay distinguishable from one another.
    ///
    /// Three accents that collapse to the same colour is not a bug the eye
    /// reports — it is a bug that makes a chart, a diff and a heading all read
    /// as the same thing.
    #[test]
    fn the_three_accents_are_distinct() {
        for dark in [true, false] {
            let roles = derive(&palette(), dark);
            let got: Vec<[u8; 3]> = ["accent", "accent2", "accent3"]
                .iter()
                .map(|k| role_rgb_static(&roles, k).unwrap())
                .collect();
            for i in 0..got.len() {
                for j in i + 1..got.len() {
                    let d: i32 = got[i]
                        .iter()
                        .zip(got[j].iter())
                        .map(|(a, b)| (*a as i32 - *b as i32).abs())
                        .sum();
                    assert!(d > 60, "dark={dark}: accent{i} and accent{j} are {d} apart");
                }
            }
        }
    }

    /// Diagnostics do not follow the artwork.
    ///
    /// This is the one place `derive` deliberately ignores `pal`, so it needs a
    /// test: an artwork that happens to be red must not turn `error` red for a
    /// different reason than it always is.
    #[test]
    fn semantic_roles_do_not_follow_the_artwork() {
        let mut red = palette();
        red.primary = [0xff, 0x00, 0x00];
        red.secondary = [0xff, 0x00, 0x00];
        red.tertiary = [0xff, 0x00, 0x00];

        for dark in [true, false] {
            let base = derive(&palette(), dark);
            let hot = derive(&red, dark);
            for name in ["error", "warning", "info", "hint", "success"] {
                assert_eq!(
                    base[name], hot[name],
                    "{name} moved with the artwork; diagnostics must not"
                );
            }
        }
    }

    /// An unknown version is refused rather than half-applied.
    #[test]
    fn a_mismatched_version_is_refused() {
        let mut t = ReactiveTheme::from_palette(&palette()).unwrap();
        t.ver = THEME_VER + 1;
        assert!(t.roles(true).is_none());
        assert!(t.roles(false).is_none());
    }

    /// A map missing a role is refused, not partially served.
    #[test]
    fn a_partial_role_set_is_refused() {
        let mut t = ReactiveTheme::from_palette(&palette()).unwrap();
        t.dark.remove("accent3");
        assert!(t.roles(true).is_none(), "dark was served without accent3");
        // The other polarity is untouched and still resolves.
        assert!(t.roles(false).is_some());
    }

    /// An empty palette is no palette.
    ///
    /// The daemon emits an empty `reactive_palette` when extraction fails, and
    /// deriving from that would produce a complete role set of black — a client
    /// would apply it and go dark, having been told nothing went wrong.
    #[test]
    fn an_empty_palette_derives_nothing() {
        let black = ReactivePalette {
            primary: [0, 0, 0],
            secondary: [0, 0, 0],
            tertiary: [0, 0, 0],
        };
        assert!(ReactiveTheme::from_palette(&black).is_none());
    }
}
