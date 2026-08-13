// Copyright (c) 2025 - present
// Author: prjctimg <prjctimg@outlook.com>
// IPC protocol: wire format with explicit cmd/event fields per GTM Protocol v2
//
// This is free software released under the GPL-3.0 license.

use crate::state::{self, DaemonState, EqPreset, RepeatMode, YTFilter};
use crate::track::{LrcData, Playlist, StreamInfo, TrackInfo, YTSearchResult};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;

/// Protocol version this implementation speaks. Matches `prjctimg/gtm.spec`
/// `protocol.md` "Version Negotiation". Bumped only on breaking wire changes.
pub const PROTOCOL_VERSION: u32 = 2;

/// `/queue` sub-commands. Internally tagged via `action` so they serialize
/// flat: `{"action":"add","path":"...","position":null}` per `commands.md`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
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

/// `/library` sub-commands. Internally tagged via `action`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum LibraryAction {
    Scan {
        path: String,
    },
    GetTracks {
        filter: Option<String>,
        sort: Option<String>,
    },
    GetPlaylists,
    GetPlaylistTracks {
        id: i64,
    },
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

/// Wire request: client -> daemon.
///
/// Per `protocol.md`, the on-the-wire shape is:
/// `{"id":<u64>,"cmd":"<name>", ...params}` where `params` is flattened
/// into the top-level object (no wrapper key). We therefore serialize each
/// `DaemonReq` variant as a flat map via `#[serde(untagged)]` and flatten
/// the nested `action` enum for `Queue`/`Library`.
///
/// Deserialization of `DaemonReq` directly from params is ambiguous for
/// unit variants; the daemon must dispatch on the `cmd` string via
/// [`DaemonReq::parse_cmd`] instead.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum DaemonReq {
    Handshake {
        version: u32,
        client: String,
        client_version: Option<String>,
    },
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
    GetVolume,
    ToggleShuffle,
    CycleRepeat {
        mode: RepeatMode,
    },
    ToggleMute,
    Crossfade {
        enabled: bool,
        duration_secs: u8,
        #[serde(skip_serializing_if = "Option::is_none")]
        easing: Option<state::Easing>,
    },
    SetLoudnessMode {
        mode: state::LoudnessMode,
    },
    ScanLoudness {
        track_ids: Option<Vec<i64>>,
        force: Option<bool>,
    },
    SetPreGain {
        pre_gain_db: f32,
    },
    SetGapless {
        enabled: bool,
    },
    SetDynamicMode {
        enabled: bool,
        min_queue_remaining: Option<u32>,
        max_history: Option<u32>,
    },
    SetScrobble {
        enabled: bool,
        api_key: Option<String>,
        session_token: Option<String>,
        min_play_secs: Option<u32>,
        min_play_pct: Option<f32>,
    },
    OrganizeLibrary {
        dry_run: Option<bool>,
    },
    SetEqPreset {
        preset: EqPreset,
    },
    SetEqEnabled {
        enabled: bool,
    },
    SetReverb {
        enabled: bool,
        room_size: f32,
    },
    ListEqPresets,
    Queue {
        #[serde(flatten)]
        action: QueueAction,
    },
    Library {
        #[serde(flatten)]
        action: LibraryAction,
    },
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
        filter: Option<YTFilter>,
    },
    YtSearchPoll,
    YtSearchCancel,
    YtResolveStream {
        url: String,
    },
    YtDownload {
        url: String,
        title: Option<String>,
        channel: Option<String>,
    },
    YtDownloadPoll,
    YtCancelDownload {
        url: String,
    },
    YtFetchPlaylist {
        url: String,
    },
    YtFetchPlaylistPoll,
    YtSetConfig {
        cookie_source: Option<String>,
        cookie_file: Option<String>,
        js_runtime: Option<String>,
        download_dir: Option<String>,
        max_concurrent: Option<u32>,
    },
    GetCoverArt {
        track_id: i64,
    },
    GetLyrics {
        track_id: i64,
    },
    SetSleepTimer {
        minutes: u32,
    },
    CancelSleepTimer,
    GetStatus,
    CheckHealth,
    Ping,
    Quit,
}

impl DaemonReq {
    /// Canonical `cmd` string for this request, as defined by `commands.md`.
    pub fn cmd_name(&self) -> &'static str {
        match self {
            DaemonReq::Handshake { .. } => "handshake",
            DaemonReq::Play { .. } => "play",
            DaemonReq::PlayPause => "play_pause",
            DaemonReq::Pause => "pause",
            DaemonReq::Stop => "stop",
            DaemonReq::Next => "next",
            DaemonReq::Prev => "prev",
            DaemonReq::Seek { .. } => "seek",
            DaemonReq::SetVolume { .. } => "set_volume",
            DaemonReq::GetVolume => "get_volume",
            DaemonReq::ToggleShuffle => "toggle_shuffle",
            DaemonReq::CycleRepeat { .. } => "cycle_repeat",
            DaemonReq::ToggleMute => "toggle_mute",
            DaemonReq::Crossfade { .. } => "crossfade",
            DaemonReq::SetLoudnessMode { .. } => "set_loudness_mode",
            DaemonReq::ScanLoudness { .. } => "scan_loudness",
            DaemonReq::SetPreGain { .. } => "set_pre_gain",
            DaemonReq::SetGapless { .. } => "set_gapless",
            DaemonReq::SetDynamicMode { .. } => "set_dynamic_mode",
            DaemonReq::SetScrobble { .. } => "set_scrobble",
            DaemonReq::OrganizeLibrary { .. } => "organize_library",
            DaemonReq::SetEqPreset { .. } => "set_eq_preset",
            DaemonReq::SetEqEnabled { .. } => "set_eq_enabled",
            DaemonReq::SetReverb { .. } => "set_reverb",
            DaemonReq::ListEqPresets => "list_eq_presets",
            DaemonReq::Queue { .. } => "queue",
            DaemonReq::Library { .. } => "library",
            DaemonReq::Search { .. } => "search",
            DaemonReq::GetFavourites => "get_favourites",
            DaemonReq::AddFavourite { .. } => "add_favourite",
            DaemonReq::RemoveFavourite { .. } => "remove_favourite",
            DaemonReq::YtSearch { .. } => "yt_search",
            DaemonReq::YtSearchPoll => "yt_search_poll",
            DaemonReq::YtSearchCancel => "yt_search_cancel",
            DaemonReq::YtResolveStream { .. } => "yt_resolve_stream",
            DaemonReq::YtDownload { .. } => "yt_download",
            DaemonReq::YtDownloadPoll => "yt_download_poll",
            DaemonReq::YtCancelDownload { .. } => "yt_cancel_download",
            DaemonReq::YtFetchPlaylist { .. } => "yt_fetch_playlist",
            DaemonReq::YtFetchPlaylistPoll => "yt_fetch_playlist_poll",
            DaemonReq::YtSetConfig { .. } => "yt_set_config",
            DaemonReq::GetCoverArt { .. } => "get_cover_art",
            DaemonReq::GetLyrics { .. } => "get_lyrics",
            DaemonReq::SetSleepTimer { .. } => "set_sleep_timer",
            DaemonReq::CancelSleepTimer => "cancel_sleep_timer",
            DaemonReq::GetStatus => "get_status",
            DaemonReq::CheckHealth => "check_health",
            DaemonReq::Ping => "ping",
            DaemonReq::Quit => "quit",
        }
    }

    /// Reconstruct a `DaemonReq` from a parsed `cmd` string and the flat
    /// `params` object delivered on the wire. The dispatch is explicit so
    /// unit variants (`Pause`, `Stop`, ...) disambiguate cleanly — something
    /// `#[serde(untagged)]` cannot do by itself.
    pub fn parse_cmd(cmd: &str, params: Value) -> std::result::Result<Self, String> {
        fn p<T: DeserializeOwned>(v: Value) -> std::result::Result<T, String> {
            serde_json::from_value(v).map_err(|e| e.to_string())
        }
        Ok(match cmd {
            "handshake" => {
                #[derive(Deserialize)]
                struct Params {
                    version: u32,
                    client: String,
                    client_version: Option<String>,
                }
                let x: Params = p(params)?;
                DaemonReq::Handshake {
                    version: x.version,
                    client: x.client,
                    client_version: x.client_version,
                }
            }
            "play" => {
                #[derive(Deserialize)]
                struct Params {
                    path: String,
                    start_pos: f64,
                }
                let x: Params = p(params)?;
                DaemonReq::Play {
                    path: x.path,
                    start_pos: x.start_pos,
                }
            }
            "play_pause" => DaemonReq::PlayPause,
            "pause" => DaemonReq::Pause,
            "stop" => DaemonReq::Stop,
            "next" => DaemonReq::Next,
            "prev" => DaemonReq::Prev,
            "seek" => {
                #[derive(Deserialize)]
                struct Params {
                    position_secs: f64,
                }
                let x: Params = p(params)?;
                DaemonReq::Seek {
                    position_secs: x.position_secs,
                }
            }
            "set_volume" => {
                #[derive(Deserialize)]
                struct Params {
                    volume: u8,
                }
                let x: Params = p(params)?;
                DaemonReq::SetVolume { volume: x.volume }
            }
            "get_volume" => DaemonReq::GetVolume,
            "toggle_shuffle" => DaemonReq::ToggleShuffle,
            "cycle_repeat" => {
                #[derive(Deserialize)]
                struct Params {
                    mode: RepeatMode,
                }
                let x: Params = p(params)?;
                DaemonReq::CycleRepeat { mode: x.mode }
            }
            "toggle_mute" => DaemonReq::ToggleMute,
            "crossfade" => {
                #[derive(Deserialize)]
                struct Params {
                    enabled: bool,
                    duration_secs: u8,
                    easing: Option<state::Easing>,
                }
                let x: Params = p(params)?;
                DaemonReq::Crossfade {
                    enabled: x.enabled,
                    duration_secs: x.duration_secs,
                    easing: x.easing,
                }
            }
            "set_eq_preset" => {
                #[derive(Deserialize)]
                struct Params {
                    preset: EqPreset,
                }
                let x: Params = p(params)?;
                DaemonReq::SetEqPreset { preset: x.preset }
            }
            "set_eq_enabled" => {
                #[derive(Deserialize)]
                struct Params {
                    enabled: bool,
                }
                let x: Params = p(params)?;
                DaemonReq::SetEqEnabled { enabled: x.enabled }
            }
            "set_reverb" => {
                #[derive(Deserialize)]
                struct Params {
                    enabled: bool,
                    room_size: f32,
                }
                let x: Params = p(params)?;
                DaemonReq::SetReverb {
                    enabled: x.enabled,
                    room_size: x.room_size,
                }
            }
            "list_eq_presets" => DaemonReq::ListEqPresets,
            "queue" => DaemonReq::Queue { action: p(params)? },
            "library" => DaemonReq::Library { action: p(params)? },
            "search" => {
                #[derive(Deserialize)]
                struct Params {
                    query: String,
                }
                let x: Params = p(params)?;
                DaemonReq::Search { query: x.query }
            }
            "get_favourites" => DaemonReq::GetFavourites,
            "add_favourite" => {
                #[derive(Deserialize)]
                struct Params {
                    track_id: i64,
                }
                let x: Params = p(params)?;
                DaemonReq::AddFavourite {
                    track_id: x.track_id,
                }
            }
            "remove_favourite" => {
                #[derive(Deserialize)]
                struct Params {
                    track_id: i64,
                }
                let x: Params = p(params)?;
                DaemonReq::RemoveFavourite {
                    track_id: x.track_id,
                }
            }
            "yt_search" => {
                #[derive(Deserialize)]
                struct Params {
                    query: String,
                    filter: Option<YTFilter>,
                }
                let x: Params = p(params)?;
                DaemonReq::YtSearch {
                    query: x.query,
                    filter: x.filter,
                }
            }
            "yt_search_poll" => DaemonReq::YtSearchPoll,
            "yt_search_cancel" => DaemonReq::YtSearchCancel,
            "yt_resolve_stream" => {
                #[derive(Deserialize)]
                struct Params {
                    url: String,
                }
                let x: Params = p(params)?;
                DaemonReq::YtResolveStream { url: x.url }
            }
            "yt_download" => {
                #[derive(Deserialize)]
                struct Params {
                    url: String,
                    title: Option<String>,
                    channel: Option<String>,
                }
                let x: Params = p(params)?;
                DaemonReq::YtDownload {
                    url: x.url,
                    title: x.title,
                    channel: x.channel,
                }
            }
            "yt_download_poll" => DaemonReq::YtDownloadPoll,
            "yt_cancel_download" => {
                #[derive(Deserialize)]
                struct Params {
                    url: String,
                }
                let x: Params = p(params)?;
                DaemonReq::YtCancelDownload { url: x.url }
            }
            "yt_fetch_playlist" => {
                #[derive(Deserialize)]
                struct Params {
                    url: String,
                }
                let x: Params = p(params)?;
                DaemonReq::YtFetchPlaylist { url: x.url }
            }
            "yt_fetch_playlist_poll" => DaemonReq::YtFetchPlaylistPoll,
            "yt_set_config" => {
                #[derive(Deserialize)]
                struct Params {
                    cookie_source: Option<String>,
                    cookie_file: Option<String>,
                    js_runtime: Option<String>,
                    download_dir: Option<String>,
                    max_concurrent: Option<u32>,
                }
                let x: Params = p(params)?;
                DaemonReq::YtSetConfig {
                    cookie_source: x.cookie_source,
                    cookie_file: x.cookie_file,
                    js_runtime: x.js_runtime,
                    download_dir: x.download_dir,
                    max_concurrent: x.max_concurrent,
                }
            }
            "get_cover_art" => {
                #[derive(Deserialize)]
                struct Params {
                    track_id: i64,
                }
                let x: Params = p(params)?;
                DaemonReq::GetCoverArt {
                    track_id: x.track_id,
                }
            }
            "get_lyrics" => {
                #[derive(Deserialize)]
                struct Params {
                    track_id: i64,
                }
                let x: Params = p(params)?;
                DaemonReq::GetLyrics {
                    track_id: x.track_id,
                }
            }
            "set_sleep_timer" => {
                #[derive(Deserialize)]
                struct Params {
                    minutes: u32,
                }
                let x: Params = p(params)?;
                DaemonReq::SetSleepTimer { minutes: x.minutes }
            }
            "cancel_sleep_timer" => DaemonReq::CancelSleepTimer,
            "get_status" => DaemonReq::GetStatus,
            "check_health" => DaemonReq::CheckHealth,
            "ping" => DaemonReq::Ping,
            "set_loudness_mode" => {
                #[derive(Deserialize)]
                struct Params {
                    mode: state::LoudnessMode,
                }
                let x: Params = p(params)?;
                DaemonReq::SetLoudnessMode { mode: x.mode }
            }
            "scan_loudness" => {
                #[derive(Deserialize)]
                struct Params {
                    track_ids: Option<Vec<i64>>,
                    force: Option<bool>,
                }
                let x: Params = p(params)?;
                DaemonReq::ScanLoudness {
                    track_ids: x.track_ids,
                    force: x.force,
                }
            }
            "set_pre_gain" => {
                #[derive(Deserialize)]
                struct Params {
                    pre_gain_db: f32,
                }
                let x: Params = p(params)?;
                DaemonReq::SetPreGain {
                    pre_gain_db: x.pre_gain_db,
                }
            }
            "set_gapless" => {
                #[derive(Deserialize)]
                struct Params {
                    enabled: bool,
                }
                let x: Params = p(params)?;
                DaemonReq::SetGapless { enabled: x.enabled }
            }
            "set_dynamic_mode" => {
                #[derive(Deserialize)]
                struct Params {
                    enabled: bool,
                    min_queue_remaining: Option<u32>,
                    max_history: Option<u32>,
                }
                let x: Params = p(params)?;
                DaemonReq::SetDynamicMode {
                    enabled: x.enabled,
                    min_queue_remaining: x.min_queue_remaining,
                    max_history: x.max_history,
                }
            }
            "set_scrobble" => {
                #[derive(Deserialize)]
                struct Params {
                    enabled: bool,
                    api_key: Option<String>,
                    session_token: Option<String>,
                    min_play_secs: Option<u32>,
                    min_play_pct: Option<f32>,
                }
                let x: Params = p(params)?;
                DaemonReq::SetScrobble {
                    enabled: x.enabled,
                    api_key: x.api_key,
                    session_token: x.session_token,
                    min_play_secs: x.min_play_secs,
                    min_play_pct: x.min_play_pct,
                }
            }
            "organize_library" => {
                #[derive(Deserialize)]
                struct Params {
                    dry_run: Option<bool>,
                }
                let x: Params = p(params)?;
                DaemonReq::OrganizeLibrary { dry_run: x.dry_run }
            }
            "quit" => DaemonReq::Quit,
            other => return Err(format!("unknown command: {other}")),
        })
    }
}

/// Wire request: client -> daemon.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WireReq {
    pub id: u64,
    pub cmd: String,
    #[serde(flatten)]
    pub params: Value,
}

/// Wire response: daemon -> client.
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

/// Wire event: daemon -> client (broadcast).
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
#[serde(tag = "event")]
pub enum DaemonEvent {
    #[serde(rename = "playback_started")]
    PlaybackStarted {
        track: TrackInfo,
        auto_advanced: bool,
        time_pos: f64,
        duration: f64,
    },
    #[serde(rename = "playback_paused")]
    PlaybackPaused { time_pos: f64 },
    #[serde(rename = "playback_stopped")]
    PlaybackStopped,
    #[serde(rename = "track_ended")]
    TrackEnded,
    #[serde(rename = "position_changed")]
    PositionChanged { time_pos: f64 },
    #[serde(rename = "duration_changed")]
    DurationChanged { duration: f64 },
    #[serde(rename = "volume_changed")]
    VolumeChanged { volume: u8 },
    #[serde(rename = "metadata_changed")]
    MetadataChanged { detail: String },
    #[serde(rename = "queue_changed")]
    QueueChanged { queue: Vec<TrackInfo>, cursor: u128 },
    #[serde(rename = "queue_index_changed")]
    QueueIndexChanged { index: u128 },
    #[serde(rename = "repeat_mode_changed")]
    RepeatModeChanged { mode: RepeatMode },
    #[serde(rename = "shuffle_changed")]
    ShuffleChanged { enabled: bool },
    #[serde(rename = "crossfade_changed")]
    CrossfadeChanged {
        enabled: bool,
        duration_secs: u8,
        #[serde(skip_serializing_if = "Option::is_none")]
        easing: Option<state::Easing>,
    },
    #[serde(rename = "loudness_mode_changed")]
    LoudnessModeChanged { mode: state::LoudnessMode },
    #[serde(rename = "loudness_scan_progress")]
    LoudnessScanProgress {
        tracks_remaining: u32,
        tracks_total: u32,
        #[serde(skip_serializing_if = "Option::is_none")]
        current_track: Option<TrackInfo>,
    },
    #[serde(rename = "loudness_scan_done")]
    LoudnessScanDone { scanned: u32, failed: u32 },
    #[serde(rename = "pre_gain_changed")]
    PreGainChanged { pre_gain_db: f32 },
    #[serde(rename = "gapless_changed")]
    GaplessChanged { enabled: bool },
    #[serde(rename = "dynamic_mode_changed")]
    DynamicModeChanged {
        enabled: bool,
        min_queue_remaining: u32,
        max_history: u32,
    },
    #[serde(rename = "scrobble_config_changed")]
    ScrobbleConfigChanged { enabled: bool },
    #[serde(rename = "library_organized")]
    LibraryOrganized {
        moves_succeeded: u32,
        moves_failed: u32,
    },
    #[serde(rename = "sleep_timer_tick")]
    SleepTimerTick { remaining_secs: u32 },
    #[serde(rename = "sleep_timer_expired")]
    SleepTimerExpired,
    #[serde(rename = "eq_preset_changed")]
    EqPresetChanged { preset: EqPreset },
    #[serde(rename = "eq_enabled_changed")]
    EqEnabledChanged { enabled: bool },
    #[serde(rename = "reverb_changed")]
    ReverbChanged { enabled: bool, room_size: f32 },
    #[serde(rename = "custom")]
    Custom {
        name: String,
        data: HashMap<String, String>,
    },
    #[serde(rename = "heartbeat")]
    Heartbeat,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum DaemonRes {
    Ok,
    Value {
        value: Value,
    },
    Tracks {
        tracks: Vec<TrackInfo>,
    },
    QueueState {
        queue: Vec<TrackInfo>,
        cursor: u128,
    },
    Status {
        state: Box<DaemonState>,
    },
    Playlists {
        playlists: Vec<Playlist>,
    },
    YtSearchResults {
        results: Vec<YTSearchResult>,
    },
    StreamInfo {
        info: Box<StreamInfo>,
    },
    Lyrics {
        lyrics: Option<LrcData>,
    },
    CoverArt {
        data: Option<String>,
    },
    SyncCoversResult {
        synced: usize,
        total: usize,
    },
    SyncLyricsResult {
        synced: usize,
        total: usize,
    },
    Pong,
    HealthReport {
        report: Box<HealthReport>,
    },
    EqPresets {
        presets: Vec<String>,
    },
    Handshake {
        version: u32,
        daemon: String,
        daemon_version: String,
    },
    Error {
        message: String,
    },
}

impl DaemonRes {
    /// Serialize this typed response into a `WireRes` for the command socket.
    pub fn to_wire(self, id: u64) -> WireRes {
        let data = match self {
            DaemonRes::Ok => None,
            DaemonRes::Value { value } => Some(value),
            DaemonRes::Tracks { tracks } => Some(serde_json::json!({ "tracks": tracks })),
            DaemonRes::QueueState { queue, cursor } => {
                Some(serde_json::json!({ "queue": queue, "cursor": cursor }))
            }
            DaemonRes::Status { state } => Some(serde_json::json!({ "state": state })),
            DaemonRes::Playlists { playlists } => {
                Some(serde_json::json!({ "playlists": playlists }))
            }
            DaemonRes::YtSearchResults { results } => {
                Some(serde_json::json!({ "results": results }))
            }
            DaemonRes::StreamInfo { info } => Some(serde_json::json!({ "info": info })),
            DaemonRes::Lyrics { lyrics } => Some(serde_json::json!({ "lyrics": lyrics })),
            DaemonRes::CoverArt { data } => Some(serde_json::json!({ "data": data })),
            DaemonRes::SyncCoversResult { synced, total } => {
                Some(serde_json::json!({ "synced": synced, "total": total }))
            }
            DaemonRes::SyncLyricsResult { synced, total } => {
                Some(serde_json::json!({ "synced": synced, "total": total }))
            }
            DaemonRes::HealthReport { report } => Some(serde_json::json!({ "report": report })),
            DaemonRes::EqPresets { presets } => Some(serde_json::json!({ "presets": presets })),
            DaemonRes::Handshake {
                version,
                daemon,
                daemon_version,
            } => Some(serde_json::json!({
                "version": version,
                "daemon": daemon,
                "daemon_version": daemon_version,
            })),
            DaemonRes::Error { message } => return WireRes::err(id, message),
            DaemonRes::Pong => None,
        };

        WireRes::ok(id, data)
    }

    /// Reconstruct a typed `DaemonRes` from a parsed `WireRes` keyed by the
    /// `cmd` string of the request that produced it.
    pub fn from_wire(cmd: &str, wire: &WireRes) -> Self {
        match (wire.ok, &wire.error) {
            (Some(false), Some(msg)) => DaemonRes::Error {
                message: msg.clone(),
            },
            (Some(false), None) => DaemonRes::Error {
                message: "unknown error".into(),
            },
            (Some(true), _) => {
                let data = wire.data.clone().unwrap_or(Value::Null);
                Self::ok_from_data(cmd, data)
            }
            _ => DaemonRes::Error {
                message: "malformed response".into(),
            },
        }
    }

    fn ok_from_data(cmd: &str, data: Value) -> Self {
        match cmd {
            "handshake" => {
                #[derive(Deserialize)]
                struct D {
                    version: u32,
                    daemon: String,
                    daemon_version: String,
                }
                match serde_json::from_value::<D>(data.clone()) {
                    Ok(d) => DaemonRes::Handshake {
                        version: d.version,
                        daemon: d.daemon,
                        daemon_version: d.daemon_version,
                    },
                    Err(_) => DaemonRes::Value { value: data },
                }
            }
            "get_status" => match serde_json::from_value::<Box<DaemonState>>(
                data.get("state").cloned().unwrap_or(Value::Null),
            ) {
                Ok(state) => DaemonRes::Status { state },
                Err(_) => DaemonRes::Value { value: data },
            },
            "queue" => {
                let queue = data.get("queue").cloned().unwrap_or(Value::Null);
                let cursor = data.get("cursor").and_then(|c| c.as_u64()).unwrap_or(0) as u128;
                match serde_json::from_value::<Vec<TrackInfo>>(queue) {
                    Ok(queue) => DaemonRes::QueueState { queue, cursor },
                    Err(_) => DaemonRes::Value { value: data },
                }
            }
            "search" | "get_favourites" | "library" => {
                let tracks = data.get("tracks").cloned().unwrap_or(Value::Null);
                match serde_json::from_value::<Vec<TrackInfo>>(tracks) {
                    Ok(tracks) => DaemonRes::Tracks { tracks },
                    Err(_) => DaemonRes::Value { value: data },
                }
            }
            "yt_search_poll" => {
                let results = data.get("results").cloned().unwrap_or(Value::Null);
                match serde_json::from_value::<Vec<YTSearchResult>>(results) {
                    Ok(results) => DaemonRes::YtSearchResults { results },
                    Err(_) => DaemonRes::Value { value: data },
                }
            }
            "yt_resolve_stream" => {
                let info = data.get("info").cloned().unwrap_or(Value::Null);
                match serde_json::from_value::<Box<StreamInfo>>(info) {
                    Ok(info) => DaemonRes::StreamInfo { info },
                    Err(_) => DaemonRes::Value { value: data },
                }
            }
            "get_cover_art" => {
                let d = data.get("data").cloned().unwrap_or(Value::Null);
                match serde_json::from_value::<Option<String>>(d) {
                    Ok(data) => DaemonRes::CoverArt { data },
                    Err(_) => DaemonRes::Value { value: data },
                }
            }
            "get_lyrics" => {
                let lyrics = data.get("lyrics").cloned().unwrap_or(Value::Null);
                match serde_json::from_value::<Option<LrcData>>(lyrics) {
                    Ok(lyrics) => DaemonRes::Lyrics { lyrics },
                    Err(_) => DaemonRes::Value { value: data },
                }
            }
            "check_health" => {
                let report = data.get("report").cloned().unwrap_or(Value::Null);
                match serde_json::from_value::<Box<HealthReport>>(report) {
                    Ok(report) => DaemonRes::HealthReport { report },
                    Err(_) => DaemonRes::Value { value: data },
                }
            }
            "list_eq_presets" => {
                let presets = data.get("presets").cloned().unwrap_or(Value::Null);
                match serde_json::from_value::<Vec<String>>(presets) {
                    Ok(presets) => DaemonRes::EqPresets { presets },
                    Err(_) => DaemonRes::Value { value: data },
                }
            }
            "ping" => DaemonRes::Pong,
            _ => {
                if data.is_null() {
                    DaemonRes::Ok
                } else {
                    DaemonRes::Value { value: data }
                }
            }
        }
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
