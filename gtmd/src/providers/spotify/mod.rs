// Copyright (c) 2026
// Author: prjctimg <prjctimg@outlook.com>
// Spotify Web API: token persistence, account link, and playlist sync
//
// This is free software released under the GPL-3.0 license.

//! Everything Spotify: the Web API manager and its token store, the catalog
//! calls, artwork lookups, the librespot streaming bridge, and the IPC command
//! handlers. Each concern is its own file so a change to one does not have to
//! be read against the others.

pub mod api;
pub mod cmd;
pub mod cover;
pub mod oauth;
pub mod stream;
pub mod ytfb;

use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::sync::Arc;

use chrono::{Duration, Utc};
use futures::StreamExt;
use rspotify::AuthCodePkceSpotify;
use rspotify::clients::{BaseClient, OAuthClient};
use rspotify::model::{AdditionalType, PlayableItem, PlaylistId, RepeatState, Token};
use rspotify::{CallbackError, Config, Credentials, OAuth, TokenCallback};
use tracing::{debug, info, warn};

use gtm::shared::secret::{
    SPOTIFY_CLIENT_ID, SPOTIFY_TOKEN_KEY, delete_secret, get_secret, set_secret,
};
use gtm::shared::spotify::{LIBRESPOT_CLIENT_ID, SpotifyPlaylist, SpotifyStatus, SpotifyTrack};

use api::track_from_playable;

const TOKEN_FILE: &str = "spotify.json";
/// Client id of the Spotify app the user authorised with, kept beside the
/// token so a refresh keeps using the same app. Without it the daemon falls
/// back to librespot's public client, whose refresh tokens do not match and
/// therefore fail on the first renewal after a restart.
const CLIENT_ID_FILE: &str = "spotify_client_id";
const TOKEN_ACCESS_PERMS: u32 = 0o600;
/// OAuth scope required for librespot native playback. Tokens issued before it
/// was requested keep working for the Web API but cannot stream audio.
const SCOPE_STREAMING: &str = "streaming";

/// Display name for an identifier that has no human label of its own.
///
/// The synthetic Liked Songs entry and bare `spotify:` URIs both reach the UI
/// without a title, and the queue's file-stem fallback renders the URI
/// verbatim. Formatting them in one place keeps a raw id from ever reaching a
/// playlist header or the now-playing widget.
pub fn pretty_id(id: &str) -> String {
    match id {
        "liked-songs" => "Liked Songs".to_string(),
        _ if id.starts_with("spotify:") => "Spotify Track".to_string(),
        _ => id.replace(['-', '_'], " "),
    }
}

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
    /// Spotify device id, which the `/me/player` control endpoints require
    /// (distinct from the display name).
    device_id: Option<String>,
    /// Shuffle/repeat state of the active device, mirrored from `/me/player`.
    shuffle: bool,
    repeat: String,
    playlists: Vec<SpotifyPlaylist>,
    /// Scopes granted by the stored token, snapshotted when the client is built.
    scopes: std::collections::HashSet<String>,
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
            device_id: None,
            shuffle: false,
            repeat: "off".to_string(),
            playlists: Vec::new(),
            scopes: std::collections::HashSet::new(),
            error: None,
        }
    }

    /// Absolute path of the token cache file.
    pub fn token_path(&self) -> PathBuf {
        self.config_dir.join(TOKEN_FILE)
    }

    /// Path of the client-id file, which keeps the config directory a
    /// self-sufficient record of the link.
    fn client_path(&self) -> PathBuf {
        self.config_dir.join(CLIENT_ID_FILE)
    }

    /// The client id to refresh with: the file written at link time, else the
    /// keychain copy, else empty (caller falls back to librespot's app).
    fn stored_client_id(&self) -> String {
        std::fs::read_to_string(self.client_path())
            .ok()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .or_else(|| get_secret(SPOTIFY_CLIENT_ID))
            .unwrap_or_default()
    }

    /// The client id the current access token was minted with.
    ///
    /// Spotify binds an access token to the app that requested it, so the
    /// librespot session has to present the *same* id. Presenting a token from
    /// one app to a session registered as another still connects and then
    /// delivers no audio at all, which is the quietest possible failure: the
    /// track resolves, the clock advances, and the mixer never gets a sample.
    pub fn streaming_client_id(&self) -> String {
        let id = self.stored_client_id();
        if id.is_empty() {
            LIBRESPOT_CLIENT_ID.to_string()
        } else {
            id
        }
    }

    /// Record the authorised client id in both stores. Called when the OAuth
    /// flow starts, so a restart can still refresh with the same app.
    pub fn save_client_id(&self, id: &str) -> Result<(), String> {
        let id = id.trim();
        if id.is_empty() {
            return Ok(());
        }
        std::fs::create_dir_all(&self.config_dir).map_err(|e| format!("create config dir: {e}"))?;
        let path = self.client_path();
        std::fs::write(&path, id).map_err(|e| format!("write client id: {e}"))?;
        set_secret(SPOTIFY_CLIENT_ID, id);
        Ok(())
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
    ///
    /// The keychain copy is used when the file is missing, so a link survives
    /// the config directory being reset or the file being removed by a
    /// partial write: the user is only asked to authorise again when both
    /// copies are gone.
    pub async fn load(&mut self) -> Result<(), String> {
        let raw = match tokio::fs::read_to_string(self.token_path()).await {
            Ok(raw) => raw,
            Err(e) => {
                info!("spotify token file unreadable ({e}); trying the keychain");
                get_secret(SPOTIFY_TOKEN_KEY).ok_or_else(|| format!("read token file: {e}"))?
            }
        };
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
        for path in [self.token_path(), self.client_path()] {
            match std::fs::remove_file(&path) {
                Ok(()) => info!("removed spotify {}", path.display()),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => warn!("failed to remove {}: {e}", path.display()),
            }
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
            device_id: self.device_id.clone(),
            shuffle: self.shuffle,
            repeat: self.repeat.clone(),
            playlists: self.playlists.len(),
            tracks,
            needs_relink: self.needs_relink(),
            error: self.error.clone(),
        }
    }

    /// Record a link error for the Settings UI (e.g. a failed OAuth flow).
    pub fn set_error(&mut self, err: String) {
        self.error = Some(err);
    }

    /// True when the stored token grants `scope`. Scopes can only be widened by
    /// re-linking, so a missing one is reported to the user instead of failing
    /// later inside librespot.
    pub fn has_scope(&self, scope: &str) -> bool {
        self.scopes.contains(scope)
    }

    /// Whether the linked token predates the `streaming` scope and therefore
    /// needs a fresh link before native playback can work.
    pub fn needs_relink(&self) -> bool {
        self.linked() && !self.has_scope(SCOPE_STREAMING)
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
                    return t.duration_ms.map(|ms| ms as f64 / 1000.0);
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
            self.device_id = None;
            return;
        };
        match client
            .current_playback(None, None::<&[AdditionalType]>)
            .await
        {
            Ok(Some(ctx)) => {
                self.playing = ctx.is_playing;
                self.device = Some(ctx.device.name.clone());
                // The control endpoints take a device *id*, not the display
                // name; keeping both lets the UI show a label and the daemon
                // address the right device.
                self.device_id = ctx.device.id.clone();
                self.shuffle = ctx.shuffle_state;
                self.repeat = <&str>::from(ctx.repeat_state).to_string();
                self.premium = true;
            }
            Ok(None) => {
                self.playing = false;
                self.device = None;
                self.device_id = None;
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
        let device = self.device_id.clone();
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

    /// Skip to the next track on the active Spotify device.
    pub async fn next(&mut self) -> Result<(), String> {
        let client = self
            .client
            .as_ref()
            .ok_or_else(|| "spotify not linked".to_string())?;
        let device = self.device_id.clone();
        client
            .next_track(device.as_deref())
            .await
            .map_err(|e| format!("{e}"))?;
        self.refresh_playback().await;
        Ok(())
    }

    /// Skip to the previous track on the active Spotify device.
    pub async fn previous(&mut self) -> Result<(), String> {
        let client = self
            .client
            .as_ref()
            .ok_or_else(|| "spotify not linked".to_string())?;
        let device = self.device_id.clone();
        client
            .previous_track(device.as_deref())
            .await
            .map_err(|e| format!("{e}"))?;
        self.refresh_playback().await;
        Ok(())
    }

    /// Seek the active Spotify device to `pos_secs`.
    pub async fn seek(&mut self, pos_secs: u32) -> Result<(), String> {
        let client = self
            .client
            .as_ref()
            .ok_or_else(|| "spotify not linked".to_string())?;
        let device = self.device_id.clone();
        client
            .seek_track(
                Duration::milliseconds(i64::from(pos_secs) * 1000),
                device.as_deref(),
            )
            .await
            .map_err(|e| format!("{e}"))?;
        self.refresh_playback().await;
        Ok(())
    }

    /// Set the active Spotify device's shuffle mode.
    pub async fn set_shuffle(&mut self, on: bool) -> Result<(), String> {
        let client = self
            .client
            .as_ref()
            .ok_or_else(|| "spotify not linked".to_string())?;
        let device = self.device_id.clone();
        client
            .shuffle(on, device.as_deref())
            .await
            .map_err(|e| format!("{e}"))?;
        self.refresh_playback().await;
        Ok(())
    }

    /// Set the active Spotify device's repeat mode (`off`/`track`/`context`).
    pub async fn set_repeat(&mut self, mode: &str) -> Result<(), String> {
        let state = match mode {
            "track" => RepeatState::Track,
            "context" => RepeatState::Context,
            _ => RepeatState::Off,
        };
        let client = self
            .client
            .as_ref()
            .ok_or_else(|| "spotify not linked".to_string())?;
        let device = self.device_id.clone();
        client
            .repeat(state, device.as_deref())
            .await
            .map_err(|e| format!("{e}"))?;
        self.refresh_playback().await;
        Ok(())
    }

    /// Set the active Spotify device's volume (0-100).
    pub async fn set_volume(&mut self, percent: u8) -> Result<(), String> {
        let client = self
            .client
            .as_ref()
            .ok_or_else(|| "spotify not linked".to_string())?;
        let device = self.device_id.clone();
        client
            .volume(percent.min(100), device.as_deref())
            .await
            .map_err(|e| format!("{e}"))?;
        self.refresh_playback().await;
        Ok(())
    }

    /// Refresh the account profile and every playlist from the Web API.
    pub async fn sync(&mut self) -> Result<(), String> {
        let client = self
            .client()
            .ok_or_else(|| "spotify not linked".to_string())?;
        let (user, playlists) = Self::run_sync(client).await?;
        self.commit_sync(user, playlists);
        // Probe `/me/player` so the Premium flag is established at sync time
        // and native streaming is unlocked without the user visiting Settings.
        self.refresh_playback().await;
        Ok(())
    }

    /// Clone the Web API client so network work can run without the manager
    /// mutex. The clone shares rspotify's token slot, so refreshing through it
    /// updates the token this manager persists. `None` when not linked.
    pub fn client(&self) -> Option<AuthCodePkceSpotify> {
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
        playlist_id: PlaylistId<'static>,
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
        // Scopes are fixed at authorization time and cannot be widened by a
        // refresh, so snapshot them here: the token itself lives behind an
        // async mutex that cannot be inspected synchronously.
        self.scopes = token.scopes.clone();
        // Fall back to librespot's public desktop client id when the user
        // linked with a plain pasted access token (which never stores a
        // client id). `Credentials::default()` is a dead end: rspotify's
        // bundled demo id cannot refresh, so such tokens silently expire and
        // every later Web API call fails with a 401.
        //
        // The fallback is also what `streaming_client_id` hands the librespot
        // session, so both sides always agree on which app the token belongs
        // to.
        let creds = Credentials::new_pkce(&self.streaming_client_id());
        // Persist a refreshed token back to disk with 0600 permissions so a
        // renewed access token survives a daemon restart instead of reverting
        // to the stale one. rspotify invokes this callback after every
        // successful refresh. The keychain copy is refreshed too, otherwise it
        // keeps holding the token from the original link and goes stale
        // exactly when it is needed as the fallback.
        let token_path = self.token_path();
        let token_callback = TokenCallback(Box::new(move |refreshed: Token| {
            let dir = token_path.parent().ok_or_else(|| {
                CallbackError::CustomizedError("token path has no parent".to_string())
            })?;
            std::fs::create_dir_all(dir)
                .map_err(|e| CallbackError::CustomizedError(format!("create dir: {e}")))?;
            let json = serde_json::to_string(&refreshed)
                .map_err(|e| CallbackError::CustomizedError(format!("serialize: {e}")))?;
            std::fs::write(&token_path, &json)
                .map_err(|e| CallbackError::CustomizedError(format!("write: {e}")))?;
            std::fs::set_permissions(
                &token_path,
                std::fs::Permissions::from_mode(TOKEN_ACCESS_PERMS),
            )
            .map_err(|e| CallbackError::CustomizedError(format!("chmod: {e}")))?;
            set_secret(SPOTIFY_TOKEN_KEY, &json);
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
}

/// Current OAuth access token for a cloned client, refreshing an expired one
/// first so librespot never connects with a stale credential; bounded so a
/// stalled refresh fails fast instead of blocking the caller.
///
/// Refresh failures are reported rather than swallowed: a token that could
/// not be refreshed is not a usable credential, and returning it anyway turns
/// every downstream call into a confusing network error.

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
    use super::{SCOPE_STREAMING, TOKEN_ACCESS_PERMS, parse_token};
    use rspotify::model::idtypes::Id;
    use rspotify::model::{AlbumId, ArtistId, TrackId};

    /// `Display` on an rspotify 0.16 id renders its **full URI**, not the bare
    /// id, so `format!("spotify:track:{id}")` produced
    /// `spotify:track:spotify:track:<id>` and every resolved track was
    /// rejected as unplayable. The `Id` accessors are the correct spelling and
    /// are what the request builders use; these pin the difference so a future
    /// rspotify bump cannot quietly reintroduce it.
    #[test]
    fn id_display_is_a_uri_not_a_bare_id() {
        let id = TrackId::from_id("1K2saNWiQjrrgeIy8gAb73").expect("valid track id");
        assert_eq!(id.id(), "1K2saNWiQjrrgeIy8gAb73");
        assert_eq!(id.uri(), "spotify:track:1K2saNWiQjrrgeIy8gAb73");
        // The regression: Display must not be used to build a URI.
        assert_eq!(format!("{id}"), id.uri());
        assert_ne!(format!("spotify:track:{id}"), id.uri());

        let album = AlbumId::from_id("1K2saNWiQjrrgeIy8gAb73").expect("valid album id");
        assert_eq!(album.uri(), format!("spotify:album:{}", album.id()));
        let artist = ArtistId::from_id("1K2saNWiQjrrgeIy8gAb73").expect("valid artist id");
        assert_eq!(artist.uri(), format!("spotify:artist:{}", artist.id()));
    }

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

    #[test]
    fn scope_round_trip() {
        let json =
            r#"{"access_token":"abc","expires_in":3600,"scope":"streaming playlist-read-private"}"#;
        let tok = parse_token(json).expect("token json should parse");
        assert!(tok.scopes.contains(SCOPE_STREAMING));
        assert!(tok.scopes.contains("playlist-read-private"));
        assert!(!tok.scopes.contains("user-modify-playback-state"));
    }

    #[test]
    fn scope_absent_is_unlinked_for_streaming() {
        let tok = parse_token("BQC8xYt0aBcDeFgHiJkLmNoPqRsTuVwXyZ").expect("plain token parses");
        assert!(!tok.scopes.contains(SCOPE_STREAMING));
    }
}
