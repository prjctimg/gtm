# 02 — Responsive TUI

## Goal

Make the TUI adapt gracefully to different terminal widths, especially very narrow terminals. Implement single-pane mode with tab navigation and adaptive footer.

## Current State

When the terminal is resized to less than ~30 columns, the TUI crashes. There is no responsive behavior for narrow terminals.

## Required Changes

### 1. Define Terminal Width Thresholds

```rust
pub enum TerminalWidth {
    Narrow,    // < 40 columns
    Medium,    // 40-80 columns
    Wide,      // > 80 columns
}

impl TerminalWidth {
    pub fn from_cols(cols: u16) -> Self {
        match cols {
            0..=39 => Self::Narrow,
            40..=80 => Self::Medium,
            _ => Self::Wide,
        }
    }
}
```

### 2. Narrow Terminal Mode (< 40 columns)

When terminal is narrow:
- **Single Pane**: Only show one content pane at a time
- **Tab Navigation**: Show tab bar at top for switching views
- **No Separate Lyrics Pane**: Lyrics replace main content instead of being a separate pane
- **Help Text**: Show "Press Esc to restore Library" in footer when lyrics are shown

#### Layout for Narrow Mode:
```
┌─────────────────────────┐
│ [NP] [Lib] [Set]  Tab  │  Tab bar (clickable/keyboard)
├─────────────────────────┤
│                          │
│  Content Area            │  Single pane
│  (NowPlaying OR          │
│   Library OR             │
│   Settings OR            │
│   Lyrics)                │
│                          │
├─────────────────────────┤
│ [▶ Song] [80%]          │  Simplified footer
│ [Esc: Library]          │  Help text when in lyrics
└─────────────────────────┘
```

### 3. Medium Terminal Mode (40-80 columns)

- Show two panes side by side when possible
- Now Playing + Library (default)
- Lyrics can be a third pane or overlay

### 4. Wide Terminal Mode (> 80 columns)

- Full 3-pane layout as currently designed
- All features available

### 5. Lyrics Pane Behavior in Narrow Mode

When lyrics are triggered from the left pane:
- Replace the main content area with lyrics
- Show help text: "Press Esc to restore Library"
- Esc returns to the previous view (Library)

```rust
pub enum ContentView {
    NowPlaying,
    Library,
    Settings,
    Lyrics,  // Replaces main content in narrow mode
}

impl App {
    fn handle_narrow_mode_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Esc => {
                if self.content_view == ContentView::Lyrics {
                    self.content_view = ContentView::Library;
                }
            }
            KeyCode::Tab => {
                // Cycle through views
                self.content_view = self.content_view.next();
            }
            _ => {}
        }
    }
}
```

### 6. Tab-Based Navigation

In narrow mode, show tab indicators at the top:

```
┌─────────────────────────┐
│ NP │ Lib │ Set │ [Ly]   │  Active tab highlighted
├─────────────────────────┤
```

Users can:
- Use `Tab` key to cycle forward
- Use `Shift+Tab` to cycle backward
- Use `[` and `]` keys (existing) to cycle

### 7. Adaptive Footer

Footer modules should respect available width:

```rust
pub struct FooterModule {
    pub content: String,
    pub priority: u8,  // Lower = more important
    pub min_width: u16,
}

impl Footer {
    pub fn render_adaptive(&self, width: u16) -> Line<'static> {
        let mut modules = self.modules.clone();
        modules.sort_by_key(|m| m.priority);
        
        let mut current_width = 0;
        let mut visible_modules = Vec::new();
        
        for module in modules {
            if current_width + module.min_width <= width {
                visible_modules.push(module);
                current_width += module.min_width;
            }
        }
        
        // Render visible modules
    }
}
```

#### Footer Module Priorities:
1. Playback status (always shown)
2. Volume (always shown)
3. Queue position (always shown)
4. Time/Date (hide when narrow)
5. Equalizer (hide when narrow)
6. Timezone (hide when narrow)

#### Shortened Formats for Narrow:
- Full: `[▶ Artist - Title]`
- Short: `[▶ Title]`
- Minimal: `[▶]`

### 8. Minimum Terminal Size

Handle terminals that are too small:

```rust
const MIN_WIDTH: u16 = 20;
const MIN_HEIGHT: u16 = 10;

impl App {
    fn check_terminal_size(&self) -> Result<(), AppError> {
        let (width, height) = crossterm::terminal::size()?;
        if width < MIN_WIDTH || height < MIN_HEIGHT {
            return Err(AppError::TerminalTooSmall { width, height });
        }
        Ok(())
    }
}
```

When too small:
```
┌────────────────────┐
│ Terminal too small  │
│                    │
│ Minimum: 20x10     │
│ Current: 15x8      │
│                    │
│ Resize to continue │
└────────────────────┘
```

## Files to Modify

- `gtm/src/ui.rs` — Main layout logic, width detection, adaptive rendering
- `gtm/src/app.rs` — Terminal width state, view management
- `gtm/src/footer.rs` — Adaptive footer rendering
- `gtm/src/overlay.rs` — Lyrics overlay behavior in narrow mode
- `gtm/src/keymap.rs` — Add keybindings for narrow mode navigation
- `gtm-core/src/state.rs` — Add terminal width to app state (optional)

## Implementation Details

### Width Detection

```rust
impl App {
    fn detect_width(&mut self) {
        let (cols, rows) = crossterm::terminal::size().unwrap_or((80, 24));
        self.terminal_width = TerminalWidth::from_cols(cols);
        self.terminal_cols = cols;
        self.terminal_rows = rows;
    }
}
```

### View Cycling

```rust
impl ContentView {
    pub fn next(&self) -> Self {
        match self {
            Self::NowPlaying => Self::Library,
            Self::Library => Self::Settings,
            Self::Settings => Self::NowPlaying,
            Self::Lyrics => Self::Library,  // Back to Library from Lyrics
        }
    }
    
    pub fn prev(&self) -> Self {
        match self {
            Self::NowPlaying => Self::Settings,
            Self::Library => Self::NowPlaying,
            Self::Settings => Self::Library,
            Self::Lyrics => Self::Library,
        }
    }
}
```

### Layout Selection

```rust
fn select_layout(width: TerminalWidth, has_lyrics: bool) -> Layout {
    match width {
        TerminalWidth::Narrow => {
            if has_lyrics {
                Layout::SinglePane(ContentView::Lyrics)
            } else {
                Layout::SinglePaneWithTabs
            }
        }
        TerminalWidth::Medium => Layout::DualPane,
        TerminalWidth::Wide => Layout::TriplePane,
    }
}
```

## Checklist

- [ ] Terminal width detection implemented
- [ ] Narrow mode (< 40 cols) shows single pane
- [ ] Tab navigation works in narrow mode
- [ ] Lyrics replace main content in narrow mode
- [ ] Help text shown when lyrics are active
- [ ] Esc restores Library view from Lyrics
- [ ] Footer adapts to available width
- [ ] Less critical footer modules hidden when narrow
- [ ] Minimum terminal size check (20x10)
- [ ] "Terminal too small" message shown when below minimum
- [ ] TUI does not crash on resize
- [ ] TUI recovers when resized back to normal
- [ ] `[` and `]` keys still work for cycling
- [ ] `Tab` and `Shift+Tab` work for navigation
- [ ] `cargo check --workspace` passes
- [ ] `cargo test --workspace` passes

## Edge Cases

- Terminal resized during playback — should adapt without interrupting
- Terminal resized while lyrics are showing — lyrics should remain
- Very small terminal (10x5) — show minimum size message
- Terminal height changes — footer should adapt vertically too