# 12 — Full Workspace File Structure

> **Status**: 🔶 Partial — reflects current repo; many files listed are stubs or aspirational.

```
gtm-rs/
│
├── Cargo.toml                      # workspace root (pure workspace, no [package])
├── Cargo.lock
├── .gitignore
│
├── specs/                          # 13 spec files, ~5500 LOC
│   ├── INDEX.md
│   ├── 00-overview.md              ✅ project overview
│   ├── 03-gtm-daemon.md            🔶 daemon (core impl done, subsystems stubbed)
│   ├── 04-gtm-daemon-library.md    📋 library plan
│   ├── 05-gtm-daemon-features.md   📋 features plan
│   ├── 06-gtm-tui-architecture.md  📋 TUI plan
│   ├── 07-gtm-tui-tabs.md          📋 TUI tabs plan
│   ├── 08-gtm-tui-overlays.md      📋 TUI overlays plan
│   ├── 09-gtm-tui-features.md      📋 TUI features plan
│   ├── 10-gtm-cli.md               📋 CLI plan
│   ├── 11-gtm-mpris.md             📋 MPRIS plan
│   ├── 12-file-structure.md        🔶 this file
│   ├── 13-development-phases.md    ✅ development phases
│   └── 14-migration-decisions.md   ✅ architecture decisions
│
├── gtm-core/                       # Shared types & IPC protocol       ✅ complete
│   ├── Cargo.toml                  # serde, serde_json, bincode, thiserror
│   └── src/
│       ├── lib.rs                  # re-exports all public types
│       ├── ipc.rs                  # DaemonReq, DaemonRes, DaemonEvent
│       ├── wire.rs                 # WireFrame, encode, decode
│       ├── track.rs                # TrackInfo, Playlist, LrcLine, LrcData
│       ├── state.rs                # DaemonState, PlaybackStatus, RepeatMode
│       ├── state_machine.rs        # state transition logic
│       ├── validate.rs             # validation helpers
│       └── tripwire.rs             # tripwire utility
│
├── gtm-audio/                      # Audio backend abstraction        ✅ complete
│   ├── Cargo.toml                  # symphonia 0.6, symphonia-adapter-libopus, rodio
│   └── src/
│       ├── lib.rs                  # re-exports
│       ├── backend.rs              # AudioBackend trait, AudioEvent, AudioError
│       ├── symphonia.rs            # SymphoniaBackend (pure Rust, incl. opus)
│       ├── rodio.rs                # RodioBackend (rodio-only, no symphonia)
│       └── ffmpeg.rs               # FfmpegBackend (ffmpeg CLI, feature-gated)
│
├── gtmd/                           # Daemon binary + library          🔶 partial
│   ├── Cargo.toml                  # gtm-core, gtm-audio, rusqlite, tokio, reqwest
│   └── src/
│       ├── main.rs                 # gtmd binary entrypoint           ✅
│       ├── lib.rs                  # module declarations              ✅
│       ├── daemon.rs               # Daemon struct, loop, handlers    ✅
│       ├── config.rs               # DaemonConfig, DaemonArgs, XDG    ✅
│       ├── ipc.rs                  # IPC handling                     📋 stub
│       ├── dispatch.rs             # request dispatch                 📋 stub
│       ├── library.rs              # Library (rusqlite)               📋 stub
│       ├── queue.rs                # QueueManager                     📋 stub
│       ├── youtube.rs              # YtManager (yt-dlp)               📋 stub
│       ├── cover_art.rs            # CoverCache (Deezer)              📋 stub
│       └── lyrics.rs               # LyricsManager (LRCLIB)           📋 stub
│
├── gtm-tui/                        # TUI binary                      📋 stub
│   ├── Cargo.toml                  # ratatui, crossterm, tokio, gtm-core
│   └── src/
│       └── main.rs                 # stub main()
│
├── gtm-cli/                        # CLI controller binary            📋 stub
│   ├── Cargo.toml                  # clap, gtm-core, tokio
│   └── src/
│       └── main.rs                 # stub main()
│
└── gtm-mpris/                      # MPRIS D-Bus server library       📋 stub
    ├── Cargo.toml                  # zbus, zvariant, gtm-core
    └── src/
        └── lib.rs                  # stub
```

## Legend

| Icon | Meaning |
|------|---------|
| ✅ | Complete — passes `cargo check`, tested |
| 🔶 | Partial — core logic works, some features stubbed |
| 📋 | Plan — spec exists, impl is stub or absent |

## Key Configuration Paths

```
Socket:     $XDG_RUNTIME_DIR/gtmd.socket       (/run/user/$UID/gtmd.socket)  → /run/user/1000/gtmd.socket
Database:   $XDG_DATA_HOME/gtm/library.db      (~/.local/share/gtm/library.db)
Config:     $XDG_CONFIG_HOME/gtm/              (~/.config/gtm/)
Cache:      $XDG_CACHE_HOME/gtm/               (~/.cache/gtm/)
Covers:     $XDG_CACHE_HOME/gtm/covers/
Log:        $XDG_DATA_HOME/gtm/gtmd.log        (~/.local/share/gtm/gtmd.log)
Audio:      $XDG_DATA_HOME/gtm/audio/          (~/.local/share/gtm/audio/) — test fixtures
```

## Summary

| Area | Files | Status |
|------|-------|--------|
| Specs | 13 | 3 final, 3 partial, 7 plans |
| gtm-core | 8 | ✅ 8 |
| gtm-audio | 5 | ✅ 5 |
| gtmd | 10 | 4 implemented, 6 stubs |
| gtm-tui | 1 | 📋 stub |
| gtm-cli | 1 | 📋 stub |
| gtm-mpris | 1 | 📋 stub |
