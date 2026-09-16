// Copyright (c) 2026
// Author: prjctimg <prjctimg@outlook.com>
// Subsonic/Navidrome integration: serializable types shared over IPC
//
// This is free software released under the GPL-3.0 license.

use serde::{Deserialize, Serialize};

/// Connection state of the Subsonic (Navidrome) integration.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct SubsonicStatus {
    /// Whether a server + credentials are configured and usable.
    pub configured: bool,
    /// Subsonic server base URL, if configured.
    pub server: Option<String>,
    /// Logged-in user name, if configured.
    pub user: Option<String>,
    /// Most recent error message, if a request failed.
    pub error: Option<String>,
}

/// A track as returned by Subsonic search / album / artist endpoints.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct SubsonicTrack {
    /// Subsonic track id (used for streaming, cover art, and playlists).
    pub id: String,
    pub title: String,
    pub artist: String,
    #[serde(default)]
    pub album: String,
    #[serde(default)]
    pub album_id: String,
    #[serde(default)]
    pub artist_id: String,
    /// Track duration in seconds (`0` when unknown).
    #[serde(default)]
    pub duration_secs: u64,
    #[serde(default)]
    pub year: Option<i64>,
    #[serde(default)]
    pub cover_id: Option<String>,
    #[serde(default)]
    pub suffix: String,
    #[serde(default)]
    pub bit_rate: Option<u64>,
}

/// An album as returned by Subsonic album-list endpoints.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SubsonicAlbum {
    pub id: String,
    pub title: String,
    #[serde(default)]
    pub artist: String,
    #[serde(default)]
    pub artist_id: String,
    #[serde(default)]
    pub year: Option<i64>,
    #[serde(default)]
    pub track_count: u64,
    #[serde(default)]
    pub cover_id: Option<String>,
}

/// A single artist index entry.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SubsonicArtist {
    pub id: String,
    pub name: String,
}

/// Aggregated search results for a `search3` query.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct SubsonicSearchResults {
    pub artists: Vec<SubsonicArtist>,
    pub albums: Vec<SubsonicAlbum>,
    pub tracks: Vec<SubsonicTrack>,
}
