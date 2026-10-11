// Copyright (c) 2026
// Author: prjctimg <prjctimg@outlook.com>
// Playlist registry — every provider that can answer a playlist verb.
//
// This is free software released under the GPL-3.0 license.

pub(crate) mod spotify;

use gtm::providers::playlist::{PlaylistCaps, PlaylistError, PlaylistId, PlaylistProvider, Verb};
use tracing::debug;

pub(crate) use spotify::SpotifyPlaylists;

use crate::daemon::DaemonInner;

/// One registered provider, as the client sees it.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct PlaylistSource {
    pub source_id: String,
    pub display_name: String,
    #[serde(default)]
    pub caps: PlaylistCaps,
    /// Why this provider is not usable right now, if it is registered and not
    /// usable. The charts registry had no way to say this and kept offering a
    /// provider whose endpoints had been removed.
    #[serde(default)]
    pub unavailable: Option<String>,
}

/// Every provider that owns playlists, routed by `source_id`.
#[derive(Default)]
pub struct PlaylistRegistry {
    providers: Vec<Box<dyn PlaylistProvider>>,
}

impl PlaylistRegistry {
    pub fn empty() -> Self {
        Self {
            providers: Vec::new(),
        }
    }

    /// Register the providers this daemon knows how to drive.
    ///
    /// Spotify is the only one today. Podcast feeds are the natural second: they
    /// have a subscribe and an unsubscribe and nothing else, which is two of the
    /// seven verbs and a `PlaylistCaps` that says so.
    pub(crate) fn with_defaults(inner: std::sync::Arc<DaemonInner>) -> Self {
        let mut r = Self::empty();
        r.register(Box::new(SpotifyPlaylists::new(inner)));
        r
    }

    pub fn register(&mut self, provider: Box<dyn PlaylistProvider>) {
        debug!("playlist: registered {}", provider.source_id());
        self.providers.push(provider);
    }

    pub fn is_empty(&self) -> bool {
        self.providers.is_empty()
    }

    pub fn get(&self, source_id: &str) -> Option<&dyn PlaylistProvider> {
        self.providers
            .iter()
            .find(|p| p.source_id() == source_id)
            .map(AsRef::as_ref)
    }

    /// What the client needs to draw a source list and grey out what is not
    /// offered.
    pub fn sources(&self, spotify_linked: bool) -> Vec<PlaylistSource> {
        self.providers
            .iter()
            .map(|p| PlaylistSource {
                source_id: p.source_id().to_string(),
                display_name: p.display_name().to_string(),
                caps: p.caps(),
                unavailable: (p.source_id() == "spotify" && !spotify_linked)
                    .then(|| "spotify not linked".to_string()),
            })
            .collect()
    }

    /// Refuse a verb the provider does not implement before anything is sent.
    ///
    /// The check the capability bits exist for: offering `follow` on a provider
    /// that cannot follow, and answering with a provider error, is worse than
    /// not offering it.
    pub fn check(
        &self,
        id: &PlaylistId,
        verb: Verb,
    ) -> Result<&dyn PlaylistProvider, PlaylistError> {
        let p = self.get(&id.source).ok_or_else(|| {
            PlaylistError::NotPermitted(format!("no playlist provider named {}", id.source))
        })?;
        if !p.caps().offers(verb) {
            return Err(PlaylistError::Unsupported(verb));
        }
        Ok(p)
    }
}
