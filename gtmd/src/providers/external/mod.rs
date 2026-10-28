// Copyright (c) 2026
// Author: prjctimg <prjctimg@outlook.com>
// Daemon-side registry for the external provider framework
//
// This is free software released under the GPL-3.0 license.

//! One registry for every catalogue provider.
//!
//! Concrete integrations land in this directory as siblings. The registry is
//! the only place that maps a wire id to a client, so an unknown id is a
//! precise error and a provider cannot be reached by guessing a path.

use std::collections::HashMap;
use std::sync::Arc;

use gtm::providers::external::{
    ExternalProvider, PROVIDER_SPECS, ProviderPlaylist, ProviderSource, ProviderStream,
    ProviderTrack, UnconfiguredProvider,
};

use crate::config::ProviderConfig;

/// All configured and unconfigured providers, in catalogue order.
pub struct ProviderRegistry {
    providers: HashMap<&'static str, Arc<dyn ExternalProvider>>,
    order: Vec<&'static str>,
}

impl ProviderRegistry {
    /// Build the registry from `config.toml`.
    ///
    /// Every catalogue entry gets a row. Concrete clients replace the
    /// placeholder as they land; until then the row is still visible and says
    /// why it cannot answer, which is what Setup and the source picker need.
    pub fn new(config: &ProviderConfig) -> Self {
        let mut providers: HashMap<&'static str, Arc<dyn ExternalProvider>> = HashMap::new();
        let mut order = Vec::with_capacity(PROVIDER_SPECS.len());
        for spec in PROVIDER_SPECS {
            let provider: Arc<dyn ExternalProvider> = if !config.enabled(spec.id) {
                Arc::new(UnconfiguredProvider::new(
                    spec,
                    format!("not configured \u{2014} run gtm setup {}", spec.id),
                ))
            } else {
                // The framework keeps a configured provider visible even while
                // its concrete client is still a separate integration. Falling
                // back to "unavailable" is better than silently dropping a
                // table the user just wrote.
                Arc::new(UnconfiguredProvider::new(
                    spec,
                    "configured, but this build has no client for it".to_string(),
                ))
            };
            providers.insert(spec.id, provider);
            order.push(spec.id);
        }
        Self { providers, order }
    }

    /// The source rows shown by the client.
    pub fn sources(&self) -> Vec<ProviderSource> {
        self.order
            .iter()
            .filter_map(|id| self.providers.get(id))
            .map(|p| ProviderSource::new(p.spec(), p.available(), p.detail()))
            .collect()
    }

    fn raw(&self, id: &str) -> Result<&Arc<dyn ExternalProvider>, String> {
        self.providers
            .get(id)
            .ok_or_else(|| format!("unknown provider {id:?}"))
    }

    /// Whether one provider can answer now, with the reason when it cannot.
    pub fn available(&self, id: &str) -> Result<(), String> {
        let p = self.raw(id)?;
        if p.available() {
            Ok(())
        } else {
            Err(p
                .detail()
                .unwrap_or_else(|| format!("{} is not configured", p.spec().name)))
        }
    }

    /// The client for an id, refusing a provider whose Setup is unfinished.
    fn get(&self, id: &str) -> Result<&Arc<dyn ExternalProvider>, String> {
        self.available(id)?;
        self.raw(id)
    }

    pub async fn search(
        &self,
        provider: &str,
        query: &str,
        limit: usize,
    ) -> Result<Vec<ProviderTrack>, String> {
        let p = self.get(provider)?;
        if !p.spec().caps.search {
            return Err(format!("{} does not support search", p.spec().name));
        }
        p.search(query, limit.clamp(1, 100)).await
    }

    pub async fn playlists(&self, provider: &str) -> Result<Vec<ProviderPlaylist>, String> {
        let p = self.get(provider)?;
        if !p.spec().caps.playlists {
            return Err(format!("{} does not expose playlists", p.spec().name));
        }
        p.playlists().await
    }

    pub async fn playlist_tracks(
        &self,
        provider: &str,
        id: &str,
    ) -> Result<Vec<ProviderTrack>, String> {
        let p = self.get(provider)?;
        if !p.spec().caps.browse {
            return Err(format!("{} does not support browsing", p.spec().name));
        }
        p.playlist_tracks(id).await
    }

    pub async fn resolve(
        &self,
        provider: &str,
        id: &str,
        title: &str,
    ) -> Result<ProviderStream, String> {
        let p = self.get(provider)?;
        if !p.spec().caps.stream {
            return Err(format!("{} does not support streaming", p.spec().name));
        }
        p.resolve(id, title).await
    }

    pub async fn like(&self, provider: &str, id: &str, title: &str) -> Result<(), String> {
        let p = self.get(provider)?;
        if !p.spec().caps.like {
            return Err(format!("{} does not support likes", p.spec().name));
        }
        p.like(id, title).await
    }

    pub async fn playlist_create(
        &self,
        provider: &str,
        name: &str,
        track_ids: &[String],
    ) -> Result<String, String> {
        let p = self.get(provider)?;
        if !p.spec().caps.playlist_write {
            return Err(format!("{} cannot create playlists", p.spec().name));
        }
        p.playlist_create(name, track_ids).await
    }

    pub async fn playlist_rename(
        &self,
        provider: &str,
        id: &str,
        name: &str,
    ) -> Result<(), String> {
        let p = self.get(provider)?;
        if !p.spec().caps.playlist_write {
            return Err(format!("{} cannot rename playlists", p.spec().name));
        }
        p.playlist_rename(id, name).await
    }

    pub async fn playlist_add(
        &self,
        provider: &str,
        id: &str,
        track_ids: &[String],
    ) -> Result<(), String> {
        let p = self.get(provider)?;
        if !p.spec().caps.playlist_write {
            return Err(format!("{} cannot add to playlists", p.spec().name));
        }
        p.playlist_add(id, track_ids).await
    }

    pub async fn playlist_remove(
        &self,
        provider: &str,
        id: &str,
        track_ids: &[String],
    ) -> Result<(), String> {
        let p = self.get(provider)?;
        if !p.spec().caps.playlist_write {
            return Err(format!("{} cannot remove from playlists", p.spec().name));
        }
        p.playlist_remove(id, track_ids).await
    }

    pub async fn playlist_delete(&self, provider: &str, id: &str) -> Result<(), String> {
        let p = self.get(provider)?;
        if !p.spec().caps.playlist_write {
            return Err(format!("{} cannot delete playlists", p.spec().name));
        }
        p.playlist_delete(id).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn empty_config() -> ProviderConfig {
        ProviderConfig::default()
    }

    #[test]
    fn every_catalogue_entry_is_listed_even_when_unconfigured() {
        let registry = ProviderRegistry::new(&empty_config());
        let sources = registry.sources();
        assert_eq!(sources.len(), PROVIDER_SPECS.len());
        assert!(sources.iter().all(|s| !s.available));
        assert!(sources.iter().all(|s| s.detail.is_some()));
    }

    #[test]
    fn configured_tables_are_still_listed_with_an_honest_detail() {
        let toml: toml::Value = toml::from_str("[qobuz]\nenabled = true\n").unwrap();
        let registry = ProviderRegistry::new(&ProviderConfig::from_toml(Some(toml)));
        let qobuz = registry
            .sources()
            .into_iter()
            .find(|s| s.id == "qobuz")
            .unwrap();
        assert!(!qobuz.available);
        assert!(qobuz.detail.unwrap().contains("configured"));
    }

    #[tokio::test]
    async fn an_unknown_provider_is_refused_by_name() {
        let registry = ProviderRegistry::new(&empty_config());
        let err = registry.search("nope", "x", 10).await.unwrap_err();
        assert!(err.contains("unknown provider"));

        let err = registry.search("qobuz", "x", 10).await.unwrap_err();
        assert!(err.contains("not configured"));
    }
}
