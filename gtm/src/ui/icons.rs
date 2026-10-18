// Copyright (c) 2026
// Author: prjctimg <prjctimg@outlook.com>
// Nerd-font and ASCII glyph tables
//
//
// This is free software released under the GPL-3.0 license.

use crate::ui::*;

// Six rows, in `LIB_CATEGORIES` order. The four views that follow Radio in the
// picker — Most Played, Recently Played, Recently Added, Folders — are groups
// (of Playlists and of Library), so they have no icon of their own and are not
// in this table: the renderer reads `icons.get(i)` positionally, so an entry
// per row that no row can reach would put the survivors on the wrong glyphs.
pub(crate) const LIBRARY_ICONS_NERD: &[&str] = &[
    "\u{f001}",  // Library: nf-md-music_note
    "\u{f03a}",  // Playlists: nf-md-playlist_music
    "\u{f04c7}", // Spotify
    "\u{f0439}", // Radio: nf-md-radio (official MDI)
    "\u{f0535}", // Top Charts: nf-md-trending_up (official MDI)
    "\u{f0994}", // Podcasts: nf-md-podcast (same glyph as provider_icon)
];

pub(crate) const LIBRARY_ICONS_ASCII: &[&str] = &["♫", "≡", "☊", "◉", "#", "◉"];

/// Whether the terminal has the nerd-font glyphs, read once.
///
/// The footer asks this fourteen times per frame and the environment cannot
/// change under a running process, so it was fourteen `env::var` calls and
/// fourteen `String` allocations every frame forever.
static NERD_FONTS: std::sync::LazyLock<bool> = std::sync::LazyLock::new(
    || !matches!(std::env::var("GTM_NERD_FONTS"), Ok(v) if v == "0" || v == "false" || v == "no"),
);

pub(crate) fn use_nerd_fonts() -> bool {
    *NERD_FONTS
}

pub(crate) fn provider_icon(name: &str) -> Option<&'static str> {
    match name {
        "Spotify" => Some("\u{f04c7}"),   // nf-md-spotify
        "YouTube" => Some("\u{f16a}"),    // nf-fa-youtube
        "Podcast" => Some("\u{f0994}"),   // nf-md-podcast
        "Radio" => Some("\u{f0439}"),     // nf-md-radio
        "SoundCloud" => Some("\u{f1be}"), // nf-fa-soundcloud
        "Bandcamp" => Some("\u{f2d5}"),   // nf-fa-bandcamp
        "Mixcloud" => Some("\u{f289}"),   // nf-fa-mixcloud
        "Twitch" => Some("\u{f1e8}"),     // nf-fa-twitch
        "Last.fm" => Some("\u{f001}"),    // nf-md-music (no brand glyph in font)
        "Discord" => Some("\u{f066f}"),   // nf-md-discord (U+F075E is volume-minus)
        "Local" => Some("\u{f0a0}"),      // nf-fa-hdd
        _ => None,
    }
}

pub(crate) fn cover_provider_label(v: &str) -> String {
    match v.trim().to_ascii_lowercase().as_str() {
        "deezer" => "Deezer".into(),
        "musicbrainz" | "mb" => "MusicBrainz".into(),
        "spotify" => "Spotify".into(),
        _ => "Auto".into(),
    }
}

pub(crate) fn theme_mode_label(v: &str) -> String {
    match v.trim().to_ascii_lowercase().as_str() {
        "dark" => "Dark".into(),
        "light" => "Light".into(),
        "manual" => "Manual".into(),
        _ => "Auto".into(),
    }
}

pub(crate) const SETTINGS_ICONS_NERD: &[&str] = &["\u{f04b}", "\u{f013}", "\u{f04c7}"];

pub(crate) const SETTINGS_ICONS_ASCII: &[&str] = &["▶", "⚙", "★"];

// No YouTube category: it held two read-only rows (cookie source, JS runtime),
// one row that toggled between two hard-coded paths, and one that said
// "read-only" and did nothing. Everything real lives on the command palette.
pub(crate) const SETTINGS_CATEGORIES: &[&str] = &["Playback", "System", "Spotify"];

pub(crate) fn service_icon_glyph(icon_style: &str, service: &str) -> &'static str {
    if icon_style == "mdi" {
        return provider_icon(service).unwrap_or("\u{f001}");
    }
    match service {
        "Spotify" => "\u{1f3a7}",
        "Last.fm" => "\u{1f3b5}",
        "YouTube" => "\u{25b6}\u{fe0f}", // same ▶️ as command palette's emoji YouTube Search
        // Emoji has no Discord glyph in the fonts this targets, and an empty
        // cell looked like a broken row: the chooser's own label column was
        // padded, so the Discord service rendered as a blank before its name.
        // A speech bubble is the closest thing in the emoji range.
        "Discord" => "\u{1f4ac}",
        _ => "",
    }
}

/// The glyph for a playback source, as the daemon reported it.
///
/// `source` is one of the `SRC_*` values the daemon sends, not a name this
/// client looks up: the whole point of moving the derivation to gtmd is that
/// there is exactly one place that decides what a path means, and a client
/// that re-derived it would be free to disagree.
///
/// Returns `None` for an unknown value rather than a fallback glyph. An empty
/// or absent source means "not known", and drawing a specific icon for that
/// would be a guess — the same rule `PLAYBACK_SOURCE.md` sets for third-party
/// clients.
pub(crate) fn source_icon(source: &str, icon_style: &str) -> Option<&'static str> {
    if !crate::shared::is_known(source) {
        return None;
    }
    if icon_style == "mdi" {
        return Some(match source {
            // nf-md-chart_bar, for the charts providers. The only source with
            // no entry in `provider_icon`, which is a table of *providers* and
            // so has no row for a category of provider.
            crate::shared::SRC_CHART => "\u{f0128}",
            crate::shared::SRC_SPOTIFY => provider_icon("Spotify")?,
            crate::shared::SRC_RADIO => provider_icon("Radio")?,
            crate::shared::SRC_PODCAST => provider_icon("Podcast")?,
            crate::shared::SRC_LOCAL => provider_icon("Local")?,
            _ => return None,
        });
    }
    let glyph = service_icon_glyph(
        icon_style,
        match source {
            crate::shared::SRC_SPOTIFY => "Spotify",
            crate::shared::SRC_RADIO => "Radio",
            crate::shared::SRC_PODCAST => "Podcast",
            crate::shared::SRC_LOCAL => "Local",
            crate::shared::SRC_CHART => return None,
            _ => return None,
        },
    );
    // Emoji has no glyph for Radio, Podcast or Chart, and a blank cell in the
    // middle of a line reads as a rendering fault rather than as an absence.
    // The caller treats `None` as "no marker", which is honest.
    if glyph.is_empty() { None } else { Some(glyph) }
}

/// Every provider the client can talk to, as (glyph, display name).
///
/// Ordered as a user would meet them rather than as the code does: the two
/// that carry playback, then the directories and scrobbling around them. The
/// onboarding view draws these inline, so the pairs have to be stable — this is
/// the only list of its kind, and it derives its glyphs from the same table the
/// rest of the UI reads so a font that cannot render one degrades in one place.
pub(crate) fn providers(icon_style: &str) -> Vec<(&'static str, &'static str)> {
    [
        "Spotify", "YouTube", "Radio", "Podcast", "Last.fm", "Discord", "Local",
    ]
    .into_iter()
    .map(|name| (service_icon_glyph(icon_style, name), name))
    .collect()
}

#[cfg(test)]
mod source_icon_tests {
    use super::source_icon;
    use crate::shared::{SOURCES, SRC_CHART, SRC_LOCAL, SRC_RADIO, SRC_SPOTIFY};

    /// Every source the daemon can send gets a glyph in the nerd style.
    ///
    /// Driven from `SOURCES` rather than a hand-written list, so a value added
    /// to the wire vocabulary cannot reach a client that has no icon for it —
    /// which would render as a silently missing marker rather than an error.
    #[test]
    fn every_source_has_a_nerd_glyph() {
        for src in SOURCES {
            assert!(
                source_icon(src, "mdi").is_some(),
                "{src:?} has no glyph in the mdi style"
            );
        }
    }

    /// The glyphs are distinct, so two sources never look alike.
    ///
    /// The risk is a copy-paste arm: two sources sharing a glyph renders as
    /// "radio" while saying "podcast", which is the failure the whole field
    /// exists to prevent.
    #[test]
    fn nerd_glyphs_are_distinct() {
        let mut seen: Vec<(&str, &str)> = SOURCES
            .iter()
            .map(|s| (*s, source_icon(s, "mdi").expect("checked above")))
            .collect();
        seen.sort_by_key(|(_, g)| *g);
        let before = seen.len();
        seen.dedup_by_key(|(_, g)| *g);
        assert_eq!(seen.len(), before, "two sources share a glyph: {seen:?}");
    }

    /// An unknown source renders nothing.
    ///
    /// The daemon may be newer than this client, so an unrecognised value is an
    /// ordinary case, not a malformed one. A fallback glyph would be a lie
    /// about where the audio came from.
    #[test]
    fn unknown_and_empty_render_nothing() {
        for s in ["", "tidal", "LOCAL", "Spotify", " spotify"] {
            assert!(source_icon(s, "mdi").is_none(), "{s:?} should be unknown");
            assert!(source_icon(s, "emoji").is_none(), "{s:?} should be unknown");
        }
    }

    /// Emoji mode drops the sources it has no glyph for, rather than drawing a
    /// blank cell.
    ///
    /// `service_icon_glyph` returns `""` for Radio and Podcast in emoji mode —
    /// the same gap the provider row already handles by padding. Here the
    /// marker is a single optional glyph, so no marker is the honest answer.
    #[test]
    fn emoji_mode_degrades_to_nothing_not_a_blank() {
        assert!(source_icon(SRC_SPOTIFY, "emoji").is_some());
        for src in [SRC_RADIO, SRC_CHART, SRC_LOCAL] {
            let got = source_icon(src, "emoji");
            assert!(
                got.is_none() || !got.unwrap().is_empty(),
                "{src} produced an empty glyph rather than None"
            );
        }
    }
}
