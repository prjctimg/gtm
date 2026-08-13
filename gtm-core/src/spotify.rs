// Copyright (c) 2025 - present
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
}
