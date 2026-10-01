use crate::app::*;

impl App {
    /// Kind of item the library track-info block is currently describing,
    /// derived from the active list and drill-down state.
    pub fn track_info_kind(&self) -> TrackInfoKind {
        if self.browse_detail.is_some() {
            if self.library_category == 5 {
                return TrackInfoKind::SpotifyTrack;
            }
            return TrackInfoKind::Track;
        }
        match self.library_category {
            2 => TrackInfoKind::Album,
            3 => TrackInfoKind::Artist,
            4 => TrackInfoKind::Playlist,
            5 => TrackInfoKind::SpotifyPlaylist,
            6 => TrackInfoKind::RadioStation,
            // Charts have their own row type. Falling through to `Track` is
            // what made the left card describe a random local library track:
            // `filtered_tracks` has no chart case, so it returned the whole
            // library and the card rendered `tracks_cache[list_pos()]`.
            //
            // Which chart row depends on how deep the drill-down is. It used to
            // be `ChartTrack` at every level, and the fields for that kind read
            // `chart_tracks`, which is empty until a chart is opened: so the
            // source list and the chart list had no card at all.
            12 => match (self.charts.selected_source, self.charts.selected_chart) {
                (None, _) => TrackInfoKind::ChartSource,
                (Some(_), None) => TrackInfoKind::Chart,
                (Some(_), Some(_)) => TrackInfoKind::ChartTrack,
            },
            _ => TrackInfoKind::Track,
        }
    }

    /// Update the track popup to describe the currently selected row in the
    /// library list, context aware of the active list type.
    /// Tracks, albums and artists resolve a representative track so cover art
    /// can be fetched; playlist and Spotify rows show meta only.
    pub fn update_track_popup(&mut self) {
        let kind = self.track_info_kind();
        let maybe_track: Option<(i64, String)> = match kind {
            TrackInfoKind::Track => {
                let filtered = self.filtered_tracks();
                let pos = self.list_pos();
                filtered.get(pos).map(|t| (t.id, t.path.clone()))
            }
            TrackInfoKind::Album => {
                let albums = self.unique_albums();
                let pos = self.list_pos();
                albums.get(pos).and_then(|(name, _)| {
                    self.tracks_cache
                        .iter()
                        .find(|t| {
                            let album: &str = if t.album.is_empty() {
                                "Unknown Album"
                            } else {
                                &t.album
                            };
                            album == name
                        })
                        .map(|t| (t.id, t.path.clone()))
                })
            }
            TrackInfoKind::Artist => {
                let artists = self.unique_artists();
                let pos = self.list_pos();
                artists.get(pos).and_then(|(name, _)| {
                    self.tracks_cache
                        .iter()
                        .find(|t| {
                            let artist: &str = if t.artist.is_empty() {
                                "Unknown Artist"
                            } else {
                                &t.artist
                            };
                            artist == name
                        })
                        .map(|t| (t.id, t.path.clone()))
                })
            }
            // Playlist and Spotify rows never resolve a local cover; the block
            // still describes the selected row. A chart row resolves its own
            // from `cover_url` below.
            TrackInfoKind::Playlist
            | TrackInfoKind::SpotifyPlaylist
            | TrackInfoKind::SpotifyTrack
            | TrackInfoKind::ChartTrack
            | TrackInfoKind::ChartSource
            | TrackInfoKind::Chart
            | TrackInfoKind::RadioStation => None,
        };

        let valid = match kind {
            TrackInfoKind::Playlist => self.list_pos() < self.playlist_cache.len(),
            TrackInfoKind::SpotifyPlaylist => self.list_pos() < self.spotify.playlists.len(),
            TrackInfoKind::SpotifyTrack => self.selected_spotify_track().is_some(),
            TrackInfoKind::ChartSource => self.list_pos() < self.charts.sources.len(),
            TrackInfoKind::Chart => self.list_pos() < self.charts.charts.len(),
            TrackInfoKind::ChartTrack => self.list_pos() < self.charts.chart_tracks.len(),
            TrackInfoKind::RadioStation => self.list_pos() < self.radio.custom.len(),
            _ => maybe_track.is_some(),
        };

        self.track_popup_visible = valid;
        if !valid {
            self.clear_popup_cover();
            return;
        }

        if kind == TrackInfoKind::SpotifyTrack {
            // Spotify drill-down rows: the cover is the selected track's
            // album-image URL (no local library id), fetched on every cursor
            // move so scrolling the list loads cover art.
            self.popup_track_id = None;
            self.fetch_spotify_popup_cover();
            return;
        }

        // A chart's own artwork and a chart row's are the same kind of thing: a
        // plain CDN URL from whichever provider published the chart, so it goes
        // through the provider-neutral image request rather than Spotify's — an
        // Apple chart row has to work with no provider linked at all. A station
        // resolves to no URL and clears the cover; see `popup_cover_url`.
        if matches!(
            kind,
            TrackInfoKind::Chart | TrackInfoKind::ChartTrack | TrackInfoKind::RadioStation
        ) {
            self.popup_track_id = None;
            self.fetch_url_cover(self.popup_cover_url());
            return;
        }

        let Some((tid, path)) = maybe_track else {
            self.clear_popup_cover();
            return;
        };
        self.popup_track_id = Some(tid);

        let current_is_selected = self
            .state
            .current_track
            .as_ref()
            .is_some_and(|t| t.path == path);
        if current_is_selected {
            // Robust fallback: if current track's art is still pending (cleared
            // on track change), fall through to fetch rather than showing blank
            //. Only reuse when we actually have bytes.
            if let Some(cover) = self.np_cover.image.clone() {
                self.track_popup_cover = Some(cover);
                self.popup_cover_sync();
                self.popup_slot.clear();
                return;
            }
            // else fall through to fetch below
        }
        // One in-flight fetch per track; `id == 0` reuse is safe because the
        // generation is what decides whether a reply is current.
        if no_image_protocol() || self.popup_slot.pending(&tid) {
            return;
        }
        let fetch_gen = self.next_cover_gen();
        self.popup_slot.claim(tid, fetch_gen);
        self.track_popup_cover = None;
        self.popup_cover_stateful = None;
        let client = self.client.clone();
        let ipc_tx = self.ipc_tx.clone();
        tokio::spawn(async move {
            // Answer on a miss too: a silent failure leaves `popup_slot`
            // claimed, and the handler treats a claimed slot as "in flight", so
            // the row could never be re-fetched for the rest of the session.
            let bytes = match client.art().cover(tid).await {
                Ok(Some(b64)) => base64::engine::general_purpose::STANDARD.decode(&b64).ok(),
                _ => None,
            };
            let msg = match bytes {
                Some(bytes) => IpcResult::PopupCoverArt(Some(bytes), tid, fetch_gen),
                None => IpcResult::PopupCoverArt(None, tid, fetch_gen),
            };
            let _ = ipc_tx.send(msg);
        });
    }

    /// Dismiss the track popup.
    pub fn dismiss_track_popup(&mut self) {
        self.track_popup_visible = false;
        self.clear_popup_cover();
    }

    /// Artwork for the highlighted row, for the kinds that name their image by
    /// URL rather than by library id.
    ///
    /// A station resolves to nothing: the left-pane Radio rows are
    /// `CustomRadioStation`, which carries a `uuid` but no icon, and turning
    /// that into a favicon means a directory lookup per row. The card describes
    /// the station in text instead.
    fn popup_cover_url(&self) -> Option<String> {
        let pos = self.list_pos();
        match self.track_info_kind() {
            TrackInfoKind::Chart => self.charts.charts.get(pos)?.cover_url.clone(),
            TrackInfoKind::ChartTrack => self.charts.chart_tracks.get(pos)?.cover_url.clone(),
            _ => None,
        }
    }

    /// Fetch a popup cover named by URL, through the same slot Spotify
    /// drill-down rows use.
    ///
    /// The slot is keyed on the URL, which is what makes the latch work: two
    /// rows on the same album share an image URL, so scrolling between them
    /// costs nothing, and a different row misses and refetches.
    pub(crate) fn fetch_url_cover(&mut self, url: Option<String>) {
        let Some(url) = url else {
            self.clear_popup_cover();
            return;
        };
        if self.spotify_popup_slot.id.as_deref() == Some(&url)
            && self.spotify_popup_slot.version.is_some()
        {
            return;
        }
        if no_image_protocol() {
            return;
        }
        let fetch_gen = self.next_cover_gen();
        self.spotify_popup_slot.claim(url.clone(), fetch_gen);
        self.track_popup_cover = None;
        self.popup_cover_stateful = None;
        let client = self.client.clone();
        let ipc_tx = self.ipc_tx.clone();
        tokio::spawn(async move {
            // A miss answers with `None` rather than staying silent: an
            // unanswered fetch leaves the slot claimed and every later cover
            // for this row dropped.
            let bytes = client.image_cover(&url).await.ok().flatten();
            let _ = ipc_tx.send(IpcResult::SpotifyPopupCover(bytes, url, fetch_gen));
        });
    }

    /// Warm the artwork for the rows of a loaded chart that are on screen or
    /// just off it.
    ///
    /// The card shows one row's cover at a time, so without this every step of
    /// the scroll is a blank card until its request comes back — the list moves
    /// faster than the network. The daemon's image cache absorbs the repeats, so
    /// the fetch the card then makes is a hit. Mirrors the local-library warm in
    /// for the local library, which has ids rather than URLs.
    pub fn preload_chart_covers(&self) {
        if no_image_protocol() || self.charts.chart_tracks.is_empty() {
            return;
        }
        let sel = self.list_pos();
        let from = sel.saturating_sub(1);
        let to = (sel + 3).min(self.charts.chart_tracks.len());
        let urls: Vec<String> = self.charts.chart_tracks[from..to]
            .iter()
            .filter_map(|t| t.cover_url.clone())
            .collect();
        if urls.is_empty() {
            return;
        }
        let client = self.client.clone();
        tokio::spawn(async move {
            for url in urls {
                // A miss is a cache miss like any other; warming is best-effort
                // and never reports.
                let _ = client.image_cover(&url).await;
            }
        });
    }

    /// Fetch cover art for the highlighted SearchLibrary picker row so the
    /// preview window can render the actual album art as ASCII.
    pub fn update_picker_preview(&mut self) {
        let Some(top) = self.pickers.top() else {
            // Robust: invalidate pending fetch_gen so close/reopen does not retain stale key
            self.picker_preview_cover = None;
            self.picker_preview_stateful = None;
            self.picker_slot.clear();
            return;
        };
        if top.id != PickerId::SearchLibrary {
            self.picker_preview_cover = None;
            self.picker_preview_stateful = None;
            self.picker_slot.clear();
            return;
        }
        let picks = self.search_library_picks();
        if picks.is_empty() {
            self.picker_preview_cover = None;
            self.picker_preview_stateful = None;
            self.picker_slot.clear();
            return;
        }
        let sel = top.selected.min(picks.len() - 1);
        let LibraryPick::Track(i) = &picks[sel] else {
            self.picker_preview_cover = None;
            self.picker_preview_stateful = None;
            self.picker_slot.clear();
            return;
        };
        let tid = self.tracks_cache[*i].id;
        // Generation-guarded dedup: id reuse (id==0) cannot block new fetches.
        // Only skip when both id and generation match current pending.
        if self.picker_slot.pending(&tid) {
            return;
        }
        let fetch_gen = self.next_cover_gen();
        self.picker_slot.claim(tid, fetch_gen);
        self.picker_preview_cover = None;
        self.picker_preview_stateful = None;
        if no_image_protocol() {
            return;
        }
        let client = self.client.clone();
        let ipc_tx = self.ipc_tx.clone();
        tokio::spawn(async move {
            // Answer on a miss so `picker_slot` is released; see the popup
            // fetch above for why a silent failure is not self-clearing.
            let bytes = match client.art().cover(tid).await {
                Ok(Some(b64)) => base64::engine::general_purpose::STANDARD.decode(&b64).ok(),
                _ => None,
            };
            let msg = match bytes {
                Some(bytes) => IpcResult::PickerPreviewCover(Some(bytes), tid, fetch_gen),
                None => IpcResult::PickerPreviewCover(None, tid, fetch_gen),
            };
            let _ = ipc_tx.send(msg);
        });
    }

    pub fn update_artist_cover(&mut self) {
        let Some(top) = self.pickers.top() else {
            self.artist_cover = None;
            self.artist_cover_stateful = None;
            self.artist_slot.clear();
            return;
        };
        if top.id != PickerId::SearchLibrary {
            self.artist_cover = None;
            self.artist_cover_stateful = None;
            self.artist_slot.clear();
            return;
        }
        let picks = self.search_library_picks();
        if picks.is_empty() {
            self.artist_cover = None;
            self.artist_cover_stateful = None;
            self.artist_slot.clear();
            return;
        }
        let sel = top.selected.min(picks.len() - 1);
        let LibraryPick::Artist(name) = &picks[sel] else {
            self.artist_cover = None;
            self.artist_cover_stateful = None;
            self.artist_slot.clear();
            return;
        };
        if self.artist_slot.pending(name) {
            return;
        }
        let fetch_gen = self.next_cover_gen();
        self.artist_slot.claim(name.clone(), fetch_gen);
        self.artist_cover = None;
        self.artist_cover_stateful = None;
        if no_image_protocol() {
            return;
        }
        let client = self.client.clone();
        let ipc_tx = self.ipc_tx.clone();
        let artist = name.clone();
        tokio::spawn(async move {
            // Answer on a miss so `artist_slot` is released.
            let bytes = match client.art().artist_cover(artist.clone()).await {
                Ok(Some(b64)) => base64::engine::general_purpose::STANDARD.decode(&b64).ok(),
                _ => None,
            };
            let msg = match bytes {
                Some(bytes) => IpcResult::ArtistCoverArt(Some(bytes), artist, fetch_gen),
                None => IpcResult::ArtistCoverArt(None, artist, fetch_gen),
            };
            let _ = ipc_tx.send(msg);
        });
    }

    /// Preload the cover art for the tracks a short scroll ahead of the cursor
    /// so fast scrolling (e.g. holding an arrow key) warms the daemon's
    /// disk/LRU cache and the on-selection fetch becomes a cache hit. Fires in
    /// the background and never blocks the UI or surfaces errors. Also warms
    /// Spotify drill-down album covers via their image URLs.
    pub fn preload_row_covers(&mut self) {
        self.preload_upcoming_spotify_covers();
        // A chart's rows are provider URLs with no library id, so `track_id_at`
        // finds nothing for them and the loop below would warm nothing.
        if self.library_category == 12 {
            self.preload_chart_covers();
        }
        let pos = self.list_pos();
        let mut ids = Vec::new();
        for off in 1..=3 {
            if let Some(id) = self.track_id_at(pos + off) {
                ids.push(id);
            }
        }
        if ids.is_empty() {
            return;
        }
        let client = self.client.clone();
        tokio::spawn(async move {
            for id in ids {
                // Errors (track without cover / daemon lookup fail) are fine:
                // a warm miss is simply skipped next time.
                let _ = client.art().cover(id).await;
            }
        });
    }

    pub(crate) fn cover_sync(&mut self) {
        match (&self.np_cover.image, &self.np_cover.picker) {
            (Some(bytes), Some(picker)) => match image::load_from_memory(bytes) {
                Ok(img) => self.np_cover.stateful = Some(picker.new_resize_protocol(img)),
                Err(_) => self.np_cover.stateful = None,
            },
            _ => self.np_cover.stateful = None,
        }
    }

    pub(crate) fn popup_cover_sync(&mut self) {
        match (&self.track_popup_cover, &self.np_cover.picker) {
            (Some(bytes), Some(picker)) => {
                if let Ok(img) = image::load_from_memory(bytes) {
                    self.popup_cover_stateful = Some(picker.new_resize_protocol(img));
                } else {
                    self.popup_cover_stateful = None;
                }
            }
            _ => self.popup_cover_stateful = None,
        }
    }

    /// Fetch cover art for the queue picker's preview strip, once per row.
    ///
    /// Keyed on the queue *row* the strip is describing, not on `queue.cursor`:
    /// the cursor is what is playing, while the strip shows the row under the
    /// highlight, so following the cursor meant the artwork belonged to a
    /// different track than the title beside it. `path` is the key because
    /// queued and provider entries share `id == 0`, so an id-keyed slot cannot
    /// tell two adjacent rows apart.
    pub fn update_preview_cover(&mut self, idx: usize) {
        if no_image_protocol() {
            return;
        }
        let Some(track) = self.queue.cache.get(idx) else {
            self.queue.preview_slot.clear();
            self.queue.preview_cover = None;
            self.queue.preview_cover_stateful = None;
            return;
        };
        let tid = track.id;
        let key = track.path.clone();
        // Any cached bytes must belong to the row on screen. A cursor jump,
        // queue replacement or thumbnail clear can reset the fetch guard
        // without invalidating the bytes, and a stale `cover_block` fallback
        // would then draw the previous row's art.
        if self.queue.preview_cover.is_some() && self.queue.preview_slot.id.as_deref() != Some(&key)
        {
            self.queue.preview_cover = None;
            self.queue.preview_cover_stateful = None;
        }
        // A failed lookup clears the gen guard so a later preview can retry;
        // this throttle stops the per-frame render from re-fetching a cover
        // that isn't there, at most once per 30s per row.
        if let Some((ref fail_key, until)) = self.queue.preview_fail_until
            && *fail_key == key
            && std::time::Instant::now() < until
        {
            return;
        }
        // Generation-guarded dedup: only skip when a fetch is already in flight.
        if self.queue.preview_slot.pending(&key) {
            return;
        }
        let fetch_gen = self.next_cover_gen();
        self.queue.preview_slot.claim(key.clone(), fetch_gen);
        self.queue.preview_cover = None;
        self.queue.preview_cover_stateful = None;
        let client = self.client.clone();
        let ipc_tx = self.ipc_tx.clone();
        tokio::spawn(async move {
            // Report failure as `None` too, so the pending-gen guard is
            // released and a later save/queue change can retry the lookup.
            let cover = if let Ok(Some(b64)) = client.art().cover_for(tid, Some(key.clone())).await {
                base64::engine::general_purpose::STANDARD.decode(&b64).ok()
            } else {
                None
            };
            let _ = ipc_tx.send(IpcResult::QueuePreviewCover(cover, key, fetch_gen));
        });
    }

    pub(crate) fn sync_preview_cover(&mut self) {
        match (&self.queue.preview_cover, &self.np_cover.picker) {
            (Some(bytes), Some(picker)) => {
                if let Ok(img) = image::load_from_memory(bytes) {
                    self.queue.preview_cover_stateful = Some(picker.new_resize_protocol(img));
                } else {
                    self.queue.preview_cover_stateful = None;
                }
            }
            _ => self.queue.preview_cover_stateful = None,
        }
    }

    pub(crate) fn picker_preview_sync(&mut self) {
        match (&self.picker_preview_cover, &self.np_cover.picker) {
            (Some(bytes), Some(picker)) => {
                if let Ok(img) = image::load_from_memory(bytes) {
                    self.picker_preview_stateful = Some(picker.new_resize_protocol(img));
                } else {
                    self.picker_preview_stateful = None;
                }
            }
            _ => self.picker_preview_stateful = None,
        }
    }

    /// (Re)build the stateful cover protocol for the Edit Metadata preview.
    pub(crate) fn metadata_cover_sync(&mut self) {
        match (&self.metadata.cover, &self.np_cover.picker) {
            (Some(bytes), Some(picker)) => {
                if let Ok(img) = image::load_from_memory(bytes) {
                    self.metadata.cover_stateful = Some(picker.new_resize_protocol(img));
                } else {
                    self.metadata.cover_stateful = None;
                }
            }
            _ => self.metadata.cover_stateful = None,
        }
    }

    pub(crate) fn artist_cover_sync(&mut self) {
        match (&self.artist_cover, &self.np_cover.picker) {
            (Some(bytes), Some(picker)) => {
                if let Ok(img) = image::load_from_memory(bytes) {
                    self.artist_cover_stateful = Some(picker.new_resize_protocol(img));
                } else {
                    self.artist_cover_stateful = None;
                }
            }
            _ => self.artist_cover_stateful = None,
        }
    }

    /// Fetch the cover art for the track currently being edited and stream it
    /// to the `MetadataCoverArt` IPC channel so the preview can refresh.
    /// Generation-guarded to prevent stale picker-reuse overwrites.
    pub(crate) fn fetch_metadata_cover(&mut self) {
        // Batch edits (album/artist rows) preview the first track's cover only.
        let Some(&track_id) = self.metadata.edit_track_ids.first() else {
            return;
        };
        if no_image_protocol() {
            return;
        }
        let fetch_gen = self.next_cover_gen();
        self.metadata.cover_fetch.claim(
            self.metadata.edit_track_ids.first().copied().unwrap_or(0),
            fetch_gen,
        );
        // Clear stale cover while new fetch is in flight; handler will repopulate.
        self.metadata.cover = None;
        self.metadata.cover_stateful = None;
        let client = self.client.clone();
        let ipc_tx = self.ipc_tx.clone();
        tokio::spawn(async move {
            if let Ok(Some(b64)) = client.art().cover(track_id).await
                && let Ok(bytes) = base64::engine::general_purpose::STANDARD.decode(&b64)
            {
                let _ = ipc_tx.send(IpcResult::MetadataCoverArt(
                    Some(bytes),
                    track_id,
                    fetch_gen,
                ));
            }
        });
    }

    /// Ask the daemon for current cover cache usage so Settings can show it.
    pub fn refresh_cover_stat(&mut self) {
        let c = self.client.clone();
        let ipc_tx = self.ipc_tx.clone();
        tokio::spawn(async move {
            if let Ok((disk, _, _)) = c.cover_cache_stat().await {
                let _ = ipc_tx.send(IpcResult::CoverCacheStat(disk));
            }
        });
    }
}
