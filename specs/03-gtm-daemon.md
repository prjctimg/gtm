# 03 — gtm-daemon: Background Audio Daemon

## Purpose

The daemon (`gtmd`) owns all audio state, manages the playback queue, communicates with the
library database, handles yt-dlp streaming, and broadcasts state changes to connected clients.
It is the single source of truth.

Depends on: `gtm-core`, `gtm-audio`, `gtm-mpris` (optional), `rusqlite`, `tokio`, `reqwest`, `tracing`, `clap`

> **Status**: 🔶 Partial implementation. Core daemon + audio backend + IPC are complete.
> Library/queue/yt/cover/lyrics subsystems have stub files only — details in [05](05-gtm-daemon-features.md).

## Daemon Struct (current implementation)

```rust
pub struct Daemon {
    pub state: Arc<RwLock<DaemonState>>,
    pub backend: Box<dyn AudioBackend>,
    pub listener: UnixListener,
    pub config: DaemonConfig,
    pub event_tx: broadcast::Sender<DaemonEvent>,
    req_tx: mpsc::UnboundedSender<(ClientId, DaemonReq)>,
    req_rx: mpsc::UnboundedReceiver<(ClientId, DaemonReq)>,
    next_client_id: ClientId,
}
```

### Aspirational fields (not yet implemented)
- `library: Option<Library>` — see [04-gtm-daemon-library.md](04-gtm-daemon-library.md)
- `clients: Vec<ClientHandle>` — per-client writer tracking for event broadcast
- `yt_manager: YoutubeManager` — see [05-gtm-daemon-features.md](05-gtm-daemon-features.md)
- `cover_cache: CoverCache` — see [05-gtm-daemon-features.md](05-gtm-daemon-features.md)
- `lyrics_manager: LyricsManager` — see [05-gtm-daemon-features.md](05-gtm-daemon-features.md)
- `sleep_timer` / `sleep_timer_future`
- `version: u32` — monotonic state version counter

## DaemonConfig

```rust
pub struct DaemonConfig {
    pub socket_path: PathBuf,
    pub library_path: PathBuf,
    pub config_dir: PathBuf,
    pub cache_dir: PathBuf,
    pub data_dir: PathBuf,
    pub log_file: Option<PathBuf>,
    pub verbose: bool,
    pub test_mode: bool,     // true = ephemeral socket, no daemonize
    pub audio_backend: AudioBackendKind,
}

pub enum AudioBackendKind {
    Symphonia,     // pure-Rust via symphonia 0.6 + symphonia-adapter-libopus
    Ffmpeg,        // ffmpeg CLI subprocess fallback
}
```

Default: `AudioBackendKind::Ffmpeg` (backward compatible; Symphonia is functional but less tested).

CLI `--backend` flag accepts `symphonia` or `ffmpeg`.

## Daemon Architecture

```
┌──────────────────────────────────────────────────────────────────┐
│                        gtmd Process                              │
│                                                                  │
│  main()                                                          │
│    1. parse CLI args with clap                                   │
│    2. resolve XDG paths for socket, db, config, cache, data      │
│    3. create DaemonConfig                                        │
│    4. init Daemon struct                                         │
│    5. daemon.run().await                                         │
│                                                                  │
│  Daemon::run() (main event loop)                                 │
│    loop {                                                        │
│        tokio::select! {                                          │
│            conn = listener.accept()  => {                        │
│                accept_client(conn).await;                         │
│                // spawns reader task + writer task per client    │
│            }                                                     │
│            Some((client_id, req)) = req_rx.recv() => {           │
│                dispatch(client_id, req).await;                   │
│                // response is currently discarded (no per-client │
│                // writer channel yet — FUTURE: add backpressure) │
│            }                                                     │
│            ev = backend.poll() => {                              │
│                handle_audio_event(ev).await;                     │
│            }                                                     │
│        }                                                         │
│    }                                                             │
└──────────────────────────────────────────────────────────────────┘
```

### Current client connection flow

```
client connects →
  accept_client() →
    spawn reader task (JSON lines → mpsc::UnboundedSender)
    spawn writer task (broadcast::Receiver → binary WireFrame frames)
```

Notes:
- Requests are sent as JSON lines; response is NOT yet sent back to the originating client
  (future: add per-client response channel with backpressure).
- Events are broadcast to ALL clients via `tokio::sync::broadcast`.
- No `flush_events()` yet — events are pushed directly to `event_tx` and the writer task
  reads them asynchronously.

## Daemon::run() method signatures

```rust
impl Daemon {
    pub fn new(config: DaemonConfig) -> Result<Self>;
    pub async fn run(&mut self) -> Result<()>;

    async fn accept_client(&mut self, stream: UnixStream);
    async fn dispatch(&mut self, client_id: ClientId, req: DaemonReq);
    async fn handle_request(&mut self, req: &DaemonReq) -> Result<(), CoreError>;
    async fn handle_audio_event(&mut self, result: AudioResult<Option<AudioEvent>>);
    async fn push_event(&self, event: DaemonEvent);

    // ─── Request handlers ───
    async fn cmd_play(&mut self, path: &str, start_pos: f64);
    async fn cmd_playpause(&mut self);
    async fn cmd_pause(&mut self);
    async fn cmd_stop(&mut self);
    async fn cmd_next(&mut self);
    async fn cmd_prev(&mut self);
    async fn cmd_seek(&mut self, pos: f64);
    async fn cmd_set_volume(&mut self, volume: u8);
    async fn cmd_toggle_shuffle(&mut self);
    async fn cmd_cycle_repeat(&mut self, mode: RepeatMode);
    async fn cmd_toggle_mute(&mut self);
    async fn cmd_crossfade(&mut self, enabled: bool, duration_secs: u8);
    async fn cmd_queue(&mut self, action: &QueueAction);        // stub
    async fn cmd_library(&mut self, action: &LibraryAction);    // stub
    async fn cmd_search(&mut self, query: &str);                // stub
    async fn cmd_get_favourites(&mut self);                     // stub
    async fn cmd_add_favourite(&mut self, track_id: i64);       // stub
    async fn cmd_remove_favourite(&mut self, track_id: i64);    // stub
    async fn cmd_yt_search(&mut self, query: &str, filter: Option<YTFilter>);  // stub
    async fn cmd_yt_search_poll(&mut self);                     // stub
    async fn cmd_yt_search_cancel(&mut self);                   // stub
    async fn cmd_yt_resolve_stream(&mut self, url: &str);       // stub
    async fn cmd_get_status(&mut self);
}
```

### Aspirational methods (not yet implemented)
- `async fn next_client_request(&mut self)` — currently uses mpsc channel directly
- `async fn flush_events(&mut self)` — currently events are pushed to broadcast channel directly

## Request Dispatch Table

```
Request                    → Handler Implementation          Status
─────────────────────────────────────────────────────────────────────
Play{path}                 │ cmd_play(path, 0.0)             ✅
PlayPause                  │ cmd_playpause()                 ✅
Pause                      │ cmd_pause()                     ✅
Stop                       │ cmd_stop()                      ✅
Next                       │ cmd_next()                      ✅
Prev                       │ cmd_prev()                      ✅
Seek{pos}                  │ backend.seek(pos)               ✅
SetVolume{v}               │ backend.set_volume(v)           ✅
ToggleShuffle              │ state.shuffle = !state.shuffle  ✅
CycleRepeat{m}             │ state.repeat = m                ✅
ToggleMute                 │ toggle + set_volume             ✅
Crossfade{enab, dur}       │ state.crossfade = ...           ✅
Queue{action}              │ dispatch_queue_action(action)   📋 stub
Library{action}            │ dispatch_library_action(action) 📋 stub
Search{query}              │ library.search_tracks(query)    📋 stub
GetFavourites              │ library.get_favourites()        📋 stub
AddFavourite{id}           │ library.add_favourite(id)       📋 stub
RemoveFavourite{id}        │ library.remove_favourite(id)    📋 stub
YtSearch{query, filter}    │ yt_manager.search()             📋 stub
YtSearchPoll               │ yt_manager.poll_results()       📋 stub
YtSearchCancel             │ yt_manager.cancel()             📋 stub
YtResolveStream{url}       │ yt_manager.resolve_stream()     📋 stub
Ping                       │ Ok(())                          ✅
Quit                       │ stop + exit                     ✅
```

## State Machine

```
                ┌──────────┐
                │  STOPPED │◀──────────────────┐
                └─────┬────┘                    │
                      │ play()                  │ stop() / track ends
                      ▼                         │
                ┌──────────┐      pause()    ┌──┴───────┐
         ┌─────▶│ PLAYING  │────────────────▶│  PAUSED  │
         │      └────┬─────┘◀────────────────└────┬─────┘
         │           │ seek()/next()/prev()        │
         │           │   (stay in PLAYING)         │ seek()/next()/prev()
         │           │                             │ (may resume PLAYING)
         │           │ track ends (auto_advance)   │
         │           └─────────────────────────────┘
         │
         │  (repeat_one = true → restart from 0)
         └──────────────────────────────────────────
```

## IPC Protocol

```
Transport: Unix Stream Socket
Socket path: $XDG_RUNTIME_DIR/gtmd.socket  →  /run/user/1000/gtmd.socket (fallback)

Request flow (JSON lines):
  client → "{"Play":{"path":"/path/to/file.opus"}}\n"
  daemon → "{"Ok":{"version":1}}\n"

Event flow (binary WireFrame):
  daemon → [u32 big-endian length][bincode::serialize(&WireFrame)]
  where WireFrame { version: u32, flags: u32, events: Vec<DaemonEvent> }
```

## DaemonConfig loading

```rust
impl DaemonConfig {
    pub fn load(args: &DaemonArgs) -> Self;
}

pub struct DaemonArgs {
    pub socket: Option<String>,
    pub library: Option<String>,
    pub config: Option<String>,
    pub verbose: bool,
    pub test_mode: bool,
    pub backend: Option<String>,
}
```

## File Structure (current)

```
gtmd/
├── Cargo.toml
└── src/
    ├── main.rs           # gtmd binary entrypoint          ✅
    ├── lib.rs            # module declarations + re-exports ✅
    ├── daemon.rs         # Daemon struct, run loop          ✅
    ├── config.rs         # DaemonConfig, DaemonArgs, XDG    ✅
    ├── ipc.rs            # IPC connection handling          📋 stub (5 lines)
    ├── dispatch.rs       # request dispatch                 📋 stub (5 lines)
    ├── library.rs        # Library (rusqlite)               📋 stub (5 lines)
    ├── queue.rs          # QueueManager                     📋 stub (5 lines)
    ├── youtube.rs        # YtManager (yt-dlp)               📋 stub (5 lines)
    ├── cover_art.rs      # CoverCache (Deezer)              📋 stub (5 lines)
    └── lyrics.rs         # LyricsManager (LRCLIB)           📋 stub (5 lines)
```
