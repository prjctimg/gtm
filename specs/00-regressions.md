# 00 — Regressions & Bug Fixes

## Goal

Fix critical regressions that prevent proper operation of the TUI, playback, and daemon.

## Bug 1: Seeking Not Working

**Current State:** Seeking (both directions) only moves the track progress indicator on the TUI but does not actually seek in the audio.

**Root Cause:** The seek command is likely not being sent to the daemon, or the daemon is not applying the seek to the audio backend.

**Required Changes:**

1. Verify IPC command flow for seek operations in `gtm/src/app.rs`
2. Check daemon handling of `Seek` command in `gtmd/src/daemon.rs`
3. Ensure `backend.seek(pos)` is called correctly in `gtmd/src/daemon.rs`
4. Verify the seek position is being communicated back to the TUI via events

**Files to Modify:**
- `gtm/src/app.rs` — Ensure seek commands are dispatched correctly
- `gtmd/src/daemon.rs` — Verify `cmd_seek` implementation
- `gtm-core/src/ipc.rs` — Verify Seek command serialization

**Checklist:**
- [ ] Forward seek moves audio position forward
- [ ] Backward seek moves audio position backward
- [ ] Progress indicator reflects actual audio position after seek
- [ ] Seeking works from both Now Playing and Library tabs

---

## Bug 2: TUI Crashes on Small Terminal

**Current State:** When resizing the terminal to less than ~30 columns, the TUI quits unexpectedly. This also happens randomly after playback stops.

**Root Cause:** Likely a panic in the rendering code when terminal dimensions are too small for the layout, or an unwrap() on a None terminal size.

**Required Changes:**

1. Add terminal size validation before rendering
2. Implement minimum terminal size handling (show message if too small)
3. Add error handling for terminal size queries
4. Fix any unwrap() calls related to terminal dimensions

**Files to Modify:**
- `gtm/src/ui.rs` — Add size checks before rendering
- `gtm/src/main.rs` — Handle terminal resize events gracefully
- `gtm/src/app.rs` — Add minimum size validation

**Checklist:**
- [ ] TUI does not crash when terminal is resized below 30 columns
- [ ] TUI shows a message when terminal is too small
- [ ] TUI recovers when terminal is resized back to normal
- [ ] No crashes after playback stops

---

## Bug 3: Playback Stops After 2 Tracks

**Current State:** When starting playback in a list, only two tracks play consecutively then playback stops.

**Root Cause:** The auto-advance logic may not be triggering correctly, or the queue cursor is not advancing properly.

**Required Changes:**

1. Verify queue advancement logic in `gtmd/src/daemon.rs`
2. Check `handle_audio_event` for track completion handling
3. Verify queue cursor updates in `gtmd/src/queue.rs`
4. Ensure `PlaybackEnded` event triggers next track

**Files to Modify:**
- `gtmd/src/daemon.rs` — Fix track advancement logic
- `gtmd/src/queue.rs` — Verify queue cursor management
- `gtm-core/src/state.rs` — Check state transitions

**Checklist:**
- [ ] Playback continues beyond 2 tracks
- [ ] Queue cursor advances correctly
- [ ] Repeat modes work correctly
- [ ] Shuffle mode works correctly

---

## Bug 4: Missing Track Duration

**Current State:** Some tracks in the Library do not show duration.

**Root Cause:** Metadata extraction may be failing for certain audio formats, or the duration field is not being populated.

**Required Changes:**

1. Review metadata extraction in `gtmd/src/library.rs`
2. Ensure all supported formats are handled
3. Add fallback duration extraction methods
4. Verify duration is stored correctly in SQLite

**Files to Modify:**
- `gtmd/src/library.rs` — Fix `extract_metadata` function
- `gtm-audio/src/symphonia.rs` — Verify duration extraction
- `gtmd/src/library.rs` — Check SQLite schema for duration field

**Checklist:**
- [ ] All audio formats show duration
- [ ] Duration is accurate for all tracks
- [ ] Library scan populates duration correctly
- [ ] Existing tracks with missing duration are updated on rescan

---

## Bug 5: Thumbnail Embedding Issue

**Current State:** Thumbnails are being embedded into tracks when downloading.

**Root Cause:** The download process is embedding metadata/thumbnails into the audio file itself.

**Required Changes:**

1. Modify download process to NOT embed thumbnails
2. Use Deezer API for cover art instead
3. Store cover art references separately

**Files to Modify:**
- `gtmd/src/cover_art.rs` — Implement Deezer API integration
- `gtmd/src/youtube.rs` — Remove thumbnail embedding from download
- `gtmd/src/library.rs` — Store cover art path separately

**Checklist:**
- [ ] Downloaded tracks do not have embedded thumbnails
- [ ] Cover art is fetched from Deezer API
- [ ] Cover art is displayed correctly in TUI
- [ ] Cover art cache works efficiently

---

## Bug 6: Now Playing State Not Synced on Track Change

**Current State:** Elapsed time and track duration are not updated when automatically advancing to the next track. The elapsed time continues incrementing as if it's still the same track.

**Root Cause:** The Now Playing state is not being properly reset/updated when a new track starts playing.

**Required Changes:**

1. Review how `DaemonEvent::PlaybackStarted` is handled
2. Ensure `time_pos` is reset to 0 on new track
3. Ensure `duration` is updated from new track metadata
4. Verify state sync mechanism in TUI

**Files to Modify:**
- `gtmd/src/daemon.rs` — Fix state updates on track change
- `gtm/src/app.rs` — Verify event handling for track changes
- `gtm-core/src/state.rs` — Check state mutation logic

**Event → State Mapping (reference):**
```
PlaybackStarted { track, time_pos, duration } → 
  status = Playing
  current_track = track
  time_pos = time_pos (should be 0)
  duration = duration (from new track)
```

**Checklist:**
- [ ] Elapsed time resets to 0 on new track
- [ ] Duration updates to new track's duration
- [ ] Track metadata updates correctly
- [ ] Progress indicator reflects new track position

---

## Bug 7: Lyrics Pane Not Showing Current Track

**Current State:** When triggered from the left pane, the lyrics pane should show lyrics/transcript of the current track but doesn't.

**Root Cause:** The lyrics pane may not be fetching lyrics for the currently playing track, or the lyrics data is not being passed correctly.

**Required Changes:**

1. Verify lyrics fetch is triggered for current track
2. Check lyrics data flow from daemon to TUI
3. Ensure lyrics pane updates on track change

**Files to Modify:**
- `gtm/src/overlay.rs` — Fix lyrics overlay data binding
- `gtmd/src/lyrics.rs` — Verify lyrics fetching
- `gtm-core/src/ipc.rs` — Check lyrics IPC command

**Checklist:**
- [ ] Lyrics pane shows lyrics for current track
- [ ] Lyrics update when track changes
- [ ] Lyrics pane works for tracks with available lyrics
- [ ] Graceful handling when lyrics are not available

---

## Implementation Order

1. Bug 6 (Now Playing State) — Most visible to users
2. Bug 3 (Playback Stops) — Critical for basic functionality
3. Bug 1 (Seeking) — Important for user control
4. Bug 2 (TUI Crashes) — Stability issue
5. Bug 4 (Missing Duration) — Data completeness
6. Bug 7 (Lyrics Pane) — Feature completeness
7. Bug 5 (Thumbnail) — Download behavior

## Verification

After implementing all fixes:
- [ ] `cargo check --workspace` passes
- [ ] `cargo test --workspace` passes
- [ ] Manual testing of all playback scenarios
- [ ] Verify no regressions in existing functionality