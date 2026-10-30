// Copyright (c) 2026
// Author: prjctimg <prjctimg@outlook.com>

use async_trait::async_trait;
use gtm::providers::external::{
    ExternalProvider, PROVIDER_SPECS, ProviderPlaylist, ProviderSpec, ProviderStream, ProviderTrack,
};
use md5;
use reqwest::blocking::Client;
use serde_json::Value;

use crate::config::ProviderConfig;

fn spec(id: &str) -> &'static ProviderSpec {
    PROVIDER_SPECS.iter().find(|s| s.id == id).unwrap()
}

#[derive(Clone)]
struct SubsonicConfig {
    base_url: String,
    user: String,
    password: String,
    token: Option<String>,
}

pub struct SubsonicProvider {
    spec: &'static ProviderSpec,
    cfg: Option<SubsonicConfig>,
    client: Client,
}

impl SubsonicProvider {
    pub fn new(id: &str, pcfg: &ProviderConfig) -> Self {
        let spec = spec(id);
        let mut base_url = pcfg
            .string(id, "url")
            .or_else(|| pcfg.string(id, "base_url"))
            .or_else(|| pcfg.string(id, "server"))
            .unwrap_or_default();
        if base_url.ends_with('/') {
            base_url.pop();
        }
        let user = pcfg
            .string(id, "user")
            .or_else(|| pcfg.string(id, "username"))
            .unwrap_or_default();
        let password = pcfg
            .string(id, "password")
            .or_else(|| pcfg.string(id, "pass"))
            .unwrap_or_default();
        let token = pcfg.string(id, "token");
        let cfg = if base_url.is_empty() || user.is_empty() {
            None
        } else {
            Some(SubsonicConfig {
                base_url,
                user,
                password,
                token,
            })
        };
        Self {
            spec,
            cfg,
            client: Client::builder()
                .timeout(std::time::Duration::from_secs(30))
                .build()
                .unwrap_or_default(),
        }
    }

    fn auth_params(&self, cfg: &SubsonicConfig) -> Vec<(String, String)> {
        let mut params = vec![
            ("u".to_string(), cfg.user.clone()),
            ("v".to_string(), "1.16.1".to_string()),
            ("c".to_string(), "gtm".to_string()),
            ("f".to_string(), "json".to_string()),
        ];
        if let Some(t) = &cfg.token
            && !t.is_empty()
        {
            // Try token auth first; some servers use token differently
            params.push(("t".to_string(), t.clone()));
            let salt: String = (0..6)
                .map(|_| format!("{:x}", fastrand::u8(..16)))
                .collect();
            params.push(("s".to_string(), salt));
            return params;
        }
        let salt: String = (0..6)
            .map(|_| format!("{:x}", fastrand::u8(..16)))
            .collect();
        let mut hasher = md5::Context::new();
        hasher.consume(format!("{}{}", cfg.password, salt));
        let token = format!("{:x}", hasher.finalize());
        params.push(("t".to_string(), token));
        params.push(("s".to_string(), salt));
        params
    }

    fn get_json(
        &self,
        cfg: &SubsonicConfig,
        endpoint: &str,
        extra: &[(&str, &str)],
    ) -> Result<Value, String> {
        let mut url = format!("{}/rest/{}", cfg.base_url, endpoint);
        let mut params = self.auth_params(cfg);
        for (k, v) in extra {
            params.push((k.to_string(), v.to_string()));
        }
        let query: String = params
            .iter()
            .map(|(k, v)| format!("{}={}", urlencoding::encode(k), urlencoding::encode(v)))
            .collect::<Vec<_>>()
            .join("&");
        url.push('?');
        url.push_str(&query);
        let resp = self.client.get(&url).send().map_err(|e| e.to_string())?;
        if !resp.status().is_success() {
            return Err(format!("subsonic http {}", resp.status()));
        }
        let v: Value = resp.json().map_err(|e| e.to_string())?;
        if let Some(err) = v.get("subsonic-response").and_then(|r| r.get("error")) {
            let msg = err
                .get("message")
                .and_then(|m| m.as_str())
                .unwrap_or("error");
            return Err(format!("subsonic: {msg}"));
        }
        Ok(v)
    }

    fn stream_url(&self, cfg: &SubsonicConfig, id: &str) -> String {
        let mut params = self.auth_params(cfg);
        params.push(("id".to_string(), id.to_string()));
        let query: String = params
            .iter()
            .map(|(k, v)| format!("{}={}", urlencoding::encode(k), urlencoding::encode(v)))
            .collect::<Vec<_>>()
            .join("&");
        format!("{}/rest/stream.view?{}", cfg.base_url, query)
    }
}

#[async_trait]
impl ExternalProvider for SubsonicProvider {
    fn spec(&self) -> &'static ProviderSpec {
        self.spec
    }

    fn available(&self) -> bool {
        self.cfg.is_some()
    }

    fn detail(&self) -> Option<String> {
        if self.cfg.is_none() {
            return Some(format!(
                "not configured \u{2014} run gtm setup {}",
                self.spec.id
            ));
        }
        self.cfg.as_ref().map(|c| c.base_url.clone())
    }

    async fn search(&self, query: &str, limit: usize) -> Result<Vec<ProviderTrack>, String> {
        let cfg = self.cfg.as_ref().ok_or("not configured")?;
        let q = query.trim();
        if q.is_empty() {
            return Ok(Vec::new());
        }
        let res = self.get_json(
            cfg,
            "search3.view",
            &[
                ("query", q),
                ("songCount", &format!("{}", limit.clamp(1, 50))),
            ],
        )?;
        let mut out = Vec::new();
        if let Some(songs) = res
            .get("subsonic-response")
            .and_then(|r| r.get("searchResult3"))
            .and_then(|s| s.get("song"))
            .and_then(|s| s.as_array())
        {
            for s in songs {
                let id = s
                    .get("id")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                if id.is_empty() {
                    continue;
                }
                let title = s
                    .get("title")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let artist = s
                    .get("artist")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let album = s
                    .get("album")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let dur = s.get("duration").and_then(|v| v.as_f64()).unwrap_or(0.0);
                out.push(ProviderTrack {
                    provider: self.spec.id.to_string(),
                    id,
                    title,
                    artist,
                    album,
                    duration_secs: dur,
                    ..Default::default()
                });
            }
        }
        Ok(out)
    }

    async fn playlists(&self) -> Result<Vec<ProviderPlaylist>, String> {
        let cfg = self.cfg.as_ref().ok_or("not configured")?;
        let res = self.get_json(cfg, "getPlaylists.view", &[])?;
        let mut out = Vec::new();
        if let Some(playlists) = res
            .get("subsonic-response")
            .and_then(|r| r.get("playlists"))
            .and_then(|p| p.get("playlist"))
            .and_then(|p| p.as_array())
        {
            for p in playlists {
                let id = p
                    .get("id")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                if id.is_empty() {
                    continue;
                }
                let name = p
                    .get("name")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let count = p.get("songCount").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
                out.push(ProviderPlaylist {
                    provider: self.spec.id.to_string(),
                    id,
                    name,
                    track_count: count,
                    editable: true,
                    ..Default::default()
                });
            }
        }
        Ok(out)
    }

    async fn playlist_tracks(&self, id: &str) -> Result<Vec<ProviderTrack>, String> {
        let cfg = self.cfg.as_ref().ok_or("not configured")?;
        let res = self.get_json(cfg, "getPlaylist.view", &[("id", id)])?;
        let mut out = Vec::new();
        if let Some(songs) = res
            .get("subsonic-response")
            .and_then(|r| r.get("playlist"))
            .and_then(|p| p.get("entry"))
            .and_then(|e| e.as_array())
        {
            for s in songs {
                let sid = s
                    .get("id")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                if sid.is_empty() {
                    continue;
                }
                let title = s
                    .get("title")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let artist = s
                    .get("artist")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let album = s
                    .get("album")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let dur = s.get("duration").and_then(|v| v.as_f64()).unwrap_or(0.0);
                out.push(ProviderTrack {
                    provider: self.spec.id.to_string(),
                    id: sid,
                    title,
                    artist,
                    album,
                    duration_secs: dur,
                    ..Default::default()
                });
            }
        }
        Ok(out)
    }

    async fn resolve(&self, id: &str, _title: &str) -> Result<ProviderStream, String> {
        let cfg = self.cfg.as_ref().ok_or("not configured")?;
        Ok(ProviderStream {
            url: self.stream_url(cfg, id),
            live: false,
        })
    }

    async fn playlist_add(&self, id: &str, track_ids: &[String]) -> Result<(), String> {
        let cfg = self.cfg.as_ref().ok_or("not configured")?;
        for tid in track_ids {
            let _ = self.get_json(
                cfg,
                "updatePlaylist.view",
                &[("playlistId", id), ("songIdToAdd", tid)],
            );
        }
        Ok(())
    }

    async fn playlist_remove(&self, id: &str, track_ids: &[String]) -> Result<(), String> {
        let cfg = self.cfg.as_ref().ok_or("not configured")?;
        for tid in track_ids {
            let _ = self.get_json(
                cfg,
                "updatePlaylist.view",
                &[("playlistId", id), ("songIndexToRemove", tid)],
            );
        }
        Ok(())
    }

    async fn playlist_delete(&self, id: &str) -> Result<(), String> {
        let cfg = self.cfg.as_ref().ok_or("not configured")?;
        let _ = self.get_json(cfg, "deletePlaylist.view", &[("id", id)]);
        Ok(())
    }
}
