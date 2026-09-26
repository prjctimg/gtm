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
