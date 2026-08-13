# Spec 2: Lyrics pane takes same full height whether visualizer is active or not

## Problem

When the visualizer is active (`show_vis` is true), the lyrics pane is rendered at the
right side of `chunks[1]` (the content area **below** the Now Playing section), making
it only as tall as the track list. When the visualizer is off and lyrics are on, the
lyrics pane gets `full_height_lyrics` mode which spans the **entire vertical** area.

The desired behavior: lyrics pane should always take the full available height,
regardless of whether the visualizer is active.

## Root Cause

In `gtm/src/ui.rs` lines 394-413, the layout splits into `(left_area, vis_area, lyrics_full_area)`
differently based on `full_height_lyrics` vs `show_vis`:

- `full_height_lyrics = app.show_lyrics && !is_narrow && !show_vis` (line 394)
  → When `show_vis=true`, `full_height_lyrics` is false, so lyrics gets `None` for `lyrics_full_area`
  → Instead, lyrics appears in `panes[2]` which is a right column within `chunks[1]` (below Now Playing)

## Files to Modify

- `gtm/src/ui.rs`

## Implementation Steps

### 1. Restructure layout logic

In `gtm/src/ui.rs`, function `render_library()`:

**Change the top-level layout** so that the lyrics column always spans full height:

```rust
// Old: full_height_lyrics depends on !show_vis
let full_height_lyrics = app.show_lyrics && !is_narrow && !show_vis;

// New: lyrics always gets full height when visible
let lyrics_takes_full_height = app.show_lyrics && !is_narrow;
```

**Change the visualizer placement** so it sits within the left column, not competing
with lyrics for vertical space:

```rust
// Split area into left + lyrics columns (lyrics always full-height)
let (left_area, lyrics_area) = if app.show_lyrics && !is_narrow {
    let lyrics_w = area.width / 3;
    let left_w = area.width - lyrics_w;
    let h = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Length(left_w), Constraint::Length(lyrics_w)])
        .split(area);
    (h[0], Some(h[1]))
} else {
    (area, None)
};
```

**Place the visualizer** either:
- To the right of the Now Playing section within `left_area`, OR
- Below the Now Playing section but above the track list in `left_area`

Here's one approach — put visualizer to the right of Now Playing within the left column:

```rust
let chunks = Layout::default()
    .direction(Direction::Vertical)
    .constraints([Constraint::Length(np_height), Constraint::Min(1)])
    .split(left_area);

// Within chunks[0] (Now Playing area), split to accommodate visualizer
let (np_area, vis_area) = if show_vis && vis_wide_enough {
    let vis_w = left_area.width / 5; // narrower than before since left column is smaller
    let h = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Min(0), Constraint::Length(vis_w)])
        .split(chunks[0]);
    (h[0], Some(h[1]))
} else {
    (chunks[0], None)
};
```

**Remove** the old `lyrics_full_area` / `vis_area` split logic and the `full_height_lyrics`
variable. The lyrics pane should always render in the full-height right column.

### 2. Update lyrics rendering

At the bottom of `render_library()`, where lyrics are rendered (~lines 1003-1015):

```rust
// ── Right lyrics pane (full height) ──
if let Some(lyrics_area) = lyrics_area {
    render_lyrics_pane(f, lyrics_area, app);
}
```

Remove the old conditional logic that chooses between `full_lyrics`, `panes[2]`, and `panes[0]`.

## Verification

1. Start TUI with terminal ≥80 columns wide
2. Toggle lyrics with `l` — should show full-height lyrics on the right
3. Toggle visualizer with `Ctrl+V` — lyrics should remain full-height
4. The visualizer should appear within the left column (either beside Now Playing
   or below it), not affecting lyrics height
5. Toggle visualizer off — lyrics should still be full-height
