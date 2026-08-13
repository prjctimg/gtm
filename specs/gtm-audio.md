# gtm-audio Revision Spec

## Phase C: Audio Crate Cleanup

### C1. Delete RodioBackend and AudioBackend trait
**Files to delete**: `gtm-audio/src/rodio.rs`, `gtm-audio/src/backend.rs`
**Lines removed**: ~282

Rationale: `RodioBackend` is never used in production. The daemon uses `AudioMixer`
via the `Mixer` trait directly. `RodioBackend` duplicates the same rodio Player/Sink
logic that `AudioMixer` already implements with crossfade, EQ, and ring buffer support.

Changes:
- Remove `pub mod rodio;` and `pub mod backend;` from `lib.rs`
- Remove `pub use backend::{AudioBackend, AudioError, AudioEvent, AudioResult};`
- Remove `async-trait` dependency from `Cargo.toml`
- Keep `AudioEvent` and `AudioError` (used by `AudioMixer`) — move to `mixer.rs` or keep in a reduced `backend.rs`
- Delete `gtm-audio/tests/playback_test.rs` and `opus_playback_test.rs` if they reference RodioBackend

### C2. Extract CrossfadeState from AudioMixer
**File**: `gtm-audio/src/mixer.rs`

Current `AudioMixer` has 25+ fields. Extract crossfade-related fields:
```rust
struct CrossfadeState {
    start: Option<Instant>,
    duration: f64,
    easing: Easing,
    pending_pause: bool,
    pause_fade_start: Option<Instant>,
    stored_volume: u8,
}
```

This reduces `AudioMixer` field count and makes crossfade logic self-contained.

### C3. Keep AudioEvent and AudioError
These types are used by the daemon's `handle_audio_event()`. Move them
to `mixer.rs` or a minimal `types.rs` instead of deleting `backend.rs` entirely.
Actually: simplest to keep `backend.rs` but strip it to just `AudioEvent` + `AudioError`
(removing the `AudioBackend` trait).
