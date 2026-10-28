// Copyright (c) 2026
// Author: prjctimg <prjctimg@outlook.com>
// External music providers: shared catalog, wire model and daemon trait
//
// This is free software released under the GPL-3.0 license.

//! The provider-agnostic half of every non-native source.
//!
//! Spotify, YouTube and radio grew specialised wire types because each has a
//! playback model the others do not share: librespot streams Spotify on its own
//! socket, YouTube is an extractor pipeline, and radio is a live stream with
//! ICY metadata. The remaining services are catalogues: search, browse a
//! playlist, resolve a track, report a like. Those four verbs are the same
//! everywhere, so they get one model here rather than sixteen near-copies.
//!
//! The specs below are deliberately data, not registration. A provider is
//! listed even when it is not configured: the UI has to be able to say "Tidal
//! is available but not linked" rather than hiding the row, and a config typo
//! must be explainable instead of looking like a missing feature.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

/// How a provider is reached.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderKind {
    /// Catalogue resolved through yt-dlp and its extractors.
    YtDlp,
    /// Account OAuth or a browser session.
    OAuth,
    /// A regional service with its own token flow.
    Regional,
    /// A self-hosted media server.
    MediaServer,
}

impl ProviderKind {
    /// Lowercase label for CLI rows and Setup.
    pub fn label(self) -> &'static str {
        match self {
            ProviderKind::YtDlp => "yt-dlp",
            ProviderKind::OAuth => "oauth",
            ProviderKind::Regional => "regional",
            ProviderKind::MediaServer => "media server",
        }
    }
}

/// What the user must supply before a provider can answer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderAuth {
    /// No account needed.
    None,
    /// A browser login is imported as cookies.
    BrowserCookies,
    /// An OAuth page is opened in the browser.
    BrowserOAuth,
    /// A device code is approved in the browser.
    DeviceCode,
    /// A token or API key pasted into Setup.
    Token,
    /// A server URL and account credentials.
    Server,
}

/// The capabilities a provider advertises.
///
/// Bits rather than "is configured": a server can be reachable and still have
/// no write API. The UI greys out verbs from these flags, so an unsupported
/// action is visible and explained instead of failing after a round trip.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderCaps {
    pub search: bool,
    pub browse: bool,
    pub playlists: bool,
    pub playlist_write: bool,
    pub like: bool,
    pub stream: bool,
}

impl ProviderCaps {
    const fn new(search: bool, browse: bool, playlists: bool, write: bool, like: bool) -> Self {
        Self {
            search,
            browse,
            playlists,
            playlist_write: write,
            like,
            stream: true,
        }
    }
}

/// Everything static about one provider.
#[derive(Debug, Clone, Copy)]
pub struct ProviderSpec {
    /// Stable id used on the wire, in config and as the queue path namespace.
    pub id: &'static str,
    /// Human name used in every list.
    pub name: &'static str,
    /// How it is reached.
    pub kind: ProviderKind,
    /// What Setup asks for.
    pub auth: ProviderAuth,
    /// The `config.toml` table Setup writes (`[qobuz]`, `[navidrome]`, ...).
    pub config_section: &'static str,
    /// A one-line description of the source.
    pub description: &'static str,
    /// What the catalogue supports.
    pub caps: ProviderCaps,
}

/// The provider catalogue, in the order the picker lists it.
///
/// Grouped by how they are configured rather than alphabetically: the four
/// groups are four different Setup conversations, and mixing a media server
/// between two OAuth services makes the list read like a bag of names.
pub const PROVIDER_SPECS: &[ProviderSpec] = &[
    ProviderSpec {
        id: "ytmusic",
        name: "YouTube Music",
        kind: ProviderKind::YtDlp,
        auth: ProviderAuth::BrowserCookies,
        config_section: "ytmusic",
        description: "YouTube Music catalogue and playlists",
        caps: ProviderCaps::new(true, true, true, false, true),
    },
    ProviderSpec {
        id: "soundcloud",
        name: "SoundCloud",
        kind: ProviderKind::YtDlp,
        auth: ProviderAuth::BrowserCookies,
        config_section: "soundcloud",
        description: "SoundCloud tracks, likes and playlists",
        caps: ProviderCaps::new(true, true, true, false, true),
    },
    ProviderSpec {
        id: "mixcloud",
        name: "Mixcloud",
        kind: ProviderKind::YtDlp,
        auth: ProviderAuth::None,
        config_section: "mixcloud",
        description: "DJ mixes and radio shows",
        caps: ProviderCaps::new(true, true, true, false, false),
    },
    ProviderSpec {
        id: "bandcamp",
        name: "Bandcamp",
        kind: ProviderKind::YtDlp,
        auth: ProviderAuth::Token,
        config_section: "bandcamp",
        description: "Bandcamp purchases and label catalogue",
        caps: ProviderCaps::new(true, true, true, true, false),
    },
    ProviderSpec {
        id: "bilibili",
        name: "Bilibili",
        kind: ProviderKind::YtDlp,
        auth: ProviderAuth::BrowserCookies,
        config_section: "bilibili",
        description: "Bilibili music videos and performances",
        caps: ProviderCaps::new(true, false, false, false, false),
    },
    ProviderSpec {
        id: "netease",
        name: "NetEase Cloud Music",
        kind: ProviderKind::YtDlp,
        auth: ProviderAuth::BrowserCookies,
        config_section: "netease",
        description: "NetEase Cloud Music catalogue",
        caps: ProviderCaps::new(true, true, true, false, true),
    },
    ProviderSpec {
        id: "qobuz",
        name: "Qobuz",
        kind: ProviderKind::OAuth,
        auth: ProviderAuth::BrowserOAuth,
        config_section: "qobuz",
        description: "Qobuz lossless catalogue and playlists",
        caps: ProviderCaps::new(true, true, true, false, true),
    },
    ProviderSpec {
        id: "tidal",
        name: "Tidal",
        kind: ProviderKind::OAuth,
        auth: ProviderAuth::DeviceCode,
        config_section: "tidal",
        description: "Tidal lossless catalogue and playlists",
        caps: ProviderCaps::new(true, true, true, false, true),
    },
    ProviderSpec {
        id: "yandex",
        name: "Yandex Music",
        kind: ProviderKind::Regional,
        auth: ProviderAuth::Token,
        config_section: "yandex",
        description: "Yandex Music catalogue, waves and playlists",
        caps: ProviderCaps::new(true, true, true, false, true),
    },
    ProviderSpec {
        id: "xiaoyuzhou",
        name: "Xiaoyuzhou",
        kind: ProviderKind::Regional,
        auth: ProviderAuth::None,
        config_section: "xiaoyuzhou",
        description: "Xiaoyuzhou podcast episodes by URL",
        caps: ProviderCaps::new(false, false, false, false, false),
    },
    ProviderSpec {
        id: "navidrome",
        name: "Navidrome",
        kind: ProviderKind::MediaServer,
        auth: ProviderAuth::Server,
        config_section: "navidrome",
        description: "Navidrome library over the Subsonic API",
        caps: ProviderCaps::new(true, true, true, true, true),
    },
    ProviderSpec {
        id: "lyrion",
        name: "Lyrion",
        kind: ProviderKind::MediaServer,
        auth: ProviderAuth::Server,
        config_section: "lyrion",
        description: "Lyrion Music Server library",
        caps: ProviderCaps::new(true, true, true, true, false),
    },
    ProviderSpec {
        id: "plex",
        name: "Plex",
        kind: ProviderKind::MediaServer,
        auth: ProviderAuth::Server,
        config_section: "plex",
        description: "Plex music library",
        caps: ProviderCaps::new(true, true, true, false, false),
    },
    ProviderSpec {
        id: "jellyfin",
        name: "Jellyfin",
        kind: ProviderKind::MediaServer,
        auth: ProviderAuth::Server,
        config_section: "jellyfin",
        description: "Jellyfin music library",
        caps: ProviderCaps::new(true, true, true, false, true),
    },
    ProviderSpec {
        id: "emby",
        name: "Emby",
        kind: ProviderKind::MediaServer,
        auth: ProviderAuth::Server,
        config_section: "emby",
        description: "Emby music library",
        caps: ProviderCaps::new(true, true, true, false, true),
    },
    ProviderSpec {
        id: "audiobookshelf",
        name: "Audiobookshelf",
        kind: ProviderKind::MediaServer,
        auth: ProviderAuth::Server,
        config_section: "audiobookshelf",
        description: "Audiobookshelf libraries and podcasts",
        caps: ProviderCaps::new(true, true, true, false, true),
    },
];

/// Look up a provider by its wire id.
pub fn provider_spec(id: &str) -> Option<&'static ProviderSpec> {
    PROVIDER_SPECS.iter().find(|spec| spec.id == id)
}

/// The display name for an id, or the id itself when it is unknown.
///
/// Unknown ids reach here from older configs and from queue rows written before
/// a provider was removed. Showing the raw id is better than an empty row.
pub fn provider_name(id: &str) -> &str {
    provider_spec(id).map_or(id, |spec| spec.name)
}

/// The current state of one provider, as the client needs it.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProviderSource {
    pub id: String,
    pub name: String,
    pub kind: ProviderKind,
    pub auth: ProviderAuth,
    pub description: String,
    pub caps: ProviderCaps,
    /// Whether this provider can answer right now. A provider that is listed but
    /// unavailable is still shown, with `detail` explaining what is missing.
    pub available: bool,
    /// Why it is unavailable, or a short account/server label when it is.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

impl ProviderSource {
    /// Build a source row from its spec.
    pub fn new(spec: &'static ProviderSpec, available: bool, detail: Option<String>) -> Self {
        Self {
            id: spec.id.to_string(),
            name: spec.name.to_string(),
            kind: spec.kind,
            auth: spec.auth,
            description: spec.description.to_string(),
            caps: spec.caps,
            available,
            detail,
        }
    }
}

/// One playlist (or album, or saved list) owned by a provider.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProviderPlaylist {
    pub provider: String,
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub owner: String,
    #[serde(default)]
    pub track_count: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cover_url: Option<String>,
    /// Whether this provider will accept create/rename/add/remove for this row.
    pub editable: bool,
}

/// One track in a provider catalogue.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct ProviderTrack {
    pub provider: String,
    /// Provider-specific id. For extractor-backed providers this is a page URL;
    /// for catalogue APIs it is the service's own track id.
    pub id: String,
    pub title: String,
    #[serde(default)]
    pub artist: String,
    #[serde(default)]
    pub album: String,
    #[serde(default)]
    pub duration_secs: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cover_url: Option<String>,
    /// The provider playlist this row came from, when it is not a search hit.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub playlist_id: Option<String>,
}

impl ProviderTrack {
    /// The synthetic queue path for this row.
    ///
    /// The id is kept verbatim after the provider segment, so ids with slashes
    /// (Plex part paths, catalogue URLs) survive the round trip. The provider id
    /// itself is a slug and can never contain `/`.
    pub fn path(&self) -> String {
        format!("provider://{}/{}", self.provider, self.id)
    }
}

/// A resolved playable URL.
///
/// `url` is the whole credential: a signed query string, a server token in the
/// path, or a redirect. The transport has no per-provider header hook, so a
/// provider that needs an `Authorization` header must produce a URL the server
/// accepts instead of relying on one the daemon cannot send.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProviderStream {
    pub url: String,
    /// A live stream has no duration and no seek.
    #[serde(default)]
    pub live: bool,
}

/// The four catalogue verbs plus the two optional writes every provider shares.
///
/// Defaults return a precise refusal rather than silently succeeding: a
/// provider that cannot write playlists must say so, and the registry maps that
/// into the capability flags the UI already shows.
#[async_trait]
pub trait ExternalProvider: Send + Sync {
    /// Static descriptor.
    fn spec(&self) -> &'static ProviderSpec;

    /// Whether calls can reach the service now.
    fn available(&self) -> bool {
        true
    }

    /// Account name, server host, or the reason `available()` is false.
    fn detail(&self) -> Option<String> {
        None
    }

    /// Search the catalogue. Extractors return page URLs as ids; catalogue
    /// services return their own ids.
    async fn search(&self, _query: &str, _limit: usize) -> Result<Vec<ProviderTrack>, String> {
        Err(format!("{} does not support search", self.spec().name))
    }

    /// List the user's playlists, albums or saved shows.
    async fn playlists(&self) -> Result<Vec<ProviderPlaylist>, String> {
        Err(format!(
            "{} does not expose a playlist list",
            self.spec().name
        ))
    }

    /// Read one playlist's tracks.
    async fn playlist_tracks(&self, _id: &str) -> Result<Vec<ProviderTrack>, String> {
        Err(format!(
            "{} does not support playlist browsing",
            self.spec().name
        ))
    }

    /// Resolve a track id into a playable URL.
    async fn resolve(&self, _id: &str, _title: &str) -> Result<ProviderStream, String> {
        Err(format!("{} cannot stream this track", self.spec().name))
    }

    /// Save a track to the provider's likes.
    async fn like(&self, _id: &str, _title: &str) -> Result<(), String> {
        Err(format!("{} does not support likes", self.spec().name))
    }

    /// Create a playlist. Returns the provider's new id.
    async fn playlist_create(&self, _name: &str, _track_ids: &[String]) -> Result<String, String> {
        Err(format!("{} cannot create playlists", self.spec().name))
    }

    /// Rename a playlist.
    async fn playlist_rename(&self, _id: &str, _name: &str) -> Result<(), String> {
        Err(format!("{} cannot rename playlists", self.spec().name))
    }

    /// Append tracks to a playlist.
    async fn playlist_add(&self, _id: &str, _track_ids: &[String]) -> Result<(), String> {
        Err(format!("{} cannot add to playlists", self.spec().name))
    }

    /// Remove tracks from a playlist.
    async fn playlist_remove(&self, _id: &str, _track_ids: &[String]) -> Result<(), String> {
        Err(format!("{} cannot remove from playlists", self.spec().name))
    }

    /// Delete a playlist.
    async fn playlist_delete(&self, _id: &str) -> Result<(), String> {
        Err(format!("{} cannot delete playlists", self.spec().name))
    }
}

/// Placeholder for a provider whose concrete client has not been built yet.
///
/// The framework ships the full catalogue first so Setup, the source list and
/// the queue namespace are stable while integrations land. Every call names the
/// missing feature instead of pretending the provider is absent.
pub struct UnconfiguredProvider {
    spec: &'static ProviderSpec,
    detail: String,
}

impl UnconfiguredProvider {
    pub fn new(spec: &'static ProviderSpec, detail: impl Into<String>) -> Self {
        Self {
            spec,
            detail: detail.into(),
        }
    }
}

#[async_trait]
impl ExternalProvider for UnconfiguredProvider {
    fn spec(&self) -> &'static ProviderSpec {
        self.spec
    }

    fn available(&self) -> bool {
        false
    }

    fn detail(&self) -> Option<String> {
        Some(self.detail.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_catalog_entry_has_a_unique_wire_id() {
        let mut ids: Vec<&str> = PROVIDER_SPECS.iter().map(|s| s.id).collect();
        let total = ids.len();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), total, "duplicate provider id");
    }

    #[test]
    fn ids_are_safe_queue_path_segments() {
        for spec in PROVIDER_SPECS {
            assert!(!spec.id.is_empty());
            assert!(
                spec.id
                    .chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit()),
                "{} is not a path-safe provider id",
                spec.id
            );
            assert_eq!(spec.id, spec.config_section);
        }
    }

    #[test]
    fn track_paths_keep_ids_with_slashes() {
        let track = ProviderTrack {
            provider: "plex".into(),
            id: "library/parts/42/file.flac".into(),
            ..Default::default()
        };
        assert_eq!(track.path(), "provider://plex/library/parts/42/file.flac");
    }

    #[test]
    fn lookup_accepts_known_ids_and_reports_unknown_ones() {
        assert_eq!(provider_name("qobuz"), "Qobuz");
        assert_eq!(provider_name("unknown"), "unknown");
        assert!(provider_spec("tidal").is_some());
        assert!(provider_spec("spotify").is_none());
    }

    #[test]
    fn media_servers_advertise_browse_and_stream() {
        for id in [
            "navidrome",
            "lyrion",
            "plex",
            "jellyfin",
            "emby",
            "audiobookshelf",
        ] {
            let spec = provider_spec(id).expect("media server spec");
            assert_eq!(spec.kind, ProviderKind::MediaServer);
            assert_eq!(spec.auth, ProviderAuth::Server);
            assert!(spec.caps.stream);
            assert!(spec.caps.search);
            assert!(spec.caps.browse);
        }
    }

    #[test]
    fn playlist_writes_are_only_advertised_where_they_exist() {
        let writable: Vec<&str> = PROVIDER_SPECS
            .iter()
            .filter(|s| s.caps.playlist_write)
            .map(|s| s.id)
            .collect();
        assert_eq!(writable, ["bandcamp", "navidrome", "lyrion"]);
    }
}
