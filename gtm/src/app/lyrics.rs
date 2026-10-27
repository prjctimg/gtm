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

/// Split a synthetic podcast row path (`podcast://<feed-id>/<index>`) into the
/// feed and episode it addresses.
///
/// A podcast row is not a song: its artist is the feed title and its title is
/// the episode, so searching for lyrics by those fields would either miss or,
/// worse, find an unrelated song with the episode's name. The path is the only
/// reliable way back to the episode whose transcript should be shown.
pub(crate) fn podcast_episode(path: &str) -> Option<(String, usize)> {
    let rest = path.strip_prefix("podcast://")?;
    let (feed_id, index) = rest.split_once('/').unwrap_or((rest, "0"));
    let feed_id = feed_id.trim();
    if feed_id.is_empty() {
        return None;
    }
    let episode_index = index.parse::<usize>().ok()?;
    Some((feed_id.to_string(), episode_index))
}

/// Index of the active time-synced lyric line for a playback position.
/// Untimed lines (timestamp < 0) are skipped for matching but keep their
/// index so the highlight tracks timed lines correctly.
pub(crate) fn lyric_index_at(lines: &[LrcLine], position: f64) -> usize {
    if lines.is_empty() {
        return 0;
    }
    // The latest timestamp at or before `position`, chosen by value rather than
    // by position in the slice.
    //
    // The two agree only while every provider hands lines back in timestamp
    // order, and when they disagree the highlight follows the highest timestamp
    // among the qualifying lines rather than whichever came last in the array.
    // One line out of order would otherwise park the highlight on the wrong
    // verse for the rest of the track, with no
    // way for the user to tell it had stopped tracking.
    //
    // Ties take the later index, so two lines sharing a timestamp resolve to the
    // one the provider listed second -- the same answer the slice-order version
    // gave for a well-formed file.
    let mut best: Option<(usize, f64)> = None;
    for (i, line) in lines.iter().enumerate() {
        if line.timestamp < 0.0 || line.timestamp > position {
            continue;
        }
        if best.is_none_or(|(_, latest)| line.timestamp >= latest) {
            best = Some((i, line.timestamp));
        }
    }
    // Nothing qualifies: before the first timestamp, or plain lyrics. The
    // highlight rests on line 0, which is what an unsynced file's first line is.
    best.map_or(0, |(index, _)| index)
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

/// Publish a lookup's outcome, reporting a miss as an error toast the way the
/// manual key always has.
fn report(
    out: Result<Option<LrcData>, CoreError>,
    ipc_tx: &mpsc::UnboundedSender<IpcResult>,
    fetch_gen: u64,
    path: String,
) {
    match out {
        Ok(lyrics) => {
            let _ = ipc_tx.send(IpcResult::Lyrics(lyrics, fetch_gen, Some(path)));
        }
        Err(e) => {
            let _ = ipc_tx.send(IpcResult::Lyrics(None, fetch_gen, Some(path)));
            let _ = ipc_tx.send(IpcResult::Error(format!("Lyrics: {e}")));
        }
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
        lyric_index_at(&lyrics.lines, self.raw_position)
    }

    /// Start a lyrics fetch for `track`, updating the view state in place.
    ///
    /// Both entry points go through here — the automatic fetch on a track change
    /// and the manual one bound to the lyrics key — so both know a provider track
    /// has to be searched by artist and title rather than by path.
    ///
    /// For a podcast, `manual` makes a transcript failure visible. The episode
    /// picker's `t` key goes one step further and passes no target, so its
    /// transcript is accepted even if another track starts; automatic requests
    /// are pinned to the episode they were made for and stay silent when the
    /// episode simply has no transcript.
    pub(crate) fn fetch_lyrics(&mut self, track: &TrackInfo, manual: bool) {
        // A podcast episode has a transcript, not lyrics. Its artist/title are
        // the show and episode, so the normal search would at best miss and at
        // worst match a song that happens to share the episode's name.
        if let Some((feed_id, episode_index)) = podcast_episode(&track.path) {
            self.start_podcast_transcript(
                feed_id,
                episode_index,
                track.title.clone(),
                Some(track.path.clone()),
                manual,
            );
            return;
        }

        // A live stream's queue row is the station, not the song on air. The
        // path stays the same for the whole session, so fetching from
        // `current_track` would either answer for the station name or keep the
        // previous song's lyrics. Resolve the live title/artist instead.
        if crate::footer::is_live_stream(&track.path) {
            match self.live_lyrics_query() {
                Some((artist, title)) => self.search_lyrics(&artist, &title, None, None),
                None => self.invalidate_lyrics(),
            }
            return;
        }

        let (track_id, path) = (track.id, Some(track.path.clone()));
        match lyrics_query(track) {
            Some((artist, title)) => self.search_lyrics(
                &artist,
                &title,
                Some(track.album.as_str()).filter(|a| !a.trim().is_empty()),
                (track.duration > 0.0).then_some(track.duration),
            ),
            None => self.library_lyrics(track_id, path),
        }
    }

    /// The current live song as `(artist, title)`, derived from the ICY title
    /// and the station's tracklist. `None` until the stream has published a
    /// song, so the station's own name is never searched for lyrics.
    fn live_lyrics_query(&self) -> Option<(String, String)> {
        if self.state.radio_title.as_deref().is_none_or(str::is_empty) {
            return None;
        }
        let (title, artist) = self.live_track()?;
        let title = title.trim();
        if title.is_empty() {
            return None;
        }
        let artist = artist.trim();
        if !artist.is_empty() {
            return Some((artist.to_string(), title.to_string()));
        }
        // ICY often packs "Artist - Title" into one string. Split it only when
        // no separate artist was published, so a real title containing a dash
        // is left alone.
        if let Some((artist, song)) = title.split_once(" - ") {
            let (artist, song) = (artist.trim(), song.trim());
            if !artist.is_empty() && !song.is_empty() {
                return Some((artist.to_string(), song.to_string()));
            }
        }
        Some((String::new(), title.to_string()))
    }

    /// Drop the displayed lyrics without starting a fetch. Used when the
    /// playing track (or live song) changed while automatic lyrics were off:
    /// leaving the old lines in place made the next `l` press toggle the pane
    /// onto the previous track's lyrics, because it only fetches when the
    /// current value is empty.
    pub(crate) fn invalidate_lyrics(&mut self) {
        self.lyrics.current = None;
        self.lyrics.kind = LyricsKind::None;
        self.lyrics.pending_gen = None;
        self.lyrics.fetching = false;
        self.lyrics.scroll = 0;
        self.lyrics.row = None;
        self.lyrics.manual_scroll = false;
    }

    /// Fetch one episode's transcript through the same generation guard the
    /// lyrics pane uses.
    ///
    /// `target` is the playing row for an automatic fetch, and `None` for the
    /// episode picker's `t` key. The daemon parses VTT/SRT/JSON into timed
    /// lines, so the existing lyric clock moves the transcript without any
    /// separate playback path.
    pub(crate) fn start_podcast_transcript(
        &mut self,
        feed_id: String,
        episode_index: usize,
        label: String,
        target: Option<String>,
        manual: bool,
    ) {
        let fetch_gen = self.begin_lyrics();
        let client = self.client.clone();
        let ipc_tx = self.ipc_tx.clone();
        tokio::spawn(async move {
            let (lyrics, error) = match client.podcast().transcript(&feed_id, episode_index).await {
                Ok(lyrics) => (Some(lyrics), None),
                Err(e) => (None, manual.then(|| format!("No transcript: {e}"))),
            };
            let _ = ipc_tx.send(IpcResult::PodcastTranscript {
                lyrics,
                label,
                fetch_gen,
                target,
                error,
            });
        });
    }

    /// Warm the daemon's on-disk lyrics cache for a synced playlist row,
    /// without touching the lyrics pane.
    ///
    /// The lazy half of playlist lyrics: the track need not be playing or even
    /// queued, so browsing a playlist warms the cache one row at a time instead
    /// of scanning the whole list.
    ///
    /// This deliberately does not go through the same path as a real fetch.
    /// `begin_lyrics` claims the generation guard,
    /// clears `lyrics.current` and resets the scroll and the manual sync offset
    /// — all of which belong to the *playing* track. So moving the cursor in a
    /// playlist would replace the now-playing lyrics with the browsed row's,
    /// reset
    /// the user's offset, and make the playing track's own in-flight reply
    /// stale, so it was dropped and the pane stayed blank. Two more: the
    /// prefetch's reply could land between a track change and that track's
    /// fetch, satisfying the guard with the wrong track's lines; and every
    /// cursor move claimed a generation, so a fetch that was genuinely in
    /// flight lost its slot to a row the user merely glanced at.
    ///
    /// The daemon writes the cache on the request side, so the reply is only
    /// ever needed to populate the pane. Discarding it costs nothing.
    pub(crate) fn prefetch_row_lyrics(&mut self, track: &SpotifyTrack) {
        if track.artists.trim().is_empty() || track.name.trim().is_empty() {
            return;
        }
        let client = self.client.clone();
        let artist = track.artists.trim().to_string();
        let title = track.name.trim().to_string();
        let album = track.album.clone();
        let duration = track.duration_ms.map(|ms| ms as f64 / 1000.0);
        tokio::spawn(async move {
            // The result is the cache write, not the reply.
            let _ = client
                .lyrics()
                .search(&artist, &title, album.as_deref(), duration)
                .await;
        });
    }

    fn search_lyrics(
        &mut self,
        artist: &str,
        title: &str,
        album: Option<&str>,
        duration: Option<f64>,
    ) {
        let (artist, title) = (artist.to_string(), title.to_string());
        let album = album.map(str::to_string);
        let fetch_gen = self.begin_lyrics();
        let client = self.client.clone();
        let ipc_tx = self.ipc_tx.clone();
        let path = self.path_display.clone().unwrap_or_default();
        tokio::spawn(async move {
            report(
                client
                    .lyrics()
                    .search(&artist, &title, album.as_deref(), duration)
                    .await,
                &ipc_tx,
                fetch_gen,
                path,
            );
        });
    }

    fn library_lyrics(&mut self, track_id: i64, path: Option<String>) {
        let fetch_gen = self.begin_lyrics();
        let for_path = path.clone().unwrap_or_default();
        let client = self.client.clone();
        let ipc_tx = self.ipc_tx.clone();
        tokio::spawn(async move {
            let out = match tokio::time::timeout(
                Duration::from_secs(12),
                client.lyrics().get(track_id, path.as_deref()),
            )
            .await
            {
                Ok(res) => res,
                Err(_) => Err(CoreError::Daemon("lyrics fetch timed out".into())),
            };
            report(out, &ipc_tx, fetch_gen, for_path);
        });
    }

    /// Clear the pane and arm the generation guard for one fetch.
    fn begin_lyrics(&mut self) -> u64 {
        let fetch_gen = self.next_lyrics_gen();
        self.lyrics.current = None;
        self.lyrics.kind = LyricsKind::None;
        self.lyrics.pending_gen = Some(fetch_gen);
        self.lyrics.fetching = true;
        self.lyrics.scroll = 0;
        self.lyrics.row = None;
        fetch_gen
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

    #[test]
    fn a_podcast_row_names_its_feed_and_episode() {
        assert_eq!(
            podcast_episode("podcast://0123456789abcdef/7"),
            Some(("0123456789abcdef".to_string(), 7))
        );
        // The daemon's own parser treats a missing index as episode zero.
        assert_eq!(
            podcast_episode("podcast://feed"),
            Some(("feed".to_string(), 0))
        );
        assert_eq!(podcast_episode("podcast:///3"), None);
        assert_eq!(podcast_episode("podcast://feed/not-a-number"), None);
        assert_eq!(podcast_episode("https://example.com/episode.mp3"), None);
    }
}
