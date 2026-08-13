# gtm-rs: Rust Rewrite Specification Index

| # | File | Status | Description |
|---|------|--------|-------------|
| 00 | [00-overview.md](00-overview.md) | ✅ Final | Project overview, dependency graph, crate architecture |
| 03 | [03-gtm-daemon.md](03-gtm-daemon.md) | 🔶 Partial | Daemon struct, main loop, state machine, IPC server |
| 04 | [04-gtm-daemon-library.md](04-gtm-daemon-library.md) | 📋 Plan | SQLite schema, Library struct, scanning, search |
| 05 | [05-gtm-daemon-features.md](05-gtm-daemon-features.md) | 📋 Plan | yt-dlp manager, cover art cache, lyrics manager, queue logic |
| 06 | [06-gtm-tui-architecture.md](06-gtm-tui-architecture.md) | 📋 Plan | App state, event loop, DaemonClient, layout, render pipeline |
| 07 | [07-gtm-tui-tabs.md](07-gtm-tui-tabs.md) | 📋 Plan | LibraryTab, QueueTab, NowPlayingTab, YouTubeTab, SettingsTab, HelpTab |
| 08 | [08-gtm-tui-overlays.md](08-gtm-tui-overlays.md) | 📋 Plan | Overlay enum, command palette, fuzzy finder, confirm, track detail |
| 09 | [09-gtm-tui-features.md](09-gtm-tui-features.md) | 📋 Plan | Theme system, keybindings, Kitty graphics, footer modules, icons |
| 10 | [10-gtm-cli.md](10-gtm-cli.md) | 📋 Plan | CLI subcommands, shell completions, IPC dispatch |
| 11 | [11-gtm-mpris.md](11-gtm-mpris.md) | 📋 Plan | MPRIS D-Bus server, properties, method dispatch |
| 12 | [12-file-structure.md](12-file-structure.md) | 🔶 Partial | Full workspace tree (all 7 crates, every file) |
| 13 | [13-development-phases.md](13-development-phases.md) | ✅ Final | Phase timeline (0-6), deliverables, risks & mitigations |
| 14 | [14-migration-decisions.md](14-migration-decisions.md) | ✅ Final | Key architecture decisions and rationale |

**Specs 01 (gtm-core) and 02 (gtm-audio) are complete — deleted after implementation.**

## Quick Reference

```
gtm-rs/
├── Cargo.toml                   # workspace root
├── gtm-core/                    # shared types, IPC, wire protocol   ✅
├── gtm-audio/                   # audio decode + output abstraction  ✅
├── gtmd/                        # gtmd binary + daemon-lib           🔶 IPC/dispatch/queue/library/yt/cover/lyrics = stubs
├── gtm-tui/                     # gtm binary (Ratatui TUI)           📋 stub binary
├── gtm-cli/                     # gtm-cli binary (CLI controller)    📋 stub binary
├── gtm-mpris/                   # MPRIS D-Bus server (optional)      📋 stub lib
└── specs/                       # ← you are here
```

### Legend
- ✅ **Final** — spec is complete; no further changes expected
- 🔶 **Partial** — spec describes aspirational target; current impl is a subset
- 📋 **Plan** — spec describes planned design; not implemented yet
