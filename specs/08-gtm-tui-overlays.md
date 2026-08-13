# 08 — TUI Overlays

Overlays float above all tabs with a semi-transparent background. Accessible via **Alt+key** from any tab. All overlays use the **Cyberdeck TUI** aesthetic: plain (sharp) borders, bracket-style widgets, `>` prompt for search inputs, and ASCII-only icons.

## Complete Overlay List

| Overlay | Alt+Key | Description |
|---------|---------|-------------|
| Queue | `Alt+Q` | View/manage playback queue |
| YTSearch | `Alt+Y` | Search YouTube, download/play results |
| SearchLibrary | `Alt+F` | Fuzzy-find local library tracks |
| SpotifySearch | `Alt+S` | Search Spotify via spot-dl |
| Equalizer | `Alt+E` | EQ presets with live waveform preview |
| CommandPalette | `Alt+P` | List/search all available actions |
| About | `Alt+A` | Version, stats, health info |
| SleepTimer | `Alt+Z` | Set sleep timer |
| ThemePicker | `Alt+T` | Live preview theme picker |
| SoundEffects | `Alt+X` | Sound effects settings |
| VolumeConfirm | (auto) | Confirm unsafe volume level |

## Generic Overlay Container

```rust
pub struct OverlayContainer<T> {
    pub fuzzy: Option<FuzzyFinder>,
    pub keymap: Keymap<T>,
    pub items: Vec<T>,
    pub selected: usize,
    pub query: String,
    pub z_index: u8,
    pub opacity: f64,
}
```

## Overlay Rendering

```
┌──────────────────────────────────────────────────┐
│  (terminal background, dimmed)                    │
│  ┌──────── > OVERLAY ──────────────────────────┐  │
│  │                                              │  │
│  │  > query_                                    │  │
│  │                                              │  │
│  │  > First Result                              │  │
│  │    Second Result                             │  │
│  │    Third Result                              │  │
│  │                                              │  │
│  │  [Enter]Select  [Esc]Close  [↑/↓]Navigate    │  │
│  └──────────────────────────────────────────────┘  │
└──────────────────────────────────────────────────┘
```

- Plain (sharp) borders throughout — no rounded corners
- Search input uses `>` prompt character with `_` cursor indicator
- No background color on input fields — plain fg text
- List items use full-width highlight on selection (`secondary-container` bg)
- All keybinding hints use bracket notation `[Key]Action`

## Per-Overlay Details

### Queue Overlay
- Shows current queue with track table layout:
  - Columns: `#`, `Title / Artist / Album`, `Duration`, `Bitrate`
- Current track: `border-l-2 border-tertiary`, `bg-secondary-container`, `>` prefix
- Other items: `hover:bg-surface-container-highest`, numbered `01.` / `02.` etc.
- `Enter` to play, `d`/`Del` to remove, `c` to clear
- Numbering: `01.` two-digit zero-padded format

### YTSearch Overlay
- Search field with `>` prompt and `_` cursor
- Results list: `Channel - Title [duration]`
- `Enter` to play, `Ctrl+d` to download, `Ctrl+a` to add to queue
- Results cached for navigation

### SearchLibrary Overlay
- Fuzzy finder on local tracks with `>` prompt and `_` cursor
- Real-time filtering as user types
- `Enter` to play, `Ctrl+a` to add to queue

### SpotifySearch Overlay
- Search Spotify via spot-dl backend
- Results: tracks, albums, playlists
- `Enter` to play/download

### Equalizer Overlay
- List of EQ presets: Flat, Rock, Pop, Jazz, Classical, Bass, Vocal
- ASCII waveform preview graph
- Active preset marked with `>` prefix
- Applies EQ in real-time

### CommandPalette Overlay
- Lists every available action as rows
- Filter field with `>` prompt and `_` cursor
- `Enter` to execute
- Shows keybinding hint in bracket notation `[Key]` for each action

### About Overlay
- Version info (gtm, gtmd, gtm-core, gtm-audio)
- Daemon health: status, volume, queue, shuffle, repeat
- All values use Cyberdeck color roles

### SleepTimer Overlay
- Quick options: 5m, 10m, 15m, 30m, 60m
- Shows remaining time when active
- Cancel option with `[Esc]`

### ThemePicker Overlay
- List of themes: Cyberdeck (default), Catppuccin Mocha, etc.
- Live preview: changes apply as user navigates

## Accessibility

All overlays:
- Open via `Alt+<key>` (configurable)
- Close via `Esc`
- Support `j`/`k` or `↑`/`↓` for navigation
- Show keybinding hints at the bottom in bracket notation
- Have tonal-layer background that dims the content below
