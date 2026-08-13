# 03 — gtm-daemon: Background Audio Daemon

## Purpose

The daemon (`gtmd`) owns all audio state, manages the playback queue, communicates with the
library database, handles yt-dlp streaming, and broadcasts state changes to connected clients.
It is the single source of truth.

Depends on: `gtm-core`, `gtm-audio`, `gtm-mpris` (optional), `rusqlite`, `tokio`, `reqwest`, `tracing`, `clap`

## Daemon Struct

```rust
pub struct Daemon {
    // Shared state (read by IPC clients, MPRIS, etc.)
    pub state: Arc<RwLock<DaemonState>>,

    // Audio
    pub backend: Box<dyn AudioBackend>,

    // Library
    pub library: Option<Library>,

    // IPC
    pub listener: UnixListener,
    pub clients: Vec<ClientHandle>,

    // Event broadcast
    pub event_tx: broadcast::Sender<DaemonEvent>,
    pub event_rx: broadcast::Receiver<DaemonEvent>,

    // Subsystems
    pub yt_manager: YoutubeManager,
    pub cover_cache: CoverCache,
    pub lyrics_manager: LyricsManager,
    pub sleep_timer: Option<tokio::time::Instant>,
    pub sleep_timer_future: Option<tokio::task::JoinHandle<()>>,

    // Config
    pub config: DaemonConfig,

    // State version counter (monotonic, incremented on every mutation)
    pub version: u32,
}

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
    Symphonia,
    Ffmpeg,
}

pub struct ClientHandle {
    pub writer: tokio::io::BufWriter<tokio::net::UnixStream>,
    pub reader: tokio::io::BufReader<tokio::net::UnixStream>,
    pub peer_addr: String,
    pub connected: bool,
}
```

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
│                let (handle, events) = accept_client(conn);       │
│                clients.push(handle);                             │
│                // Subscribe new client to event broadcast        │
│                spawn client_event_writer(handle, event_rx);      │
│            }                                                     │
│            req = next_client_request() => {                      │
│                dispatch(req).await;                              │
│                flush_events().await;  // broadcast to all        │
│            }                                                     │
│            ev = backend.poll() => {                              │
│                handle_audio_event(ev).await;                     │
│                flush_events().await;                             │
│            }                                                     │
│            _ = sleep_timer_future => {                           │
│                handle_sleep_timer();                              │
│                flush_events().await;                             │
│            }                                                     │
│        }                                                         │
│    }                                                             │
└──────────────────────────────────────────────────────────────────┘
```

## Daemon::run() method signatures

```rust
impl Daemon {
    pub fn new(config: DaemonConfig) -> Result<Self>;
    pub async fn run(&mut self) -> Result<()>;

    // Accept a new client connection
    async fn accept_client(&mut self, stream: UnixStream) -> ClientHandle;

    // Read a JSON request line from any client
    async fn next_client_request(&mut self) -> Option<(ClientId, DaemonRequest)>;

    // Dispatch request to appropriate handler
    async fn dispatch(&mut self, client_id: ClientId, req: DaemonRequest) -> Result<()>;

    /// Broadcast pending events to all connected clients.
    /// Serializes accumulated events into binary frames.
    async fn flush_events(&mut self);

    // ─── Request handlers ───
    async fn cmd_play(&mut self, path: &str, start_pos: f64);
    async fn cmd_playpause(&mut self);
    async fn cmd_stop(&mut self);
    async fn cmd_next(&mut self);
    async fn cmd_prev(&mut self);
    async fn cmd_seek(&mut self, pos: f64);
    async fn cmd_set_volume(&mut self, volume: u8);
    async fn cmd_toggle_shuffle(&mut self);
    async fn cmd_cycle_repeat(&mut self, mode: RepeatMode);
    async fn cmd_toggle_mute(&mut self);
    async fn cmd_crossfade(&mut self, enabled: bool, duration_secs: u8);
    async fn cmd_queue(&mut self, action: QueueAction);
    async fn cmd_library(&mut self, action: LibraryAction);
    async fn cmd_search(&mut self, query: &str);
    async fn cmd_yt_search(&mut self, query: &str, filter: Option<YtFilter>);
    async fn cmd_get_status(&mut self) -> DaemonState;
    async fn cmd_quit(&mut self);
}
```

## Request Dispatch Table

```
Request                    → Handler Implementation
─────────────────────────────────────────────────────────
Play{path}                 │ cmd_play(path, 0.0)
PlayPause                  │ if playing → cmd_pause() else cmd_resume()
Stop                       │ cmd_stop()
Next                       │ advance_queue(1)
Prev                       │ advance_queue(-1)
Seek{pos}                  │ backend.seek(pos); state.time_pos = pos
SetVolume{v}               │ backend.set_volume(v); state.volume = v
ToggleShuffle              │ state.shuffle = !state.shuffle; reshuffle_queue()
CycleRepeat{m}             │ state.repeat = m
ToggleMute                 │ if mute: restore_volume else: backend.set_volume(0)
Crossfade{enab, dur}       │ state.crossfade = Some/None based on enabled
Queue{action}              │ dispatch_queue_action(action)
Library{action}            │ dispatch_library_action(action)
Search{query}              │ library.search_tracks(query) → respond
GetFavourites              │ library.get_favourites() → respond
AddFavourite{id}           │ library.add_favourite(id)
RemoveFavourite{id}        │ library.remove_favourite(id)
YtSearch{query, filter}    │ yt_manager.search(query, filter) → spawn yt-dlp
YtSearchPoll               │ yt_manager.poll_results() → respond
YtSearchCancel             │ yt_manager.cancel() → kill subprocess
YtResolveStream{url}       │ yt_manager.resolve_stream(url) → respond
Ping                       │ respond DaemonResponse::Pong
Quit                       │ shutdown() → stop backend, close socket, exit
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

## Event Broadcasting

```rust
impl Daemon {
    /// Flush accumulated events to all connected clients.
    /// Serializes events into one or more binary frames.
    async fn flush_events(&mut self) {
        // 1. Collect pending events from event_tx buffer
        let mut events: Vec<DaemonEvent> = Vec::new();
        while let Ok(ev) = self.event_rx.try_recv() {
            events.push(ev);
        }
        if events.is_empty() {
            return;
        }

        // 2. Increment state version
        self.version += 1;

        // 3. Serialize frame
        let frame = encode_frame(&events).unwrap();

        // 4. Write to each connected client
        self.clients.retain(|client| client.connected);
        for client in &mut self.clients {
            if let Err(e) = client.writer.write_all(&frame).await {
                warn!("write to {} failed: {}", client.peer_addr, e);
                client.connected = false;
            }
        }
    }

    /// Push an event. Called by dispatchers after mutating state.
    fn push_event(&mut self, event: DaemonEvent) {
        let _ = self.event_tx.send(event);
    }
}
```

## IPC Server

```
Unix Socket: /run/user/$UID/gtmd.socket
(or $XDG_RUNTIME_DIR/gtmd.socket → $TMPDIR/gtmd-XXXX.sock)

Request flow:
  client writes JSON line → parsed as DaemonRequest
  → dispatch() → handler → response written as JSON line

Event flow:
  Daemon::flush_events() → serialize WireFrame → write to all clients
```

## DaemonConfig loading

```rust
impl DaemonConfig {
    /// Load config from CLI args + XDG paths.
    /// Priority: CLI overrides > config file > defaults.
    pub fn load(args: &DaemonArgs) -> Self;
}

pub struct DaemonArgs {
    pub socket: Option<String>,
    pub library: Option<String>,
    pub config: Option<String>,
    pub verbose: bool,
    pub test_mode: bool,
}
```

## File Structure

```
gtmd/
├── Cargo.toml
└── src/
    ├── main.rs           # gtmd binary: CLI parsing, Daemon init, Daemon::run()
    ├── daemon.rs         # Daemon struct, run loop, state machine
    ├── ipc.rs            # IpcServer, ClientHandle, read/write helpers
    ├── dispatch.rs       # request → handler dispatch
    ├── library.rs        # Library (rusqlite wrapper, queries)
    ├── queue.rs          # QueueManager (cursor, shuffle, repeat)
    ├── youtube.rs        # YoutubeManager (yt-dlp subprocess)
    ├── cover_art.rs      # CoverCache (Deezer API, LRU + disk)
    ├── lyrics.rs         # LyricsManager (sidecar, LRCLIB, cache)
    └── config.rs         # DaemonConfig, DaemonArgs, XDG path resolution
```
