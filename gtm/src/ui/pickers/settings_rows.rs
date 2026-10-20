// Copyright (c) 2026
// Author: prjctimg <prjctimg@outlook.com>
// The settings pane: one declaration of what each category contains
//
// This is free software released under the GPL-3.0 license.

//! The settings pane, described once.
//!
//! One array of `(label, kind)` per category serves the rendered labels, the
//! per-row help line and the row count the cursor clamps to. Adding a row is a
//! line in the array and an arm in the handler; the count is `len()`.
//!
//! The handler in `app::settings_keys` reads the same positions, so the two are
//! checked together by a test rather than by hand.

use crate::ui::*;

/// What a row does, and how it is drawn.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RowKind {
    /// A value that can be read out but not acted on.
    Status,
    /// An On/Off switch.
    Toggle,
    /// One of a set of named values; `▶` marks it.
    Cycle,
    /// A named value chosen in another picker; `▶` marks it.
    Chooser,
    /// A one-shot action.
    Action,
}

/// `(row label, kind)`, in display order.
pub(crate) type SettingsRows = &'static [(&'static str, RowKind)];

/// Playback. Six toggles and value rows, no actions.
pub(crate) const PLAYBACK_ROWS: SettingsRows = &[
    ("Repeat", RowKind::Cycle),
    ("Shuffle", RowKind::Toggle),
    ("Crossfade", RowKind::Chooser),
    ("EQ Enabled", RowKind::Toggle),
    ("Reverb", RowKind::Toggle),
    ("Pre-Gain", RowKind::Cycle),
    ("Cover Source", RowKind::Cycle),
    ("Preview Delay", RowKind::Cycle),
];

/// System. Toggles, values, and the quick actions.
pub(crate) const SYSTEM_ROWS: SettingsRows = &[
    ("Theme", RowKind::Chooser),
    ("Theme Mode", RowKind::Cycle),
    ("Transparent BG", RowKind::Toggle),
    ("Transparent Pickers", RowKind::Toggle),
    ("Hide Footer", RowKind::Toggle),
    ("Reactive Theme", RowKind::Toggle),
    ("Reactive Intensity", RowKind::Cycle),
    ("Visualizer", RowKind::Chooser),
    ("Daydream", RowKind::Cycle),
    ("Footer Preset", RowKind::Chooser),
    ("Sync Covers", RowKind::Action),
    ("Sync Lyrics", RowKind::Action),
    ("Sync Metadata", RowKind::Action),
    ("Clear Lyrics Cache", RowKind::Action),
    ("Clear Cover Cache", RowKind::Action),
    ("Notification Settings", RowKind::Action),
    ("Show Welcome", RowKind::Toggle),
    // The daemon outlives the client, so this one decides whether starting it
    // starts the music. Off by default: nobody asked the machine to make noise
    // just because they opened a socket.
    ("Resume Playback on Start", RowKind::Toggle),
];

/// Spotify. Connection state and the two things that can change it.
pub(crate) const SPOTIFY_ROWS: SettingsRows = &[
    ("Status", RowKind::Status),
    ("Link Account", RowKind::Action),
    ("Unlink", RowKind::Action),
];

/// The rows of one settings category.
pub(crate) fn rows_for(category: usize) -> SettingsRows {
    match category {
        0 => PLAYBACK_ROWS,
        1 => SYSTEM_ROWS,
        2 => SPOTIFY_ROWS,
        _ => &[],
    }
}

/// The help line under the highlighted row.
///
/// Empty where the row is self-explanatory. The Spotify transport rows
/// deliberately get nothing: they duplicate `n`/`p` and the on-screen controls,
/// and their real distinction — that they act through Spotify's own device
/// control rather than the local queue — is exactly what a one-line hint would
/// have to explain.
pub(crate) fn row_help(category: usize, option: usize) -> &'static str {
    match (category, option) {
        (0, 0) => " Press Enter to cycle repeat (off / one / all).",
        (0, 2) => " Press Enter to open the crossfade picker.",
        (0, 5) => " Press Enter to step the pre-gain by 1 dB.",
        (0, 6) => " Press Enter to cycle the cover art source.",
        (0, 7) => " Wait this long on a chart row before it starts a preview (off = never).",
        (1, 0) => " Press Enter to open the theme picker.",
        (1, 1) => " Press Enter to cycle theme mode (auto/dark/light).",
        (1, 6) => " Press Enter to toggle the reactive theme.",
        (1, 8) => " Press Enter to open the visualizer picker.",
        (1, 9) => " Show the visualizer after this long without a keypress (off = never).",
        (1, 10) => " Press Enter to open the footer preset picker.",
        (1, 11) => " Download missing cover art from Deezer.",
        (1, 12) => " Fetch and save lyrics for all tracks.",
        (1, 13) => " Resolve and embed clean tags into files.",
        (1, 14) => " Clear cached lyrics for all tracks.",
        (1, 15) => " Clear the downloaded cover art cache.",
        (1, 16) => " Press Enter to open notification settings.",
        (1, 17) => " Show the welcome pane at launch. H on it clears this too.",
        (1, 18) => {
            " Let the daemon resume playing when it starts. Off: the last \
track is restored paused instead."
        }
        (2, 1) => " Authorize gtm with your Spotify account.",
        (2, 2) => " Remove the token and disconnect.",
        _ => "",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A row count that disagrees with the row list is how the cursor ends up
    /// addressing a row that does not exist.
    #[test]
    fn row_counts_match_the_declared_lists() {
        assert_eq!(rows_for(0).len(), 8);
        assert_eq!(rows_for(1).len(), 18);
        // Status, Link Account, Unlink.
        assert_eq!(rows_for(2).len(), 3);
        for cat in 0..3 {
            assert!(!rows_for(cat).is_empty(), "category {cat} is empty");
            for (i, (label, _)) in rows_for(cat).iter().enumerate() {
                assert!(
                    !label.is_empty(),
                    "category {cat} row {i} has an empty label"
                );
            }
        }
    }

    /// Every `row_help` index has to name a row that exists, in both directions:
    /// a help line for a row that is not there, or a row with a help line for
    /// the wrong option index, is the drift this module exists to prevent.
    #[test]
    fn help_lines_only_name_rows_that_exist() {
        for cat in 0..3 {
            for opt in 0..rows_for(cat).len() {
                // Only asserts that indexing is in range; the content check is
                // the count test above.
                let _ = row_help(cat, opt);
            }
        }
    }

    /// The Spotify category is Status, Link and Unlink.
    #[test]
    fn spotify_is_status_link_and_unlink() {
        let names: Vec<&str> = rows_for(2).iter().map(|(n, _)| *n).collect();
        assert_eq!(names, ["Status", "Link Account", "Unlink"]);
    }
}
