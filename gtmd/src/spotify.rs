// Copyright (c) 2026 - present
// Author: prjctimg <prjctimg@outlook.com>
// Spotify Web API: token persistence, account link, and playlist sync
//
// This is free software released under the GPL-3.0 license.

use std::collections::HashSet;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;

use chrono::Duration;
use futures::StreamExt;
use rspotify::AuthCodeSpotify;
use rspotify::clients::{BaseClient, OAuthClient};
use rspotify::model::{AdditionalType, PlayableItem, SearchType, Token};
use tracing::{debug, info, warn};

use gtm_core::spotify::{SpotifyPlaylist, SpotifyStatus, SpotifyTrack};

const TOKEN_FILE: &str = "spotify.json";
const TOKEN_ACCESS_PERMS: u32 = 0o600;

/// Owns the Spotify Web API client, its token file, and the playlist cache.
///
/// The access token is stored as `spotify.json` inside the daemon config
/// directory with 0600 permissions. The client is built from the raw token
/// without client credentials, so requests stop working once the token
/// expires; the user re-links with a fresh token at that point.
pub struct SpotifyManager {
    config_dir: PathBuf,
    client: Option<AuthCodeSpotify>,
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

    /// Read the token file and set up the client + cached playlists.
    pub async fn load(&mut self) -> Result<(), String> {
        let raw = std::fs::read_to_string(self.token_path())
            .map_err(|e| format!("read token file: {e}"))?;
        let token = parse_token(&raw)?;
        self.init_client(token).await
    }

    /// Accept a token (plain access token or full Token JSON), persist it with
    /// 0600 permissions, then link and sync.
    pub async fn set_token(&mut self, raw: &str) -> Result<(), String> {
        let token = parse_token(raw)?;
        self.save_token(&token)?;
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
        if !self.linked() {
            self.error = Some(err);
        }
    }

    /// Current OAuth access token, if a client is linked.
    pub async fn access_token(&self) -> Option<String> {
        use rspotify::clients::BaseClient;
        let client = self.client.as_ref()?;
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
            .client
            .as_ref()
            .ok_or_else(|| "spotify not linked".to_string())?;

        let me = client.me().await.map_err(|e| format!("me: {e}"))?;
        self.user = me.display_name.or_else(|| Some(me.id.as_ref().to_string()));
        // NOTE: rspotify's `me().product` was removed upstream (Spotify no
        // longer exposes the plan); Premium is instead probed via the
        // playback endpoint in `refresh_playback()`.

        // Collect the playlist metadata first (paginator borrows the client),
        // then fetch each playlist's tracks in a second pass.
        let mut metas = Vec::new();
        let mut paginator = client.current_user_playlists();
        while let Some(item) = paginator.next().await {
            let pl = item.map_err(|e| format!("playlists: {e}"))?;
            metas.push(pl);
        }
        debug!(
            "fetched {} spotify playlists for {:?}",
            metas.len(),
            self.user
        );

        let mut playlists = Vec::new();
        for meta in &metas {
            let tracks = self.fetch_playlist_tracks(client, meta.id.clone()).await;
            playlists.push(SpotifyPlaylist {
                id: meta.id.as_ref().to_string(),
                name: meta.name.clone(),
                owner: meta.owner.display_name.clone().unwrap_or_default(),
                tracks,
            });
        }
        self.playlists = playlists;
        Ok(())
    }

    async fn fetch_playlist_tracks(
        &self,
        client: &AuthCodeSpotify,
        playlist_id: rspotify::model::PlaylistId<'static>,
    ) -> Vec<SpotifyTrack> {
        let mut tracks = Vec::new();
        let mut items = client.playlist_items(playlist_id, None, None);
        while let Some(item) = items.next().await {
            match item {
                Ok(item) => {
                    if let Some(playable) = item.item.as_ref() {
                        if let Some(mut track) = track_from_playable(playable) {
                            track.index = tracks.len();
                            tracks.push(track);
                        }
                    }
                }
                Err(e) => warn!("spotify playlist item: {e}"),
            }
        }
        tracks
    }

    async fn init_client(&mut self, token: Token) -> Result<(), String> {
        self.client = Some(AuthCodeSpotify::from_token(token));
        self.error = None;
        match self.sync().await {
            Ok(()) => {
                info!(
                    "linked spotify as {:?} ({} playlists)",
                    self.user,
                    self.playlists.len()
                );
                Ok(())
            }
            Err(e) => {
                self.error = Some(e.clone());
                self.client = None;
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
        let q = format!("track:{query}");
        match client
            .search(&q, SearchType::Track, None, None, Some(limit), None)
            .await
        {
            Ok(rspotify::model::SearchResult::Tracks(page)) => page
                .items
                .iter()
                .enumerate()
                .map(|(i, t)| SpotifyTrack {
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
                })
                .collect(),
            _ => Vec::new(),
        }
    }
}

/// Convert an rspotify playable item into our IPC-friendly track shape.
fn track_from_playable(item: &PlayableItem) -> Option<SpotifyTrack> {
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
        }),
        PlayableItem::Episode(_) | PlayableItem::Unknown(_) => None,
    }
}

/// Accept either a full rspotify `Token` JSON object or a bare access token
/// string. Full-token JSON is preferred so scopes/refresh info is preserved.
fn parse_token(raw: &str) -> Result<Token, String> {
    let raw = raw.trim();
    if raw.is_empty() {
        return Err("empty token".into());
    }
    if let Ok(token) = serde_json::from_str::<Token>(raw) {
        if !token.access_token.is_empty() {
            return Ok(token);
        }
    }
    if !raw.contains('{') {
        return Ok(Token {
            access_token: raw.to_string(),
            expires_in: Duration::try_seconds(3600).ok_or("invalid default expiry")?,
            expires_at: None,
            refresh_token: None,
            scopes: HashSet::new(),
        });
    }
    Err("could not parse spotify token".into())
}

#[cfg(test)]
mod tests {
    use super::{TOKEN_ACCESS_PERMS, parse_token};

    #[test]
    fn parse_token_plain_access_token() {
        let tok =
            parse_token("BQC8xYt0aBcDeFgHiJkLmNoPqRsTuVwXyZ").expect("plain token should parse");
        assert_eq!(tok.access_token, "BQC8xYt0aBcDeFgHiJkLmNoPqRsTuVwXyZ");
        assert!(tok.refresh_token.is_none());
    }

    #[test]
    fn parse_token_full_json() {
        let json = r#"{"access_token":"abc","expires_in":3600,"scopes":""}"#;
        let tok = parse_token(json).expect("full token json should parse");
        assert_eq!(tok.access_token, "abc");
        assert_eq!(tok.expires_in.num_seconds(), 3600);
    }

    #[test]
    fn parse_token_rejects_empty() {
        assert!(parse_token("").is_err());
        assert!(parse_token("   ").is_err());
    }

    #[test]
    fn token_permissions_are_owner_only() {
        assert_eq!(TOKEN_ACCESS_PERMS, 0o600);
    }
}
