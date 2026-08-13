# gtm-rs: Rust Rewrite Specification Index

| # | File | Status | Description |
|---|------|--------|-------------|
| 00 | [00-overview.md](00-overview.md) | ✅ Final | Project overview, dependency graph, crate architecture |
| 01 | [01-ipc-redesign.md](01-ipc-redesign.md) | 📋 Plan | snake_case IPC, pulse thread, non-blocking client |
| 02 | [02-crossfade-audio.md](02-crossfade-audio.md) | 📋 Plan | Easing, reverb, volume dip, unsafe volume challenge |
| 03 | [03-gtm-daemon.md](03-gtm-daemon.md) | 🔶 Partial | Daemon struct, main loop, state machine, IPC server |
| 04 | [04-gtm-daemon-library.md](04-gtm-daemon-library.md) | 🔶 Partial | SQLite schema, Library struct, scanning, search |
| 05 | [05-gtm-daemon-features.md](05-gtm-daemon-features.md) | 📋 Plan | yt-dlp manager, cover art cache, lyrics manager, queue logic |
| 06 | [06-gtm-tui-architecture.md](06-gtm-tui-architecture.md) | 📋 Plan | Event loop, non-blocking DaemonClient, state mirror, layout |
| 07 | [07-gtm-tui-tabs.md](07-gtm-tui-tabs.md) | 📋 Plan | 3 tabs: NowPlaying, Library, Settings |
| 08 | [08-gtm-tui-overlays.md](08-gtm-tui-overlays.md) | 📋 Plan | 9 overlays with generic container, fuzzy finder, keymap |
| 09 | [09-gtm-tui-features.md](09-gtm-tui-features.md) | 📋 Plan | Aesthetics, notifications, footer modules, icons, progress bar |
| 10 | [10-gtm-cli.md](10-gtm-cli.md) | 📋 Plan | CLI subcommands, shell completions, IPC dispatch |
| 11 | [11-gtm-mpris.md](11-gtm-mpris.md) | 📋 Plan | MPRIS D-Bus server, properties, method dispatch |
| 12 | [12-file-structure.md](12-file-structure.md) | 🔶 Partial | Full workspace tree (all 7 crates, every file) |
| 13 | [13-development-phases.md](13-development-phases.md) | ✅ Final | 5-phase plan: bugs → IPC → UI → audio → compliance |
| 14 | [14-migration-decisions.md](14-migration-decisions.md) | ✅ Final | Key architecture decisions and rationale |

## Quick Reference

```
gtm-rs/
├── Cargo.toml                   # workspace root
├── gtm-core/                    # shared types, IPC, wire protocol   ✅
├── gtm-audio/                   # audio decode + output abstraction  ✅
├── gtmd/                        # gtmd binary + daemon-lib           🔶 IPC/dispatch/queue/library/yt/cover/lyrics = stubs
├── gtm/                         # gtm binary (Ratatui TUI + CLI)     🔶 partial
├── gtm-mpris/                   # MPRIS D-Bus server (optional)      📋 stub lib
└── specs/                       # ← you are here
```

### Legend
- ✅ **Final** — spec is complete; no further changes expected
- 🔶 **Partial** — spec describes aspirational target; current impl is a subset
- 📋 **Plan** — spec describes planned design; not implemented yet
