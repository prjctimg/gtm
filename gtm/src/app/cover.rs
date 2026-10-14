use crate::app::*;
use crate::ui::step_viewport;
use ratatui::layout::Rect;
use ratatui_image::protocol::StatefulProtocol;

/// Decoded cover bytes from a base64 reply, or `None` for a miss or a body the
/// image crate cannot read.
pub(crate) fn b64(s: String) -> Option<Vec<u8>> {
    base64::engine::general_purpose::STANDARD.decode(&s).ok()
}

/// Ask the daemon for one cover, claim `slot` for it, and answer `reply` on both
/// a hit and a miss.
///
/// Every cover surface claimed a slot, spawned a task, decoded the reply and
/// built its own message; eight copies of that shape is eight chances to forget
/// the miss. A silent miss leaves the slot claimed, and a claimed slot is one the
/// surface reads as in-flight forever — so a track whose art the daemon had not
/// cached yet stayed blank for the rest of the session however much you scrolled.
pub(crate) fn cover_get<T, G, R>(
    client: &DaemonClient,
    ipc_tx: &mpsc::UnboundedSender<IpcResult>,
    slot: &mut FetchSlot<T>,
    key: T,
    reply: R,
    get: G,
) -> u64
where
    T: Clone + PartialEq + Send + 'static,
    G: FnOnce(DaemonClient) -> futures::future::BoxFuture<'static, Option<Vec<u8>>>
        + Send
        + 'static,
    R: FnOnce(Option<Vec<u8>>, T, u64) -> IpcResult + Send + 'static,
{
    let version = cover_gen();
    slot.claim(key.clone(), version);
    let client = client.clone();
    let ipc_tx = ipc_tx.clone();
    tokio::spawn(async move {
        let bytes = get(client).await;
        let _ = ipc_tx.send(reply(bytes, key, version));
    });
    version
}

/// The next cover-fetch generation, shared by every surface that stamps one.
///
/// A process-wide counter rather than a field on `App`: the only thing it has to
/// guarantee is that no two replies carry the same generation, so holding it in
/// one place means a surface that rolls its own cannot collide with a slot it
/// does not own. Disambiguates stale replies and `id == 0` reuse alike.
pub(crate) fn cover_gen() -> u64 {
    static GEN: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    GEN.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}

impl App {
    /// The image protocol for `bytes`, decoded once.
    ///
    /// Every cover on screen is drawn through one of these: `Render::cover`
    /// wants a protocol rather than bytes, and decoding an image per frame is
    /// what makes a preview flicker rather than hold still. One decoder for all
    /// of them, so a cover can never be missing because its own surface spelled
    /// the decode differently from the other eight.
    pub(crate) fn protocol(&self, bytes: Option<&[u8]>) -> Option<StatefulProtocol> {
        let img = image::load_from_memory(bytes?).ok()?;
        Some(self.np_cover.picker.as_ref()?.new_resize_protocol(img))
    }
    /// Fetch the artwork for the track on air.
    ///
    /// One entry point because three things want these bytes — a track change,
    /// entering Zen, and a Zen surface switch back to the cover — and three
    /// copies of the guard chain meant three places where one of them could
    /// answer "no art" and leave the reactive theme on its base palette for
    /// the whole track. A fetch already in flight is left alone: the reply is
    /// generation-checked, so a second request would only throw the first away.
    pub(crate) fn fetch_np_cover(&mut self) {
        if !(self.reactive_theme || !no_image_protocol()) {
            return;
        }
        let Some(track) = self.state.current_track.clone() else {
            return;
        };
        let tid = track.id;
        if self.np_cover.pending_gen.is_some() {
            return;
        }
        // The identity of the request belongs to the request, not to the caller.
        // The reply handler drops anything whose path no longer matches, so the
        // two fields are set here for every caller, entering Zen included.
        self.np_cover.track_id = Some(tid);
        self.np_cover.track_path = Some(track.path.clone());
        let fetch_gen = cover_gen();
        self.np_cover.pending_gen = Some(fetch_gen);
        // One request, the same one every other surface makes: by library id
        // *and* exact path. The daemon prefers a resolved `cover_path` and
        // falls back to `cover_url` for a row it cannot otherwise place, so
        // this one call reaches both local files and synced remote rows and
        // returns the same bytes the previews get.
        //
        // The URL stays as a second attempt: a row whose path names nothing the
        // daemon can match still has artwork behind that url, and asking for it
        // costs one request that only happens after the first has missed.
        let url = (!std::path::Path::new(&track.path).is_absolute())
            .then(|| track.cover_url.clone())
            .flatten()
            .filter(|u| !u.is_empty());
        let art_path = Some(track.path.clone());
        let client = self.client.clone();
        let ipc_tx = self.ipc_tx.clone();
        tokio::spawn(async move {
            let mut bytes = match client.art().cover_for(tid, art_path).await {
                Ok(enc) => enc.and_then(b64),
                Err(_) => match client.art().cover(tid).await {
                    Ok(Some(enc)) => b64(enc),
                    _ => None,
                },
            };
            if bytes.is_none()
                && let Some(u) = url
            {
                bytes = client.image_cover(&u).await.ok().flatten();
            }
            // Always answer, including "no art": the reply handler only clears
            // `pending_gen` on a reply, so a silent miss would leave the slot
            // claimed for the rest of the session.
            let _ = ipc_tx.send(IpcResult::CoverArt(bytes, Some(tid), fetch_gen));
        });
    }

    /// Kind of item the library track-info block is currently describing,
    /// derived from the active list and drill-down state.
    pub fn track_info_kind(&self) -> TrackInfoKind {
        if self.browse_detail.is_some() {
            if self.library_category == LIB_SPOTIFY {
                return TrackInfoKind::SpotifyTrack;
            }
            return TrackInfoKind::Track;
        }
        // The Library view's grouped filters describe the row they highlight,
        // so the preview card follows the filter rather than the view.
        if self.library_category == LIB_ALL {
            return match self.library_filter {
                LibraryFilter::Albums => TrackInfoKind::Album,
                LibraryFilter::Artists => TrackInfoKind::Artist,
                // Genres and folders have no field of their own to describe, so
                // the card borrows the representative track's — the same one the
                // grid and the drill-down use.
                LibraryFilter::Tracks | LibraryFilter::Genres | LibraryFilter::Folders => {
                    TrackInfoKind::Track
                }
            };
        }
        match self.library_category {
            // The three levels of the Playlists view are three row types, each
            // with its own card.
            LIB_PLAYLISTS if self.playlist_row() => TrackInfoKind::PlaylistGroup,
            LIB_PLAYLISTS
                if self.playlist_open && self.playlist_group != PlaylistGroup::Playlists =>
            {
                TrackInfoKind::Track
            }
            LIB_PLAYLISTS => TrackInfoKind::Playlist,
            LIB_SPOTIFY => TrackInfoKind::SpotifyPlaylist,
            LIB_RADIO => TrackInfoKind::RadioStation,
            // Charts have their own row type, and which one depends on how deep
            // the drill-down is: the source list, an opened chart, or an opened
            // chart's tracks.
            LIB_CHARTS => match (self.charts.selected_source, self.charts.selected_chart) {
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
        // Artwork address for a row that has no local file behind it. "All
        // Tracks" is a union of Spotify playlists, so its rows are remote: they
        // carry a `cover_url` and no path, and `id` is 0 for every one of them.
        // Asking the local art cache for id 0 therefore returned nothing for
        // the whole list, which is why the left pane stayed empty there while
        // every other category filled in. Read alongside the id/path pair
        // because that pair is all the local path below needs.
        let mut remote_url: Option<String> = None;
        // A grouped row (a genre, or the drills into one) is a name, not a
        // track: it needs the track it stands for, or the card shows the
        // artwork of whatever row shares its index in the library.
        let maybe_track: Option<(i64, String)> = match kind {
            TrackInfoKind::Track => {
                if self.group_row() && self.browse_detail.is_none() {
                    let pos = self.list_pos();
                    if let Some(hit) = self.group_representative(pos) {
                        if !std::path::Path::new(&hit.path).is_absolute() {
                            remote_url = hit.cover_url.clone();
                        }
                        Some((hit.id, hit.path.clone()))
                    } else {
                        None
                    }
                } else {
                    let filtered = self.filtered_tracks();
                    let pos = self.list_pos();
                    filtered.get(pos).map(|hit| {
                        // Remote rows are everything the local art cache cannot
                        // serve: empty paths (chart-style rows) and provider URIs
                        // (`spotify:`, …) that happen to be stored in `path`,
                        // which is all of "All Tracks". Both carry `cover_url`.
                        if hit.path.is_empty() || !std::path::Path::new(&hit.path).is_absolute() {
                            remote_url = hit.cover_url.clone();
                        }
                        (hit.id, hit.path.clone())
                    })
                }
            }
            TrackInfoKind::Album | TrackInfoKind::Artist => {
                let pos = self.list_pos();
                let (hit, url) = self.group_representative(pos).map_or((None, None), |t| {
                    // A group whose representative is remote (a Spotify album
                    // row) has no library id to look the art up by; it is named
                    // by the cover url just like a remote track row.
                    let url = (!std::path::Path::new(&t.path).is_absolute())
                        .then(|| t.cover_url.clone())
                        .flatten();
                    (Some((t.id, t.path.clone())), url)
                });
                remote_url = url;
                hit
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
            | TrackInfoKind::RadioStation
            | TrackInfoKind::PlaylistGroup => None,
        };

        let valid = match kind {
            TrackInfoKind::PlaylistGroup => self.list_pos() < PlaylistGroup::ALL.len(),
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
            self.fetch_spot_cover();
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

        // A remote row: same slot as a chart row's, because both are a plain
        // URL rather than a library id. Checked before the local lookup below,
        // which would otherwise ask the art cache for id 0 and find nothing.
        if !std::path::Path::new(&path).is_absolute()
            && let Some(url) = remote_url.filter(|u| !u.is_empty())
        {
            self.popup_track_id = None;
            self.fetch_url_cover(Some(url));
            return;
        }

        self.popup_track_id = Some(tid);

        // Repaint from the last fetch of this id before anything else, so a
        // scroll back onto it does not blank while a fresh request is in
        // flight. Mirrors the url latch in `fetch_url_cover`.
        if let Some(bytes) = self
            .popup_id_cover
            .as_ref()
            .filter(|(id, _)| *id == tid)
            .map(|(_, b)| b.clone())
        {
            self.track_popup_cover = Some(bytes);
            self.popup_cover_sync();
            self.cover_art_dirty = true;
            self.popup_slot.clear();
            return;
        }

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
        //
        // The cover is cleared *before* this check rather than after it: an
        // early return here is the case where the row's own request is still in
        // flight, and leaving the previous row's bytes on the card put one
        // track's artwork under another track's title for as long as the fetch
        // took.
        if no_image_protocol() || self.popup_slot.pending(&tid) {
            self.track_popup_cover = None;
            self.popup_cover_stateful = None;
            return;
        }
        self.track_popup_cover = None;
        self.popup_cover_stateful = None;
        cover_get(
            &self.client,
            &self.ipc_tx,
            &mut self.popup_slot,
            tid,
            IpcResult::PopupCoverArt,
            move |c| {
                let (tid, path) = (tid, path.clone());
                Box::pin(async move {
                    // By id *and* path. The daemon resolves a library row by id
                    // first and only then falls back to the queue, so asking by
                    // id alone missed every row whose art is known from the queue
                    // entry — which is every row the resolver labelled without a
                    // local file behind it.
                    let enc = c.art().cover_for(tid, Some(path)).await.ok().flatten()?;
                    b64(enc)
                })
            },
        );
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
        let own = |i: usize| {
            self.charts
                .charts
                .get(i)
                .and_then(|c| c.cover_url.clone())
                .or_else(|| self.charts.cover_of(i))
        };
        match self.track_info_kind() {
            TrackInfoKind::Chart => own(pos),
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
        if no_image_protocol() {
            return;
        }
        // Repaint from what we still hold, which is what makes the latch safe.
        // The latch keys the bytes by their url: a hit repaints without a
        // request, and a miss re-fetches, so a url whose image failed is not
        // memoised as a hit.
        if let Some(bytes) = self
            .popup_cover_bytes
            .as_ref()
            .filter(|(u, _)| u == &url)
            .map(|(_, b)| b.clone())
        {
            self.track_popup_cover = Some(bytes);
            self.popup_cover_sync();
            self.cover_art_dirty = true;
            return;
        }
        // One in flight per url, like every other cover: without this a cursor
        // move re-claimed the slot and threw the previous request's answer away,
        // and a chart row — the only one named by a CDN url rather than a local
        // id — re-fetched faster than a cold image GET could answer, so its
        // cover never landed at all.
        if self.spotify_popup_slot.pending(&url) {
            return;
        }
        self.track_popup_cover = None;
        self.popup_cover_stateful = None;
        cover_get(
            &self.client,
            &self.ipc_tx,
            &mut self.spotify_popup_slot,
            url.clone(),
            IpcResult::SpotifyPopupCover,
            |c| Box::pin(async move { c.image_cover(&url).await.ok().flatten() }),
        );
    }

    /// Warm the artwork for the chart rows on screen and just off it, at
    /// whichever level is showing: the chart list or the tracks of an opened
    /// chart. Both name their image by a plain CDN URL, so both go out as image
    /// requests.
    ///
    /// The card shows one row's cover at a time, so without this every step of
    /// the scroll is a blank card until its request comes back — the list moves
    /// faster than the network. The daemon's image cache absorbs the repeats, so
    /// the fetch the card then makes is a hit. Mirrors the local-library warm
    /// above, which has ids rather than URLs.
    pub fn preload_chart_covers(&self) {
        if no_image_protocol() {
            return;
        }
        let first = self.list_scroll;
        let last = first
            .saturating_add(self.viewport_items.max(8))
            .saturating_add(3);
        let urls: Vec<String> = match self.track_info_kind() {
            TrackInfoKind::Chart => self.charts.charts
                [first.min(self.charts.charts.len())..last.min(self.charts.charts.len())]
                .iter()
                .filter_map(|c| c.cover_url.clone())
                .collect(),
            TrackInfoKind::ChartTrack => self.charts.chart_tracks[first
                .min(self.charts.chart_tracks.len())
                ..last.min(self.charts.chart_tracks.len())]
                .iter()
                .filter_map(|t| t.cover_url.clone())
                .collect(),
            _ => return,
        };
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
        // Robust: invalidate pending fetch_gen so close/reopen does not retain stale key
        let tid = self
            .pickers
            .top()
            .filter(|top| top.id == PickerId::SearchLibrary)
            .and_then(|top| {
                let picks = self.search_library_picks();
                let sel = top.selected.min(picks.len().saturating_sub(1));
                match picks.get(sel)? {
                    LibraryPick::Track(i) => self.tracks_cache.get(*i).map(|t| t.id),
                    _ => None,
                }
            });
        let Some(tid) = tid else {
            self.picker_preview_cover = None;
            self.picker_preview_stateful = None;
            self.picker_slot.clear();
            return;
        };
        // Generation-guarded dedup: id reuse (id==0) cannot block new fetches.
        // Only skip when both id and generation match current pending.
        if self.picker_slot.pending(&tid) {
            return;
        }
        self.picker_preview_cover = None;
        self.picker_preview_stateful = None;
        if no_image_protocol() {
            return;
        }
        cover_get(
            &self.client,
            &self.ipc_tx,
            &mut self.picker_slot,
            tid,
            IpcResult::PickerPreviewCover,
            move |c| Box::pin(async move { c.art().cover(tid).await.ok().flatten().and_then(b64) }),
        );
    }

    pub fn update_artist_cover(&mut self) {
        let name = self
            .pickers
            .top()
            .filter(|top| top.id == PickerId::SearchLibrary)
            .and_then(|top| {
                let picks = self.search_library_picks();
                let sel = top.selected.min(picks.len().saturating_sub(1));
                match picks.get(sel)? {
                    LibraryPick::Artist(name) => Some(name.clone()),
                    _ => None,
                }
            });
        let Some(name) = name else {
            self.artist_cover = None;
            self.artist_cover_stateful = None;
            self.artist_slot.clear();
            return;
        };
        if self.artist_slot.pending(&name) {
            return;
        }
        self.artist_cover = None;
        self.artist_cover_stateful = None;
        if no_image_protocol() {
            return;
        }
        cover_get(
            &self.client,
            &self.ipc_tx,
            &mut self.artist_slot,
            name.clone(),
            IpcResult::ArtistCoverArt,
            move |c| {
                let artist = name.clone();
                Box::pin(async move {
                    c.art()
                        .artist_cover(artist)
                        .await
                        .ok()
                        .flatten()
                        .and_then(b64)
                })
            },
        );
    }

    /// Whether the cover grid can be shown now, saying why not if it cannot.
    ///
    /// Refuses out loud on a list with nothing to show as a grid: a key that
    /// does nothing is worse than one that says why. Shared by the `V` key and
    /// the palette row, which are one command and so agree about it.
    pub fn grid_ready(&mut self) -> bool {
        if !self.group_row() {
            self.notify_typed(
                "System",
                "The grid is for albums, artists and genres",
                NotificationKind::Info,
                false,
                NotifType::Library,
            );
            return false;
        }
        if self.browse_detail.is_some() {
            self.notify_typed(
                "System",
                "Backspace out of this list to use the grid",
                NotificationKind::Info,
                false,
                NotifType::Library,
            );
            return false;
        }
        if no_image_protocol() {
            self.notify_typed(
                "System",
                "The cover grid needs image rendering",
                NotificationKind::Info,
                false,
                NotifType::Library,
            );
            return false;
        }
        true
    }

    /// Enter or leave zen, always on the now-playing surface.
    ///
    /// That surface whatever was cycled to last: the lyrics and the visualizer
    /// are Tab's to choose, not the key's.
    pub fn toggle_zen(&mut self) {
        self.zen = !self.zen;
        if self.zen {
            self.zen_surface = ZenSurface::NowPlaying;
            self.dismiss_track_popup();
        }
        // The cover for this track may never have been asked for: the band can
        // be hidden behind a list, a picker's preview can claim the slot, or the
        // fetch answered before the track settled. Zen draws the same bytes, so
        // ask for them on the way in rather than showing the placeholder to
        // full screen.
        self.fetch_np_cover();
    }

    /// Switch the album/artist/genre lists between rows and a cover grid.
    ///
    /// The setting is per-app, not per-category, and it survives switching
    /// categories: browsing albums as a grid and then artists is the same
    /// gesture twice, and having to ask again each time was the friction the
    /// grid was meant to remove. `true` when the grid is now showing.
    pub fn toggle_grid(&mut self) -> bool {
        if self.grid_active() {
            self.grid.on = false;
        } else if self.grid_filter() && self.browse_detail.is_none() {
            self.grid.on = true;
        }
        self.grid.on
    }

    /// The names the grid draws, one per cell, in list order.
    ///
    /// Read from the same helpers the row view reads, so the two cannot drift:
    /// a grid cell is an album, an artist or a genre because its row was one.
    pub fn grid_labels(&self) -> Vec<String> {
        self.library_groups().into_iter().map(|(n, _)| n).collect()
    }

    /// Whether the Library filter on screen is one of the grouped lists the
    /// grid can draw. The filter is what decides now that the four of them are
    /// one view: the same view is a grid for three of its four groupings.
    pub fn grid_filter(&self) -> bool {
        self.library_category == LIB_ALL && !matches!(self.library_filter, LibraryFilter::Tracks)
    }

    /// Whether the active category draws as a cover grid.
    ///
    /// Albums and artists are the lists with covers to browse; genres borrow a
    /// representative track's artwork, which is a real album sleeve for the
    /// genre rather than a picture of the genre, but it is the only artwork a
    /// genre has and a grid of grey cells is not a browse view.
    ///
    /// Off entirely without an image protocol: the grid is forty identical
    /// placeholders in a row, which is strictly less useful than the list it
    /// replaced.
    pub fn grid_active(&self) -> bool {
        self.grid.on && self.browse_detail.is_none() && !no_image_protocol() && self.grid_filter()
    }

    /// Where the grid puts its cells in a pane this size, with the selected
    /// item in view.
    ///
    /// The window is stepped rather than recomputed from the cursor alone, by
    /// the same helper the row lists use: a move inside the window leaves it
    /// alone, so a single step does not reflow the grid under the cursor.
    pub fn grid_plan(&self, area: Rect, total: usize) -> GridPlan {
        let cols = (area.width / GRID_CELL_W).max(1) as usize;
        // One row is the blank line the row view starts with, and the last is the
        // stats line drawn under the list.
        let rows = (area.height.saturating_sub(2) / GRID_CELL_H).max(1) as usize;
        let page = cols * rows;
        let sel = self.list_pos().min(total.saturating_sub(1));
        let (first, _end) = step_viewport(self.grid.first, sel, page, total);
        GridPlan { first, cols, rows }
    }

    /// Record a plan and ask for the covers of the cells it shows.
    ///
    /// Called from the draw path rather than the cursor path, because the cells
    /// on screen are a function of the pane's height: a resize brings in cells no
    /// keypress will ever mention. Bounded by [`GRID_FETCH_BATCH`] per call, so
    /// a first paint of a wide pane queues a few cells a frame instead of forty
    /// at once.
    pub fn grid_fetch(&mut self, plan: &GridPlan) {
        if !self.grid_active() {
            return;
        }
        self.grid.cols = plan.cols;
        self.grid.rows = plan.rows;
        self.grid.first = plan.first;
        // The representative id of every cell on screen. Recomputed each time,
        // because the underlying list changes under a rescan without any cursor
        // move.
        let ids: Vec<Option<i64>> = (plan.first..plan.first + plan.cols * plan.rows)
            .map(|pos| self.track_id_at(pos))
            .collect();
        if ids != self.grid.ids {
            self.grid.ids = ids;
            self.grid.round = self.grid.round.wrapping_add(1);
        }

        let missing: Vec<i64> = self
            .grid
            .ids
            .iter()
            .filter_map(|id| *id)
            .filter(|id| !self.grid.asked.contains(id) && !self.grid.covers.contains_key(id))
            .take(GRID_FETCH_BATCH)
            .collect();
        if missing.is_empty() {
            return;
        }
        for id in &missing {
            self.grid.asked.insert(*id);
        }
        let client = self.client.clone();
        let ipc_tx = self.ipc_tx.clone();
        let round = self.grid.round;
        tokio::spawn(async move {
            for id in missing {
                // Answer on a miss too, so a track with no artwork is not
                // re-requested on every frame after it.
                let bytes = match client.art().cover(id).await {
                    Ok(Some(b64)) => base64::engine::general_purpose::STANDARD.decode(&b64).ok(),
                    _ => None,
                };
                let _ = ipc_tx.send(IpcResult::GridCover(bytes, id, round));
            }
        });
    }

    /// Store a cover the grid asked for, decoding it once.
    pub(crate) fn grid_put(&mut self, track_id: i64, bytes: Vec<u8>) {
        let picker = self.np_cover.picker.clone();
        self.grid
            .covers
            .insert(track_id, GridCell::new(bytes, picker.as_ref()));
        if self.grid.covers.len() > GRID_CACHE_MAX {
            self.grid.prune_hard();
        }
        // One repaint: the cell is on screen and its artwork just arrived. The
        // periodic repaint would get there within ten frames, which for a
        // first paint is the whole visible delay.
        self.data_dirty = true;
    }

    /// Preload the cover art for the rows a short scroll ahead of the cursor, so
    /// fast scrolling (e.g. holding an arrow key) warms the daemon's disk/LRU
    /// cache and the on-selection fetch becomes a cache hit. Fires in the
    /// background and never blocks the UI or surfaces errors.
    ///
    /// Called on every cursor move in every category, and each one starts from
    /// what the current list actually holds rather than assuming a shape: a
    /// chart's rows are CDN URLs with no library id, and Spotify drill-down rows
    /// are album images. Both are named by URL, so they go out as image
    /// requests; only a local row has an id to look up.
    pub fn preload_row_covers(&mut self) {
        self.preload_spot_covers();
        // Chart rows have no library id, so `track_id_at` finds nothing for
        // them and the loop below would warm nothing.
        self.preload_chart_covers();
        let pos = self.list_pos();
        let mut ids = Vec::new();
        let mut urls = Vec::new();
        for off in 1..=3 {
            // `track_id_at` answers 0 for a row with no library row, and the
            // local art cache has nothing under 0: those were three identical
            // dead lookups per cursor move. A provider row is named by its
            // album-art url, so it warms through that instead.
            if let Some(id) = self.track_id_at(pos + off)
                && id != 0
            {
                ids.push(id);
            }
            if let Some(url) = self
                .filtered_tracks()
                .get(pos + off)
                .filter(|t| !std::path::Path::new(&t.path).is_absolute())
                .and_then(|t| t.cover_url.clone())
                .filter(|u| !u.is_empty())
            {
                urls.push(url);
            }
        }
        if ids.is_empty() && urls.is_empty() {
            return;
        }
        let client = self.client.clone();
        tokio::spawn(async move {
            for id in ids {
                // Errors (track without cover / daemon lookup fail) are fine:
                // a warm miss is simply skipped next time.
                let _ = client.art().cover(id).await;
            }
            for url in urls {
                let _ = client.image_cover(&url).await;
            }
        });
    }

    pub(crate) fn cover_sync(&mut self) {
        self.np_cover.stateful = self.protocol(self.np_cover.image.as_deref());
    }

    pub(crate) fn popup_cover_sync(&mut self) {
        self.popup_cover_stateful = self.protocol(self.track_popup_cover.as_deref());
    }

    /// The PCM of whatever is playing, resampled to `w` columns.
    ///
    /// The ring is interleaved L/R, so a column reads every other sample and a
    /// column wider than the ring repeats its last frame rather than running off
    /// the end of the buffer — the bug that made a wide window show half a
    /// waveform and then flat silence.
    pub fn wave_columns(&self, w: u16) -> Vec<f32> {
        let ring = &self.state.wave_samples;
        let n = ring.len();
        (0..w as usize)
            .map(|x| {
                if n < 2 {
                    return 0.0;
                }
                let i = ((x as f64 * n as f64 / w.max(1) as f64) as usize / 2 * 2).min(n - 2);
                ring[i]
            })
            .collect()
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
        cover_get(
            &self.client,
            &self.ipc_tx,
            &mut self.queue.preview_slot,
            key.clone(),
            IpcResult::QueuePreviewCover,
            move |c| {
                let key = key.clone();
                Box::pin(async move {
                    let enc = c.art().cover_for(tid, Some(key)).await.ok().flatten()?;
                    b64(enc)
                })
            },
        );
    }

    pub(crate) fn sync_preview_cover(&mut self) {
        self.queue.preview_cover_stateful = self.protocol(self.queue.preview_cover.as_deref());
    }

    pub(crate) fn picker_preview_sync(&mut self) {
        self.picker_preview_stateful = self.protocol(self.picker_preview_cover.as_deref());
    }

    /// (Re)build the stateful cover protocol for the Edit Metadata preview.
    pub(crate) fn metadata_cover_sync(&mut self) {
        self.metadata.cover_stateful = self.protocol(self.metadata.cover.as_deref());
    }

    pub(crate) fn artist_cover_sync(&mut self) {
        self.artist_cover_stateful = self.protocol(self.artist_cover.as_deref());
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
        // Clear stale cover while new fetch is in flight; handler will repopulate.
        self.metadata.cover = None;
        self.metadata.cover_stateful = None;
        cover_get(
            &self.client,
            &self.ipc_tx,
            &mut self.metadata.cover_fetch,
            track_id,
            IpcResult::MetadataCoverArt,
            move |c| {
                Box::pin(async move { c.art().cover(track_id).await.ok().flatten().and_then(b64) })
            },
        );
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
