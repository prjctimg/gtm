# 07 — gtm-tui: Tabs

## Purpose

Each of the 6 tabs implements the `TabWidget` trait and renders its content into a `Rect` area
of the terminal. Tabs share the `AppState` for data and `DaemonClient` for IPC.

## TabWidget Trait

```rust
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::prelude::*;
use crossterm::event::KeyEvent;

pub trait TabWidget {
    /// Render this tab's content into the given area
    fn render(&mut self, area: Rect, buf: &mut Buffer, state: &mut AppState);

    /// Handle a keyboard event. Returns Action (consumed or not).
    fn handle_key(&mut self, key: KeyEvent, state: &mut AppState) -> Action;
}

pub enum Action {
    Consumed,
    NotConsumed,
    Quit,
    SwitchTab(Tab),
    OpenOverlay(Overlay),
}

pub enum Tab {
    NowPlaying,
    Library,
    Queue,
    YouTube,
    Settings,
    Help,
}
```

## Per-Tab View States

```rust
// ─── LibraryTab State ───

#[derive(Debug, Default)]
pub struct LibraryViewState {
    pub sub_tab: LibrarySubTab,         // Tracks | Playlists | Favourites | Recent
    pub tracks: Vec<TrackInfo>,
    pub cursor: usize,
    pub scroll: u16,
    pub sort_column: SortColumn,
    pub sort_desc: bool,
    pub filter_text: String,
    pub loading: bool,
    pub scan_progress: Option<ScanProgress>,
    pub playlist_tracks: Vec<TrackInfo>,    // when viewing inside a playlist
    pub selected_playlist_id: Option<i64>,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum LibrarySubTab {
    #[default]
    Tracks,
    Playlists,
    Favourites,
    Recent,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SortColumn {
    Title,
    Artist,
    Album,
    Duration,
    Year,
    AddedAt,
}

impl Default for SortColumn { fn default() -> Self { Self::Title } }

// ─── QueueTab State ───

#[derive(Debug, Default)]
pub struct QueueViewState {
    pub cursor: usize,
    pub scroll: u16,
    pub move_mode: bool,
    pub move_source: usize,
    pub move_cursor: usize,      // insertion indicator position
}

// ─── NowPlayingTab State ───

#[derive(Debug, Default)]
pub struct NowPlayingState {
    pub show_up_next: bool,
    pub up_next_list: Vec<TrackInfo>,
    pub lyrics_scroll: u16,
    pub album_art_id: Option<u32>,     // Kitty image ID for cleanup
}

// ─── YouTubeTab State ───

#[derive(Debug, Default)]
pub struct YouTubeViewState {
    pub sub_tab: YouTubeSubTab,     // Search | Playlists
    pub query: String,
    pub results: Vec<YtSearchResult>,
    pub cursor: usize,
    pub scroll: u16,
    pub loading: bool,
    pub page: usize,
    pub total_pages: usize,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum YouTubeSubTab {
    #[default]
    Search,
    Playlists,
}

// ─── SettingsTab State ───

#[derive(Debug, Default)]
pub struct SettingsState {
    pub cursor: usize,
    pub scroll: u16,
    pub editing: Option<usize>,        // index of setting being edited
}
```

---

## LibraryTab

```
┌──────────────────────────────────────────────────────────────┐
│  [Tracks] │ Playlists │ Favourites │ Recent                  │
├──────────────────────────────────────────────────────────────┤
│  ♪  artist - title                              3:45        │
│  ♪  another artist - longer title title         5:12        │
│  ♪  third artist - song                            ← cursor │
│  ♪  fourth - short                               2:01        │
│  ♪  fifth artist - really long title here       4:33        │
│  ♪  ...                                                        │
│                                                               │
│  Scan progress: ████████░░░░░░░░░░ 12/48 tracks scanned     │
│                                                               │
│  Filter: /search term...                                      │
├──────────────────────────────────────────────────────────────┤
│  Total: 248 tracks  •  15 playlists  •  23 favourites        │
└──────────────────────────────────────────────────────────────┘
```

### LibraryTab Implementation

```rust
pub struct LibraryTab;

impl TabWidget for LibraryTab {
    fn render(&mut self, area: Rect, buf: &mut Buffer, state: &mut AppState) {
        // 1. Render sub-tab header (4 horizontal tabs)
        // 2. If filter_text not empty, show filter bar
        // 3. If scan_progress, show progress bar
        // 4. Render track list (or playlist list, or favourite list)
        // 5. Render status line at bottom
    }

    fn handle_key(&mut self, key: KeyEvent, state: &mut AppState) -> Action {
        match state.mode {
            UIMode::Filter => { /* type into filter_text */ }
            UIMode::Normal => {
                match key.code {
                    // Navigation
                    KeyCode::Up | KeyCode::Char('k') => { move_cursor(-1); }
                    KeyCode::Down | KeyCode::Char('j') => { move_cursor(1); }
                    KeyCode::PageUp => { page_up(); }
                    KeyCode::PageDown => { page_down(); }
                    KeyCode::Home | KeyCode::Char('g') => { cursor = 0; }
                    KeyCode::End | KeyCode::Char('G') => { cursor = max; }

                    // Actions
                    KeyCode::Enter => { play_selected(); }
                    KeyCode::Char(' ') => { add_to_queue_end(); }
                    KeyCode::Char('a') => { add_to_queue_next(); }
                    KeyCode::Char('/') => { enter_filter_mode(); }
                    KeyCode::Char('d') => { open_track_detail(); }
                    KeyCode::Char('f') => { toggle_favourite(); }
                    KeyCode::Char('p') => { add_to_playlist_prompt(); }
                    KeyCode::Char('s') => { cycle_sort(); }
                    KeyCode::Char('S') => { toggle_sort_desc(); }
                    KeyCode::Char('r') => { start_scan(); }

                    // Sub-tab switching
                    KeyCode::Tab | KeyCode::Char('l') => { next_subtab(); }
                    KeyCode::BackTab | KeyCode::Char('h') => { prev_subtab(); }

                    // Navigation
                    KeyCode::Esc => { if in_playlist_view: back_to_playlists() }
                    KeyCode::Enter => { if on_playlist: open_playlist() }
                    _ => {}
                }
            }
            _ => {}
        }
        Action::Consumed
    }
}
```

### Navigation

```
j/k or ↑/↓            Move cursor
Enter                 Play selected track
Space                 Add to queue (append)
a                     Add to queue (next)
/                     Enter filter mode
Esc                   Exit filter / go back
Tab, Shift+Tab       Cycle header tabs (or h/l)
d                     Show track detail overlay
f                     Toggle favourite
p                     Add to playlist prompt
s                     Sort by (artist/album/title/duration/year)
S                     Reverse sort
r                     Rescan directory
gg / G                Jump to top / bottom
Ctrl+d / Ctrl+u       Page down / page up
```

### Playlist sub-view

```
┌──────────────────────────────────────────────────────────────┐
│  [Tracks] │ [Playlists] │ Favourites │ Recent                │
├──────────────────────────────────────────────────────────────┤
│  ┌──────────────────────────────────────────────────────┐   │
│  │  My Chill Mix                    12 tracks           │   │
│  │  Road Trip                                        ← │   │
│  │  2024 Favourites                   42 tracks        │   │
│  │  Workout Energy                     8 tracks        │   │
│  │  Jazz for Studying                 23 tracks        │   │
│  ├──────────────────────────────────────────────────────┤   │
│  │  n  New playlist /  d  Delete selected              │   │
│  └──────────────────────────────────────────────────────┘   │
└──────────────────────────────────────────────────────────────┘

  Enter on playlist → enter playlist track list view
  Esc in playlist track list → back to playlists
  n → command palette open with ":create playlist " prefilled
  d → confirm dialog → delete playlist
```

---

## QueueTab

```
┌──────────────────────────────────────────────────────────────┐
│  Queue (5 tracks)                                             │
├──────────────────────────────────────────────────────────────┤
│  ▶ Now Playing                                                │
│    ┌──────────────────────────────────────────────────────┐   │
│    │  ♪  Currently Playing Artist - Playing Song          │   │
│    │  ⏵  1:23 / 4:56  ████████░░░░░ 28%                  │   │
│    └──────────────────────────────────────────────────────┘   │
│                                                               │
│  ▸ Up Next                                                    │
│    ┌──────────────────────────────────────────────────────┐   │
│    │  1.  Artist A - Song One                 5:01        │   │
│    │  2.  Artist B - Song Two                 3:45    ←   │   │
│    │  3.  Artist C - Song Three               4:12        │   │
│    │  4.  Artist A - Song Four                3:22        │   │
│    │                                                    │   │
│  ═══════════════════════════════════════════════════════════   │
│                                                               │
│  (end of queue — 3 more in library)                          │
│                                                               │
├──────────────────────────────────────────────────────────────┤
│  Total: 5  |  D: delete  M: move  C: clear  S: save playlist │
└──────────────────────────────────────────────────────────────┘
```

### QueueTab Implementation

```rust
pub struct QueueTab;

impl TabWidget for QueueTab {
    fn render(&mut self, area: Rect, buf: &mut Buffer, state: &mut AppState) {
        // 1. Render "Now Playing" section (header + current track + progress)
        // 2. If move_mode true: show "Move mode: j/k to position, Enter to confirm"
        // 3. Render "Up Next" track list with cursor
        // 4. If queue empty, show empty message
    }

    fn handle_key(&mut self, key: KeyEvent, state: &mut AppState) -> Action {
        if state.queue.move_mode {
            // move mode keys
            match key.code {
                KeyCode::Up | KeyCode::Char('k') => move_cursor(-1);
                KeyCode::Down | KeyCode::Char('j') => move_cursor(1);
                KeyCode::Enter => confirm_move();
                KeyCode::Esc => cancel_move();
                _ => {}
            }
        } else {
            match key.code {
                KeyCode::Up | KeyCode::Char('k') => move_cursor(-1);
                KeyCode::Down | KeyCode::Char('j') => move_cursor(1);
                KeyCode::Enter | KeyCode::Char(' ') => play_selected();
                KeyCode::Char('d') => remove_selected();
                KeyCode::Char('m') | KeyCode::Char('M') => enter_move_mode();
                KeyCode::Char('C') => open_confirm_clear_queue();
                KeyCode::Char('s') => save_as_playlist();
                _ => {}
            }
        }
        Action::Consumed
    }
}
```

### Move Mode

```
Move mode: j/k to position, Enter to confirm, Esc to cancel

  1.  Artist A - Song One                 5:01
  →  2.  Artist B - Song Two             3:45   ◄─ source marked
  3.  Artist C - Song Three               4:12
   ↑
   └─ cursor shows insertion point

Implementation:
  move_source = cursor (original index)
  Enter move mode → cursor shows current insertion preview
  j/k moves cursor (insertion point), source stays highlighted
  Enter → DaemonRequest::Queue(QueueAction::Move { from: source, to: cursor })
  Esc → cancel, restore cursor to source
```

---

## NowPlayingTab

```
┌──────────────────────────────────────────────────────────────┐
│  ┌────────────────────┐  ◀── Album art (Kitty image or box)  │
│  │                    │                                       │
│  │    ♫   Album      │  Song Title                            │
│  │        Art         │  Artist Name                          │
│  │                    │  Album Name                    2014   │
│  │    (Kitty image)   │  ─────────────────────────────         │
│  │                    │  ████████████████░░░░  2:34 / 4:20    │
│  │                    │                                       │
│  └────────────────────┘  ◀◀  ⏸  ▶▶  │◀  ▶│  🔀  🔁  🔂     │
│                                                               │
│  ┌─ Lyrics ───────────────────────────────────────────────┐   │
│  │                                                         │   │
│  │  🎤  This is the first line of the song                │   │
│  │  🎤  Second line goes here                             │   │
│  │  🎤  Third line — current position → green highlight   │   │
│  │  🎤  Fourth line                                       │   │
│  │  🎤  Fifth line of lyrics                              │   │
│  │                                                         │   │
│  └─────────────────────────────────────────────────────────┘   │
│                                                               │
│  ▸ Up Next                                                    │
│    1. Next Artist - Next Song                                 │
│    2. Another - Different Song                                │
└──────────────────────────────────────────────────────────────┘
```

### Lyric Line Highlighting

```
For synced lyrics (LrcData with timestamps):

  1. Get extrapolated position
  2. Binary search to find line with largest timestamp ≤ position
  3. That line is "current" — rendered with accent (green) color
  4. Lines before: dimmed / subtext color
  5. Lines after: normal text color
  6. Auto-scroll: keep current line in center of viewport

  For unsynced lyrics (single block, no timestamps):
  → Display all lines, no highlighting, user scrolls manually
```

### Navigation

```
Space              Play / Pause
→ / l             Seek forward 5s
← / h             Seek backward 5s
s                 Toggle shuffle
r                 Cycle repeat (Off → One → All → Off)
m                 Toggle mute
↑ / ↓ / j / k     Scroll lyrics
+ / =             Volume up 5
-                 Volume down 5
q                 Toggle Up Next panel
```

---

## YouTubeTab

```
┌──────────────────────────────────────────────────────────────┐
│  [Search] │ Playlists                                          │
├──────────────────────────────────────────────────────────────┤
│  ┌──────────────────────────────────────────────────────┐    │
│  │  🔍 Search YouTube (press / to search)               │    │
│  └──────────────────────────────────────────────────────┘    │
│                                                               │
│  Search results for "jazz lofi chill":                        │
│  ┌──────────────────────────────────────────────────────┐    │
│  │  ▶  Lofi Jazz Beats for Studying 2024               │    │
│  │     by Chill Music Lab         • 1,234,567 views    │    │
│  │     ──────────────────────────────────────────────── │    │
│  │     ▶  Smooth Jazz Playlist — 5 Hours             ← │    │
│  │     by Jazz Vibes          • 892,451 views          │    │
│  │     ──────────────────────────────────────────────── │    │
│  │     ▶  Late Night Jazz — Relaxing Mix               │    │
│  │     by Coffee Shop Jazz    • 2,103,456 views        │    │
│  │     ──────────────────────────────────────────────── │    │
│  │     ▶  Jazz for Sleep — Calm Piano & Sax           │    │
│  │     by Relaxing Music      • 567,890 views          │    │
│  │  └──────────────────────────────────────────────────────┘    │
│                                                               │
│  Results 1-4 of 10  (page 1/3)                                │
├──────────────────────────────────────────────────────────────┤
│  Enter: play stream  |  a: add to queue  |  /: search        │
└──────────────────────────────────────────────────────────────┘
```

### Navigation

```
/                   Enter search (opens filter bar)
Enter               Play stream (resolve URL → daemon → play)
a                   Add stream URL to local queue
d                   Show stream details
l                   Toggle playlist subtab
j/k or ↑/↓          Move cursor
r                   Refresh / re-search
Esc                 Clear search / back
```

---

## SettingsTab

```
┌──────────────────────────────────────────────────────────────┐
│  Settings                                                     │
├──────────────────────────────────────────────────────────────┤
│  ⚙  Volume              ░░░░░░░░▓▓▓▓▓▓▓▓  75%               │
│  ⚙  Crossfade           [x]  3s                              │
│  ⚙  Theme               Light v Dark  ▶  ← cursor           │
│  ⚙  Icons               Nerd Font  ▶                        │
│  ⚙  Audio Backend       Symphonia  ▶                        │
│  ⚙  Tab Order           [Library] [Queue] [Now Playing] ...  │
│  ⚙  Library Paths       ~/Music/  ▶                         │
│  ⚙  Search History      Clear                                │
│  ⚙  Cache Size          500 items                            │
│  ────────────────────────────────────────────────────────     │
│  ⚙  About gtm v0.2.0                                         │
│  ⚙  Check for Updates                                        │
│                                                               │
└──────────────────────────────────────────────────────────────┘
```

### Settings Items

```rust
pub enum SettingItem {
    Volume,
    Crossfade(bool, u8),
    Theme(ThemeMode),
    IconSet(IconChoice),
    AudioBackend(AudioBackendKind),
    TabOrder(Vec<Tab>),
    LibraryPaths(Vec<PathBuf>),
    ClearSearchHistory,
    ClearCache,
    About,
}
```

### Navigation

```
j/k or ↑/↓          Move cursor
Enter               Toggle / open sub-setting
← / →               Decrease / increase value
Esc                 Back
```

---

## HelpTab

```
┌──────────────────────────────────────────────────────────────┐
│  Help — Keybindings                                           │
├──────────────────────────────────────────────────────────────┤
│                                                               │
│  Global                                                       │
│  ┌──────────────────────────────────────────────────────┐    │
│  │  Tab / Shift+Tab     Next/Previous tab              │    │
│  │  1-6                 Switch to tab #                │    │
│  │  Ctrl+c / q          Quit                           │    │
│  │  :                   Command palette                │    │
│  │  ?                   Toggle help tab                │    │
│  │  Ctrl+r              Reload config                  │    │
│  └──────────────────────────────────────────────────────┘    │
│  ...                                                           │
│  Press any key to close                                       │
└──────────────────────────────────────────────────────────────┘
```

Rendered as a static widget — no state needed.

## File Structure

```
gtm-tui/src/tabs/
├── mod.rs             # TabWidget trait + Tab enum + Action enum
├── library.rs         # LibraryTab + LibraryViewState
├── queue.rs           # QueueTab + QueueViewState
├── now_playing.rs     # NowPlayingTab + NowPlayingState
├── youtube.rs         # YouTubeTab + YouTubeViewState
├── settings.rs        # SettingsTab + SettingsState
└── help.rs            # HelpTab (static widget)
```
