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

pub struct MixcloudProvider {
    spec: &'static ProviderSpec,
    auth: YtAuth,
    enabled: bool,
    access_token: Option<String>,
    username: Option<String>,
}

impl MixcloudProvider {
    pub fn new(cfg: &ProviderConfig) -> Self {
        let id = "mixcloud";
        let spec = spec(id);
        Self {
            spec,
            auth: YtAuth::from_config(cfg, id),
            enabled: cfg.enabled(id),
            access_token: cfg.string(id, "access_token"),
            username: cfg.string(id, "username"),
        }
    }

    fn api_get(&self, path: &str, q: &[(&str, &str)]) -> Result<Value, String> {
        let client = Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .map_err(|e| e.to_string())?;
        let mut url = format!("https://api.mixcloud.com{}", path);
        let mut params: Vec<(String, String)> = vec![];
        if let Some(t) = &self.access_token {
            params.push(("access_token".to_string(), t.clone()));
        }
        for (k, v) in q {
            params.push((k.to_string(), v.to_string()));
        }
        if !params.is_empty() {
            url.push('?');
            url.push_str(
                &params
                    .iter()
                    .map(|(k, v)| format!("{}={}", urlencoding::encode(k), urlencoding::encode(v)))
                    .collect::<Vec<_>>()
                    .join("&"),
            );
        }
        let resp = client
            .get(&url)
            .header("User-Agent", "gtm")
            .send()
            .map_err(|e| e.to_string())?;
        if !resp.status().is_success() {
            return Err(format!("mixcloud http {}", resp.status()));
        }
        resp.json::<Value>().map_err(|e| e.to_string())
    }
}

#[async_trait]
impl ExternalProvider for MixcloudProvider {
    fn spec(&self) -> &'static ProviderSpec {
        self.spec
    }

    fn available(&self) -> bool {
        self.enabled
    }

    fn detail(&self) -> Option<String> {
        if !self.enabled {
            return Some("not configured \u{2014} run gtm setup mixcloud".to_string());
        }
        self.username.as_ref().map(|u| format!("user: {u}"))
    }

    async fn search(&self, query: &str, limit: usize) -> Result<Vec<ProviderTrack>, String> {
        if !self.enabled {
            return Err("Mixcloud is not configured".to_string());
        }
        let q = query.trim();
        if q.is_empty() {
            return Ok(Vec::new());
        }
        let limit_s = format!("{}", limit.clamp(1, 50));
        let res = self.api_get(
            "/search/",
            &[("q", q), ("type", "cloudcast"), ("limit", &limit_s)],
        )?;
        let mut out = Vec::new();
        if let Some(data) = res.get("data").and_then(|d| d.as_array()) {
            for item in data {
                let name = item
                    .get("name")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let url = item
                    .get("url")
                    .and_then(|v| v.as_str())
                    .map(|u| u.to_string())
                    .or_else(|| {
                        item.get("key")
                            .and_then(|k| k.as_str())
                            .map(|k| format!("https://www.mixcloud.com{}", k))
                    })
                    .unwrap_or_default();
                if name.is_empty() || url.is_empty() {
                    continue;
                }
                let artist = item
                    .get("user")
                    .and_then(|u| u.get("name").or_else(|| u.get("username")))
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let dur = item
                    .get("audio_length")
                    .and_then(|v| v.as_i64())
                    .unwrap_or(0) as f64;
                out.push(ProviderTrack {
                    provider: self.spec.id.to_string(),
                    id: url,
                    title: name,
                    artist,
                    duration_secs: dur,
                    ..Default::default()
                });
            }
        }
        Ok(out)
    }

    async fn playlists(&self) -> Result<Vec<ProviderPlaylist>, String> {
        if !self.enabled {
            return Err("Mixcloud is not configured".to_string());
        }
        let out = vec![
            ProviderPlaylist {
                provider: self.spec.id.to_string(),
                id: "discover:latest".to_string(),
                name: "Recent Releases".to_string(),
                editable: false,
                ..Default::default()
            },
            ProviderPlaylist {
                provider: self.spec.id.to_string(),
                id: "discover:popular".to_string(),
                name: "Popular".to_string(),
                editable: false,
                ..Default::default()
            },
        ];
        Ok(out)
    }

    async fn playlist_tracks(&self, id: &str) -> Result<Vec<ProviderTrack>, String> {
        if !self.enabled {
            return Err("Mixcloud is not configured".to_string());
        }
        if id.starts_with("http") {
            return ytmp::flat_playlist(&YtAuth::default(), self.spec.id, id, 200).await;
        }
        let path = match id {
            "discover:latest" => "/discover/all/latest/",
            "discover:popular" => "/discover/all/popular/",
            _ => "/discover/all/latest/",
        };
        let res = self.api_get(path, &[("limit", "100")])?;
        let mut out = Vec::new();
        if let Some(data) = res.get("data").and_then(|d| d.as_array()) {
            for item in data {
                let name = item
                    .get("name")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let url = item
                    .get("url")
                    .and_then(|v| v.as_str())
                    .map(|u| u.to_string())
                    .or_else(|| {
                        item.get("key")
                            .and_then(|k| k.as_str())
                            .map(|k| format!("https://www.mixcloud.com{}", k))
                    })
                    .unwrap_or_default();
                if name.is_empty() || url.is_empty() {
                    continue;
                }
                let artist = item
                    .get("user")
                    .and_then(|u| u.get("name").or_else(|| u.get("username")))
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let dur = item
                    .get("audio_length")
                    .and_then(|v| v.as_i64())
                    .unwrap_or(0) as f64;
                out.push(ProviderTrack {
                    provider: self.spec.id.to_string(),
                    id: url,
                    title: name,
                    artist,
                    duration_secs: dur,
                    ..Default::default()
                });
            }
        }
        Ok(out)
    }

    async fn resolve(&self, id: &str, _title: &str) -> Result<ProviderStream, String> {
        if !self.enabled {
            return Err("Mixcloud is not configured".to_string());
        }
        let direct = ytmp::direct_url(&self.auth, id).await?;
        Ok(ProviderStream {
            url: direct,
            live: false,
        })
    }
}
