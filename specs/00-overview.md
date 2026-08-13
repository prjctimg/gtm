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
| IPC | Unix sockets (manual) | `tokio::net::UnixStream` + pulse socket |
| Async | single-threaded poll | `tokio` (multi-task) |
| D-Bus | `dbus` bindings | `zbus` + `zvariant` |
| Tabs | 3 tabs (Playlist, Library, Now Playing) | 3 tabs (NowPlaying, Library, Settings) |
| Overlays | 14 overlays | 9 overlays (Queue, YTSearch, SearchLibrary, SpotifySearch, Equalizer, CommandPalette, About, SleepTimer, ThemePicker) |

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
                        │ IPC (Unix socket, JSON)
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
                        │ pulse socket (bincode events)
                        ▼
┌─────────────────────────────────────────────────┐
│                gtm (event receiver)              │
│  ┌───────────────────────────────────────────┐  │
│  │ Dedicated pulse reader task               │  │
│  │ → pushes DaemonEvent to shared queue      │  │
│  └───────────────────────────────────────────┘  │
└─────────────────────────────────────────────────┘
```

## Crate Legend

| Crate | Type | Deps In | Deps Out | Key Crates |
|-------|------|---------|----------|------------|
| `gtm-core` | lib | all | — | `serde`, `bincode`, `thiserror` |
| `gtm-audio` | lib | daemon, TUI | `gtm-core` | `symphonia 0.6`, `symphonia-adapter-libopus`, `rodio` |
| `gtmd` | lib+bin | — | `gtm-core`, `gtm-audio`, `gtm-mpris` | `rusqlite`, `tokio`, `reqwest` |
| `gtm` | bin | — | `gtm-core`, `gtm-audio` | `ratatui`, `crossterm`, `tokio` |
| `gtm-mpris` | lib | gtmd | `gtm-core` | `zbus`, `zvariant` |

## Directory Layout

```
gtm-rs/
├── Cargo.toml               # workspace root (pure workspace, no [package])
├── Cargo.lock
├── README.md
├── PROMPT.md                # feature requirements (this drives implementation)
├── docs/                    # documentation
├── docs-legacy/             # legacy man pages for Phase 5 compliance
├── specs/                   # spec files (this directory)
├── gtm-core/                # shared types & IPC protocol
├── gtm-audio/               # audio backend abstraction
├── gtmd/                    # daemon binary + library
├── gtm/                     # TUI + CLI binary (single bin)
├── gtm-mpris/               # MPRIS D-Bus server library
└── scripts/                 # build/release scripts
```

## Key Spec Files

| File | Content |
|------|---------|
| `01-ipc-redesign.md` | snake_case IPC, pulse thread, non-blocking DaemonClient |
| `02-crossfade-audio.md` | Easing, reverb, volume dip, unsafe volume challenge |
| `03-gtm-daemon.md` | Daemon struct, event loop, dispatch table, DaemonConfig |
| `04-gtm-daemon-library.md` | SQLite schema, Library struct, scanning, metadata extraction |
| `06-gtm-tui-architecture.md` | Event loop, non-blocking DaemonClient, state mirror, layout |
| `07-gtm-tui-tabs.md` | 3 TabWidget implementations (NowPlaying, Library, Settings) |
| `08-gtm-tui-overlays.md` | 9 overlay specs with generic container |
| `09-gtm-tui-features.md` | Aesthetics, notifications, footer, icons, progress bar |
| `13-development-phases.md` | 5-phase plan with milestones and checklists |
