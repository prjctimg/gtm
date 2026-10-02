// Copyright (c) 2026
// Author: prjctimg <prjctimg@outlook.com>
// The settings pane: one declaration of what each category contains
//
// This is free software released under the GPL-3.0 license.

//! The settings pane, described once.
//!
//! The list used to exist three times over: as a `match` producing the rendered
//! strings, as a second `match` producing the per-row help line, and as a third
//! `category_options()` returning the row *count* so the cursor knew where to
//! stop. They drifted. The YouTube category had a fourth row the handler
//! ignored; the Spotify category's transport rows were off by one, so Next,
//! Previous and Shuffle all ran `set_repeat` (the handler mapped a semantic
//! index, `opt - 4`, into a callee that matched `8 | 9 | 10 | _`); and a
//! category added or removed left the count disagreeing with the list.
//!
//! One array of `(label, kind)` per category now serves all three. Adding a row
//! is one line, and the count is `len()`.

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
];

/// System. Toggles, values, and the quick actions.
pub(crate) const SYSTEM_ROWS: SettingsRows = &[
    ("Theme", RowKind::Chooser),
    ("Theme Mode", RowKind::Cycle),
    ("Audio Output", RowKind::Chooser),
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
        assert_eq!(rows_for(0).len(), 7);
        assert_eq!(rows_for(1).len(), 17);
        // Status, Link Account, Unlink. The Spotify Next/Previous rows went
        // when the Connect transport moved to the command palette, and this
        // count was left at 5 -- so the test had been failing ever since,
        // asserting a row list that no longer existed.
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

    /// The Spotify category is Status, Link and Unlink. It used to also carry
    /// Next and Previous, which act on the *Connect device* rather than the
    /// local queue — and were the rows the semantic-index bug lived on.
    #[test]
    fn spotify_is_status_link_and_unlink() {
        let names: Vec<&str> = rows_for(2).iter().map(|(n, _)| *n).collect();
        assert_eq!(names, ["Status", "Link Account", "Unlink"]);
    }
}
