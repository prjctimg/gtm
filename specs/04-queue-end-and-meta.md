# Spec 4: Queue end behavior, track meta, and cover image updates

## Problem

Three related issues:

**A.** Pressing Next on the last queue track should return the user to the default
     "All Tracks" view, but instead the stale queue remains and the TUI is stuck
     showing the old queue.

**B.** Track metadata (title, artist, album) is not correctly shown in the Now Playing
     tab when playing from the queue. Queue tracks show "Unknown Artist/Unknown Album".

**C.** The cover image is not updating when tracks change via the queue.

## Root Cause

**4A:** `cmd_next()` in `gtmd/src/daemon.rs:1264` calls `state.advance_queue(1)`.
When at the last track with `RepeatMode::Off`, `advance_queue()` returns `None`,
and `cmd_next()` just returns `Ok(DaemonRes::Ok)` without clearing the queue
or resetting state. The stale queue lingers.

**4B:** `cmd_play()` in `gtmd/src/daemon.rs:1070-1146` resolves track info using
`Library::track_by_path()`, but the fallback path (when test_mode or library fails)
produces bare `TrackInfo` with `artist: "Unknown Artist"` and `album: "Unknown Album"`.
Queue-set tracks (via `queue_set` → `resolve_track()` in `gtmd/src/queue.rs`) always
have `id: 0`, meaning library lookups fail to match.

**4C:** Cover art fetching in `gtm/src/app.rs:555-572` depends on `current_track.id`.
When queue tracks have `id: 0`, the daemon's `get_cover_art(tid)` can't find the track
in the library, so no cover is returned.

## Files to Modify

- `gtmd/src/daemon.rs`
- `gtmd/src/queue.rs`
- `gtm/src/app.rs`
- `gtm-core/src/ipc.rs` (maybe)

## Implementation Steps

### 4A: Clear queue on last-track-next

In `gtmd/src/daemon.rs`, `cmd_next()`:

When `advance_queue(1)` returns `None` (end of queue, no repeat):
1. Stop playback by calling the mixer's stop
2. Clear the queue using `queue_clear(state)`
3. Set `current_track = None` and `status = Stopped`
4. Push `DaemonEvent::TrackEnded` (or a new `QueueCleared` event)
5. Return `Ok(DaemonRes::Ok)`

```rust
async fn cmd_next(inner: &DaemonInner) -> Result<DaemonRes, CoreError> {
    let mut state = inner.state.write().await;
    let track = match state.advance_queue(1)? {
        Some(t) => t.clone(),
        None => {
            // No more tracks — stop and clear
            drop(state);
            inner.mixer.lock().await.stop()?;
            let mut state = inner.state.write().await;
            state.queue.clear();
            state.queue_cursor = 0;
            state.current_track = None;
            state.status = PlaybackStatus::Stopped;
            state.time_pos = 0.0;
            drop(state);
            Self::push_event(inner, DaemonEvent::TrackEnded);
            return Ok(DaemonRes::Ok);
        }
    };
    // ... rest of existing cmd_next logic
}
```

In `gtm/src/app.rs`, handle the `TrackEnded` event by clearing the queue cache and
resetting the queue cursor. This makes the TUI fall back to showing "All Tracks":

```rust
// In the event handler loop, where TrackEnded is detected:
DaemonEvent::TrackEnded => {
    self.queue_cache.clear();
    self.queue_cursor = 0;
    self.browse_detail = None; // Back to All Tracks view
}
```

### 4B: Fix track metadata for queued tracks

In `gtmd/src/daemon.rs`, `cmd_play()`:

After the existing library lookup (lines 1071-1099), if the lookup fails or the
track is not found in the library, try a more thorough lookup:

```rust
let track = if !inner.config.test_mode {
    let lib = Library::new(inner.config.data_dir.to_str().unwrap_or("")).ok();
    if let Some(ref lib) = lib {
        match lib.track_by_path(&path_owned) {
            Ok(Some(mut t)) => {
                t.duration = dur;
                t
            }
            _ => {
                // Fallback: try to find by path substring match
                match lib.tracks_by_path_prefix(&path_owned) {
                    Ok(mut tracks) if !tracks.is_empty() => {
                        let mut t = tracks.remove(0);
                        t.duration = dur;
                        t
                    }
                    _ => create_minimal_track(&path_owned, dur),
                }
            }
        }
    } else {
        create_minimal_track(&path_owned, dur)
    }
} else {
    create_minimal_track(&path_owned, dur)
};
```

Also, in `gtmd/src/queue.rs`, `resolve_track()`:
- After creating the minimal `TrackInfo`, try to look up the track in the library
  to fill in the metadata fields
- This requires adding a `Library` reference to `resolve_track` or moving the
  resolution to the call site in `daemon.rs`

Alternatively, modify `library.rs` to add a `tracks_by_path_prefix` method.

### 4C: Fix cover image for queue tracks

In `gtm/src/app.rs`, the cover art fetch condition at line 555-572 already triggers
on `current_track.id` changes. The fix is ensuring `current_track.id` is valid (non-zero).

Once 4B is fixed (track IDs are properly resolved), cover art fetching will work
naturally because the daemon's `get_cover_art(tid)` will receive a valid library ID.

Additionally, in `gtmd/src/daemon.rs`, the `GetCoverArt` handler should fall back to
looking up the artist/album from the current track's metadata if the track ID is 0:

```rust
DaemonReq::GetCoverArt { track_id } => {
    let (artist, album) = {
        let state = inner.state.read().await;
        let track = if track_id > 0 {
            // Look up by ID in library
            None // TODO: library lookup
        } else {
            state.current_track.clone()
        };
        match track {
            Some(t) => (t.artist, t.album),
            None => (String::new(), String::new()),
        }
    };
    // Use artist/album for cover lookup instead of ID
    // ...
}
```

## Verification

1. Play a track from "All Tracks" — verify correct metadata shows in Now Playing
2. Verify cover art displays correctly
3. Queue several tracks and play through them
4. On the last track, press Next — should stop playback and show "All Tracks" view
5. Start playback from a different list — verify queue is properly replaced
