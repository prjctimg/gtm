use crate::app::*;
use crate::ui::{LIBRARY_ICONS_ASCII, LIBRARY_ICONS_NERD};

#[test]
fn lyridx_timed_lines() {
    let lines = vec![
        LrcLine {
            timestamp: -1.0,
            text: "intro (untimed)".into(),
            words: Vec::new(),
        },
        LrcLine {
            timestamp: 0.0,
            text: "first".into(),
            words: Vec::new(),
        },
        LrcLine {
            timestamp: 5.0,
            text: "second".into(),
            words: Vec::new(),
        },
        LrcLine {
            timestamp: 10.0,
            text: "third".into(),
            words: Vec::new(),
        },
    ];
    assert_eq!(lyric_index_at(&lines, -1.0), 0);
    assert_eq!(lyric_index_at(&lines, 0.0), 1);
    assert_eq!(lyric_index_at(&lines, 4.9), 1);
    assert_eq!(lyric_index_at(&lines, 5.0), 2);
    assert_eq!(lyric_index_at(&lines, 999.0), 3);
}

#[test]
fn lyridx_empty_zero() {
    assert_eq!(lyric_index_at(&[], 42.0), 0);
}

#[test]
fn lib_focus_forward() {
    let (lib, lyr) = cycle_library_focus(true, false, true);
    assert_eq!((lib, lyr), (false, false));
    let (lib, lyr) = cycle_library_focus(false, false, true);
    assert_eq!((lib, lyr), (false, true));
    let (lib, lyr) = cycle_library_focus(false, true, true);
    assert_eq!((lib, lyr), (true, false));
}

#[test]
fn lib_focus_backward() {
    let (lib, lyr) = cycle_library_focus(true, false, false);
    assert_eq!((lib, lyr), (false, true));
    let (lib, lyr) = cycle_library_focus(false, false, false);
    assert_eq!((lib, lyr), (true, false));
    let (lib, lyr) = cycle_library_focus(false, true, false);
    assert_eq!((lib, lyr), (false, false));
}

// The client-id field is back in the link picker, but it is no longer the app
// gtm streams with: it is the user's own *Web API* app, which moves search,
// artwork and playlist sync into a rate-limit bucket of their own. Playback
// keeps registering as librespot's app, because Spotify Connect only accepts a
// recognised playback app. There is no validator — the id is opaque, and the
// Web API rejects a bad one by refusing the flow. See
// `SpotifyManager::client_id` and `SpotifyManager::web_client_id`.

/// Every category needs a glyph at its own index, in both tables.
///
/// The icon tables are positional and indexed by the category index, so
/// appending a category without appending to both silently renders a blank
/// (or, worse, borrows the previous category's glyph). Nothing at the call
/// site would notice, since `icons.get(i)` returns `None` and falls back to a
/// space rather than failing.
#[test]
fn every_library_category_has_an_icon() {
    assert_eq!(
        LIBRARY_CATEGORIES.len(),
        LIBRARY_ICONS_NERD.len(),
        "a category was added without its nerd-font glyph"
    );
    assert_eq!(
        LIBRARY_CATEGORIES.len(),
        LIBRARY_ICONS_ASCII.len(),
        "a category was added without its ASCII glyph"
    );
}

/// Categories are dispatched by index, so the indices other code hardcodes
/// (5 Spotify, 12 Top Charts) are part of the contract. New categories must
/// therefore be appended: inserting one silently re-points every `== N`
/// comparison at the wrong list.
#[test]
fn pinned_category_indices_do_not_move() {
    assert_eq!(LIBRARY_CATEGORIES[5], "Spotify");
    assert_eq!(LIBRARY_CATEGORIES[6], "Radio");
    assert_eq!(LIBRARY_CATEGORIES[12], "Top Charts");
    assert_eq!(
        *LIBRARY_CATEGORIES.last().unwrap(),
        "Browse",
        "the newest category must be appended, not inserted"
    );
}

/// A config that predates the Podcasts category must not lose the categories
/// it does name. The list is a filter, not a reorder, so unknown names are
/// dropped while known ones survive.
#[test]
fn stale_left_pane_lists_keep_known_categories() {
    let names: Vec<String> = LIBRARY_CATEGORIES[..13]
        .iter()
        .map(|s| s.to_string())
        .collect();
    let out = clean_left_pane(&names);
    assert_eq!(
        out.len(),
        13,
        "every known category must survive the filter"
    );
    assert!(!out.iter().any(|c| c == "Podcasts"));
}

/// A client id pasted into the port box must be reported, not swallowed.
///
/// The two fields are one Tab apart, and a non-numeric port used to fall back
/// to the default: the flow then linked the account against librespot's shared
/// app while appearing to honour the id, with no message anywhere. The symptom
/// — a token that never gets its own quota, and playlists that fail to sync —
/// was indistinguishable from the id being rejected.
#[test]
fn a_non_numeric_port_is_an_error_not_a_default() {
    // A real client id in the port field is the exact mistake that was silent.
    assert!(super::run::parse_oauth_port("0123456789abcdef0123456789abcdef").is_err());
    // And the error has to point at the cause, or the user fixes the wrong box.
    let err = super::run::parse_oauth_port("8990a").unwrap_err();
    assert!(err.contains("Tab"), "the error should name the fix: {err}");
}

/// Blank means "I did not change this", which is a legitimate answer and must
/// not be treated as a mistake.
#[test]
fn a_blank_port_still_defaults() {
    assert_eq!(super::run::parse_oauth_port("").unwrap(), 8990);
    assert_eq!(super::run::parse_oauth_port("   ").unwrap(), 8990);
    assert_eq!(super::run::parse_oauth_port("1234").unwrap(), 1234);
}
