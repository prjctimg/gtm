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

### 2.3.1 — Below-list descriptions (Subjective style)
- After each equalizer preset line in the list, add a **subjective** description of how that preset sounds
- Descriptions for each preset:
  - **Braille**: "Smooth, flowing bars with a vintage feel — warm and understated, like watching an old oscilloscope"
  - **Blocks**: "Bold, chunky blocks with a retro arcade vibe — punchy and direct, great for electronic music"
  - **Mirror**: "Symmetrical, hypnotic waves radiating from center — mesmerizing and balanced, ideal for ambient"
  - **Gradient**: "Color-shifting bands that pulse with intensity — vibrant and dynamic, perfect for high-energy tracks"
  - **Spectrum**: "Scrolling waveform that captures every beat — fluid and responsive, like a living equalizer"

### 2.3.2 — Description style rationale
- Using **subjective only** style (no technical/use-case/mixed options)
- Keeps descriptions concise and focused on the "feel" of each visualizer
- Avoids cluttering the picker with multiple description modes
- Users can see at a glance what each preset "feels like"

---

## 2.4 — Fuzzy finder with cover art + metadata

**File**: `gtm/src/ui.rs` (search picker render function)

### 2.4.1 — Layout: Side-by-side (cover left, meta right)
- When fuzzy finder is open, below the list of matching tracks, show a preview panel
- Layout:
  ```
  ┌─────────────────────────────────────────┐
  │  Search: "endless"                      │
  ├─────────────────────────────────────────┤
  │  1. An Endless Summer - The Waves       │
  │  2. Endless Road - John Mayer           │
  │  3. Never Ending Story - Limahl         │
  ├─────────────────────────────────────────┤
  │  ┌─────────┐  Track: An Endless Summer  │
  │  │         │  Artist: The Waves         │
  │  │  Cover  │  Album: Ocean Drive        │
  │  │   Art   │  Duration: 3:45            │
  │  │         │                            │
  │  └─────────┘                            │
  └─────────────────────────────────────────┘
  ```
- Cover art is fetched from the track's cover art URL or local file
- If cover art fails to load, show a placeholder colored square with "♫" glyph

### 2.4.2 — Cover art render
- Cover image rendered as a small widget on the left side of the preview panel
- If cover art fails to load, show a placeholder colored square

### 2.4.3 — Track metadata render
- To the right of the cover art preview, show:
  - Track title (bold)
  - Artist name
  - Album name
  - Duration (formatted as mm:ss)

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
