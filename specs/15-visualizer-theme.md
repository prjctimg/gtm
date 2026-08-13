# Spec 15 — Visualizer Theme Integration + Remove Animation

## Requirements

- Make the visualizer respect theme changes.
- Do not animate the visualizer so that it appears to move out of view, just show the amplitude changes vertically.

## Current State

`gtm/src/visualizer.rs` uses hardcoded `Color::Rgb(0, 200, 80)` green/teal colors. When paused, bars decay to zero (`* 0.92` per tick, line 109), making the visualizer "move out of view."

## Changes

### 15a. Theme colors (`gtm/src/visualizer.rs`)

- Add `theme: Option<&AppTheme>` parameter to `render()` and all `render_*` methods (Bars, Dots, Braille, Wave)
- Pass theme through from `gtm/src/ui.rs` render calls
- Replace hardcoded colors with theme-derived colors:
  - High amplitude (>0.7): `theme.accent`
  - Medium amplitude (>0.4): `theme.fg_bright`
  - Low amplitude (else): `theme.fg_dim`
  - Empty cells: no explicit color (or `theme.bg` for braille empty cells)
- For Wave style: gradient from `theme.fg_dim` (center) to `theme.accent` (peaks)

### 15b. Remove pause decay animation (`gtm/src/visualizer.rs:98-112`)

- When paused (`!is_playing`): set `target_bars` to 0.0 (keep this)
- Remove the `* 0.92` decay multiplier (line 109) — this causes the "move out of view" effect
- Instead, when paused, bars snap to 0 immediately (or decay very fast, e.g. `*= 0.5`)
- This way the visualizer shows amplitude changes while playing and goes flat when paused — no sweeping/scrolling out of view

### 15c. Pass theme to render calls (`gtm/src/ui.rs`)

- In `render_library()` where visualizer is rendered (lines 551-563):
  - Change `app.visualizer.render(vis_inner)` to `app.visualizer.render(vis_inner, &app.theme)`

## Verification
- Visualizer colors change when switching themes
- Visualizer bars don't sweep/fade out of view — they drop to flat immediately when paused
- While playing, bars show amplitude changes vertically only
