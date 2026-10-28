// Copyright (c) 2026
// Author: prjctimg <prjctimg@outlook.com>

use async_trait::async_trait;
use gtm::providers::external::{
    ExternalProvider, PROVIDER_SPECS, ProviderPlaylist, ProviderSpec, ProviderStream, ProviderTrack,
};

use crate::config::ProviderConfig;
use crate::providers::external::ytmp::{self, YtAuth};

fn spec(id: &str) -> &'static ProviderSpec {
    PROVIDER_SPECS.iter().find(|s| s.id == id).unwrap()
}

pub struct SoundCloudProvider {
    spec: &'static ProviderSpec,
    auth: YtAuth,
    enabled: bool,
    user: Option<String>,
}

impl SoundCloudProvider {
    pub fn new(cfg: &ProviderConfig) -> Self {
        let id = "soundcloud";
        let spec = spec(id);
        Self {
            spec,
            auth: YtAuth::from_config(cfg, id),
            enabled: cfg.enabled(id),
            user: cfg.string(id, "user"),
        }
    }
}

#[async_trait]
impl ExternalProvider for SoundCloudProvider {
    fn spec(&self) -> &'static ProviderSpec {
        self.spec
    }

    fn available(&self) -> bool {
        self.enabled
    }

    fn detail(&self) -> Option<String> {
        if !self.enabled {
            return Some("not configured \u{2014} run gtm setup soundcloud".to_string());
        }
        self.user.as_ref().map(|u| format!("user: {u}"))
    }

    async fn search(&self, query: &str, limit: usize) -> Result<Vec<ProviderTrack>, String> {
        if !self.enabled {
            return Err("SoundCloud is not configured".to_string());
        }
        let q = query.trim();
        if q.is_empty() {
            return Ok(Vec::new());
        }
        let url = format!("scsearch{}:{}", limit.clamp(1, 50), q);
        ytmp::search(&self.auth, self.spec.id, &url, limit).await
    }

    async fn playlists(&self) -> Result<Vec<ProviderPlaylist>, String> {
        if !self.enabled {
            return Err("SoundCloud is not configured".to_string());
        }
        let mut out = Vec::new();
        if let Some(u) = &self.user {
            let base = format!("https://soundcloud.com/{}", u);
            out.push(ProviderPlaylist {
                provider: self.spec.id.to_string(),
                id: base.clone(),
                name: "Tracks".to_string(),
                editable: false,
                ..Default::default()
            });
            out.push(ProviderPlaylist {
                provider: self.spec.id.to_string(),
                id: format!("{}/likes", base),
                name: "Likes".to_string(),
                editable: false,
                ..Default::default()
            });
            out.push(ProviderPlaylist {
                provider: self.spec.id.to_string(),
                id: format!("{}/reposts", base),
                name: "Reposts".to_string(),
                editable: false,
                ..Default::default()
            });
        } else {
            for (name, q) in [
                ("Trending", "trending"),
                ("Hip-Hop", "hip hop"),
                ("Electronic", "electronic"),
                ("House", "house"),
                ("Lo-Fi", "lofi"),
                ("Indie", "indie"),
                ("Pop", "pop"),
            ] {
                out.push(ProviderPlaylist {
                    provider: self.spec.id.to_string(),
                    id: format!("scsearch50:{}", q),
                    name: name.to_string(),
                    editable: false,
                    ..Default::default()
                });
            }
        }
        Ok(out)
    }

    async fn playlist_tracks(&self, id: &str) -> Result<Vec<ProviderTrack>, String> {
        if !self.enabled {
            return Err("SoundCloud is not configured".to_string());
        }
        if id.starts_with("scsearch") {
            ytmp::search(&self.auth, self.spec.id, id, 200).await
        } else if id.starts_with("http") {
            ytmp::flat_playlist(&self.auth, self.spec.id, id, 200).await
        } else {
            ytmp::search(&self.auth, self.spec.id, &format!("scsearch50:{}", id), 200).await
        }
    }

    async fn resolve(&self, id: &str, _title: &str) -> Result<ProviderStream, String> {
        if !self.enabled {
            return Err("SoundCloud is not configured".to_string());
        }
        let direct = ytmp::direct_url(&self.auth, id).await?;
        Ok(ProviderStream {
            url: direct,
            live: false,
        })
    }
}
