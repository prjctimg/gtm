// Copyright (c) 2026
// Author: prjctimg <prjctimg@outlook.com>
// The playlist capability every provider can answer, in provider-neutral terms.
//
// This is free software released under the GPL-3.0 license.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

/// A provider that owns playlists and can be asked to change them.
///
/// The read half is deliberately *not* on this trait. A playlist is a list of
/// tracks everywhere, but the data each provider holds about one is not: Spotify
/// embeds tracks in its playlist cache and hands out a position for each, a
/// chart provider publishes an editorial list nobody may write to, and the local
/// library has rows in SQLite. Forcing one read shape over those means either
/// losing the positions `resolve` indexes by or inventing a union list nobody
/// asked for. So reading stays provider-specific and this trait is the verbs
/// that genuinely are the same everywhere: create, rename, add, remove, delete,
/// subscribe, copy.
#[async_trait]
#[allow(clippy::double_must_use)]
pub trait PlaylistProvider: Send + Sync {
    /// Stable identifier for this provider on the wire (`spotify`, `local`).
    fn source_id(&self) -> &str;

    /// What the picker calls it.
    fn display_name(&self) -> &str;

    /// What this provider will and will not do.
    ///
    /// Not a `bool`. A registry that reports a provider `is_configured` while
    /// every call into it fails is how the charts provider kept answering
    /// "no charts available" for a year after Spotify removed the endpoint it
    /// had implemented: configured, listed, and permanently broken, with nothing
    /// in the type able to say so. Bits let the UI grey out `follow` where it
    /// is meaningless instead of offering it and failing.
    fn caps(&self) -> PlaylistCaps;

    /// Create a playlist and answer its id.
    async fn create(&self, spec: NewPlaylist) -> Result<PlaylistId, PlaylistError>;

    /// Change a playlist's name, description or privacy.
    async fn rename(&self, id: &PlaylistId, spec: NewPlaylist) -> Result<(), PlaylistError>;

    /// Append items, or insert at `at` where the provider positions them.
    async fn add_items(
        &self,
        id: &PlaylistId,
        items: &[PlaylistItem],
        at: Option<u32>,
    ) -> Result<(), PlaylistError>;

    /// Remove these occurrences of these items.
    ///
    /// The track *and* where it sits, never the track alone: a playlist may
    /// hold the same track twice, and "remove this one" is not the same request
    /// as "remove every copy". It is also what a provider actually supports --
    /// Spotify's remove endpoint takes `{uri, positions}` and has no other form.
    async fn remove_occurrences(
        &self,
        id: &PlaylistId,
        slots: &[PlaylistSlot],
    ) -> Result<(), PlaylistError>;

    /// Delete a playlist the account owns.
    async fn delete(&self, id: &PlaylistId) -> Result<(), PlaylistError>;

    /// Follow or unfollow someone else's playlist.
    async fn subscribe(&self, id: &PlaylistId, on: bool) -> Result<(), PlaylistError>;

    /// Copy a playlist this account can read into one it owns.
    async fn copy(&self, id: &PlaylistId, name: Option<&str>) -> Result<PlaylistId, PlaylistError>;
}

/// Which verbs a provider actually implements.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct PlaylistCaps {
    pub create: bool,
    pub rename: bool,
    pub add: bool,
    pub remove: bool,
    pub delete: bool,
    pub subscribe: bool,
    pub copy: bool,
}

impl PlaylistCaps {
    /// Every verb on, for a provider that implements the whole trait.
    pub const ALL: Self = Self {
        create: true,
        rename: true,
        add: true,
        remove: true,
        delete: true,
        subscribe: true,
        copy: true,
    };

    /// Whether `verb` is offered, so the UI can grey it instead of failing.
    pub fn offers(&self, verb: Verb) -> bool {
        match verb {
            Verb::Create => self.create,
            Verb::Rename => self.rename,
            Verb::Add => self.add,
            Verb::Remove => self.remove,
            Verb::Delete => self.delete,
            Verb::Subscribe => self.subscribe,
            Verb::Copy => self.copy,
        }
    }
}

/// The seven verbs, named for the capability check and for error messages.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Verb {
    Create,
    Rename,
    Add,
    Remove,
    Delete,
    Subscribe,
    Copy,
}

impl Verb {
    /// The refusal a caller gets when the provider does not offer this.
    pub fn unsupported(verb: Verb) -> PlaylistError {
        PlaylistError::Unsupported(verb)
    }
}

/// Names a playlist at a provider. Every field is optional so one shape covers
/// create (name is the only one that matters) and rename (name may be `None` to
/// leave it alone).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct NewPlaylist {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    /// `None` leaves the existing privacy alone; `Some(false)` makes it private.
    #[serde(default)]
    pub public: Option<bool>,
}

/// A playlist's identity at its provider: opaque here, a Spotify id there.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlaylistId {
    /// The provider this id belongs to, so a registry can route it.
    pub source: String,
    /// The provider's own identifier.
    pub id: String,
}

impl PlaylistId {
    pub fn new(source: &str, id: impl Into<String>) -> Self {
        Self {
            source: source.to_string(),
            id: id.into(),
        }
    }
}

/// One item to add to a playlist.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlaylistItem {
    /// Provider-native reference: a `spotify:track:` uri, a library row id, a
    /// provider track id. Left as a string because it is opaque by design.
    pub item: String,
    /// Optional label the provider can use to find the track when `item` is a
    /// bare id. Providers that need it and do not get it say so themselves.
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub artist: Option<String>,
}

/// One item, at the positions it occupies in a playlist.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlaylistSlot {
    pub item: PlaylistItem,
    #[serde(default)]
    pub positions: Vec<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum PlaylistError {
    /// The provider does not offer this verb, so nothing was sent.
    Unsupported(Verb),
    /// Not linked, or linked as an account that cannot do this.
    NotPermitted(String),
    /// The provider said no, in its own words.
    Rejected(String),
    /// The provider could not be reached.
    Unreachable(String),
}

impl std::fmt::Display for PlaylistError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unsupported(v) => write!(f, "{v:?} is not supported by this provider"),
            Self::NotPermitted(m) => write!(f, "not permitted: {m}"),
            Self::Rejected(m) => write!(f, "{m}"),
            Self::Unreachable(m) => write!(f, "could not reach the provider: {m}"),
        }
    }
}

impl std::error::Error for PlaylistError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn caps_gate_every_verb() {
        let mut caps = PlaylistCaps::default();
        for verb in [
            Verb::Create,
            Verb::Rename,
            Verb::Add,
            Verb::Remove,
            Verb::Delete,
            Verb::Subscribe,
            Verb::Copy,
        ] {
            assert!(!caps.offers(verb), "{verb:?} should start unsupported");
        }
        caps = PlaylistCaps::ALL;
        for verb in [
            Verb::Create,
            Verb::Rename,
            Verb::Add,
            Verb::Remove,
            Verb::Delete,
            Verb::Subscribe,
            Verb::Copy,
        ] {
            assert!(caps.offers(verb), "{verb:?} should be supported");
        }
    }

    /// A source-less id is the failure mode that makes a registry answer with
    /// the wrong provider's playlists, so it has to round-trip intact.
    #[test]
    fn an_id_keeps_its_source_across_the_wire() {
        let id = PlaylistId::new("spotify", "37i9dQ");
        let json = serde_json::to_string(&id).unwrap();
        assert_eq!(serde_json::from_str::<PlaylistId>(&json).unwrap(), id);
    }
}
