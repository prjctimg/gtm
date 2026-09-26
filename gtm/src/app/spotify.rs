use crate::app::*;

/// Album-art previews kept in memory for the Spotify search picker. Each entry
/// is a decoded album JPEG of roughly 100-200 KB, so this is deliberately
/// small: enough to cover a page of results plus a little scrolling back, not
/// enough to grow for the whole session.
pub(crate) const PREVIEW_CACHE_MAX: usize = 24;

impl App {
    /// Run one Spotify Connect control from the Settings panel and feed the
    /// refreshed status back into the view, so the row text updates in place.
    pub(crate) fn spot_ctrl(&mut self, opt: usize) {
        let c = self.client.clone();
        let ipc_tx = self.ipc_tx.clone();
        let st = self.spotify.status.clone().unwrap_or_default();
        let (label, res) = match opt {
            8 => ("next", 0),
            9 => ("previous", 1),
            10 => ("shuffle", 2),
            _ => ("repeat", 3),
        };
        tokio::spawn(async move {
            let out = match res {
                0 => c.spotify().next().await,
                1 => c.spotify().previous().await,
                2 => c.spotify().set_shuffle(!st.shuffle).await,
                _ => {
                    // off → context → track → off
                    let next = match st.repeat.as_str() {
                        "off" => "context",
                        "context" => "track",
                        _ => "off",
                    };
                    c.spotify().set_repeat(next).await
                }
            };
            match out {
                Ok(status) => {
                    let _ = ipc_tx.send(IpcResult::SpotifyStatus(status));
                }
                Err(e) => {
                    let _ = ipc_tx.send(IpcResult::Error(format!("Spotify {label}: {e}")));
                }
            }
        });
    }

    /// Search picker matching the focused list. The library search covers the
    /// local library; provider categories open their own provider's search so
    /// `/` always searches what the user is looking at.
    pub fn search_picker(&self) -> PickerId {
        match LIBRARY_CATEGORIES.get(self.library_category).copied() {
            Some("Spotify") => PickerId::SpotifySearch,
            Some("Radio") => PickerId::Radio,
            _ => PickerId::SearchLibrary,
        }
    }

    /// Start the Spotify OAuth PKCE flow for `client_id` on `port` and watch it
    /// in the background. Validates the client id first (the empty-input
    /// fallback id always passes); an invalid id or a missing redirect-URI
    /// registration is reported inline instead of silently dying in the
    /// browser.
    pub(crate) fn start_spotify_oauth(&mut self, client_id: String, port: u16) {
        if let Some(err) = client_id_error(&client_id, port) {
            self.spotify.oauth_error = Some(err);
            self.spotify.oauth_pending = false;
            self.spotify.link_input.clear();
            return;
        }
        set_secret(SPOTIFY_CLIENT_ID, &client_id);
        let c = self.client.clone();
        let ipc_tx = self.ipc_tx.clone();
        self.spotify.link_input.clear();
        self.spotify.oauth_pending = true;
        self.spotify.oauth_url = None;
        self.spotify.oauth_error = None;
        tokio::spawn(async move {
            match c.spotify().oauth_start(&client_id, port).await {
                Ok(url) => {
                    let _ = ipc_tx.send(IpcResult::AuthUrl("Spotify", url.clone()));
                    let _ = ipc_tx.send(IpcResult::Notification(
                        "Spotify".to_string(),
                        "Authorize gtm in your browser, then playlists sync automatically…"
                            .to_string(),
                        NotificationKind::Info,
                        NotifType::Spotify,
                    ));
                    try_open_browser(&url, &ipc_tx);
                }
                Err(e) => {
                    let _ = ipc_tx.send(IpcResult::AuthError(
                        "Spotify",
                        format!("Spotify link failed: {e}"),
                    ));
                }
            }
        });
    }

    /// Fetch the album cover for the highlighted Spotify drill-down row from
    /// the track's album-image URL, so scrolling the playlist loads cover art.
    /// Generation-guarded by URL via `spotify_popup_slot` (stale replies from
    /// earlier rows are dropped).
    pub(crate) fn fetch_spotify_popup_cover(&mut self) {
        let Some(track) = self.selected_spotify_track().cloned() else {
            self.clear_popup_cover();
            return;
        };
        let Some(url) = track.image_url.clone() else {
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
            match client.spotify().track_image(&url).await {
                Ok(Some(b64)) => {
                    if let Ok(bytes) = base64::engine::general_purpose::STANDARD.decode(&b64) {
                        let _ =
                            ipc_tx.send(IpcResult::SpotifyPopupCover(Some(bytes), url, fetch_gen));
                    }
                }
                Ok(None) | Err(_) => {
                    let _ = ipc_tx.send(IpcResult::SpotifyPopupCover(None, url, fetch_gen));
                }
            }
        });
    }

    pub fn search_spotify(&mut self) {
        let q = self
            .pickers
            .top()
            .map_or(String::new(), |o| o.query.to_lowercase());
        self.spotify.search_results.clear();
        self.spotify.preview_fetch.clear();
        self.spotify.preview_cover = None;
        self.spotify.preview_shown = None;
        self.spotify.preview_cover_stateful = None;
        if q.is_empty() {
            self.spotify.search_loading = false;
            return;
        }
        for pl in &self.spotify.playlists {
            for track in &pl.tracks {
                if track.name.to_lowercase().contains(&q)
                    || track.artists.to_lowercase().contains(&q)
                    || track
                        .album
                        .as_deref()
                        .unwrap_or("")
                        .to_lowercase()
                        .contains(&q)
                {
                    self.spotify.search_results.push((
                        pl.id.clone(),
                        pl.name.clone(),
                        track.clone(),
                    ));
                }
            }
        }
        let query = self
            .pickers
            .top()
            .map_or(String::new(), |o| o.query.clone());
        let c = self.client.clone();
        let ipc_tx = self.ipc_tx.clone();
        let seq = self.spotify.web_seq;
        self.spotify.search_loading = true;
        tokio::spawn(async move {
            let res = c
                .spotify()
                .search_web(&query)
                .await
                .map_err(|e| e.to_string());
            let _ = ipc_tx.send(IpcResult::SpotifySearchWebDone(seq, res));
        });
    }

    /// Fire one background playlist sync when the Spotify pane opens with an
    /// empty cache (linked accounts only). Latch `synced_once` prevents
    /// re-triggering on every pane visit (accounts can legitimately have zero
    /// playlists); the daemon additionally auto-syncs at startup with
    /// retry-forever backoff, so this is a second net, not the primary.
    pub(crate) fn maybe_auto_sync_spotify(&mut self) {
        let linked = self.spotify.status.as_ref().is_some_and(|s| s.linked);
        if linked && !self.spotify.synced_once && self.spotify.playlists.is_empty() {
            self.trigger_spotify_sync(false);
        }
    }

    /// Kick a TUI-side playlist sync. `notify` toggles the completion toast
    /// (manual Settings -> Sync yes, pane auto-fill no). The daemon caches the
    /// results; on success the fresh playlist list is re-pulled and handed
    /// back so the pane refreshes immediately instead of only after the next
    /// status event.
    pub(crate) fn trigger_spotify_sync(&mut self, notify: bool) {
        if self.spotify.sync_pending {
            return;
        }
        self.spotify.sync_pending = true;
        let c = self.client.clone();
        let ipc_tx = self.ipc_tx.clone();
        tokio::spawn(async move {
            match c.spotify().sync().await {
                Ok(()) => {
                    if notify {
                        let _ = ipc_tx.send(IpcResult::Notification(
                            "Spotify".to_string(),
                            "Spotify sync complete".to_string(),
                            NotificationKind::Success,
                            NotifType::Spotify,
                        ));
                    }
                    // Re-pull the cache so the playlist pane reflects the sync
                    // without a restart (the manual flow previously toasted
                    // but left the list stale).
                    if let Ok(playlists) = c.spotify().playlists().await {
                        let _ = ipc_tx.send(IpcResult::SpotifyPlaylists(playlists));
                    }
                    let _ = ipc_tx.send(IpcResult::SpotifySyncFinished(true));
                }
                Err(e) => {
                    let _ = ipc_tx.send(IpcResult::Error(format!("Spotify sync: {e}")));
                    let _ = ipc_tx.send(IpcResult::SpotifySyncFinished(false));
                }
            }
        });
    }

    /// True while the right pane is showing the track list of a Spotify
    /// playlist (drilled down from the Spotify playlists category).
    pub fn in_spotify_playlist(&self) -> bool {
        self.browse_detail.is_some() && self.library_category == 5
    }

    /// Row count of the Spotify playlist drill-down list, including the two
    /// virtual action rows (`Play All`, `Shuffle`) at the top.
    pub fn spotify_playlist_rows(&self) -> usize {
        self.spotify.playlist_tracks_cache.len() + Self::SPOTIFY_PLAYLIST_ROWS
    }

    /// The track the current list position maps to in a Spotify playlist
    /// drill-down. `None` for the action rows and non-Spotify views.
    pub fn selected_spotify_track(&self) -> Option<&SpotifyTrack> {
        if !self.in_spotify_playlist() {
            return None;
        }
        self.spotify
            .playlist_tracks_cache
            .get(self.list_pos().saturating_sub(Self::SPOTIFY_PLAYLIST_ROWS))
    }

    /// Preload the album-cover URLs of Spotify drill-down rows a short scroll
    /// ahead of the cursor so fast scrolling warms the daemon's image cache
    /// (covers are keyed by URL, not by a local library id). Fires in the
    /// background and never blocks the UI or surfaces errors.
    pub(crate) fn preload_upcoming_spotify_covers(&mut self) {
        if !self.in_spotify_playlist() {
            return;
        }
        let pos = self.list_pos();
        let tracks = &self.spotify.playlist_tracks_cache;
        let mut urls = Vec::new();
        for off in 1..=3 {
            let idx = (pos + off).saturating_sub(Self::SPOTIFY_PLAYLIST_ROWS);
            if let Some(t) = tracks.get(idx)
                && let Some(url) = t.image_url.clone()
                && !urls.contains(&url)
            {
                urls.push(url);
            }
        }
        if urls.is_empty() {
            return;
        }
        let client = self.client.clone();
        tokio::spawn(async move {
            for url in urls {
                // The daemon persists this through the cover cache, so the next
                // visit is served from disk. A miss is simply retried later.
                let _ = client.spotify().track_image(&url).await;
            }
        });
    }

    /// The `"{artist} - {title}"` of the track currently on air, if it is a
    /// live stream whose title the daemon has resolved into an artist and a
    /// title separately. `None` otherwise, which is what makes the like and
    /// add actions unavailable rather than acting on a guess.
    pub fn live_query(&self) -> Option<String> {
        if self.live_queue().is_empty() {
            return None;
        }
        let list = &self.state.radio_tracks;
        let at = match self.state.radio_title.as_deref() {
            Some(t) if !t.is_empty() => list.match_title(t),
            _ => list.at,
        };
        let track = list.tracks.get(at)?;
        if track.title.is_empty() {
            return None;
        }
        Some(track.query())
    }

    /// Spotify destinations for `query`: `(id, name)` per synced playlist,
    /// filtered by the picker's query. Liked Songs is not a playlist and is
    /// always offered as row 0, so it is not in this list.
    pub fn live_dest_names(&self, query: &str) -> Vec<(String, String)> {
        let q = self
            .pickers
            .top()
            .map_or(String::new(), |o| o.query.to_lowercase());
        let _ = query;
        self.spotify
            .playlists
            .iter()
            .filter(|p| q.is_empty() || p.name.to_lowercase().contains(&q))
            .map(|p| (p.id.clone(), p.name.clone()))
            .collect()
    }

    /// Row count of the destination picker: Liked Songs plus the filtered
    /// playlists.
    pub fn live_dests_rows(&self) -> usize {
        let query = self.live_query().unwrap_or_default();
        self.live_dest_names(&query).len() + 1
    }

    /// Open the destination picker for the track on air, resolving it to a
    /// Spotify URI first so the search happens once and the user can confirm
    /// which track was matched before anything is written.
    pub(crate) fn open_live_dest(&mut self) {
        if self.spotify.status.as_ref().is_none_or(|s| !s.linked) {
            self.notify_typed(
                "Spotify",
                "Not linked — nothing to add the track to",
                NotificationKind::Info,
                true,
                NotifType::Spotify,
            );
            return;
        }
        let Some(query) = self.live_query() else {
            self.notify_typed(
                "Spotify",
                "No track on air to add",
                NotificationKind::Info,
                false,
                NotifType::NowPlaying,
            );
            return;
        };
        self.live_dests.clear();
        self.live_uri = None;
        self.live_query = Some(query.clone());
        self.pickers.open(PickerId::SpotifyDest);
        let client = self.client.clone();
        let ipc_tx = self.ipc_tx.clone();
        tokio::spawn(async move {
            match client.spotify().match_track(&query).await {
                Ok(uri) => {
                    let _ = ipc_tx.send(IpcResult::SpotifyMatch(uri));
                }
                Err(e) => {
                    let _ = ipc_tx.send(IpcResult::Error(format!("Spotify match: {e}")));
                }
            }
        });
    }

    /// Write the resolved track to every ticked destination. One failure does
    /// not stop the rest, so a single unwritable playlist cannot lose the
    /// others.
    pub(crate) fn commit_live_dest(&mut self, dests: Vec<String>) {
        let Some(uri) = self.live_uri.clone() else {
            self.notify_typed(
                "Spotify",
                "Still matching the track — try again in a moment",
                NotificationKind::Info,
                false,
                NotifType::Spotify,
            );
            return;
        };
        let names = self.live_dest_names(&self.live_query().unwrap_or_default());
        let client = self.client.clone();
        let ipc_tx = self.ipc_tx.clone();
        let asked = dests.len();
        tokio::spawn(async move {
            let mut done = 0usize;
            let mut failed: Vec<String> = Vec::new();
            for dest in dests {
                let res = if dest.is_empty() {
                    client.spotify().like(&uri).await.map_err(|e| e.to_string())
                } else {
                    let label = names
                        .iter()
                        .find(|(id, _)| *id == dest)
                        .map(|(_, n)| n.clone())
                        .unwrap_or_else(|| dest.clone());
                    client
                        .spotify()
                        .playlist_add(&uri, &dest)
                        .await
                        .map_err(|e| format!("{label}: {e}"))
                };
                match res {
                    Ok(()) => done += 1,
                    Err(e) => failed.push(e),
                }
            }
            if !failed.is_empty() {
                let _ = ipc_tx.send(IpcResult::Error(format!("Spotify: {}", failed.join("; "))));
            }
            if done > 0 {
                let _ = ipc_tx.send(IpcResult::Notification(
                    "Spotify".to_string(),
                    format!("Added to {done} of {asked}"),
                    NotificationKind::Success,
                    NotifType::Spotify,
                ));
            }
        });
    }

    /// Save the track on air to Liked Songs, resolving it first.
    pub(crate) fn like_live(&mut self) {
        if self.spotify.status.as_ref().is_none_or(|s| !s.linked) {
            self.notify_typed(
                "Spotify",
                "Not linked — nothing to like the track on",
                NotificationKind::Info,
                true,
                NotifType::Spotify,
            );
            return;
        }
        let Some(query) = self.live_query() else {
            self.notify_typed(
                "Spotify",
                "No track on air to like",
                NotificationKind::Info,
                false,
                NotifType::NowPlaying,
            );
            return;
        };
        let client = self.client.clone();
        let ipc_tx = self.ipc_tx.clone();
        tokio::spawn(async move {
            let uri = match client.spotify().match_track(&query).await {
                Ok(uri) => uri,
                Err(e) => {
                    let _ = ipc_tx.send(IpcResult::Error(format!("Spotify match: {e}")));
                    return;
                }
            };
            match client.spotify().like(&uri).await {
                Ok(()) => {
                    let _ = ipc_tx.send(IpcResult::Notification(
                        "Spotify".to_string(),
                        "Saved to Liked Songs".to_string(),
                        NotificationKind::Success,
                        NotifType::Spotify,
                    ));
                }
                Err(e) => {
                    let _ = ipc_tx.send(IpcResult::Error(format!("Spotify like: {e}")));
                }
            }
        });
    }

    pub(crate) fn spotify_preview_sync(&mut self) {
        match (&self.spotify.preview_cover, &self.np_cover.picker) {
            (Some(bytes), Some(picker)) => {
                if let Ok(img) = image::load_from_memory(bytes) {
                    self.spotify.preview_cover_stateful = Some(picker.new_resize_protocol(img));
                } else {
                    self.spotify.preview_cover_stateful = None;
                }
            }
            _ => self.spotify.preview_cover_stateful = None,
        }
    }

    /// Fetch cover art for the highlighted SpotifySearch picker row (a web
    /// search hit carrying an album-cover URL) so the preview window can render
    /// it as ASCII, mirroring the SearchLibrary picker behaviour.
    pub fn update_spot_preview(&mut self) {
        let Some(top) = self.pickers.top() else {
            self.spotify.preview_cover = None;
            self.spotify.preview_shown = None;
            self.spotify.preview_cover_stateful = None;
            self.spotify.preview_fetch.clear();
            return;
        };
        if top.id != PickerId::SpotifySearch {
            self.spotify.preview_cover = None;
            self.spotify.preview_shown = None;
            self.spotify.preview_cover_stateful = None;
            self.spotify.preview_fetch.clear();
            return;
        }
        let picks = self.spot_picks();
        if picks.is_empty() {
            self.spotify.preview_cover = None;
            self.spotify.preview_shown = None;
            self.spotify.preview_cover_stateful = None;
            self.spotify.preview_fetch.clear();
            return;
        }
        let sel = top.selected.min(picks.len() - 1);
        let Some(url) = self.spotify.search_results[picks[sel]].2.image_url.clone() else {
            self.spotify.preview_cover = None;
            self.spotify.preview_shown = None;
            self.spotify.preview_cover_stateful = None;
            self.spotify.preview_fetch.clear();
            return;
        };
        // Already fetched this session: publish straight away. Adjacent search
        // hits very often share an album, and returning to a row after a query
        // change should not blank the preview while bytes we already hold are
        // re-requested.
        if let Some(bytes) = self.spotify.preview_cache.get(&url).cloned() {
            if self.spotify.preview_shown.as_deref() != Some(url.as_str()) {
                self.spotify.preview_cover = Some(bytes);
                self.spotify.preview_shown = Some(url);
                self.spotify_preview_sync();
            }
            self.spotify.preview_fetch.clear();
            return;
        }
        // One in-flight fetch per selection: scrolling back to a row whose
        // cover is already loading reuses it instead of re-requesting. The
        // current image stays up rather than flashing blank until it lands.
        if self.spotify.preview_fetch.pending(&url) {
            return;
        }
        let fetch_gen = self.next_cover_gen();
        self.spotify.preview_fetch.claim(url.clone(), fetch_gen);
        if no_image_protocol() {
            return;
        }
        let client = self.client.clone();
        let ipc_tx = self.ipc_tx.clone();
        tokio::spawn(async move {
            match client.spotify().track_image(&url).await {
                Ok(Some(b64)) => {
                    if let Ok(bytes) = base64::engine::general_purpose::STANDARD.decode(&b64) {
                        let _ = ipc_tx.send(IpcResult::SpotifyPreviewCover(
                            Some(bytes),
                            url,
                            fetch_gen,
                        ));
                    }
                }
                Ok(None) | Err(_) => {
                    let _ = ipc_tx.send(IpcResult::SpotifyPreviewCover(None, url, fetch_gen));
                }
            }
        });
    }
}

impl App {
    /// Open the Spotify OAuth link form. Guarded so the row reports why it did
    /// nothing instead of silently re-opening the flow on an already-linked
    /// account.
    pub(crate) fn open_spotify_link(&mut self) {
        if self.spotify.status.as_ref().is_some_and(|s| s.linked) {
            self.notify_typed(
                "Spotify",
                "Already linked — use Unlink to switch accounts",
                NotificationKind::Info,
                true,
                NotifType::Spotify,
            );
            return;
        }
        self.spotify.link_input.clear();
        self.spotify.oauth_port = "8990".to_string();
        self.spotify.link_field = 0;
        if let Some(cid) = get_secret(SPOTIFY_CLIENT_ID) {
            self.spotify.link_input = cid;
        }
        self.pickers.open(PickerId::SpotifyLink);
    }

    /// Drop the stored Spotify credentials. Guarded for the same reason as
    /// [`App::open_spotify_link`].
    pub(crate) fn unlink_spotify(&mut self, tx: tokio::sync::mpsc::Sender<TuiCommand>) {
        if !self.spotify.status.as_ref().is_some_and(|s| s.linked) {
            self.notify_typed(
                "Spotify",
                "Not linked — nothing to unlink",
                NotificationKind::Info,
                true,
                NotifType::Spotify,
            );
            return;
        }
        let c = self.client.clone();
        let ipc_tx = self.ipc_tx.clone();
        let _ = tx.try_send(TuiCommand::fire(move || async move {
            match c.spotify().clear().await {
                Ok(status) => {
                    let _ = ipc_tx.send(IpcResult::SpotifyStatus(status));
                    let _ = ipc_tx.send(IpcResult::Notification(
                        "Spotify".to_string(),
                        "Account unlinked".to_string(),
                        NotificationKind::Info,
                        NotifType::Spotify,
                    ));
                }
                Err(e) => {
                    let _ = ipc_tx.send(IpcResult::Error(format!("Spotify unlink: {e}")));
                }
            }
        }));
    }
}
