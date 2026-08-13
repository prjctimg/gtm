# 03 — Live Audio Visualizer

## Goal

Add a live audio visualizer that takes 1/4 width on full-width viewport, uses the same height as the Now Playing section, and can be toggled via keybinding and command palette. Hide it in small terminals. Adjust lyrics pane extension behavior based on visualizer visibility.

## Current State

No audio visualizer exists. The Now Playing section shows static metadata and progress.

## Required Changes

### 1. Create Visualizer Widget

Create a new file `gtm/src/visualizer.rs`:

```rust
pub struct AudioVisualizer {
    enabled: bool,
    style: VisualizerStyle,
    bars: Vec<f32>,  // Frequency data
    height: u16,
    width: u16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VisualizerStyle {
    Bars,        // Classic bar graph
    Wave,        // Waveform line
    Spectrum,    // Heat map spectrum
    Mirrored,    // Bars mirrored top/bottom
}

impl AudioVisualizer {
    pub fn new() -> Self;
    pub fn toggle(&mut self);
    pub fn set_enabled(&mut self, enabled: bool);
    pub fn is_enabled(&self) -> bool;
    pub fn update(&mut self, frequency_data: &[f32]);
    pub fn render(&self, area: Rect) -> Widget<'static>;
}
```

### 2. Frequency Data Acquisition

The daemon needs to provide frequency data for visualization:

```rust
// In gtmd/src/daemon.rs
impl Daemon {
    async fn push_visualizer_data(&self) {
        if let Some(data) = self.backend.get_frequency_data() {
            let event = DaemonEvent::VisualizerData { 
                frequencies: data.to_vec() 
            };
            self.push_event(event).await;
        }
    }
}
```

Add to `gtm-core/src/ipc.rs`:
```rust
pub enum DaemonEvent {
    // ... existing events
    VisualizerData { frequencies: Vec<f32> },
}
```

### 3. Layout Integration

On full-width viewport (> 80 columns), visualizer takes 1/4 width:

```
┌──────────────────────────────────────────────────────────────────┐
│ [1] Now Playing  [2] Library  [3] Settings                       │
├──────────────────────────────────────────────────────────────────┤
│                                                                   │
│  ┌─ NOW PLAYING ──────────────────────┐  ┌─ VISUALIZER ───────┐ │
│  │  ┌──┐  NOW PLAYING                 │  │  ████              │ │
│  │  │▀▀│  Codeine Crazy               │  │  ████ ████         │ │
│  │  │▀▀│  Artist: Future              │  │  ████ ████ ████    │ │
│  │  │▀▀│  Format: FLAC 24-bit/96kHz   │  │  ████ ████ ████    │ │
│  │  └──┘                              │  │  ████ ████ ████    │ │
│  │      00:45          5:52           │  │  ████ ████ ████    │ │
│  │      ════════════════════════      │  │  ████ ████ ████    │ │
│  └─────────────────────────────────────┘  └────────────────────┘ │
│                                                                   │
│  ┌─ LIBRARY ───────────────────────────────────────────────────┐ │
│  │  #  │ Title / Artist / Album      │ Dur   │ Bitrate         │ │
│  │  >01│ Future - Codeine Crazy      │ 05:41 │ 128kbps         │ │
│  │   02│ Juice WRLD - Stay High      │ 03:48 │ 320kbps         │ │
│  └─────────────────────────────────────────────────────────────┘ │
│                                                                   │
├──────────────────────────────────────────────────────────────────┤
│  [0:06] [1/13] [65%] [ALSA] [▶]                                 │
└──────────────────────────────────────────────────────────────────┘
```

### 4. Responsive Behavior

#### Wide Terminal (> 80 cols)
- Visualizer shown (if enabled) on right side
- Takes 1/4 width, same height as Now Playing section

#### Medium Terminal (40-80 cols)
- Visualizer hidden
- Now Playing takes full width

#### Narrow Terminal (< 40 cols)
- Visualizer hidden
- Single pane mode

```rust
fn should_show_visualizer(width: TerminalWidth, enabled: bool) -> bool {
    enabled && matches!(width, TerminalWidth::Wide)
}
```

### 5. Toggle Mechanism

#### Keybinding
- Default: `Ctrl+V` to toggle visualizer
- Add to `gtm/src/keymap.rs`:

```rust
KeyboardAction::ToggleVisualizer,
```

#### Command Palette
- Add "Toggle Visualizer" command to command palette
- Icon: 🎵 or nerd font equivalent

#### Settings
- Add to Settings tab:
```
┌─ SETTINGS ──────────────────────────────────────┐
│  ♫ Audio       Visualizer         [ ● ] On      │
│  ▶ Appearance  ...                               │
└─────────────────────────────────────────────────┘
```

### 6. Visualizer Styles

#### Bars Style (Default)
```
    ▏  ▏    ▏
   ▟▙ ▟▙   ▟▙
  ▟▙▟▙ ▟▙  ▟▙
 ▟▙▟▙▟▙ ▟▙ ▟▙▟▙
▟▙▟▙▟▙▟▙▟▙▟▙▟▙▟▙
```
- Classic frequency bars
- Smooth animation with decay

#### Wave Style
```
    ∿∿∿
  ∿     ∿
∿         ∿∿
            ∿
```
- Continuous waveform line
- Smooth interpolation

#### Spectrum Style
```
████████
████████
  ████
    ██
```
- Heat map visualization
- Color gradient from bottom to top

#### Mirrored Style
```
▟▙▟▙▟▙▟▙▟▙
▟▙▟▙▟▙▟▙▟▙
  ▟▙  ▟▙
    ▟▙
```
- Bars mirrored top and bottom
- Symmetric visualization

### 7. Lyrics Pane Extension Behavior

When lyrics pane is active:

#### Visualizer Hidden
```
┌──────────────────────────────────────────────────────────────────┐
│ [1] Now Playing  [2] Library  [3] Settings                       │
├──────────────────────────────────────────────────────────────────┤
│                                                                   │
│  ┌─ NOW PLAYING ──────────────────────────────────────────────┐ │
│  │  ... (same as before)                                      │ │
│  └─────────────────────────────────────────────────────────────┘ │
│                                                                   │
│  ┌─ LYRICS ───────────────────────────────────────────────────┐ │
│  │                                                            │ │
│  │  [Verse 1]                                                 │ │
│  │  Lyrics content here...                                    │ │
│  │                                                            │ │
│  │  [Chorus]                                                  │ │
│  │  More lyrics...                                            │ │
│  │                                                            │ │
│  └─────────────────────────────────────────────────────────────┘ │
│                                                                   │
├──────────────────────────────────────────────────────────────────┤
│  [0:06] [1/13] [65%] [ALSA] [▶]                                 │
└──────────────────────────────────────────────────────────────────┘
```

Lyrics extend from below Now Playing to bottom of content area.

#### Visualizer Shown
```
┌──────────────────────────────────────────────────────────────────┐
│ [1] Now Playing  [2] Library  [3] Settings                       │
├──────────────────────────────────────────────────────────────────┤
│                                                                   │
│  ┌─ NOW PLAYING ──────────────────────┐  ┌─ VISUALIZER ───────┐ │
│  │  ...                               │  │  ████              │ │
│  └─────────────────────────────────────┘  └────────────────────┘ │
│                                                                   │
│  ┌─ LYRICS ───────────────────────────────────────────────────┐ │
│  │  (shrunk to accommodate visualizer height)                 │ │
│  │  Lyrics content here...                                    │ │
│  └─────────────────────────────────────────────────────────────┘ │
│                                                                   │
├──────────────────────────────────────────────────────────────────┤
│  [0:06] [1/13] [65%] [ALSA] [▶]                                 │
└──────────────────────────────────────────────────────────────────┘
```

Lyrics pane shrinks when visualizer is shown.

```rust
fn calculate_lyrics_height(
    total_height: u16,
    now_playing_height: u16,
    visualizer_visible: bool,
    visualizer_height: u16,
) -> u16 {
    let available = total_height - now_playing_height;
    if visualizer_visible {
        available.saturating_sub(visualizer_height)
    } else {
        available
    }
}
```

### 8. Animation

- Update visualizer at 30fps (33ms intervals)
- Smooth bar decay when audio stops
- Use easing for natural-looking animation
- Pause animation when playback is paused

```rust
impl AudioVisualizer {
    pub fn tick(&mut self) {
        if !self.enabled { return; }
        
        // Apply decay to bars
        for bar in &mut self.bars {
            *bar *= 0.85;  // Decay factor
        }
        
        // Update with new frequency data if playing
        if self.is_playing {
            // Apply new frequency data
        }
    }
}
```

## Files to Modify

- `gtm/src/visualizer.rs` — New file: Visualizer widget
- `gtm/src/ui.rs` — Layout integration
- `gtm/src/app.rs` — Visualizer state management
- `gtm/src/keymap.rs` — Add Ctrl+V keybinding
- `gtm/src/overlay.rs` — Add to command palette
- `gtm/src/settings.rs` — Add visualizer toggle setting
- `gtm-core/src/ipc.rs` — Add VisualizerData event
- `gtm-core/src/state.rs` — Add visualizer state
- `gtmd/src/daemon.rs` — Generate frequency data
- `gtm-audio/src/backend.rs` — Add get_frequency_data() method

## Implementation Details

### Frequency Data Processing

```rust
impl AudioVisualizer {
    pub fn update_from_frequencies(&mut self, freq_data: &[f32]) {
        // Map frequency bins to visualizer bars
        let num_bars = self.width as usize;
        let bin_size = freq_data.len() / num_bars;
        
        for (i, bar) in self.bars.iter_mut().enumerate() {
            let start = i * bin_size;
            let end = (start + bin_size).min(freq_data.len());
            
            // Average frequency magnitude for this bar
            let avg: f32 = freq_data[start..end]
                .iter()
                .sum::<f32>() / bin_size as f32;
            
            // Apply smoothing
            *bar = bar.mul_add(0.7, avg * 0.3);
        }
    }
}
```

### Bar Rendering

```rust
fn render_bars(&self, area: Rect) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    
    for row in (0..area.height).rev() {
        let mut spans = Vec::new();
        for (col, &bar_height) in self.bars.iter().enumerate() {
            let normalized_height = bar_height * area.height as f32;
            if normalized_height > row as f32 {
                spans.push(Span::styled(
                    "█",
                    Style::default().fg(Color::Green)
                ));
            } else {
                spans.push(Span::raw(" "));
            }
        }
        lines.push(Line::from(spans));
    }
    
    lines
}
```

## Checklist

- [ ] AudioVisualizer struct created in `gtm/src/visualizer.rs`
- [ ] VisualizerData event added to IPC
- [ ] Daemon generates frequency data
- [ ] Visualizer renders in 1/4 width on wide terminals
- [ ] Visualizer hidden on medium/narrow terminals
- [ ] Ctrl+V toggles visualizer
- [ ] Command palette entry added
- [ ] Settings toggle added
- [ ] All 4 visualizer styles implemented
- [ ] Animation runs at 30fps
- [ ] Bars decay smoothly when audio stops
- [ ] Animation pauses when playback paused
- [ ] Lyrics pane extends to bottom when visualizer hidden
- [ ] Lyrics pane shrinks when visualizer shown
- [ ] Visualizer uses same height as Now Playing section
- [ ] `cargo check --workspace` passes
- [ ] `cargo test --workspace` passes

## Performance Considerations

- Frequency data should be processed in a separate task
- Use ring buffer for frequency data to avoid allocation
- Limit update rate to 30fps to reduce CPU usage
- Consider using SIMD for frequency processing if needed