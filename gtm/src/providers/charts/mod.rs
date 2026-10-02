// Copyright (c) 2026
// Author: prjctimg <prjctimg@outlook.com>
// Top charts: provider-agnostic data model and the client's tree actions
//
// This is free software released under the GPL-3.0 license.

//! A new chart source implements the `ChartProvider` trait on the daemon side
//! (gtmd/src/providers/charts/) and appears in this UI without any
//! shared-crate changes.

pub mod app;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ChartSource {
    pub id: String,       // "spotify" | "apple" | <community provider id> ...
    pub display: String,  // human label e.g. "Spotify Charts"
    pub configured: bool, // has valid auth / is available
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ChartPlaylist {
    pub source_id: String, // matches ChartSource.id
    pub id: String,        // provider-specific playlist/chart id
    pub title: String,
    pub description: Option<String>,
    pub cover_url: Option<String>,
    pub owner: Option<String>,
    pub track_count: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ChartTrack {
    pub index: usize,
    pub title: String,
    pub artists: String,
    pub album: Option<String>,
    pub duration_ms: Option<u64>,
    pub uri: String, // provider-specific playable URI (e.g. spotify:track:...)
    pub cover_url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ChartError {
    Unconfigured(String),
    Network(String),
    Parse(String),
    Empty,
}

impl std::fmt::Display for ChartError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ChartError::Unconfigured(s) => write!(f, "charts not configured: {s}"),
            ChartError::Network(s) => write!(f, "network error: {s}"),
            ChartError::Parse(s) => write!(f, "parse error: {s}"),
            ChartError::Empty => write!(f, "no charts available"),
        }
    }
}

impl std::error::Error for ChartError {}

// ─── Music browse: search, artist pages, album tracklists ───

/// One result of a music search: a song, a release, or a person.
///
/// Internally tagged on `kind` so the client reads one list and switches on the
/// variant, which is what lets the three sit in a single scroller without a
/// second picker per type.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum BrowseHit {
    Track(BrowseTrack),
    Album(BrowseAlbum),
    Artist(BrowseArtist),
}

impl BrowseHit {
    /// What the row says, for a list that renders every variant.
    pub fn title(&self) -> &str {
        match self {
            BrowseHit::Track(t) => &t.title,
            BrowseHit::Album(a) => &a.title,
            BrowseHit::Artist(a) => &a.name,
        }
    }

    /// The second line: an artist for a song or album, nothing for a person,
    /// whose name is already the whole of it.
    pub fn artist(&self) -> Option<&str> {
        match self {
            BrowseHit::Track(t) => (!t.artist.is_empty()).then_some(t.artist.as_str()),
            BrowseHit::Album(a) => (!a.artist.is_empty()).then_some(a.artist.as_str()),
            BrowseHit::Artist(_) => None,
        }
    }

    pub fn image_url(&self) -> Option<&str> {
        match self {
            BrowseHit::Track(t) => t.image_url.as_deref(),
            BrowseHit::Album(a) => a.image_url.as_deref(),
            BrowseHit::Artist(a) => a.image_url.as_deref(),
        }
    }

    /// Provider id, for the drill-down that fetches this row's own page.
    pub fn provider_id(&self) -> u64 {
        match self {
            BrowseHit::Track(t) => t.id,
            BrowseHit::Album(a) => a.id,
            BrowseHit::Artist(a) => a.id,
        }
    }
}

/// A song, with the 30-second preview that is the only audio this source has.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BrowseTrack {
    pub id: u64,
    pub title: String,
    pub artist: String,
    pub album: Option<String>,
    pub duration_secs: Option<u64>,
    pub preview_url: Option<String>,
    pub image_url: Option<String>,
}

/// A release. `track_count` is the catalogue's own count, which for a single
/// or an EP is not a hundred tracks.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BrowseAlbum {
    pub id: u64,
    pub title: String,
    pub artist: String,
    pub image_url: Option<String>,
    pub track_count: Option<u64>,
}

/// A person, and how many releases the catalogue credits them with.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BrowseArtist {
    pub id: u64,
    pub name: String,
    pub image_url: Option<String>,
    pub album_count: Option<u64>,
}

/// An artist page: the person, their top tracks, and their releases.
///
/// The tracks are the source's *top* list rather than a discography — it is the
/// only per-artist track list the API exposes. The client labels it that way
/// rather than presenting it as everything they have recorded.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ArtistPage {
    pub artist: BrowseArtist,
    pub top_tracks: Vec<BrowseTrack>,
    pub albums: Vec<BrowseAlbum>,
}

/// An album and its full tracklist — the one question the Spotify API cannot be
/// asked any more.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AlbumPage {
    pub album: BrowseAlbum,
    pub tracks: Vec<BrowseTrack>,
}

// Daemon-side trait. Implementations live in gtmd/src/charts/.
// async_trait makes it dyn-compatible for `Box<dyn ChartProvider>`. It also
// stamps `#[must_use]` onto the trait, duplicating the one its generated
// futures already carry, which `clippy::double_must_use` rejects. The lint has
// no machine-applicable fix, so `clippy --fix` cannot clear it and CI fails
// until it is allowed here.
#[async_trait]
#[allow(clippy::double_must_use)]
pub trait ChartProvider: Send + Sync {
    fn source_id(&self) -> &str;
    fn display_name(&self) -> &str;
    fn is_configured(&self) -> bool;

    /// List available charts/editorial playlists for this provider.
    async fn list_charts(&self) -> Result<Vec<ChartPlaylist>, ChartError>;

    /// Fetch tracks for a specific chart playlist.
    async fn chart_tracks(&self, chart_id: &str) -> Result<Vec<ChartTrack>, ChartError>;
}
