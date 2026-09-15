// Copyright (c) 2026
// Author: prjctimg <prjctimg@outlook.com>
// Last.fm scrobbling and now-playing support
//
// This is free software released under the GPL-3.0 license.

use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use md5::Context;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;
use tracing::{debug, info, warn};

use gtm_core::track::TrackInfo;

const LASTFM_API_URL: &str = "https://ws.audioscrobbler.com/2.0/";

/// Upper bound on the number of failed scrobbles cached for later retry.
const MAX_RETRY_QUEUE: usize = 200;

/// A scrobble that failed transiently and is queued for a later flush.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct PendingScrobble {
    artist: String,
    track: String,
    album: String,
    timestamp: i64,
}

/// Manages Last.fm authentication, scrobbling, and track loving.
pub struct LastfmManager {
    client: Client,
    api_key: Option<String>,
    api_secret: Option<String>,
    session_key: Arc<Mutex<Option<String>>>,
    last_scrobble: Arc<Mutex<Option<(String, Instant)>>>,
    last_now_playing: Arc<Mutex<Option<(String, Instant)>>>,
    retry: Arc<Mutex<VecDeque<PendingScrobble>>>,
    retry_path: Option<PathBuf>,
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
            retry: Arc::new(Mutex::new(VecDeque::new())),
            retry_path: None,
        }
    }

    /// Set where the offline retry queue is persisted and load any cached
    /// scrobbles from a previous run so failures survive daemon restarts.
    pub fn set_retry_path(&mut self, path: PathBuf) {
        self.retry_path = Some(path.clone());
        if let Ok(data) = std::fs::read(&path)
            && let Ok(queued) = serde_json::from_slice::<VecDeque<PendingScrobble>>(&data)
        {
            let count = queued.len();
            *self.retry.blocking_lock() = queued;
            if count > 0 {
                info!("Last.fm: restored {count} queued scrobble(s)");
            }
        }
    }

    /// Persist a snapshot of the retry queue to disk (best effort).
    fn persist_snapshot(&self, snapshot: &VecDeque<PendingScrobble>) {
        let Some(path) = self.retry_path.as_ref() else {
            return;
        };
        if snapshot.is_empty() {
            let _ = std::fs::remove_file(path);
            return;
        }
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Ok(json) = serde_json::to_vec(snapshot) {
            let _ = std::fs::write(path, json);
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

    /// Check if Last.fm is configured and authenticated. Async so the
    /// session-key guard never needs a blocking lock inside the daemon's
    /// async command loop.
    pub async fn is_ready(&self) -> bool {
        self.api_key.is_some()
            && self.api_secret.is_some()
            && self.session_key.lock().await.is_some()
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

        let resp = self
            .client
            .post(LASTFM_API_URL)
            .form(&params)
            .send()
            .await
            .map_err(|e| format!("Request failed: {e}"))?;

        let json: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| format!("Invalid JSON: {e}"))?;

        if let Some(error) = json.get("error") {
            return Err(format!("Last.fm error: {}", error));
        }

        let session_key = json
            .get("session")
            .and_then(|s| s.get("key"))
            .and_then(|k| k.as_str())
            .ok_or("No session key in response")?;

        *self.session_key.lock().await = Some(session_key.to_string());
        info!("Last.fm authenticated successfully");
        Ok(session_key.to_string())
    }

    /// Update "now playing" status on Last.fm.
    /// Throttled per track to once per minute per Last.fm API guidelines, so
    /// skipping back to a recently played track still refreshes its now-playing.
    pub async fn update_now_playing(&self, track: &TrackInfo) -> Result<(), String> {
        if !self.is_ready().await {
            return Err("Last.fm not configured".into());
        }

        let key = format!("{}|{}", track.artist, track.title);
        let mut last_np = self.last_now_playing.lock().await;
        if let Some((last_key, last)) = last_np.as_ref()
            && *last_key == key
            && last.elapsed() < Duration::from_secs(60)
        {
            return Ok(()); // Skip throttled update for the same track
        }
        *last_np = Some((key, Instant::now()));
        drop(last_np);

        let session_key = self
            .session_key
            .lock()
            .await
            .clone()
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

        let resp = self
            .client
            .post(LASTFM_API_URL)
            .form(&params)
            .send()
            .await
            .map_err(|e| format!("Request failed: {e}"))?;

        let json: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| format!("Invalid JSON: {e}"))?;

        if let Some(error) = json.get("error") {
            warn!("Last.fm now playing failed: {}", error);
            return Err(error.to_string());
        }

        debug!("Last.fm now playing updated: {} - {}", artist, track_name);
        Ok(())
    }

    /// Scrobble a track to Last.fm.
    /// Only scrobbles if track meets minimum play criteria. Same-track
    /// duplicate scrobbles within a short window are suppressed, but distinct
    /// tracks are never dropped (a cross-track throttle would silently lose
    /// legitimate scrobbles during quick skip-ahead). Transient delivery
    /// failures are queued for a later retry instead of being dropped.
    pub async fn scrobble(
        &self,
        track: &TrackInfo,
        played_secs: f64,
        min_secs: u32,
        min_pct: f32,
    ) -> Result<(), String> {
        if !self.is_ready().await {
            return Err("Last.fm not configured".into());
        }

        let duration = track.duration.max(1.0);
        let pct_played = played_secs / duration;

        if played_secs < min_secs as f64 && pct_played < min_pct as f64 {
            debug!(
                "Track didn't meet scrobble criteria: {:.0}s/{:.0}s ({:.0}%)",
                played_secs,
                duration,
                pct_played * 100.0
            );
            return Ok(());
        }

        // Deduplicate: skip only when the very same track was just scrobbled.
        let key = format!("{}|{}|{}", track.artist, track.title, track.album);
        let mut last_scrobble = self.last_scrobble.lock().await;
        if let Some((last_key, last)) = last_scrobble.as_ref()
            && *last_key == key
            && last.elapsed() < Duration::from_secs(10)
        {
            debug!("track already scrobbled recently, skipping duplicate");
            return Ok(());
        }
        *last_scrobble = Some((key, Instant::now()));
        drop(last_scrobble);

        let artist = track.artist.clone();
        let title = track.title.clone();
        let album = track.album.clone();
        let timestamp = chrono::Utc::now().timestamp();

        match self.submit_scrobble(&artist, &title, &album, timestamp).await {
            Ok(()) => {
                info!("Last.fm scrobbled: {} - {}", artist, title);
                Ok(())
            }
            Err(e) if e.starts_with("Last.fm error") => {
                warn!("Last.fm scrobble failed: {e}");
                Err(e)
            }
            Err(e) => {
                // Transient failure (network/timeout/bad response): cache it.
                warn!("Last.fm scrobble queued for retry: {e}");
                self.enqueue_retry(PendingScrobble {
                    artist,
                    track: title,
                    album,
                    timestamp,
                })
                .await;
                Err(e)
            }
        }
    }

    /// Append a failed scrobble to the retry queue (capped, oldest dropped).
    async fn enqueue_retry(&self, item: PendingScrobble) {
        let mut retry = self.retry.lock().await;
        if retry.len() >= MAX_RETRY_QUEUE {
            retry.pop_front();
        }
        retry.push_back(item);
        let snapshot = retry.clone();
        drop(retry);
        self.persist_snapshot(&snapshot);
    }

    /// Retry any queued scrobbles. Successful entries are removed; entries that
    /// still fail on a transient error stay queued for the next flush. Called
    /// from a background task so playback is never blocked.
    pub async fn flush_retries(&self) {
        if !self.is_ready().await {
            return;
        }
        let to_try = {
            let retry = self.retry.lock().await;
            retry.iter().cloned().collect::<Vec<_>>()
        };
        let mut kept = VecDeque::new();
        for item in to_try {
            match self
                .submit_scrobble(&item.artist, &item.track, &item.album, item.timestamp)
                .await
            {
                Ok(()) => info!(
                    "Last.fm retried scrobble: {} - {}",
                    item.artist, item.track
                ),
                Err(e) if e.starts_with("Last.fm error") => {
                    warn!(
                        "Last.fm cached scrobble dropped ({e}): {} - {}",
                        item.artist, item.track
                    );
                }
                Err(e) => {
                    debug!("Last.fm scrobble still failing ({e}), kept for later");
                    kept.push_back(item);
                }
            }
        }
        {
            let mut retry = self.retry.lock().await;
            *retry = kept.clone();
        }
        self.persist_snapshot(&kept);
    }

    /// Love the current track on Last.fm.
    pub async fn love(&self, track: &TrackInfo) -> Result<(), String> {
        self.set_loved(track, true).await
    }

    /// Un-love the current track on Last.fm.
    pub async fn unlove(&self, track: &TrackInfo) -> Result<(), String> {
        self.set_loved(track, false).await
    }

    /// Submit `track.love` / `track.unlove` for the given track.
    async fn set_loved(&self, track: &TrackInfo, love: bool) -> Result<(), String> {
        if !self.is_ready().await {
            return Err("Last.fm not configured".into());
        }
        let session_key = self
            .session_key
            .lock()
            .await
            .clone()
            .ok_or("No Last.fm session")?;

        let track_name = track.title.clone();
        let artist = track.artist.clone();
        let method = if love { "track.love" } else { "track.unlove" };

        let sig = self.sign_params(&[
            ("api_key", self.api_key.as_ref().unwrap()),
            ("artist", &artist),
            ("track", &track_name),
            ("method", method),
            ("sk", &session_key),
        ]);

        let params = [
            ("method", method),
            ("api_key", self.api_key.as_ref().unwrap()),
            ("artist", &artist),
            ("track", &track_name),
            ("sk", &session_key),
            ("api_sig", &sig),
            ("format", "json"),
        ];

        let resp = self
            .client
            .post(LASTFM_API_URL)
            .form(&params)
            .send()
            .await
            .map_err(|e| format!("Request failed: {e}"))?;

        let json: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| format!("Invalid JSON: {e}"))?;

        if let Some(error) = json.get("error") {
            warn!("Last.fm {method} failed: {}", error);
            return Err(error.to_string());
        }

        info!("Last.fm {} - {} {}", method, artist, track_name);
        Ok(())
    }

    /// Raw scrobble submission for a resolved identity (used by both the
    /// criteria-gated path and the offline retry queue).
    async fn submit_scrobble(
        &self,
        artist: &str,
        track_name: &str,
        album: &str,
        timestamp: i64,
    ) -> Result<(), String> {
        let session_key = self
            .session_key
            .lock()
            .await
            .clone()
            .ok_or("No Last.fm session")?;

        let sig = self.sign_params(&[
            ("api_key", self.api_key.as_ref().unwrap()),
            ("artist", artist),
            ("track", track_name),
            ("album", album),
            ("method", "track.scrobble"),
            ("sk", &session_key),
            ("timestamp", &timestamp.to_string()),
        ]);

        let params = [
            ("method", "track.scrobble"),
            ("api_key", self.api_key.as_ref().unwrap()),
            ("artist", artist),
            ("track", track_name),
            ("album", album),
            ("sk", &session_key),
            ("timestamp", &timestamp.to_string()),
            ("api_sig", &sig),
            ("format", "json"),
        ];

        let resp = self
            .client
            .post(LASTFM_API_URL)
            .form(&params)
            .send()
            .await
            .map_err(|e| format!("Request failed: {e}"))?;

        let json: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| format!("Invalid JSON: {e}"))?;

        if let Some(error) = json.get("error") {
            return Err(format!("Last.fm error: {error}"));
        }

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
        let mut sorted: Vec<(&str, &str)> = params.to_vec();
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

    pub async fn get_session_key(&self) -> Option<String> {
        self.session_key.lock().await.clone()
    }
}
