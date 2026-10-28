// Copyright (c) 2026
// Author: prjctimg <prjctimg@outlook.com>

use async_trait::async_trait;
use gtm::providers::external::{
    ExternalProvider, PROVIDER_SPECS, ProviderPlaylist, ProviderSpec, ProviderStream, ProviderTrack,
};
use reqwest::blocking::Client;
use serde_json::Value;

use crate::config::ProviderConfig;
use crate::providers::external::ytmp::{self, YtAuth};

fn spec(id: &str) -> &'static ProviderSpec {
    PROVIDER_SPECS.iter().find(|s| s.id == id).unwrap()
}

pub struct NetEaseProvider {
    spec: &'static ProviderSpec,
    auth: YtAuth,
    enabled: bool,
    user_id: Option<String>,
}

impl NetEaseProvider {
    pub fn new(cfg: &ProviderConfig) -> Self {
        let id = "netease";
        let spec = spec(id);
        Self {
            spec,
            auth: YtAuth::from_config(cfg, id),
            enabled: cfg.enabled(id),
            user_id: cfg.string(id, "user_id"),
        }
    }

    fn api_get(&self, path: &str, q: &[(&str, &str)]) -> Result<Value, String> {
        let client = Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .map_err(|e| e.to_string())?;
        let mut url = format!("https://music.163.com{}", path);
        if !q.is_empty() {
            url.push('?');
            url.push_str(
                &q.iter()
                    .map(|(k, v)| format!("{}={}", urlencoding::encode(k), urlencoding::encode(v)))
                    .collect::<Vec<_>>()
                    .join("&"),
            );
        }
        let resp = client
            .get(&url)
            .header("User-Agent", "Mozilla/5.0")
            .header("Referer", "https://music.163.com/")
            .send()
            .map_err(|e| e.to_string())?;
        if !resp.status().is_success() {
            return Err(format!("netease http {}", resp.status()));
        }
        resp.json::<Value>().map_err(|e| e.to_string())
    }

    fn songs_to_tracks(&self, songs: &Vec<Value>) -> Vec<ProviderTrack> {
        let mut out = Vec::new();
        for s in songs {
            let id = s.get("id").and_then(|v| v.as_i64()).unwrap_or(0);
            if id == 0 {
                continue;
            }
            let title = s
                .get("name")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            if title.is_empty() {
                continue;
            }
            let artist = s
                .get("artists")
                .and_then(|a| a.as_array())
                .and_then(|arr| arr.first())
                .and_then(|a| a.get("name"))
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let album = s
                .get("album")
                .and_then(|a| a.get("name"))
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let dur_ms = s.get("duration").and_then(|v| v.as_i64()).unwrap_or(0);
            let url = format!("https://music.163.com/#/song?id={}", id);
            out.push(ProviderTrack {
                provider: self.spec.id.to_string(),
                id: url,
                title,
                artist,
                album,
                duration_secs: dur_ms as f64 / 1000.0,
                ..Default::default()
            });
        }
        out
    }
}

#[async_trait]
impl ExternalProvider for NetEaseProvider {
    fn spec(&self) -> &'static ProviderSpec {
        self.spec
    }

    fn available(&self) -> bool {
        self.enabled
    }

    fn detail(&self) -> Option<String> {
        if !self.enabled {
            return Some("not configured \u{2014} run gtm setup netease".to_string());
        }
        self.user_id.as_ref().map(|u| format!("user: {u}"))
    }

    async fn search(&self, query: &str, limit: usize) -> Result<Vec<ProviderTrack>, String> {
        if !self.enabled {
            return Err("NetEase is not configured".to_string());
        }
        let q = query.trim();
        if q.is_empty() {
            return Ok(Vec::new());
        }
        let res = self.api_get(
            "/api/search/get/web",
            &[
                ("s", q),
                ("type", "1"),
                ("limit", &format!("{}", limit.clamp(1, 50))),
                ("offset", "0"),
            ],
        )?;
        if res.get("code").and_then(|v| v.as_i64()).unwrap_or(-1) != 200 {
            return Err("netease search failed".to_string());
        }
        let songs = res
            .get("result")
            .and_then(|r| r.get("songs"))
            .and_then(|s| s.as_array())
            .cloned()
            .unwrap_or_default();
        Ok(self.songs_to_tracks(&songs))
    }

    async fn playlists(&self) -> Result<Vec<ProviderPlaylist>, String> {
        if !self.enabled {
            return Err("NetEase is not configured".to_string());
        }
        let mut out = Vec::new();
        for (id, name) in [
            ("3778678", "Hot Songs"),
            ("3779629", "New Songs"),
            ("19723756", "Rising Songs"),
            ("2884035", "Original Songs"),
        ] {
            out.push(ProviderPlaylist {
                provider: self.spec.id.to_string(),
                id: format!("chart:{}", id),
                name: name.to_string(),
                editable: false,
                ..Default::default()
            });
        }
        if let Some(uid) = &self.user_id
            && let Ok(res) = self.api_get(
                "/api/user/playlist",
                &[("uid", uid), ("limit", "100"), ("offset", "0")],
            )
            && res.get("code").and_then(|v| v.as_i64()).unwrap_or(-1) == 200
            && let Some(arr) = res.get("playlist").and_then(|p| p.as_array())
        {
            for item in arr {
                let pid = item.get("id").and_then(|v| v.as_i64()).unwrap_or(0);
                if pid == 0 {
                    continue;
                }
                let name = item
                    .get("name")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                if name.is_empty() {
                    continue;
                }
                let count = item.get("trackCount").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
                out.push(ProviderPlaylist {
                    provider: self.spec.id.to_string(),
                    id: format!("user:{}", pid),
                    name,
                    track_count: count,
                    editable: false,
                    ..Default::default()
                });
            }
        }
        Ok(out)
    }

    async fn playlist_tracks(&self, id: &str) -> Result<Vec<ProviderTrack>, String> {
        if !self.enabled {
            return Err("NetEase is not configured".to_string());
        }
        let pid = if let Some(x) = id.strip_prefix("chart:") {
            x
        } else if let Some(x) = id.strip_prefix("user:") {
            x
        } else if id.starts_with("http") {
            return ytmp::flat_playlist(&self.auth, self.spec.id, id, 200).await;
        } else {
            id
        };
        let res = self.api_get("/api/playlist/detail", &[("id", pid)])?;
        if res.get("code").and_then(|v| v.as_i64()).unwrap_or(-1) != 200 {
            return Err("netease playlist detail failed".to_string());
        }
        let songs = res
            .get("result")
            .and_then(|r| r.get("tracks"))
            .and_then(|s| s.as_array())
            .cloned()
            .unwrap_or_default();
        Ok(self.songs_to_tracks(&songs))
    }

    async fn resolve(&self, id: &str, _title: &str) -> Result<ProviderStream, String> {
        if !self.enabled {
            return Err("NetEase is not configured".to_string());
        }
        let direct = ytmp::direct_url(&self.auth, id).await?;
        Ok(ProviderStream {
            url: direct,
            live: false,
        })
    }
}
