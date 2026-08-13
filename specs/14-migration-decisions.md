# 14 — Migration Decisions

## Architecture Decisions

| # | Decision | Rationale | Alternative Considered |
|---|----------|-----------|----------------------|
| 1 | **symphonia primary, ffmpeg-next fallback** | Pure Rust → no C build dependency, faster compiles, better portability | `ffmpeg-next` everywhere (original Nim approach) |
| 2 | **Keep binary IPC for events** | Proven in Nim: compact, fast, deterministic. Events are high-frequency (position at 10Hz) | Full JSON streaming (slow to parse); protobuf (heavy dep) |
| 3 | **JSON for request/response** | Human-debuggable, easy to test with `socat`, low volume (1 per user action) | Binary for everything (hard to debug); custom format (waste) |
| 4 | **State mirror pattern** | TUI caches daemon state, updated via events. Same as Nim, proven correct. | TUI queries daemon on each render (latency); shared memory (complex) |
| 5 | **tokio async runtime** | Required for concurrent IPC, yt-dlp subprocess, MPRIS D-Bus. | `async-std` (less ecosystem); `smol` (too minimal) |
| 6 | **Single-threaded TUI render** | Ratatui requires `&mut Frame`, no benefit from parallel rendering. | Multi-threaded render (complexity, no real gain) |
| 7 | **Separate `gtm-mpris` crate** | Optional feature, reduces binary size. D-Bus not needed on all platforms. | Feature flag inside daemon (conditional compilation complexity) |
| 8 | **rusqlite with bundled sqlite3** | No system dependency, consistent version, easy cross-compilation. | System sqlite3 (version mismatch risk) |
| 9 | **Workspace with 7 crates** | Clear dependency boundaries, separate compilation units, fast incremental builds. | Single crate (monolithic, slow to compile) |
| 10 | **`clap` derive for CLI** | Industry standard, auto-generated help, shell completion support. | `structopt` (deprecated); manual parse (error-prone) |
| 11 | **serde internally-tagged enums** | Clear JSON output with type+content fields, human-readable | Adjacently tagged (noisy); untagged (ambiguous) |
| 12 | **GTMD as lib+bin crate** | Integration tests can link against library; binary is thin wrapper | Pure binary (can't test without spawning process) |

## Nim → Rust Mapping

| Nim Concept | Rust Equivalent | Notes |
|-------------|----------------|-------|
| `nimwave` TUI | `ratatui` widgets | Both are immediate-mode TUI frameworks |
| `illwave` rendering | `crossterm` + `ratatui::Buffer` | Lower-level, more control |
| `json` module | `serde_json` | Derive macros instead of runtime reflection |
| `async` (single thread) | `tokio` (multithreaded) | Need `Send + Sync` on shared state |
| `Arc` for sharing | Same | Direct port |
| `RwLock` | Same | Direct port |
| `Table[string, type]` | `HashMap<String, type>` | Direct port |
| `seq[byte]` | `Vec<u8>` | Same |
| `Option[type]` | `Option<type>` | Same |
| `tuple[a: A, b: B]` | `struct { a: A, b: B }` | Named struct instead of tuple |
| `case obj of variants` | `enum` with `#[serde(tag = ...)]` | Serde tagged enums |
| `{.this.}` / UFCS | `impl Trait for Type` | Methods instead of UFCS |
| `import` | `use` | Same concept, different syntax |
| `discard` | `let _ =` | Slightly different idiom |
| `try: ... except:` | `match result { Ok(v) => ..., Err(e) => ... }` | Result type instead of exceptions |
| `var` (mutable) | `let mut` | Same concept |
| `proc` | `fn` | Same |
| `echo` | `println!` / `eprintln!` | Macro vs statement |

## Key Differences from Original Nim gtm

| Aspect | Nim Original | Rust Rewrite |
|--------|-------------|--------------|
| **Audio pipeline** | Dual-pipeline MixerBackend for crossfade | Single-pipeline Symphonia, daemon-level crossfade |
| **TUI rendering** | illwave + nimwave (2 layers) | Ratatui Widget trait (single layer) |
| **IPC event encoding** | Custom binary with type+version+payload | bincode-serialized WireFrame with u32 length prefix |
| **Queue shuffle** | Same Fisher-Yates | Same algorithm, ported directly |
| **Theme generation** | HSL-based catppuccin | Same HSL algorithm, Rust implementation |
| **Tabs** | 3 tabs (Playlist, Library, Now Playing) | 6 tabs (NowPlaying, Library, Queue, YouTube, Settings, Help) |
| **Overlays** | 14 overlays (modals, menus, dialogs) | 6 overlays (CommandPalette, FuzzyFinder, QueuePicker, ThemePicker, Confirm, TrackDetail) |
| **Keybinding** | Inline match statements | Keybinding struct with context-aware dispatch |
| **Library table** | `Table[string, string]` for settings | `rusqlite` with 10 SQL tables |
| **HTTP client** | internal `httpclient` module | `reqwest` async HTTP client |
| **Binary name** | `gtm` (combined) | `gtm` (TUI) + `gtmd` (daemon) |

## Theme Generation Algorithm (ported from Nim)

```rust
/// Generate a catppuccin-inspired theme from an HSL seed.
/// Ported from gtm's src/ui.nim:generateTheme()
pub fn generate_theme(seed: &str, mode: ThemeMode) -> Theme {
    // 1. Hash seed string to get base hue (0-360)
    let base_hue = hash_to_hue(seed);

    // 2. Define 26 color positions as (hue_offset, saturation, lightness)
    let palette = if mode == ThemeMode::Dark {
        vec![
            ("rosewater",   0,  42, 85),
            ("flamingo",    0,  59, 83),
            ("pink",      -10,  59, 83),
            ("mauve",     -20,  60, 78),
            ("red",         0,  75, 69),
            ("maroon",     -5,  65, 65),
            ("peach",      25,  75, 65),
            ("yellow",     40,  75, 65),
            ("green",      80,  60, 60),
            ("teal",      120,  50, 55),
            ("sky",       160,  40, 60),
            ("sapphire",  180,  40, 60),
            ("blue",      200,  50, 65),
            ("lavender",  220,  55, 70),
            // neutrals
            ("text",        0,   0, 92),
            ("subtext1",    0,   0, 82),
            ("subtext0",    0,   0, 72),
            ("overlay2",    0,   0, 62),
            ("overlay1",    0,   0, 52),
            ("overlay0",    0,   0, 42),
            ("surface2",    0,   0, 35),
            ("surface1",    0,   0, 28),
            ("surface0",    0,   0, 22),
            ("base",        0,   0, 17),
            ("mantle",      0,   0, 13),
            ("crust",       0,   0, 10),
        ]
    } else {
        // Light mode: invert lightness scale
        vec![
            ("rosewater",   0,  42, 92),
            ("flamingo",    0,  59, 90),
            ("pink",      -10,  59, 88),
            ("mauve",     -20,  60, 80),
            ("red",         0,  75, 72),
            ("maroon",     -5,  65, 70),
            ("peach",      25,  75, 68),
            ("yellow",     40,  75, 65),
            ("green",      80,  60, 58),
            ("teal",      120,  50, 55),
            ("sky",       160,  40, 55),
            ("sapphire",  180,  40, 55),
            ("blue",      200,  50, 58),
            ("lavender",  220,  55, 62),
            // neutrals (inverted: higher lightness = darker)
            ("text",        0,   0, 12),
            ("subtext1",    0,   0, 18),
            ("subtext0",    0,   0, 25),
            ("overlay2",    0,   0, 32),
            ("overlay1",    0,   0, 40),
            ("overlay0",    0,   0, 48),
            ("surface2",    0,   0, 58),
            ("surface1",    0,   0, 68),
            ("surface0",    0,   0, 78),
            ("base",        0,   0, 88),
            ("mantle",      0,   0, 92),
            ("crust",       0,   0, 96),
        ]
    };

    // 3. For each color, compute actual hue = base_hue + offset
    // 4. Clamp hue to 0-360
    // 5. Convert HSL(hue, sat/100, light/100) → RGB via hsl_to_rgb
    // 6. Return Theme struct with all fields populated
}
```

## Why Not...

| Alternative | Why Not |
|-------------|---------|
| **Rust native TUI (not Ratatui)** | Ratatui is the de facto standard, active community, well-documented |
| **`libmpv` for audio** | Adds C dependency, complex build, overkill for music player |
| **`gstreamer` for audio** | Heavy dependency, complex pipeline, Linux-only |
| **WebSocket for IPC** | Overkill for local Unix socket, adds framing overhead |
| **Cap'n Proto / FlatBuffers** | Heavy schema compilation, no advantage over bincode for this use case |
| **SQLite via `diesel`** | ORM overhead, compile-time queries add complexity. `rusqlite` is simpler |
| **Single binary (no daemon)** | Can't have MPRIS + CLI without a persistent process |
| **Crossbeam channels instead of tokio** | Inconsistent with tokio-based architecture, would require sync bridges |
| **Custom event loop (no tokio)** | Would need to reimplement async I/O, timer, subprocess management |
