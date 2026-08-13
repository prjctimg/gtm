# 07 — TUI Tabs

Only 3 tabs: **NowPlaying**, **Library**, **Settings**.

Queue, YouTube, and Help are removed as tabs and become overlays (see `08-gtm-tui-overlays.md`).

---

## NowPlaying Tab

Displays current playback state with track metadata, progress bar, volume, and controls.

### Layout
```
┌─ Now Playing (rounded) ──────────────────────────┐
│                                                    │
│  Title:   Song Title (bold white)                  │
│  Artist:  Artist Name (yellow)                     │
│  Album:   Album Name (cyan)                        │
│  Genre:   Genre (green) (if available)             │
│                                                    │
├─ 1:23 / 4:56 (rounded) ───────────────────────────┤
│  ════════════○══════════════════════════════════   │
│  (line progress bar with oscillating head)         │
│                                                    │
├─ Volume: 80% (rounded) ───────────────────────────┤
│  ████████████░░░░░░░░░░░░░░░░░░░░                  │
│                                                    │
├────────────────────────────────────────────────────┤
│  Controls: [Space]P/P [n]Next [p]Prev [+/-]Vol    │
└────────────────────────────────────────────────────┘
```

- Progress bar: line with oscillating head (material design style)
- Volume bar changes color by level: green (<50%), yellow (50-85%), red (>85%)
- Braille spinner when buffering/loading
- Nerd icons for status, emoji fallback

---

## Library Tab

Displays all tracks indexed in the daemon's SQLite library. Filterable via `/` search.

### Layout
```
┌─ Library (rounded) ──────────────────────────────┐
│  ▶ Artist - Song Title [3:45]                     │
│    Artist2 - Another Song [4:20]                  │
│    Artist3 - Third Song [2:15]                    │
│    ...                                            │
│                                                    │
│  [Enter] Play  [a] Add to Queue  [f] Favourite    │
│  [/] Search                                       │
└────────────────────────────────────────────────────┘
```

- Filterable by title, artist, album (local filter on cached data)
- Selected item highlighted with cyan background
- Navigation: j/k or ↑/↓
- Action keys: Enter (play), 'a' (add to queue), 'f' (toggle favourite)

---

## Settings Tab

Configures daemon and TUI settings.

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
| Theme | select | Dark / Light / Transparent |
| Progress Bar Style | select | Line / Braille / Waveform |
| Footer Preset | select | Preset 1 / Preset 2 / Minimal |
| Opacity | slider | 0-100% for overlay backgrounds |

### Layout
```
┌─ Settings (rounded) ─────────────────────────────┐
│  Volume:      80% [████████░░░░]                  │
│  Repeat:      OFF  [Cycle]                        │
│  Shuffle:     OFF [Toggle]                        │
│  Mute:        OFF [Toggle]                        │
│  Crossfade:   ON   Duration: 7s [─╶──]           │
│  Easing:      Linear ▼                            │
│  Library:     /home/user/Music [...]              │
│  Progress:    Line ▼                              │
│  Footer:      Preset 1 ▼                          │
│  Opacity:     90% [██████████░░]                  │
└────────────────────────────────────────────────────┘
```
