use crate::app::*;

/// The `(artist, title)` a lyrics lookup should search with, or `None` when the
/// daemon can resolve the track from its own library row.
///
/// A provider track — a `spotify:` URI, a station, an episode — has no library
/// row and no file to read tags from, so `lyrics().get(id, path)` can only miss
/// for it. Asking by artist and title is the only route that works, and both
/// the automatic and the manual fetch have to take it or one of them silently
/// returns nothing.
pub(crate) fn lyrics_query(track: &TrackInfo) -> Option<(String, String)> {
    if !path_is_remote(&track.path) {
        return None;
    }
    match (track.artist.trim(), track.title.trim()) {
        ("", _) | (_, "") => None,
        (artist, title) => Some((artist.to_string(), title.to_string())),
    }
}

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
pub(crate) fn cycle_library_focus(
    library_focus: bool,
    lyrics_focus: bool,
    forward: bool,
) -> (bool, bool) {
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

    /// Start a lyrics fetch for `track`, updating the view state in place.
    ///
    /// Both entry points go through here — the automatic fetch on a track change
    /// and the manual one bound to the lyrics key. They used to disagree: only
    /// the automatic one knew a provider track has to be searched by artist and
    /// title, so asking manually for a Spotify track always came back empty.
    pub(crate) fn fetch_lyrics(&mut self, track: &TrackInfo) {
        let fetch_gen = self.next_lyrics_gen();
        self.lyrics.current = None;
        self.lyrics.pending_gen = Some(fetch_gen);
        self.lyrics.fetching = true;
        self.lyrics.scroll = 0;
        self.lyrics.offset_secs = 0.0;
        let track_id = track.id;
        let path = Some(track.path.clone());
        let query = lyrics_query(track);
        let client = self.client.clone();
        let ipc_tx = self.ipc_tx.clone();
        tokio::spawn(async move {
            let out = match query {
                Some((artist, title)) => client.lyrics().search(&artist, &title).await,
                None => match tokio::time::timeout(
                    Duration::from_secs(12),
                    client.lyrics().get(track_id, path.as_deref()),
                )
                .await
                {
                    Ok(res) => res,
                    Err(_) => Err(CoreError::Daemon("lyrics fetch timed out".into())),
                },
            };
            match out {
                Ok(lyrics) => {
                    let _ = ipc_tx.send(IpcResult::Lyrics(lyrics, fetch_gen));
                }
                Err(e) => {
                    let _ = ipc_tx.send(IpcResult::Lyrics(None, fetch_gen));
                    let _ = ipc_tx.send(IpcResult::Error(format!("Lyrics: {e}")));
                }
            }
        });
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

#[cfg(test)]
mod tests {
    use super::*;

    fn track(path: &str, artist: &str, title: &str) -> TrackInfo {
        TrackInfo {
            path: path.to_string(),
            artist: artist.to_string(),
            title: title.to_string(),
            ..Default::default()
        }
    }

    /// A Spotify entry has no library row and no file to tag-read, so the
    /// daemon can only answer by artist and title. This is the branch the
    /// manual lyrics key was missing.
    #[test]
    fn a_spotify_track_searches_by_artist_and_title() {
        let t = track(
            "spotify:track:4cOdK2wGLETKBW3PvgPWqT",
            "Daft Punk",
            "Get Lucky",
        );
        assert_eq!(
            lyrics_query(&t),
            Some(("Daft Punk".to_string(), "Get Lucky".to_string()))
        );
    }

    /// A local file must keep going through the daemon, which knows its library
    /// row and the sidecar on disk.
    #[test]
    fn a_local_file_stays_on_the_library_route() {
        assert_eq!(lyrics_query(&track("/music/song.mp3", "A", "B")), None);
    }

    /// Half-filled metadata cannot be searched on, so the daemon route is the
    /// only one that can try.
    #[test]
    fn partial_metadata_falls_back_to_the_daemon() {
        let t = track("spotify:track:4cOdK2wGLETKBW3PvgPWqT", "", "Get Lucky");
        assert_eq!(lyrics_query(&t), None);
    }
}
