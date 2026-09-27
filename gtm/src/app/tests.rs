use crate::app::*;

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

/// An empty client id must be accepted here, because the daemon resolves it to
/// librespot's public desktop app — the one Spotify permits for streaming. This
/// check used to reject it while its own doc claimed the opposite, which left a
/// self-registered app as the only linkable option.
#[test]
fn empty_client_id_is_accepted() {
    assert_eq!(client_id_error("", 8990), None);
    assert_eq!(client_id_error("   ", 8990), None);
}

/// A real id still has to look like one, and the error has to name the redirect
/// URI, which is the part that fails silently in the browser.
#[test]
fn a_malformed_client_id_is_rejected() {
    let err = client_id_error("not-a-client-id", 8990).expect("should reject");
    assert!(
        err.contains("127.0.0.1:8990/login"),
        "names the redirect: {err}"
    );
    assert!(client_id_error("zzzz", 8990).is_some());
    assert_eq!(
        client_id_error("65b708073fc0480ea92a077233ca87bd", 8990),
        None
    );
}
