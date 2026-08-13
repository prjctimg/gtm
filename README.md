# gtm-rs

A terminal-based music player with daemon architecture, SQLite library, Symphonia audio decoding,
LRC lyrics, YouTube integration, and MPRIS D-Bus remote control.

Rewrite of [gtm](https://github.com/prjctimg/gtm) from Nim to Rust.

## Crate Architecture

```
┌─────────────────────────────────────────────────┐
│                 gtm-tui (TUI binary)              │
│  ┌──────────┐  ┌──────────┐  ┌───────────────┐  │
│  │ ratatui  │  │ crossterm│  │ gtm-core      │  │
│  │ (widgets)│  │ (input)  │  │ (types, IPC)  │  │
│  └──────────┘  └──────────┘  └───────┬───────┘  │
│                                      │          │
│                             ┌────────▼───────┐  │
│                             │ gtm-audio      │  │
│                             │ (symphonia)    │  │
│                             └────────────────┘  │
└─────────────────────────────────────────────────┘
                        │ IPC (Unix socket, JSON + binary)
                        ▼
┌─────────────────────────────────────────────────┐
│              gtmd (daemon binary)                 │
│  ┌──────────┐  ┌──────────┐  ┌───────────────┐  │
│  │ gtm-core │  │ gtm-audio│  │ gtm-mpris     │  │
│  │ (types)  │  │ (backend)│  │ (zbus D-Bus)  │  │
│  └──────────┘  └──────────┘  └───────────────┘  │
│  ┌──────────┐  ┌──────────┐                     │
│  │ rusqlite │  │ reqwest  │                     │
│  │ (lib)    │  │ (HTTP)   │                     │
│  └──────────┘  └──────────┘                     │
└─────────────────────────────────────────────────┘
```

| Crate | Type | Description |
|-------|------|-------------|
| `gtm-core` | lib | Shared types, IPC protocol enums, wire format |
| `gtm-audio` | lib | AudioBackend trait, Symphonia decoder, cpal output |
| `gtmd` | lib+bin | Background daemon, SQLite library, queue, yt-dlp |
| `gtm-tui` | bin | Ratatui TUI client with 6 tabs and overlays |
| `gtm-cli` | bin | Headless CLI controller for scripting |
| `gtm-mpris` | lib | MPRIS D-Bus server (optional, zbus) |

## Building

```
cargo build --workspace
cargo build -p gtm-tui          # TUI binary only
cargo build -p gtmd             # daemon binary only
```

## IPC Protocol

Communication between the TUI/CLI and daemon uses a Unix domain socket with two wire formats:
- **JSON lines** for request/response (human-debuggable)
- **Binary frames** (bincode) for daemon event streaming

## License

GPL-3.0-only
