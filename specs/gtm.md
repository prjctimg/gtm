# gtm (Client) Revision Spec

## Phase E: Client Cleanup

### E1. Fix connection retry to exponential backoff
**File**: `gtm-core/src/client.rs`

Current (linear):
```rust
let delay = (50 * (i + 1)).min(500);
```

Required (exponential per spec client.md):
```rust
let delay = (100u64 * 2u64.pow(i as u32)).min(5000);
```

Sequence: 100ms, 200ms, 400ms, 800ms, 1600ms, 3200ms, 5000ms, 5000ms, 5000ms, 5000ms

### E2. Reduce theme presets
**File**: `gtm/src/theme.rs`

Keep 5 core themes: Default, TokyoNight, Catppuccin, Dracula, Nord.
Remaining themes can be moved to a user-configurable format or behind a feature flag.

This is a lower priority change and can be deferred if it risks TUI breakage.
