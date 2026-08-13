# Spec 6: Playback commands throw errors, seek doesn't work

## Problem

Playback commands (play, pause, next, prev) work functionally but error notifications
appear in the TUI. The seek command specifically does not work at all — pressing `,`/`.`
does nothing or causes errors.

## Root Cause (Seek)

`cmd_seek()` in `gtmd/src/daemon.rs:1423` calls `mixer.seek(pos)` which in
`gtm-audio/src/mixer.rs:497` does a **blocking spin-loop**:

```rust
pub fn seek(&mut self, position_secs: f64) -> AudioResult<()> {
    let Some(ref ctrl) = self.active_control else {
        return Ok(());
    };
    ctrl.signal_seek(position_secs);
    let deadline = Instant::now() + Duration::from_secs(2);
    while ctrl.seeking.load(Ordering::Acquire) && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(5)); // Blocks the async runtime!
    }
    // ...
}
```

This blocks the tokio async task, preventing the daemon from processing other events
and potentially causing timeouts elsewhere. Additionally, `signal_seek()` on rodio's
`Sink` may not work correctly for all decoded audio formats (e.g., symphonia sources
may not support seeking properly).

## Root Cause (Playback Errors)

The error likely comes from `error_handler` callbacks in `gtm/src/app.rs` at lines
955-961. The `tokio::spawn` tasks for `client.next()`, `client.prev()`, etc. report
errors through `IpcResult::Error`. The daemon's `cmd_next`/`cmd_prev` may be returning
errors for edge cases not properly handled.

Check `gtmd/src/daemon.rs` around line 1266 — when `advance_queue` returns `None`,
`cmd_next` returns `Ok(DaemonRes::Ok)` which shouldn't cause an error. But the client
side might interpret certain responses as errors.

## Files to Modify

- `gtm-audio/src/mixer.rs`
- `gtmd/src/daemon.rs`
- `gtm-core/src/client.rs` (add debug logging)

## Log Path

The daemon writes debug logs to: `~/.local/share/gtm/gtmd.log`
(or `$XDG_DATA_HOME/gtm/gtmd.log`)

## Implementation Steps

### 1. Fix seek: make it async-aware

In `gtm-audio/src/mixer.rs`, replace the blocking spin-loop in `seek()`:

**Option A: Non-blocking seek with timeout**
```rust
pub fn seek(&mut self, position_secs: f64) -> AudioResult<()> {
    let Some(ref ctrl) = self.active_control else {
        return Ok(());
    };
    ctrl.signal_seek(position_secs);
    // Don't block — just update internal position and trust the seek signal
    *self.position.lock().unwrap() = position_secs;
    *self.start_time.lock().unwrap() = Some(Instant::now());
    *self.start_pos.lock().unwrap() = position_secs;
    Ok(())
}
```

**Option B: Move seeking to a background thread**
- Spawn a dedicated blocking task for the seek spin-loop
- Use a channel to signal completion

### 2. Add debug logging to seek path

In `gtmd/src/daemon.rs`, `cmd_seek()`:

```rust
async fn cmd_seek(inner: &DaemonInner, pos: f64) -> Result<DaemonRes, CoreError> {
    let state = inner.state.read().await;
    if state.status == PlaybackStatus::Stopped {
        return Err(CoreError::Daemon("cannot seek while stopped".into()));
    }
    drop(state);
    tracing::debug!("cmd_seek: requested position={}", pos);
    let actual = {
        let mut mixer = inner.mixer.lock().await;
        mixer.seek(pos)?;
        let current = mixer.current_position();
        tracing::debug!("cmd_seek: actual position after seek={}", current);
        current
    };
    let mut state = inner.state.write().await;
    state.seek(actual)?;
    drop(state);
    Self::push_event(inner, DaemonEvent::PositionChanged { time_pos: actual });
    Ok(DaemonRes::Ok)
}
```

### 3. Debug playback command errors

In `gtm/src/app.rs`, the `error_handler` closures at lines 955-961:

```rust
let error_handler = move |e: gtm_core::CoreError| {
    eprintln!("[debug] IPC command error: {e:?}"); // Temporarily add stderr logging
    let _ = err_tx.send(IpcResult::Error(e.to_string()));
};
```

Check `gtmd/src/daemon.rs` for the actual error source:
- `cmd_play` — does it fail to decode the file?
- `cmd_next` / `cmd_prev` — does `advance_queue` fail?
- Add `tracing::debug!` to each command handler

### 4. Check client IPC methods

In `gtm-core/src/client.rs`, verify `seek()` sends the correct IPC message:

```rust
pub async fn seek(&self, position_secs: f64) -> Result<()> {
    // Verify the request format matches DaemonReq::Seek
    self.send(DaemonReq::Seek { position_secs }).await?;
    let res = self.recv().await?;
    // Check the response type
    match res {
        DaemonRes::Ok => Ok(()),
        DaemonRes::Error { message } => Err(CoreError::Daemon(message)),
        _ => Err(CoreError::Daemon(format!("unexpected response: {:?}", res))),
    }
}
```

### 5. Check for symphonica seek support

In `gtm-audio/src/symphonia.rs`, verify that the decoded source supports seeking.
If not, the `ctrl.signal_seek()` call may be silently ignored, and the mixer's
internal position update (Option A) would be sufficient for the display to reflect
the seek even if the audio doesn't actually jump.

## Verification

1. Start the daemon with `RUST_LOG=debug gtmd` (or check `~/.local/share/gtm/gtmd.log`)
2. Start the TUI and play a track
3. Press `.` (seek forward) — verify position changes
4. Press `,` (seek backward) — verify position changes
5. Check the debug log for seek-related messages
6. Verify no error notifications appear for any playback commands
7. If seek still doesn't actually jump the audio position, implement Option B
   (background thread for rodio seek)
