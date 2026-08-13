# 08 — Miscellaneous Improvements

## Goal

Various UI/UX improvements including settings keybinding, help row toggle, version display, About overlay fixes, layout adjustments, and time/date positioning.

## Current State

- Settings accessed via '2' keybinding
- Help row always visible
- Version displayed in UI
- About overlay shows incorrect copyright
- Now Playing tab takes too much space
- Time/date module not positioned correctly

## Required Changes

### 1. Settings Keybinding Change

**Current:** Settings accessed via '2' keybinding
**New:** Settings accessed via `Ctrl+,` (standard settings shortcut)

#### Remove Number Navigation

Remove all number keybindings for tab switching:
- Remove `1` → NowPlaying
- Remove `2` → Library
- Remove `3` → Settings

Keep only:
- `Tab` → Next tab
- `Shift+Tab` → Previous tab
- `[` → Previous tab
- `]` → Next tab

```rust
// In gtm/src/keymap.rs
impl Keybindings {
    pub fn default() -> Self {
        let mut bindings = Vec::new();
        
        // Remove number navigation
        // bindings.push(Keybinding::new(KeyCode::Char('1'), KeyboardAction::SwitchTab(Tab::NowPlaying)));
        // bindings.push(Keybinding::new(KeyCode::Char('2'), KeyboardAction::SwitchTab(Tab::Library)));
        // bindings.push(Keybinding::new(KeyCode::Char('3'), KeyboardAction::SwitchTab(Tab::Settings)));
        
        // Add Ctrl+, for settings
        bindings.push(Keybinding::new(
            KeyEvent::new(KeyCode::Char(','), KeyModifiers::CONTROL),
            KeyboardAction::OpenSettings,
        ));
        
        // Keep tab cycling
        bindings.push(Keybinding::new(KeyCode::Tab, KeyboardAction::NextTab));
        bindings.push(Keybinding::new(
            KeyEvent::new(KeyCode::BackTab, KeyModifiers::SHIFT),
            KeyboardAction::PrevTab,
        ));
        bindings.push(Keybinding::new(KeyCode::Char('['), KeyboardAction::PrevTab));
        bindings.push(Keybinding::new(KeyCode::Char(']'), KeyboardAction::NextTab));
        
        Self { bindings }
    }
}
```

### 2. Help Row Toggle

**Current:** Help row always visible
**New:** Add option to hide help row

#### Settings Option

```
┌─ SETTINGS ──────────────────────────────────────┐
│  ♫ Audio       Show Help Row     [ ● ] On       │
│  ▶ Appearance  ...                               │
└─────────────────────────────────────────────────┘
```

#### Implementation

```rust
// In gtm/src/app.rs
impl App {
    pub fn toggle_help_row(&mut self) {
        self.show_help_row = !self.show_help_row;
    }
}

// In gtm/src/ui.rs
fn render_help_row(f: &mut Frame, area: Rect, show: bool) {
    if !show {
        return;
    }
    
    let help_text = "Space: Play/Pause  n/p: Next/Prev  q: Quit  ?: Help";
    let help = Paragraph::new(help_text)
        .style(Style::default().fg(Color::DarkGray));
    
    f.render_widget(help, area);
}
```

#### Config File

```toml
[appearance]
show_help_row = true
```

### 3. Remove Version from UI

**Current:** Version displayed in tab bar
**New:** Remove 'gtm [version]' from UI

```rust
// In gtm/src/ui.rs
fn render_tab_bar(f: &mut Frame, area: Rect, state: &AppState) {
    // Before
    // let title = format!("{} v{}", env!("CARGO_PKG_NAME"), env!("CARGO_PKG_VERSION"));
    
    // After
    let tabs = vec![
        Tab::NowPlaying,
        Tab::Library,
        Tab::Settings,
    ];
    
    let tab_titles = tabs.iter().map(|t| {
        Line::from(Span::styled(
            t.title(),
            Style::default().fg(Color::White),
        ))
    });
    
    let tabs_widget = Tabs::new(tab_titles)
        .block(Block::default().borders(Borders::BOTTOM))
        .select(state.current_tab.index())
        .style(Style::default().fg(Color::White))
        .highlight_style(Style::default().fg(Color::Green).add_modifier(Modifier::BOLD));
    
    f.render_widget(tabs_widget, area);
}
```

### 4. Fix About Overlay

**Current:** Shows incorrect copyright
**New:** Show proper copyright and library versions

#### Expected Output

```
┌─────────────────────────────────────────────┐
│                 About gtm                     │
├─────────────────────────────────────────────┤
│                                               │
│  gtm v0.1.0                                   │
│  Terminal Music Player                        │
│                                               │
│  Copyright (c) 2024 gtm-rs contributors      │
│  Licensed under GPL-3.0                       │
│                                               │
│  Dependencies:                                │
│  ├── ratatui: 0.28.0                          │
│  ├── symphonia: 0.6.0                         │
│  ├── rodio: 0.19.0                            │
│  └── crossterm: 0.28.0                        │
│                                               │
│  [GitHub]  [Report Issue]                     │
│                                               │
└─────────────────────────────────────────────┘
```

#### Implementation

```rust
// In gtm/src/overlay.rs
impl AboutOverlay {
    pub fn render(&self, f: &mut Frame, area: Rect) {
        let content = vec![
            Line::from(""),
            Line::from(Span::styled(
                format!("gtm v{}", env!("CARGO_PKG_VERSION")),
                Style::default().fg(Color::Green).add_modifier(Modifier::BOLD),
            )),
            Line::from("Terminal Music Player"),
            Line::from(""),
            Line::from(Span::styled(
                "Copyright (c) 2024 gtm-rs contributors",
                Style::default().fg(Color::White),
            )),
            Line::from("Licensed under GPL-3.0"),
            Line::from(""),
            Line::from(Span::styled(
                "Dependencies:",
                Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD),
            )),
            Line::from(format!("├── ratatui: {}", env!("RATATUI_VERSION"))),
            Line::from(format!("├── symphonia: {}", env!("SYMPHONIA_VERSION"))),
            Line::from(format!("├── rodio: {}", env!("RODIO_VERSION"))),
            Line::from(format!("└── crossterm: {}", env!("CROSSTERM_VERSION"))),
            Line::from(""),
            Line::from(vec![
                Span::styled("[GitHub] ", Style::default().fg(Color::Blue)),
                Span::styled("[Report Issue]", Style::default().fg(Color::Blue)),
            ]),
        ];
        
        let paragraph = Paragraph::new(content)
            .alignment(Alignment::Center);
        
        let block = Block::default()
            .title(" About gtm ")
            .borders(Borders::ALL)
            .border_type(BorderType::Plain);
        
        f.render_widget(paragraph.block(block), area);
    }
}
```

#### Build Script

Create `gtm/build.rs` to capture dependency versions:

```rust
fn main() {
    // Capture versions at build time
    println!("cargo:rustc-env=RATATUI_VERSION={}", get_version("ratatui"));
    println!("cargo:rustc-env=SYMPHONIA_VERSION={}", get_version("symphonia"));
    println!("cargo:rustc-env=RODIO_VERSION={}", get_version("rodio"));
    println!("cargo:rustc-env=CROSSTERM_VERSION={}", get_version("crossterm"));
}

fn get_version(package: &str) -> String {
    // Parse Cargo.lock to get version
    // Or use include_str! to embed version info
    "0.0.0".to_string()
}
```

### 5. Remove One Row from Now Playing

**Current:** Now Playing takes too much vertical space
**New:** Remove one row from Now Playing, give to library panes

#### Before

```
┌─ NOW PLAYING ──────────────────────────────────────────┐
│  ┌──┐  NOW PLAYING                                     │  Line 1
│  │▀▀│  Codeine Crazy (Official Audio)                   │  Line 2
│  │▀▀│  Artist: Future                                   │  Line 3
│  │▀▀│  Format: [FLAC | 24-bit/96kHz]                   │  Line 4
│  └──┘                                                   │  Line 5
│      00:45                          5:52                │  Line 6
│      ════════════════════════                          │  Line 7
└─────────────────────────────────────────────────────────┘
```

#### After

```
┌─ NOW PLAYING ──────────────────────────────────────────┐
│  ┌──┐  NOW PLAYING                                     │  Line 1
│  │▀▀│  Codeine Crazy (Official Audio)                   │  Line 2
│  │▀▀│  Artist: Future                                   │  Line 3
│  └──┘                                                   │  Line 4
│      00:45                          5:52                │  Line 5
│      ════════════════════════                          │  Line 6
└─────────────────────────────────────────────────────────┘
```

Remove "Format" line to save one row.

```rust
// In gtm/src/tabs/now_playing.rs
fn render_now_playing(f: &mut Frame, area: Rect, state: &AppState) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),  // Title
            Constraint::Length(1),  // Artist
            Constraint::Length(1),  // Format (REMOVED)
            Constraint::Length(1),  // Time/Progress
            Constraint::Min(0),    // Visualizer/Additional info
        ])
        .split(area);
    
    // Line 1: Title
    f.render_widget(
        Paragraph::new(Span::styled(
            &state.current_track.title,
            Style::default().fg(Color::White),
        )),
        chunks[0],
    );
    
    // Line 2: Artist
    f.render_widget(
        Paragraph::new(Span::styled(
            &state.current_track.artist,
            Style::default().fg(Color::DarkGray),
        )),
        chunks[1],
    );
    
    // Line 3: Time/Progress (was chunks[3], now chunks[2])
    let time_text = format!("{:02}:{:02}  {:02}:{:02}",
        state.time_pos as u32 / 60,
        state.time_pos as u32 % 60,
        state.duration as u32 / 60,
        state.duration as u32 % 60,
    );
    f.render_widget(
        Paragraph::new(time_text),
        chunks[2],
    );
}
```

### 6. Fix Time/Date Position

**Current:** Time/date module not positioned correctly
**New:** Make time/date module absolute and place at bottom right

```rust
// In gtm/src/footer.rs
fn render_footer(f: &mut Frame, area: Rect, state: &AppState) {
    let footer_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Min(0),      // Left modules
            Constraint::Length(20),  // Time/date (right-aligned)
        ])
        .split(area);
    
    // Left side: playback info, volume, queue
    let left_modules = vec![
        Span::styled(
            format!("{} ", state.playback_status_icon()),
            Style::default().fg(Color::Green),
        ),
        Span::styled(
            format!("{} ", state.current_track_display()),
            Style::default().fg(Color::White),
        ),
        Span::styled(
            format!("{} ", state.volume_display()),
            Style::default().fg(Color::DarkGray),
        ),
        Span::styled(
            format!("{} ", state.queue_display()),
            Style::default().fg(Color::DarkGray),
        ),
    ];
    
    f.render_widget(
        Paragraph::new(Line::from(left_modules)),
        footer_chunks[0],
    );
    
    // Right side: time/date (absolute position at bottom right)
    let time_date = format!("{}", state.current_time_display());
    f.render_widget(
        Paragraph::new(Span::styled(
            time_date,
            Style::default().fg(Color::DarkGray),
        ))
        .alignment(Alignment::Right),
        footer_chunks[1],
    );
}
```

```rust
impl AppState {
    pub fn current_time_display(&self) -> String {
        let now = chrono::Local::now();
        now.format("%H:%M %Z").to_string()
    }
}
```

## Files to Modify

- `gtm/src/keymap.rs` — Remove number navigation, add Ctrl+, for settings
- `gtm/src/app.rs` — Add show_help_row state, toggle_help_row method
- `gtm/src/ui.rs` — Remove version from tab bar, conditionally render help row
- `gtm/src/overlay.rs` — Fix About overlay with correct copyright and versions
- `gtm/src/tabs/now_playing.rs` — Remove format line
- `gtm/src/footer.rs` — Fix time/date positioning
- `gtm/src/settings.rs` — Add show_help_row setting
- `gtm/build.rs` — New file: capture dependency versions
- `~/.config/gtom/config.toml` — Add show_help_row config option

## Implementation Details

### Settings Access

```rust
// In gtm/src/keymap.rs
KeyboardAction::OpenSettings,

// In gtm/src/app.rs
fn handle_key(&mut self, key: KeyEvent) -> bool {
    match key.code {
        KeyCode::Char(',') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            self.state.current_tab = Tab::Settings;
            true
        }
        // ... other keys
    }
}
```

### Help Row State

```rust
// In gtm/src/app.rs
pub struct App {
    // ... existing fields
    pub show_help_row: bool,
}

impl App {
    pub fn new() -> Self {
        Self {
            // ...
            show_help_row: true,
        }
    }
    
    pub fn toggle_help_row(&mut self) {
        self.show_help_row = !self.show_help_row;
    }
}
```

### Time/Date Absolute Positioning

```rust
// In gtm/src/footer.rs
fn render_footer(f: &mut Frame, area: Rect, state: &AppState) {
    // Time/date is always at bottom right, regardless of other modules
    let time_area = Rect {
        x: area.right().saturating_sub(20),
        y: area.y,
        width: 20,
        height: area.height,
    };
    
    let time_text = format!("{:02}:{:02} {}", 
        state.local_time.hour(),
        state.local_time.minute(),
        state.local_time.format("%Z"),
    );
    
    f.render_widget(
        Paragraph::new(Span::styled(
            time_text,
            Style::default().fg(Color::DarkGray),
        ))
        .alignment(Alignment::Right),
        time_area,
    );
}
```

## Checklist

- [ ] Settings keybinding changed to `Ctrl+,`
- [ ] Number navigation (1, 2, 3) removed
- [ ] Help row toggle added
- [ ] Settings option for show_help_row added
- [ ] Config file option added
- [ ] Version removed from UI
- [ ] About overlay shows correct copyright
- [ ] About overlay shows dependency versions
- [ ] Now Playing format line removed (one row saved)
- [ ] Time/date module positioned at bottom right
- [ ] Time/date uses absolute positioning
- [ ] `cargo check --workspace` passes
- [ ] `cargo test --workspace` passes

## Visual Design

### Updated Tab Bar (No Version)
```
┌──────────────────────────────────────────────────────────────┐
│ [Now Playing]  [Library]  [Settings]                          │
├──────────────────────────────────────────────────────────────┤
```

### Updated Footer (Time at Bottom Right)
```
┌──────────────────────────────────────────────────────────────┐
│ [▶] Future - Codeine Crazy [80%] [Q:12]           14:30 UTC │
└──────────────────────────────────────────────────────────────┘
```

### Updated Now Playing (No Format Line)
```
┌─ NOW PLAYING ──────────────────────────────────────────┐
│  ┌──┐  NOW PLAYING                                     │
│  │▀▀│  Codeine Crazy (Official Audio)                   │
│  │▀▀│  Artist: Future                                   │
│  └──┘                                                   │
│      00:45                          5:52                │
│      ════════════════════════                          │
└─────────────────────────────────────────────────────────┘
```

### About Overlay
```
┌─────────────────────────────────────────────┐
│                 About gtm                     │
├─────────────────────────────────────────────┤
│                                               │
│  gtm v0.1.0                                   │
│  Terminal Music Player                        │
│                                               │
│  Copyright (c) 2024 gtm-rs contributors      │
│  Licensed under GPL-3.0                       │
│                                               │
│  Dependencies:                                │
│  ├── ratatui: 0.28.0                          │
│  ├── symphonia: 0.6.0                         │
│  ├── rodio: 0.19.0                            │
│  └── crossterm: 0.28.0                        │
│                                               │
│  [GitHub]  [Report Issue]                     │
│                                               │
└─────────────────────────────────────────────┘
```