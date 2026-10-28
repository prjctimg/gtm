// Copyright (c) 2026
// Author: prjctimg <prjctimg@outlook.com>

use async_trait::async_trait;
use gtm::providers::external::{
    ExternalProvider, PROVIDER_SPECS, ProviderSpec, ProviderStream, ProviderTrack,
};

use crate::config::ProviderConfig;
use crate::providers::external::ytmp::{self, YtAuth};

fn spec(id: &str) -> &'static ProviderSpec {
    PROVIDER_SPECS.iter().find(|s| s.id == id).unwrap()
}

pub struct BilibiliProvider {
    spec: &'static ProviderSpec,
    auth: YtAuth,
    enabled: bool,
}

impl BilibiliProvider {
    pub fn new(cfg: &ProviderConfig) -> Self {
        let id = "bilibili";
        let spec = spec(id);
        Self {
            spec,
            auth: YtAuth::from_config(cfg, id),
            enabled: cfg.enabled(id),
        }
    }
}

#[async_trait]
impl ExternalProvider for BilibiliProvider {
    fn spec(&self) -> &'static ProviderSpec {
        self.spec
    }

    fn available(&self) -> bool {
        self.enabled
    }

    fn detail(&self) -> Option<String> {
        if !self.enabled {
            return Some("not configured \u{2014} run gtm setup bilibili".to_string());
        }
        None
    }

    async fn search(&self, query: &str, limit: usize) -> Result<Vec<ProviderTrack>, String> {
        if !self.enabled {
            return Err("Bilibili is not configured".to_string());
        }
        let q = query.trim();
        if q.is_empty() {
            return Ok(Vec::new());
        }
        let url = format!("bilisearch{}:{}", limit.clamp(1, 50), q);
        ytmp::search(&self.auth, self.spec.id, &url, limit).await
    }

    async fn resolve(&self, id: &str, _title: &str) -> Result<ProviderStream, String> {
        if !self.enabled {
            return Err("Bilibili is not configured".to_string());
        }
        let direct = ytmp::direct_url(&self.auth, id).await?;
        Ok(ProviderStream {
            url: direct,
            live: false,
        })
    }
}
