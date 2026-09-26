use crate::app::*;

impl App {
    pub fn start_upnext(&mut self, track: TrackInfo) {
        let total_secs = self.crossfade_duration as f64 + 3.0;
        let fetch_gen = if no_image_protocol() {
            None
        } else {
            Some(self.next_cover_gen())
        };
        let fetch_id = if fetch_gen.is_some() {
            Some(track.id)
        } else {
            None
        };
        self.upnext = Some(UpNextNotif {
            track: track.clone(),
            cover: None,
            cover_stateful: None,
            started_at: std::time::Instant::now(),
            total_secs,
            cover_fetch: FetchSlot {
                id: fetch_id,
                version: fetch_gen,
            },
        });
        let Some(fetch_gen) = fetch_gen else {
            return;
        };
        let tid = track.id;
        let client = self.client.clone();
        let ipc_tx = self.ipc_tx.clone();
        tokio::spawn(async move {
            if let Ok(Some(b64)) = client.art().cover(tid).await
                && let Ok(bytes) = base64::engine::general_purpose::STANDARD.decode(&b64)
            {
                let _ = ipc_tx.send(IpcResult::UpNextCover(Some(bytes), tid, fetch_gen));
            }
        });
    }

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
            // Playlist and Spotify rows never resolve a cover; the block still
            // describes the selected row (playlist name / spotify track).
            TrackInfoKind::Playlist
            | TrackInfoKind::SpotifyPlaylist
            | TrackInfoKind::SpotifyTrack => None,
        };

        let valid = match kind {
            TrackInfoKind::Playlist => self.list_pos() < self.playlist_cache.len(),
            TrackInfoKind::SpotifyPlaylist => self.list_pos() < self.spotify.playlists.len(),
            TrackInfoKind::SpotifyTrack => self.selected_spotify_track().is_some(),
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
            if let Ok(Some(b64)) = client.art().cover(tid).await
                && let Ok(bytes) = base64::engine::general_purpose::STANDARD.decode(&b64)
            {
                let _ = ipc_tx.send(IpcResult::PopupCoverArt(Some(bytes), tid, fetch_gen));
            }
        });
    }

    /// Dismiss the track popup.
    pub fn dismiss_track_popup(&mut self) {
        self.track_popup_visible = false;
        self.clear_popup_cover();
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
            if let Ok(Some(b64)) = client.art().cover(tid).await
                && let Ok(bytes) = base64::engine::general_purpose::STANDARD.decode(&b64)
            {
                let _ = ipc_tx.send(IpcResult::PickerPreviewCover(Some(bytes), tid, fetch_gen));
            }
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
            if let Ok(Some(b64)) = client.art().artist_cover(artist.clone()).await
                && let Ok(bytes) = base64::engine::general_purpose::STANDARD.decode(&b64)
            {
                let _ = ipc_tx.send(IpcResult::ArtistCoverArt(Some(bytes), artist, fetch_gen));
            }
        });
    }

    /// Preload the cover art for the tracks a short scroll ahead of the cursor
    /// so fast scrolling (e.g. holding an arrow key) warms the daemon's
    /// disk/LRU cache and the on-selection fetch becomes a cache hit. Fires in
    /// the background and never blocks the UI or surfaces errors. Also warms
    /// Spotify drill-down album covers via their image URLs.
    pub fn preload_upcoming_covers(&mut self) {
        self.preload_upcoming_spotify_covers();
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
            (Some(bytes), Some(picker)) => {
                if let Ok(img) = image::load_from_memory(bytes) {
                    self.np_cover.stateful = Some(picker.new_resize_protocol(img));
                } else {
                    self.np_cover.stateful = None;
                }
            }
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

    pub(crate) fn upnext_cover_sync(&mut self) {
        let Some(picker) = self.np_cover.picker.as_ref() else {
            return;
        };
        if let Some(u) = self.upnext.as_mut() {
            u.cover_stateful = match u.cover.as_ref() {
                Some(bytes) => image::load_from_memory(bytes)
                    .ok()
                    .map(|img| picker.new_resize_protocol(img)),
                None => None,
            };
        }
    }

    /// Fetch cover art for the queue picker "Up Next" strip, once per
    /// track.  Locally-inserted tracks (`id == 0`) are fetched too; if the
    /// daemon has no art the renderer falls back to a glyph.
    pub fn update_upnext_cover(&mut self) {
        if no_image_protocol() {
            return;
        }
        let next_idx = self.queue.cursor + 1;
        let Some(track) = self.queue.cache.get(next_idx) else {
            self.queue.preview_slot.clear();
            self.queue.preview_cover = None;
            self.queue.preview_cover_stateful = None;
            return;
        };
        let tid = track.id;
        let track_path = track.path.clone();
        // Any cached cover bytes must belong to the track currently shown as
        // up-next. A cursor jump, queue replacement, or thumbnail clear can
        // reset the fetch guard without invalidating the bytes; dropping them
        // here guarantees the preview can never show art for the previous
        // track (rendered from a stale `cover_block` fallback).
        if self.queue.preview_cover.is_some() && self.queue.preview_slot.id != Some(tid) {
            self.queue.preview_cover = None;
            self.queue.preview_cover_stateful = None;
        }
        // A failed lookup clears the gen guard so a later preview can retry;
        // this throttle prevents the per-frame render from re-fetching a
        // cover that isn't there, at most once per 30s per track.
        if let Some((fail_tid, until)) = self.queue.preview_fail_until
            && fail_tid == tid
            && std::time::Instant::now() < until
        {
            return;
        }
        // If the up-next notification refers to the very same track that the
        // queue picker is showing, reuse its cover bytes so both surfaces are
        // always in sync and the now-playing cover can never appear here.
        let reuse = self
            .upnext
            .as_ref()
            .filter(|u| {
                u.track.id == tid
                    && u.track.path == track_path
                    && u.cover.is_some()
                    && self.queue.preview_slot.id != Some(tid)
            })
            .and_then(|u| u.cover.clone());
        if let Some(cover) = reuse {
            self.queue.preview_cover = Some(cover);
            self.sync_preview_cover();
            // Keep the id so a retry reuses the cached bytes, but drop the
            // generation so the next request is allowed through.
            self.queue.preview_slot.version = None;
            return;
        }
        // Generation-guarded dedup: allows `id == 0` tracks to refetch
        // distinctly. Only skip when pending fetch_gen exists.
        if self.queue.preview_slot.pending(&tid) {
            return;
        }
        let fetch_gen = self.next_cover_gen();
        self.queue.preview_slot.claim(tid, fetch_gen);
        self.queue.preview_cover = None;
        self.queue.preview_cover_stateful = None;
        let client = self.client.clone();
        let ipc_tx = self.ipc_tx.clone();
        tokio::spawn(async move {
            // Report failure as `None` too, so the pending-gen guard is
            // released and a later save/queue change can retry the lookup.
            let cover = if let Ok(Some(b64)) = client.art().cover_for(tid, Some(track_path)).await {
                base64::engine::general_purpose::STANDARD.decode(&b64).ok()
            } else {
                None
            };
            let _ = ipc_tx.send(IpcResult::QueuePreviewCover(cover, tid, fetch_gen));
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

    /// Step the on-disk cover cache budget to the next preset and push it to
    /// the daemon, which prunes immediately if it is already over budget.
    pub fn cycle_cover_cache(&mut self) {
        let next = COVER_CACHE_STEPS
            .iter()
            .copied()
            .find(|mb| *mb > self.cover_cache_mb)
            .unwrap_or(COVER_CACHE_STEPS[0]);
        self.cover_cache_mb = next;
        let bytes = next * 1024 * 1024;
        let c = self.client.clone();
        let ipc_tx = self.ipc_tx.clone();
        tokio::spawn(async move {
            let _ = c.set_cover_cache(bytes).await;
            if let Ok((disk, _, _)) = c.cover_cache_stat().await {
                let _ = ipc_tx.send(IpcResult::CoverCacheStat(disk));
            }
        });
        save_prefs(&self.current_prefs());
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
