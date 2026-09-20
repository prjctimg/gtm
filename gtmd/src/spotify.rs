// Copyright (c) 2026
// Author: prjctimg <prjctimg@outlook.com>
// Spotify Web API: token persistence, account link, and playlist sync
//
// This is free software released under the GPL-3.0 license.

use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::sync::Arc;

use chrono::{Duration, Utc};
use futures::StreamExt;
use rspotify::AuthCodePkceSpotify;
use rspotify::clients::{BaseClient, OAuthClient};
use rspotify::model::{
    AdditionalType, AlbumId, AlbumType, ArtistId, PlayableItem, SearchType, Token,
};
use rspotify::{CallbackError, Config, Credentials, OAuth, TokenCallback};
use tracing::{debug, info, warn};

use gtm::shared::secret::{
    SPOTIFY_CLIENT_ID, SPOTIFY_TOKEN_KEY, delete_secret, get_secret, set_secret,
};
use gtm::shared::spotify::{
    LIBRESPOT_CLIENT_ID, SpotifyPlaylist, SpotifySearchKind, SpotifyStatus, SpotifyTrack,
};

const TOKEN_FILE: &str = "spotify.json";
const TOKEN_ACCESS_PERMS: u32 = 0o600;

/// Owns the Spotify Web API client, its token file, and the playlist cache.
///
/// The access token is stored as `spotify.json` inside the daemon config
/// directory with 0600 permissions and mirrored into the OS keychain. The
/// client is built with the stored client ID and a full token so that
/// rspotify's automatic reauthentication can refresh the access token once it
/// expires, keeping the link alive without a manual re-login.
pub struct SpotifyManager {
    config_dir: PathBuf,
    client: Option<AuthCodePkceSpotify>,
    user: Option<String>,
    /// Whether the linked account has a Premium subscription.
    premium: bool,
    /// Whether the Spotify device was playing on the last playback refresh.
    playing: bool,
    /// Name of the active playback device, if known.
    device: Option<String>,
    playlists: Vec<SpotifyPlaylist>,
    error: Option<String>,
}

impl SpotifyManager {
    pub fn new(config_dir: PathBuf) -> Self {
        Self {
            config_dir,
            client: None,
            user: None,
            premium: false,
            playing: false,
            device: None,
            playlists: Vec::new(),
            error: None,
        }
    }

    /// Absolute path of the token cache file.
    pub fn token_path(&self) -> PathBuf {
        self.config_dir.join(TOKEN_FILE)
    }

    /// True if a token file exists on disk (regardless of load status).
    pub fn has_token_file(&self) -> bool {
        self.token_path().exists()
    }

    /// True if a usable client is currently set up.
    pub fn linked(&self) -> bool {
        self.client.is_some()
    }

    /// Read the token file and build a usable client WITHOUT any network I/O.
    /// Playlist sync and the playback probe run in the daemon's background
    /// tasks so startup (and the OAuth picker) never block on the network
    /// while holding the manager mutex. `linked()` becomes true on return.
    pub async fn load(&mut self) -> Result<(), String> {
        let raw = tokio::fs::read_to_string(self.token_path())
            .await
            .map_err(|e| format!("read token file: {e}"))?;
        let token = parse_token(&raw)?;
        self.set_client(token).await
    }

    /// Accept a token (plain access token or full Token JSON), persist it with
    /// 0600 permissions, then link and sync.
    pub async fn set_token(&mut self, raw: &str) -> Result<(), String> {
        let token = parse_token(raw)?;
        self.save_token(&token)?;
        // Mirror the token into the OS keychain so it survives the file-based
        // token being cleared and can be restored without a re-login.
        set_secret(SPOTIFY_TOKEN_KEY, raw);
        self.init_client(token).await
    }

    /// Remove the token file and reset all in-memory state.
    pub fn clear(&mut self) {
        self.client = None;
        self.user = None;
        self.premium = false;
        self.playing = false;
        self.device = None;
        self.playlists.clear();
        self.error = None;
        match std::fs::remove_file(self.token_path()) {
            Ok(()) => info!("removed spotify token file"),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => warn!("failed to remove spotify token file: {e}"),
        }
        // Drop any keychain-stored credentials too.
        delete_secret(SPOTIFY_TOKEN_KEY);
        delete_secret(SPOTIFY_CLIENT_ID);
    }

    /// Snapshot of the current link state for the Settings UI.
    pub fn status(&self) -> SpotifyStatus {
        let tracks = self.playlists.iter().map(|p| p.tracks.len()).sum();
        SpotifyStatus {
            linked: self.linked(),
            user: self.user.clone(),
            premium: self.premium,
            playing: self.playing,
            device: self.device.clone(),
            playlists: self.playlists.len(),
            tracks,
            error: self.error.clone(),
        }
    }

    /// Record a link error for the Settings UI (e.g. a failed OAuth flow).
    pub fn set_error(&mut self, err: String) {
        self.error = Some(err);
    }

    /// Current OAuth access token, if a client is linked. Refreshes an expired
    /// token first so librespot never connects with a stale credential;
    /// bounded so a stalled refresh fails fast instead of blocking the caller.
    pub async fn access_token(&self) -> Option<String> {
        let client = self.client.clone()?;
        let _ =
            tokio::time::timeout(std::time::Duration::from_secs(10), client.auto_reauth()).await;
        let arc = client.get_token();
        let guard = arc.lock().await.ok()?;
        let token: &rspotify::Token = (*guard).as_ref()?;
        Some(token.access_token.clone())
    }

    /// Whether native librespot streaming is possible: linked account with
    /// an access token and a Premium subscription.
    pub async fn can_stream(&self) -> bool {
        self.access_token().await.is_some() && self.premium
    }

    /// Whether the linked account is a Premium subscriber (probed via the
    /// playback endpoint).
    pub fn is_premium(&self) -> bool {
        self.premium
    }

    /// The cached playlist list (playlists keep their tracks embedded).
    pub fn playlists(&self) -> Vec<SpotifyPlaylist> {
        self.playlists.clone()
    }

    /// Cached tracks of a single playlist, if it has been synced.
    pub fn playlist_tracks(&self, id: &str) -> Option<Vec<SpotifyTrack>> {
        self.playlists
            .iter()
            .find(|p| p.id == id)
            .map(|p| p.tracks.clone())
    }

    /// Look up duration in seconds for a Spotify track URI across cached playlists.
    pub fn find_track_duration(&self, uri: &str) -> Option<f64> {
        for p in &self.playlists {
            for t in &p.tracks {
                if t.uri.as_deref() == Some(uri) {
                    if let Some(ms) = t.duration_ms {
                        return Some(ms as f64 / 1000.0);
                    }
                }
            }
        }
        None
    }

    /// Poll the Web API for the current playback device and playing state.
    ///
    /// `/me/player` requires a Premium account: a `403 PREMIUM_REQUIRED`
    /// response sets `premium` to false (disabling the Settings control rows),
    /// while a successful response implies playback control is available.
    /// Other failures leave the cached fields untouched.
    pub async fn refresh_playback(&mut self) {
        let Some(client) = self.client.as_ref() else {
            self.playing = false;
            self.device = None;
            return;
        };
        match client
            .current_playback(None, None::<&[AdditionalType]>)
            .await
        {
            Ok(Some(ctx)) => {
                self.playing = ctx.is_playing;
                self.device = Some(ctx.device.name.clone());
                self.premium = true;
            }
            Ok(None) => {
                self.playing = false;
                self.device = None;
                self.premium = true;
            }
            Err(e) => {
                let msg = e.to_string();
                if msg.contains("403") && msg.to_lowercase().contains("premium") {
                    debug!("spotify playback control unavailable (premium required)");
                    self.premium = false;
                } else {
                    debug!("spotify playback refresh failed: {e}");
                }
            }
        }
    }

    /// Toggle play/pause on the active Spotify device.
    ///
    /// Requires Premium; a `403 PREMIUM_REQUIRED` from the playback endpoint
    /// is surfaced as an error and clears the `premium` flag so the UI can
    /// disable the control rows.
    pub async fn play_pause(&mut self) -> Result<(), String> {
        if self.client.is_none() {
            return Err("spotify not linked".to_string());
        }
        self.refresh_playback().await;
        let device = self.device.clone();
        let client = self
            .client
            .as_ref()
            .ok_or_else(|| "spotify not linked".to_string())?;
        let res = if self.playing {
            client.pause_playback(device.as_deref()).await
        } else {
            client.resume_playback(device.as_deref(), None).await
        };
        res.map_err(|e| format!("{e}"))?;
        self.refresh_playback().await;
        Ok(())
    }

    /// Refresh the account profile and every playlist from the Web API.
    pub async fn sync(&mut self) -> Result<(), String> {
        let client = self
            .sync_client()
            .ok_or_else(|| "spotify not linked".to_string())?;
        let (user, playlists) = Self::run_sync(client).await?;
        self.commit_sync(user, playlists);
        // Probe `/me/player` so the Premium flag is established at sync time
        // and native streaming is unlocked without the user visiting Settings.
        self.refresh_playback().await;
        Ok(())
    }

    /// Clone the underlying Web API client so a sync can paginate without
    /// holding the manager mutex. `None` when not linked.
    pub fn sync_client(&self) -> Option<AuthCodePkceSpotify> {
        self.client.clone()
    }

    /// Paginate the full account profile, every playlist, and every playlist's
    /// tracks on a cloned client so the caller never holds the manager mutex
    /// across the network pass. Returns the snapshot to commit via
    /// [`Self::commit_sync`].
    pub async fn run_sync(
        client: AuthCodePkceSpotify,
    ) -> Result<(Option<String>, Vec<SpotifyPlaylist>), String> {
        let me = client.me().await.map_err(|e| format!("me: {e}"))?;
        let user = me.display_name.or_else(|| Some(me.id.as_ref().to_string()));
        // NOTE: rspotify's `me().product` was removed upstream (Spotify no
        // longer exposes the plan); Premium is instead probed via the
        // playback endpoint in `refresh_playback()`.

        // Collect the playlist metadata first (paginator borrows the client),
        // then fetch each playlist's tracks in a second pass.
        let mut metas = Vec::new();
        let mut paginator = client.current_user_playlists();
        while let Some(item) = paginator.next().await {
            match item {
                Ok(pl) => metas.push(pl),
                Err(e) => {
                    // A transient failure on one page must not abort the whole
                    // sync (which previously cleared the entire cache): skip the
                    // remaining pages gracefully instead.
                    warn!("spotify playlists: {e} — skipping remainder");
                    break;
                }
            }
        }
        debug!("fetched {} spotify playlists for {:?}", metas.len(), user);

        let mut playlists = Vec::new();
        let saved = Self::fetch_saved_tracks(&client).await;
        if !saved.is_empty() {
            playlists.push(SpotifyPlaylist {
                id: "liked-songs".to_string(),
                name: "Liked Songs".to_string(),
                owner: user.clone().unwrap_or_default(),
                tracks: saved,
            });
        }

        for meta in &metas {
            // Per-playlist failures are already tolerated inside
            // `fetch_playlist_tracks`; an unparseable playlist only logs.
            let tracks = Self::fetch_playlist_tracks(&client, meta.id.clone()).await;
            playlists.push(SpotifyPlaylist {
                id: meta.id.as_ref().to_string(),
                name: meta.name.clone(),
                owner: meta.owner.display_name.clone().unwrap_or_default(),
                tracks,
            });
        }
        Ok((user, playlists))
    }

    /// Swap a completed sync snapshot into the manager. `status()` and
    /// `playlists()` only ever contend for this brief swap, never for the
    /// minutes of network pagination that preceded it.
    pub fn commit_sync(&mut self, user: Option<String>, playlists: Vec<SpotifyPlaylist>) {
        self.error = None;
        self.user = user;
        self.playlists = playlists;
    }

    async fn fetch_saved_tracks(client: &AuthCodePkceSpotify) -> Vec<SpotifyTrack> {
        let mut saved = Vec::new();
        let mut stream = client.current_user_saved_tracks(None);
        while let Some(item) = stream.next().await {
            match item {
                Ok(item) => {
                    if let Some(mut track) = track_from_playable(&PlayableItem::Track(item.track)) {
                        track.index = saved.len();
                        saved.push(track);
                    }
                }
                Err(e) => {
                    warn!("spotify saved tracks: {e}");
                    break;
                }
            }
        }
        saved
    }

    async fn fetch_playlist_tracks(
        client: &AuthCodePkceSpotify,
        playlist_id: rspotify::model::PlaylistId<'static>,
    ) -> Vec<SpotifyTrack> {
        let mut tracks = Vec::new();
        let mut items = client.playlist_items(playlist_id, None, None);
        while let Some(item) = items.next().await {
            match item {
                Ok(item) => {
                    if let Some(playable) = item.item.as_ref()
                        && let Some(mut track) = track_from_playable(playable)
                    {
                        track.index = tracks.len();
                        tracks.push(track);
                    }
                }
                Err(e) => warn!("spotify playlist item: {e}"),
            }
        }
        tracks
    }

    /// Accept a token, persist it, then build a usable client WITHOUT syncing
    /// playlists. `linked()` becomes true immediately so the TUI can close its
    /// OAuth picker and start loading playlists while the sync runs in the
    /// background. Returns once the client is ready.
    ///
    /// The eager profile/playback probes below are bounded (10s each) so a
    /// stalled network delays the picker-close event by seconds, never
    /// indefinitely; the background sync refreshes both when it finishes.
    pub async fn link(&mut self, raw: &str) -> Result<(), String> {
        let token = parse_token(raw)?;
        self.save_token(&token)?;
        // Mirror the token into the OS keychain so it survives the file-based
        // token being cleared and can be restored without a re-login.
        set_secret(SPOTIFY_TOKEN_KEY, raw);
        self.set_client(token).await?;
        // Populate the display name eagerly so the TUI can greet the user as
        // soon as the picker closes; the playlist sync continues in the
        // background and refreshes the cache when it finishes.
        if let Some(client) = self.client.clone()
            && let Ok(Ok(me)) =
                tokio::time::timeout(std::time::Duration::from_secs(10), client.me()).await
        {
            self.user = me.display_name.or_else(|| Some(me.id.as_ref().to_string()));
        }
        // Probe `/me/player` right after linking so `premium` is set before any
        // play command arrives (playlists sync purely via the Web API and never
        // implied Premium).
        let _ =
            tokio::time::timeout(std::time::Duration::from_secs(10), self.refresh_playback()).await;
        Ok(())
    }

    /// Build a usable rspotify client from a token (persist-free). Does not
    /// touch the playlist cache or call the network; `linked()` becomes true
    /// once this returns.
    async fn set_client(&mut self, token: Token) -> Result<(), String> {
        let refreshable = token.refresh_token.is_some();
        let client_id = get_secret(SPOTIFY_CLIENT_ID).unwrap_or_default();
        // Fall back to librespot's public desktop client id when the user
        // linked with a plain pasted access token (which never stores a
        // client id). `Credentials::default()` is a dead end: rspotify's
        // bundled demo id cannot refresh, so such tokens silently expire and
        // every later Web API call fails with a 401.
        let creds = if client_id.is_empty() {
            Credentials::new_pkce(LIBRESPOT_CLIENT_ID)
        } else {
            Credentials::new_pkce(&client_id)
        };
        // Persist a refreshed token back to disk with 0600 permissions so a
        // renewed access token survives a daemon restart instead of reverting
        // to the stale one. rspotify invokes this callback after every
        // successful refresh.
        let token_path = self.token_path();
        let token_callback = TokenCallback(Box::new(move |refreshed: Token| {
            let dir = token_path.parent().ok_or_else(|| {
                CallbackError::CustomizedError("token path has no parent".to_string())
            })?;
            std::fs::create_dir_all(dir)
                .map_err(|e| CallbackError::CustomizedError(format!("create dir: {e}")))?;
            let json = serde_json::to_string(&refreshed)
                .map_err(|e| CallbackError::CustomizedError(format!("serialize: {e}")))?;
            std::fs::write(&token_path, json)
                .map_err(|e| CallbackError::CustomizedError(format!("write: {e}")))?;
            std::fs::set_permissions(
                &token_path,
                std::fs::Permissions::from_mode(TOKEN_ACCESS_PERMS),
            )
            .map_err(|e| CallbackError::CustomizedError(format!("chmod: {e}")))?;
            Ok::<(), CallbackError>(())
        }));
        let config = Config {
            token_cached: false,
            token_refreshing: refreshable,
            token_callback_fn: Arc::new(Some(token_callback)),
            ..Default::default()
        };
        let oauth = OAuth::default();
        self.client = Some(AuthCodePkceSpotify::from_token_with_config(
            token, creds, oauth, config,
        ));
        self.error = None;
        Ok(())
    }

    async fn init_client(&mut self, token: Token) -> Result<(), String> {
        let refreshable = token.refresh_token.is_some();
        self.set_client(token).await?;
        match self.sync().await {
            Ok(()) => {
                info!(
                    "linked spotify as {:?} ({} playlists, auto-refresh: {refreshable})",
                    self.user,
                    self.playlists.len()
                );
                Ok(())
            }
            Err(e) => {
                self.error = Some(e.clone());
                // Keep the linked client: a transient network failure is not an
                // unlink. Dropping it here removed the credentials, made every
                // follow-up call fail with "spotify not linked", and left the
                // account linked in name only until a full OAuth re-link.
                Err(e)
            }
        }
    }

    fn save_token(&self, token: &Token) -> Result<(), String> {
        let dir = &self.config_dir;
        std::fs::create_dir_all(dir).map_err(|e| format!("create config dir: {e}"))?;
        let path = self.token_path();
        let json = serde_json::to_string(token).map_err(|e| format!("serialize token: {e}"))?;
        std::fs::write(&path, json).map_err(|e| format!("write token file: {e}"))?;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(TOKEN_ACCESS_PERMS))
            .map_err(|e| format!("set token permissions: {e}"))?;
        Ok(())
    }

    pub async fn search(&self, query: &str, limit: u32) -> Vec<SpotifyTrack> {
        let Some(client) = self.client.as_ref() else {
            return Vec::new();
        };
        let mut tracks: Vec<SpotifyTrack> = Vec::new();

        // Track results first (the most useful).
        if let Ok(rspotify::model::SearchResult::Tracks(page)) = client
            .search(query, SearchType::Track, None, None, Some(limit), None)
            .await
        {
            tracks.extend(page.items.iter().enumerate().map(|(i, t)| {
                SpotifyTrack {
                    index: i,
                    name: t.name.clone(),
                    artists: t
                        .artists
                        .iter()
                        .map(|a| a.name.clone())
                        .collect::<Vec<_>>()
                        .join(", "),
                    album: Some(t.album.name.clone()),
                    duration_ms: Some(t.duration.num_milliseconds().max(0) as u64),
                    uri: t.id.as_ref().map(|id| format!("spotify:track:{id}")),
                    image_url: pick_largest_image(&t.album.images),
                    kind: None,
                }
            }));
        }

        let mut idx = tracks.len();
        let album_limit = (limit / 3).max(5);
        let artist_limit = (limit / 4).max(4);

        // Album results.
        if let Ok(rspotify::model::SearchResult::Albums(page)) = client
            .search(
                query,
                SearchType::Album,
                None,
                None,
                Some(album_limit),
                None,
            )
            .await
        {
            for a in &page.items {
                tracks.push(SpotifyTrack {
                    index: idx,
                    name: a.name.clone(),
                    artists: a
                        .artists
                        .iter()
                        .map(|a| a.name.clone())
                        .collect::<Vec<_>>()
                        .join(", "),
                    album: Some(a.name.clone()),
                    duration_ms: None,
                    uri: a.id.as_ref().map(|id| format!("spotify:album:{id}")),
                    image_url: pick_largest_image(&a.images),
                    kind: Some(SpotifySearchKind::Album),
                });
                idx += 1;
            }
        }

        // Artist results.
        if let Ok(rspotify::model::SearchResult::Artists(page)) = client
            .search(
                query,
                SearchType::Artist,
                None,
                None,
                Some(artist_limit),
                None,
            )
            .await
        {
            for a in &page.items {
                tracks.push(SpotifyTrack {
                    index: idx,
                    name: a.name.clone(),
                    artists: String::new(),
                    album: None,
                    duration_ms: None,
                    uri: Some(format!("spotify:artist:{}", a.id)),
                    image_url: pick_largest_image(&a.images),
                    kind: Some(SpotifySearchKind::Artist),
                });
                idx += 1;
            }
        }

        tracks
    }

    /// Resolve a web-search album result to its track list.
    pub async fn album_tracks(&self, uri: &str) -> Result<Vec<SpotifyTrack>, String> {
        let Some(client) = self.client.as_ref() else {
            return Err("spotify not linked".into());
        };
        let album_id = AlbumId::from_uri(uri).map_err(|e| format!("bad album uri: {e}"))?;
        let page = client
            .album_track_manual(album_id, None, Some(50), Some(0))
            .await
            .map_err(|e| format!("album tracks: {e}"))?;
        let mut tracks = Vec::new();
        for (i, t) in page.items.into_iter().enumerate() {
            tracks.push(SpotifyTrack {
                index: i,
                name: t.name.clone(),
                artists: t
                    .artists
                    .iter()
                    .map(|a| a.name.clone())
                    .collect::<Vec<_>>()
                    .join(", "),
                album: t.album.as_ref().map(|a| a.name.clone()),
                duration_ms: Some(t.duration.num_milliseconds().max(0) as u64),
                uri: t.id.as_ref().map(|id| format!("spotify:track:{id}")),
                image_url: t.album.as_ref().and_then(|a| pick_largest_image(&a.images)),
                kind: Some(SpotifySearchKind::Track),
            });
        }
        Ok(tracks)
    }

    /// Resolve an artist URI to their top tracks via their most recent albums.
    /// Spotify removed the dedicated top-tracks endpoint, so we collect tracks
    /// from the artist's newest albums/singles instead.
    pub async fn artist_top_tracks(&self, uri: &str) -> Result<Vec<SpotifyTrack>, String> {
        let Some(client) = self.client.as_ref() else {
            return Err("spotify not linked".into());
        };
        let artist_id = ArtistId::from_uri(uri).map_err(|e| format!("bad artist uri: {e}"))?;
        let page = client
            .artist_albums_manual(
                artist_id,
                [AlbumType::Album, AlbumType::Single],
                None,
                Some(20),
                Some(0),
            )
            .await
            .map_err(|e| format!("artist albums: {e}"))?;

        let mut tracks: Vec<SpotifyTrack> = Vec::new();
        let mut seen = std::collections::HashSet::new();
        let target = 50u32;
        let mut albums_fetched = 0u32;

        for album in page.items {
            if tracks.len() as u32 >= target || albums_fetched >= 4 {
                break;
            }
            let Some(album_id) = album.id else {
                continue;
            };
            let page = match client
                .album_track_manual(album_id, None, Some(50), Some(0))
                .await
            {
                Ok(p) => p,
                Err(_) => continue,
            };
            albums_fetched += 1;
            for t in page.items {
                let Some(track_id) = t.id.as_ref() else {
                    continue;
                };
                let track_uri = format!("spotify:track:{track_id}");
                if !seen.insert(track_uri.clone()) {
                    continue;
                }
                tracks.push(SpotifyTrack {
                    index: tracks.len(),
                    name: t.name.clone(),
                    artists: t
                        .artists
                        .iter()
                        .map(|a| a.name.clone())
                        .collect::<Vec<_>>()
                        .join(", "),
                    album: t
                        .album
                        .as_ref()
                        .map(|a| a.name.clone())
                        .or(Some(album.name.clone())),
                    duration_ms: Some(t.duration.num_milliseconds().max(0) as u64),
                    uri: Some(track_uri),
                    image_url: t.album.as_ref().and_then(|a| pick_largest_image(&a.images)),
                    kind: Some(SpotifySearchKind::Track),
                });
                if tracks.len() as u32 >= target {
                    break;
                }
            }
        }
        Ok(tracks)
    }

    /// Fetch the largest album-cover image bytes for an artist + album via the
    /// Web API, when the account is linked. `None` when not linked or no hit.
    pub async fn album_cover(&self, artist: &str, album: &str) -> Option<Vec<u8>> {
        let client = self.client.as_ref()?;
        let q = format!("album:\"{}\" artist:\"{}\"", album.trim(), artist.trim());
        let albums = match client
            .search(&q, SearchType::Album, None, None, Some(3), None)
            .await
        {
            Ok(rspotify::model::SearchResult::Albums(page)) => page.items,
            _ => return None,
        };
        let images = albums
            .into_iter()
            .flat_map(|a| a.images)
            .collect::<Vec<_>>();
        let url = pick_largest_image(&images)?;
        self.download_image(&url).await
    }

    /// Fetch the largest artist portrait image bytes via the Web API, when the
    /// account is linked. `None` when not linked or no hit.
    pub async fn artist_image(&self, artist: &str) -> Option<Vec<u8>> {
        let client = self.client.as_ref()?;
        let q = format!("artist:\"{}\"", artist.trim());
        let artists = match client
            .search(&q, SearchType::Artist, None, None, Some(3), None)
            .await
        {
            Ok(rspotify::model::SearchResult::Artists(page)) => page.items,
            _ => return None,
        };
        let images = artists
            .into_iter()
            .flat_map(|a| a.images)
            .collect::<Vec<_>>();
        let url = pick_largest_image(&images)?;
        self.download_image(&url).await
    }

    async fn download_image(&self, url: &str) -> Option<Vec<u8>> {
        let token = self.access_token().await?;
        let resp = reqwest::Client::new()
            .get(url)
            .bearer_auth(&token)
            .send()
            .await
            .ok()?;
        if !resp.status().is_success() {
            return None;
        }
        let bytes = resp.bytes().await.ok()?;
        (!bytes.is_empty()).then_some(bytes.to_vec())
    }

    /// Fetch the raw bytes of an album-cover image located at `image_url`
    /// (as exposed via `SpotifyTrack::image_url`), without an extra search.
    pub async fn image_by_url(&self, image_url: &str) -> Option<Vec<u8>> {
        let image_url = image_url.trim();
        if image_url.is_empty() {
            return None;
        }
        self.download_image(image_url).await
    }
}

/// Pick the largest (first-sorted-by-area) image URL from a set of Spotify
/// image variants.
pub fn pick_largest_image(images: &[rspotify::model::Image]) -> Option<String> {
    images
        .iter()
        .max_by_key(|img| {
            let w = img.width.unwrap_or(0);
            let h = img.height.unwrap_or(0);
            w.saturating_mul(h)
        })
        .map(|img| img.url.clone())
}

/// Convert an rspotify playable item into our IPC-friendly track shape.
pub fn track_from_playable(item: &PlayableItem) -> Option<SpotifyTrack> {
    match item {
        PlayableItem::Track(t) => Some(SpotifyTrack {
            index: 0,
            name: t.name.clone(),
            artists: t
                .artists
                .iter()
                .map(|a| a.name.clone())
                .collect::<Vec<_>>()
                .join(", "),
            album: Some(t.album.name.clone()),
            duration_ms: Some(t.duration.num_milliseconds().max(0) as u64),
            uri: t.id.as_ref().map(|id| format!("spotify:track:{id}")),
            image_url: pick_largest_image(&t.album.images),
            kind: None,
        }),
        PlayableItem::Episode(_) | PlayableItem::Unknown(_) => None,
    }
}

/// Accept either a full rspotify `Token` JSON object or a bare access token
/// string. Full-token JSON is preferred so scopes/refresh info is preserved.
/// The expiry is derived from `expires_in` whenever the token file lacks an
/// explicit `expires_at` so expiry checks (and auto-refresh) behave correctly.
fn parse_token(raw: &str) -> Result<Token, String> {
    let raw = raw.trim();
    if raw.is_empty() {
        return Err("empty token".into());
    }
    let mut token = if let Ok(token) = serde_json::from_str::<Token>(raw)
        && !token.access_token.is_empty()
    {
        token
    } else if !raw.contains('{') {
        Token {
            access_token: raw.to_string(),
            expires_in: Duration::try_seconds(3600).ok_or("invalid default expiry")?,
            expires_at: None,
            refresh_token: None,
            scopes: Default::default(),
        }
    } else {
        return Err("could not parse spotify token".into());
    };
    if token.expires_at.is_none() {
        token.expires_at = Utc::now().checked_add_signed(token.expires_in);
    }
    Ok(token)
}

#[cfg(test)]
mod tests {
    use super::{TOKEN_ACCESS_PERMS, parse_token};

    #[test]
    fn token_plain() {
        let tok =
            parse_token("BQC8xYt0aBcDeFgHiJkLmNoPqRsTuVwXyZ").expect("plain token should parse");
        assert_eq!(tok.access_token, "BQC8xYt0aBcDeFgHiJkLmNoPqRsTuVwXyZ");
        assert!(tok.refresh_token.is_none());
    }

    #[test]
    fn token_full_json() {
        let json = r#"{"access_token":"abc","expires_in":3600,"scopes":""}"#;
        let tok = parse_token(json).expect("full token json should parse");
        assert_eq!(tok.access_token, "abc");
        assert_eq!(tok.expires_in.num_seconds(), 3600);
    }

    #[test]
    fn token_rejects_empty() {
        assert!(parse_token("").is_err());
        assert!(parse_token("   ").is_err());
    }

    #[test]
    fn token_owner_only() {
        assert_eq!(TOKEN_ACCESS_PERMS, 0o600);
    }
}
