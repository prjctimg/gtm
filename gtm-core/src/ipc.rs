// Copyright (c) 2025 - present
// Author: prjctimg <prjctimg@outlook.com>
// IPC protocol enums: DaemonReq, DaemonRes, DaemonEvent, and action types
//
// This is free software released under the GPL-3.0 license.
//
// ```text
//  Client (gtm)                      Server (gtmd)
//  ────────────                      ────────────
//  DaemonReq (JSON line) ──────▶     handle_request()
//                                    └── cmd_*() handler
//  DaemonRes (JSON line)  ◀──────    reply_tx.send(response)
//
//  Additionally, the server pushes broadcast events to all connected
//  clients via DaemonEvent frames (bincode-encoded) sent over the
//  same Unix socket:
//
//  DaemonEvent ◀───────              event_tx.send(event)
//    (bincode frame)
//
//  Event types: PlaybackStarted, PlaybackPaused, PlaybackStopped,
//  PositionChanged, DurationChanged, VolumeChanged, QueueChanged, etc.
// ```

use crate::state::{self, DaemonState, RepeatMode};
use crate::track::{LrcData, Playlist, StreamInfo, TrackInfo, YTSearchResult};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LibraryAction {
    Scan {
        path: String,
    },
    GetTracks {
        filter: Option<String>,
        sort: Option<String>,
    },
    GetPlaylists,
    CreatePlaylist {
        name: String,
    },
    DeletePlaylist {
        id: i64,
    },
    AddToPlaylist {
        playlist_id: i64,
        track_ids: Vec<i64>,
    },
    ImportM3u {
        path: String,
    },
    ExportM3u {
        playlist_id: i64,
        path: String,
    },
    GetRecent {
        count: u128,
    },
    SyncCovers,
    SyncLyrics,
    RemoveFromPlaylist {
        playlist_id: i64,
        track_id: i64,
    },
    RemoveTrack {
        id: i64,
    },
    UpdateMetadata {
        track_id: i64,
        title: Option<String>,
        artist: Option<String>,
        album: Option<String>,
        genre: Option<String>,
        year: Option<i32>,
        track_number: Option<i32>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QueueAction {
    List,
    Clear,
    Remove {
        index: u128,
    },
    Move {
        from: u128,
        to: u128,
    },
    Add {
        path: String,
        position: Option<u128>,
    },
    AddMany {
        paths: Vec<String>,
    },
    AddFolder {
        path: String,
    },
    Set {
        paths: Vec<String>,
        start_idx: u128,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DaemonEvent {
    PlaybackStarted {
        track: TrackInfo,
        auto_advanced: bool,
        time_pos: f64,
        duration: f64,
    },
    PlaybackPaused {
        time_pos: f64,
    },
    PlaybackStopped,
    TrackEnded,
    PositionChanged {
        time_pos: f64,
    },
    DurationChanged {
        duration: f64,
    },
    VolumeChanged {
        volume: u8,
    },
    MetadataChanged {
        event: String,
    },
    QueueChanged {
        queue: Vec<TrackInfo>,
        cursor: u128,
    },
    QueueIndexChanged {
        index: u128,
    },
    RepeatModeChanged {
        mode: RepeatMode,
    },
    ShuffleChanged {
        enabled: bool,
    },
    CrossfadeChanged {
        enabled: bool,
        duration_secs: u8,
    },
    SleepTimerTick {
        remaining_secs: u32,
    },
    SleepTimerExpired,
    EqPresetChanged {
        preset: state::EqPreset,
    },
    EqEnabledChanged {
        enabled: bool,
    },
    ReverbChanged {
        enabled: bool,
        room_size: f32,
    },
    Custom {
        name: String,
        data: HashMap<String, String>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DaemonReq {
    // ─── Playback ───
    Play {
        path: String,
        start_pos: f64,
    },
    PlayPause,
    Pause,
    Stop,
    Next,
    Prev,
    Seek {
        position_secs: f64,
    },
    SetVolume {
        volume: u8,
    },
    ToggleShuffle,
    CycleRepeat {
        mode: state::RepeatMode,
    },
    ToggleMute,
    Crossfade {
        enabled: bool,
        duration_secs: u8,
    },
    SetCrossfadeEasing {
        easing: state::Easing,
    },
    SetEqPreset {
        preset: state::EqPreset,
    },
    SetEqEnabled {
        enabled: bool,
    },
    SetReverb {
        enabled: bool,
        room_size: f32,
    },

    // ─── Queue ───
    Queue {
        action: QueueAction,
    },

    // ─── Library ───
    Library {
        action: LibraryAction,
    },

    // ─── Discovery ───
    Search {
        query: String,
    },
    GetFavourites,
    AddFavourite {
        track_id: i64,
    },
    RemoveFavourite {
        track_id: i64,
    },
    YtSearch {
        query: String,
        filter: Option<state::YTFilter>,
    },
    YtSearchPoll,
    YtSearchCancel,
    YtResolveStream {
        url: String,
    },

    // ─── Cover Art ───
    GetCoverArt {
        track_id: i64,
    },

    // ─── Lyrics ───
    GetLyrics {
        track_id: i64,
    },

    // ─── Sleep Timer ───
    SetSleepTimer {
        minutes: u32,
    },
    CancelSleepTimer,

    // ─── System ───
    GetStatus,
    CheckHealth,
    Ping,
    Quit,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DaemonRes {
    Ok {
        version: u32,
    },
    Value {
        version: u32,
        value: serde_json::Value,
    },
    Tracks {
        version: u32,
        tracks: Vec<TrackInfo>,
    },
    QueueState {
        version: u32,
        tracks: Vec<TrackInfo>,
        cursor: u128,
    },
    Status {
        version: u32,
        state: Box<DaemonState>,
    },
    Playlists {
        version: u32,
        playlists: Vec<Playlist>,
    },
    YtSearchResults {
        version: u32,
        results: Vec<YTSearchResult>,
    },
    StreamInfo {
        version: u32,
        info: Box<StreamInfo>,
    },
    Lyrics {
        version: u32,
        lyrics: Option<LrcData>,
    },
    CoverArt {
        version: u32,
        data: Option<String>, // base64-encoded PNG bytes
    },
    SyncCoversResult {
        version: u32,
        synced: usize,
        total: usize,
    },
    SyncLyricsResult {
        version: u32,
        synced: usize,
        total: usize,
    },
    Pong,
    HealthReport {
        version: u32,
        report: Box<HealthReport>,
    },
    Error {
        version: u32,
        message: String,
    },
}

/// Wire envelope for requests: wraps a `DaemonReq` with a monotonic request ID
/// so the client can correlate responses with in-flight requests.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WireReq {
    pub id: u64,
    #[serde(flatten)]
    pub req: DaemonReq,
}

/// Wire envelope for responses: wraps a `DaemonRes` with the same request ID
/// the client sent, enabling out-of-order response dispatch.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WireRes {
    pub id: u64,
    #[serde(flatten)]
    pub res: DaemonRes,
}

/// Health status of a daemon component.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComponentHealth {
    pub name: String,
    pub status: HealthStatus,
    pub message: Option<String>,
    pub uptime_secs: Option<f64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HealthStatus {
    Ok,
    Degraded,
    Error,
}

/// Diagnostic report from the daemon, similar to Neovim's `:checkhealth`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HealthReport {
    pub daemon_uptime_secs: f64,
    pub version: String,
    pub components: Vec<ComponentHealth>,
}
