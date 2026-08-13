# Spec 02 — TUI Structural Redesign

Status: **Planned**. Goal: borderless, padded, hierarchy-via-typography layout
with a floating command palette. Constraint: **audio and state-management
logic remain untouched**; this is a `gtm/src/ui.rs` + `gtm/src/app.rs` render
layer change only (plus theme schema).

Theme rule: no hardcoded hex/RGB/base colors anywhere in the render layer —
use `app.theme.*` exclusively.

---

## C0 — Theme schema: `elevated_bg` + `muted_border`

`gtm/src/theme.rs`:

- Add `pub elevated_bg: Color` and `pub muted_border: Color` to `AppTheme`
  (lines 12-31).
- Set a value in **all 12 built-in themes** (`theme.rs:79-340`):
  - `elevated_bg`: one step darker/lighter than `picker_bg` (e.g. Chadrula:
    picker `#1f2335` → elevated `#1a1e2e`; Catppuccin Latte: picker `#e6e9ef`
    → elevated `#dde1e9`). Derive per palette, don't share one value.
  - `muted_border`: use the existing `border` value, or `fg_dim` where the
    palette's `border` is too loud. Keep it visibly quieter than
    `border_active`.
- Add the two fields to `UserThemeFile` (`theme.rs:425-447`) **with
  `#[serde(default = ...)]` fallbacks** so existing user theme TOMLs do not
  break: `elevated_bg` defaults to `picker_bg`, `muted_border` to `border`
  (read from the same file), or to the parsed fallback functions.
- Update the round-trip test at `theme.rs:594-614` with the new keys.

## C1 — Remove structural borders from the 7 main panes

All use `Block::default().borders(Borders::ALL)` (`BorderType::Plain`). For
each pane below: drop the border, add `.padding(Padding::horizontal(1))` (or a
manual `Rect` inset), and render a 1-line pane header instead of a block title.

| Pane | Block at `gtm/src/ui.rs` | New header | Separator |
|---|---|---|---|
| Now Playing | 455-459 | `NOW PLAYING` (bold `fg_bright`; `accent` when the app has no left/right focus concept for it) | `Borders::TOP` in `muted_border` under the header, or none (rely on spacing) |
| Visualizer | 665-669 | `VISUALIZER` (bold `fg_dim`) | none |
| Library left (categories) | 718-726 | `LIBRARY` (bold `fg_bright`; `accent` when `library_pane_focus`) | `Borders::RIGHT` in `muted_border` (or spacing) |
| Library right (tracks) | 1065-1073 | `TRACKS` / category name (bold `fg_bright`; `accent` when right pane focused) | `Borders::LEFT` in `muted_border` |
| Settings left | 1124-1127 | category header bold `fg_bright`; `accent` when `settings_pane_focus` | spacing |
| Settings right | 1275-1278 | option header bold `fg_bright` | spacing |
| Lyrics | 1741-1745 | `LYRICS` (bold `fg_bright`; `accent` when `lyrics_pane_focus`) | `Borders::LEFT` in `muted_border` |

Focus indication: header color switches to `accent` + bold for the focused
pane, plus a 1-cell `▎` accent bar rendered at the focused pane's left edge
(a `Span::styled("▎", accent)` line, or `Borders::LEFT` in `accent`). Never
use `border_active` full bounding boxes for focus.

Cover-art placeholders `ui.rs:280` and `ui.rs:293` go flat (no border, plain
`fg_dim` `" ♫ "` glyph area).

## C2 — Spacing and padding

- `render_library` layout (`ui.rs:384-427`): insert a `Length(1)` spacer row
  between the Now Playing chunk and the library chunk.
- `render_settings` (`ui.rs:1092`): same 1-row spacer after the header.
- Content padding: `.padding(Padding::horizontal(1))` on every pane block; the
  track-list rows already carry a leading space (`ui.rs:1032`) — widen to two
  spaces so text never sits flush against the separator edge.
- Pane headers: render as a `Paragraph`/`Line` 1 row tall with a `Length(1)`
  gap beneath (or `Block::title` style but borderless — title without borders
  renders inline; prefer explicit header rows for full control).

## C3 — Typographic hierarchy (no borders)

- Pane headers: `Modifier::BOLD` + `fg_bright`; focused → `accent` + BOLD.
- Metadata/counts (`st_line`, durations): `fg_dim`.
- Current track row: `accent` + BOLD (already `ui.rs:1036-1043` — keep).
- Selected row: `selection_fg` on `selection_bg` full-width (already — keep).
- Toggle hint rows / dim labels in settings: `fg_dim`.

## C4 — Floating command palette

Rendering order is already correct: main UI renders first, overlays last
(`render()` at `ui.rs:210-223`; `render_picker` at `ui.rs:1343-1370` renders
`Clear` over a centered rect, then the picker body).

Changes:

1. `render_picker` (`ui.rs:1348-1362`): special-case `PickerId::CommandPalette`
   → centered rect of **60% width × 40% height** (currently the shared path
   uses 60%×70%):
   ```rust
   let w = (area.width as f64 * 0.6) as u16;
   let h = (area.height as f64 * 0.4) as u16;
   let x = (area.width - w) / 2;
   let y = (area.height - h) / 2;
   ```
   (respect `transparent_bg` as today).
2. `render_command_palette_picker` (`ui.rs:2408-2418`): replace the bordered
   block with a flat elevated panel:
   ```rust
   let block = Block::default()
       .borders(Borders::ALL)
       .border_style(Style::default().fg(theme.muted_border))
       .title(" Commands ")
       .style(Style::default().bg(if app.transparent_bg { Color::Reset } else { theme.elevated_bg }));
   ```
   Keep `f.render_widget(Clear, area)` (already in `render_picker`) so no
   underlying text bleeds through.
3. Selection highlight (`ui.rs:2438-2458`): selected row uses `selection_bg`
   with `selection_fg`; make the row span the full inner width (pad the name
   column so the highlight runs edge-to-edge).
4. Apply the same elevated-panel treatment to the other floating pickers
   (Queue `1449`, YT search `1522`, Library search `1609`, Equalizer `2503`,
   Sound Effects `2557`, Theme `2627`, Playlist select `2790`, Edit metadata
   `2831`, About `1993`, Sleep timer `2217/2239`): drop `Borders::ALL` (or use
   `muted_border`) + `elevated_bg` fill. Small anchored popups that are not
   full-bleed panels (track popup `1863`, health panel `2896`) keep their
   current treatment but use `elevated_bg` for the fill.
5. Cleanup: remove the **duplicate equalizer block** — `render_equalizer_picker`
   draws its bordered block twice (`ui.rs:2502-2510` and `2519-2527`); collapse
   to one.

## Acceptance

- `cargo test --workspace` green (theme tests updated for new fields).
- TUI visual pass: no bounding boxes on main panes; panes separated by
  padding + subtle `muted_border` lines; focused pane clearly indicated by
  accent header + edge bar; command palette floats at 60%×40% with opaque
  `elevated_bg`, full-width selection highlight, no text bleed-through.
- Resize to <60 cols still lays out correctly (narrow path, `ui.rs:390-411`).
- No hardcoded colors introduced in `gtm/src/*.rs` render code.
