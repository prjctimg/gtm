# SPECS: PROMPT.md Resolution Plan

Comprehensive specifications for all regressions, overlay refactors, and preset improvements
requested in `PROMPT.md`.

---

## PART A: REGRESSIONS

### A1. TUI Startup Stall

**Problem:** Terminal is blank for >1 minute after launch.

**Root cause:** Entirely serial initialization with blocking sleeps before first render.

| Step | File:Line | Delay |
|------|-----------|-------|
| `ensure_daemon_running()` sync socket ping | `ui.rs:69-109` | 0–250ms |
| `DaemonClient::connect()` 10 retries w/ backoff | `client.rs:44-93` | up to 2,850ms |
| `fetch_state()` IPC | `app.rs:389` | variable |
| `set_volume(85)` IPC | `app.rs:392-395` | variable |
| `fetch_queue()` IPC | `app.rs:396` | variable |
| `fetch_library_tracks()` 3 retries w/ 500ms sleeps | `app.rs:882-903` | up to 1,000ms |
| `Picker::from_query_stdio()` blocking terminal query | `app.rs:401-405` | 10–100ms |

**Spec:**
1. Parallelize `fetch_state()`, `set_volume()`, `fetch_queue()` via `tokio::join!`.
2. Move `fetch_library_tracks()` to background spawn — do not block first render.
3. Remove retry sleeps from `fetch_library_tracks()`.
4. Render a loading splash immediately after `terminal.clear()`.

**Files:** `gtm/src/ui.rs`, `gtm/src/app.rs`

---

### A2. CLI-Triggered Playback Not Reflected in TUI

**Problem:** Starting playback via `gtm play <path>` then opening TUI shows no Now Playing state.

**Root cause:** Clock is never seeded from initial `get_status()` response. `estimated_position()`
relies on `base_pos`/`base_time` only set by `apply_clock_events()` or `seed_clock_from_state()`,
neither of which fires on the initial state fetch.

**Spec:**
1. Call `self.client.seed_clock_from_state(&self.state).await` after `fetch_state()`.
2. Verify daemon sends current state to new clients on connect.

**Files:** `gtm/src/app.rs:~390`, `gtm-core/src/client.rs:105-163`

---

### A3. Visualizer Width Mismatch

**Problem:** Lyrics pane width jumps when visualizer is toggled.

**Root cause:** Lyrics full-height width = `area.width / 3`, visualizer width = `terminal_cols / 4`.

**Spec:**
1. Change visualizer width from `terminal_cols / 4` to `area.width / 3` at both split points.

**Files:** `gtm/src/ui.rs:363-371, 381-392`

---

### A4. Track Info Popup Layout Shift

**Problem:** Popup changes size when cover image loads.

**Root cause:** Width/height differ between `has_cover` (51×7) and `!has_cover` (48×6).

**Spec:**
1. Use fixed dimensions (51×7) regardless of cover state.
2. Render placeholder when no cover loaded.
3. Ensure animation doesn't affect outer dimensions.

**Files:** `gtm/src/ui.rs:1641-1778`

---

### A5. Seeking Crashes TUI

**Problem:** Seeking crashes the client.

**Root cause:** `seeking` flag on `DecodeControl` is never set to `true` anywhere in the codebase
(confirmed: zero `seeking.store` matches). The mixer busy-wait (`mixer.rs:467-472`) falls through
immediately. Ring buffer `flush()` races with the playback thread consumer.

**Spec:**
1. Set `seeking = true` in `signal_seek()` (`ring_buffer.rs:158`).
2. Clear `seeking = false` in decode thread after flush+restart (`decode_thread.rs:162-167`).
3. Clamp seek-forward position to `state.duration` in TUI (`app.rs:1501`).
4. Guard against `active_control` being `None` in `mixer.seek()`.

**Files:** `gtm-audio/src/ring_buffer.rs`, `gtm-audio/src/decode_thread.rs`, `gtm/src/app.rs`, `gtm-audio/src/mixer.rs`

---

### A6. Headphone/Speaker Presets Unreachable in EQ Picker

**Problem:** Cannot navigate to Headphones or Speaker in EQ picker.

**Root cause:** `clamp_overlay_selection()` hardcodes `OverlayId::Equalizer => 12` but presets
list has 15 items (indices 0–14). Index 13 (Headphones) and 14 (Speaker) unreachable.

**Spec:**
1. Fix max to `14` or use dynamic `.len() - 1`.
2. Centralize presets list as a const.

**Files:** `gtm/src/app.rs:1246`

---

### A7. Playback Speed Not Adjustable

**Problem:** Speed displayed but not changeable.

**Root cause:** `playback_speed` field is display-only. No IPC, no audio backend support.

**Spec:** Full-stack feature: trait method, IPC, state, decode-thread resampling, TUI controls.
(Deferred to separate feature branch due to complexity.)

**Files:** `gtm-audio/src/*`, `gtm-core/src/*`, `gtmd/src/daemon.rs`, `gtm/src/*`

---

### A8. Lyrics Pane in Small Terminals

**Problem:** Lyrics in narrow mode can't be dismissed with Esc.

**Spec:**
1. In narrow mode, Esc dismisses lyrics overlay.
2. Modify `Back` handler: if `is_narrow && show_lyrics`, clear `show_lyrics`.

**Files:** `gtm/src/app.rs:1594-1608`

---

### A9. Remove Quit from About Overlay

**Problem:** About overlay shows misleading `[q] Quit gtm` help text (q is actually a no-op).

**Spec:**
1. Change help bar to `" [Esc] Close"`.

**Files:** `gtm/src/ui.rs:1919`

---

### A10. Playback Not Auto-Advancing from List

**Problem:** Playing a track from a list doesn't auto-advance to next track.

**Root cause:** Likely queue not populated when playing from library list context.

**Spec:**
1. When playing from list, populate daemon queue with all visible tracks.
2. Set queue cursor to selected track index.

**Files:** `gtm/src/app.rs` (play handler), `gtmd/src/queue.rs`, `gtmd/src/daemon.rs`

---

## PART B: REFACTOR OVERLAYS (PICKERS)

### B1. Remove Widget Nesting

**Problem:** Command Palette list shrinks smaller than its surface.

**Root cause:** `palette_area` at `ui.rs:2238-2246` narrows to 50% width, capped at 60 cols.
Also EQ overlay uses `area.height` instead of `inner.height` for scroll.

**Spec:**
1. Remove `palette_area` narrowing in Command Palette — render Block on `area`.
2. Fix EQ scroll to use `inner.height`.

**Files:** `gtm/src/ui.rs:2205-2300, 2326`

---

### B2. Minimum Picker Width

**Problem:** Pickers are too narrow on small terminals.

**Spec:**
1. Add minimum width (50) and height (15) to overlay dispatcher.

**Files:** `gtm/src/ui.rs:1199-1200`

---

### B3. Fuzzy Finder for Theme Picker

**Problem:** Theme picker has no search.

**Spec:**
1. Add query input and subsequence filtering (same as Command Palette).
2. Add search line rendering.
3. Handle Char input for ThemePicker in overlay key handler.

**Files:** `gtm/src/ui.rs:2437-2486`, `gtm/src/app.rs`

---

### B4. Rename Overlays to Pickers

**Spec:** Mechanical rename: `OverlayId`→`PickerId`, `Overlay`→`Picker`, etc. Rename file.

**Files:** `gtm/src/overlay.rs` → `gtm/src/picker.rs`, all references.

---

## PART C: IMPROVED PRESETS

### C1. Retune EQ Presets with Headphone/Speaker Variants

**Problem:** Presets poorly tuned. Podcast/Vocal identical. Headphones/Speaker are standalone
instead of output-mode variants.

**Spec:**
1. Add `OutputMode { Speaker, Headphones }` to `DaemonState`.
2. Each preset defines dual gain arrays (speaker + headphone).
3. Headphone variant compensates for proximity effect / reduced spatial cues.
4. Speaker variant compensates for room acraftics.
5. Add output mode toggle in EQ picker.
6. Fix Vocal/Podcast duplicate.

**Files:** `gtm-core/src/state.rs`, `gtm-core/src/ipc.rs`, `gtm-audio/src/eq.rs`,
`gtm/src/app.rs`, `gtm/src/ui.rs`, `gtmd/src/daemon.rs`

---

## IMPLEMENTATION PHASES

### Phase 1 — Critical Bugs
- A5: Seek crash fix (data race)
- A6: EQ picker max selection fix
- A9: About overlay help text fix

### Phase 2 — UX Regressions
- A1: Startup stall (parallelize + splash)
- A2: CLI playback state (clock seeding)
- A10: Auto-advance from list
- A3: Visualizer width
- A4: Track popup layout shift
- A8: Narrow lyrics overlay

### Phase 3 — Overlay Refactor
- B1: Command Palette nesting fix
- B2: Minimum picker width
- B3: Theme picker fuzzy finder
- B4: Rename overlays→pickers

### Phase 4 — EQ Presets
- C1: Retune presets + dual variants + output mode
