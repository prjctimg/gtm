// Copyright (c) 2026
// Author: prjctimg <prjctimg@outlook.com>
// Nerd-font and ASCII glyph tables
//
//
// This is free software released under the GPL-3.0 license.

use crate::ui::*;

pub(crate) const LIBRARY_ICONS_NERD: &[&str] = &[
    "\u{f001}",
    "\u{f004}",
    "\u{f025}",
    "\u{f007}",
    "\u{f03a}",
    "\u{f04c7}",
    "\u{f0439}", // Radio: nf-md-radio (official MDI)
    "\u{f0128}", // Most Played: nf-md-chart_bar (verified: U+F0120 was tray-arrow-down)
    "\u{f02da}", // Recently Played: nf-md-history (official MDI)
    "\u{f1da}",
    "\u{f04fb}", // Genres: nf-md-tag_multiple (official MDI)
    "\u{f07b}",
    "\u{f0535}", // Top Charts: nf-md-trending_up (official MDI)
];

pub(crate) const LIBRARY_ICONS_ASCII: &[&str] = &[
    "♫", "♥", "▤", "♪", "≡", "☊", "◉", "▥", "◆", "♫", "◎", "▽", "#",
];

pub(crate) fn use_nerd_fonts() -> bool {
    !matches!(std::env::var("GTM_NERD_FONTS"), Ok(v) if v == "0" || v == "false" || v == "no")
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

pub(crate) const SETTINGS_ICONS_NERD: &[&str] = &["\u{f16a}", "\u{f04b}", "\u{f013}", "\u{f04c7}"];

pub(crate) const SETTINGS_ICONS_ASCII: &[&str] = &["YT", "▶", "⚙", "★"];

pub(crate) const SETTINGS_CATEGORIES: &[&str] = &["YouTube", "Playback", "System", "Spotify"];

pub(crate) fn service_icon_glyph(icon_style: &str, service: &str) -> &'static str {
    if icon_style == "mdi" {
        return provider_icon(service).unwrap_or("\u{f001}");
    }
    match service {
        "Spotify" => "\u{1f3a7}",
        "Last.fm" => "\u{1f3b5}",
        "YouTube" => "\u{25b6}\u{fe0f}", // same ▶️ as command palette's emoji YouTube Search
        _ => "",
    }
}
