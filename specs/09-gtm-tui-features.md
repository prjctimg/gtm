# 09 — TUI Features

## Aesthetics

### Borders
- Rounded borders (`BorderType::Rounded`) for all panels and overlays
- Flat borders (`BorderType::Plain`) for the tab bar

### Loading Spinners
- Braille characters (`⠋ ⠙ ⠹ ⠸ ⠼ ⠴ ⠦ ⠧ ⠇ ⠏`) for loading states
- Single-char spinners in status bar, multi-char in overlay footers

### Icons
- Nerd icons via `nerd-finder` or hardcoded mapping
- Emoji fallback when nerd font not detected
- Icon set:
  - Music: `\u{F025}` (nerd) / `🎵` (emoji)
  - Play: `\u{F04B}` / `▶`
  - Pause: `\u{F04C}` / `⏸`
  - Next: `\u{F050}` / `⏭`
  - Volume: `\u{F028}` / `🔊`
  - Search: `\u{F002}` / `🔍`
  - Queue: `\u{f0c9}` / `📋`
  - Settings: `\u{f013}` / `⚙`

### Progress Bar
- Line style with oscillating head (material design inspired)
- Characters: `═` (track), `●` (head), `─` (remaining)
- Head oscillates when playing (sine wave amplitude 1 char)

```
════════════●═══════════════════════════════════
```

### Semi-Transparent Overlays
- Default opacity: 90%
- Uses `ratatui::widgets::Clear` + `set_style` with background color at reduced alpha
- Configurable per-overlay via settings

### Terminal Background
- Custom background color by default
- "Transparent" mode: don't draw background, inherit terminal color
- Opacity and blend controls for custom backgrounds

## Notifications

### Up Next Notification
- Shows when crossfade starts (next track fading in)
- Appears from top-right, elastic bounce-out animation
- Content: "Up Next: Artist — Title"
- Duration: ~3s visible
- Uses easing for smooth animation

### Volume Toast
- Shows briefly when volume changes
- Top-right position
- Color-coded by level:
  - Green: 1-50% (quiet)
  - Yellow: 51-85% (medium)
  - Red: 86-100% (loud)
- Auto-dismisses after 1.5s

## Customizable Footer

### Footer Modules
Each module shows specific information with its own styled background:

| Module | Content | Default Preset |
|--------|---------|---------------|
| Playback | Status icon + "Artist - Title" | Always on |
| Volume | Vol icon + percentage | Always on |
| Queue | "Q: 12 tracks" | Always on |
| Backend | Audio backend name | Optional |
| System | CPU/Mem usage | Optional |
| Clock | Current time | Optional |
| KeyHint | Last pressed keybinding action | On during overlay |
| Device | Audio output device | Optional |

### Footer Presets
Users can define presets (ordered list of modules):

```
Preset "Default":  [▶ Playing] [Vol: 80%] [Q: 12]
Preset "Minimal":  [▶ Song Title]
Preset "Full":     [▶ Song] [80%] [Q:12] [rodio] [CPU:5%] [14:30]
```

Modules with related context can share a background by being in the same "group."

## Keybinding System

```rust
pub struct Keybinding {
    pub key: KeyEvent,
    pub action: KeyboardAction,
    pub context: KeyContext,
    pub description: &'static str,
}

pub enum KeyContext {
    Normal,
    Searching,
    Command,
    Overlay(&'static str),  // per-overlay keymaps
}

pub enum KeyboardAction {
    // Navigation
    NextTab, PrevTab, SwitchTab(Tab),
    MoveUp, MoveDown, MoveLeft, MoveRight,
    ScrollUp, ScrollDown,
    Select, Delete,
    
    // Playback
    PlayPause, Next, Prev, Stop,
    VolumeUp, VolumeDown, ToggleMute,
    ToggleShuffle, CycleRepeat,
    SeekForward, SeekBackward,
    
    // Modes
    EnterFilter, EnterCommand, Quit,
    
    // Overlays (open/close each)
    OpenQueue, OpenYTSearch, OpenSearchLibrary,
    OpenSpotifySearch, OpenEqualizer,
    OpenCommandPalette, OpenAbout,
    OpenSleepTimer, OpenThemePicker,
    
    // Overlay actions
    Download, AddToQueue, PlayNext,
    Confirm, Cancel,
}
```

All keybindings are user-configurable in a keybindings config file.

## Nerd Font Detection

Check if the terminal supports nerd fonts:
1. Try to write a nerd font glyph
2. Read cursor position response
3. If the width changed, nerd font is supported
4. Fall back to emoji alternatives

Detection runs once at startup, caches result.
