# Spec 7: Queue clears after last track is played

## Problem

After the last track in the queue finishes playing (either naturally or via pressing
Next), the stale queue remains in the daemon state. When the user then selects a
different track from the library, the queue is replaced by `queue_set`, but the stale
state lingers for any intermediate operation (e.g., pressing PlayPause or viewing
the queue picker).

## Root Cause

`cmd_next()` in `gtmd/src/daemon.rs:1264-1337` handles the case where
`advance_queue(1)` returns `None` (end of queue, repeat off) by simply returning
`Ok(DaemonRes::Ok)`. It does NOT clear the queue or reset the state.

Similarly, `handle_audio_event()` for `AudioEvent::Finished` at line 972-1008 sends
an internal `DaemonReq::Next` which triggers `cmd_next()` — same behavior.

## Files to Modify

- `gtmd/src/daemon.rs`
- `gtm-core/src/ipc.rs` (optional: add QueueCleared event)
- `gtm-core/src/state_machine.rs`

## Implementation Steps

### 1. Clear queue on end-of-queue in cmd_next

In `gtmd/src/daemon.rs`, `cmd_next()`:

```rust
async fn cmd_next(inner: &DaemonInner) -> Result<DaemonRes, CoreError> {
    let mut state = inner.state.write().await;
    let track = match state.advance_queue(1)? {
        Some(t) => t.clone(),
        None => {
            // End of queue — stop playback, clear queue
            let was_playing = state.status == PlaybackStatus::Playing;
            drop(state);

            if was_playing {
                inner.mixer.lock().await.stop()?;
            }

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

### 2. Handle TrackEnded for same behavior from AudioEvent::Finished

In `gtmd/src/daemon.rs`, `handle_audio_event()` at line 972:

The `AudioEvent::Finished` handler already sends `DaemonReq::Next` via
`inner.internal_req_tx`, which will go through `cmd_next()`. With the above change,
it will automatically clear the queue. No additional change needed here.

### 3. Update client state machine

In `gtm-core/src/state_machine.rs`, `apply_event()`:

When `DaemonEvent::TrackEnded` is received:
```rust
DaemonEvent::TrackEnded => {
    self.current_track = None;
    self.status = PlaybackStatus::Stopped;
    self.time_pos = 0.0;
    self.queue.clear();
    self.queue_cursor = 0;
}
```

### 4. Update TUI state

In `gtm/src/app.rs`, when the `Queue` IPC result arrives with an empty queue:

```rust
IpcResult::Queue(tracks, cursor) => {
    self.queue_cache = tracks;
    self.queue_cursor = cursor;
    // If queue is empty and nothing is playing, reset browse_detail
    if self.queue_cache.is_empty() && self.state.current_track.is_none() {
        self.browse_detail = None;
    }
}
```

### 5. Queue picker display

In `gtm/src/ui.rs`, `render_queue_picker()`:

When the queue is empty, show "Queue is empty — add tracks from the Library"
instead of rendering an empty list.

## Verification

1. Queue several tracks (e.g., 3 tracks)
2. Play through them using Next
3. On the last track, press Next — playback should stop
4. Open the queue picker (`Ctrl+Q`) — should show "Queue is empty"
5. Go back to the library — should show "All Tracks" as the active view
6. Select and play a track — should work correctly with a fresh queue
