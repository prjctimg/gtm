// Copyright (c) 2026
// Author: prjctimg <prjctimg@outlook.com>
// Last.fm scrobbling and now-playing support
//
// This is free software released under the GPL-3.0 license.

use std::sync::Arc;
use std::time::{Duration, Instant};

use md5::Context;
use reqwest::Client;
use tokio::sync::Mutex;
use tracing::{debug, info, warn};

use gtm_core::track::TrackInfo;

const LASTFM_API_URL: &str = "https://ws.audioscrobbler.com/2.0/";

/// Manages Last.fm authentication and scrobbling.
pub struct LastfmManager {
    client: Client,
    api_key: Option<String>,
    api_secret: Option<String>,
    session_key: Arc<Mutex<Option<String>>>,
    last_scrobble: Arc<Mutex<Option<Instant>>>,
    last_now_playing: Arc<Mutex<Option<Instant>>>,
}

impl Default for LastfmManager {
    fn default() -> Self {
        Self::new()
    }
}

impl LastfmManager {
    pub fn new() -> Self {
        Self {
            client: Client::new(),
            api_key: None,
            api_secret: None,
            session_key: Arc::new(Mutex::new(None)),
            last_scrobble: Arc::new(Mutex::new(None)),
            last_now_playing: Arc::new(Mutex::new(None)),
        }
    }

    /// Initialize with API credentials from config.
    pub async fn init(&mut self, api_key: String, api_secret: String, session_key: Option<String>) {
        self.api_key = Some(api_key);
        self.api_secret = Some(api_secret);
        if let Some(sk) = session_key {
            *self.session_key.lock().await = Some(sk);
        }
    }

    /// Check if Last.fm is configured and authenticated.
    pub fn is_ready(&self) -> bool {
        self.api_key.is_some() && self.api_secret.is_some() && self.session_key.blocking_lock().is_some()
    }

    /// Get the authorization URL for the user to grant permission.
    pub fn auth_url(&self) -> Option<String> {
        let api_key = self.api_key.as_ref()?;
        Some(format!("https://www.last.fm/api/auth/?api_key={api_key}"))
    }

    /// Exchange a token for a session key after user authorization.
    pub async fn authenticate(&mut self, token: &str) -> Result<String, String> {
        let api_key = self.api_key.as_ref().ok_or("API key not set")?;
        let _api_secret = self.api_secret.as_ref().ok_or("API secret not set")?;

        let sig = self.sign_params(&[
            ("api_key", api_key),
            ("method", "auth.getSession"),
            ("token", token),
        ]);

        let params = [
            ("method", "auth.getSession"),
            ("api_key", api_key.as_str()),
            ("token", token),
            ("api_sig", &sig),
            ("format", "json"),
        ];

        let resp = self.client.post(LASTFM_API_URL).form(&params).send().await
            .map_err(|e| format!("Request failed: {e}"))?;

        let json: serde_json::Value = resp.json().await.map_err(|e| format!("Invalid JSON: {e}"))?;

        if let Some(error) = json.get("error") {
            return Err(format!("Last.fm error: {}", error));
        }

        let session_key = json.get("session")
            .and_then(|s| s.get("key"))
            .and_then(|k| k.as_str())
            .ok_or("No session key in response")?;

        *self.session_key.lock().await = Some(session_key.to_string());
        info!("Last.fm authenticated successfully");
        Ok(session_key.to_string())
    }

    /// Update "now playing" status on Last.fm.
    /// Throttled to once per minute per Last.fm API guidelines.
    pub async fn update_now_playing(&self, track: &TrackInfo) -> Result<(), String> {
        if !self.is_ready() {
            return Err("Last.fm not configured".into());
        }

        // Throttle: Last.fm recommends max 1 update per minute
        let mut last_np = self.last_now_playing.lock().await;
        if let Some(last) = *last_np {
            if last.elapsed() < Duration::from_secs(60) {
                return Ok(()); // Skip throttled update
            }
        }
        *last_np = Some(Instant::now());
        drop(last_np);

        let session_key = self.session_key.lock().await.clone()
            .ok_or("No Last.fm session")?;

        let track_name = track.title.clone();
        let artist = track.artist.clone();
        let album = track.album.clone();

        let sig = self.sign_params(&[
            ("api_key", self.api_key.as_ref().unwrap()),
            ("artist", &artist),
            ("track", &track_name),
            ("album", &album),
            ("method", "track.updateNowPlaying"),
            ("sk", &session_key),
        ]);

        let params = [
            ("method", "track.updateNowPlaying"),
            ("api_key", self.api_key.as_ref().unwrap()),
            ("artist", &artist),
            ("track", &track_name),
            ("album", &album),
            ("sk", &session_key),
            ("api_sig", &sig),
            ("format", "json"),
        ];

        let resp = self.client.post(LASTFM_API_URL).form(&params).send().await
            .map_err(|e| format!("Request failed: {e}"))?;

        let json: serde_json::Value = resp.json().await.map_err(|e| format!("Invalid JSON: {e}"))?;

        if let Some(error) = json.get("error") {
            warn!("Last.fm now playing failed: {}", error);
            return Err(error.to_string());
        }

        debug!("Last.fm now playing updated: {} - {}", artist, track_name);
        Ok(())
    }

    /// Scrobble a track to Last.fm.
    /// Only scrobbles if track meets minimum play criteria.
    pub async fn scrobble(&self, track: &TrackInfo, played_secs: f64, min_secs: u32, min_pct: f32) -> Result<(), String> {
        if !self.is_ready() {
            return Err("Last.fm not configured".into());
        }

        let duration = track.duration.max(1.0);
        let pct_played = played_secs / duration;

        if played_secs < min_secs as f64 && pct_played < min_pct as f64 {
            debug!("Track didn't meet scrobble criteria: {:.0}s/{:.0}s ({:.0}%)", played_secs, duration, pct_played * 100.0);
            return Ok(());
        }

        // Throttle: avoid rapid successive scrobbles
        let mut last_scrobble = self.last_scrobble.lock().await;
        if let Some(last) = *last_scrobble {
            if last.elapsed() < Duration::from_secs(5) {
                return Ok(()); // Skip if too recent
            }
        }
        *last_scrobble = Some(Instant::now());
        drop(last_scrobble);

        let session_key = self.session_key.lock().await.clone()
            .ok_or("No Last.fm session")?;

        let track_name = track.title.clone();
        let artist = track.artist.clone();
        let album = track.album.clone();
        let timestamp = chrono::Utc::now().timestamp();

        let sig = self.sign_params(&[
            ("api_key", self.api_key.as_ref().unwrap()),
            ("artist", &artist),
            ("track", &track_name),
            ("album", &album),
            ("method", "track.scrobble"),
            ("sk", &session_key),
            ("timestamp", &timestamp.to_string()),
        ]);

        let params = [
            ("method", "track.scrobble"),
            ("api_key", self.api_key.as_ref().unwrap()),
            ("artist", &artist),
            ("track", &track_name),
            ("album", &album),
            ("sk", &session_key),
            ("timestamp", &timestamp.to_string()),
            ("api_sig", &sig),
            ("format", "json"),
        ];

        let resp = self.client.post(LASTFM_API_URL).form(&params).send().await
            .map_err(|e| format!("Request failed: {e}"))?;

        let json: serde_json::Value = resp.json().await.map_err(|e| format!("Invalid JSON: {e}"))?;

        if let Some(error) = json.get("error") {
            warn!("Last.fm scrobble failed: {}", error);
            return Err(error.to_string());
        }

        info!("Last.fm scrobbled: {} - {}", artist, track_name);
        Ok(())
    }

    /// Clear the stored session (logout).
    pub async fn clear_session(&mut self) {
        *self.session_key.lock().await = None;
        info!("Last.fm session cleared");
    }

    /// Generate API signature for Last.fm request.
    fn sign_params(&self, params: &[(&str, &str)]) -> String {
        let api_secret = self.api_secret.as_ref().unwrap();
        let mut sorted: Vec<(&str, &str)> = params.iter().copied().collect();
        sorted.sort_by_key(|(k, _)| *k);
        
        let mut sig_string = String::new();
        for (k, v) in sorted {
            sig_string.push_str(k);
            sig_string.push_str(v);
        }
        sig_string.push_str(api_secret);

        let mut ctx = Context::new();
        ctx.consume(sig_string.as_bytes());
        let digest = ctx.finalize();
        hex::encode(digest.0)
    }

    pub fn get_api_key(&self) -> Option<String> {
        self.api_key.clone()
    }

    pub fn get_session_key(&self) -> Option<String> {
        self.session_key.blocking_lock().clone()
    }
}