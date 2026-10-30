// Copyright (c) 2026
// Author: prjctimg <prjctimg@outlook.com>

use async_trait::async_trait;
use gtm::providers::external::{
    ExternalProvider, ProviderPlaylist, ProviderSpec, ProviderStream, ProviderTrack,
};
use urlencoding::encode;

use crate::config::ProviderConfig;
use crate::providers::external::ytmp::{self, YtAuth};

pub struct YtDlpSearchProvider {
    spec: &'static ProviderSpec,
    auth: YtAuth,
    enabled: bool,
    search_prefix: String,
}

impl YtDlpSearchProvider {
    pub fn new(spec: &'static ProviderSpec, cfg: &ProviderConfig) -> Self {
        let enabled = cfg.enabled(spec.id);
        let search_prefix = match spec.id {
            "ytmusic" => "https://music.youtube.com/search?q=".to_string(),
            "soundcloud" => "scsearch".to_string(),
            "bilibili" => "bilisearch".to_string(),
            _ => "ytsearch".to_string(),
        };
        Self {
            spec,
            auth: YtAuth::from_config(cfg, spec.id),
            enabled,
            search_prefix,
        }
    }
}

#[async_trait]
impl ExternalProvider for YtDlpSearchProvider {
    fn spec(&self) -> &'static ProviderSpec {
        self.spec
    }

    fn available(&self) -> bool {
        self.enabled
    }

    fn detail(&self) -> Option<String> {
        if !self.enabled {
            return Some(format!(
                "not configured \u{2014} run gtm setup {}",
                self.spec.id
            ));
        }
        None
    }

    async fn search(&self, query: &str, limit: usize) -> Result<Vec<ProviderTrack>, String> {
        if !self.enabled {
            return Err(format!("{} is not configured", self.spec.name));
        }
        let q = query.trim();
        if q.is_empty() {
            return Ok(Vec::new());
        }
        match self.spec.id {
            "ytmusic" => {
                let url = format!("{}{}", self.search_prefix, encode(q));
                ytmp::search(&self.auth, self.spec.id, &url, limit).await
            }
            "soundcloud" => {
                let url = format!("{}{}:{}", self.search_prefix, limit.clamp(1, 50), q);
                ytmp::search(&self.auth, self.spec.id, &url, limit).await
            }
            "bilibili" => {
                let url = format!("{}{}:{}", self.search_prefix, limit.clamp(1, 50), q);
                ytmp::search(&self.auth, self.spec.id, &url, limit).await
            }
            _ => {
                let url = format!("ytsearch{}:{}", limit.clamp(1, 50), q);
                ytmp::search(&self.auth, self.spec.id, &url, limit).await
            }
        }
    }

    async fn playlists(&self) -> Result<Vec<ProviderPlaylist>, String> {
        if !self.enabled {
            return Err(format!("{} is not configured", self.spec.name));
        }
        if self.spec.id == "ytmusic" {
            return Ok(vec![ProviderPlaylist {
                provider: self.spec.id.to_string(),
                id: "https://music.youtube.com/playlist?list=LM".to_string(),
                name: "Liked Music".to_string(),
                editable: false,
                ..Default::default()
            }]);
        }
        Ok(Vec::new())
    }

    async fn playlist_tracks(&self, id: &str) -> Result<Vec<ProviderTrack>, String> {
        if !self.enabled {
            return Err(format!("{} is not configured", self.spec.name));
        }
        if id.starts_with("http") {
            return ytmp::flat_playlist(&self.auth, self.spec.id, id, 200).await;
        }
        ytmp::search(&self.auth, self.spec.id, id, 50).await
    }

    async fn resolve(&self, id: &str, _title: &str) -> Result<ProviderStream, String> {
        if !self.enabled {
            return Err(format!("{} is not configured", self.spec.name));
        }
        let direct = ytmp::direct_url(&self.auth, id).await?;
        Ok(ProviderStream {
            url: direct,
            live: false,
        })
    }
}
