# 09 — gtm-tui: Features (Theme, Keybinding, Kitty, Footer, Icons)

## Theme System

HSL-based theme generation (ported from Nim's catppuccin-inspired algorithm).

### Theme Struct

```rust
#[derive(Debug, Clone, Copy)]
pub struct Color {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

impl From<Color> for ratatui::style::Color {
    fn from(c: Color) -> Self {
        ratatui::style::Color::Rgb(c.r, c.g, c.b)
    }
}

#[derive(Debug, Clone)]
pub struct Theme {
    pub rosewater:  Color,
    pub flamingo:   Color,
    pub pink:       Color,
    pub mauve:      Color,
    pub red:        Color,
    pub maroon:     Color,
    pub peach:      Color,
    pub yellow:     Color,
    pub green:      Color,
    pub teal:       Color,
    pub sky:        Color,
    pub sapphire:   Color,
    pub blue:       Color,
    pub lavender:   Color,
    pub text:       Color,
    pub subtext1:   Color,
    pub subtext0:   Color,
    pub overlay2:   Color,
    pub overlay1:   Color,
    pub overlay0:   Color,
    pub surface2:   Color,
    pub surface1:   Color,
    pub surface0:   Color,
    pub base:       Color,
    pub mantle:     Color,
    pub crust:      Color,
}
```

### Presets

```rust
pub struct ThemeEntry {
    pub name: &'static str,
    pub mode: ThemeMode,
    pub apply: fn() -> Theme,
}

pub const THEMES: &[ThemeEntry] = &[
    ThemeEntry { name: "Catppuccin Mocha",   mode: ThemeMode::Dark,  apply: catppuccin_mocha },
    ThemeEntry { name: "Catppuccin Latte",   mode: ThemeMode::Light, apply: catppuccin_latte },
    ThemeEntry { name: "Nord",               mode: ThemeMode::Dark,  apply: nord },
    ThemeEntry { name: "Gruvbox Dark",       mode: ThemeMode::Dark,  apply: gruvbox_dark },
    ThemeEntry { name: "Gruvbox Light",      mode: ThemeMode::Light, apply: gruvbox_light },
    ThemeEntry { name: "Tokyo Night",        mode: ThemeMode::Dark,  apply: tokyo_night },
    ThemeEntry { name: "Solarized Dark",     mode: ThemeMode::Dark,  apply: solarized_dark },
    ThemeEntry { name: "Solarized Light",    mode: ThemeMode::Light, apply: solarized_light },
];

pub fn catppuccin_mocha() -> Theme;
pub fn catppuccin_latte() -> Theme;
pub fn nord() -> Theme;
pub fn gruvbox_dark() -> Theme;
pub fn gruvbox_light() -> Theme;
pub fn tokyo_night() -> Theme;
pub fn solarized_dark() -> Theme;
pub fn solarized_light() -> Theme;
pub fn custom_theme(seed: &str, mode: ThemeMode) -> Theme;  // HSL seed generation
```

### Catppuccin Mocha (Dark, Default)

```rust
pub fn catppuccin_mocha() -> Theme {
    Theme {
        rosewater:  Color { r: 245, g: 224, b: 220 },
        flamingo:   Color { r: 242, g: 205, b: 205 },
        pink:       Color { r: 245, g: 194, b: 231 },
        mauve:      Color { r: 203, g: 166, b: 247 },
        red:        Color { r: 243, g: 139, b: 168 },
        maroon:     Color { r: 235, g: 160, b: 172 },
        peach:      Color { r: 250, g: 179, b: 135 },
        yellow:     Color { r: 249, g: 226, b: 175 },
        green:      Color { r: 166, g: 227, b: 161 },
        teal:       Color { r: 148, g: 226, b: 213 },
        sky:        Color { r: 137, g: 220, b: 235 },
        sapphire:   Color { r: 116, g: 199, b: 236 },
        blue:       Color { r: 137, g: 180, b: 250 },
        lavender:   Color { r: 180, g: 190, b: 254 },
        text:       Color { r: 205, g: 214, b: 244 },
        subtext1:   Color { r: 186, g: 194, b: 222 },
        subtext0:   Color { r: 166, g: 173, b: 200 },
        overlay2:   Color { r: 147, g: 153, b: 178 },
        overlay1:   Color { r: 127, g: 132, b: 156 },
        overlay0:   Color { r: 108, g: 112, b: 134 },
        surface2:   Color { r: 88, g: 91, b: 112 },
        surface1:   Color { r: 69, g: 71, b: 90 },
        surface0:   Color { r: 49, g: 50, b: 68 },
        base:       Color { r: 30, g: 30, b: 46 },
        mantle:     Color { r: 24, g: 24, b: 37 },
        crust:      Color { r: 17, g: 17, b: 27 },
    }
}
```

### Catppuccin Latte (Light)

```rust
pub fn catppuccin_latte() -> Theme {
    Theme {
        rosewater:  Color { r: 220, g: 138, b: 120 },
        flamingo:   Color { r: 221, g: 120, b: 120 },
        pink:       Color { r: 234, g: 118, b: 203 },
        mauve:      Color { r: 136, g: 57, b: 239 },
        red:        Color { r: 210, g: 15, b: 57 },
        maroon:     Color { r: 230, g: 69, b: 83 },
        peach:      Color { r: 254, g: 100, b: 11 },
        yellow:     Color { r: 223, g: 142, b: 29 },
        green:      Color { r: 64, g: 160, b: 43 },
        teal:       Color { r: 23, g: 146, b: 153 },
        sky:        Color { r: 4, g: 165, b: 229 },
        sapphire:   Color { r: 32, g: 159, b: 181 },
        blue:       Color { r: 30, g: 102, b: 245 },
        lavender:   Color { r: 114, g: 135, b: 253 },
        text:       Color { r: 76, g: 79, b: 105 },
        subtext1:   Color { r: 92, g: 95, b: 119 },
        subtext0:   Color { r: 108, g: 111, b: 133 },
        overlay2:   Color { r: 124, g: 127, b: 147 },
        overlay1:   Color { r: 140, g: 143, b: 161 },
        overlay0:   Color { r: 156, g: 160, b: 176 },
        surface2:   Color { r: 172, g: 176, b: 190 },
        surface1:   Color { r: 188, g: 192, b: 204 },
        surface0:   Color { r: 204, g: 208, b: 218 },
        base:       Color { r: 239, g: 241, b: 245 },
        mantle:     Color { r: 230, g: 233, b: 239 },
        crust:      Color { r: 220, g: 224, b: 232 },
    }
}
```

### HSL→RGB

```rust
/// Convert HSL (h: 0-360, s: 0-100, l: 0-100) to RGB (0-255)
pub fn hsl_to_rgb(h: f64, s: f64, l: f64) -> Color {
    let s = s / 100.0;
    let l = l / 100.0;
    let c = (1.0 - (2.0 * l - 1.0).abs()) * s;  // chroma
    let x = c * (1.0 - ((h / 60.0) % 2.0 - 1.0).abs());
    let m = l - c / 2.0;

    let (r, g, b) = match h as u32 % 360 {
        0..=59   => (c, x, 0.0),
        60..=119  => (x, c, 0.0),
        120..=179 => (0.0, c, x),
        180..=239 => (0.0, x, c),
        240..=299 => (x, 0.0, c),
        _         => (c, 0.0, x),
    };

    Color {
        r: ((r + m) * 255.0).round() as u8,
        g: ((g + m) * 255.0).round() as u8,
        b: ((b + m) * 255.0).round() as u8,
    }
}
```

### Custom Theme Generation (HSL seed)

```rust
/// Generate a deterministic theme from a seed string.
/// Hash the seed → base_hue (0-360).
/// Use relative HSL offsets defined per color to build full palette.
pub fn custom_theme(seed: &str, mode: ThemeMode) -> Theme {
    let base_hue = hash_to_hue(seed);
    // For each color, compute: hue = base_hue + offset, sat, light from table
    // Dark/light variants determined by lightness offsets
    // Same algorithm as catppuccin but with dynamic base hue
}
```

## Keybinding System

```rust
#[derive(Debug, Clone)]
pub struct Keybindings {
    pub bindings: Vec<(KeyEvent, BoundCommand)>,
}

#[derive(Debug, Clone)]
pub struct BoundCommand {
    pub action: KeyboardAction,
    pub contexts: Vec<KeyContext>,
    pub description: &'static str,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyContext {
    Global,
    Normal,
    Filter,
    Overlay,
    List,
    MoveMode,
}

#[derive(Debug, Clone)]
pub enum KeyboardAction {
    // Tab switching
    NextTab,
    PrevTab,
    SwitchTab(Tab),

    // Cursor
    MoveUp,
    MoveDown,
    PageUp,
    PageDown,
    Top,
    Bottom,

    // Selection / action
    Select,
    EnqueueNext,
    EnqueueEnd,
    Delete,
    Move,

    // Filter
    EnterFilter,
    EnterCommand,
    ClearFilter,
    Confirm,

    // Playback
    PlayPause,
    Next,
    Prev,
    VolumeUp,
    VolumeDown,
    SeekForward,
    SeekBackward,
    ToggleShuffle,
    CycleRepeat,
    ToggleMute,

    // Navigation
    Back,

    // Meta
    Quit,
    ReloadConfig,
    ToggleHelp,
    Custom(String),
}

/// Parse a key name string like "ctrl-c", "enter", "space", "f1", "char:a" into KeyEvent.
pub fn parse_keycode(name: &str) -> KeyEvent;
```

### Default Keybindings

```rust
pub fn default_keybindings() -> Keybindings {
    Keybindings {
        bindings: vec![
            // Global
            (KeyCode::Char('q').into(),        BoundCommand { action: Quit,        contexts: vec![Global, Normal], .. }),
            (KeyCode::Char('?').into(),        BoundCommand { action: ToggleHelp,  contexts: vec![Global, Normal], .. }),
            (KeyCode::Char(':').into(),        BoundCommand { action: EnterCommand,contexts: vec![Normal], .. }),
            (KeyCode::Tab.into(),              BoundCommand { action: NextTab,     contexts: vec![Normal], .. }),
            (KeyCode::BackTab.into(),          BoundCommand { action: PrevTab,     contexts: vec![Normal], .. }),

            // Cursor (list context)
            (KeyCode::Up.into(),               BoundCommand { action: MoveUp,   contexts: vec![List, Normal], .. }),
            (KeyCode::Down.into(),             BoundCommand { action: MoveDown, contexts: vec![List, Normal], .. }),
            (KeyCode::Char('k').into(),        BoundCommand { action: MoveUp,   contexts: vec![List, Normal], .. }),
            (KeyCode::Char('j').into(),        BoundCommand { action: MoveDown, contexts: vec![List, Normal], .. }),

            // Playback
            (KeyCode::Char(' ').into(),        BoundCommand { action: PlayPause, contexts: vec![Global, Normal], .. }),
            (KeyCode::Char('n').into(),        BoundCommand { action: Next,      contexts: vec![Global, Normal], .. }),
            (KeyCode::Char('p').into(),        BoundCommand { action: Prev,      contexts: vec![Global, Normal], .. }),

            // ... more bindings
        ],
    }
}
```

### Key Dispatch

```
In the TUI event loop:
  1. If overlay is active: overlay.handle_key(key, state)
  2. Else if mode is Filter/Command: handle filter input
  3. Else: active_tab.handle_key(key, state)
  4. If no tab handled it: check global keybindings
```

## Kitty Graphics Protocol

### KittyGraphics struct

```rust
pub struct KittyGraphics {
    pub supported: bool,
    next_image_id: u32,
}

impl KittyGraphics {
    /// Probe terminal for Kitty graphics support.
    /// Sends escape sequence and checks response.
    /// Sets supported = true/false.
    pub fn probe() -> Self;

    /// Transmit an image to the terminal.
    /// Compresses PNG if JPEG (Kitty spec prefers PNG).
    /// Formats: f=24 (RGB), f=32 (RGBA), or f=100 (PNG).
    pub fn transmit_image(&mut self, data: &[u8], mime: &str) -> Option<u32>;

    /// Place a previously transmitted image at character coordinates.
    /// x, y: character cell position
    /// width, height: character cell dimensions
    pub fn place_image(&self, id: u32, x: u16, y: u16, width: u16, height: u16);

    /// Delete a transmitted image from terminal memory.
    pub fn delete_image(&self, id: u32);

    /// Delete all transmitted images.
    pub fn delete_all(&self);
}
```

### Escape Sequence Format

```
Transmit:   ESC_Gi=<id>,f=<fmt>,s=<width>,v=<height>,m=1;<base64>ESC\
Place:      ESC_Gi=<id>,a=p,c=<col>,r=<row>,C=1;ESC\
Delete:     ESC_Gi=<id>,a=d;ESC\
DeleteAll:  ESC_Gi=a=d;ESC\

Where:
  <id> = image number (u32)
  <fmt> = 24 (RGB raw), 32 (RGBA raw), 100 (PNG)
  <width>, <height> = pixel dimensions
  <col>, <row> = character cell coordinates (1-indexed)
  C=1 = place at cursor (relative)
  m=1 = more chunks follow (last chunk has m=0)
```

### Cover Image Flow

```
1. PlaybackStarted event received
2. Fetch album art:
   a. Check app state for embedded cover data in MetadataChanged event
   b. If not: request cover from daemon via GetStatus → CoverData in response
3. Decode image using `image` crate → get pixel dimensions
4. If Kitty supported:
   a. Delete previous image (if any)
   b. transmit_image(bytes, mime)
   c. place_image(id, x, y, width, height)
5. If Kitty not supported: render solid color block as placeholder
6. On resize: re-place image at new coordinates
7. On track change / stop: delete image, show placeholder
```

## Footer Modules

```
┌──────────────────────────────────────────────────────────────┐
│  ⏸  1:23 / 4:20  ████████░░░░  28%  │  Vol: 75%  🔀  🔁 All │
│  └──────── left section ─────────┘    └──── right section ───┘│
└──────────────────────────────────────────────────────────────┘
```

### FooterModule enum

```rust
pub enum FooterModule {
    // Left-aligned
    PlaybackStatus,       // ▶ ⏸ ⏹ (from IconSet)
    TimePosition,         // "1:23 / 4:20"
    ProgressBar,          // ████████░░░░ (10 chars wide)
    TimePercent,          // "28%"

    // Right-aligned
    Volume,               // "Vol: 75%"
    ShuffleIndicator,     // 🔀 (hidden when off)
    RepeatIndicator,      // 🔁 All / 🔂 One (hidden when Off)
    SleepTimer,           // ⏰ 5:00 (hidden when no timer)
    Date,                 // "2024-03-15"
    Time,                 // "14:30"
    QueueLength,          // "Q: 5"

    /// Custom module with label + dynamic value function
    Custom {
        label: String,
        value: fn(&AppState) -> String,
    },
}

pub struct FooterBar {
    pub left_modules: Vec<(FooterModule, Constraint)>,
    pub right_modules: Vec<(FooterModule, Constraint)>,
}

impl FooterBar {
    pub fn new() -> Self {
        FooterBar {
            left_modules: vec![
                (FooterModule::PlaybackStatus, Constraint::Length(2)),
                (FooterModule::TimePosition,   Constraint::Length(14)),
                (FooterModule::ProgressBar,    Constraint::Length(12)),
                (FooterModule::TimePercent,    Constraint::Length(5)),
            ],
            right_modules: vec![
                (FooterModule::Volume,          Constraint::Length(10)),
                (FooterModule::ShuffleIndicator, Constraint::Length(2)),
                (FooterModule::RepeatIndicator,  Constraint::Length(6)),
                (FooterModule::Time,            Constraint::Length(6)),
            ],
        }
    }

    pub fn render(&self, area: Rect, buf: &mut Buffer, state: &AppState);
}
```

### Progress Bar Rendering

```
Progress bar: 10 character cells
  filled_chars = (10 * position / duration).round()
  ████████░░░░ (8 filled, 2 empty)

Color: green (filled), surface2 (empty background)
```

### Time Formatting

```
fn format_time(secs: f64) -> String {
    let total = secs as u64;
    let h = total / 3600;
    let m = (total % 3600) / 60;
    let s = total % 60;
    if h > 0 {
        format!("{}:{:02}:{:02}", h, m, s)
    } else {
        format!("{}:{:02}", m, s)
    }
}
```

## Icons

### IconSet

```rust
pub struct IconSet {
    pub play: &'static str,
    pub pause: &'static str,
    pub stop: &'static str,
    pub next: &'static str,
    pub prev: &'static str,
    pub shuffle: &'static str,
    pub repeat_all: &'static str,
    pub repeat_one: &'static str,
    pub volume_high: &'static str,
    pub volume_low: &'static str,
    pub volume_mute: &'static str,
    pub heart: &'static str,
    pub heart_empty: &'static str,
    pub search: &'static str,
    pub music: &'static str,
    pub album: &'static str,
    pub artist: &'static str,
    pub clock: &'static str,
    pub list: &'static str,
    pub settings: &'static str,
    pub help: &'static str,
    pub youtube: &'static str,
    pub cursor: &'static str,
    pub cursor_sel: &'static str,
}
```

### Nerd Font Variant (default)

```rust
pub const NERD_FONT: IconSet = IconSet {
    play:        "\u{23F5}",      // ▶
    pause:       "\u{23F8}",      // ⏸
    stop:        "\u{23F9}",      // ⏹
    next:        "\u{23ED}",      // ⏭
    prev:        "\u{23EE}",      // ⏮
    shuffle:     "\u{1F500}",     // 🔀
    repeat_all:  "\u{1F501}",     // 🔁
    repeat_one:  "\u{1F502}",     // 🔂
    volume_high: "\u{1F50A}",     // 🔊
    volume_low:  "\u{1F509}",     // 🔉
    volume_mute: "\u{1F507}",     // 🔇
    heart:       "\u{2665}",      // ♥
    heart_empty: "\u{2661}",      // ♡
    search:      "\u{1F50D}",     // 🔍
    music:       "\u{266B}",      // ♫
    album:       "\u{1F4BF}",     // 💿
    artist:      "\u{1F3A4}",     // 🎤
    clock:       "\u{1F552}",     // 🕒
    list:        "\u{2630}",      // ☰
    settings:    "\u{2699}",      // ⚙
    help:        "\u{2753}",      // ❓
    youtube:     "\u{25B6}",      // ▶
    cursor:      " \u{2190}",     // ←
    cursor_sel:  " \u{25B6}",     // ▶
};
```

### Emoji Variant (fallback)

```rust
pub const EMOJI: IconSet = IconSet {
    play:        "▶️",
    pause:       "⏸️",
    stop:        "⏹️",
    next:        "⏭️",
    prev:        "⏮️",
    shuffle:     "🔀",
    repeat_all:  "🔁",
    repeat_one:  "🔂",
    volume_high: "🔊",
    volume_low:  "🔉",
    volume_mute: "🔇",
    heart:       "❤️",
    heart_empty: "🤍",
    search:      "🔍",
    music:       "🎵",
    album:       "💿",
    artist:      "🎤",
    clock:       "🕐",
    list:        "📋",
    settings:    "⚙️",
    help:        "❓",
    youtube:     "▶️",
    cursor:      " ◀️",
    cursor_sel:  " ▶️",
};
```

### Icon Detection

```
Icon set selection at startup:

  1. Check if a Nerd Font is installed:
     - Read TERM_PROGRAM env var
     - Terminals known to support Nerd Fonts:
       "alacritty", "kitty", "wezterm", "foot", "st", "urxvt",
       "gnome-terminal", "konsole", "xterm-256color" (with Nerd Font patched)
     - Check if locale supports Unicode (LC_ALL, LANG contain "UTF-8" or "utf8")
     - Also check STY (if running inside tmux)

  2. If Nerd Font likely supported: use NERD_FONT
  3. Else: use EMOJI

  4. User can override with --icons flag:
     gtm --icons emoji
     gtm --icons nerd
```

## File Structure

```
gtm-tui/src/
├── keymap.rs       # Keybindings, parse_keycode, KeyContext, KeyboardAction
├── theme.rs        # Theme struct, presets, hsl_to_rgb, custom generation
├── graphics.rs     # KittyGraphics (probe, transmit, delete, place)
├── icons.rs        # IconSet, NERD_FONT, EMOJI constants
└── footer.rs       # FooterBar, FooterModule enum, rendering
```
