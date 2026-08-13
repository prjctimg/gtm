# Spec 5: TUI crashes on small viewports

## Problem

The TUI crashes when the terminal is resized to very small dimensions. The current
guard only handles extreme cases (cols < 40 || rows < 10), but intermediate sizes
cause panics in the layout system due to:
- Negative widths from `Constraint::Length` exceeding available space
- Division by zero in layout calculations
- Buffer overruns from `Rect` coordinates outside the buffer area

## Root Cause

1. `gtm/src/ui.rs:386-390` — `lib_width = (app.terminal_cols / 3).max(12)` can be
   larger than available width in narrow modes, causing `Layout::split()` to panic.
2. `Constraint::Length(28)` at line 389 — hardcoded width may exceed terminal width.
3. Various `Rect` operations use regular subtraction instead of `saturating_sub`,
   leading to underflow.
4. The render function doesn't guard against zero-height areas from nested layouts.

## Files to Modify

- `gtm/src/ui.rs`

## Implementation Steps

### 1. Global minimum size guard

At the top of `render()` (~line 177), after `area = f.area()`:

```rust
pub fn render(f: &mut ratatui::Frame, app: &mut App) {
    let area = f.area();
    if area.width < 20 || area.height < 6 {
        let msg = Paragraph::new("Terminal too small (min 20x6)")
            .alignment(Alignment::Center)
            .style(Style::default().fg(app.theme.fg_dim));
        f.render_widget(msg, area);
        return;
    }
    // ... rest of render
}
```

Also update the existing guard in `app.rs` (~line 753):

```rust
if cols < 20 || rows < 6 {
    // Don't attempt to draw — just show a message
    let _ = terminal.draw(|f| {
        let msg = Paragraph::new("Terminal too small (min 20x6)")
            .alignment(Alignment::Center);
        f.render_widget(msg, f.area());
    });
} else if terminal.draw(|f| ui::render(f, &mut self)).is_ok() {
    // ...
}
```

### 2. Safe constraint values

Replace hardcoded `Constraint::Length(28)` in `render_library()`:

```rust
let lib_width: u16 = if is_narrow {
    (app.terminal_cols / 3).max(12).min(area.width.saturating_sub(2))
} else {
    28u16.min(area.width.saturating_sub(2))
};
```

### 3. Saturating math throughout

Search for all subtraction operations on `u16` values in `render_library()` and
replace with `saturating_sub()`. Key areas:

- `area.width.saturating_sub(lyrics_w)` instead of `area.width - lyrics_w`
- `area.height.saturating_sub(reserve as u16)` instead of direct subtraction
- All `x + width` comparisons should check against available space

### 4. Guard Layout::split() calls

Create a helper function:

```rust
fn safe_split(layout: Layout, area: Rect) -> Vec<Rect> {
    if area.width == 0 || area.height == 0 {
        return vec![area; layout.constraints.len()];
    }
    layout.split(area)
}
```

Use `safe_split()` everywhere instead of `layout.split()`.

### 5. Zero-area guards in sub-renderers

In every rendering function (`render_library`, `render_settings`, `render_lyrics_pane`,
`render_cover`, etc.), add at the top:

```rust
if area.width < 2 || area.height < 1 {
    return;
}
```

## Verification

1. Start the TUI in a normal-sized terminal (80×24)
2. Gradually resize the terminal to smaller dimensions
3. Verify the TUI shows "Terminal too small" message instead of crashing
4. Verify the TUI recovers properly when resized back up
5. Test at exact boundary sizes: 20×6, 40×10, 60×15
6. Test rapid resize events
