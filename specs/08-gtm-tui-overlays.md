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

## Generic Overlay Container

```rust
pub struct OverlayContainer<T> {
    /// Optional fuzzy finder for filtering items
    pub fuzzy: Option<FuzzyFinder>,
    /// Keymap specific to this overlay
    pub keymap: Keymap<T>,
    /// Items to display (type varies by overlay)
    pub items: Vec<T>,
    /// Selected index
    pub selected: usize,
    /// Search query
    pub query: String,
    /// Z-index (higher = on top)
    pub z_index: u8,
    /// Background opacity (0.0 - 1.0)
    pub opacity: f64,
}
```

## Overlay Rendering

```
┌──────────────────────────────────────────────────┐
│  (terminal background, dimmed via tonal layer)    │
│  ┌──────── > OVERLAY ──────────────────────────┐  │
│  │                                              │  │
│  │  > query_                                    │  │
│  │                                              │  │
│  │  [1] First Result                            │  │
│  │  [2] Second Result                           │  │
│  │  [3] Third Result                            │  │
│  │                                              │  │
│  │  [Enter]Select  [Esc]Close  [↑/↓]Navigate    │  │
│  └──────────────────────────────────────────────┘  │
└──────────────────────────────────────────────────┘
```

- Plain (sharp) borders throughout — no rounded corners
- Search input uses `>` prompt character instead of labeled "Search:" prefix
- No background color on input fields — plain fg text with `_` cursor indicator
- List items in overlays use full-width highlight on selection
- All keybinding hints use bracket notation `[Key]Action`

## Per-Overlay Details

### Queue Overlay
- Shows current queue items with `[1]`, `[2]`, etc. numbering
- Current track marked with `>` prefix
- `Enter` to play, `d`/`Del` to remove, `c` to clear
- Selected item highlighted with full-width background block

### YTSearch Overlay
- Search field with `>` prompt
- Results list: `Channel - Title [duration]`
- `Enter` to play, `Ctrl+d` to download, `Ctrl+a` to add to queue
- Results cached for navigation

### SearchLibrary Overlay
- Fuzzy finder on local tracks with `>` prompt
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
- Applies EQ in real-time via audio backend

### CommandPalette Overlay
- Lists every available action as rows
- Filter field with `>` prompt
- `Enter` to execute
- Shows keybinding hint in bracket notation `[Key]` for each action

### About Overlay
- Version info (gtm, gtmd, gtm-core, gtm-audio)
- Daemon health: status, volume, queue, shuffle, repeat
- All values use Cyberdeck color roles:
  - Status: `tertiary` (neon green) when playing
  - Volume: color-coded by level (green/yellow/red)
  - Queue/Shuffle/Repeat: primary foreground

### SleepTimer Overlay
- Quick options: 5m, 10m, 15m, 30m, 60m
- Shows remaining time when active
- Cancel option with `[Esc]`

### ThemePicker Overlay
- List of themes: Cyberdeck (default), Catppuccin Mocha, etc.
- Live preview: changes apply as user navigates
- Preview panel showing theme colors

## Accessibility

All overlays:
- Open via `Alt+<key>` (configurable)
- Close via `Esc`
- Support `j`/`k` or `↑`/`↓` for navigation
- Show keybinding hints at the bottom in bracket notation
- Have tonal-layer background that dims the content below
