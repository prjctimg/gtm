# gtm-rs: Rust Rewrite Specification Index

| # | File | Description |
|---|------|-------------|
| 00 | [00-overview.md](00-overview.md) | Project overview, dependency graph, crate architecture |
| 01 | [01-gtm-core.md](01-gtm-core.md) | Shared types, IPC protocol enums, wire format, DaemonState |
| 02 | [02-gtm-audio.md](02-gtm-audio.md) | AudioBackend trait, SymphoniaBackend, FfmpegBackend |
| 03 | [03-gtm-daemon.md](03-gtm-daemon.md) | Daemon struct, main loop, state machine, IPC server |
| 04 | [04-gtm-daemon-library.md](04-gtm-daemon-library.md) | SQLite schema, Library struct, scanning, search |
| 05 | [05-gtm-daemon-features.md](05-gtm-daemon-features.md) | yt-dlp manager, cover art cache, lyrics manager, queue logic |
| 06 | [06-gtm-tui-architecture.md](06-gtm-tui-architecture.md) | App state, event loop, DaemonClient, layout, render pipeline |
| 07 | [07-gtm-tui-tabs.md](07-gtm-tui-tabs.md) | LibraryTab, QueueTab, NowPlayingTab, YouTubeTab, SettingsTab, HelpTab |
| 08 | [08-gtm-tui-overlays.md](08-gtm-tui-overlays.md) | Overlay enum, command palette, fuzzy finder, confirm, track detail |
| 09 | [09-gtm-tui-features.md](09-gtm-tui-features.md) | Theme system, keybindings, Kitty graphics, footer modules, icons |
| 10 | [10-gtm-cli.md](10-gtm-cli.md) | CLI subcommands, shell completions, IPC dispatch |
| 11 | [11-gtm-mpris.md](11-gtm-mpris.md) | MPRIS D-Bus server, properties, method dispatch |
| 12 | [12-file-structure.md](12-file-structure.md) | Full workspace tree (all 7 crates, every file) |
| 13 | [13-development-phases.md](13-development-phases.md) | Phase timeline (0-6), deliverables, risks & mitigations |
| 14 | [14-migration-decisions.md](14-migration-decisions.md) | Key architecture decisions and rationale |

## Quick Reference

```
gtm-rs/
├── Cargo.workspace.toml       # workspace root
├── gtm-core/                  # shared types, IPC, wire protocol
├── gtm-audio/                 # audio decode + output abstraction
├── gtm-daemon/                # gtmd binary + daemon-lib
├── gtm-tui/                   # gtm binary (Ratatui TUI)
├── gtm-cli/                   # gtm-cli binary (CLI controller)
├── gtm-mpris/                 # MPRIS D-Bus server (optional)
└── specs/                     # ← you are here
```
