# 01 — gtm-core: Shared Types & IPC Protocol

## Purpose

Defines every type shared across the workspace: IPC request/response enums, daemon event structs,
the binary wire protocol, track metadata, LRC lyrics data, and daemon state snapshot.

Depends on: `serde`, `serde_json`, `bincode`, `thiserror`, `chrono`, `uuid`

Used by: all crates

## Serde Tag Strategy

All enums use `#[serde(tag = "type", content = "data")]` (internally tagged) for JSON
serialization. Binary (bincode) uses the default enum representation (u32 discriminant).

```rust
// In Cargo.toml:
// serde = { version = "1", features = ["derive"] }
// serde_json = "1"
// bincode = "1"
// thiserror = "2"
// chrono = { version = "0.4", features = ["serde"] }
// uuid = { version = "1", features = ["v4", "serde"] }
```

## Primitives

```rust
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PlaybackStatus {
    Stopped,
    Playing,
    Paused,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RepeatMode {
    Off,
    One,
    All,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum YtFilter {
    Song,
    Video,
    Playlist,
    Channel,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum UIMode {
    Normal,
    Filter,
    Command,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ThemeMode {
    Dark,
    Light,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Tab {
    NowPlaying,
    Library,
    Queue,
    YouTube,
    Settings,
    Help,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ImageData {
    pub data: Vec<u8>,
    pub mime: String,       // "image/jpeg", "image/png"
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CrossfadeConfig {
    pub enabled: bool,
    pub duration_secs: u8,  // default 3, max 30
}
```

## IPC Protocol — Request/Response

```
 ┌─────────┐          JSON line            ┌─────────┐
 │  gtm    │  ──── DaemonRequest ────────▶ │  gtmd   │
 │ (client)│                               │ (daemon)│
 │         │  ◀──── DaemonResponse ──────── │         │
 └─────────┘          JSON line            └─────────┘
```

### DaemonRequest (client → daemon)

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "cmd", content = "args")]
pub enum DaemonRequest {
    // ─── Playback ───
    Play { path: String },
    PlayPause,
    Stop,
    Next,
    Prev,
    Seek { position_secs: f64 },
    SetVolume { volume: u8 },           // 0-100
    ToggleShuffle,
    CycleRepeat { mode: RepeatMode },
    ToggleMute,
    Crossfade { enabled: bool, duration_secs: u8 },

    // ─── Queue ───
    Queue { action: QueueAction },

    // ─── Library ───
    Library { action: LibraryAction },

    // ─── Discovery ───
    Search { query: String },
    GetFavourites,
    AddFavourite { track_id: i64 },
    RemoveFavourite { track_id: i64 },
    YtSearch { query: String, filter: Option<YtFilter> },
    YtSearchPoll,
    YtSearchCancel,
    YtResolveStream { url: String },

    // ─── System ───
    GetStatus,
    Ping,
    Quit,
}
```

### DaemonResponse (daemon → client)

Every request gets exactly one response JSON line back.

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", content = "data")]
pub enum DaemonResponse {
    Ok { version: u32 },
    Value { version: u32, value: serde_json::Value },
    Tracks { version: u32, tracks: Vec<TrackInfo> },
    QueueState { version: u32, tracks: Vec<TrackInfo>, cursor: usize },
    Status { version: u32, state: Box<DaemonState> },
    Playlists { version: u32, playlists: Vec<Playlist> },
    YtSearchResults { version: u32, results: Vec<YtSearchResult> },
    StreamInfo { version: u32, info: Box<StreamInfo> },
    Lyrics { version: u32, lyrics: Option<LrcData> },
    Pong,
    Error { version: u32, message: String },
}
```

### Queue actions

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "action", content = "args")]
pub enum QueueAction {
    List,
    Clear,
    Remove { index: usize },
    Move { from: usize, to: usize },
    Add { path: String, position: Option<usize> },
    AddMany { paths: Vec<String> },
    Set { paths: Vec<String>, start_index: usize },
}
```

### Library actions

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "action", content = "args")]
pub enum LibraryAction {
    Scan { path: String },
    GetTracks { filter: Option<String>, sort: Option<String> },
    GetPlaylists,
    CreatePlaylist { name: String },
    DeletePlaylist { id: i64 },
    AddToPlaylist { playlist_id: i64, track_ids: Vec<i64> },
    ImportM3u { path: String },
    GetRecent { count: usize },
}
```

## Binary Wire Protocol (DaemonEvents)

```
 ┌─────────┐    binary frames over same socket    ┌─────────┐
 │  gtm    │  ◀══════ DaemonEvent stream ═══════   │  gtmd   │
 │ (client)│                                       │ (daemon)│
 └─────────┘                                       └─────────┘
```

### WireFrame (bincode wrapper)

```rust
/// A single frame on the wire. The daemon may batch multiple
/// DaemonEvents into one WireFrame (up to 255 events).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WireFrame {
    pub version: u32,                // wire format version = 1
    pub events: Vec<DaemonEvent>,
}
```

### On-wire encoding

```
┌──────────────────────────────────────────────────────────────────┐
│ total_len: u32 BE (bytes 4..end, NOT including these 4 bytes)     │
├──────────────────────────────────────────────────────────────────┤
│ frame: bincode(WireFrame)                                         │
│  ┌─ version: u32 LE (bincode default = 1)                        │
│  ├─ events: Vec<DaemonEvent>                                     │
│  │   ├─ length: u64 LE (number of events)                        │
│  │   ├─ event[0]: bincode(DaemonEvent)                           │
│  │   ├─ event[1]: ...                                            │
│  │   └─ ...                                                      │
│  └───────────────────────────────────────────────────────────────│
└──────────────────────────────────────────────────────────────────┘
```

### Encode / Decode functions

```rust
use std::io::{Read, Write};

/// Serialize events into a byte buffer with length prefix.
/// Format: [total_len: u32 BE][bincode(WireFrame)]
pub fn encode_frame(events: &[DaemonEvent]) -> Result<Vec<u8>, bincode::Error> {
    let frame = WireFrame { version: 1, events: events.to_vec() };
    let payload = bincode::serialize(&frame)?;
    let len = payload.len() as u32;
    let mut buf = Vec::with_capacity(4 + payload.len());
    buf.extend_from_slice(&len.to_be_bytes());
    buf.extend_from_slice(&payload);
    Ok(buf)
}

/// Try to decode one frame from a byte buffer.
/// Returns (frame, bytes_consumed) or None if not enough data.
pub fn decode_frame(buf: &[u8]) -> Result<Option<(WireFrame, usize)>, bincode::Error> {
    if buf.len() < 4 {
        return Ok(None);
    }
    let total_len = u32::from_be_bytes([buf[0], buf[1], buf[2], buf[3]]) as usize;
    if buf.len() < 4 + total_len {
        return Ok(None);
    }
    let frame: WireFrame = bincode::deserialize(&buf[4..4 + total_len])?;
    Ok(Some((frame, 4 + total_len)))
}
```

### DaemonEvent (daemon → client, streamed)

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "event", content = "data")]
pub enum DaemonEvent {
    PlaybackStarted {
        track: TrackInfo,
        auto_advanced: bool,
        time_pos: f64,
        duration: f64,
    },
    PlaybackPaused,
    PlaybackStopped,
    TrackEnded,
    PositionChanged { time_pos: f64 },
    DurationChanged { duration: f64 },
    VolumeChanged { volume: u8 },
    MetadataChanged { event: String },         // "cover", "lyrics", "tags"
    QueueChanged { queue: Vec<TrackInfo>, cursor: usize },
    QueueIndexChanged { index: usize },
    RepeatModeChanged { mode: RepeatMode },
    ShuffleChanged { enabled: bool },
    SleepTimerTick { remaining_secs: u32 },
    Custom { name: String, data: HashMap<String, String> },
}
```

## Track & Library Types

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrackInfo {
    pub id: i64,
    pub path: String,
    pub title: String,
    pub artist: String,
    pub album: String,
    pub duration: f64,
    pub track_number: Option<i32>,
    pub genre: String,                  // comma-separated, never null
    pub year: Option<i32>,
    pub bitrate: Option<i32>,
    pub samplerate: Option<i32>,
    pub hash: String,                   // SHA-256 of first 64KB
    pub cover_path: Option<String>,
    pub favourite: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Playlist {
    pub id: i64,
    pub name: String,
    pub created_at: String,             // ISO 8601
    pub track_count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LrcLine {
    pub timestamp: f64,                 // seconds
    pub text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LrcData {
    pub title: Option<String>,
    pub artist: Option<String>,
    pub album: Option<String>,
    pub lines: Vec<LrcLine>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct YtSearchResult {
    pub id: String,                     // YouTube video ID
    pub title: String,
    pub url: String,                    // https://youtube.com/watch?v=<id>
    pub channel: String,
    pub duration: f64,                  // seconds
    pub views: u64,
    pub thumbnail: Option<String>,      // URL
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StreamInfo {
    pub url: String,                    // direct media URL
    pub title: String,
    pub ext: String,                    // "webm", "m4a"
    pub duration: f64,
}
```

## DaemonState (snapshot)

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DaemonState {
    pub version: u32,                   // monotonic counter, incremented on every change
    pub status: PlaybackStatus,
    pub queue: Vec<TrackInfo>,
    pub queue_cursor: usize,            // index into queue for current/normal mode
    pub volume: u8,                     // 0-100
    pub repeat: RepeatMode,
    pub shuffle: bool,
    pub mute: bool,
    pub crossfade: Option<CrossfadeConfig>,
    pub current_track: Option<TrackInfo>,
    pub time_pos: f64,                  // seconds
    pub duration: f64,                  // seconds
    pub sleep_timer: Option<u32>,       // remaining seconds
}
```

## CoreError

```rust
#[derive(Debug, thiserror::Error)]
pub enum CoreError {
    #[error("serialization error: {0}")]
    Serialization(#[from] serde_json::Error),
    #[error("bincode error: {0}")]
    Bincode(#[from] bincode::Error),
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("daemon error: {0}")]
    Daemon(String),
    #[error("not connected")]
    NotConnected,
    #[error("timeout")]
    Timeout,
}

pub type Result<T> = std::result::Result<T, CoreError>;
```

## File Structure

```
gtm-core/src/
├── lib.rs           # re-exports all public types
├── ipc.rs           # DaemonRequest, DaemonResponse, DaemonEvent, QueueAction, LibraryAction
├── wire.rs          # WireFrame, encode_frame, decode_frame
├── track.rs         # TrackInfo, Playlist, LrcLine, LrcData, YtSearchResult, StreamInfo
└── state.rs         # DaemonState, PlaybackStatus, RepeatMode, CrossfadeConfig,
                     #   YtFilter, UIMode, ThemeMode, ImageData, Tab, CoreError
```
