// Copyright (c) 2026 - present
// Author: prjctimg <prjctimg@outlook.com>
// Spotify integration: serializable types shared over IPC
//
// This is free software released under the GPL-3.0 license.

use serde::{Deserialize, Serialize};

/// Connection state of the Spotify integration, surfaced in Settings.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct SpotifyStatus {
    /// Whether a token is configured and the client is usable.
    pub linked: bool,
    /// Display name of the linked Spotify account, if known.
    pub user: Option<String>,
    /// Number of synced playlists currently cached by the daemon.
    pub playlists: usize,
    /// Total number of tracks across all synced playlists.
    pub tracks: usize,
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
    /// Spotify track URI (`spotify:track:<id>`), used to play this exact
    /// track through Soloist. `None` for entries without a resolvable ID.
    #[serde(default)]
    pub uri: Option<String>,
}

/// Connection state of the Soloist playback bridge, surfaced in Settings.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct SoloistStatus {
    /// Whether a Soloist API key is configured (persisted on disk).
    pub key_set: bool,
    /// Whether the `soloist` daemon process has been spawned.
    pub running: bool,
    /// Whether the WebSocket connection to Soloist is live.
    pub connected: bool,
    /// Whether Soloist reports an authenticated Spotify Connect session.
    pub logged_in: bool,
    /// Spotify display name of the connected user, if known.
    pub user: Option<String>,
    /// Active output device name reported by Soloist.
    pub device: Option<String>,
    /// Currently playing track mirrored from Soloist events.
    pub track: Option<SpotifyTrack>,
    /// Whether playback is active (status `playing`/`buffering`).
    pub playing: bool,
    /// Most recent error, if Soloist failed to start/connect or an event
    /// reported an error.
    pub error: Option<String>,
}
