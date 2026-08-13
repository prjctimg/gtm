# 14 — Migration Decisions

## Architecture Decisions

| # | Decision | Rationale | Alternative Considered |
|---|----------|-----------|----------------------|
| 1 | **symphonia primary, ffmpeg CLI fallback** | Pure Rust for most formats; opus via system libopus | `ffmpeg-next` (C bindings) |
| 2 | **Dedicated pulse socket for events** | Clean separation from JSON command/response; no heuristic first-byte parsing | Mixed socket (current fragile approach); separate FIFO |
| 3 | **JSON for request/response** | Human-debuggable, easy to test with `socat`, low volume | Binary for everything (hard to debug) |
| 4 | **State mirror pattern** | TUI caches daemon state, updated via events. No IPC on each render. | TUI queries daemon on each render (latency) |
| 5 | **tokio async runtime** | Required for concurrent IPC, subprocess, D-Bus | `async-std` (less ecosystem) |
| 6 | **Single-threaded TUI render** | Ratatui requires `&mut Frame`, no benefit from parallel rendering | Multi-threaded (complexity, no real gain) |
| 7 | **Background IPC worker task** | Non-blocking render loop; commands queue without freezing UI | Inline blocking IPC (current buggy approach) |
| 8 | **rusqlite with bundled sqlite3** | No system dependency, consistent version | System sqlite3 (version mismatch risk) |
| 9 | **Workspace with 6 crates** | Clear dependency boundaries, fast incremental builds | Single crate (monolithic) |
| 10 | **`clap` derive for CLI** | Industry standard, auto-generated help | `structopt` (deprecated) |
| 11 | **serde snake_case IPC** | Conventional, matches legacy docs convention | CamelCase (current approach, inconsistent) |
| 12 | **3 tabs + 9 overlays** | Overlays are NOT tabs; floating windows accessible from any tab | 6 tabs with no overlay concept (current wrong approach) |
| 13 | **Round borders, braille spinners, nerd icons** | Modern, polished appearance matching PROMPT.md | Plain borders (current), ASCII spinners |
| 14 | **Line progress bar with oscillating head** | Material design inspired, visually appealing | Standard Gauge widget (current) |
| 15 | **Semi-transparent overlays (90% default)** | Clean visual hierarchy, content visible beneath | Opaque overlays (blocks content) |

## Key Differences from Current Implementation

| Aspect | Current (buggy) | Target (Phase 5) |
|--------|----------------|------------------|
| **Cold start** | "Connection refused" on first launch | Connect with retry loop |
| **Screen** | Terminal prompt visible in TUI | Clear screen before draw |
| **Audio detection** | Only `~/.local/share/gtm/audio/`, duration always 0 | Configurable paths, real metadata extraction |
| **IPC blocking** | Render loop freezes during IPC | Background worker task, non-blocking |
| **IPC format** | Mixed JSON + bincode on same socket | Separate pulse socket for events |
| **IPC naming** | CamelCase variants | snake_case everywhere |
| **Tabs** | 6 tabs (NowPlaying, Library, Queue, YouTube, Settings, Help) | 3 tabs (NowPlaying, Library, Settings) |
| **Overlays** | None (replaced by tabs) | 9 floating overlays with Alt+key access |
| **Progress bar** | Ratatui Gauge widget | Custom line widget with oscillating head |
| **Borders** | Plain | Rounded |
| **Spinners** | None / text | Braille characters |
| **Icons** | Unicode only | Nerd icons with emoji fallback |
| **Footer** | Static text | Customizable modules with presets |
| **Crossfade** | 5s, no easing | 7s default, 5 easing options, reverb |
| **Pause behavior** | Abrupt stop | Volume dip then pause |
| **Volume safety** | No warning | Warning at >85%, daemon challenge |

## Nim → Rust Mapping

| Nim Concept | Rust Equivalent |
|-------------|----------------|
| `nimwave` TUI | `ratatui` widgets |
| `illwave` rendering | `crossterm` + `ratatui::Buffer` |
| `json` module | `serde_json` |
| `async` (single thread) | `tokio` (multithreaded) |
| `Arc` for sharing | Same |
| `RwLock` | Same |
| `Table[string, type]` | `HashMap<String, type>` |
| `seq[byte]` | `Vec<u8>` |
| `Option[type]` | `Option<type>` |
| `tuple[a: A, b: B]` | `struct { a: A, b: B }` |
| `case obj of variants` | `enum` with serde attributes |
| `import` | `use` |
| `var` (mutable) | `let mut` |
| `proc` | `fn` |
