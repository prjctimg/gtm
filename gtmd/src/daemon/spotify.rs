use super::*;

use librespot_core::spotify_uri::SpotifyUri;

pub(crate) struct Spotify;

/// Display metadata for a track being queued for native streaming. Grouped
/// into one argument so [`Spotify::queue_stream`] stays readable and under the
/// argument ceiling as fields are added.
pub(crate) struct StreamMeta<'a> {
    pub title: &'a str,
    pub artist: &'a str,
    pub album: &'a str,
    /// Album art from the web API. Preferred over a text cover search, which
    /// misses often enough to leave spotify rows with no artwork.
    pub image_url: Option<&'a str>,
    pub duration: Option<f64>,
}

/// True when `uri` is a `spotify:track:` (or episode) URI librespot can
/// actually stream. A track id is 22 base62 characters; anything else is a
/// caller bug or a stale cache entry, and streaming it would fail deep inside
/// the protocol layer with a message that names neither the URI nor the song.
pub(crate) fn is_playable(uri: &str) -> bool {
    // A playable URI is `spotify:<kind>:<id>` — exactly three colon-separated
    // parts. A fourth means the prefix was applied twice, to something that
    // was already a URI (`spotify:track:spotify:track:<id>`). `SpotifyUri`
    // parses that happily and hands the inner `spotify:track:<id>` back as the
    // id, so the doubled form would otherwise pass every check below and only
    // fail once librespot is already streaming.
    if uri.split(':').count() > 3 {
        return false;
    }
    match SpotifyUri::from_uri(uri) {
        Ok(parsed) => parsed.is_playable() && parsed.to_id().is_ok(),
        Err(_) => false,
    }
}

/// Clone of the Web API client, or the "not linked" error reply. Callers must
/// drop this before any `.await` that reaches Spotify: the manager mutex is
/// also taken by the cover paths, and holding it across a network call is
/// what wedged playback, cover art and search.
pub(crate) async fn linked(inner: &DaemonInner) -> Result<AuthCodePkceSpotify, Box<DaemonRes>> {
    match inner.spotify.lock().await.client() {
        Some(client) => Ok(client),
        None => Err(Box::new(DaemonRes::Error {
            message: "spotify not linked".into(),
        })),
    }
}

/// Whether a `spotify:track:` URI can stream natively right now: a linked
/// Premium account with a usable access token. The manager lock is released
/// before the token check, so this never pins it across a refresh.
pub(crate) async fn can_stream(inner: &DaemonInner) -> bool {
    let (premium, client) = {
        let spotify = inner.spotify.lock().await;
        (spotify.is_premium(), spotify.client())
    };
    match client {
        Some(client) => premium && access_token(&client).await.is_ok(),
        None => false,
    }
}

/// One Spotify Connect transport command, dispatched by
/// [`Spotify::connect_ctrl`] so every control shares the same status refresh
/// and error mapping.
pub(crate) enum ConnectCmd {
    PlayPause,
    Next,
    Previous,
    Seek(u32),
    Shuffle(bool),
    Repeat(String),
    Volume(u8),
}

impl Spotify {
    pub async fn set_token(inner: &DaemonInner, token: &str) -> Result<DaemonRes, CoreError> {
        let mut spotify = inner.spotify.lock().await;
        match tokio::time::timeout(Duration::from_secs(60), spotify.set_token(token)).await {
            Ok(Ok(())) => Ok(DaemonRes::SpotifyStatusRes {
                status: spotify.status(),
            }),
            Ok(Err(e)) => Ok(DaemonRes::Error {
                message: format!("spotify token failed: {e}"),
            }),
            Err(_) => Ok(DaemonRes::Error {
                message: "spotify token link timed out".into(),
            }),
        }
    }

    /// Kick off an OAuth PKCE link flow: build the authorize URL, serve the
    /// redirect on the supplied local port in a background task, and exchange +
    /// persist the token when the browser round-trip completes. `port` lets the
    /// user reuse a redirect URI already registered in their Spotify dashboard.
    pub async fn oauth_start(
        inner: &Arc<DaemonInner>,
        client_id: &str,
        port: u16,
    ) -> Result<DaemonRes, CoreError> {
        // Abort any previous pending flow so its listener socket is freed.
        if let Some(handle) = inner.oauth_task.lock().await.take() {
            handle.abort();
        }

        let cid = client_id.trim();
        if cid.is_empty() {
            return Err(CoreError::Daemon("empty spotify client id".into()));
        }
        if cid.len() != 32 || !cid.chars().all(|c| c.is_ascii_hexdigit()) {
            return Err(CoreError::Daemon(format!(
                "invalid spotify client id (expected 32 hex chars) — your app must also \
                 list http://127.0.0.1:{port}/login as a Redirect URI \
                 (127.0.0.1, not localhost) or the link fails silently in the browser"
            )));
        }
        // Persist the client id in the config dir and the OS keychain so a
        // restart can still refresh with the same app: the refresh token is
        // only valid for the client id it was issued to.
        if let Err(e) = inner.spotify.lock().await.save_client_id(cid) {
            warn!("spotify: could not persist client id: {e}");
        }
        let flow = OauthFlow::new(cid, port);
        // Bind the loopback callback server *before* returning the URL so the
        // browser always opens to a live listener (a previously spawned task
        // raced the bind, hitting a dead port).
        let listener = flow
            .listen()
            .await
            .map_err(|e| CoreError::Daemon(format!("OAuth callback server: {e}")))?;
        let url = flow.authorize_url();

        let inner2 = Arc::clone(inner);
        let handle = tokio::spawn(async move {
            match flow.wait_token(listener).await {
                Ok(token) => {
                    let mut spotify = inner2.spotify.lock().await;
                    // Link (persist token + build client) without the full
                    // playlist sync so the TUI's auth picker can close right
                    // away; the sync runs below on a cloned client and emits
                    // a second event when the playlist cache is ready.
                    match spotify.link(&token).await {
                        Ok(()) => {
                            info!("spotify oauth link complete (client ready)");
                        }
                        Err(e) => {
                            warn!("spotify oauth link failed: {e}");
                            spotify.set_error(format!("oauth link failed: {e}"));
                        }
                    }
                    drop(spotify);
                    // Immediately tell the TUI the account is linked so the
                    // picker closes and playlist loading commences.
                    let _ = inner2.event_tx.send(DaemonEvent::SpotifyStatusChanged);
                    // The charts registry is built before any token exists, so
                    // register the Spotify provider now that a client is ready
                    // (idempotent: a no-op when already registered).
                    {
                        let mut charts = inner2.charts.lock().await;
                        charts.ensure_spotify(inner2.spotify.clone());
                    }
                    // Background playlist sync. It pages every playlist on a
                    // cloned client so the manager mutex is never held across
                    // the network pass; no artificial timeout here — the first
                    // sync after linking can legitimately take longer than a
                    // minute on large libraries.
                    let inner3 = Arc::clone(&inner2);
                    tokio::spawn(async move {
                        let client = { inner3.spotify.lock().await.client() };
                        let Some(client) = client else {
                            return;
                        };
                        match SpotifyManager::run_sync(client).await {
                            Ok((user, playlists)) => {
                                {
                                    let mut spotify = inner3.spotify.lock().await;
                                    // Guard: the user may have unlinked while we
                                    // paginated; do not resurrect credentials.
                                    if spotify.linked() {
                                        let count = playlists.len();
                                        spotify.commit_sync(user, playlists);
                                        info!("spotify playlists synced ({:?} playlists)", count);
                                    }
                                }
                                // Bounded playback probe off the commit lock.
                                {
                                    let mut spotify = inner3.spotify.lock().await;
                                    if spotify.linked() {
                                        let _ = tokio::time::timeout(
                                            Duration::from_secs(10),
                                            spotify.refresh_playback(),
                                        )
                                        .await;
                                    }
                                }
                            }
                            Err(e) => {
                                warn!("spotify playlist sync failed: {e}");
                                let mut spotify = inner3.spotify.lock().await;
                                if spotify.linked() {
                                    spotify.set_error(format!("playlist sync failed: {e}"));
                                }
                            }
                        }
                        let _ = inner3.event_tx.send(DaemonEvent::SpotifyStatusChanged);
                    });
                }
                Err(e) => {
                    warn!("spotify oauth link failed: {e}");
                    let mut spotify = inner2.spotify.lock().await;
                    spotify.set_error(format!("OAuth link failed: {e}"));
                    drop(spotify);
                    let _ = inner2.event_tx.send(DaemonEvent::SpotifyStatusChanged);
                }
            }
        });
        *inner.oauth_task.lock().await = Some(handle);

        Ok(DaemonRes::SpotifyOauthStarted { url })
    }

    pub async fn oauth_cancel(inner: &DaemonInner) -> Result<DaemonRes, CoreError> {
        if let Some(handle) = inner.oauth_task.lock().await.take() {
            handle.abort();
            info!("spotify oauth link flow cancelled");
        }
        Ok(DaemonRes::Ok)
    }

    pub async fn clear(inner: &DaemonInner) -> Result<DaemonRes, CoreError> {
        let mut spotify = inner.spotify.lock().await;
        spotify.clear();
        Ok(DaemonRes::SpotifyStatusRes {
            status: spotify.status(),
        })
    }

    pub async fn status(inner: &std::sync::Arc<DaemonInner>) -> Result<DaemonRes, CoreError> {
        // Refresh the Web API playback state in the background: the call can
        // stall for up to 5 seconds and must never hold the Spotify manager
        // mutex, which would queue every other Spotify command (play/pause,
        // resolve, sync, oauth cancel) behind a `current_playback` probe.
        let inner2 = inner.clone();
        tokio::spawn(async move {
            let mut spotify = inner2.spotify.lock().await;
            let _ = tokio::time::timeout(Duration::from_secs(5), spotify.refresh_playback()).await;
        });
        let spotify = inner.spotify.lock().await;
        Ok(DaemonRes::SpotifyStatusRes {
            status: spotify.status(),
        })
    }

    pub async fn play_pause(inner: &DaemonInner) -> Result<DaemonRes, CoreError> {
        Self::connect_ctrl(inner, ConnectCmd::PlayPause).await
    }

    /// Run one Spotify Connect control and answer with the refreshed status.
    /// Every control shares the same locking, error mapping and response shape;
    /// only the Web API call differs.
    pub(crate) async fn connect_ctrl(
        inner: &DaemonInner,
        cmd: ConnectCmd,
    ) -> Result<DaemonRes, CoreError> {
        let mut spotify = inner.spotify.lock().await;
        let res = match cmd {
            ConnectCmd::PlayPause => spotify.play_pause().await,
            ConnectCmd::Next => spotify.next().await,
            ConnectCmd::Previous => spotify.previous().await,
            ConnectCmd::Seek(pos) => spotify.seek(pos).await,
            ConnectCmd::Shuffle(on) => spotify.set_shuffle(on).await,
            ConnectCmd::Repeat(ref mode) => spotify.set_repeat(mode).await,
            ConnectCmd::Volume(pct) => spotify.set_volume(pct).await,
        };
        match res {
            Ok(()) => Ok(DaemonRes::SpotifyStatusRes {
                status: spotify.status(),
            }),
            Err(e) => Ok(DaemonRes::Error { message: e }),
        }
    }

    pub async fn sync(inner: &DaemonInner) -> Result<DaemonRes, CoreError> {
        // Clone the Web API client out of the manager, then paginate without
        // holding `inner.spotify`: a concurrent `SpotifyStatus`/
        // `SpotifyPlaylists` keeps working against the previous snapshot.
        let client = match linked(inner).await {
            Ok(client) => client,
            Err(res) => return Ok(*res),
        };
        let res =
            tokio::time::timeout(Duration::from_secs(60), SpotifyManager::run_sync(client)).await;
        match res {
            Ok(Ok((user, playlists))) => {
                let mut spotify = inner.spotify.lock().await;
                spotify.commit_sync(user, playlists);
                spotify.refresh_playback().await;
                Ok(DaemonRes::Ok)
            }
            Ok(Err(e)) => Ok(DaemonRes::Error { message: e }),
            Err(_) => Ok(DaemonRes::Error {
                message: "spotify sync timed out".into(),
            }),
        }
    }

    pub async fn playlists(inner: &DaemonInner) -> Result<DaemonRes, CoreError> {
        let spotify = inner.spotify.lock().await;
        if !spotify.linked() {
            return Ok(DaemonRes::Error {
                message: "spotify not linked".into(),
            });
        }
        Ok(DaemonRes::SpotifyPlaylistsRes {
            playlists: spotify.playlists(),
        })
    }

    pub async fn playlist_tracks(inner: &DaemonInner, id: &str) -> Result<DaemonRes, CoreError> {
        let spotify = inner.spotify.lock().await;
        if !spotify.linked() {
            return Ok(DaemonRes::Error {
                message: "spotify not linked".into(),
            });
        }
        match spotify.playlist_tracks(id) {
            Some(tracks) => Ok(DaemonRes::SpotifyTracksRes { tracks }),
            None => Ok(DaemonRes::Error {
                message: "unknown spotify playlist".into(),
            }),
        }
    }

    #[cfg_attr(not(feature = "youtube"), allow(unused_variables))]
    pub async fn resolve(
        inner: &DaemonInner,
        playlist_id: &str,
        track_index: usize,
        play: bool,
    ) -> Result<DaemonRes, CoreError> {
        let track = {
            let spotify = inner.spotify.lock().await;
            spotify
                .playlist_tracks(playlist_id)
                .and_then(|tracks| tracks.into_iter().find(|t| t.index == track_index))
        };
        let Some(track) = track else {
            return Ok(DaemonRes::Error {
                message: "track not found in spotify cache".into(),
            });
        };
        let spotify_title = track.name.clone();
        let spotify_artist = track.artists.clone();
        let spotify_album = track.album.clone().unwrap_or_default();

        // Premium accounts stream natively via librespot; the queue entry
        // carries the `spotify:track:` URI and Cmd::play routes it to the
        // streaming bridge. Everyone else falls back to the YT match.
        if can_stream(inner).await
            && let Some(uri) = track.uri.clone()
        {
            let duration = track.duration_ms.map(|ms| ms as f64 / 1000.0);
            return Spotify::queue_stream(
                inner,
                &uri,
                StreamMeta {
                    title: &spotify_title,
                    artist: &spotify_artist,
                    album: &spotify_album,
                    image_url: track.image_url.as_deref(),
                    duration,
                },
                play,
            )
            .await;
        }

        let query = if track.artists.is_empty() {
            track.name.clone()
        } else {
            format!("{} - {}", track.artists, track.name)
        };

        let path = match spotify_yt_fallback(
            inner,
            &format!("spotify-{playlist_id}-{track_index}"),
            &query,
        )
        .await
        {
            Ok(path) => path,
            Err(message) => {
                return Ok(DaemonRes::Error {
                    message: message.to_string(),
                });
            }
        };

        let spotify_title = track.name.clone();
        let spotify_artist = track.artists.clone();
        let spotify_album = track.album.unwrap_or_default();
        let was_empty = {
            let mut state = inner.state.write().await;
            state.fallback_disabled = false;
            let w = state.queue.is_empty() && state.status == PlaybackStatus::Stopped;
            let added = queue::add(&mut state, &path, None);
            if let Some(entry) = state.queue.iter_mut().rev().find(|t| t.path == added.path) {
                entry.title = spotify_title.clone();
                entry.artist = spotify_artist.clone();
                entry.album = spotify_album.clone();
            }
            drop(state);
            w
        };
        // Start playback reliably: on an empty queue, and always when the
        // caller asked to play (Enter) — `Cmd::play` stops the current source
        // first, so switching from another source is smooth.
        if was_empty || play {
            let _ = Cmd::play(inner, &path, 0.0, false).await;
        }

        {
            let mut guard = inner.cover_cache().await;
            if let Some(ref mut cc) = *guard {
                let _ = cc
                    .get(
                        &spotify_artist,
                        &spotify_album,
                        inner.effective_cover_provider().await,
                    )
                    .await;
            }
        }

        Daemon::push_queue_state(inner).await;
        Daemon::save_state(inner);
        Ok(DaemonRes::Ok)
    }

    pub async fn search_web(inner: &DaemonInner, query: &str) -> Result<DaemonRes, CoreError> {
        let client = match linked(inner).await {
            Ok(client) => client,
            Err(res) => return Ok(*res),
        };
        let tracks = search(&client, query, 20)
            .await
            .map_err(CoreError::Daemon)?;
        Ok(DaemonRes::SpotifyTracksRes { tracks })
    }

    /// Resolve a web-search album result (an album `spotify:` URI) to its full
    /// track list so the TUI can queue and play it.
    pub async fn album_tracks(inner: &DaemonInner, uri: &str) -> Result<DaemonRes, CoreError> {
        let client = match linked(inner).await {
            Ok(client) => client,
            Err(res) => return Ok(*res),
        };
        match album_tracks(&client, uri).await {
            Ok(tracks) => Ok(DaemonRes::SpotifyTracksRes { tracks }),
            Err(e) => Ok(DaemonRes::Error { message: e }),
        }
    }

    /// Resolve a web-search artist result (an artist `spotify:` URI) to their
    /// top tracks.
    pub async fn artist_top_tracks(inner: &DaemonInner, uri: &str) -> Result<DaemonRes, CoreError> {
        let client = match linked(inner).await {
            Ok(client) => client,
            Err(res) => return Ok(*res),
        };
        match artist_top(&client, uri).await {
            Ok(tracks) => Ok(DaemonRes::SpotifyTracksRes { tracks }),
            Err(e) => Ok(DaemonRes::Error { message: e }),
        }
    }

    /// Resolve a web-search playlist result (a `spotify:playlist:` URI) to its
    /// track list.
    pub async fn web_playlist_tracks(
        inner: &DaemonInner,
        uri: &str,
    ) -> Result<DaemonRes, CoreError> {
        let client = match linked(inner).await {
            Ok(client) => client,
            Err(res) => return Ok(*res),
        };
        match web_playlist(&client, uri).await {
            Ok(tracks) => Ok(DaemonRes::SpotifyTracksRes { tracks }),
            Err(e) => Ok(DaemonRes::Error { message: e }),
        }
    }

    /// Resolve a Spotify track into a playable stream and append it to the
    /// user queue. On a Premium account an accompanying `spotify:track:` URI
    /// streams natively via librespot; everyone else falls back to the top
    /// YouTube match for the track metadata.
    #[allow(clippy::too_many_arguments)]
    pub async fn resolve_track(
        inner: &DaemonInner,
        name: &str,
        artists: &str,
        album: &str,
        uri: &Option<String>,
        image_url: Option<&str>,
        play: bool,
    ) -> Result<DaemonRes, CoreError> {
        if uri.is_some()
            && can_stream(inner).await
            && let Some(uri) = uri.clone()
        {
            let duration = {
                let spotify = inner.spotify.lock().await;
                spotify.find_track_duration(&uri)
            };
            Spotify::queue_stream(
                inner,
                &uri,
                StreamMeta {
                    title: name,
                    artist: artists,
                    album,
                    image_url,
                    duration,
                },
                play,
            )
            .await?;
            return Ok(DaemonRes::Ok);
        }

        let query = if artists.is_empty() {
            name.to_string()
        } else {
            format!("{artists} - {name}")
        };
        let path = match spotify_yt_fallback(inner, &format!("spotify-web-{name}"), &query).await {
            Ok(path) => path,
            Err(message) => {
                return Ok(DaemonRes::Error {
                    message: message.to_string(),
                });
            }
        };

        let was_empty = {
            let mut state = inner.state.write().await;
            state.fallback_disabled = false;
            let w = state.queue.is_empty() && state.status == PlaybackStatus::Stopped;
            let added = queue::add(&mut state, &path, None);
            if let Some(entry) = state.queue.iter_mut().rev().find(|t| t.path == added.path) {
                entry.title = name.to_string();
                entry.artist = artists.to_string();
                entry.album = album.to_string();
            }
            drop(state);
            w
        };
        // Start playback reliably: on an empty queue, and always when the
        // caller asked to play (Enter) — `Cmd::play` stops the current source
        // first, so switching from another source is smooth.
        if was_empty || play {
            let _ = Cmd::play(inner, &path, 0.0, false).await;
        }

        {
            let mut guard = inner.cover_cache().await;
            if let Some(ref mut cc) = *guard {
                let _ = cc
                    .get(artists, album, inner.effective_cover_provider().await)
                    .await;
            }
        }

        Daemon::push_queue_state(inner).await;
        Daemon::save_state(inner);
        Ok(DaemonRes::Ok)
    }

    /// Enqueue a native `spotify:track:` URI into the user queue with the
    /// given title/artist/album metadata, pre-warm the cover cache for the
    /// album, and start playback when the queue was empty or the caller asked
    /// to play. Mirrors the Premium branch of `resolve`.
    ///
    /// Every native-stream path funnels through here, so the URI is validated
    /// once: librespot rejects a malformed id with an opaque
    /// "ID cannot be parsed" from deep inside its protobuf layer, long after
    /// the request has already been queued and reported as a success.
    pub(crate) async fn queue_stream(
        inner: &DaemonInner,
        uri: &str,
        meta: StreamMeta<'_>,
        play: bool,
    ) -> Result<DaemonRes, CoreError> {
        let StreamMeta {
            title,
            artist,
            album,
            image_url,
            duration,
        } = meta;
        if !is_playable(uri) {
            return Ok(DaemonRes::Error {
                message: format!("not a playable spotify uri: {uri}"),
            });
        }
        let was_empty = {
            let mut state = inner.state.write().await;
            let w = state.queue.is_empty() && state.status == PlaybackStatus::Stopped;
            let added = queue::add(&mut state, uri, None);
            if let Some(entry) = state.queue.iter_mut().rev().find(|t| t.path == added.path) {
                entry.title = title.to_string();
                entry.artist = artist.to_string();
                entry.album = album.to_string();
                if let Some(duration) = duration {
                    entry.duration = duration;
                }
            }
            drop(state);
            w
        };
        // Start playback reliably: on an empty queue, and always when the
        // caller asked to play (Enter) — `Cmd::play` stops the current source
        // first, so switching from another source is smooth. A rejected
        // librespot handshake answers `Ok(Error)`, so surface it instead of
        // reporting a successful queue for a track that never plays.
        if (was_empty || play)
            && let DaemonRes::Error { message } = Cmd::play(inner, uri, 0.0, false).await?
        {
            return Ok(DaemonRes::Error { message });
        }

        // The web API hands us the album art directly, so use it instead of
        // searching Deezer/MusicBrainz for `artist - album`: that misses often
        // enough that spotify rows rendered with no cover at all. Clone the
        // client before taking the cache — holding the cache guard while the
        // spotify manager is locked is the inversion that wedged playback and
        // cover art together.
        let client = match image_url {
            Some(_) => linked(inner).await.ok(),
            None => None,
        };
        {
            let mut guard = inner.cover_cache().await;
            if let Some(ref mut cc) = *guard {
                match (image_url, client.as_ref()) {
                    (Some(url), Some(cl)) => {
                        let _ = cc.get_url(url, || image_at(cl, url)).await;
                    }
                    _ => {
                        let _ = cc
                            .get(artist, album, inner.effective_cover_provider().await)
                            .await;
                    }
                }
            }
        }

        // Point the queued entry at the file just warmed so the TUI renders the
        // row without looking the artwork up a second time.
        if let Some(url) = image_url {
            let path = inner
                .cover_cache()
                .await
                .as_ref()
                .and_then(|cc| cc.url_disk_path(url));
            if let Some(path) = path
                && path.exists()
            {
                let mut state = inner.state.write().await;
                if let Some(entry) = state.queue.iter_mut().rev().find(|t| t.path == uri) {
                    entry.cover_path = Some(path.to_string_lossy().into_owned());
                }
            }
        }

        Daemon::push_queue_state(inner).await;
        Daemon::save_state(inner);
        Ok(DaemonRes::Ok)
    }

    /// Play every track of a synced Spotify playlist. With `shuffle` the order
    /// is randomised first. Premium enqueues all `spotify:track:` URIs and
    /// starts the first; non-Premium resolves the first via YouTube and lazily
    /// resolves + enqueues the rest in the background.
    pub async fn play_all(
        inner: &Arc<DaemonInner>,
        playlist_id: &str,
        shuffle: bool,
    ) -> Result<DaemonRes, CoreError> {
        let tracks = {
            let spotify = inner.spotify.lock().await;
            spotify.playlist_tracks(playlist_id).unwrap_or_default()
        };
        if tracks.is_empty() {
            return Ok(DaemonRes::Error {
                message: "spotify playlist not in cache (run Sync first)".into(),
            });
        }
        let mut order: Vec<usize> = (0..tracks.len()).collect();
        if shuffle {
            fastrand::shuffle(&mut order);
        }

        let can_stream = can_stream(inner).await;

        if can_stream {
            let pairs: Vec<(String, SpotifyTrack)> = order
                .iter()
                .filter_map(|&i| tracks.get(i))
                .filter_map(|t| t.uri.clone().map(|uri| (uri, t.clone())))
                .collect();
            if pairs.is_empty() {
                return Ok(DaemonRes::Error {
                    message: "playlist tracks carry no streamable spotify URIs".into(),
                });
            }
            let uris: Vec<String> = pairs.iter().map(|(uri, _)| uri.clone()).collect();
            let was_empty = {
                let mut state = inner.state.write().await;
                let w = state.queue.is_empty() && state.status == PlaybackStatus::Stopped;
                let added = queue::add_many(&mut state, &uris, None);
                for (entry, (_, st)) in added.iter().zip(pairs.iter()) {
                    if let Some(e) = state.queue.iter_mut().find(|t| t.path == entry.path) {
                        e.title = st.name.clone();
                        e.artist = st.artists.clone();
                        e.album = st.album.clone().unwrap_or_default();
                        e.duration = st.duration_ms.map(|ms| ms as f64 / 1000.0).unwrap_or(0.0);
                    }
                }
                drop(state);
                w
            };
            if was_empty {
                let first = {
                    let read = inner.state.read().await;
                    read.queue.first().map(|t| t.path.clone())
                };
                if let Some(uri) = first {
                    Cmd::play(inner, &uri, 0.0, false).await?;
                }
            }
            Daemon::push_queue_state(inner).await;
            Daemon::save_state(inner);
            return Ok(DaemonRes::Ok);
        }

        // Non-Premium: resolve and start the first track, then lazily resolve
        // the rest so the queue fills as each download completes.
        let first_track = tracks[order[0]].clone();
        let query = if first_track.artists.is_empty() {
            first_track.name.clone()
        } else {
            format!("{} - {}", first_track.artists, first_track.name)
        };
        let path = match spotify_yt_fallback(inner, &format!("spotify-{playlist_id}-first"), &query)
            .await
        {
            Ok(path) => path,
            Err(message) => {
                return Ok(DaemonRes::Error {
                    message: message.to_string(),
                });
            }
        };
        {
            let mut state = inner.state.write().await;
            state.fallback_disabled = false;
            let added = queue::add(&mut state, &path, None);
            if let Some(entry) = state.queue.iter_mut().rev().find(|t| t.path == added.path) {
                entry.title = first_track.name.clone();
                entry.artist = first_track.artists.clone();
                entry.album = first_track.album.clone().unwrap_or_default();
            }
        }
        let _ = Cmd::play(inner, &path, 0.0, false).await;

        let inner2 = Arc::clone(inner);
        let playlist_id = playlist_id.to_string();
        let rest: Vec<SpotifyTrack> = order[1..]
            .iter()
            .filter_map(|&i| tracks.get(i).cloned())
            .collect();
        tokio::spawn(async move {
            let mut position = {
                let state = inner2.state.read().await;
                (state.queue.len() + state.default_list.len()) as u64
            };
            for st in rest {
                let query = if st.artists.is_empty() {
                    st.name.clone()
                } else {
                    format!("{} - {}", st.artists, st.name)
                };
                let cache_key = format!("spotify-{playlist_id}-{position}");
                match spotify_yt_fallback(&inner2, &cache_key, &query).await {
                    Ok(path) => {
                        let mut state = inner2.state.write().await;
                        state.fallback_disabled = false;
                        let added = queue::add(&mut state, &path, Some(position));
                        if let Some(entry) =
                            state.queue.iter_mut().rev().find(|t| t.path == added.path)
                        {
                            entry.title = st.name.clone();
                            entry.artist = st.artists.clone();
                            entry.album = st.album.clone().unwrap_or_default();
                        }
                        drop(state);
                        position += 1;
                        Daemon::push_queue_state(&inner2).await;
                        Daemon::save_state(&inner2);
                    }
                    Err(e) => warn!("spotify play-all: failed to resolve `{query}`: {e}"),
                }
            }
        });

        Daemon::push_queue_state(inner).await;
        Daemon::save_state(inner);
        Ok(DaemonRes::Ok)
    }

    /// Fetch the raw bytes of a Spotify album-cover URL as base64 for the
    /// search picker preview. Persisted in the cover cache so browsing a
    /// playlist warms disk instead of re-downloading per visit.
    pub async fn track_image(inner: &DaemonInner, image_url: &str) -> Result<DaemonRes, CoreError> {
        // Clone the client before touching the cover cache: the cache guard
        // must never be alive while the Spotify manager is locked, or this and
        // `Cover::artist` acquire the two in opposite orders and deadlock.
        let client = match linked(inner).await {
            Ok(client) => client,
            Err(res) => return Ok(*res),
        };
        let cache = inner.cover_cache().await;
        let data = match cache.as_ref() {
            Some(cc) => cc
                .get_url(image_url, || image_at(&client, image_url))
                .await
                .map(|cd| cd.data),
            None => image_at(&client, image_url).await,
        };
        let data = data.map(|bytes| base64::engine::general_purpose::STANDARD.encode(&bytes));
        Ok(DaemonRes::SpotifyImageRes { data })
    }
}

#[cfg(test)]
mod tests {
    use super::is_playable;

    /// A track id is 22 base62 characters; `to_id` refuses anything else, so
    /// the positive cases need a real one.
    const ID: &str = "4cOdK2wGLETKBW3PvgPWqT";

    #[test]
    fn plain_track_uri_is_playable() {
        assert!(is_playable(&format!("spotify:track:{ID}")));
        assert!(is_playable(&format!("spotify:episode:{ID}")));
    }

    #[test]
    fn doubled_prefix_is_rejected() {
        // `SpotifyUri::from_uri` accepts this and returns the inner
        // `spotify:track:<id>` as the id, so without the part-count guard the
        // doubled form passed every check and only failed once librespot was
        // already streaming.
        let doubled = format!("spotify:track:spotify:track:{ID}");
        assert!(!is_playable(&doubled));
        assert!(!is_playable(&format!("spotify:track:{doubled}")));
    }

    #[test]
    fn non_playable_shapes_are_rejected() {
        assert!(!is_playable(""));
        assert!(!is_playable("spotify:"));
        assert!(!is_playable("spotify:track:"));
        // Wrong id length: `to_id` refuses it.
        assert!(!is_playable("spotify:track:1K2saeIy8gAb73"));
        assert!(!is_playable(&format!(
            "https://open.spotify.com/track/{ID}"
        )));
        // `spotify:user:<id>:playlist:<id>` is four parts and is not playable.
        assert!(!is_playable("spotify:user:owner:playlist:playlist"));
    }
}
