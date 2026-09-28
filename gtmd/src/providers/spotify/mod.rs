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

use api::{pick_largest_image, track_from_playable};

pub use gtm::shared::spotify::pretty_id;

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
#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct PlaylistSnapshot {
    user: Option<String>,
    playlists: Vec<SpotifyPlaylist>,
}
/// Name of the retired client-id file. Nothing writes it any more — the app id
/// is fixed, see [`SpotifyManager::client_id`] — but [`SpotifyManager::clear`]
/// still unlinks it so an install that linked under the old scheme stops
/// carrying an id that no longer has any meaning.
const CLIENT_ID_FILE: &str = "spotify_client_id";
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
    fn save_snapshot(&self, user: Option<String>, playlists: &[SpotifyPlaylist]) {
        let snap = PlaylistSnapshot {
            user,
            playlists: playlists.to_vec(),
        };
        if let Ok(json) = serde_json::to_string(&snap)
            && let Err(e) = std::fs::write(self.playlists_path(), json)
        {
            warn!("spotify: could not write playlist snapshot: {e}");
        }
    }

    /// The single Spotify app this installation identifies as, for the OAuth /
    /// Web API side *and* the Spotify Connect session.
    ///
    /// These are the same app, and that is not a simplification — it is
    /// required. librespot logs in with `Login_method::StoredCredential`, which
    /// sends the session's client id next to the credential, and login5 refuses
    /// the pair unless the id is the app that issued it: *"this request will
    /// only work when the store credentials match the client-id"*
    /// (librespot-core `login5.rs`). Both halves of that rule were broken
    /// separately, and each looked like an unrelated bug:
    ///
    /// * a self-registered app is not a recognised playback app, so presenting
    ///   it to Connect is answered `BAD_REQUEST` — even though the Web API
    ///   accepts its tokens, so sync, search, artwork and lyrics all worked
    ///   and only audio was missing;
    /// * pairing librespot's id with a token minted by a self-registered app is
    ///   answered `INVALID_CREDENTIALS`.
    ///
    /// So the id is fixed rather than configured, and never persisted: with only
    /// one identity there is nothing to keep in sync, which is precisely what
    /// made the conflation possible. The cost is that the Web API now shares
    /// an app id with every other librespot install and can be rate-limited
    /// into `429`; that is absorbed by caching, not by changing identity.
    pub fn client_id(&self) -> &'static str {
        LIBRESPOT_CLIENT_ID
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
        // The client-id file is no longer written, but an install that linked
        // before the id was retired still has one on disk. Clear it out so the
        // config dir does not keep advertising an id that nothing reads.
        for path in [self.token_path(), self.config_dir.join(CLIENT_ID_FILE)] {
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
        let refreshable = token.refresh_token.is_some();
        // Scopes are fixed at authorization time and cannot be widened by a
        // refresh, so snapshot them here: the token itself lives behind an
        // async mutex that cannot be inspected synchronously.
        self.scopes = token.scopes.clone();
        // The refresh has to be presented as the app that ran the
        // authorization, or the renewed token is rejected and every later Web
        // API call 401s. That app is the same one the Connect session
        // registers as — see [`Self::client_id`] — so a token pasted by hand
        // refreshes correctly too, with no stored id to go missing.
        let creds = Credentials::new_pkce(self.client_id());
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
    use super::{SCOPE_STREAMING, TOKEN_ACCESS_PERMS, parse_token, pretty_id};
    use gtm::shared::spotify::SpotifyPlaylist;
    use rspotify::model::idtypes::Id;
    use rspotify::model::{AlbumId, ArtistId, TrackId};

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
        let dir = std::env::temp_dir().join(format!("gtm-snap-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let mut mgr = super::SpotifyManager::new(dir.clone());
        mgr.commit_sync(Some("Ada".into()), vec![playlist("a"), playlist("b")]);
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
