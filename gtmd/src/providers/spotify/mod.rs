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
pub mod lyrics;
pub mod oauth;
pub mod stream;
pub mod ytfb;

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use chrono::{Duration, Utc};
use futures::StreamExt;
use rspotify::AuthCodePkceSpotify;
use rspotify::clients::{BaseClient, OAuthClient};
use rspotify::model::{AdditionalType, PlayableItem, PlaylistId, RepeatState, Token};
use rspotify::{CallbackError, Config, Credentials, OAuth, TokenCallback};
use tracing::{debug, info, warn};

use gtm::shared::secret::{
    SPOTIFY_CLIENT_ID, SPOTIFY_STREAM_KEY, SPOTIFY_TOKEN_KEY, delete_secret, get_secret, set_secret,
};
use gtm::shared::spotify::{LIBRESPOT_CLIENT_ID, SpotifyPlaylist, SpotifyStatus, SpotifyTrack};

use api::{pick_largest_image, track_from_playable};

pub use gtm::shared::spotify::pretty_id;

/// How long a playlist snapshot is served without re-fetching, in seconds.
///
/// Long enough that ordinary restarts and reconnects cost nothing, short enough
/// that a stale library is not what a user sees after a day of listening.
const SNAPSHOT_TTL: i64 = 6 * 3600;

const TOKEN_FILE: &str = "spotify.json";
/// Last good playlist snapshot, so a reconnect does not have to re-fetch the
/// whole catalogue to show anything.
///
/// The Web API answers `429` for a development-mode app once its quota is
/// spent, and it answers it for *every* request until the window resets — not
/// just the one that overspent. `/v1/me` alone was hit 17 times during a single
/// reconnect, every attempt a rung on a backoff ladder that never gave up, so
/// the quota never recovered. This file is what lets a reconnect still succeed
/// while the quota is spent.
const PLAYLISTS_FILE: &str = "spotify_playlists.json";

/// The snapshot written by [`SpotifyManager::commit_sync`].
///
/// `synced_at` is unix seconds, absent in snapshots written before it existed so
/// they read as stale rather than as fresh-and-zero.
#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct PlaylistSnapshot {
    user: Option<String>,
    playlists: Vec<SpotifyPlaylist>,
    #[serde(default)]
    synced_at: Option<i64>,
}
/// Name of the file holding the Web API app id, which is also the id that
/// minted the stored token.
///
/// Written at link time alongside the token, because the id is not free to
/// change afterwards: a refresh must be presented to the app that ran the
/// authorization, so a token minted by one app cannot be renewed by another.
/// The file is therefore read to learn the token's issuer, not merely to
/// configure a preference — which is why an install that predates it defaults
/// to librespot's id, the only id that ever issued these tokens.
const CLIENT_ID_FILE: &str = "spotify_client_id";
/// The Connect credential, kept apart from the Web API one.
///
/// login5 sends the session's client id next to the stored credential and
/// refuses any pairing that does not match ("this request will only work when
/// the store credentials match the client-id"), so a token minted by the user's
/// own app cannot be presented to a session registering as librespot's — it is
/// answered `INVALID_CREDENTIALS`, while every Web API call still succeeds
/// because `api.spotify.com` accepts any valid app's token. One token can only
/// be minted by one app, so the two legs need one each.
const STREAM_FILE: &str = "spotify_stream.json";
const TOKEN_ACCESS_PERMS: u32 = 0o600;
/// OAuth scope required for librespot native playback. Tokens issued before it
/// was requested keep working for the Web API but cannot stream audio.
const SCOPE_STREAMING: &str = "streaming";

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
    /// The Connect half: a second client, always built with
    /// [`LIBRESPOT_CLIENT_ID`](gtm::shared::spotify::LIBRESPOT_CLIENT_ID)
    /// whatever the Web API leg is configured with. `None` until the playback
    /// link has run, which is what [`Self::needs_play_link`] reports.
    stream: Option<AuthCodePkceSpotify>,
    /// Scopes of the stream token, snapshotted like [`Self::scopes`]. The
    /// `streaming` scope is what the Connect half is gated on.
    stream_scopes: std::collections::HashSet<String>,
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
    /// Set when the Web API last answered `429`, cleared on the next success.
    ///
    /// Spotify's quota is per-window, not per-request: once a development-mode
    /// app spends it, *every* call is refused until the window resets, and each
    /// refusal is charged against the same budget. So a spent quota means stop
    /// calling, not call again more slowly — the retry ladder in the daemon's
    /// startup sync was what turned one 429 into seventeen.
    rate_limited: bool,
    error: Option<String>,
}

impl SpotifyManager {
    pub fn new(config_dir: PathBuf) -> Self {
        Self {
            config_dir,
            client: None,
            stream: None,
            stream_scopes: std::collections::HashSet::new(),
            user: None,
            premium: false,
            playing: false,
            device: None,
            device_id: None,
            shuffle: false,
            repeat: "off".to_string(),
            playlists: Vec::new(),
            scopes: std::collections::HashSet::new(),
            rate_limited: false,
            error: None,
        }
    }

    /// True while the Web API is refusing calls with `429`.
    pub fn rate_limited(&self) -> bool {
        self.rate_limited
    }

    /// True when a previous sync left a library on disk, i.e. there is something
    /// to fall back on if the quota is spent.
    pub fn has_cached_playlists(&self) -> bool {
        !self.playlists.is_empty()
    }

    /// True when the snapshot is recent enough to skip a re-fetch.
    ///
    /// A full sync is the single most expensive thing gtm asks of the Web API —
    /// `/v1/me` plus the whole paginator plus a track pass per playlist, which
    /// is 17 calls for a modest library and scales with it. Paying that on every
    /// reconnect is what spends a shared app id's quota, and a reconnect is not a
    /// user action that implies fresh data. A snapshot younger than
    /// [`SNAPSHOT_TTL`] is reused as-is; older, or one written before `synced_at`
    /// existed, is re-fetched.
    pub fn snapshot_fresh(&self) -> bool {
        self.load_snapshot().is_some_and(|s| {
            s.synced_at.is_some_and(|at| {
                let age = Utc::now().timestamp().saturating_sub(at);
                (0..=SNAPSHOT_TTL).contains(&age)
            })
        })
    }

    /// Note a Web API outcome: `429` marks the quota spent, anything else clears
    /// the mark so a recovered quota is noticed on the next call.
    pub fn note_api_status(&mut self, status: u16) {
        if status == 429 {
            if !self.rate_limited {
                warn!("spotify web api rate limited (429) — serving cached data until it resets");
            }
            self.rate_limited = true;
        } else {
            self.rate_limited = false;
        }
    }

    /// Absolute path of the token cache file.
    pub fn token_path(&self) -> PathBuf {
        self.config_dir.join(TOKEN_FILE)
    }

    /// Absolute path of the playlist snapshot.
    fn playlists_path(&self) -> PathBuf {
        self.config_dir.join(PLAYLISTS_FILE)
    }

    /// The last playlist snapshot written by a successful sync, if any.
    ///
    /// Loaded at construction so the TUI has a library to show before any
    /// request goes out, and so a quota-spent reconnect degrades to the
    /// previous sync rather than to an empty pane.
    pub fn load_snapshot(&self) -> Option<PlaylistSnapshot> {
        let raw = std::fs::read_to_string(self.playlists_path()).ok()?;
        match serde_json::from_str::<PlaylistSnapshot>(&raw) {
            Ok(snap) if !snap.playlists.is_empty() => Some(snap),
            Ok(_) => None,
            Err(e) => {
                warn!("spotify playlist snapshot unreadable ({e}) — ignoring it");
                None
            }
        }
    }

    /// Write the snapshot, best-effort: a failure here only costs a slower
    /// next start, so it must never fail the sync that produced it.
    ///
    /// The config dir is created first because the snapshot is the whole reason
    /// a quota-spent reconnect can serve anything — silently skipping the write
    /// when the dir happens to be missing would leave the feature dead exactly
    /// where it is needed, with nothing in the log but a swallowed error.
    fn save_snapshot(&self, user: Option<String>, playlists: &[SpotifyPlaylist]) {
        let snap = PlaylistSnapshot {
            user,
            playlists: playlists.to_vec(),
            synced_at: Some(Utc::now().timestamp()),
        };
        let Ok(json) = serde_json::to_string(&snap) else {
            warn!("spotify: could not serialise the playlist snapshot");
            return;
        };
        if let Err(e) = std::fs::create_dir_all(&self.config_dir) {
            warn!("spotify: could not create the config dir for the snapshot: {e}");
            return;
        }
        if let Err(e) = std::fs::write(self.playlists_path(), json) {
            warn!("spotify: could not write playlist snapshot: {e}");
        }
    }

    /// The app id the Spotify Connect session registers as, which is fixed.
    ///
    /// This one genuinely cannot be chosen. librespot logs in with
    /// `Login_method::StoredCredential`, which sends the session's client id
    /// next to the credential, and login5 refuses the pair unless the id is the
    /// app that issued it: *"this request will only work when the store
    /// credentials match the client-id"* (librespot-core `login5.rs`). Both
    /// halves of that rule were broken separately, and each looked like an
    /// unrelated bug:
    ///
    /// * a self-registered app is not a recognised playback app, so presenting
    ///   it to Connect is answered `BAD_REQUEST` — even though the Web API
    ///   accepts its tokens, so sync, search, artwork and lyrics all worked
    ///   and only audio was missing;
    /// * pairing librespot's id with a token minted by a self-registered app is
    ///   answered `INVALID_CREDENTIALS`.
    ///
    /// So this stays librespot's, whatever else is configured.
    pub fn client_id(&self) -> &'static str {
        LIBRESPOT_CLIENT_ID
    }

    /// The app id the Web API token is presented as, and the one that refreshes
    /// it.
    ///
    /// Deliberately separate from [`Self::client_id`]. Rate limits are keyed per
    /// app, not per user, so a Web API token minted by librespot's public id
    /// shares one quota with every other librespot install on the internet —
    /// which is why this installation's sync, search and artwork were being
    /// answered `429` by a pool it does not use. Supplying the user's own app
    /// id moves those calls into a bucket of their own.
    ///
    /// The split is safe because only the *Connect* half has to match the
    /// session: `api.spotify.com` accepts any valid app's token, and the
    /// refresh only has to be presented to the app that ran the authorization.
    /// The audio path never sees this id.
    ///
    /// Resolution order is `GTM_SPOTIFY_CLIENT_ID`, then the id file, then
    /// librespot's id — so an install that configures nothing behaves exactly as
    /// before, and CI needs no Spotify app to exercise the code.
    pub fn web_client_id(&self) -> String {
        if let Ok(id) = std::env::var("GTM_SPOTIFY_CLIENT_ID") {
            let id = id.trim().to_string();
            if !id.is_empty() {
                return id;
            }
        }
        if let Ok(id) = std::fs::read_to_string(self.config_dir.join(CLIENT_ID_FILE)) {
            let id = id.trim().to_string();
            if !id.is_empty() {
                return id;
            }
        }
        LIBRESPOT_CLIENT_ID.to_string()
    }

    /// Whether the Web API has its own app id, i.e. its quota is its own.
    pub fn has_own_web_quota(&self) -> bool {
        self.web_client_id() != LIBRESPOT_CLIENT_ID
    }

    /// Persist `id` as the Web API app id, so a link started now mints its
    /// token against it and a later refresh is presented to the same app.
    ///
    /// A link is required before the id takes effect: an existing token belongs
    /// to the app that issued it, so swapping the id out from under one would
    /// make its next refresh fail. The caller is expected to run the OAuth flow
    /// immediately after this.
    pub fn set_web_client_id(&self, id: &str) {
        self.save_client_id(id);
    }

    /// The app id that issued the stored token, for refreshing it.
    ///
    /// Falls back to librespot's id when the file is missing, which is the
    /// correct answer rather than a fallback: it is the only id that could
    /// have minted a token from before the file existed, so those installs
    /// keep refreshing. The distinction matters because a token cannot be
    /// renewed by a different app than issued it.
    fn token_client_id(&self) -> String {
        if let Ok(id) = std::fs::read_to_string(self.config_dir.join(CLIENT_ID_FILE)) {
            let id = id.trim().to_string();
            if !id.is_empty() {
                return id;
            }
        }
        LIBRESPOT_CLIENT_ID.to_string()
    }

    /// Record the id that issued the stored token, so a later refresh is
    /// presented to the right app. Best-effort: a failure here costs a
    /// re-authorize on the next expiry, so it must not fail the link that
    /// succeeded.
    fn save_client_id(&self, id: &str) {
        let path = self.config_dir.join(CLIENT_ID_FILE);
        if let Err(e) = std::fs::create_dir_all(&self.config_dir) {
            warn!("spotify: could not create the config dir for the client id: {e}");
            return;
        }
        if let Err(e) = std::fs::write(&path, id) {
            warn!("spotify: could not write the client id: {e}");
            return;
        }
        if let Err(e) =
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(TOKEN_ACCESS_PERMS))
        {
            warn!("spotify: could not chmod the client id: {e}");
        }
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
        self.set_client(token).await?;
        // The Connect half is independent: an install linked before the split,
        // or one whose playback link has not run yet, must still surface its
        // Web API work rather than being reported unlinked.
        if self.needs_play_link()
            && let Ok(raw) = tokio::fs::read_to_string(self.stream_path()).await
        {
            match self.link_play(&raw) {
                Ok(()) => info!("spotify playback token loaded"),
                Err(e) => warn!("spotify playback token unusable ({e}) — re-link for playback"),
            }
        }
        Ok(())
    }

    /// Accept a token (plain access token or full Token JSON), persist it with
    /// 0600 permissions, then link and sync.
    pub async fn set_token(&mut self, raw: &str) -> Result<(), String> {
        let token = parse_token(raw)?;
        self.save_token(&token)?;
        // Mirror the token into the OS keychain so it survives the file-based
        // token being cleared and can be restored without a re-login.
        set_secret(SPOTIFY_TOKEN_KEY, raw);
        // The token is being authorized against the configured Web API id, so
        // that is the id a later refresh has to present.
        let id = self.web_client_id();
        self.save_client_id(&id);
        self.init_client(token).await
    }

    /// Remove the token file and reset all in-memory state.
    pub fn clear(&mut self) {
        self.client = None;
        self.stream = None;
        self.stream_scopes.clear();
        self.user = None;
        self.premium = false;
        self.playing = false;
        self.device = None;
        self.playlists.clear();
        self.error = None;
        // Only the token goes. The client-id file is kept: it is a preference,
        // not a credential, and it outlives the token it issued. Deleting it
        // would make every unlink/re-link cycle cost the user a retyped id for
        // no benefit, and there is no mismatch to guard against — the token it
        // named is gone, so the next link simply mints a new one against it.
        // Both credentials go: keeping the Connect one would let a re-link
        // silently resume the previous account's playback session.
        for path in [self.token_path(), self.stream_path()] {
            match std::fs::remove_file(&path) {
                Ok(()) => info!("removed spotify {}", path.display()),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => warn!("failed to remove {}: {e}", path.display()),
            }
        }
        // Drop any keychain-stored credentials too, including the client id an
        // older build kept there.
        delete_secret(SPOTIFY_TOKEN_KEY);
        delete_secret(SPOTIFY_STREAM_KEY);
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
            needs_play_link: self.needs_play_link(),
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

    /// Whether the Web API token predates the `streaming` scope, which is what
    /// a re-link would fix. Playback no longer depends on this token — it has
    /// its own — so this is a statement about the Web API grant alone, and the
    /// play path must not gate on it.
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
                name: pretty_id("liked-songs"),
                owner: user.clone().unwrap_or_default(),
                // The synthetic entry does have art on Spotify's side, but it
                // is not in `metas` — the paginator only walks real playlists.
                image_url: None,
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
                image_url: pick_largest_image(&meta.images),
                tracks,
            });
        }
        Ok((user, playlists))
    }

    /// Swap a completed sync snapshot into the manager. `status()` and
    /// `playlists()` only ever contend for this brief swap, never for the
    /// minutes of network pagination that preceded it.
    ///
    /// A snapshot that is empty, or a large regression against what is already
    /// cached, is rejected: `run_sync` tolerates a page error by stopping the
    /// walk, so a partial pass was committing a truncated — sometimes empty —
    /// library over a good one and blanking the TUI's playlist list until the
    /// next restart.
    pub fn commit_sync(&mut self, user: Option<String>, playlists: Vec<SpotifyPlaylist>) {
        if playlists.is_empty() {
            warn!("spotify sync returned no playlists — keeping the cached list");
            self.error = Some("playlist sync returned nothing".into());
            return;
        }
        if self.playlists.len() > playlists.len() / 2 {
            warn!(
                "spotify sync returned {} playlists against {} cached — keeping the cached list",
                playlists.len(),
                self.playlists.len()
            );
            return;
        }
        self.error = None;
        self.save_snapshot(user.clone(), &playlists);
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
        // Record the app that just issued this token, so the refresh below and
        // every later one is presented to it. Without this the id file is
        // absent on the OAuth path and the refresh falls back to librespot's,
        // which is right for a default link and wrong the moment a user
        // supplied their own.
        let id = self.web_client_id();
        self.save_client_id(&id);
        self.set_client(token).await?;
        // Reconnecting previously cost `/v1/me` here, again inside `run_sync`,
        // and again on every rung of the retry ladder — 17 calls to one endpoint
        // for a single reconnect, which is enough to spend a development-mode
        // app's quota and earn a `429` for the rest of the window. The snapshot
        // already holds the display name and the whole library, so a reconnect
        // that has one goes to it and issues no request at all.
        if let Some(snap) = self.load_snapshot() {
            let count = snap.playlists.len();
            self.user = snap.user;
            self.playlists = snap.playlists;
            self.error = None;
            debug!("spotify reconnect: {count} playlists from the snapshot, no request sent");
        }
        // Only reach for `/v1/me` when the snapshot cannot name the user. A
        // first link has nothing cached, so this still runs then.
        if self.user.is_none()
            && let Some(client) = self.client.clone()
            && let Ok(Ok(me)) =
                tokio::time::timeout(std::time::Duration::from_secs(10), client.me()).await
        {
            self.user = me.display_name.or_else(|| Some(me.id.as_ref().to_string()));
        }
        // Probe `/me/player` right after linking so `premium` is set before any
        // play command arrives (playlists sync purely via the Web API and never
        // implied Premium). It is a second request against the same quota, and
        // Premium is not needed to browse, so a quota-spent reconnect skips it
        // and reports playback as unavailable instead of being throttled for it.
        if !self.rate_limited {
            let _ =
                tokio::time::timeout(std::time::Duration::from_secs(10), self.refresh_playback())
                    .await;
        }
        Ok(())
    }

    /// Build a usable rspotify client from a token (persist-free). Does not
    /// touch the playlist cache or call the network; `linked()` becomes true
    /// once this returns.
    async fn set_client(&mut self, token: Token) -> Result<(), String> {
        self.scopes = token.scopes.clone();
        // The refresh has to be presented as the app that ran the
        // authorization, or the renewed token is rejected and every later Web
        // API call 401s. That is the Web API's id, which is *not* necessarily
        // the Connect session's — so it is read back from the file written at
        // link time rather than from the current preference. Reading the
        // preference instead would 401 every existing install the moment
        // someone set an id, since their token belongs to the old app.
        let id = self.token_client_id();
        let client = self.build(token, &id, self.token_path(), SPOTIFY_TOKEN_KEY);
        self.client = Some(client.clone());
        self.adopt_stream(id, client);
        self.error = None;
        Ok(())
    }

    /// Point the Connect leg at the Web API token when librespot's app issued
    /// it, so a default install needs one authorization rather than two.
    ///
    /// This is not a shortcut, it is the only correct answer for that pairing:
    /// when both legs run on the same app, a second grant is not a second
    /// credential — Spotify rotates the refresh token on every new grant for
    /// the same app, so authorizing again would invalidate the token just
    /// stored and leave the Web API 401ing.
    fn adopt_stream(&mut self, id: String, client: AuthCodePkceSpotify) {
        if id != LIBRESPOT_CLIENT_ID {
            return;
        }
        self.stream_scopes = self.scopes.clone();
        self.stream = Some(client);
    }

    /// An rspotify client that persists every refresh back to `path` under
    /// `key`. Shared by both legs: they differ only in the app id presented and
    /// where the token is written, and duplicating the callback would let the
    /// two drift apart on the one thing that must not drift — a refreshed
    /// token that is never written is a session that silently stops working
    /// after its first expiry.
    fn build(
        &self,
        token: Token,
        id: &str,
        path: PathBuf,
        key: &'static str,
    ) -> AuthCodePkceSpotify {
        let refreshable = token.refresh_token.is_some();
        let token_callback = TokenCallback(Box::new(move |refreshed: Token| {
            let dir = path.parent().ok_or_else(|| {
                CallbackError::CustomizedError("token path has no parent".to_string())
            })?;
            std::fs::create_dir_all(dir)
                .map_err(|e| CallbackError::CustomizedError(format!("create dir: {e}")))?;
            let json = serde_json::to_string(&refreshed)
                .map_err(|e| CallbackError::CustomizedError(format!("serialize: {e}")))?;
            std::fs::write(&path, &json)
                .map_err(|e| CallbackError::CustomizedError(format!("write: {e}")))?;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(TOKEN_ACCESS_PERMS))
                .map_err(|e| CallbackError::CustomizedError(format!("chmod: {e}")))?;
            set_secret(key, &json);
            Ok::<(), CallbackError>(())
        }));
        AuthCodePkceSpotify::from_token_with_config(
            token,
            Credentials::new_pkce(id),
            OAuth::default(),
            Config {
                token_cached: false,
                token_refreshing: refreshable,
                token_callback_fn: Arc::new(Some(token_callback)),
                ..Default::default()
            },
        )
    }

    /// The Connect client, or `None` before the playback link has run. Only
    /// ever built with librespot's id — see [`STREAM_FILE`].
    pub fn stream_client(&self) -> Option<AuthCodePkceSpotify> {
        self.stream.clone()
    }

    /// Whether the Connect half has a usable credential.
    pub fn play_linked(&self) -> bool {
        self.stream.is_some() && self.stream_scopes.contains(SCOPE_STREAMING)
    }

    /// Whether a linked account still needs its playback authorization: the Web
    /// API works but the Connect session would be refused, which the user sees
    /// as every track silently producing no audio.
    pub fn needs_play_link(&self) -> bool {
        self.linked() && !self.play_linked()
    }

    /// Accept the playback credential, persist it, and build the Connect
    /// client. Separate from [`Self::link`] because the two tokens have
    /// different issuers and are stored in different files.
    pub fn link_play(&mut self, raw: &str) -> Result<(), String> {
        let token = parse_token(raw)?;
        let json = serde_json::to_string(&token).map_err(|e| format!("serialize token: {e}"))?;
        self.save(&self.stream_path(), &json)?;
        set_secret(SPOTIFY_STREAM_KEY, &json);
        self.stream_scopes = token.scopes.clone();
        self.stream = Some(self.build(
            token,
            LIBRESPOT_CLIENT_ID,
            self.stream_path(),
            SPOTIFY_STREAM_KEY,
        ));
        Ok(())
    }

    /// Absolute path of the Connect credential.
    fn stream_path(&self) -> PathBuf {
        self.config_dir.join(STREAM_FILE)
    }

    async fn init_client(&mut self, token: Token) -> Result<(), String> {
        let refreshable = token.refresh_token.is_some();
        self.set_client(token).await?;
        match self.sync().await {
            Ok(()) => {
                info!(
                    "linked spotify as {:?} ({} playlists, auto-refresh: {refreshable}, web app: {})",
                    self.user,
                    self.playlists.len(),
                    // Which app the Web API calls bill against. A `429` here is
                    // a shared-quota symptom, and the id is the only thing that
                    // distinguishes "contended" from "your own app is spent".
                    if self.has_own_web_quota() {
                        "own"
                    } else {
                        "shared (librespot)"
                    }
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
        let json = serde_json::to_string(token).map_err(|e| format!("serialize token: {e}"))?;
        self.save(&self.token_path(), &json)
    }

    /// Write a token to `path` with 0600 permissions. Both credentials go
    /// through here, so neither can end up world-readable by omission.
    fn save(&self, path: &Path, json: &str) -> Result<(), String> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| format!("create config dir: {e}"))?;
        }
        std::fs::write(path, json).map_err(|e| format!("write token file: {e}"))?;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(TOKEN_ACCESS_PERMS))
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
    use super::{SCOPE_STREAMING, TOKEN_ACCESS_PERMS, parse_token, pretty_id};
    use chrono::Utc;
    use gtm::shared::spotify::SpotifyPlaylist;
    use rspotify::model::idtypes::Id;
    use rspotify::model::{AlbumId, ArtistId, TrackId};

    /// The two app ids are independent, and only the playback one is fixed.
    ///
    /// This is the whole point of the split, and it is easy to undo by
    /// accident: pointing the Web API back at librespot's id puts every
    /// install on one shared quota again, and the `429`s come back with
    /// nothing in the code to explain them.
    #[test]
    fn the_web_id_is_separate_from_the_connect_id() {
        let dir = std::env::temp_dir().join("gtm-spotify-id-test");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let mgr = super::SpotifyManager::new(dir.clone());

        // Unconfigured: both legs fall back to librespot's, so an install that
        // sets nothing behaves exactly as it did before the split.
        assert_eq!(mgr.web_client_id(), super::LIBRESPOT_CLIENT_ID);
        assert_eq!(mgr.token_client_id(), super::LIBRESPOT_CLIENT_ID);
        assert!(!mgr.has_own_web_quota());

        // The Connect leg is fixed and ignores the file entirely.
        mgr.set_web_client_id("0123456789abcdef0123456789abcdef");
        assert_eq!(mgr.client_id(), super::LIBRESPOT_CLIENT_ID);
        assert_eq!(mgr.web_client_id(), "0123456789abcdef0123456789abcdef");
        assert!(mgr.has_own_web_quota());
        // The issuer is remembered, so a later refresh is presented to the app
        // that actually minted the token.
        assert_eq!(mgr.token_client_id(), "0123456789abcdef0123456789abcdef");

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A Web API link with no playback credential is the state that used to
    /// play nothing while reporting itself as fully linked.
    ///
    /// One token can only be minted by one app, so a Web API token from the
    /// user's own app is refused by a session registering as librespot's while
    /// every Web API call it makes still succeeds. `needs_play_link` is what
    /// turns that silence into something the Settings panel can state.
    #[tokio::test]
    async fn playback_needs_its_own_token() {
        let dir = std::env::temp_dir().join(format!("gtm-play-link-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let mut mgr = super::SpotifyManager::new(dir.clone());
        mgr.set_web_client_id("0123456789abcdef0123456789abcdef");

        // The web leg linked, streaming scope and all. `needs_relink` is
        // therefore false, so the pre-split signal says everything is fine.
        let web =
            r#"{"access_token":"web","expires_in":3600,"scope":"streaming playlist-read-private"}"#;
        mgr.set_token(web).await.expect("web link");
        assert!(mgr.linked());
        assert!(!mgr.needs_relink(), "the web token has the scope");
        assert!(
            mgr.needs_play_link(),
            "a web-only link cannot play, and must say so"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A bare access token carries no scope list, which is the real shape of a
    /// token the API did not echo scopes back for. It must not be mistaken for a
    /// playable one.
    #[test]
    fn a_scopeless_token_does_not_enable_playback() {
        let dir = std::env::temp_dir().join(format!("gtm-no-scope-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let mut mgr = super::SpotifyManager::new(dir.clone());
        let bare = "BQC8xYt0aBcDeFgHiJkLmNoPqRsTuVwXyZ";
        let token = parse_token(bare).expect("bare token parses");
        assert!(token.scopes.is_empty(), "a bare token has no scopes");
        mgr.stream_scopes = token.scopes.clone();
        assert!(!mgr.play_linked(), "no scopes means no playback");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The playback file is distinct from the Web API one, so a refresh of
    /// either cannot clobber the other and silently unlink playback.
    #[test]
    fn the_two_tokens_do_not_share_a_file() {
        let dir = std::env::temp_dir().join(format!("gtm-two-tokens-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let mgr = super::SpotifyManager::new(dir.clone());
        assert_ne!(mgr.token_path(), mgr.stream_path());
        assert!(mgr.token_path().starts_with(&dir));
        assert!(mgr.stream_path().starts_with(&dir));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Unlinking must take the Connect credential with it. Keeping it would let
    /// the next link inherit the previous account's playback session, which is
    /// a credential surviving an explicit unlink.
    #[test]
    fn clear_takes_both_credentials() {
        let dir = std::env::temp_dir().join(format!("gtm-clear-both-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let mut mgr = super::SpotifyManager::new(dir.clone());
        std::fs::write(mgr.token_path(), b"{}").unwrap();
        std::fs::write(mgr.stream_path(), b"{}").unwrap();
        mgr.clear();
        assert!(!mgr.token_path().exists(), "web token must be gone");
        assert!(!mgr.stream_path().exists(), "playback token must be gone");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A default install authorizes once, not twice.
    ///
    /// With no id file the Web API token is minted by librespot's app, so it is
    /// already a valid Connect credential. Running the playback flow anyway
    /// would be actively harmful: Spotify rotates the refresh token on every
    /// new grant for the same app, so the second authorization would
    /// invalidate the token just stored and leave every Web API call 401ing.
    #[tokio::test]
    async fn a_shared_app_needs_only_one_authorization() {
        let dir = std::env::temp_dir().join(format!("gtm-shared-app-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let web = r#"{"access_token":"shared","expires_in":3600,"scope":"streaming"}"#;
        let mut mgr = super::SpotifyManager::new(dir.clone());
        // No id file: both legs fall back to librespot's app.
        assert!(!mgr.has_own_web_quota());
        mgr.set_token(web).await.expect("link");
        assert!(mgr.linked());
        assert!(
            mgr.play_linked(),
            "the web token is already a valid connect credential"
        );
        assert!(
            !mgr.needs_play_link(),
            "a shared app must not trigger a second authorization"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// With the user's own app the Web API token cannot be reused for Connect,
    /// so that is the one case that does need a second authorization.
    #[tokio::test]
    async fn an_own_app_does_need_a_second_authorization() {
        let dir = std::env::temp_dir().join(format!("gtm-own-app-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let web = r#"{"access_token":"mine","expires_in":3600,"scope":"streaming"}"#;
        let mut mgr = super::SpotifyManager::new(dir.clone());
        mgr.set_web_client_id("0123456789abcdef0123456789abcdef");
        mgr.set_token(web).await.expect("link");
        assert!(mgr.has_own_web_quota());
        assert!(
            mgr.needs_play_link(),
            "login5 refuses a token the session's app did not mint"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A linked account whose playback token is on disk comes back linked for
    /// playback too, so a daemon restart does not put every track back into the
    /// silent state. The credential is read independently of the Web API one:
    /// a missing playback token must not stop the Web API from linking.
    #[tokio::test]
    async fn a_stored_playback_token_survives_a_restart() {
        let dir = std::env::temp_dir().join(format!("gtm-play-restart-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let web = r#"{"access_token":"web","expires_in":3600,"scope":"streaming"}"#;
        let play = r#"{"access_token":"play","expires_in":3600,"scope":"streaming"}"#;
        {
            let mut mgr = super::SpotifyManager::new(dir.clone());
            // The user's own app, so the web token is not a Connect credential
            // and a second one is genuinely required.
            mgr.set_web_client_id("0123456789abcdef0123456789abcdef");
            mgr.set_token(web).await.expect("web link");
            assert!(mgr.needs_play_link());
            mgr.link_play(play).expect("playback link");
            assert!(mgr.play_linked(), "both legs linked");
        }
        // A brand-new manager, as after a daemon restart.
        let mut fresh = super::SpotifyManager::new(dir.clone());
        fresh.load().await.expect("load");
        assert!(fresh.linked(), "the web leg comes back");
        assert!(
            fresh.play_linked(),
            "the playback leg must come back too, or every track is silent"
        );
        assert!(!fresh.needs_play_link());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// An account with no playback token on disk still links for the Web API.
    /// The pre-split code reported such an install as needing a re-link, which
    /// cannot help: re-linking mints the same unusable pairing.
    #[tokio::test]
    async fn a_web_only_link_still_works_for_the_web_api() {
        let dir = std::env::temp_dir().join(format!("gtm-web-only-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let web = r#"{"access_token":"web","expires_in":3600,"scope":"streaming"}"#;
        let mut mgr = super::SpotifyManager::new(dir.clone());
        mgr.set_token(web).await.expect("web link");
        assert!(
            mgr.linked(),
            "a missing playback token must not unlink the web api"
        );
        assert!(mgr.needs_play_link());
        assert!(
            mgr.stream_client().is_none(),
            "no playback client exists yet"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A token cannot be renewed by a different app than the one that issued
    /// it, so the persisted id — not the current preference — is what a
    /// refresh presents. Reading the preference instead would 401 every
    /// existing install the moment someone set an id.
    #[tokio::test]
    async fn a_missing_id_file_means_the_token_came_from_librespot() {
        let dir = std::env::temp_dir().join(format!("gtm-issuer-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let mgr = super::SpotifyManager::new(dir.clone());
        // No file at all: the only id that could have issued this token is
        // librespot's, so that is the correct answer rather than a guess.
        assert_eq!(mgr.token_client_id(), super::LIBRESPOT_CLIENT_ID);
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn playlist(id: &str) -> SpotifyPlaylist {
        SpotifyPlaylist {
            image_url: None,
            id: id.to_string(),
            name: id.to_string(),
            owner: String::new(),
            tracks: Vec::new(),
        }
    }

    /// The synthetic Liked Songs entry and a bare track URI are the two ids that
    /// reach a UI with no label of their own; both must render as words.
    #[test]
    fn raw_ids_are_labelled() {
        assert_eq!(pretty_id("liked-songs"), "Liked Songs");
        assert_eq!(
            pretty_id("spotify:track:4cOdK2wGLETKBW3PvgPWqT"),
            "Spotify Track"
        );
    }

    /// A sync that came back empty is a failed pass, not an empty library.
    /// Committing it blanked the TUI's playlist list until the next restart.
    #[test]
    fn an_empty_sync_keeps_the_cache() {
        let mut mgr = super::SpotifyManager::new(std::path::PathBuf::from("/nonexistent"));
        mgr.commit_sync(None, vec![playlist("a"), playlist("b")]);
        mgr.commit_sync(None, Vec::new());
        assert_eq!(mgr.playlists.len(), 2);
        assert!(mgr.error.is_some(), "the failure must be reported");
    }

    /// `run_sync` stops paginating on a page error, so a truncated pass must not
    /// replace a good library either.
    #[test]
    fn a_truncated_sync_keeps_the_cache() {
        let mut mgr = super::SpotifyManager::new(std::path::PathBuf::from("/nonexistent"));
        mgr.commit_sync(None, vec![playlist("a"), playlist("b"), playlist("c")]);
        mgr.commit_sync(None, vec![playlist("a")]);
        assert_eq!(mgr.playlists.len(), 3);
    }

    /// The rate-limit mark is what stops the startup ladder from re-issuing a
    /// refused request, so it has to latch on 429 and clear on any success.
    #[test]
    fn a_429_latches_until_a_call_succeeds() {
        let mut mgr = super::SpotifyManager::new(std::path::PathBuf::from("/nonexistent"));
        assert!(!mgr.rate_limited(), "a fresh manager is not limited");
        mgr.note_api_status(429);
        assert!(mgr.rate_limited());
        mgr.note_api_status(429);
        assert!(mgr.rate_limited(), "a second 429 keeps it latched");
        mgr.note_api_status(200);
        assert!(!mgr.rate_limited(), "a success means the window reset");
    }

    /// A snapshot round-trips through disk, which is the whole point: a
    /// reconnect that reads it back must not need to ask Spotify for anything.
    #[test]
    fn a_snapshot_survives_a_restart() {
        // A fresh dir that does not exist yet, which is the first-link case and
        // the one where a snapshot that silently failed to write would leave the
        // feature dead exactly when it is needed.
        let dir = std::env::temp_dir().join(format!("gtm-snap-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let mut mgr = super::SpotifyManager::new(dir.clone());
        mgr.commit_sync(Some("Ada".into()), vec![playlist("a"), playlist("b")]);
        assert_eq!(mgr.playlists.len(), 2, "the sync must have committed");
        // A brand-new manager, as after a daemon restart: the library and the
        // display name must come back off disk with no network involved.
        let fresh = super::SpotifyManager::new(dir.clone());
        let snap = fresh
            .load_snapshot()
            .expect("snapshot should round-trip through commit_sync");
        assert_eq!(snap.playlists.len(), 2);
        assert_eq!(snap.user.as_deref(), Some("Ada"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The TTL decides whether a boot pays for a full paginator, so both edges
    /// have to hold: just-written is fresh, expired is not, and a snapshot with
    /// no timestamp is not fresh — treating a missing field as "epoch" would
    /// silently disable the skip for every install written before `synced_at`.
    #[test]
    fn snapshot_freshness_follows_the_ttl() {
        let dir = std::env::temp_dir().join(format!("gtm-snap-ttl-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let mut mgr = super::SpotifyManager::new(dir.clone());
        assert!(!mgr.snapshot_fresh(), "nothing cached is not fresh");
        mgr.commit_sync(Some("Ada".into()), vec![playlist("a")]);
        assert!(
            mgr.snapshot_fresh(),
            "a snapshot written just now must skip the refetch"
        );

        let mut snap = mgr.load_snapshot().expect("just committed");
        snap.synced_at = Some(Utc::now().timestamp() - super::SNAPSHOT_TTL - 1);
        std::fs::write(
            dir.join(super::PLAYLISTS_FILE),
            serde_json::to_string(&snap).unwrap(),
        )
        .unwrap();
        assert!(
            !mgr.snapshot_fresh(),
            "past the ttl the library must be refetched"
        );

        snap.synced_at = None;
        std::fs::write(
            dir.join(super::PLAYLISTS_FILE),
            serde_json::to_string(&snap).unwrap(),
        )
        .unwrap();
        assert!(
            !mgr.snapshot_fresh(),
            "a snapshot with no timestamp predates the field and is not fresh"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// An empty or corrupt snapshot must read as "nothing cached" rather than
    /// panicking or handing back a blank library.
    #[test]
    fn an_unusable_snapshot_is_ignored() {
        let dir = std::env::temp_dir().join(format!("gtm-snap-bad-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let mgr = super::SpotifyManager::new(dir.clone());

        assert!(mgr.load_snapshot().is_none(), "no file at all");

        std::fs::write(dir.join(super::PLAYLISTS_FILE), b"not json").unwrap();
        assert!(mgr.load_snapshot().is_none(), "corrupt json must not panic");

        std::fs::write(
            dir.join(super::PLAYLISTS_FILE),
            br#"{"user":null,"playlists":[]}"#,
        )
        .unwrap();
        assert!(
            mgr.load_snapshot().is_none(),
            "an empty library is not a usable snapshot"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A real growth still commits.
    #[test]
    fn a_growing_sync_commits() {
        let mut mgr = super::SpotifyManager::new(std::path::PathBuf::from("/nonexistent"));
        mgr.commit_sync(None, vec![playlist("a")]);
        mgr.commit_sync(None, vec![playlist("a"), playlist("b"), playlist("c")]);
        assert_eq!(mgr.playlists.len(), 3);
    }

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
