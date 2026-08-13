# 01 — Customizable Track Progress Indicators

## Goal

Move all track progress indicators into a separate struct and add a settings option to allow users to change the style at will. Create new creative styles and improve the existing filled bar style.

## Current State

Progress indicator styles are currently tied to the theme toggle. Only one style is visible at a time, and switching requires changing the entire theme.

## Required Changes

### 1. Create ProgressIndicator Struct

Create a new file `gtm/src/progress.rs` with a dedicated struct for progress indicators:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ProgressStyle {
    FilledBar,      // Current style (improved)
    Line,           // Line with oscillating head
    Waveform,       // Audio waveform style
    AnalogSignal,   // Analog sine wave style
    Dots,           // Dot matrix style
    Blocks,         // Block characters (█▓▒░)
    Arrows,         // Arrow progression (→)
    Braille,        // Braille dot pattern
    Gradient,       // Gradient fill effect
}

pub struct ProgressIndicator {
    style: ProgressStyle,
    width: u16,
    elapsed: f64,
    duration: f64,
    is_playing: bool,
}

impl ProgressIndicator {
    pub fn new(style: ProgressStyle, width: u16) -> Self;
    pub fn set_style(&mut self, style: ProgressStyle);
    pub fn update(&mut self, elapsed: f64, duration: f64, is_playing: bool);
    pub fn render(&self) -> Line<'static>;
}
```

### 2. Implement New Styles

#### Waveform Style
```
▓▓▓▓▓▓▓▓░░░░░░░░░░░░░░░░░░░░░░░░
█▇▆▅▄▃▂▂▃▄▅▆▇█▇▆▅▄▃▂▂▃▄▅▆▇█▇▆▅
```
- Uses varying block heights to simulate audio waveform
- Animated when playing (wave moves)

#### Analog Signal Style
```
~∿∿∿∿∿∿∿∿────────────────────────
```
- Sine wave pattern for played portion
- Flat line for remaining

#### Dots Style
```
●●●●●●●●○○○○○○○○○○○○○○○○○○○○○○○
```
- Filled dots for played, empty for remaining
- Simple and clean

#### Blocks Style
```
████████░░░░░░░░░░░░░░░░░░░░░░░░
```
- Full block characters for played
- Light shade for remaining

#### Arrows Style
```
━━━━━━━━━━▶─────────────────────
```
- Arrow head at current position
- Bold line for played, thin for remaining

#### Braille Style
```
⣿⣿⣿⣿⣿⣿⣿⣿⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀
```
- Braille characters for played portion
- Empty braille for remaining

#### Gradient Style
```
████████▓▓▓▓▒▒▒▒░░░░░░░░░░░░░░░
```
- Gradient from full to light shade

### 3. Improve Filled Bar Style

Current:
```
[═══════════════════════════════════════════════════════════]
```

Improved:
```
════════════════════════●──────────────────────────────────
```

Changes:
- Remove opening and closing square brackets `[]`
- Add seek head `●` at current position
- Make the bar thinner (1 line instead of potential 2)
- Use `═` for played, `─` for remaining

### 4. Add Settings Option

Add to Settings tab:

```
┌─ SETTINGS ──────────────────────────────────────┐
│  ♫ Audio       Progress Style    [ FilledBar  ▶ ] │
│  ▶ Appearance  ...                               │
└─────────────────────────────────────────────────┘
```

- Cycle through styles with Enter/Arrow keys
- Preview changes in real-time on Now Playing tab
- Persist selection in config file

### 5. Configuration

Add to config file (`~/.config/gtom/config.toml`):

```toml
[appearance]
progress_style = "Waveform"  # FilledBar, Line, Waveform, AnalogSignal, Dots, Blocks, Arrows, Braille, Gradient
```

## Files to Modify

- `gtm/src/progress.rs` — New file: ProgressIndicator struct and styles
- `gtm/src/ui.rs` — Import and use ProgressIndicator
- `gtm/src/theme.rs` — Remove progress style from theme (separate concern)
- `gtm/src/app.rs` — Add progress style to app state
- `gtm/src/overlay.rs` — Add progress style selector to Settings
- `gtm-core/src/state.rs` — Add progress style to DaemonState (or TUI state)
- `gtm/src/config.rs` — Add progress_style config option

## Implementation Details

### Style Rendering

Each style should implement a `render` method that returns a `Line<'static>`:

```rust
impl ProgressIndicator {
    pub fn render(&self) -> Line<'static> {
        match self.style {
            ProgressStyle::FilledBar => self.render_filled_bar(),
            ProgressStyle::Waveform => self.render_waveform(),
            ProgressStyle::AnalogSignal => self.render_analog(),
            // ...
        }
    }
    
    fn render_filled_bar(&self) -> Line<'static> {
        let progress = if self.duration > 0.0 {
            (self.elapsed / self.duration) as f64
        } else {
            0.0
        };
        
        let filled = (self.width as f64 * progress) as usize;
        let empty = self.width as usize - filled;
        
        let mut spans = vec![
            Span::styled("═".repeat(filled), Style::default().fg(Color::Green)),
            Span::styled("●", Style::default().fg(Color::White).add_modifier(Modifier::BOLD)),
            Span::styled("─".repeat(empty.saturating_sub(1)), Style::default().fg(Color::DarkGray)),
        ];
        
        Line::from(spans)
    }
}
```

### Animation

For animated styles (Waveform, AnalogSignal):
- Use a frame counter that increments on each render
- Shift the pattern based on frame counter
- Only animate when `is_playing` is true
- Use `crossterm::terminal::disable_raw_mode()` / `enable_raw_mode()` for smooth animation if needed

### Settings Integration

Add to Settings tab UI:

```rust
fn render_progress_style_setting(f: &mut Frame, area: Rect, state: &AppState) {
    let styles = [
        "FilledBar", "Line", "Waveform", "AnalogSignal", 
        "Dots", "Blocks", "Arrows", "Braille", "Gradient"
    ];
    
    let selected = styles.iter().position(|s| *s == state.progress_style);
    
    // Render dropdown/selector
}
```

## Checklist

- [ ] ProgressIndicator struct created in `gtm/src/progress.rs`
- [ ] All 9 styles implemented and rendering correctly
- [ ] Filled Bar style improved (no brackets, thinner, seek head)
- [ ] Settings option added to Settings tab
- [ ] Config file option added
- [ ] Style persists across sessions
- [ ] Animation works for animated styles
- [ ] Style can be changed without restarting TUI
- [ ] Preview updates in real-time
- [ ] All styles work at different terminal widths
- [ ] `cargo check --workspace` passes
- [ ] `cargo test --workspace` passes

## Visual Examples

### FilledBar (Improved)
```
════════════════════════●──────────────────────────────────
```

### Line
```
────────────────────────●─────────────────────────────────
```

### Waveform
```
█▇▆▅▄▃▂▂▃▄▅▆▇█▇▆▅▄▃▂▃▄▅▆▇█▇▆▅▄▃▂▃▄▅▆▇█▇▆▅▄▃▂▃▄▅▆▇█▇▆
```

### AnalogSignal
```
~∿∿∿∿∿∿∿∿∿∿∿∿────────────────────────────────────────────
```

### Dots
```
●●●●●●●●●●●●●●●●●●●●●●●●○○○○○○○○○○○○○○○○○○○○○○○○○○○○○○○
```

### Blocks
```
████████████████████████░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░
```

### Arrows
```
━━━━━━━━━━━━━━━━━━━━━━━━▶─────────────────────────────────
```

### Braille
```
⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀
```

### Gradient
```
████████████████████▓▓▓▓▓▓▒▒▒▒░░░░░░░░░░░░░░░░░░░░░░░░░░
```