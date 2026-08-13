# Phase 3 — Protocol v2 Feature Commands and Events

**Goal**: Implement the v2 feature surface so the daemon advertises compliance with the full v2 command set, even if some handlers are initially stubs.

## Source of truth
- `prjctimg/gtm.spec` `commands.md` § "Loudness Compensation", § "Gapless Playback", § "Dynamic Mode", § "Scrobbling", § "Library Organization"
- `prjctimg/gtm.spec` `events.md` corresponding events
- `prjctimg/gtm.spec` `state.md` § loudness, gapless, dynamic mode
- `prjctimg/gtm.spec` `fixes/gtm-rs.md` item "Protocol v2 features (loudness, gapless, etc.) ❌ Not yet implemented"

## Acceptance criteria

### 3.1 New DaemonReq variants
- File: `gtm-core/src/ipc.rs` `enum DaemonReq` (line ~247)
- Add (untagged, from Phase 1):
  - `SetLoudnessMode { mode: LoudnessMode }` (cmd: `set_loudness_mode`)
  - `ScanLoudness { track_ids: Option<Vec<i64>>, force: Option<bool> }` (cmd: `scan_loudness`)
  - `SetPreGain { pre_gain_db: f32 }` (cmd: `set_pre_gain`)
  - `SetGapless { enabled: bool }` (cmd: `set_gapless`)
  - `SetDynamicMode { enabled: bool, min_queue_remaining: Option<u32>, max_history: Option<u32> }` (cmd: `set_dynamic_mode`)
  - `SetScrobble { enabled: bool, api_key: Option<String>, session_token: Option<String>, min_play_secs: Option<u32>, min_play_pct: Option<f32> }` (cmd: `set_scrobble`)
  - `OrganizeLibrary { dry_run: Option<bool> }` (cmd: `organize_library`)
- Wire each to the spec `cmd` string in the client's `send_request_by_id` match arm (~`gtm-core/src/client.rs:755-823`).

### 3.2 New DaemonEvent variants
- File: `gtm-core/src/ipc.rs` `enum DaemonEvent` (line ~111)
- Add with `#[serde(rename = "...")]`:
  - `LoudnessModeChanged { mode: LoudnessMode }` → `loudness_mode_changed`
  - `LoudnessScanProgress { scanned: usize, total: usize }` → `loudness_scan_progress`
  - `LoudnessScanDone { scanned: usize }` → `loudness_scan_done`
  - `PreGainChanged { pre_gain_db: f32 }` → `pre_gain_changed`
  - `GaplessChanged { enabled: bool }` → `gapless_changed`
  - `DynamicModeChanged { enabled: bool, min_queue_remaining: u32, max_history: u32 }` → `dynamic_mode_changed`
  - `ScrobbleConfigChanged { enabled: bool }` → `scrobble_config_changed`
  - `LibraryOrganized { moves: usize }` → `library_organized`
- Add a `to_wire_event` arm for each (`ipc.rs:161-242`).

### 3.3 New state types
- File: `gtm-core/src/state.rs`
- `pub enum LoudnessMode { Off, Track, Album, Auto }` with `#[serde(rename_all = "snake_case")]`.
- Add fields to `DaemonState`:
  - `loudness_mode: LoudnessMode` (default `Off`)
  - `pre_gain_db: f32` (default 0.0)
  - `gapless: bool` (default false)
  - `dynamic_mode: DynamicModeConfig` (default disabled, `min_queue_remaining: 3`, `max_history: 50`)
  - `scrobble: ScrobbleConfig` (default disabled)
- `SavedState` (`state.rs`) must persist new fields so they survive restart (see existing `SavedState::from_state` / `apply_to`).

### 3.4 Daemon command handlers
- File: `gtmd/src/daemon.rs` `handle_request` (~line 580)
- Add match arms that:
  - Update the corresponding `DaemonState` field under `state.write().await`.
  - Emit the corresponding `DaemonEvent` via `inner.event_tx.send(...)`.
  - Return `DaemonRes::Ok { version: PROTOCOL_VERSION }` (or appropriate typed response).
- For `OrganizeLibrary { dry_run }`: if `dry_run == Some(true)` (default), return `DaemonRes::Ok` with `{"moves":[]}` (no actual file moves). When `dry_run == Some(false)`, log a warning that destructive organize is not yet implemented and return an error (or stub the moves list as empty). The spec requires a `library_organized` event only when not dry_run.
- For `ScanLoudness`: spawn an async task that emits `LoudnessScanProgress` periodically and `LoudnessScanDone` at the end. Loudness analysis itself can be a no-op (all tracks treated as 0 LUFS) for now — the spec requires the command/event surface to exist; actual loudness measurement is an implementation detail.

### 3.5 Remove non-spec `SetCrossfadeEasing` variant
- File: `gtm-core/src/ipc.rs`
- Delete `SetCrossfadeEasing` variant (line ~264).
- Fold `easing: Easing` into `Crossfade { enabled, duration_secs, easing }`.
- Update `gtmd/src/daemon.rs` match arm and `cmd_crossfade` handler signature.
- Update `gtm-core/src/client.rs` `send_request_by_id` mapping (remove the duplicate `"crossfade"` mapping at line ~770; emit single mapping with `easing` field).
- The spec `commands.md` § `crossfade` lists `easing` as an optional param of the `crossfade` command, so this aligns.

## Verification
- `cargo build --workspace`
- `cargo test --workspace`
- New unit test in `gtm-core/tests/core_tests.rs` ensuring each new command serializes with the correct `cmd` string and untagged params.
- New integration test in `gtmd/tests/daemon_test.rs` exercising each new command via raw spec-shaped JSON and asserting `ok:true` + corresponding event.

## Commit message
`feat(spec): phase 3 — v2 commands and events (loudness, gapless, dynamic, scrobble, organize)`