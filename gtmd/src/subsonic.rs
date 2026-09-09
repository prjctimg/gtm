// Copyright (c) 2026
// Author: prjctimg <prjctimg@outlook.com>
// Subsonic API client for Navidrome and compatible servers
//
// This is free software released under the GPL-3.0 license.

use std::path::PathBuf;
use std::time::Duration;

use base64::Engine;
use tracing::{info, warn};

use gtm_core::subsonic::{
    SubsonicAlbum, SubsonicArtist, SubsonicSearchResults, SubsonicStatus, SubsonicTrack,
};

const CONFIG_FILE: &str = "subsonic.json";
const CONFIG_PERMS: u32 = 0o600;
const API_VERSION: &str = "1.16.1";
const CLIENT_NAME: &str = "gtm";

/// Stored credentials for a Subsonic server.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SubsonicConfig {
    pub server: String,
    pub username: String,
    pub password: String,
}

/// Owns the Subsonic REST client. Credentials live in a 0600 JSON file inside
/// the daemon config directory; authentication uses the Subsonic token scheme
/// (`t` = md5-hex(password + salt), `s` = per-request salt) so the plaintext
/// password is never transmitted. Playback uses native HTTP streaming of the
/// `/rest/stream` endpoint.
pub struct SubsonicManager {
    config_dir: PathBuf,
    client: reqwest::Client,
    config: Option<SubsonicConfig>,
    error: Option<String>,
}

impl SubsonicManager {
    pub fn new(config_dir: PathBuf) -> Self {
        Self {
            config_dir,
            client: reqwest::Client::builder()
                .user_agent(concat!("gtm/", env!("CARGO_PKG_VERSION")))
                .timeout(Duration::from_secs(20))
                .build()
                .unwrap_or_default(),
            config: None,
            error: None,
        }
    }

    fn config_path(&self) -> PathBuf {
        self.config_dir.join(CONFIG_FILE)
    }

    pub fn configured(&self) -> bool {
        self.config.is_some()
    }

    /// Load persisted credentials at daemon startup.
    pub fn load(&mut self) {
        let Ok(raw) = std::fs::read_to_string(self.config_path()) else {
            return;
        };
        match serde_json::from_str::<SubsonicConfig>(&raw) {
            Ok(cfg) => {
                self.config = Some(cfg);
                info!("loaded subsonic config for {}", self.config.as_ref().unwrap().server);
            }
            Err(e) => warn!("ignoring corrupt subsonic config: {e}"),
        }
    }

    /// Configure the server, validate the credentials with a ping, then persist.
    pub async fn set_config(
        &mut self,
        server: String,
        username: String,
        password: String,
    ) -> Result<(), String> {
        let cfg = SubsonicConfig {
            server: server.trim().trim_end_matches('/').to_string(),
            username: username.trim().to_string(),
            password,
        };
        if cfg.server.trim().is_empty() || cfg.username.trim().is_empty() || cfg.password.is_empty() {
            return Err("server, username, and password are all required".into());
        }
        let probe = self.request(&cfg, "ping", &[]).await?;
        if probe.get("status").and_then(|v| v.as_str()) != Some("ok") {
            return Err("server did not answer ping".into());
        }
        self.config = Some(cfg.clone());
        self.error = None;
        gtm_core::secret::set_secret(gtm_core::secret::SUBSONIC_KEY, &serde_json::to_string(&cfg).unwrap_or_default());
        self.save_config(&cfg)
    }

    fn save_config(&self, cfg: &SubsonicConfig) -> Result<(), String> {
        use std::os::unix::fs::PermissionsExt;
        let raw = serde_json::to_string_pretty(cfg).map_err(|e| e.to_string())?;
        std::fs::write(self.config_path(), raw).map_err(|e| format!("write: {e}"))?;
        let _ = std::fs::set_permissions(self.config_path(), std::fs::Permissions::from_mode(CONFIG_PERMS));
        Ok(())
    }

    /// Forget the configured server and drop stored credentials.
    pub fn clear(&mut self) {
        self.config = None;
        self.error = None;
        let _ = std::fs::remove_file(self.config_path());
        gtm_core::secret::delete_secret(gtm_core::secret::SUBSONIC_KEY);
    }

    pub fn status(&self) -> SubsonicStatus {
        SubsonicStatus {
            configured: self.config.is_some(),
            server: self.config.as_ref().map(|c| c.server.clone()),
            user: self.config.as_ref().map(|c| c.username.clone()),
            error: self.error.clone(),
        }
    }

    /// Verify connectivity with the current config.
    pub async fn ping(&self) -> Result<(), String> {
        let cfg = self.require()?;
        let resp = self.request(cfg, "ping", &[]).await?;
        match resp.get("status").and_then(|v| v.as_str()) {
            Some("ok") => Ok(()),
            _ => Err(format!("ping failed: {resp:?}")),
        }
    }

    pub async fn search(&mut self, query: &str) -> Result<SubsonicSearchResults, String> {
        let cfg = self.require()?;
        let resp = self.request(cfg, "search3", &[("query", query), ("artistCount", "20"), ("albumCount", "20"), ("songCount", "40")]).await?;
        let body = resp
            .get("searchResult3")
            .cloned()
            .unwrap_or_else(|| serde_json::json!({}));
        Ok(SubsonicSearchResults {
            artists: parse_artists(body.get("artist")),
            albums: parse_albums(body.get("album")),
            tracks: parse_tracks(body.get("song")),
        })
    }

    /// Recent/alpha albums: `getAlbumList2?type=alphabetical`.
    pub async fn albums(&self, offset: u64, size: u64) -> Result<Vec<SubsonicAlbum>, String> {
        let cfg = self.require()?;
        let resp = self
            .request(
                cfg,
                "getAlbumList2",
                &[
                    ("type", "alphabetical"),
                    ("offset", &offset.to_string()),
                    ("size", &size.to_string()),
                ],
            )
            .await?;
        let list = resp
            .get("albumList2")
            .and_then(|v| v.get("album"))
            .cloned()
            .unwrap_or_else(|| serde_json::json!([]));
        Ok(parse_albums(Some(&list)))
    }

    /// Tracks within one album.
    pub async fn album_tracks(&self, album_id: &str) -> Result<Vec<SubsonicTrack>, String> {
        let cfg = self.require()?;
        let resp = self.request(cfg, "getAlbum", &[("id", album_id)]).await?;
        let list = resp
            .get("album")
            .and_then(|v| v.get("song"))
            .cloned()
            .unwrap_or_else(|| serde_json::json!([]));
        Ok(parse_tracks(Some(&list)))
    }

    /// Build a playable `/rest/stream` URL with fresh authentication params.
    pub fn stream_url(&self, track_id: &str) -> Result<String, String> {
        let cfg = self.require()?;
        let mut params = self.auth_params(cfg);
        params.push(("id".to_string(), track_id.to_string()));
        params.push(("maxBitRate".to_string(), "0".to_string()));
        Ok(format!("{}/rest/stream?{}", cfg.server, encode_params(&params)))
    }

    /// Fetch cover art bytes for a track/album, base64-encoded.
    pub async fn cover_base64(&self, track_id: &str, size: u32) -> Option<String> {
        let cfg = self.require().ok()?;
        let mut params = self.auth_params(cfg);
        params.push(("id".to_string(), track_id.to_string()));
        params.push(("size".to_string(), size.to_string()));
        let url = format!("{}/rest/getCoverArt?{}", cfg.server, encode_params(&params));
        let bytes = self.client.get(&url).send().await.ok()?.bytes().await.ok()?;
        (!bytes.is_empty()).then(|| base64::engine::general_purpose::STANDARD.encode(&bytes))
    }

    fn require(&self) -> Result<&SubsonicConfig, String> {
        self.config
            .as_ref()
            .ok_or_else(|| "subsonic not configured (use `set-config`) ".to_string())
    }

    /// Build the Subsonic token-auth query params for one request.
    fn auth_params(&self, cfg: &SubsonicConfig) -> Vec<(String, String)> {
        let salt = format!("{:08x}", fastrand::u64(..));
        let digest = md5::compute(format!("{}{}", cfg.password, salt));
        vec![
            ("u".to_string(), cfg.username.clone()),
            ("t".to_string(), format!("{digest:x}")),
            ("s".to_string(), salt),
            ("v".to_string(), API_VERSION.to_string()),
            ("c".to_string(), CLIENT_NAME.to_string()),
            ("f".to_string(), "json".to_string()),
        ]
    }

    /// Perform a Subsonic API request and return the `subsonic-response` object.
    async fn request(
        &self,
        cfg: &SubsonicConfig,
        method: &str,
        extra: &[(&str, &str)],
    ) -> Result<serde_json::Value, String> {
        let mut params = self.auth_params(cfg);
        for (k, v) in extra {
            params.push((k.to_string(), v.to_string()));
        }
        let url = format!("{}/rest/{}?{}", cfg.server, method, encode_params(&params));
        let resp = self
            .client
            .get(&url)
            .send()
            .await
            .map_err(|e| format!("{method}: {e}"))?;
        if !resp.status().is_success() {
            return Err(format!("{method}: HTTP {}", resp.status()));
        }
        let json: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| format!("{method}: bad JSON: {e}"))?;
        let sr = json
            .get("subsonic-response")
            .ok_or_else(|| format!("{method}: missing subsonic-response"))?;
        if let Some(code) = sr.get("error").and_then(|e| e.get("code")).and_then(|c| c.as_i64()) {
            let msg = sr
                .get("error")
                .and_then(|e| e.get("message"))
                .and_then(|m| m.as_str())
                .unwrap_or("unknown error");
            let friendly = match code {
                40 => "wrong username or password".to_string(),
                41 => format!("token authentication failed ({msg})"),
                _ => format!("server error {code}: {msg}"),
            };
            return Err(friendly);
        }
        Ok(sr.clone())
    }
}

fn parse_artists(v: Option<&serde_json::Value>) -> Vec<SubsonicArtist> {
    list(v)
        .iter()
        .filter_map(|a| {
            Some(SubsonicArtist {
                id: a.get("id")?.as_str()?.to_string(),
                name: a.get("name").and_then(|n| n.as_str()).unwrap_or("Unknown").to_string(),
            })
        })
        .collect()
}

fn parse_albums(v: Option<&serde_json::Value>) -> Vec<SubsonicAlbum> {
    list(v)
        .iter()
        .filter_map(|a| {
            Some(SubsonicAlbum {
                id: a.get("id")?.as_str()?.to_string(),
                title: a.get("title").and_then(|n| n.as_str()).unwrap_or("Unknown").to_string(),
                artist: a.get("artist").and_then(|n| n.as_str()).unwrap_or_default().to_string(),
                artist_id: a.get("artistId").and_then(|n| n.as_str()).unwrap_or_default().to_string(),
                year: a.get("year").and_then(|y| y.as_i64()),
                track_count: a.get("trackCount").and_then(|c| c.as_u64()).unwrap_or(0),
                cover_id: a.get("coverArt").and_then(|c| c.as_str()).map(|s| s.to_string()),
            })
        })
        .collect()
}

fn parse_tracks(v: Option<&serde_json::Value>) -> Vec<SubsonicTrack> {
    list(v)
        .iter()
        .filter_map(|t| {
            Some(SubsonicTrack {
                id: t.get("id")?.as_str()?.to_string(),
                title: t.get("title").and_then(|n| n.as_str()).unwrap_or("Unknown").to_string(),
                artist: t.get("artist").and_then(|n| n.as_str()).unwrap_or_default().to_string(),
                album: t.get("album").and_then(|n| n.as_str()).unwrap_or_default().to_string(),
                album_id: t.get("albumId").and_then(|n| n.as_str()).unwrap_or_default().to_string(),
                artist_id: t.get("artistId").and_then(|n| n.as_str()).unwrap_or_default().to_string(),
                duration_secs: t.get("duration").and_then(|d| d.as_u64()).unwrap_or(0),
                year: t.get("year").and_then(|y| y.as_i64()),
                cover_id: t.get("coverArt").and_then(|c| c.as_str()).map(|s| s.to_string()),
                suffix: t.get("suffix").and_then(|c| c.as_str()).unwrap_or_default().to_string(),
                bit_rate: t.get("bitRate").and_then(|b| b.as_u64()),
            })
        })
        .collect()
}

fn list(v: Option<&serde_json::Value>) -> Vec<&serde_json::Value> {
    match v {
        Some(serde_json::Value::Array(items)) => items.iter().collect(),
        Some(item) if !item.is_null() => vec![item],
        _ => Vec::new(),
    }
}

fn encode_params(params: &[(String, String)]) -> String {
    params
        .iter()
        .map(|(k, v)| format!("{}={}", k, urlencoding::encode(v)))
        .collect::<Vec<_>>()
        .join("&")
}