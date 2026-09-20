// Copyright (c) 2026
// Author: prjctimg <prjctimg@outlook.com>
// Deezer search for enriching unreliable track metadata
//
// This is free software released under the GPL-3.0 license.

use std::time::{Duration, Instant};

use reqwest::Client;
use serde_json::Value;
use tracing::warn;

use gtm::shared::secret::DEEZER_ARL;

const DEEZER_API: &str = "https://api.deezer.com/search";
const DEEZER_ARTIST_API: &str = "https://api.deezer.com/artist";
const DEEZER_PUBLIC_TRACK: &str = "https://api.deezer.com/track";
/// Internal Deezer web gateway used by deftones (ARL session auth).
const GW_LIGHT: &str = "https://www.deezer.com/ajax/gw-light.php";
/// CDN token exchange that yields the encrypted stream URL for a track token.
const MEDIA_GET_URL: &str = "https://media.deezer.com/v1/get_url";
const RATE_LIMIT_MS: u64 = 200;
const REQUEST_TIMEOUT: Duration = Duration::from_secs(15);
/// Well-known dzr/deemix Blowfish secret folded into the per-track key.
const DZR_BLOWFISH_SECRET: &[u8; 16] = b"jo6aey6haid2Teih";
/// Re-login when the session is older than this (Deezer sessions last ~1h).
const LOGIN_TTL: Duration = Duration::from_secs(55 * 60);

/// Resolved track metadata from a Deezer search hit.
#[derive(Debug, Clone)]
pub struct DeezerTrack {
    pub title: String,
    pub artist: String,
    pub album: String,
    pub genre: Option<String>,
    pub year: Option<i32>,
    pub track_number: Option<i32>,
    pub duration: f64,
    pub cover_url: Option<String>,
    /// Deezer album id, usable for album-level cover reuse.
    pub album_id: Option<String>,
}

pub struct DeezerSearch {
    client: Client,
}

impl Default for DeezerSearch {
    fn default() -> Self {
        Self::new()
    }
}

impl DeezerSearch {
    pub fn new() -> Self {
        Self {
            client: Client::builder()
                .timeout(REQUEST_TIMEOUT)
                .build()
                .unwrap_or_else(|_| Client::new()),
        }
    }

    /// Search Deezer for a track and return the best matching result.
    ///
    /// Results are scored against the expected artist/title/duration; a hit is
    /// only accepted when the title matches and the overall score clears a
    /// minimum bar, so bogus lookalikes are not written back.
    pub async fn search(
        &self,
        artist: &str,
        title: &str,
        duration: f64,
    ) -> Result<Option<DeezerTrack>, String> {
        let query = build_query(artist, title);
        if query.is_empty() {
            return Ok(None);
        }

        tokio::time::sleep(Duration::from_millis(RATE_LIMIT_MS)).await;

        let resp = self
            .client
            .get(DEEZER_API)
            .query(&[("q", query)])
            .send()
            .await
            .map_err(|e| format!("Deezer request failed: {e}"))?;
        let json: Value = resp
            .json()
            .await
            .map_err(|e| format!("Deezer JSON parse failed: {e}"))?;
        let data = match json.get("data").and_then(|d| d.as_array()) {
            Some(d) if !d.is_empty() => d,
            _ => return Ok(None),
        };

        Ok(best_match(data, artist, title, duration))
    }

    /// Download cover art bytes for an album/cover URL.
    pub async fn download_cover(&self, url: &str) -> Option<Vec<u8>> {
        match self.client.get(url).send().await {
            Ok(r) => match r.bytes().await {
                Ok(b) if !b.is_empty() => Some(b.to_vec()),
                Ok(_) => None,
                Err(e) => {
                    warn!("Failed to read cover bytes from {url}: {e}");
                    None
                }
            },
            Err(e) => {
                warn!("Failed to download cover from {url}: {e}");
                None
            }
        }
    }

    /// Fetch an artist portrait image URL from Deezer, then download the bytes.
    pub async fn artist_image(&self, artist: &str) -> Option<Vec<u8>> {
        let artist = artist.trim();
        if artist.is_empty() {
            return None;
        }
        tokio::time::sleep(Duration::from_millis(RATE_LIMIT_MS)).await;
        let query = format!("artist:\"{artist}\"");
        let resp = match self
            .client
            .get(DEEZER_API)
            .query(&[("q", query.as_str())])
            .send()
            .await
        {
            Ok(r) => r,
            Err(e) => {
                warn!("Deezer search failed for artist '{artist}': {e}");
                return None;
            }
        };
        let json: Value = match resp.json().await {
            Ok(j) => j,
            Err(e) => {
                warn!("Deezer JSON parse failed for artist '{artist}': {e}");
                return None;
            }
        };
        let artist_id = json
            .get("data")
            .and_then(|d| d.as_array())
            .and_then(|arr| arr.first())
            .and_then(|r| r.get("artist"))
            .and_then(|a| a.get("id"))
            .and_then(|v| v.as_u64());
        let artist_id = match artist_id {
            Some(id) => id,
            None => return None,
        };
        tokio::time::sleep(Duration::from_millis(RATE_LIMIT_MS)).await;
        let url = format!("{DEEZER_ARTIST_API}/{artist_id}");
        let resp = match self.client.get(&url).send().await {
            Ok(r) => r,
            Err(e) => {
                warn!("Deezer artist request failed for id {artist_id}: {e}");
                return None;
            }
        };
        let json: Value = match resp.json().await {
            Ok(j) => j,
            Err(e) => {
                warn!("Deezer artist JSON parse failed for id {artist_id}: {e}");
                return None;
            }
        };
        let img_url = json
            .get("picture_big")
            .or_else(|| json.get("picture_medium"))
            .and_then(|v| v.as_str())
            .unwrap_or("");
        if img_url.is_empty() {
            return None;
        }
        self.download_cover(img_url).await
    }
}

/// Build a Deezer `q` parameter, quoted per-field for a tighter match.
///
/// The values are intentionally NOT percent-encoded here: the HTTP client
/// encodes the whole `q` value when building the URL, and pre-encoding would
/// double-encode it (breaking multi-byte titles in particular).
fn build_query(artist: &str, title: &str) -> String {
    let mut parts = Vec::new();
    let artist = artist.trim();
    let title = title.trim();
    if !artist.is_empty() {
        parts.push(format!("artist:\"{artist}\""));
    }
    if !title.is_empty() {
        parts.push(format!("track:\"{title}\""));
    }
    parts.join(" ")
}

/// Pick the highest-scoring result that plausibly matches the expected track.
fn best_match(results: &[Value], artist: &str, title: &str, duration: f64) -> Option<DeezerTrack> {
    let na = normalize(artist);
    let nt = normalize(title);

    let mut best: Option<(DeezerTrack, u32, bool)> = None;
    for r in results {
        let r_artist = r
            .pointer("/artist/name")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let r_title = r.get("title").and_then(|v| v.as_str()).unwrap_or("");
        let r_dur = r.get("duration").and_then(|v| v.as_f64()).unwrap_or(0.0);

        let ra = normalize(r_artist);
        let rt = normalize(r_title);
        let mut score = 0u32;

        if !na.is_empty() && !ra.is_empty() {
            if na == ra {
                score += 4;
            } else if contains_any(&na, &ra) {
                score += 2;
            }
        }
        let title_matched = !nt.is_empty() && !rt.is_empty() && {
            if nt == rt {
                score += 4;
                true
            } else if contains_any(&nt, &rt) {
                score += 2;
                true
            } else {
                false
            }
        };
        if duration > 0.0 && r_dur > 0.0 {
            let diff = (duration - r_dur).abs();
            if diff <= 3.0 {
                score += 3;
            } else if diff <= 10.0 {
                score += 2;
            } else if diff <= 20.0 {
                score += 1;
            }
        }

        if !title_matched {
            continue;
        }
        if best
            .as_ref()
            .is_none_or(|(_, best_score, _)| score > *best_score)
        {
            best = Some((to_deezer_track(r), score, title_matched));
        }
    }

    best.and_then(|(t, score, _)| (score >= 4).then_some(t))
}

fn to_deezer_track(r: &Value) -> DeezerTrack {
    let cover_url = r
        .get("album")
        .and_then(|a| a.get("cover_big"))
        .or_else(|| r.get("album").and_then(|a| a.get("cover_medium")))
        .or_else(|| r.get("cover_big"))
        .or_else(|| r.get("cover_medium"))
        .and_then(|c| c.as_str())
        .map(str::to_string);

    let genre = r
        .get("genres")
        .and_then(|g| g.get("data"))
        .and_then(|d| d.as_array())
        .and_then(|arr| arr.first())
        .and_then(|g| g.get("name"))
        .and_then(|n| n.as_str())
        .map(str::to_string);

    let year = r
        .get("release_date")
        .or_else(|| r.get("album").and_then(|a| a.get("release_date")))
        .and_then(|v| v.as_str())
        .and_then(|s| s.get(..4))
        .and_then(|y| y.parse::<i32>().ok());

    let track_number = r
        .get("track_position")
        .and_then(|v| v.as_i64())
        .map(|n| n as i32);

    DeezerTrack {
        title: r
            .get("title")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string(),
        artist: r
            .pointer("/artist/name")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string(),
        album: r
            .get("album")
            .and_then(|a| a.get("title"))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string(),
        genre,
        year,
        track_number,
        duration: r.get("duration").and_then(|v| v.as_f64()).unwrap_or(0.0),
        cover_url,
        album_id: r
            .get("album")
            .and_then(|a| a.get("id"))
            .and_then(|v| v.as_u64())
            .map(|n| n.to_string()),
    }
}

/// Lowercase alphanumerics/whitespace only, for lenient matching.
fn normalize(s: &str) -> String {
    s.chars()
        .filter(|c| c.is_alphanumeric() || c.is_whitespace())
        .flat_map(char::to_lowercase)
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// True when one normalized string contains the other and the contained
/// (shorter) one is long enough to be meaningful (avoids "Be" matching
/// "Beatles").
fn contains_any(a: &str, b: &str) -> bool {
    let (shorter, longer) = if a.len() <= b.len() { (a, b) } else { (b, a) };
    shorter.len() >= 3 && longer.contains(shorter)
}

/// Streaming backend for the Deezer provider: ARL-authenticated full-track
/// MP3_128 CDN URLs through the internal gw-light API (`song.getListData` +
/// `media.deezer.com/v1/get_url`) with a 30s-preview fallback through the
/// public `api.deezer.com` endpoint when no ARL is configured (or the full
/// stream cannot be resolved).
///
/// The full-track body is Blowfish-CBC-encrypted per 2048-byte chunk; the
/// per-track key (see [`blowfish_key`]) must be handed to the decode pipeline
/// alongside the URL so `BlowfishReader` can decrypt on the fly.
pub struct DeezerStream {
    client: Client,
    arl: Option<String>,
    api_token: Option<String>,
    sid: Option<String>,
    license_token: Option<String>,
    login_at: Option<Instant>,
}

/// A resolved Deezer track: the stream URL plus (for full tracks) the
/// Blowfish chunk-decryption key. `blowfish: None` marks the unencrypted
/// 30s preview fallback.
pub struct ResolvedDeezer {
    pub url: String,
    pub blowfish: Option<[u8; 16]>,
}

impl Default for DeezerStream {
    fn default() -> Self {
        Self::new()
    }
}

impl DeezerStream {
    pub fn new() -> Self {
        Self {
            client: Client::builder()
                .timeout(REQUEST_TIMEOUT)
                .build()
                .unwrap_or_else(|_| Client::new()),
            arl: None,
            api_token: None,
            sid: None,
            license_token: None,
            login_at: None,
        }
    }

    /// Load the persisted ARL from the OS keychain (file fallback) at daemon
    /// startup, so playback works across restarts without re-entering it.
    pub fn from_keyring() -> Self {
        let mut s = Self::new();
        if let Some(arl) = gtm::shared::secret::get_secret(DEEZER_ARL) {
            s.set_arl(&arl);
        }
        s
    }

    /// Set (non-empty) or clear (empty) the ARL token; clearing also drops
    /// any cached session so the next play re-authenticates.
    pub fn set_arl(&mut self, arl: &str) {
        let arl = arl.trim();
        self.arl = (!arl.is_empty()).then(|| arl.to_string());
        if self.arl.is_none() {
            self.api_token = None;
            self.sid = None;
            self.license_token = None;
            self.login_at = None;
        }
    }

    pub fn configured(&self) -> bool {
        self.arl.is_some()
    }

    /// Resolve a track to a playable URL: full MP3 when an ARL is configured,
    /// otherwise (or on full-stream failure) the unencrypted 30s preview.
    pub async fn resolve_track(&mut self, track_id: u64) -> Result<ResolvedDeezer, String> {
        if self.configured() {
            match self.full_track_url(track_id).await {
                Ok(url) => {
                    return Ok(ResolvedDeezer {
                        url,
                        blowfish: Some(blowfish_key(track_id)),
                    });
                }
                Err(e) => warn!("deezer full track failed ({e}); using 30s preview"),
            }
        }
        let url = self.preview_url(track_id).await?;
        Ok(ResolvedDeezer {
            url,
            blowfish: None,
        })
    }

    /// Authenticate the ARL against gw-light and capture the API/license
    /// tokens plus the session id.
    async fn login(&mut self) -> Result<(), String> {
        let arl = self
            .arl
            .as_deref()
            .ok_or_else(|| "Deezer ARL not configured".to_string())?;
        let resp = self
            .client
            .get(GW_LIGHT)
            .query(&[
                ("method", "deezer.getUserData"),
                ("input", "3"),
                ("api_version", "1.0"),
                ("api_token", ""),
            ])
            .header("Cookie", format!("arl={arl}"))
            .header("Origin", "https://www.deezer.com")
            .header("Referer", "https://www.deezer.com/")
            .send()
            .await
            .map_err(|e| format!("deezer login: {e}"))?;
        let body: Value = resp
            .json()
            .await
            .map_err(|e| format!("deezer login parse: {e}"))?;
        let res = body.get("results").cloned().unwrap_or(Value::Null);
        let check = res.get("checkForm").and_then(|v| v.as_str()).unwrap_or("");
        if check.is_empty() {
            return Err("Deezer ARL rejected (invalid or expired token)".to_string());
        }
        self.api_token = Some(check.to_string());
        self.sid = res
            .get("SESSION_ID")
            .or_else(|| res.get("SESSION"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        self.license_token = res
            .pointer("/USER/OPTIONS/license_token")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        self.login_at = Some(Instant::now());
        Ok(())
    }

    /// Re-login when the session has expired or no token is cached.
    async fn ensure_login(&mut self) -> Result<(), String> {
        let fresh = self
            .login_at
            .map(|t| t.elapsed() < LOGIN_TTL && self.api_token.is_some())
            .unwrap_or(false);
        if !fresh {
            self.login().await?;
        }
        if self.license_token.is_none() {
            return Err("Deezer login did not expose a license token".to_string());
        }
        Ok(())
    }

    /// Exchange a track id for its playable CDN track token.
    async fn track_token(&mut self, track_id: u64) -> Result<String, String> {
        self.ensure_login().await?;
        let api_token = self.api_token.clone().unwrap_or_default();
        let sid = self.sid.clone().unwrap_or_default();
        let mut req = self.client.post(GW_LIGHT).query(&[
            ("method", "song.getListData"),
            ("input", "3"),
            ("api_version", "1.0"),
            ("api_token", api_token.as_str()),
        ]);
        if !sid.is_empty() {
            req = req.query(&[("sid", sid.as_str())]);
        }
        let resp = req
            .header("Origin", "https://www.deezer.com")
            .header("Referer", "https://www.deezer.com/")
            .body(format!(r#"{{"sng_ids":[{track_id}]}}"#))
            .send()
            .await
            .map_err(|e| format!("deezer track token: {e}"))?;
        let body: Value = resp
            .json()
            .await
            .map_err(|e| format!("deezer track token parse: {e}"))?;
        let data = body
            .pointer("/results/data")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();
        let track = data.first().cloned().unwrap_or(Value::Null);
        for key in ["TRACK_TOKEN", "/FALLBACK/TRACK_TOKEN"] {
            if let Some(tok) = track.pointer(key).and_then(|v| v.as_str()) {
                if !tok.is_empty() {
                    return Ok(tok.to_string());
                }
            }
        }
        Err(format!(
            "no playable track token for Deezer track {track_id}"
        ))
    }

    /// Resolve a full-track CDN URL for `track_id` (MP3_128, Blowfish-crypted).
    async fn full_track_url(&mut self, track_id: u64) -> Result<String, String> {
        self.ensure_login().await?;
        let track_token = self.track_token(track_id).await?;
        let license_token = self.license_token.clone().unwrap_or_default();
        let payload = serde_json::json!({
            "license_token": license_token,
            "media": [{
                "type": "FULL",
                "formats": [{ "cipher": "BF_CBC_STRIPE", "format": "MP3_128" }],
            }],
            "track_tokens": [track_token],
        });
        let resp = self
            .client
            .post(MEDIA_GET_URL)
            .json(&payload)
            .send()
            .await
            .map_err(|e| format!("deezer get_url: {e}"))?;
        let body: Value = resp
            .json()
            .await
            .map_err(|e| format!("deezer get_url parse: {e}"))?;
        let data = body
            .pointer("/data")
            .or_else(|| body.pointer("/results/data"))
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();
        let entry = data.first().cloned().unwrap_or(Value::Null);
        if let Some(media) = entry.get("media").and_then(|m| m.as_array()) {
            for item in media {
                if let Some(sources) = item.get("sources").and_then(|s| s.as_array()) {
                    for src in sources {
                        if let Some(url) = src.get("url").and_then(|v| v.as_str()) {
                            if !url.is_empty() {
                                return Ok(url.to_string());
                            }
                        }
                    }
                }
            }
        }
        Err(format!("no stream source for Deezer track {track_id}"))
    }

    /// Fetch the public (ARL-free) 30s preview URL for `track_id`.
    async fn preview_url(&mut self, track_id: u64) -> Result<String, String> {
        let resp = self
            .client
            .get(format!("{DEEZER_PUBLIC_TRACK}/{track_id}"))
            .send()
            .await
            .map_err(|e| format!("deezer preview: {e}"))?;
        let body: Value = resp
            .json()
            .await
            .map_err(|e| format!("deezer preview parse: {e}"))?;
        if let Some(url) = body.get("preview").and_then(|v| v.as_str()) {
            if !url.is_empty() {
                return Ok(url.to_string());
            }
        }
        Err(format!("no 30s preview for Deezer track {track_id}"))
    }
}

/// Derive the Blowfish-CBC stream key for a Deezer track: the md5 hex digest
/// of the track id is XOR-folded against the fixed dzr secret, matching the
/// deezer-downloader/deemix convention shared by dzr, orpheusdl and deezpy.
pub fn blowfish_key(track_id: u64) -> [u8; 16] {
    let digest = md5::compute(track_id.to_string());
    let hex = format!("{digest:x}");
    let h = hex.as_bytes();
    let mut key = [0u8; 16];
    for i in 0..16 {
        key[i] = h[i] ^ h[i + 16] ^ DZR_BLOWFISH_SECRET[i];
    }
    key
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_normalize() {
        assert_eq!(
            normalize("Beautiful (feat. Camila)"),
            "beautiful feat camila"
        );
        assert_eq!(normalize("God's Plan"), "gods plan");
    }

    #[test]
    fn test_contains_any() {
        assert!(contains_any("beautiful feat camila", "beautiful"));
        assert!(contains_any("beautiful", "beautiful feat camila"));
        assert!(!contains_any("be", "beatles"));
    }

    #[test]
    fn best_match_hit() {
        let results = vec![json!({
            "id": 1,
            "title": "Beautiful (feat. Camila Cabello)",
            "artist": {"name": "Bazzi"},
            "album": {"title": "Cosmic Latte", "cover_big": "http://x/c.jpg"},
            "duration": 208,
            "track_position": 4,
            "release_date": "2018-07-13"
        })];
        let hit = best_match(&results, "Bazzi", "Beautiful feat. Camila", 207.0).unwrap();
        assert_eq!(hit.title, "Beautiful (feat. Camila Cabello)");
        assert_eq!(hit.artist, "Bazzi");
        assert_eq!(hit.year, Some(2018));
        assert_eq!(hit.track_number, Some(4));
        assert_eq!(hit.duration, 208.0);
        assert_eq!(hit.cover_url.as_deref(), Some("http://x/c.jpg"));
        assert_eq!(hit.album_id, None);
    }

    #[test]
    fn best_match_reject() {
        let results = vec![json!({
            "id": 2,
            "title": "Something Completely Different",
            "artist": {"name": "Random Band"},
            "album": {"title": "Other"},
            "duration": 240
        })];
        assert!(best_match(&results, "Bazzi", "Beautiful feat. Camila", 207.0).is_none());
    }

    #[test]
    fn best_match_empty() {
        assert!(best_match(&[], "Bazzi", "Beautiful", 0.0).is_none());
    }

    #[test]
    fn build_query_raw() {
        assert_eq!(
            build_query("Bazzi", "Beautiful feat. Camila"),
            "artist:\"Bazzi\" track:\"Beautiful feat. Camila\""
        );
        assert_eq!(
            build_query("Kygo, Avicii", "Forever Yours ⧸ Remix"),
            "artist:\"Kygo, Avicii\" track:\"Forever Yours ⧸ Remix\""
        );
        assert_eq!(build_query("", "Mama Africa"), "track:\"Mama Africa\"");
        assert_eq!(build_query("  ", ""), "");
    }
}
