# 07 — TUI Tabs

Only 3 tabs: **NowPlaying**, **Library**, **Settings**.

Queue, YouTube, and Help are removed as tabs and become overlays (see `08-gtm-tui-overlays.md`).

All tabs use the **Cyberdeck TUI** design system: plain (sharp) borders, high information density, monospaced JetBrains Mono aesthetic, bracket-style widgets, no emoji.

---

## Tab Bar

```
[1] Now Playing   [2] Library   [3] Settings                    gtm 0.7.34
──────────────────────────────────────────────────────────────────────────
```

- Active tab: `bg-secondary-container text-on-secondary-container` (`#454747` background, `#e5e2e1` text)
- Inactive tab: `text-on-surface-variant` (`#c4c7c7`), hover → `bg-primary text-on-primary`
- Version string right-aligned in header
- Plain border-bottom separator

---

## NowPlaying Tab

Two-column layout: album art (left) + metadata + controls (right).

```
──────────────────────────────────────────────────────────────────────────
┌─ Album Art ──┐  NOW PLAYING
│               │  ──────────────────────────────────
│   (grayscale  │  Codeine Crazy (Official Audio)
│    album      │  Artist: Future
│    art)       │  Format: [FLAC | 24-bit/96kHz]
│               │
│               │  ── Progress ──
│               │  00:45                         5:52
│               │  ▓▓▓▓▓▓░░░░░░░░░░░░░░░░░░░░░░░░░░░  (visualizer bars)
│               │  ▓▓▓▓▓▓▓▓▓▓▓▓░░░░░░░░░░░░░░░░░░░░
│               │  ▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓░░░░░░░░░░░░░░
│               │  ▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓░░░░░░░░░░
│               │  ▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓░░░░░░
│               │
│               │  [Space]P/P  [n]Next  [p]Prev  ...
└───────────────┘
```

- Progress is displayed as a **vertical audio visualizer** — a series of vertical bars of varying height
  - Played portion: `bg-tertiary` (`#00e639`)
  - Remaining portion: `bg-outline-variant` (`#444748`)
- Track metadata: title in prominent `headline-lg` bold, artist as label:value pair, format chip in brackets
- Album art rendered via half-block Unicode (`▀`) with CatmullRom resampling
- Controls at bottom in bracket notation

---

## LibraryTab

Full library view with track table and sidebar.

```
──────────────────────────────────────────────────────────────────────────
┌─ Library · 13 Tracks ──────────────────────────────────────────────┐
│  Up Next: Juice WRLD - Stay High              Listening time      │
│                                               48:12 / -32:15       │
│                                                                     │
│  #  │ Title / Artist / Album            │ Duration │ Bitrate       │
│  ───┼───────────────────────────────────┼──────────┼────────────── │
│  >01│ Future - Codeine Crazy            │ 05:41    │ 128kbps       │
│   02│ Juice WRLD - Stay High            │ 03:48    │ 320kbps       │
│   03│ Kanye West - Can't Tell Me Nothing│ 04:31    │ 320kbps       │
│  ...│                                   │          │               │
│                                                                     │
│  [Enter]Play  [a]Add  [d]Remove  [/]Search  [q]Quit                 │
└─────────────────────────────────────────────────────────────────────┘
```

- Sidebar (collapsed in this view — shown in separate overlay or as left pane):
  - Items: All Tracks, Artists, Albums, Playlists, Recent, Favorites
  - Active item: `bg-secondary-container text-on-secondary-container` with `border-l-4 border-tertiary`
  - Inactive items: `text-on-surface-variant`, `hover:bg-surface-container-highest`
  - Icons before each label (music_note, person, album, list, history, star)
- Track table uses fixed columns: `#` (3rem), `Title/Artist/Album` (1fr), `Duration` (6rem), `Bitrate` (6rem)
- Current playing track: `bg-secondary-container text-on-secondary-container font-bold`, `>` prefix, `play_arrow` icon in tertiary
- Other tracks: `hover:bg-surface-container-highest transition-colors cursor-pointer`
- Header shows track count in tertiary + up-next info + listening time
- Filterable via `/` search

---

## Settings Tab

Two-pane layout: sidebar (left) + settings panel (right).

```
──────────────────────────────────────────────────────────────────────────
┌─ ♫ Audio ───────────────────────────────────────────────────────────┐
│  ──────────────── YouTube ─────────────────                         │
│                                                                      │
│  Cookie Source          [ chromium    ▶ ]                            │
│  Cookie File            [ (none)      ▶ ]                            │
│  JS Runtime             [ deno        ▶ ]                            │
│  Max Downloads          ▓▓▓▓▓▓▓▓▓░░░  3                              │
│  Results Per Page       10                                            │
│  Search History         [ 0 entries  ▶ ]                              │
│  Auto Download          [ ● ]  On                                    │
│  Clear Search History   [Clear]                                      │
│                                                                      │
│  ── Help ───────────────────────────────                             │
│  YouTube integration: JS runtime, download limits, search prefs.     │
└──────────────────────────────────────────────────────────────────────┘
```

- Sidebar: `w-64` with icons (`♫`, `▶`, `✧`, `⚙`, `☊`) before each category
  - Active item: `border-l-2 border-magenta/60`, `bg-white/10` overlay
  - Inactive: `text-gray-400`, `hover:text-white`
  - Categories: Audio, YouTube, Appearance, System, Spotify
- Settings panel section headers: `─── Title ───` with accent color horizontal lines
- Settings items in key-value pairs with `hover:bg-white/5`
- Values shown in bracket style: `[ value ▶ ]`
- Toggles: `[ ● ]` with filled dot and "On"/"Off" label
- Sliders: vertical colored bars
- Action buttons: `[Clear]` in accent color
- Help section at bottom of each panel
