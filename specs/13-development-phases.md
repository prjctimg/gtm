# 13 — Development Phases

## Phase 0 — Foundation (Week 1)

**Goal:** gtm-core types are complete, validated, serializable, tested, and instrumented with debug-only state invariants and Tripwire-style fail points.

| Step | Files | Deliverable |
|------|-------|-------------|
| 0.1 | `gtm-core/src/validate.rs` | `CrossfadeConfig::new()` with bounds validation; `DaemonState::new()` default state; `TrackInfo::is_valid()`, `duration_formatted()` helpers |
| 0.2 | `gtm-core/src/state_machine.rs` | `DaemonState` mutation methods: `play()`, `pause()`, `stop()`, `seek()`, `set_volume()`, `toggle_shuffle()`, `cycle_repeat()`, `toggle_mute()`, `set_crossfade()`, `advance_queue()` — each enforces valid state transitions, auto-increments `version` |
| 0.3 | `gtm-core/src/state_machine.rs` | `DaemonState::apply_event(&mut self, event: &DaemonEvent)` — state mirror matching spec §06 state mirror table |
| 0.4 | `gtm-core/src/state_machine.rs` | `DaemonState::check_invariants(&self)` — guarded by `debug_assert!` / `#[cfg(debug_assertions)]`, called at end of every mutating method. Checks: volume ∈ [0,100], queue_cursor < queue.len(), time_pos ≤ duration, Playing ⇒ current_track.is_some(), crossfade enabled ⇒ duration > 0 |
| 0.5 | `gtm-core/src/tripwire.rs` + `Cargo.toml` | `[features] debug-fail = []` — Tripwire-style fail point injection. `FailPoint` enum with `SerializeEvent`, `DeserializeFrame`, `StateTransition`, `QueueAdvance`, `VolumeChange`, `CrossfadeApply`. `fn check(fp) -> Result<()>` is a no-op in release, thunkable to `Err` in tests. Inline `#[inline(always)]` so optimizer strips entirely without the feature. |
| 0.6 | `gtm-core/src/state_machine.rs` | Wire `check()` calls into `tripwire::check()` at each state transition point for testability of error paths |
| 0.7 | `gtm-core/tests/core_tests.rs` | **Serde round-trips:** every type (`TrackInfo`, `Playlist`, `LrcLine`, `LrcData`, `YTSearchResult`, `StreamInfo`, `CrossfadeConfig`, `DaemonState`, `Image`) with both `serde_json` and `bincode`. **IPC enum round-trips:** every variant of `DaemonReq`, `DaemonRes`, `DaemonEvent`, `LibraryAction`, `QueueAction` — verify JSON tag structure, bincode identity. **Wire protocol:** `encode_frame` / `decode_frame` with 0, 1, N events; partial buffer yields `None`; corrupted data yields error. **State transitions:** valid: Stopped→Playing→Paused→Playing→Stopped; invalid: Stopped→Pause, Playing→Play, etc. **Validation:** out-of-range volume, crossfade duration, seek position. **Invariants:** `check_invariants()` passes on valid state, panics on deliberately broken state in debug. **Fail points:** each `FailPoint` armed and triggered. **Error paths:** malformed JSON, truncated bincode, empty frames, unknown enum tags. |
| 0.8 | `lib.rs` | Re-export: `pub use state_machine::*`, `pub use validate::*`, `pub use tripwire::*` |

```
Phase 0 Checklist:
[ ] cargo check --workspace passes
[ ] gtm-core compiles with all new modules + features
[ ] all serde round-trip tests pass (all types, both formats)
[ ] all IPC enum round-trip tests pass (every variant)
[ ] encode_frame / decode_frame tests pass
[ ] DaemonState::check_invariants fires on broken state, passes on valid
[ ] tripwire fail points work under `--features debug-fail`
[ ] cargo test --workspace passes
[ ] cargo clippy --workspace is clean
```

## Cargo.toml changes

```toml
[features]
debug-fail = []

[dependencies]
tracing = { version = "0.1", optional = true }
```

`tracing` is optional, used only by fail-point diagnostics when `debug-fail` is enabled.
