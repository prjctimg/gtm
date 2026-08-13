# Spec 10: Remove volume safety prompt, add master volume setting

## Problem

When the user changes the volume to high levels (>85%), a confirmation prompt appears
asking them to confirm. This is disruptive to the flow. The user wants to remove this
prompt and instead have a "master volume" setting that controls the maximum output
loudness, while still allowing the volume slider to go from 0-100%.

## Root Cause

The volume safety prompt exists in three places in `gtm/src/app.rs`:

1. **Command input mode** (~line 1527-1535): When a number >85 is entered in the
   command palette, it sets `self.pending_volume` instead of directly applying it.

2. **VolumeUp action** (~line 1693-1701): When `+` is pressed and volume exceeds 85,
   it sets `self.pending_volume` instead of directly applying it.

3. **Prompt handling** (~line 1546-1561): When `pending_volume` is set, Enter confirms
   and Esc cancels.

Additionally, the `pending_volume` field in the App struct (line 159) and its
initialization (line 328) exist solely for this purpose.

## Files to Modify

- `gtm/src/app.rs`
- `gtm-core/src/ipc.rs`
- `gtmd/src/daemon.rs`
- `gtmd/src/state.rs` (optional)
- `gtm/src/ui.rs`

## Implementation Steps

### 1. Remove pending_volume and prompt logic

In `gtm/src/app.rs`:

**Remove the field:**
```rust
// Delete: pub pending_volume: Option<u8>,
```

**Remove from constructor (~line 328):**
```rust
// Delete: pending_volume: None,
```

**Remove the prompt handling in Normal mode (~lines 1546-1561):**
```rust
// Delete the entire block:
// if self.pending_volume.is_some() {
//     match key.code { ... }
//     return true;
// }
```

**Remove the >85 check in Command mode (~lines 1527-1535):**
```rust
// Change from:
if let Ok(vol) = cmd.parse::<u8>() {
    if vol > 85 {
        self.pending_volume = Some(vol);
    } else {
        self.send_high(TuiCommand::SetVolume(vol));
    }
}
// To:
if let Ok(vol) = cmd.parse::<u8>() {
    self.send_high(TuiCommand::SetVolume(vol));
}
```

**Remove the >85 check in VolumeUp (~lines 1693-1701):**
```rust
// Change from:
Some(KeyboardAction::VolumeUp) => {
    let new_vol = (self.state.volume + 5).min(100);
    if new_vol > 85 {
        self.pending_volume = Some(new_vol);
    } else {
        self.send_high(TuiCommand::SetVolume(new_vol));
        self.notify(format!("Volume: {}%", new_vol), NotificationKind::Info);
    }
}
// To:
Some(KeyboardAction::VolumeUp) => {
    let new_vol = (self.state.volume + 5).min(100);
    self.send_high(TuiCommand::SetVolume(new_vol));
    self.notify(format!("Volume: {}%", new_vol), NotificationKind::Info);
}
```

### 2. Add master volume to daemon

In `gtm-core/src/ipc.rs`, add a new request variant:
```rust
SetMasterVolume { volume: u8 },
```

In `gtmd/src/daemon.rs`, add a handler:
```rust
DaemonReq::SetMasterVolume { volume } => {
    let vol = volume.min(100);
    inner.mixer.lock().await.set_master_volume(vol)?;
    let mut state = inner.state.write().await;
    state.master_volume = vol;
    drop(state);
    Self::push_event(inner, DaemonEvent::MasterVolumeChanged { volume: vol });
    Self::save_state(inner);
    Ok(DaemonRes::Ok)
}
```

In `gtm-audio/src/mixer.rs`, add master volume support:
```rust
pub fn set_master_volume(&mut self, volume: u8) -> AudioResult<()> {
    let vol = volume.min(100) as f32 / 100.0;
    self.master_volume.store(volume.min(100), Ordering::SeqCst);
    // Apply as gain multiplier to the active sink
    self.active().set_volume(self.volume.load(Ordering::SeqCst) as f32 / 100.0 * vol);
    Ok(())
}
```

### 3. Add master volume to daemon state

In `gtm-core/src/state.rs`:
```rust
pub master_volume: u8, // default: 100
```

### 4. Add master volume to TUI Settings

In `gtm/src/ui.rs`, Settings Audio category:
```rust
0 => vec![
    format!("Master Volume   [ {:>3}%  ]", app.state.master_volume), // NEW
    format!("Volume          [ {:>3}%  ]", app.state.volume),
    format!("Mute            [ {} ]", if app.state.mute { "●   On " } else { "○   Off" }),
],
```

In `gtm/src/app.rs`, settings options count:
```rust
0 => 3, // Audio: Master Volume, Volume, Mute (was 2)
```

Add handling for the Master Volume setting option:
```rust
// In the Select handler for Settings
0 => match opt {
    0 => {
        // Master Volume: cycle through increments or let user type
        let current = self.state.master_volume;
        let new_vol = if current >= 100 { 50 } else { (current + 10).min(100) };
        self.send_high(TuiCommand::SetMasterVolume(new_vol));
    }
    1 => {
        // Volume toggle or input
    }
    2 => {
        // Mute toggle
    }
},
```

### 5. Default behavior

- `master_volume` defaults to 100 — no change in behavior for existing users
- Users can lower master_volume to cap maximum loudness
- The volume slider still goes from 0-100% in the UI
- The actual output volume = `volume * (master_volume / 100)`
- No confirmation prompts for any volume level

## Verification

1. Start the TUI
2. Press `+` repeatedly — volume should increase without any prompt
3. Type `:95` in command palette — should set volume to 95% immediately
4. Go to Settings → Audio → Master Volume
5. Set Master Volume to 50
6. Set Volume to 100 — the actual loudness should be capped at 50% level
7. Verify the master volume is saved across daemon restarts
