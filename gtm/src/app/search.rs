use crate::app::*;

impl App {
    /// Virtual action rows (Play All / Shuffle) prepended to a Spotify playlist
    /// drill-down track list.
    pub const SPOTIFY_PLAYLIST_ROWS: usize = 2;

    /// Shared guard text for "add" actions that need a playlist drilled down.
    pub const NEED_PLAYLIST_FOR_ADD: &'static str = "Open the playlist first to add its tracks";

    /// Shared guard text for "remove" actions that only work in a playlist view.
    pub const PLAYLIST_VIEW_ONLY_REMOVE: &'static str =
        "Remove from list only available in playlist view";

    /// Shown when a queue-mutating key is pressed while a live stream plays. A
    /// station's tracklist is a view of what is on air, not a queue, so it
    /// cannot be reordered, cleared or added to.
    pub const RADIO_QUEUE_LOCKED: &'static str = "Queue is read-only while a station is playing";
}

impl App {
    /// Fuzzy-finder rows for the SearchLibrary picker, filtered by the
    /// picker's active `PickerSource` and query.
    pub fn search_library_picks(&self) -> Vec<LibraryPick> {
        let Some(top) = self.pickers.top() else {
            return Vec::new();
        };
        let q = top.query.to_lowercase();
        let mut picks = Vec::new();
        match top.source {
            PickerSource::Tracks | PickerSource::All => {
                for (i, t) in self.tracks_cache.iter().enumerate() {
                    if q.is_empty()
                        || t.title.to_lowercase().contains(&q)
                        || t.artist.to_lowercase().contains(&q)
                        || t.album.to_lowercase().contains(&q)
                    {
                        picks.push(LibraryPick::Track(i));
                    }
                }
            }
            _ => {}
        }
        if matches!(top.source, PickerSource::Artists | PickerSource::All) {
            let mut seen = std::collections::HashSet::new();
            for t in &self.tracks_cache {
                if t.artist.is_empty() || !seen.insert(t.artist.to_lowercase()) {
                    continue;
                }
                if q.is_empty() || t.artist.to_lowercase().contains(&q) {
                    picks.push(LibraryPick::Artist(t.artist.clone()));
                }
            }
        }
        if matches!(top.source, PickerSource::Albums | PickerSource::All) {
            let mut seen = std::collections::HashSet::new();
            for t in &self.tracks_cache {
                if t.album.is_empty() || !seen.insert(t.album.to_lowercase()) {
                    continue;
                }
                if q.is_empty() || t.album.to_lowercase().contains(&q) {
                    picks.push(LibraryPick::Album(t.album.clone()));
                }
            }
        }
        if matches!(top.source, PickerSource::Playlists | PickerSource::All) {
            for (i, p) in self.playlist_cache.iter().enumerate() {
                if q.is_empty() || p.name.to_lowercase().contains(&q) {
                    picks.push(LibraryPick::Playlist(i));
                }
            }
        }
        if matches!(top.source, PickerSource::Radio | PickerSource::All) {
            for (i, s) in self.radio.custom.iter().enumerate() {
                if q.is_empty() || s.name.to_lowercase().contains(&q) {
                    picks.push(LibraryPick::Radio(i));
                }
            }
        }
        picks
    }

    /// Kick off data fetches right after a remote-service picker opens.
    pub fn on_picker_opened(&mut self, id: PickerId) {
        match id {
            PickerId::SpotifyDest => {
                // The destination filter is scoped to this picker, so a query
                // left over from a previous open must not hide every row.
                if let Some(top) = self.pickers.top_mut() {
                    top.query.clear();
                    top.selected = 0;
                    top.viewport_offset = 0;
                }
            }
            PickerId::SpotifySearch => {
                // Reopening must not inherit a spinner from a search that was
                // abandoned when the picker closed.
                self.spotify.search_loading = false;
                // Alt+s on an unlinked account hands off to the client-id
                // form instead of auto-starting the OAuth flow: the picker
                // must show the client-ID input first (Enter starts the flow).
                if self.spotify.status.as_ref().is_none_or(|s| !s.linked) {
                    self.close_picker();
                    self.open_spotify_link_form();
                }
            }
            PickerId::PodcastFeeds => {
                self.podcast.feeds_pending = true;
                let c = self.client.clone();
                let ipc_tx = self.ipc_tx.clone();
                tokio::spawn(async move {
                    match c.podcast().feeds().await {
                        Ok(f) => {
                            let _ = ipc_tx.send(IpcResult::PodcastFeeds(f));
                            match c.podcast().status().await {
                                Ok(s) => {
                                    let _ = ipc_tx.send(IpcResult::PodcastStatus(Some(s)));
                                }
                                Err(e) => {
                                    self_err(&ipc_tx, format!("podcast status failed: {e}"));
                                }
                            }
                        }
                        Err(e) => {
                            self_err(&ipc_tx, format!("podcast feeds failed: {e}"));
                        }
                    }
                });
            }
            PickerId::PodcastEpisodes => {
                if let Some(feed_id) = self.podcast.episodes_feed_id.clone() {
                    self.fetch_podcast_episodes(feed_id);
                }
            }
            PickerId::Radio => {
                // Reopening always starts from the merged root view.
                self.radio.section = RadioSection::Root;
                self.seed_radio_picker(false);
            }
            PickerId::Setup => {
                let c = self.client.clone();
                let ipc_tx = self.ipc_tx.clone();
                tokio::spawn(async move {
                    match c.lastfm().status().await {
                        Ok(st) => {
                            let _ = ipc_tx.send(IpcResult::LastfmStatus(Some(st)));
                        }
                        Err(e) => {
                            self_err(&ipc_tx, format!("last.fm status failed: {e}"));
                        }
                    }
                    match c.spotify().status().await {
                        Ok(st) => {
                            let _ = ipc_tx.send(IpcResult::SpotifyStatus(st));
                        }
                        Err(e) => {
                            self_err(&ipc_tx, format!("spotify status failed: {e}"));
                        }
                    }
                });
            }
            PickerId::LastfmAuth => self.refresh_lastfm_status(),
            PickerId::YoutubeSetup => {
                // Seed the form with the currently configured cookie path so
                // the user sees whether a file is set (Enter without edits
                // keeps it, Backspace clears it).
                if let Some(path) = self.cookie_file.clone() {
                    self.setup.youtube_cookie_input = path;
                }
            }
            _ => {}
        }
    }

    /// Raw indices into `LIBRARY_CATEGORIES` that are currently visible,
    /// in the user's configured display order. Indices stay stable so every
    /// hardcoded `library_category == N` comparison keeps working.
    pub fn visible_library_indices(&self) -> Vec<usize> {
        let mut out = Vec::new();
        for name in &self.left_pane_lists {
            if let Some(i) = LIBRARY_CATEGORIES.iter().position(|c| c == name)
                && !out.contains(&i)
            {
                out.push(i);
            }
        }
        if out.is_empty() {
            (0..LIBRARY_CATEGORIES.len()).collect()
        } else {
            out
        }
    }

    /// Fully reset the library-left-pane view to a target category + drill-down.
    /// Clears the list position, multiselect selection and drill-down caches so
    /// a stale Enter/toggle can never act on a row from a previous view.
    pub(crate) fn reset_library_view(&mut self, category: usize, detail: Option<String>) {
        self.browse_detail = detail;
        self.library_category = category.min(LIBRARY_CATEGORIES.len() - 1);
        self.clear_selection();
        self.playlist_tracks_cache.clear();
        self.spotify.playlist_tracks_cache.clear();
        match self.library_category {
            6 => self.refresh_custom_stations(),
            7 => self.fetch_list_tracks(7),
            8 => self.fetch_list_tracks(8),
            9 => self.fetch_list_tracks(9),
            12 => self.fetch_chart_sources(),
            _ => {}
        }
        // Spotify pane: self-heal an empty playlist cache with a single
        // background sync so playlists appear without visiting Settings.
        if self.library_category == 5 {
            self.maybe_auto_sync_spotify();
        }
        self.set_list_pos(0);
    }

    pub fn filtered_tracks(&self) -> Vec<&TrackInfo> {
        if self.library_category == 4 && self.browse_detail.is_some() {
            return self.playlist_tracks_cache.iter().collect();
        }
        if self.browse_detail.is_none() {
            match self.library_category {
                7 => return self.most_played_cache.iter().collect(),
                8 => return self.recently_played_cache.iter().collect(),
                9 => return self.recently_added_cache.iter().collect(),
                _ => {}
            }
        }
        let mut tracks: Vec<&TrackInfo> = self.tracks_cache.iter().collect();
        if !self.search_query.is_empty() {
            let q = self.search_query.to_lowercase();
            tracks.retain(|t| {
                t.title.to_lowercase().contains(&q)
                    || t.artist.to_lowercase().contains(&q)
                    || t.album.to_lowercase().contains(&q)
            });
        }
        if let Some(ref detail) = self.browse_detail {
            // Drill-down must match the browse key exactly: a substring OR
            // across album/artist/title pulls in unrelated tracks (e.g. an
            // album name that appears in another track's title) and misses
            // empty-field keys that `unique_albums`/`unique_artists` render
            // as "Unknown Album"/"Unknown Artist".
            tracks.retain(|t| match self.library_category {
                2 => {
                    let album: &str = if t.album.is_empty() {
                        "Unknown Album"
                    } else {
                        &t.album
                    };
                    album.eq_ignore_ascii_case(detail)
                }
                3 => {
                    let artist: &str = if t.artist.is_empty() {
                        "Unknown Artist"
                    } else {
                        &t.artist
                    };
                    artist.eq_ignore_ascii_case(detail)
                }
                10 => {
                    let genre: &str = if t.genre.is_empty() {
                        "Unknown Genre"
                    } else {
                        &t.genre
                    };
                    genre.eq_ignore_ascii_case(detail)
                }
                11 => folder_dir(&t.path) == detail.as_str(),
                _ => {
                    t.album.eq_ignore_ascii_case(detail)
                        || t.artist.eq_ignore_ascii_case(detail)
                        || t.title.eq_ignore_ascii_case(detail)
                }
            });
        }
        if self.library_category == 1 {
            tracks.retain(|t| t.favourite);
        } else if self.library_category == 5 {
            // Spotify: category renders the synced playlist browser, not a flat
            // TrackInfo list: resolve/play goes through the daemon.
            tracks.clear();
        } else if self.library_category == 6 {
            // Radio: category renders custom stations; rows are virtual and act
            // on radio:// paths, never on the flat TrackInfo list.
            tracks.clear();
        }
        // Sorting applies to the flat track list (All Tracks / Favourites and the
        // album/artist drill-downs). Playlist and Spotify views sort upstream.
        if self.browse_detail.is_none() && self.library_category <= 1 {
            match self.track_sort {
                TrackSort::Recents => {
                    tracks.sort_by(|a, b| b.year.cmp(&a.year).then_with(|| a.title.cmp(&b.title)));
                }
                TrackSort::RecentlyAdded => {
                    tracks.sort_by_key(|a| std::cmp::Reverse(a.id));
                }
                TrackSort::Alphabetical => {
                    tracks.sort_by(|a, b| {
                        a.title
                            .to_lowercase()
                            .cmp(&b.title.to_lowercase())
                            .then_with(|| a.artist.to_lowercase().cmp(&b.artist.to_lowercase()))
                    });
                }
                TrackSort::Artist => {
                    tracks.sort_by(|a, b| {
                        a.artist
                            .to_lowercase()
                            .cmp(&b.artist.to_lowercase())
                            .then_with(|| a.title.to_lowercase().cmp(&b.title.to_lowercase()))
                    });
                }
                TrackSort::Album => {
                    tracks.sort_by(|a, b| {
                        a.album
                            .to_lowercase()
                            .cmp(&b.album.to_lowercase())
                            .then_with(|| {
                                a.track_number
                                    .cmp(&b.track_number)
                                    .then_with(|| a.title.cmp(&b.title))
                            })
                    });
                }
            }
        }
        tracks
    }

    /// Expand the highlighted album/artist row to the ids of every cached track
    /// in that album/artist. Returns `None` in flat views where the highlighted
    /// row maps 1:1 to `filtered_tracks()` (the caller falls back to that list).
    pub(crate) fn motion_row_ids(&self) -> Option<Vec<i64>> {
        match self.library_category {
            2 => {
                let albums = self.unique_albums();
                let (name, _) = albums.get(self.list_pos())?;
                Some(
                    self.tracks_cache
                        .iter()
                        .filter(|t| {
                            let album: &str = if t.album.is_empty() {
                                "Unknown Album"
                            } else {
                                &t.album
                            };
                            album == name
                        })
                        .map(|t| t.id)
                        .collect(),
                )
            }
            3 => {
                let artists = self.unique_artists();
                let (name, _) = artists.get(self.list_pos())?;
                Some(
                    self.tracks_cache
                        .iter()
                        .filter(|t| {
                            let artist: &str = if t.artist.is_empty() {
                                "Unknown Artist"
                            } else {
                                &t.artist
                            };
                            artist == name
                        })
                        .map(|t| t.id)
                        .collect(),
                )
            }
            10 => {
                let genres = self.unique_genres();
                let (name, _) = genres.get(self.list_pos())?;
                Some(
                    self.tracks_cache
                        .iter()
                        .filter(|t| {
                            let genre: &str = if t.genre.is_empty() {
                                "Unknown Genre"
                            } else {
                                &t.genre
                            };
                            genre == name
                        })
                        .map(|t| t.id)
                        .collect(),
                )
            }
            11 => {
                let folders = self.unique_folders();
                let (dir, _) = folders.get(self.list_pos())?;
                Some(
                    self.tracks_cache
                        .iter()
                        .filter(|t| folder_dir(&t.path) == *dir)
                        .map(|t| t.id)
                        .collect(),
                )
            }
            _ => None,
        }
    }

    /// Play the track highlighted in the current library view, replacing the
    /// queue with the filtered list and starting at that row.
    pub(crate) fn play_filtered_highlighted(&self) {
        let filtered = self.filtered_tracks();
        let idx = self.list_pos();
        if idx >= filtered.len() {
            return;
        }
        let paths: Vec<String> = filtered.iter().map(|t| t.path.clone()).collect();
        let path = paths[idx].clone();
        let c = self.client.clone();
        tokio::spawn(async move {
            let _ = c.queue().set(paths, idx as u64).await;
            let _ = c.play(&path, 0.0).await;
        });
    }

    /// Unique album names with track counts, sorted by album.
    pub fn unique_albums(&self) -> Vec<(String, usize)> {
        if let Ok(guard) = self.cached_albums.lock()
            && let Some((cached_gen, cached)) = guard.as_ref()
            && *cached_gen == self.tracks_cache_gen
        {
            return cached.clone();
        }
        let mut albums: std::collections::BTreeMap<String, usize> =
            std::collections::BTreeMap::new();
        for t in &self.tracks_cache {
            let key = if t.album.is_empty() {
                "Unknown Album".into()
            } else {
                t.album.clone()
            };
            *albums.entry(key).or_insert(0) += 1;
        }
        let out: Vec<(String, usize)> = albums.into_iter().collect();
        if let Ok(mut guard) = self.cached_albums.lock() {
            *guard = Some((self.tracks_cache_gen, out.clone()));
        }
        out
    }

    /// Length of the list currently visible in the library right pane,
    /// depending on the active category and drill-down state.
    pub fn library_list_len(&self) -> usize {
        if self.library_category == 12 {
            // Top Charts is a three-level tree: sources / charts / tracks.
            if self.charts.selected_chart.is_some() {
                return self.charts.chart_tracks.len();
            }
            if self.charts.selected_source.is_some() {
                return self.charts.charts.len();
            }
            return self.charts.sources.len();
        }
        if self.browse_detail.is_some() {
            if self.library_category == 5 {
                return self.spotify_playlist_rows();
            }
            return self.filtered_tracks().len();
        }
        match self.library_category {
            2 => self.unique_albums().len(),
            3 => self.unique_artists().len(),
            4 => self.playlist_cache.len(),
            5 => self.spotify.playlists.len(),
            6 => self.radio.custom.len(),
            10 => self.unique_genres().len(),
            11 => self.unique_folders().len(),
            _ => self.filtered_tracks().len(),
        }
    }

    /// The rows the right pane currently lists, as (selection key, play
    /// target, library id). Views whose rows aren't tracks (chart source /
    /// chart levels, Spotify browse, radio stations, playlist overview)
    /// return an empty vector so Select mode can't act on the wrong list.
    pub(crate) fn selectable_rows(&self) -> Vec<(String, String, Option<i64>)> {
        if self.library_category == 12 {
            // Charts Level 2: a chart's tracks, selected by playable URI.
            if self.charts.selected_chart.is_some() {
                return self
                    .charts
                    .chart_tracks
                    .iter()
                    .map(|t| (t.uri.clone(), t.uri.clone(), None))
                    .collect();
            }
            return Vec::new();
        }
        // Spotify browse and radio stations render non-library rows.
        if self.library_category == 5 || self.library_category == 6 {
            return Vec::new();
        }
        // Playlist overview rows are playlists, not tracks.
        if self.library_category == 4 && self.browse_detail.is_none() {
            return Vec::new();
        }
        self.filtered_tracks()
            .iter()
            .map(|t| (t.path.clone(), t.path.clone(), Some(t.id)))
            .collect()
    }

    /// Stable selection key of the row at `index` in the active pane.
    pub(crate) fn select_key_at(&self, index: usize) -> Option<String> {
        self.selectable_rows().get(index).map(|r| r.0.clone())
    }

    /// Play target (path/URI) of the row at `index` in the active pane.
    pub(crate) fn play_target_at(&self, index: usize) -> Option<String> {
        self.selectable_rows().get(index).map(|r| r.1.clone())
    }

    /// True when the row identified by `key` is part of the Select-mode
    /// selection. Used by the renderer so the highlight follows the stable
    /// selection even if the visible list shifts.
    pub fn row_is_selected(&self, key: &str) -> bool {
        self.selected_keys.contains(key)
    }

    /// Number of currently selected rows.
    pub fn selected_count(&self) -> usize {
        self.selected_keys.len()
    }

    /// Toggle the row at `index` (Tab in Select mode). Rows without a
    /// selectable key (non-track views) are ignored.
    pub(crate) fn toggle_row(&mut self, index: usize) {
        if let Some(key) = self.select_key_at(index)
            && !self.selected_keys.remove(&key)
        {
            self.selected_keys.insert(key);
        }
    }

    /// Add the row at `index` to the selection without toggling.
    pub(crate) fn add_row_selection(&mut self, index: usize) {
        if let Some(key) = self.select_key_at(index) {
            self.selected_keys.insert(key);
        }
    }

    /// Clear the Select-mode selection entirely.
    pub(crate) fn clear_selection(&mut self) {
        self.selected_keys.clear();
    }

    /// Play targets of every selected row, re-resolved against the active
    /// pane at call time. Stale keys (rows no longer visible) drop out, so
    /// the operation acts exactly on what is still there.
    pub(crate) fn selected_play_targets(&self) -> Vec<String> {
        if self.selected_keys.is_empty() {
            return Vec::new();
        }
        self.selectable_rows()
            .into_iter()
            .filter(|(key, _, _)| self.selected_keys.contains(key))
            .map(|(_, target, _)| target)
            .collect()
    }

    /// Library ids of every selected row. Rows without a library id (streamed
    /// chart tracks) are skipped.
    pub(crate) fn selected_library_ids(&self) -> Vec<i64> {
        if self.selected_keys.is_empty() {
            return Vec::new();
        }
        self.selectable_rows()
            .into_iter()
            .filter(|(key, _, _)| self.selected_keys.contains(key))
            .filter_map(|(_, _, id)| id)
            .collect()
    }

    /// Track ids owned by the list position `pos` (when that row maps to a
    /// concrete track). Returns `None` for album/artist/playlist/spotify rows
    /// whose cover is derived from a different key.
    pub(crate) fn track_id_at(&self, pos: usize) -> Option<i64> {
        if self.browse_detail.is_some() {
            // Spotify drill-down rows are remote tracks (no local id to warm
            // the disk-cache with); only local track rows have an id to preload.
            if self.library_category != 5 {
                return self.filtered_tracks().get(pos).map(|t| t.id);
            }
            return None;
        }
        match self.library_category {
            0 | 1 => self.filtered_tracks().get(pos).map(|t| t.id),
            _ => None,
        }
    }

    /// Unique artist names with track counts, sorted by artist.
    pub fn unique_artists(&self) -> Vec<(String, usize)> {
        if let Ok(guard) = self.cached_artists.lock()
            && let Some((cached_gen, cached)) = guard.as_ref()
            && *cached_gen == self.tracks_cache_gen
        {
            return cached.clone();
        }
        let mut artists: std::collections::BTreeMap<String, usize> =
            std::collections::BTreeMap::new();
        for t in &self.tracks_cache {
            let key = if t.artist.is_empty() {
                "Unknown Artist".into()
            } else {
                t.artist.clone()
            };
            *artists.entry(key).or_insert(0) += 1;
        }
        let out: Vec<(String, usize)> = artists.into_iter().collect();
        if let Ok(mut guard) = self.cached_artists.lock() {
            *guard = Some((self.tracks_cache_gen, out.clone()));
        }
        out
    }

    pub fn unique_genres(&self) -> Vec<(String, usize)> {
        if let Ok(guard) = self.cached_genres.lock()
            && let Some((cached_gen, cached)) = guard.as_ref()
            && *cached_gen == self.tracks_cache_gen
        {
            return cached.clone();
        }
        let mut genres: std::collections::BTreeMap<String, usize> =
            std::collections::BTreeMap::new();
        for t in &self.tracks_cache {
            let key = if t.genre.is_empty() {
                "Unknown Genre".into()
            } else {
                t.genre.clone()
            };
            *genres.entry(key).or_insert(0) += 1;
        }
        let out: Vec<(String, usize)> = genres.into_iter().collect();
        if let Ok(mut guard) = self.cached_genres.lock() {
            *guard = Some((self.tracks_cache_gen, out.clone()));
        }
        out
    }

    pub fn unique_folders(&self) -> Vec<(String, usize)> {
        if let Ok(guard) = self.cached_folders.lock()
            && let Some((cached_gen, cached)) = guard.as_ref()
            && *cached_gen == self.tracks_cache_gen
        {
            return cached.clone();
        }
        let mut folders: std::collections::BTreeMap<String, usize> =
            std::collections::BTreeMap::new();
        for t in &self.tracks_cache {
            let dir = folder_dir(&t.path);
            *folders.entry(dir).or_insert(0) += 1;
        }
        let out: Vec<(String, usize)> = folders.into_iter().collect();
        if let Ok(mut guard) = self.cached_folders.lock() {
            *guard = Some((self.tracks_cache_gen, out.clone()));
        }
        out
    }

    /// Interleave YT search results: insert one playlist entry after every 3 track entries.
    pub(crate) fn interleave_yt_results(mut results: Vec<YTSearchResult>) -> Vec<YTSearchResult> {
        let tracks: Vec<_> = results.drain(..).filter(|r| !r.is_playlist).collect();
        let playlists: Vec<_> = results; // remaining are playlists
        let mut out = Vec::with_capacity(tracks.len() + playlists.len());
        let mut pl_idx = 0;
        for (i, track) in tracks.into_iter().enumerate() {
            out.push(track);
            if (i + 1) % 3 == 0 && pl_idx < playlists.len() {
                out.push(playlists[pl_idx].clone());
                pl_idx += 1;
            }
        }
        // Append remaining playlists
        while pl_idx < playlists.len() {
            out.push(playlists[pl_idx].clone());
            pl_idx += 1;
        }
        out
    }
}
