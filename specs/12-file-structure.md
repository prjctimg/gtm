# 12 — Full Workspace File Structure

```
gtm-rs/
│
├── Cargo.toml                      # workspace root (pure workspace, no [package])
├── Cargo.lock
├── README.md
├── .gitignore
├── .github/
│   └── workflows/
│       ├── ci.yml                  # cargo test, cargo clippy, cargo fmt
│       └── release.yml             # build + deploy binaries
│
├── docs/
│   ├── ipc.md                      # IPC protocol specification
│   ├── architecture.md             # high-level architecture diagram
│   └── theme.md                    # theme customization guide
│
├── assets/
│   └── default_cover.png           # fallback album art image
│
├── specs/                          # ← this directory (15 files, ~6000 LOC total)
│   ├── INDEX.md
│   ├── 00-overview.md
│   ├── 01-gtm-core.md              # Shared types, IPC enums, wire protocol
│   ├── 02-gtm-audio.md             # Audio backend trait, symphonia/ffmpeg
│   ├── 03-gtm-daemon.md            # Daemon struct, event loop, dispatch
│   ├── 04-gtm-daemon-library.md    # SQLite library, schema, queries
│   ├── 05-gtm-daemon-features.md   # yt-dlp, cover art, lyrics, queue, crossfade
│   ├── 06-gtm-tui-architecture.md  # TUI event loop, layout, AppState, DaemonClient
│   ├── 07-gtm-tui-tabs.md          # 6 TabWidget implementations, view states
│   ├── 08-gtm-tui-overlays.md      # 6 overlay states, fuzzy matching
│   ├── 09-gtm-tui-features.md      # Theme, keybindings, Kitty protocol, footer
│   ├── 10-gtm-cli.md               # CLI subcommand tree, IPC dispatch
│   ├── 11-gtm-mpris.md             # MPRIS D-Bus server, zbus interfaces
│   ├── 12-file-structure.md        # Full file tree (this file)
│   ├── 13-development-phases.md    # Phase breakdown, checklist, risks
│   └── 14-migration-decisions.md   # Architecture decisions, Nim→Rust mapping
│
├── gtm-core/                       # Shared types & IPC protocol
│   ├── Cargo.toml                  # serde, serde_json, bincode, thiserror, chrono, uuid
│   └── src/
│       ├── lib.rs                  # re-exports all public types
│       ├── ipc.rs                  # DaemonRequest, DaemonResponse, DaemonEvent
│       ├── wire.rs                 # WireFrame, encode_frame, decode_frame
│       ├── track.rs                # TrackInfo, Playlist, LrcLine, LrcData
│       └── state.rs                # DaemonState, PlaybackStatus, RepeatMode, CoreError
│
├── gtm-audio/                      # Audio backend abstraction
│   ├── Cargo.toml                  # symphonia, cpal, rubato, crossbeam, async-trait
│   └── src/
│       ├── lib.rs                  # re-exports
│       ├── backend.rs              # AudioBackend trait, AudioEvent, AudioError
│       ├── symphonia.rs            # SymphoniaBackend
│       └── ffmpeg.rs               # FfmpegBackend (feature-gated)
│
├── gtmd/                           # Daemon binary + library
│   ├── Cargo.toml                  # gtm-core, gtm-audio, rusqlite, tokio, reqwest, tracing, clap
│   └── src/
│       ├── main.rs                 # gtmd binary entrypoint
│       ├── lib.rs                  # re-exports for integration tests
│       ├── daemon.rs               # Daemon struct, main loop, state machine
│       ├── ipc.rs                  # ClientHandle, read/write helpers
│       ├── dispatch.rs             # request → handler dispatch
│       ├── library.rs              # Library (rusqlite wrapper, schema, queries)
│       ├── queue.rs                # QueueManager (cursor, shuffle, repeat)
│       ├── youtube.rs              # YoutubeManager (yt-dlp subprocess)
│       ├── cover_art.rs            # CoverCache (Deezer API, LRU + disk)
│       ├── lyrics.rs               # LyricsManager (sidecar, LRCLIB, cache)
│       └── config.rs               # DaemonConfig, DaemonArgs, XDG path resolution
│
├── gtm-tui/                        # TUI binary
│   ├── Cargo.toml                  # ratatui, crossterm, tokio, gtm-core, gtm-audio, image, base64
│   └── src/
│       ├── main.rs                 # gtm binary: args, terminal init, event loop
│       ├── app.rs                  # App struct, render(), terminal.draw()
│       ├── state.rs                # AppState, per-tab view states
│       ├── daemon_client.rs        # DaemonClient (IPC transport + state mirror)
│       ├── keymap.rs               # Keybindings, parse_keycode, KeyboardAction
│       ├── theme.rs                # Theme struct, presets, hsl_to_rgb
│       ├── graphics.rs             # KittyGraphics (probe, transmit, delete, place)
│       ├── icons.rs                # IconSet, NERD_FONT, EMOJI constants
│       ├── footer.rs               # FooterBar, FooterModule enum
│       ├── tabs/
│       │   ├── mod.rs              # TabWidget trait, Tab enum, Action enum
│       │   ├── library.rs          # LibraryTab
│       │   ├── queue.rs            # QueueTab
│       │   ├── now_playing.rs      # NowPlayingTab
│       │   ├── youtube.rs          # YouTubeTab
│       │   ├── settings.rs         # SettingsTab
│       │   └── help.rs             # HelpTab
│       └── overlays/
│           ├── mod.rs              # Overlay enum + dispatch + centered_rect
│           ├── command_palette.rs   # CommandPaletteState
│           ├── fuzzy_finder.rs     # FuzzyFinderState, fuzzy_score algorithm
│           ├── queue_picker.rs     # QueuePickerState
│           ├── theme_picker.rs     # ThemePickerState
│           ├── confirm_dialog.rs   # ConfirmState
│           └── track_detail.rs     # TrackDetailState
│
├── gtm-cli/                        # CLI controller binary
│   ├── Cargo.toml                  # clap (derive), gtm-core, tokio
│   └── src/
│       ├── main.rs                 # CLI parser, dispatch, output formatting
│       └── completions.rs          # Shell completions (feature-gated)
│
├── gtm-mpris/                      # MPRIS D-Bus server library
│   ├── Cargo.toml                  # zbus (tokio), zvariant, gtm-core, thiserror
│   └── src/
│       └── lib.rs                  # MprisServer, zbus interface impls
│
└── tests/
    ├── integration/
    │   ├── daemon_ipc.rs           # spawn daemon, send requests, verify responses
    │   └── wire_protocol.rs        # binary frame encode/decode roundtrips
    └── fixtures/
        ├── sample.mp3
        ├── sample.flac
        ├── sample.ogg
        ├── sample.wav
        └── sample.lrc
```

## Summary

| Area | Files | Lines (est.) |
|------|-------|-------------|
| Specs | 15 | ~6,000 |
| gtm-core | 5 | ~800 |
| gtm-audio | 4 | ~600 |
| gtmd | 10 | ~3,000 |
| gtm-tui | 20+ | ~5,000+ |
| gtm-cli | 3 | ~300 |
| gtm-mpris | 2 | ~400 |
| Tests | 3+ | ~500 |
| **Total** | **~62** | **~16,600** |

## Key Configuration Paths

```
Socket:     $XDG_RUNTIME_DIR/gtmd.socket       (/run/user/$UID/gtmd.socket)
Database:   $XDG_DATA_HOME/gtm/library.db      (~/.local/share/gtm/library.db)
Config:     $XDG_CONFIG_HOME/gtm/config.toml   (~/.config/gtm/config.toml)
Cache:      $XDG_CACHE_HOME/gtm/               (~/.cache/gtm/)
Covers:     $XDG_CACHE_HOME/gtm/covers/
Log:        $XDG_STATE_HOME/gtm/gtmd.log       (~/.local/state/gtm/gtmd.log)
```
