# Spec 09 — Bugs

Status: **Planned** — fix prev/next crossfade bug, raw filename-as-title bug, master volume bug.

Green gate: `cargo fmt --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`

---

## 9.1 — Crossfade + prev/next bug (more than 2 tracks)

**Bug**: When pressing prev/next whilst there's more than two tracks crossfading, the command replays the same track that had been loaded into the main decoding channel. Can't have more than two crossfading tracks and continuous prev/next is broken.

**Root cause**: The crossfade state (`crossfade_loaded_for`) is never cleared when a new prev/next operation occurs, causing the crossfade system to get confused.

**Fix location**: `gtmd/src/daemon.rs:1238-1270` (`try_start_crossfade`)

### 9.1.1 — Fix details
- When `cmd_next` or `cmd_prev` is called, check if `crossfade_loaded_for` is set
- If set, clear it before starting a new crossfade attempt
- Add a maximum limit on crossfade depth (2 active crossfades) — if already 2 tracks are crossfading, cancel the oldest crossfade and start fresh
- The fix should be in `cmd_next` and `cmd_prev` handler in `daemon.rs` (lines 1685-1756)

### 9.1.2 — Code change in daemon.rs
In `cmd_next` (line 1685) and `cmd_prev` (line 1719), before calling `try_start_crossfade`, check and clear `crossfade_loaded_for`:
```rust
// Clear crossfade state before starting new crossfade
inner.crossfade_loaded_for.lock().await = None;
```

### 9.1.3 — Also fix in step_crossfade (mixer.rs:687-748)
When `step_crossfade` finishes and sets `is_a_active = !self.is_a_active`, it should also clear the `crossfade_loaded_for` state to allow the next prev/next to work.

---

## 9.2 — Raw filename used as track title

**Bug**: Trace any references to the raw filename being used as the track title and fix it.

**Root cause**: In `gtmd/src/queue.rs` and `gtmd/src/daemon.rs`, `resolve_track` and `resolve_track_meta` construct `TrackInfo` with `file_stem()` as the title when no library lookup succeeds.

**Fix location**: `gtmd/src/queue.rs:22-45` (`resolve_track`) and `gtmd/src/daemon.rs:1286-1320` (`resolve_track_meta`)

### 9.2.1 — Fix details
In `resolve_track` (queue.rs):
- Instead of using `path.file_stem().unwrap_or("Unknown")` as title, use the library's track metadata title
- If library lookup fails, fall back to `filename_stem` but also include artist/album metadata

In `resolve_track_meta` (daemon.rs):
- Line 1290: `file_stem()` is used as title — replace with `track.title` from library lookup
- If library lookup fails, the `Unknown Artist` / `Unknown Album` fallback is used, which is correct

### 9.2.2 — Changes
- `resolve_track` in `gtmd/src/queue.rs` should use `track.title` (from library) instead of `file_stem`
- `resolve_track_meta` in `gtmd/src/daemon.rs` should call `resolve_track` first to get the full metadata

---

## 9.3 — Master volume feature is broken

**Bug**: The master volume feature is broken.

**Root cause**: In `gtm-audio/src/mixer.rs`, the `master_volume` interacts with `volume` in a way that causes the master volume to attenuate incorrectly.

**Fix location**: `gtm-audio/src/mixer.rs:396-399` (load_active) and `gtm-audio/src/mixer.rs:440-443` (load_active_decoded)

### 9.3.1 — Fix details
- In `load_active` (line 396), the current volume is multiplied by master volume: `vol * (self.master_volume / 100.0)`
- The same issue exists in `load_active_decoded` (line 440-443)
- The fix should ensure master volume properly affects the active track volume

### 9.3.2 — Alternative: deprecate master volume
- If the fix cannot be resolved cleanly, deprecate master volume
- The master_volume field exists in the mixer struct but is not used in a correct way
- Add a deprecation note to the `master_volume` interface

---

## 9.4 — Verification

All three bugs should be tested:
1. Crossfade + prev/next with 3+ tracks: continuous prev/next should work
2. Track title should never be raw filename stem — always use library metadata
3. Master volume: playing a track at non-100% volume should produce correct output with master volume applied
