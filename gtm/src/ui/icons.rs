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

/// Every provider the client can talk to, as (glyph, display name).
///
/// Ordered as a user would meet them rather than as the code does: the two
/// that carry playback, then the directories and scrobbling around them. The
/// onboarding view draws these inline, so the pairs have to be stable — this is
/// the only list of its kind, and it derives its glyphs from the same table the
/// rest of the UI reads so a font that cannot render one degrades in one place.
pub(crate) fn providers() -> Vec<(&'static str, &'static str)> {
    [
        "Spotify", "YouTube", "Radio", "Podcast", "Last.fm", "Discord", "Local",
    ]
    .into_iter()
    .map(|name| (service_icon_glyph("mdi", name), name))
    .collect()
}
