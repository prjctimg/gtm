# 08 — gtm-tui: Overlays

## Purpose

Overlays are modal widgets rendered on top of the active tab. They capture keyboard input
until dismissed. Examples: command palette, fuzzy finder, track detail, confirm dialog.

## Overlay Enum and Trait

```rust
pub enum Overlay {
    CommandPalette(CommandPaletteState),
    FuzzyFinder(FuzzyFinderState),
    QueuePicker(QueuePickerState),
    ThemePicker(ThemePickerState),
    Confirm(ConfirmState),
    TrackDetail(TrackDetailState),
}

impl Overlay {
    pub fn render(&mut self, area: Rect, buf: &mut Buffer, state: &AppState) {
        // 1. Dim the background with semi-transparent block (surface0 color)
        let overlay_area = centered_rect(area, 80, 70);
        clear_area(overlay_area, state.theme.surface0, buf);

        // 2. Render specific overlay content
        match self {
            Overlay::CommandPalette(p) => p.render(overlay_area, buf, state),
            Overlay::FuzzyFinder(f)    => f.render(overlay_area, buf, state),
            Overlay::QueuePicker(q)    => q.render(overlay_area, buf, state),
            Overlay::ThemePicker(t)    => t.render(overlay_area, buf, state),
            Overlay::Confirm(c)        => c.render(overlay_area, buf, state),
            Overlay::TrackDetail(t)    => t.render(overlay_area, buf, state),
        }
    }

    pub fn handle_key(&mut self, key: KeyEvent, state: &mut AppState) -> bool {
        let consumed = match self {
            Overlay::CommandPalette(p) => p.handle_key(key, state),
            Overlay::FuzzyFinder(f)    => f.handle_key(key, state),
            Overlay::QueuePicker(q)    => q.handle_key(key, state),
            Overlay::ThemePicker(t)    => t.handle_key(key, state),
            Overlay::Confirm(c)        => c.handle_key(key, state),
            Overlay::TrackDetail(t)    => t.handle_key(key, state),
        };
        // If overlay closes itself, set state.overlay = None
        consumed
    }
}

/// Center an overlay rect within the given area
fn centered_rect(area: Rect, percent_x: u16, percent_y: u16) -> Rect {
    let popup_x = (area.width as u16 * percent_x / 100).max(40);
    let popup_y = (area.height as u16 * percent_y / 100).max(10);
    Rect {
        x: area.x + (area.width - popup_x) / 2,
        y: area.y + (area.height - popup_y) / 2,
        width: popup_x,
        height: popup_y,
    }
}
```

---

## Command Palette (`:`)

```
┌──────────────────────────────────────────────────────────────────┐
│  :play some song                                       (28/100)  │
├──────────────────────────────────────────────────────────────────┤
│  > play some song                                          🎵   │
│  play track "some" from library                           ▶     │
│  play album "Some Album"                                         │
│  search YouTube for "some song"                                  │
│  add "some song" to queue                                        │
│  play "some other song" from favourites                          │
│  scan music directory                                            │
│  shuffle all tracks                                              │
│  set volume 75                                                   │
│  create playlist "new mix"                                       │
│  ...                                                             │
│                                                                  │
│  Available commands:                                             │
│    play <query>        play search library                       │
│    pause               pause playback                            │
│    stop                stop playback                             │
│    next                next track                                │
│    prev                previous track                            │
│    volume <0-100>      set volume                                │
│    shuffle             toggle shuffle                            │
│    repeat <off|one|all> set repeat mode                          │
│    crossfade <secs>    set crossfade duration                    │
│    sleep <min>         set sleep timer                           │
│    scan <path>         scan directory into library               │
│    search <query>      search tracks                             │
│    quit                quit gtm                                  │
└──────────────────────────────────────────────────────────────────┘
```

### Command Matching Algorithm

```
Input is matched against a list of CommandItem entries:
  1. Split input into command part (first word) and args (rest)
  2. Find command whose name is a prefix of the input command
     → "pl" matches "play", "pl" matches "playlist" — show both
  3. If exact command match, show that command highlighted
  4. Also do fuzzy search across all command descriptions
  5. Return results sorted by: exact > prefix > fuzzy description

Example:
  ":pl" → "play <query>", "playlist create <name>", "playlist list"
  ":v 75" → "set volume 75" (implicit: volume command)
```

### State

```rust
pub struct CommandPaletteState {
    pub input: String,
    pub cursor_pos: usize,             // cursor position within input
    pub results: Vec<CommandItem>,
    pub selected: usize,
}

pub struct CommandItem {
    pub command: String,               // display text
    pub description: String,
    pub icon: &'static str,
    pub run: fn(&mut AppState),         // what to execute on Enter
}

impl CommandPaletteState {
    pub fn new() -> Self;
    pub fn render(&self, area: Rect, buf: &mut Buffer, state: &AppState);
    pub fn handle_key(&mut self, key: KeyEvent, state: &mut AppState) -> bool;

    fn update_results(&mut self);
    fn execute_selected(&mut self, state: &mut AppState);
}
```

---

## Fuzzy Finder

```
┌──────────────────────────────────────────────────────────────────┐
│  Filter tracks: lofij                                           │
├──────────────────────────────────────────────────────────────────┤
│  ♪  Lofi Jazz Beats — Chill Music Lab                  3:45    │
│  ♪  Lo-fi Study Mix — Study Girl                       2:15    │
│  ♪  Jazz & Lofi — Relax Daily                          4:30    │
│  ♪  lofi hip hop radio — beats to relax/study to    ∞  │
│  ♪  Midnight Lo-Fi — Sleepy Beats                    3:00  ← │
│  ♪  ...                                                         │
│                                                                  │
│  Matches: 48 of 1,234 tracks                                    │
└──────────────────────────────────────────────────────────────────┘
```

### Fuzzy Matching Algorithm

```rust
/// Score how well `query` matches `text` using subsequence matching.
/// Returns (score, matched_ranges) where score is:
///   - Exact match: 1000 + text.len()
///   - Prefix match: 900 + text.len()
///   - Subsequence match: text.len() * 10 - gaps * 5
///   - No match: 0
///
/// Case-insensitive matching.
pub fn fuzzy_score(query: &str, text: &str) -> (u32, Vec<usize>);

/// Generate an iterator of (score, item) pairs sorted descending by score.
pub fn fuzzy_filter<T>(
    query: &str,
    items: &[T],
    get_text: fn(&T) -> &str,
) -> Vec<(u32, &T)>;
```

### State

```rust
pub enum FuzzyMode {
    Tracks,
    Playlists,
    Queue,
    YouTube,
}

pub struct FuzzyFinderState {
    pub query: String,
    pub items: Vec<FuzzyItem>,
    pub cursor: usize,
    pub mode: FuzzyMode,
}

pub struct FuzzyItem {
    pub primary: String,
    pub secondary: String,
    pub detail: String,
    pub icon: &'static str,
    pub track: Option<TrackInfo>,
    pub playlist: Option<Playlist>,
}

impl FuzzyFinderState {
    pub fn new(mode: FuzzyMode, state: &AppState) -> Self;
    pub fn render(&self, area: Rect, buf: &mut Buffer, state: &AppState);
    pub fn handle_key(&mut self, key: KeyEvent, state: &mut AppState) -> bool;
}
```

---

## Queue Picker

```
┌──────────────────────────────────────────────────────────────────┐
│  Insert at position in queue:                                    │
├──────────────────────────────────────────────────────────────────┤
│  ▶ Now Playing by Current Artist                   1:23        │
│  ─────────────────────────────────────────────────────────────  │
│  1.  First track in queue                         3:45         │
│  2.  Second track                                2:15  ← pos  │
│  3.  Third track                                 4:30         │
│  4.  Fourth track                                3:00         │
│  ─────────────────────────────────────────────────────────────  │
│  (end of queue) — append here                                  │
│                                                                  │
│  j/k to position, Enter to confirm, Esc to cancel                │
└──────────────────────────────────────────────────────────────────┘
```

### State

```rust
pub struct QueuePickerState {
    pub cursor: usize,
    pub on_pick: fn(&mut AppState, position: usize),
}

impl QueuePickerState {
    pub fn new(on_pick: fn(&mut AppState, usize)) -> Self;
    pub fn render(&self, area: Rect, buf: &mut Buffer, state: &AppState);
    pub fn handle_key(&mut self, key: KeyEvent, state: &mut AppState) -> bool;
}
```

---

## Theme Picker

```
┌──────────────────────────────────────────────────────────────────┐
│  Select Theme: cat                                              │
├──────────────────────────────────────────────────────────────────┤
│  ╭──────────────────────────────────────────────────────────╮   │
│  │  Preview: Catppuccin Mocha                                │   │
│  │  ┌──────────────────────────────────────────────────────┐ │   │
│  │  │  ♪  Sample Text (base text)                         │ │   │
│  │  │  ▶  Active item (mauve)    ← cursor (lavender)     │ │   │
│  │  │  ████████░░░░ progress (green, sky, surface2)       │ │   │
│  │  │  🔀 🔁 (peach, yellow, teal icons)                 │ │   │
│  │  └──────────────────────────────────────────────────────┘ │   │
│  ╰──────────────────────────────────────────────────────────╯   │
│                                                                  │
│  > Catppuccin Mocha                                        Dark │
│  > Catppuccin Latte                                      Light  │
│  > Nord                                                    Dark │
│  > Gruvbox Dark                                           Dark  │
│  > Gruvbox Light                                         Light  │
│  > Tokyo Night                                            Dark  │
│  > Solarized Dark                                        Dark  │
│  > Random (seed: 0x7f3a)                                       │
│                                                                  │
│  Enter to apply, Esc to cancel                                   │
└──────────────────────────────────────────────────────────────────┘
```

### State

```rust
pub struct ThemePickerState {
    pub query: String,                  // filter theme list
    pub cursor: usize,
    pub themes: Vec<ThemeEntry>,
    pub preview_theme: Option<(Theme, ThemeMode)>,
}

pub struct ThemeEntry {
    pub name: &'static str,
    pub mode: ThemeMode,
    pub apply: fn() -> Theme,
}

impl ThemePickerState {
    pub fn new(state: &AppState) -> Self;
    pub fn render(&self, area: Rect, buf: &mut Buffer, state: &AppState);
    pub fn handle_key(&mut self, key: KeyEvent, state: &mut AppState) -> bool;

    fn preview(&mut self, index: usize);
    fn apply_selected(state: &mut AppState);
}
```

### Preview Rendering

```
The preview box shows a miniature sample of the theme:
  Uses theme's actual colors to render:
  - base background
  - text (normal)
  - mauve (accent)
  - lavender (cursor)
  - green (progress fill)
  - surface2 (progress empty)
  - peach, yellow, teal (icons)
  The preview is drawn as a bordered block with sample content.
```

---

## Confirm Dialog

```
┌──────────────────────────────────────────────────────────────────┐
│                                                                  │
│  ╭──────────────────────────────────────────────────────────╮   │
│  │  ⚠  Clear entire queue?                                 │   │
│  │                                                          │   │
│  │        [Y] Yes              [N] No                       │   │
│  ╰──────────────────────────────────────────────────────────╯   │
│                                                                  │
└──────────────────────────────────────────────────────────────────┘
```

### State

```rust
pub struct ConfirmState {
    pub message: String,
    pub details: Option<String>,
    pub on_confirm: fn(&mut AppState),
    pub on_cancel: fn(&mut AppState),
}

impl ConfirmState {
    pub fn new(
        message: String,
        details: Option<String>,
        on_confirm: fn(&mut AppState),
    ) -> Self;
    pub fn render(&self, area: Rect, buf: &mut Buffer, state: &AppState);
    pub fn handle_key(&mut self, key: KeyEvent, state: &mut AppState) -> bool;
}
```

### Key handling

```
Y / y / Enter → call on_confirm, close overlay
N / n / Esc   → call on_cancel, close overlay
```

---

## Track Detail

```
┌──────────────────────────────────────────────────────────────────┐
│  ╭──────────────────────────────────────────────────────────╮   │
│  │  ♪  Song Title                                        ♫  │   │
│  │  ──────────────────────────────────────────────────────  │   │
│  │                                                          │   │
│  │    Artist:    Artist Name                                │   │
│  │    Album:     Album Name (2014)                          │   │
│  │    Duration:  4:20                                       │   │
│  │    Genre:     Jazz / Ambient                             │   │
│  │    Bitrate:   320 kbps                                   │   │
│  │    Sample    44,100 Hz                                   │   │
│  │                                                          │   │
│  │    File:      /home/user/Music/Artist/Album/song.flac    │   │
│  │    Added:     2024-03-15                                 │   │
│  │    Favourite: Yes ☆                                      │   │
│  │                                                          │   │
│  │  ──────────────────────────────────────────────────────  │   │
│  │  [P] Play  [Q] Queue  [F] Favourite  [Esc] Close        │   │
│  ╰──────────────────────────────────────────────────────────╯   │
└──────────────────────────────────────────────────────────────────┘
```

### State

```rust
pub struct TrackDetailState {
    pub track: TrackInfo,
    pub cover: Option<ImageData>,
    pub lyrics: Option<LrcData>,
}

impl TrackDetailState {
    pub fn new(track: TrackInfo, state: &mut AppState) -> Self;
    // Fetches cover + lyrics asynchronously when created

    pub fn render(&self, area: Rect, buf: &mut Buffer, state: &AppState);
    pub fn handle_key(&mut self, key: KeyEvent, state: &mut AppState) -> bool;
}
```

### Key handling

```
P / p → play this track
Q / q → add to queue
F / f → toggle favourite
Esc   → close
```

## File Structure

```
gtm-tui/src/overlays/
├── mod.rs              # Overlay enum + dispatch + centered_rect
├── command_palette.rs  # CommandPaletteState
├── fuzzy_finder.rs     # FuzzyFinderState + fuzzy_score
├── queue_picker.rs     # QueuePickerState
├── theme_picker.rs     # ThemePickerState + ThemeEntry
├── confirm_dialog.rs   # ConfirmState
└── track_detail.rs     # TrackDetailState
```
