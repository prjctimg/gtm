// Copyright (c) 2026
// Author: prjctimg <prjctimg@outlook.com>
// Spotify integration: serializable types shared over IPC
//
// This is free software released under the GPL-3.0 license.

use serde::{Deserialize, Serialize};

/// librespot's public desktop client id. Works for the OAuth PKCE flow
/// (playlist sync + playback scopes) without creating a dashboard app.
pub const LIBRESPOT_CLIENT_ID: &str = "65b708073fc0480ea92a077233ca87bd";

/// Connection state of the Spotify integration, surfaced in Settings.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct SpotifyStatus {
    /// Whether a token is configured and the client is usable.
    pub linked: bool,
    /// Display name of the linked Spotify account, if known.
    pub user: Option<String>,
    /// Whether the linked account has a Premium subscription. Playback
    /// control endpoints (`/me/player/*`) require Premium; without it the
    /// Settings control rows are disabled.
    pub premium: bool,
    /// Whether the Spotify device is currently playing (as last reported by
    /// the Web API playback endpoint).
    pub playing: bool,
    /// Name of the active playback device, if the Web API reported one.
    pub device: Option<String>,
    /// Spotify device id, which the `/me/player` control endpoints require
    /// (distinct from the display name above).
    pub device_id: Option<String>,
    /// Shuffle state of the active device, as last reported by `/me/player`.
    pub shuffle: bool,
    /// Repeat state of the active device (`off`/`track`/`context`).
    pub repeat: String,
    /// Number of synced playlists currently cached by the daemon.
    pub playlists: usize,
    /// Total number of tracks across all synced playlists.
    pub tracks: usize,
    /// Whether the stored token lacks the `streaming` scope and must be
    /// re-linked before native playback works. Scopes cannot be widened by
    /// refreshing, so this only clears after a fresh authorization.
    pub needs_relink: bool,
    /// Most recent error message, if the link or sync failed.
    pub error: Option<String>,
}

/// A synced Spotify playlist with its cached track list.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SpotifyPlaylist {
    pub id: String,
    pub name: String,
    pub owner: String,
    pub tracks: Vec<SpotifyTrack>,
}

impl SpotifyPlaylist {
    pub fn track_count(&self) -> usize {
        self.tracks.len()
    }
}

/// What kind of object a Spotify web-search result represents. Tracks stream
/// directly; albums/artists/playlists resolve to their track lists on the
/// daemon.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum SpotifySearchKind {
    /// A searchable single track.
    #[default]
    Track,
    /// A searchable album; resolves to the album's track list.
    Album,
    /// A searchable artist; resolves to the artist's top tracks.
    Artist,
    /// A searchable playlist; resolves to the playlist's track list.
    Playlist,
}

/// A single track inside a Spotify playlist.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SpotifyTrack {
    /// Index used to resolve and enqueue this track (position in the cached
    /// playlist track list).
    pub index: usize,
    pub name: String,
    pub artists: String,
    pub album: Option<String>,
    pub duration_ms: Option<u64>,
    /// Spotify track URI (`spotify:track:<id>`), used to resolve and stream
    /// this exact track. `None` for entries without a resolvable ID.
    #[serde(default)]
    pub uri: Option<String>,
    /// URL of the highest-resolution album-cover image, when the Web API
    /// exposes one. Used to render cover art in the Spotify search picker
    /// without an extra network round-trip to the daemon.
    #[serde(default)]
    pub image_url: Option<String>,
    /// What the web search returned for this entry. Absent for synced
    /// playlist tracks (defaults to `Track`).
    #[serde(default, skip_serializing_if = "is_track_kind")]
    pub kind: Option<SpotifySearchKind>,
}

fn is_track_kind(kind: &Option<SpotifySearchKind>) -> bool {
    matches!(kind, None | Some(SpotifySearchKind::Track))
}

impl SpotifyTrack {
    /// True when the track carries a resolvable `spotify:track:` URI.
    pub fn has_uri(&self) -> bool {
        self.uri.is_some()
    }
}
