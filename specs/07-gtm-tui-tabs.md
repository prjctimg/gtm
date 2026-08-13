# 07 — TUI Tabs

Only 3 tabs: **NowPlaying**, **Library**, **Settings**.

Queue, YouTube, and Help are removed as tabs and become overlays (see `08-gtm-tui-overlays.md`).

All tabs use the **Cyberdeck TUI** aesthetic: plain (sharp) borders, high information density, monospaced JetBrains Mono aesthetic, and bracket-style ASCII widgets. No emoji — all icons are ASCII characters.

---

## NowPlaying Tab

Displays current playback state with track metadata, progress bar, volume, and controls.

### Layout
```
> NOW PLAYING ───────────────────────────────────────
│                                                     │
│  Title:   Song Title                                │
│  Artist:  Artist Name                               │
│  Album:   Album Name                                │
│  Genre:   Genre (if available)                      │
│                                                     │
├─ 1:23 / 4:56 ──────────────────────────────────────┤
│  [###############-------------]                     │
│                                                     │
├─ [ VOL 80% ] ──────────────────────────────────────┤
│  [########-------------------]                      │
│                                                     │
├─────────────────────────────────────────────────────┤
│  [Space]P/P  [n]Next  [p]Prev  [+/-]Vol  [:]Cmd    │
│  [m]Mute  [r]Repeat  [h]Shuffle  [q]Quit            │
└─────────────────────────────────────────────────────┘
```

- Progress bar: bracketed `[###---]` style with `#` fill and `-` empty characters
- Volume bar changes color by level: tertiary green (<50%), on_surface (50-85%), error red (>85%)
- Track metadata labels (Title/Artist/Album/Genre) use dim foreground, values use bright/primary
- Controls use bracket notation for keybindings
- Cover art renders as half-block Unicode art (`▀`) with CatmullRom resampling when available

---

## Library Tab

Displays all tracks indexed in the daemon's SQLite library. Filterable via `/` search. Two-pane layout: numbered categories sidebar (left) + track listing (right).

### Layout
```
> LIBRARY ──────────────────────────────────────────────
│  1. All Tracks      1234  │  ▶ Artist - Song Title [3:45] │
│  2. Albums                 │    Artist2 - Another Song [4:20] │
│  3. Artists                │    Artist3 - Third Song [2:15]  │
│  4. Playlists              │    ...                          │
│  5. Recently Added         │                                  │
│  6. Most Played            │                                  │
│  7. Least Played           │                                  │
│  8. Spotify                │                                  │
│  9. Downloads              │                                  │
│                            │                                  │
├────────────────────────────┴──────────────────────────────────┤
│  1234 tracks · 123h 45m of playback                           │
└──────────────────────────────────────────────────────────────┘
```

- Left pane: categories numbered `1.` through `9.`, with track count right-aligned
- Active pane border highlighted with primary color (`#c8c6c5`)
- Selected item gets full-width background block in secondary container (`#454747`) with dark text (`#313030`)
- Filterable via `/` search (local filter on cached data: title, artist, album)
- Navigation: j/k or ↑/↓; Tab toggles pane focus; Enter to play selected
- Right pane shows tracks in `Artist - Title [duration]` format
- Stats bar: total filtered count + total playback time

---

## Settings Tab

Configures daemon and TUI settings. Two-pane layout: categories (left) + options (right). Help bar at bottom.

### Options
| Setting | Type | Description |
|---------|------|-------------|
| Volume | slider | 0-100% with unsafe warning at >85% |
| Repeat | toggle | Off / One / All |
| Shuffle | toggle | On/Off |
| Mute | toggle | On/Off |
| Crossfade | toggle + slider | Enable + duration (1-15s) |
| Crossfade Easing | select | Linear / Slow fade in-fast out / etc |
| Audio Backend | select | Rodio (default) |
| Library Paths | multi-input | Directories to scan for audio |
| Theme | select | Cyberdeck (default) / Catppuccin Mocha / etc |
| Progress Bar Style | select | Bracket / Line / Waveform |
| Footer Preset | select | Preset 1 / Preset 2 / Minimal |
| Opacity | slider | 0-100% for overlay backgrounds |

### Layout
```
> CATEGORIES ──────────── > AUDIO ────────────────────
│  Audio                  │  Volume:     80% [########--------] │
│  General                │  Mute:       OFF                    │
│  Playback               │                                     │
│  Appearance             │                                     │
│  Spotify                │                                     │
│                         │                                     │
│                         │                                     │
├───────────────────────────────────────────────────────────────┤
│  Volume: Adjust playback volume  |  Mute: Toggle mute          │
└───────────────────────────────────────────────────────────────┘
```

- Left pane: category list in a `>` block titled pane
- Right pane: options for selected category, content varies by category:
  - **Audio**: Volume slider, Mute toggle
  - **General**: Status indicator, Queue count
  - **Playback**: Repeat mode, Shuffle toggle, Crossfade toggle + duration
  - **Appearance**: Theme selector, Notification toggle
  - **Spotify**: Connection status
- Right pane content updates immediately when cycling categories
- Help bar shows relevant key hints for the selected category
