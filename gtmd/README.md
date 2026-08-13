# gtmd — gtm daemon

Background audio daemon that owns all playback state, manages the music library,
and communicates with clients over a Unix domain socket.

## Architecture

```
┌──────────────────────────────────────────────────┐
│                    gtmd                            │
│                                                    │
│  main():                                           │
│    1. parse CLI args (clap)                        │
│    2. load config                                  │
│    3. init Daemon struct                           │
│    4. daemon.run().await                           │
│                                                    │
│  Daemon::run() (main loop):                        │
│    loop {                                          │
│        tokio::select! {                            │
│            conn = listener.accept()                 │
│            req  = read_request() → dispatch()      │
│            ev   = backend.poll()   → handle_event()│
│            _    = sleep_timer.tick()                │
│        }                                           │
│        flush_events()  // broadcast to clients     │
│    }                                               │
└──────────────────────────────────────────────────┘
```

## Modules

| Module | Description |
|--------|-------------|
| `daemon.rs` | Daemon struct, main loop, state machine |
| `ipc.rs` | Unix socket listener, ClientHandle, JSON line I/O |
| `dispatch.rs` | Request → handler dispatch table |
| `queue.rs` | QueueManager (shuffle, repeat, cursor) |
| `library.rs` | SQLite library wrapper (rusqlite, bundled) |
| `youtube.rs` | yt-dlp subprocess manager |
| `cover_art.rs` | Deezer API cover art cache (LRU + disk) |
| `lyrics.rs` | LRC sidecar + LRCLIB API resolver |
| `config.rs` | XDG config loading |

## Dependencies

`gtm-core`, `gtm-audio`, `gtm-mpris` (optional), `rusqlite` (bundled), `tokio`, `reqwest`, `tracing`, `clap`
