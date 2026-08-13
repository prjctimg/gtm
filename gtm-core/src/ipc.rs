// Copyright (c) 2025 - present
// Author: prjctimg <prjctimg@outlook.com>
// IPC protocol: wire format with explicit cmd/event fields per GTM Protocol v1
//
// This is free software released under the GPL-3.0 license.

use crate::state::{self, DaemonState, RepeatMode};
use crate::track::{LrcData, Playlist, StreamInfo, TrackInfo, YTSearchResult};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LibraryAction {
    Scan { path: String },
    GetTracks { filter: Option<String>, sort: Option<String> },
    GetPlaylists,
    CreatePlaylist { name: String },
    DeletePlaylist { id: i64 },
    AddToPlaylist { playlist_id: i64, track_ids: Vec<i64> },
    ImportM3u { path: String },
    ExportM3u { playlist_id: i64, path: String },
    GetRecent { count: u128 },
    SyncCovers,
    SyncLyrics,
    RemoveFromPlaylist { playlist_id: i64, track_id: i64 },
    RemoveTrack { id: i64 },
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
    Remove { index: u128 },
    Move { from: u128, to: u128 },
    Add { path: String, position: Option<u128> },
    AddMany { paths: Vec<String> },
    AddFolder { path: String },
    Set { paths: Vec<String>, start_idx: u128 },
}

/// Wire request: client -> daemon
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WireReq {
    pub id: u64,
    pub cmd: String,
    #[serde(flatten)]
    pub params: Value,
}

/// Wire response: daemon -> client (success)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WireRes {
    pub id: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ok: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(flatten)]
    pub data: Option<Value>,
}

impl WireRes {
    pub fn ok(id: u64, data: Option<Value>) -> Self {
        Self {
            id,
            ok: Some(true),
            error: None,
            data,
        }
    }

    pub fn err(id: u64, error: String) -> Self {
        Self {
            id,
            ok: Some(false),
            error: Some(error),
            data: None,
        }
    }
}

/// Wire event: daemon -> client (broadcast)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WireEvent {
    pub event: String,
    #[serde(flatten)]
    pub data: Value,
}

impl WireEvent {
    pub fn new(event: &str, data: Value) -> Self {
        Self {
            event: event.to_string(),
            data,
        }
    }
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
    PlaybackPaused { time_pos: f64 },
    PlaybackStopped,
    TrackEnded,
    PositionChanged { time_pos: f64 },
    DurationChanged { duration: f64 },
    VolumeChanged { volume: u8 },
    MetadataChanged { event: String },
    QueueChanged { queue: Vec<TrackInfo>, cursor: u128 },
    QueueIndexChanged { index: u128 },
    RepeatModeChanged { mode: RepeatMode },
    ShuffleChanged { enabled: bool },
    CrossfadeChanged { enabled: bool, duration_secs: u8 },
    SleepTimerTick { remaining_secs: u32 },
    SleepTimerExpired,
    EqPresetChanged { preset: state::EqPreset },
    EqEnabledChanged { enabled: bool },
    ReverbChanged { enabled: bool, room_size: f32 },
    Custom { name: String, data: HashMap<String, String> },
    Heartbeat,
}

impl DaemonEvent {
    pub fn to_wire_event(&self) -> WireEvent {
        match self {
            DaemonEvent::PlaybackStarted { track, auto_advanced, time_pos, duration } => {
                WireEvent::new("playback_started", serde_json::json!({
                    "track": track,
                    "auto_advanced": auto_advanced,
                    "time_pos": time_pos,
                    "duration": duration,
                }))
            }
            DaemonEvent::PlaybackPaused { time_pos } => {
                WireEvent::new("playback_paused", serde_json::json!({ "time_pos": time_pos }))
            }
            DaemonEvent::PlaybackStopped => {
                WireEvent::new("playback_stopped", serde_json::json!({}))
            }
            DaemonEvent::TrackEnded => {
                WireEvent::new("track_ended", serde_json::json!({}))
            }
            DaemonEvent::PositionChanged { time_pos } => {
                WireEvent::new("position_changed", serde_json::json!({ "time_pos": time_pos }))
            }
            DaemonEvent::DurationChanged { duration } => {
                WireEvent::new("duration_changed", serde_json::json!({ "duration": duration }))
            }
            DaemonEvent::VolumeChanged { volume } => {
                WireEvent::new("volume_changed", serde_json::json!({ "volume": volume }))
            }
            DaemonEvent::MetadataChanged { event } => {
                WireEvent::new("metadata_changed", serde_json::json!({ "event": event }))
            }
            DaemonEvent::QueueChanged { queue, cursor } => {
                WireEvent::new("queue_changed", serde_json::json!({ "queue": queue, "cursor": cursor }))
            }
            DaemonEvent::QueueIndexChanged { index } => {
                WireEvent::new("queue_index_changed", serde_json::json!({ "index": index }))
            }
            DaemonEvent::RepeatModeChanged { mode } => {
                WireEvent::new("repeat_mode_changed", serde_json::json!({ "mode": mode }))
            }
            DaemonEvent::ShuffleChanged { enabled } => {
                WireEvent::new("shuffle_changed", serde_json::json!({ "enabled": enabled }))
            }
            DaemonEvent::CrossfadeChanged { enabled, duration_secs } => {
                WireEvent::new("crossfade_changed", serde_json::json!({
                    "enabled": enabled,
                    "duration_secs": duration_secs,
                }))
            }
            DaemonEvent::SleepTimerTick { remaining_secs } => {
                WireEvent::new("sleep_timer_tick", serde_json::json!({ "remaining_secs": remaining_secs }))
            }
            DaemonEvent::SleepTimerExpired => {
                WireEvent::new("sleep_timer_expired", serde_json::json!({}))
            }
            DaemonEvent::EqPresetChanged { preset } => {
                WireEvent::new("eq_preset_changed", serde_json::json!({ "preset": preset }))
            }
            DaemonEvent::EqEnabledChanged { enabled } => {
                WireEvent::new("eq_enabled_changed", serde_json::json!({ "enabled": enabled }))
            }
            DaemonEvent::ReverbChanged { enabled, room_size } => {
                WireEvent::new("reverb_changed", serde_json::json!({
                    "enabled": enabled,
                    "room_size": room_size,
                }))
            }
            DaemonEvent::Custom { name, data } => {
                let mut map = serde_json::json!({ "name": name });
                if let serde_json::Value::Object(obj) = &mut map {
                    for (k, v) in data {
                        obj.insert(k.clone(), serde_json::Value::String(v.clone()));
                    }
                }
                WireEvent::new("custom", map)
            }
            DaemonEvent::Heartbeat => {
                WireEvent::new("heartbeat", serde_json::json!({}))
            }
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DaemonReq {
    // Playback
    Play { path: String, start_pos: f64 },
    PlayPause,
    Pause,
    Stop,
    Next,
    Prev,
    Seek { position_secs: f64 },
    SetVolume { volume: u8 },
    GetVolume,
    ToggleShuffle,
    CycleRepeat { mode: state::RepeatMode },
    ToggleMute,
    Crossfade { enabled: bool, duration_secs: u8 },
    SetCrossfadeEasing { easing: state::Easing },
    SetEqPreset { preset: state::EqPreset },
    SetEqEnabled { enabled: bool },
    SetReverb { enabled: bool, room_size: f32 },
    ListEqPresets,

    // Queue
    Queue { action: QueueAction },

    // Library
    Library { action: LibraryAction },

    // Discovery
    Search { query: String },
    GetFavourites,
    AddFavourite { track_id: i64 },
    RemoveFavourite { track_id: i64 },
    YtSearch { query: String, filter: Option<state::YTFilter> },
    YtSearchPoll,
    YtSearchCancel,
    YtResolveStream { url: String },
    YtDownload { url: String, title: Option<String>, channel: Option<String> },
    YtDownloadPoll,
    YtCancelDownload { url: String },
    YtFetchPlaylist { url: String },
    YtFetchPlaylistPoll,
    YtSetConfig { cookie_source: Option<String>, js_runtime: Option<String>, download_dir: Option<String>, max_concurrent: Option<u32> },

    // Cover Art
    GetCoverArt { track_id: i64 },

    // Lyrics
    GetLyrics { track_id: i64 },

    // Sleep Timer
    SetSleepTimer { minutes: u32 },
    CancelSleepTimer,

    // System
    GetStatus,
    CheckHealth,
    Ping,
    Quit,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DaemonRes {
    Ok { version: u32 },
    Value { version: u32, value: Value },
    Tracks { version: u32, tracks: Vec<TrackInfo> },
    QueueState { version: u32, tracks: Vec<TrackInfo>, cursor: u128 },
    Status { version: u32, state: Box<DaemonState> },
    Playlists { version: u32, playlists: Vec<Playlist> },
    YtSearchResults { version: u32, results: Vec<YTSearchResult> },
    StreamInfo { version: u32, info: Box<StreamInfo> },
    Lyrics { version: u32, lyrics: Option<LrcData> },
    CoverArt { version: u32, data: Option<String> },
    SyncCoversResult { version: u32, synced: usize, total: usize },
    SyncLyricsResult { version: u32, synced: usize, total: usize },
    Pong,
    HealthReport { version: u32, report: Box<HealthReport> },
    EqPresets { version: u32, presets: Vec<String> },
    Error { version: u32, message: String },
}

impl DaemonRes {
    pub fn to_wire(self, id: u64) -> WireRes {
        let version = match &self {
            DaemonRes::Ok { version } => *version,
            DaemonRes::Value { version, .. } => *version,
            DaemonRes::Tracks { version, .. } => *version,
            DaemonRes::QueueState { version, .. } => *version,
            DaemonRes::Status { version, .. } => *version,
            DaemonRes::Playlists { version, .. } => *version,
            DaemonRes::YtSearchResults { version, .. } => *version,
            DaemonRes::StreamInfo { version, .. } => *version,
            DaemonRes::Lyrics { version, .. } => *version,
            DaemonRes::CoverArt { version, .. } => *version,
            DaemonRes::SyncCoversResult { version, .. } => *version,
            DaemonRes::SyncLyricsResult { version, .. } => *version,
            DaemonRes::HealthReport { version, .. } => *version,
            DaemonRes::EqPresets { version, .. } => *version,
            DaemonRes::Error { version, .. } => *version,
            DaemonRes::Pong => 0,
        };

        let data = match self {
            DaemonRes::Ok { .. } => None,
            DaemonRes::Value { value, .. } => Some(value),
            DaemonRes::Tracks { tracks, .. } => Some(serde_json::json!({ "tracks": tracks })),
            DaemonRes::QueueState { tracks, cursor, .. } => Some(serde_json::json!({ "tracks": tracks, "cursor": cursor })),
            DaemonRes::Status { state, .. } => Some(serde_json::json!({ "state": state })),
            DaemonRes::Playlists { playlists, .. } => Some(serde_json::json!({ "playlists": playlists })),
            DaemonRes::YtSearchResults { results, .. } => Some(serde_json::json!({ "results": results })),
            DaemonRes::StreamInfo { info, .. } => Some(serde_json::json!({ "info": info })),
            DaemonRes::Lyrics { lyrics, .. } => Some(serde_json::json!({ "lyrics": lyrics })),
            DaemonRes::CoverArt { data, .. } => Some(serde_json::json!({ "data": data })),
            DaemonRes::SyncCoversResult { synced, total, .. } => Some(serde_json::json!({ "synced": synced, "total": total })),
            DaemonRes::SyncLyricsResult { synced, total, .. } => Some(serde_json::json!({ "synced": synced, "total": total })),
            DaemonRes::HealthReport { report, .. } => Some(serde_json::json!({ "report": report })),
            DaemonRes::EqPresets { presets, .. } => Some(serde_json::json!({ "presets": presets })),
            DaemonRes::Error { message, .. } => return WireRes::err(id, message),
            DaemonRes::Pong => None,
        };

        WireRes::ok(id, data)
    }
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