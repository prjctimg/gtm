use crate::app::*;

/// Index of the active time-synced lyric line for a playback position.
/// Untimed lines (timestamp < 0) are skipped for matching but keep their
/// index so the highlight tracks timed lines correctly. Uses
/// rposition semantics over sorted timed entries.
pub(crate) fn lyric_index_at(lines: &[LrcLine], position: f64) -> usize {
    if lines.is_empty() {
        return 0;
    }
    // Last timed line with timestamp <= position. Untimed
    // lines keep their index but never match; before the first timestamp
    // (and for plain lyrics) the highlight rests on line 0.
    lines
        .iter()
        .enumerate()
        .filter(|(_, l)| l.timestamp >= 0.0 && l.timestamp <= position)
        .map(|(i, _)| i)
        .next_back()
        .unwrap_or(0)
}

/// Whether the current lyrics have any time-synced lines. Plain lyrics
/// (`timestamp < 0` for all lines) should not highlight an active line.
pub fn lyrics_are_synced(lines: &[LrcLine]) -> bool {
    lines.iter().any(|l| l.timestamp >= 0.0)
}

/// Pure focus-state transition for Tab/Shift-Tab pane cycling on the Library
/// tab with lyrics open.  States are `(library_focus, lyrics_focus)`:
/// left `(true, false)`, right `(false, false)`, lyrics `(false, true)`.
/// Returns the next `(library_focus, lyrics_focus)` moving forward (Tab) or
/// backward (Shift-Tab) around the three-pane cycle.
pub(crate) fn cycle_library_focus(library_focus: bool, lyrics_focus: bool, forward: bool) -> (bool, bool) {
    if lyrics_focus {
        // lyrics → left (Tab) or right (Shift-Tab)
        (forward, false)
    } else if library_focus {
        // left → right (Tab) or lyrics (Shift-Tab)
        if forward {
            (false, false)
        } else {
            (false, true)
        }
    } else if forward {
        // right → lyrics
        (false, true)
    } else {
        // right → left
        (true, false)
    }
}

impl App {
    /// Index of the time-synced lyric line for the current playback position.
    /// Untimed lines (timestamp < 0) are skipped for matching but keep their
    /// index so the highlight tracks timed lines correctly.
    pub fn current_lyric_index(&self) -> usize {
        let Some(ref lyrics) = self.lyrics.current else {
            return 0;
        };
        lyric_index_at(&lyrics.lines, self.raw_position + self.lyrics.offset_secs)
    }

    /// Shift the lyric time baseline by `delta` seconds so lines whose timing
    /// is early or late line up with the audio. Only meaningful while a track
    /// with synced lyrics is loaded. Re-engages auto-follow and clears the
    /// in-flight offset adjustment once the user stops nudging.
    pub fn nudge_lyrics_offset(&mut self, delta: f64) {
        if self.lyrics.current.is_none() {
            return;
        }
        let mut offset = self.lyrics.offset_secs + delta;
        if !offset.is_finite() {
            offset = 0.0;
        }
        offset = offset.clamp(-120.0, 120.0);
        self.lyrics.offset_secs = offset;
        self.lyrics.manual_scroll = false;
        self.notify_typed(
            "Lyrics",
            format!("offset {:+.2}s — press [ / ] to adjust", offset),
            NotificationKind::Info,
            true,
            NotifType::NowPlaying,
        );
    }
}
