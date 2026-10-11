// Copyright (c) 2026
// Author: prjctimg <prjctimg@outlook.com>
// Playlist registry — every provider that can answer a playlist verb.
//
// This is free software released under the GPL-3.0 license.

pub(crate) mod spotify;

use std::sync::Arc;

use gtm::providers::playlist::{PlaylistError, PlaylistId, PlaylistProvider, PlaylistSource, Verb};
use tracing::debug;

pub(crate) use spotify::SpotifyPlaylists;

use crate::daemon::DaemonInner;

/// Every provider that owns playlists, routed by `source_id`.
///
/// Providers are held behind an `Arc` and cloned out on lookup so the daemon
/// can drop the registry lock before it touches the network. A verb that awaits
/// while holding this lock would serialise the source list and every other
/// provider behind one slow call.
#[derive(Default)]
pub struct PlaylistRegistry {
    providers: Vec<Arc<dyn PlaylistProvider>>,
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
        r.register(Arc::new(SpotifyPlaylists::new(inner)));
        r
    }

    pub fn register(&mut self, provider: Arc<dyn PlaylistProvider>) {
        debug!("playlist: registered {}", provider.source_id());
        self.providers.push(provider);
    }

    pub fn is_empty(&self) -> bool {
        self.providers.is_empty()
    }

    pub fn get(&self, source_id: &str) -> Option<Arc<dyn PlaylistProvider>> {
        self.providers
            .iter()
            .find(|p| p.source_id() == source_id)
            .cloned()
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
    ) -> Result<Arc<dyn PlaylistProvider>, PlaylistError> {
        let p = self.get(&id.source).ok_or_else(|| {
            PlaylistError::NotPermitted(format!("no playlist provider named {}", id.source))
        })?;
        if !p.caps().offers(verb) {
            return Err(PlaylistError::Unsupported(verb));
        }
        Ok(p)
    }
}
