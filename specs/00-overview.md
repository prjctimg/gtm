# 00 — Project Overview

Rewrite **gtm** (terminal music player) from Nim → Rust.

| Aspect | Nim Original | Rust Rewrite |
|--------|-------------|--------------|
| TUI framework | `nimwave` / `illwave` | `ratatui` + `crossterm` |
| Audio decode | FFmpeg subprocess | `symphonia 0.6` + `symphonia-adapter-libopus` (pure Rust); `ffmpeg` CLI fallback |
| Audio output | ALSA (Linux only) | `rodio` + `cpal` (cross-platform) |
| SQLite | vendored `sqlite3.c` | `rusqlite` (bundled) |
| HTTP | `httpclient` stdin pipe | `reqwest` (async) |
| Serialization | `json` module | `serde` + `serde_json` + `bincode` |
| IPC | Unix sockets (manual) | `tokio::net::UnixStream` |
| Async | single-threaded poll | `tokio` (multi-task) |
| D-Bus | `dbus` bindings | `zbus` + `zvariant` |

## Crate Dependency Graph

```
┌─────────────────────────────────────────────────┐
│                 gtm (TUI binary)                 │
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
│              gtmd (daemon binary)                │
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

## Crate Legend

| Crate | Type | Deps In | Deps Out | Key Crates |
|-------|------|---------|----------|------------|
| `gtm-core` | lib | all | — | `serde`, `bincode`, `thiserror` |
| `gtm-audio` | lib | daemon, TUI | `gtm-core` | `symphonia 0.6`, `symphonia-adapter-libopus`, `rodio` |
| `gtmd` | lib+bin | — | `gtm-core`, `gtm-audio`, `gtm-mpris` | `rusqlite`, `tokio`, `reqwest` |
| `gtm-tui` | bin | — | `gtm-core`, `gtm-audio` | `ratatui`, `crossterm`, `tokio` |
| `gtm-cli` | bin | — | `gtm-core` | `clap`, `tokio` |
| `gtm-mpris` | lib | gtmd | `gtm-core` | `zbus`, `zvariant` |

## Directory Layout

```
gtm-rs/
├── Cargo.toml               # workspace root (pure workspace, no [package])
├── Cargo.lock
├── README.md
├── docs/                    # documentation
├── assets/                  # static assets (default cover)
├── specs/                   # spec files (this directory)
├── gtm-core/                # shared types & IPC protocol
├── gtm-audio/               # audio backend abstraction
├── gtmd/                    # daemon binary + library
├── gtm-tui/                 # TUI binary
├── gtm-cli/                 # CLI controller binary
├── gtm-mpris/               # MPRIS D-Bus server library
└── tests/                   # integration tests + fixtures
```

## Key Spec Files

| File | Content |
|------|---------|
| `01-gtm-core.md` | All shared types: DaemonRequest/Response, DaemonEvent, WireFrame, TrackInfo, DaemonState, CoreError |
| `02-gtm-audio.md` | AudioBackend trait, SymphoniaBackend, FfmpegBackend, AudioError |
| `03-gtm-daemon.md` | Daemon struct, event loop, dispatch table, ClientHandle, DaemonConfig |
| `04-gtm-daemon-library.md` | SQLite schema (10 tables), Library struct, all full SQL queries, MetadataExtractor |
| `05-gtm-daemon-features.md` | YoutubeManager, CoverCache (Deezer), LyricsManager (LRCLIB), QueueManager, crossfade |
| `06-gtm-tui-architecture.md` | Event loop, AppState, DaemonClient, state mirror, position extrapolation |
| `07-gtm-tui-tabs.md` | 6 TabWidget implementations, all view states, keybindings |
| `08-gtm-tui-overlays.md` | 6 overlay states, fuzzy scoring algorithm, centered_rect |
| `09-gtm-tui-features.md` | Theme (all 26 colors with presets), keybinding system, Kitty graphics, footer, icons |
| `10-gtm-cli.md` | Full clap subcommand tree, command→request mapping, output formatting |
| `11-gtm-mpris.md` | MprisServer, zbus interfaces, metadata map, event→signal bridge |
| `13-development-phases.md` | 7-phase plan with milestones, checklists, risk table |
| `14-migration-decisions.md` | 12 architecture decisions, Nim→Rust mapping, theme algorithm |
