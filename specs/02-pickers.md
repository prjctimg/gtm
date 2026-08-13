# Spec 02 — Better Pickers and Floating Windows

Status: **Planned** — enhanced pickers with cover art, swatches, descriptions, and ASCII demos.

Green gate: `cargo fmt --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`

---

## 2.1 — Metadata editor with cover sync (floating window)

**File**: `gtm/src/app.rs`, `gtm/src/ui.rs`

### 2.1.1 — Floating metadata editor
- New `EditMetadataPicker` component (new UI module, or expanded existing `app.rs` picker state)
- Floating window opens at bottom-right of the current tab with:
  - Track title (left)
  - Artist name (left)
  - Album name (left)
  - Cover image (bottom-right, embedded preview)
  - "Sync Cover" keybinding button (below cover)
  - Track metadata fields editable inline
- When user clicks "Sync Cover": force re-read cover art from disk, update `cover_bytes` in the app, and re-render the metadata editor
- When file is ready to display, show preview image immediately via forced refresh

### 2.1.2 — Cover image render
- Cover art is fetched via `gtm-core/src/cover_art.rs` or similar
- Cover preview rendered as a small image widget in the bottom-right corner of the metadata editor
- After sync: force a full redraw of the editor

---

## 2.2 — Theme picker with swatches

**File**: `gtm/src/ui.rs` (theme picker render function)

### 2.2.1 — Color swatches on far right
- In the theme picker list, add a small color swatch (e.g., 16x16 pixel colored square) on the far right of each row
- Swatch uses the theme's `accent` color rendered as a `Color::Rgb` fill
- This lets users preview the accent color before selecting a theme
- When a new theme is selected, the swatch updates to show the new accent

### 2.2.2 — Theme preview
- Add a small preview strip below each theme row showing:
  - Background color (filled rectangle)
  - Pane fill color
  - Accent color
  - A tiny color wheel indicator (optional)
- Preview renders in real-time as the user selects a theme

---

## 2.3 — Equalizer picker with descriptions

**File**: `gtm/src/ui.rs` (equalizer picker render function)

### 2.3.1 — Below-list descriptions
- After each equalizer preset line in the list, add a description of how that preset sound works
- Descriptions for each preset:
  - **Braille**: Uses 4-step color ramp; bars with higher amplitude get brighter accent colors
  - **Blocks**: Solid blocks with fine fractional heights; blocks are colored by amplitude
  - **Mirror**: Center-symmetric bars with mirrored shape; bars expand outward from center
  - **Gradient**: Color ramp through gradient pattern; bars with higher value get brighter colors
  - **Spectrum**: Scrolling waveform bar with phase shift; bars change with frequency content

### 2.3.2 — Description styles (mixed)
- Each description follows a multi-style approach:
  - **Technical**: "Frequency spectrum analyzer using phase-locked bandpass filter"
  - **Subjective**: "Warm vintage sound, great for jazz and lo-fi"
  - **Use-case**: "Best for vocal-heavy tracks, cut for bass-heavy music"
- User can select a description style when the picker opens
- A small toggle in the picker header (or a description panel next to each item) allows choosing the style

---

## 2.4 — Fuzzy finder with cover art + metadata

**File**: `gtm/src/ui.rs` (search picker render function)

### 2.4.1 — Cover art preview below list
- When fuzzy finder is open, below the list of matching tracks, show a cover art preview
- Cover image is fetched from the track's cover art URL or local file
- If cover art fails to load, show a placeholder colored square

### 2.4.2 — Track meta beside preview
- To the right of the cover art preview, show:
  - Track title
  - Artist name
  - Album name
  - Duration

### 2.4.3 — ASCII demos first
- Before settling on a final design, show ASCII-style demo of the layout
- ASCII demo:
  ```
  Track: "An Endless Summer" — by The Waves
  ┌─────────────────────────────────────────┐
  │  📷 Cover Preview                    │
  │  ┌───────────────────────────────┐     │
  │  │  ◆ Track Title:                │     │
  │  │  │  An Endless Summer         │     │
  │  │  │  ───────────────────────── │     │
  │  │  │  Artist: The Waves         │     │
  │  │  │  Album: Ocean Drive        │     │
  │  │  │  Duration: 3:45            │     │
  │  │  └───────────────────────────────┘     │
  │  └─────────────────────────────────────────┘
  │  [Search results ...]
  ```
- This ASCII demo is shown in a small overlay panel at the top of the fuzzy finder picker
- After the user makes a selection, the final design is applied

---

## 2.5 — Pickers for all preset types with previews

**File**: `gtm/src/ui.rs`

### 2.5.1 — Visualizer preset picker
- List: Braille, Blocks, Mirror, Gradient, Spectrum
- Below each preset item, show a text preview of the visualizer character set used
- Highlighted preset has a preview strip showing the characters used

### 2.5.2 — Progress style picker
- List: Braille, SeekHead, Classic, Dots, Arrows, Blocks, Gradient
- Below each preset, show preview characters

### 2.5.3 — Footer preset picker
- List: Default, Minimal, Full
- Below each, show footer module content preview (what text appears in each section)

### 2.5.4 — Crossfade type picker
- List: auto/on/off/adaptive
- Below each, show how crossfade behaves (character/description preview)

### 2.5.5 — Implementation pattern
- For each picker type, add a `preview()` function that renders a small preview of the highlighted preset below the list
- The preview uses characters or simple ASCII patterns specific to the preset type
- The picker scrolls when the list exceeds terminal height
- Highlighted item gets a distinct highlight color (from the current accent theme)

---

## 2.6 — Notification system (for these pickers)

When a preset is selected from a picker, a floating toast notification slides in from the right (configurable) using `tachyonfx` for easing. The notification shows the preset name and a brief description, then auto-dismisses after 3 seconds.
