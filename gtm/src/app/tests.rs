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

/// A provider that returns lines out of timestamp order must still highlight
/// the right verse.
///
/// Matching used to take the last qualifying line *in slice order*, which is
/// the same as the latest timestamp only while the file is sorted. One line out
/// of order parked the highlight on the wrong verse for the rest of the track,
/// and the failure was invisible: the highlight kept moving, just against the
/// wrong lyrics.
#[test]
fn lyridx_out_of_order_lines_match_by_timestamp() {
    let mk = |timestamp: f64, text: &str| LrcLine {
        timestamp,
        text: text.into(),
        words: Vec::new(),
    };
    // "third" is listed before "second", and the untimed header is not first.
    let lines = vec![
        mk(10.0, "third"),
        mk(-1.0, "header"),
        mk(5.0, "second"),
        mk(0.0, "first"),
    ];
    // Sorted order would give 1 -> 2 -> 3 -> 0 here.
    assert_eq!(
        lyric_index_at(&lines, -1.0),
        0,
        "before the first timestamp"
    );
    assert_eq!(lyric_index_at(&lines, 0.0), 3, "at 0s: `first`");
    assert_eq!(lyric_index_at(&lines, 4.9), 3, "before 5s: still `first`");
    assert_eq!(
        lyric_index_at(&lines, 5.0),
        2,
        "at 5s: `second`, not `third`"
    );
    assert_eq!(lyric_index_at(&lines, 9.9), 2, "before 10s: still `second`");
    assert_eq!(lyric_index_at(&lines, 10.0), 0, "at 10s: `third`");
    assert_eq!(lyric_index_at(&lines, 999.0), 0, "past the end: `third`");
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

/// Categories are dispatched by index, so the named constants other code uses
/// (`LIB_SPOTIFY`, `LIB_CHARTS`) are part of the contract: a constant that
/// names a different row than its label is a whole pane dispatching on the
/// wrong list.
///
/// The absorbed views are deliberately *not* rows any more. `Liked` went last
/// and took a number with it, so `LIB_PLAYLISTS` and every index above it moved
/// down one; the pinned assertion is what makes that renumbering deliberate
/// instead of accidental.
#[test]
fn pinned_category_indices_do_not_move() {
    assert_eq!(LIBRARY_CATEGORIES[LIB_ALL], "Library");
    assert_eq!(LIBRARY_CATEGORIES[LIB_PLAYLISTS], "Playlists");
    assert_eq!(LIBRARY_CATEGORIES[LIB_SPOTIFY], "Spotify");
    assert_eq!(LIBRARY_CATEGORIES[LIB_RADIO], "Radio");
    assert_eq!(LIBRARY_CATEGORIES[LIB_CHARTS], "Top Charts");
    assert_eq!(LIBRARY_CATEGORIES[LIB_PODCASTS], "Podcasts");
    assert_eq!(
        LIBRARY_CATEGORIES.len(),
        LIB_PODCASTS + 1,
        "a category was inserted rather than appended"
    );
    // Nothing may name an absorbed view as a row: they are groups now, and a
    // `LIB_CATEGORIES` entry for one would render a pane with no way to reach
    // it.
    for name in LIB_ABSORBED {
        assert!(
            !LIBRARY_CATEGORIES.contains(name),
            "{name} is a group now, not a left-pane row"
        );
    }
}

/// Every absorbed view has to land somewhere, or the rows it used to occupy are
/// simply gone.
#[test]
fn every_absorbed_view_became_a_group() {
    use PlaylistGroup::{MostPlayed, RecentlyAdded, RecentlyPlayed};
    let groups: Vec<&str> = PlaylistGroup::ALL.iter().map(|g| g.label()).collect();
    for name in ["Most Played", "Recently Played", "Recently Added", "Liked"] {
        assert!(
            groups.contains(&name),
            "{name} lost its row and has no group to replace it"
        );
    }
    // Folders joined the Library's groupings rather than the Playlists'.
    let lib: Vec<&str> = LibraryFilter::ALL.iter().map(|f| f.label()).collect();
    assert!(
        lib.contains(&"Folders"),
        "Folders has neither a row nor a grouping"
    );
    assert_eq!(MostPlayed.hist(), Some(HistList::Most));
    assert_eq!(RecentlyPlayed.hist(), Some(HistList::Recent));
    assert_eq!(RecentlyAdded.hist(), Some(HistList::Added));
    assert_eq!(
        PlaylistGroup::Playlists.hist(),
        None,
        "the playlists group is pushed by the daemon, not fetched"
    );
    assert_eq!(
        PlaylistGroup::Liked.hist(),
        None,
        "Liked is one fetch of its own, not a rank-ordered query"
    );
}

/// A config that predates the merge must not lose the list it named.
///
/// All Tracks, Albums, Artists and Genres were four rows; the Library view is
/// one, so all four names resolve to it — a config naming three of them keeps
/// one row rather than dropping three.
#[test]
fn stale_left_pane_lists_keep_known_categories() {
    let out = clean_left_pane(&[
        "All Tracks".to_string(),
        "Liked".to_string(),
        "Albums".to_string(),
        "Spotify".to_string(),
    ]);
    assert_eq!(
        out,
        vec![
            "Library".to_string(),
            "Liked".to_string(),
            "Spotify".to_string()
        ],
        "the four merged list names must collapse to the one row that replaced them"
    );
    // Folders made the same move a view further along, so a config naming it
    // keeps the Library row rather than losing the list.
    let out = clean_left_pane(&["Folders".to_string(), "Liked".to_string()]);
    assert_eq!(
        out,
        vec!["Library".to_string(), "Liked".to_string()],
        "a config naming Folders must keep the row that replaced it"
    );
    // The three history lists have no left-pane row at all, so naming one is a
    // stale config rather than a view to open.
    let out = clean_left_pane(&["Most Played".to_string(), "Radio".to_string()]);
    assert_eq!(
        out,
        vec!["Radio".to_string()],
        "an absorbed history list must not keep a row"
    );
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
