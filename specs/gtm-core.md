# gtm-core Revision Spec

## Spec Compliance Fixes (Phase A)

### A1. LoudnessScanProgress event fields
**File**: `gtm-core/src/ipc.rs`

Current:
```rust
LoudnessScanProgress { scanned: usize, total: usize }
```

Required (per spec events.md):
```rust
LoudnessScanProgress {
    tracks_remaining: u32,
    tracks_total: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    current_track: Option<TrackInfo>,
}
```

### A2. LoudnessScanDone event fields
**File**: `gtm-core/src/ipc.rs`

Current:
```rust
LoudnessScanDone { scanned: usize }
```

Required:
```rust
LoudnessScanDone { scanned: u32, failed: u32 }
```

### A3. LibraryOrganized event fields
**File**: `gtm-core/src/ipc.rs`

Current:
```rust
LibraryOrganized { moves: usize }
```

Required:
```rust
LibraryOrganized { moves_succeeded: u32, moves_failed: u32 }
```

### A4. DynamicModeConfig missing cooldown_weight
**File**: `gtm-core/src/state.rs`

Add field:
```rust
pub struct DynamicModeConfig {
    pub enabled: bool,
    pub min_queue_remaining: u32,
    pub max_history: u32,
    pub cooldown_weight: f32, // NEW: default 0.1
}
```

### A5. TrackInfo missing spec fields
**File**: `gtm-core/src/track.rs`

Add fields:
```rust
pub actual_duration: Option<f64>,
pub loudness_lufs: Option<f32>,
pub loudness_peak_db: Option<f32>,
pub loudness_range: Option<f32>,
```

### A6. Easing alignment with spec
Keep all 7 variants (implementation-specific extensions are fine).
The spec only mandates 3: `equal_power`, `linear`, `exponential`.
Our extras (`SlowFadeInFastFadeOut`, `FastFadeInSlowFadeOut`, `Logarithmic`, `Smoothstep`) are extensions.

## Dead Code & Duplication Removal (Phase B)

### B1. Delete to_wire_event()
**File**: `gtm-core/src/ipc.rs:774-911`
137-line method never called. DaemonEvent serializes directly via #[serde(tag)].

### B2. Remove version from DaemonRes variants
**File**: `gtm-core/src/ipc.rs`
Every variant carries `version: u32` which is always `PROTOCOL_VERSION`.
Remove from all variants, use constant directly in `to_wire()`.

### B3. Delete default_socket_path() alias
**File**: `gtm-core/src/paths.rs:91-101`
Trivial wrapper around `resolve_command_socket()`.

### B4. Fix cmd_name() dead branch
**File**: `gtm-core/src/ipc.rs:338-342`
Crossfade arm: `if easing.is_some() { "crossfade" } else { "crossfade" }` -> just `"crossfade"`

### B5. Change u128 queue indices to u64
**Files**: state.rs, ipc.rs, client.rs
Spec defines `uint64` for queue indices.

### B6. TrackInfo::from_path() helper
**File**: `gtm-core/src/validate.rs`
Extract the duplicated TrackInfo construction pattern used 6x in daemon.rs.
