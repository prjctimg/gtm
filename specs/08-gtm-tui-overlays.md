# 08 — TUI Overlays

Overlays float above all tabs with a semi-transparent background (default 90% opacity). Accessible via **Alt+key** from any tab.

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
| SleepTimer | `Alt+T` | Set sleep timer |
| ThemePicker | `Alt+M` (theme) | Live preview theme picker |

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
│  ⋮ (terminal background, dimmed)                  │
│  ┌─────── Overlay (semi-transparent) ──────────┐  │
│  │  Search: [________________]                  │  │
│  │                                              │  │
│  │  ▶ Result 1                                  │  │
│  │    Result 2                                  │  │
│  │    Result 3                                  │  │
│  │                                              │  │
│  │  [Enter] Select  [Esc] Close  [↑/↓] Navigate │  │
│  └──────────────────────────────────────────────┘  │
└──────────────────────────────────────────────────┘
```

## Per-Overlay Details

### Queue Overlay
- Shows current queue with drag-to-reorder (MoveUp/MoveDown keys)
- Add tracks from library, remove, clear
- Shows current track indicator
- `Enter` to play, `d`/`Del` to remove, `c` to clear

### YTSearch Overlay
- Search field, results list
- `Enter` to play, `Ctrl+d` to download, `Ctrl+a` to add to queue
- Playlist support (download entire playlist)
- Results cached for navigation

### SearchLibrary Overlay
- Fuzzy finder on local tracks
- Real-time filtering as user types
- `Enter` to play, `Ctrl+a` to add to queue

### SpotifySearch Overlay
- Search Spotify via spot-dl backend
- Results: tracks, albums, playlists
- `Enter` to play/download

### Equalizer Overlay
- List of EQ presets: Flat, Rock, Pop, Jazz, Classical, Bass Boost, Vocal, Custom
- Live waveform preview graph (ASCII) updating as user navigates
- Applies EQ in real-time via audio backend

### CommandPalette Overlay
- Lists every available action as rows
- Fuzzy search to filter
- `Enter` to execute
- Shows keybinding hint for each action

### About Overlay
- Version info (gtm, gtmd, gtm-core, gtm-audio)
- System stats: CPU usage, memory, storage
- Daemon health: uptime, connected clients, queue length
- Library stats: tracks, playlists, favourites

### SleepTimer Overlay
- Quick options: 15m, 30m, 45m, 60m, Custom
- Shows remaining time when active
- Cancel option

### ThemePicker Overlay
- Live preview: changes apply as user navigates
- List of themes: Dark (default), Light, Transparent
- HSL seed input for custom theme generation
- Preview panel showing all 26 theme colors

## Accessibility

All overlays:
- Open via `Alt+<key>` (configurable)
- Close via `Esc`
- Support `Tab`/`Shift+Tab` for focus cycling within the overlay
- Show keybinding hints at the bottom
- Have semi-transparent background that dims the content below
