# Spec 1: Visualizer crashes the TUI when playback is active

## Problem

The `AudioVisualizer::tick()` method runs every frame, computing synthetic audio data.
When playback is active, it recalculates all bar amplitudes using trigonometric functions.
If the visualizer area has zero width or height (e.g., during terminal resize), `resize()`
and `render()` can panic or produce invalid state.

## Root Cause

1. `tick()` in `gtm/src/visualizer.rs:75` doesn't guard against zero-width areas.
   It calls `self.bars.resize(num_bars, 0.0)` where `num_bars = width.max(4) as usize`.
   With width=0, `num_bars = 4` which is safe, but the decay math uses `dt` which can
   accumulate NaN if `self.last_tick` is stale.

2. `Lines` widget in `gtm/src/visualizer.rs:244` calls `buf.cell_mut((x, area.y + row as u16))`
   without bounds checking the buffer dimensions, which can panic.

3. `render()` in `gtm/src/visualizer.rs:125` checks `area.width < 4 || area.height < 3`
   and returns `None`, but `tick()` runs unconditionally before `render()`.

## Files to Modify

- `gtm/src/visualizer.rs`
- `gtm/src/ui.rs`

## Implementation Steps

### 1. Guard tick() against zero width

In `gtm/src/visualizer.rs`, `tick()` method (~line 75):

```rust
pub fn tick(&mut self, is_playing: bool, width: u16) {
    if !self.enabled || width == 0 {
        return;
    }
    // ... rest of existing code
}
```

### 2. Guard visualizer rendering in ui.rs

In `gtm/src/ui.rs`, around lines 674-689, wrap the visualizer block:

```rust
if let Some(vis_a) = vis_area {
    if vis_a.width >= 4 && vis_a.height >= 3 {
        app.visualizer.tick(
            app.state.status == gtm_core::state::PlaybackStatus::Playing,
            vis_a.width,
        );
        let vis_block = Block::default()
            .borders(Borders::ALL)
            .title(" Visualizer ")
            .border_type(BorderType::Plain)
            .border_style(Style::default().fg(app.theme.fg_dim));
        let vis_inner = vis_block.inner(vis_a);
        f.render_widget(vis_block, vis_a);
        if let Some(lines) = app.visualizer.render(vis_inner, &app.theme) {
            f.render_widget(lines, vis_inner);
        }
    }
}
```

### 3. Bounds-check in Lines widget

In `gtm/src/visualizer.rs`, the `Widget for Lines` impl (~line 244):

```rust
impl<'a> Widget for Lines<'a> {
    fn render(self, area: Rect, buf: &mut ratatui::buffer::Buffer) {
        for (row, line) in self.0.iter().enumerate() {
            if row as u16 >= area.height {
                break;
            }
            let mut x = area.x;
            for span in &line.spans {
                for ch in span.content.chars() {
                    if x >= area.x + area.width {
                        break;
                    }
                    let cell_x = x;
                    let cell_y = area.y + row as u16;
                    if cell_x < buf.area.width && cell_y < buf.area.height {
                        if let Some(cell) = buf.cell_mut((cell_x, cell_y)) {
                            cell.set_symbol(&ch.to_string()).set_style(span.style);
                        }
                    }
                    x += 1;
                }
            }
        }
    }
}
```

## Verification

1. Start the TUI with a terminal ≥80 columns wide
2. Toggle visualizer with `Ctrl+V`
3. Play a track
4. Resize terminal to various sizes, including very small (e.g., 50x10)
5. Verify no crash occurs
6. Verify visualizer reappears when terminal is resized back to ≥80 columns
