# GTM Specification

Project: gtm-rs — a terminal-based music player with daemon/client architecture.

## Architecture

```
┌─────────────┐     JSON lines      ┌─────────────┐     ALSA/Pulse     ┌──────────┐
│  gtm (TUI)  │ ◄───────────────► │ gtmd (Daemon)│ ◄───────────────► │  Audio   │
│  gtm (CLI)  │   Unix socket IPC  │              │                   │ Backend  │
└─────────────┘                    └─────────────┘                   └──────────┘
       │                                   │
       │    bincode frames (events)        │
       └───────────────────────────────►   │
           Dedicated pulse socket          │
```

### Components

| Component | Crate | Description |
|---|---|---|
| **gtm** | `gtm/` | TUI (ratatui + crossterm) and CLI (clap). Single binary, two modes. |
| **gtmd** | `gtmd/` | Background daemon. Audio playback via gtm-audio, SQLite library, IPC server. |
| **gtm-core** | `gtm-core/` | Shared types: IPC commands/events, state machine, track metadata. |
| **gtm-audio** | `gtm-audio/` | Audio abstraction: Mixer trait, rodio backend, crossfade support. |
| **gtm-mpris** | `gtm-mpris/` | MPRIS D-Bus interface (stub). |

## IPC Protocol

See `docs/ipc-protocol.md` for full specification.

**Framing:**
- Requests: JSON lines (`\n` delimited)
- Responses: JSON lines
- Events: binary frames `[4-byte BE length][bincode WireFrame]`
- Dedicated pulse socket for high-frequency events

**New commands (not yet in legacy docs):**
- `Crossfade` — `{"Crossfade":{"enabled":true,"duration_secs":7}}`
- `SetEqPreset` — `{"SetEqPreset":{"preset":"Rock"}}`

**New events:**
- `CrossfadeChanged` — crossfade config toggled
- `EqPresetChanged` — EQ preset changed

## Tabs

The TUI has exactly 3 tabs:

| # | Tab | Description |
|---|---|---|
| 1 | NowPlaying | Current track info, progress bar, volume, controls |
| 2 | Library | Left/right pane: list types | tracks |
| 3 | Settings | Left/right pane: categories | options |

Tab switching: `[1]` `[2]` `[3]` or `Tab`/`Shift+Tab` cycle.

## Overlays

Overlays float above tabs with semi-transparent background. Accessible via Alt+key.

| Overlay | Alt+Key | Status |
|---|---|---|
| Queue | `Alt+Q` | ✅ Implemented |
| YTSearch | `Alt+Y` | ✅ Implemented |
| SearchLibrary | `Alt+F` | ✅ Implemented |
| VolumeConfirm | (auto) | ✅ Implemented |
| ThemePicker | `Alt+T` | ❌ Not implemented |
| SpotifySearch | `Alt+S` | ❌ Not implemented |
| Equalizer | `Alt+E` | ❌ Not implemented |
| CommandPalette | `Alt+P` | ❌ Not implemented |
| About | `Alt+A` | ❌ Not implemented |
| SleepTimer | `Alt+Z` | ❌ Not implemented |
| SoundEffects | `Alt+X` | ❌ Not implemented |

## Themes

The TUI uses hardcoded colors. Planned themes:

- Catppuccin (Mocha, Macchiato, Frappe, Latte)
- Tokyonight (Night, Storm, Day)
- Gruvbox (Dark, Light)
- Ayu (Dark, Light, Mirage)
- mini.colorschemes (random, default)

## Daemon State

The `DaemonState` (gtm-core/src/state.rs) is the central state machine:

- **Track**: `TrackInfo` with path, title, artist, album, genre, duration, etc.
- **Queue**: `Vec<TrackInfo>` + cursor index
- **Volume**: 0–100, with mute flag
- **Playback**: Playing / Paused / Stopped
- **Repeat**: Off / One / All
- **Shuffle**: bool
- **Crossfade**: `Option<CrossfadeConfig>`
- **EQ Preset**: `EqPreset` enum

## Audio Pipeline

```
Track file → symphonia decoder → rodio Sink → ALSA/Pulse
                                    │
                            CrossfadeController
                            (linear fade, planned: easing)
```

### Crossfade

- Config: enabled + duration (0–30s, default 7s)
- Triggered on track end or manual Next/Prev
- Linear fade (planned: easing functions)

### EQ (planned)

- 10-band IIR filter bank
- Presets: Flat, Pop, Rock, Jazz, Classical, Bass, Vocal, Custom([f32;10])
- No DSP implemented yet (placeholder enum only)

## Implementation Phases

### ✅ Phase 3.3 — Aesthetic Polish
- Progress bar with oscillating head
- Braille spinners
- Nerd icons with emoji fallback
- Volume colors (green/yellow/red)

### ✅ Phase 4 — Audio & Crossfade Features
- Crossfade toggle/duration in Settings
- Volume safety challenge
- EQ preset type + IPC
- Default crossfade: 7s

### ✅ Phase 4.5 — Bug Fixes
- Fixed `blocking_lock()` crash in ipc worker
- Fixed 10s blank screen on startup (background auto-scan)
- Clamp unsafe volume on TUI start

### ⬜ Phase 0 — Tab/Overlay Architecture
- Shrink `Tab` enum to 3 variants
- Remove Queue/YouTube/Help from tabs
- Fix tab switching dead code

### ⬜ Phase 1 — Overlay Completion
- About overlay (version, GPL, stats)
- SleepTimer overlay
- CommandPalette overlay
- Equalizer overlay (placeholder)
- SoundEffects overlay
- ThemePicker overlay

### ⬜ Phase 2 — Theme System
- AppTheme struct with color tokens
- Replace all hardcoded colors
- Catppuccin, Tokyonight, Gruvbox, Ayu, mini presets
- ThemePicker overlay

### ⬜ Phase 3 — Cover Art
- Wire up ratatui-image
- Trigger daemon cover fetch
- Render in NowPlaying tab
- Embedded cover extraction

### ⬜ Phase 4 — Library/Settings Redesign
- Left/right pane layout
- Library stats
- Settings categories + help text
- Responsive layout

### ⬜ Phase 5 — Footer, Notifications, Progress
- Toast notification system
- Customizable footer modules
- Progress bar variants

### ⬜ Phase 6 — Audio Enhancements
- EQ DSP (biquad filter bank)
- Crossfade easing
- Reverb
- Volume dip on pause
- Smart volume normalization
- Playback speed

### ⬜ Phase 7 — Packaging
- DEB packaging with cargo-deb
- APT repository CI
- Shell completions
- Manpage generation

### ⬜ Phase 8 — Legacy Compliance
- Missing CLI commands (daemon, sleep, now)
- snake_case IPC
- Copyright headers
- Dynamic versioning

### ⬜ Phase 9 — Polish
- Error handling audit (unwrap → ?)
- Responsive design (min 80x24)
- MPRIS implementation
