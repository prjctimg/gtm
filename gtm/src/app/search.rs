use crate::app::*;
use crate::providers::charts::ChartTrack;

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
            PickerId::Queue => {
                // A station's tracklist is mirrored in `state` off the daemon's
                // refresh tick, which may not have landed yet on the first open
                // after pressing play. Pull it directly in that case so the
                // queue never shows the user queue for a live stream that does
                // publish one.
                self.fetch_live_list();
            }
            PickerId::Libraries => {
                // Open on the category already showing, so the picker is a
                // view of the list rather than a jump back to the top. Read
                // through the filtered list so a lingering query cannot leave
                // the cursor on a row the picker is not drawing.
                if let Some(row) = self
                    .filtered_library_indices()
                    .iter()
                    .position(|&i| i == self.library_category)
                    && let Some(top) = self.pickers.top_mut()
                {
                    top.selected = row;
                    top.viewport_offset = 0;
                }
            }
            PickerId::DiscordSetup => {
                // Seed from the saved id so the form shows what is set rather
                // than starting blank and inviting an overwrite.
                if let Some(top) = self.pickers.top_mut() {
                    top.query.clear();
                }
                self.setup.discord_input = self.discord_id.clone().unwrap_or_default();
            }
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
                    self.open_spot_link();
                }
            }
            PickerId::PodcastFeeds => {
                self.fetch_podcast_feeds();
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

    /// The library categories the picker is currently showing: the configured
    /// lists, narrowed by the picker's search query.
    ///
    /// Every consumer of the picker's rows goes through here — the count the
    /// cursor is clamped against, the renderer, and the Enter handler. Those
    /// three used to derive the list independently, and when the query narrowed
    /// what was drawn but not what Enter opened, a filtered picker highlighted
    /// one category and opened another.
    pub fn filtered_library_indices(&self) -> Vec<usize> {
        // Only the library picker's own query filters it. Another picker on top
        // of the stack owns `query` and must not narrow this list.
        let query = match self.pickers.top() {
            Some(t) if t.id == PickerId::Libraries => t.query.as_str(),
            _ => "",
        };
        self.visible_library_indices()
            .into_iter()
            .filter(|&i| fuzzy_match(query, LIBRARY_CATEGORIES[i]))
            .collect()
    }

    /// Item count shown next to a library category. `Top Charts` and anything
    /// unmapped counts as zero, which the picker renders as no count at all.
    pub fn library_count(&self, cat: &str) -> usize {
        match cat {
            "All Tracks" => self.tracks_cache.len(),
            "Liked" => self.tracks_cache.iter().filter(|t| t.favourite).count(),
            "Albums" => self.unique_albums().len(),
            "Artists" => self.unique_artists().len(),
            "Playlists" => self.playlist_cache.len(),
            "Spotify" => self.spotify.playlists.len(),
            "Radio" => self.radio.custom.len(),
            "Most Played" => self.most_played_cache.len(),
            "Recently Played" => self.recently_played_cache.len(),
            "Recently Added" => self.recently_added_cache.len(),
            "Genres" => self.unique_genres().len(),
            "Folders" => self.unique_folders().len(),
            "Podcasts" => self.podcast.feeds.len(),
            _ => 0,
        }
    }

    /// Fully reset the library-left-pane view to a target category + drill-down.
    /// Clears the list position, multiselect selection and drill-down caches so
    /// a stale Enter/toggle can never act on a row from a previous view.
    /// Enter a library category, optionally drilled into `detail`.
    ///
    /// `title` is the display name when the detail is not one — see
    /// [`App::browse_title`]. Passing `None` for it is correct for every
    /// category except Spotify.
    pub(crate) fn reset_library_view(&mut self, category: usize, detail: Option<String>) {
        self.reset_library_titled(category, detail, None);
    }

    /// [`Self::reset_library_view`], with an explicit display name.
    pub(crate) fn reset_library_titled(
        &mut self,
        category: usize,
        detail: Option<String>,
        title: Option<String>,
    ) {
        self.browse_detail = detail;
        self.browse_title = title;
        self.library_category = category.min(LIBRARY_CATEGORIES.len() - 1);
        self.clear_selection();
        self.playlist_tracks_cache.clear();
        // Leaving the drill-down must not leave its cover bound to a row that
        // is no longer selected, and arriving at a new category must not leave
        // the previous one's playlist art behind.
        self.clear_row_cover();
        self.clear_list_cover();
        // Entering the Podcasts category must land on the feed list, not on
        // whatever feed was drilled into last time — the episode list belongs
        // to the feed it was opened from, and re-entering the category is a
        // fresh start rather than a resume.
        if self.library_category == 13 {
            self.podcast.episodes.clear();
            self.podcast.episodes_feed_id = None;
        }
        if self.in_spotify_playlists() {
            self.fetch_list_cover();
        }
        self.spotify.playlist_tracks_cache.clear();
        match self.library_category {
            6 => self.refresh_custom_stations(),
            7 => self.fetch_list_tracks(7),
            8 => self.fetch_list_tracks(8),
            9 => self.fetch_list_tracks(9),
            12 => self.fetch_chart_sources(),
            13 => self.fetch_podcast_feeds(),
            _ => {}
        }
        // Spotify pane: self-heal an empty playlist cache with a single
        // background sync so playlists appear without visiting Settings.
        if self.library_category == 5 {
            self.auto_sync_spotify();
        }
        self.set_list_pos(0);
        // Build the card for the category just entered. Charts and Radio had no
        // card at all, and the others waited for the first cursor move to show
        // one, so arriving at a list showed an empty pane until you touched the
        // keys. Radio is read from disk synchronously, so it is ready here.
        self.update_track_popup();
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

    /// Play one chart row. Returns why it could not be played, if it could not.
    pub(crate) async fn play_chart(c: &DaemonClient, track: ChartTrack) -> Result<(), String> {
        Self::resolve_chart(c, track, true).await
    }

    /// Queue one chart row, without starting it. Same routing as
    /// [`App::play_chart`].
    pub(crate) async fn queue_chart(c: &DaemonClient, track: ChartTrack) -> Result<(), String> {
        Self::resolve_chart(c, track, false).await
    }

    /// Resolve a chart row to a playable Spotify track and play or queue it.
    ///
    /// A chart row's `uri` is whatever its provider happens to publish, so the
    /// two cases are genuinely different and were both wrong:
    ///
    /// * `spotify:track:<id>` — a provider resource, not a path. `queue().add()`
    ///   treats its argument as a filesystem path, so it went to the library
    ///   route: a directory expansion, no Premium streaming, and a row titled
    ///   "Spotify Track". The resolver takes the same metadata the chart already
    ///   has and streams it, or falls back to a download.
    /// * anything else — an Apple chart row carries a `preview_url`, which is
    ///   neither a track identifier nor playable, and many carry none at all.
    ///   The row does know the title and the artist, so the track is matched on
    ///   those and then resolved the same way, which lands on the same Spotify
    ///   track the other rows would.
    ///
    /// A row with neither a uri nor a usable title cannot be played, and says
    /// so instead of queueing a row that silently refuses to start.
    async fn resolve_chart(c: &DaemonClient, track: ChartTrack, play: bool) -> Result<(), String> {
        let ChartTrack {
            title,
            artists,
            album,
            cover_url,
            uri,
            ..
        } = track;
        if uri.starts_with("spotify:") {
            return c
                .spotify()
                .resolve_track(
                    &title,
                    &artists,
                    album.as_deref().unwrap_or(""),
                    Some(uri),
                    cover_url,
                    play,
                )
                .await
                .map_err(|e| e.to_string());
        }
        if title.trim().is_empty() {
            return Err("this row carries neither a provider URI nor a title".into());
        }
        let query = if artists.trim().is_empty() {
            title.clone()
        } else {
            format!("{artists} - {title}")
        };
        let uri = c
            .spotify()
            .match_track(&query)
            .await
            .map_err(|e| format!("no Spotify match for {query:?}: {e}"))?;
        c.spotify()
            .resolve_track(
                &title,
                &artists,
                album.as_deref().unwrap_or(""),
                Some(uri),
                cover_url,
                play,
            )
            .await
            .map_err(|e| e.to_string())
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
                    .enumerate()
                    .map(|(i, t)| (App::chart_row_key(i, t), t.uri.clone(), None))
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

    /// Stable selection key of one chart row.
    ///
    /// The uri is the natural identity, but an Apple Music row without a
    /// `preview_url` has an empty one — and every such row therefore shared the
    /// key `""`, so `toggle_row` could only ever select the first of them and
    /// multiselect on an Apple chart silently dropped the rest. The position
    /// disambiguates.
    fn chart_row_key(index: usize, t: &ChartTrack) -> String {
        if t.uri.is_empty() {
            format!("chart-row:{index}")
        } else {
            t.uri.clone()
        }
    }

    /// Queue a set of rows through whichever route each one needs, and report
    /// how many landed.
    ///
    /// Every "add to queue" entry point funnels through here: the `a` key, the
    /// multiselect prompt, and the command palette. They all used to hand their
    /// targets straight to `queue().add()`, which reads a filesystem path — so
    /// a chart row carrying a `spotify:` uri went to the library route, and one
    /// carrying no uri at all went to the empty path. Neither is a queue entry.
    pub(crate) fn queue_rows(&self, keys: Vec<String>) -> usize {
        if self.library_category == 12 && self.charts.selected_chart.is_some() {
            let rows = self.selectable_rows();
            let picked: Vec<ChartTrack> = keys
                .iter()
                .filter_map(|k| {
                    let pos = rows.iter().position(|(key, ..)| key == k)?;
                    self.charts.chart_tracks.get(pos).cloned()
                })
                .collect();
            if picked.is_empty() {
                return 0;
            }
            let c = self.client.clone();
            let ipc = self.ipc_tx.clone();
            let n = picked.len();
            tokio::spawn(async move {
                for t in picked {
                    if let Err(e) = App::queue_chart(&c, t).await {
                        let _ = ipc.send(IpcResult::Error(format!("Could not queue: {e}")));
                    }
                }
            });
            return n;
        }
        let c = self.client.clone();
        let n = keys.len();
        tokio::spawn(async move {
            for target in keys {
                if !target.is_empty() {
                    let _ = c.queue().add(&target, None).await;
                }
            }
        });
        n
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
