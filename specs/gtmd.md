# gtmd (Daemon) Revision Spec

## Phase D: Daemon Cleanup

### D1. Extract TrackInfo::from_path() helper
**File**: `gtm-core/src/validate.rs`

Current: TrackInfo is constructed identically 6 times in daemon.rs with:
```rust
gtm_core::track::TrackInfo {
    id: 0,
    path: path_owned.clone(),
    title: Path::new(&path_owned).file_stem()...unwrap_or("Unknown")...to_string(),
    artist: "Unknown Artist".to_string(),
    album: "Unknown Album".to_string(),
    duration: dur,
    track_number: None,
    genre: String::new(),
    year: None,
    bitrate: None,
    samplerate: None,
    hash: String::new(),
    cover_path: None,
    favourite: false,
}
```

Add to `gtm-core/src/validate.rs`:
```rust
impl TrackInfo {
    pub fn from_path(path: &str, duration: f64) -> Self {
        Self {
            id: 0,
            path: path.to_string(),
            title: std::path::Path::new(path)
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("Unknown")
                .to_string(),
            artist: "Unknown Artist".into(),
            album: "Unknown Album".into(),
            duration,
            ..Default::default()  // requires Default impl
        }
    }
}
```

### D2. Add PID file write/remove
**File**: `gtmd/src/daemon.rs`

In `Daemon::new()`, after binding sockets:
```rust
std::fs::write(&config.pid_file, std::process::id().to_string())?;
```

In `Daemon::run()` quit handler:
```rust
let _ = std::fs::remove_file(&inner.config.pid_file);
```

### D3. Add periodic state persistence
**File**: `gtmd/src/daemon.rs`

In `Daemon::run()`, spawn:
```rust
let state_clone = self.inner.state.clone();
let state_file = self.inner.config.state_file.clone();
tokio::spawn(async move {
    let mut interval = tokio::time::interval(Duration::from_secs(1800));
    loop {
        interval.tick().await;
        let s = state_clone.read().await;
        let saved = SavedState::from_state(&s);
        drop(s);
        let _ = saved.save(&state_file);
    }
});
```

### D4. Fix background_scan to reuse daemon Library
**File**: `gtmd/src/daemon.rs`

Currently creates new `Library::new()` instances in `background_scan`.
Store library in `DaemonInner` and share via `Arc`.

### D5. Fix heartbeat interval
**File**: `gtmd/src/daemon.rs`

Change `Duration::from_secs(10)` to `Duration::from_secs(30)`.

### D6. Fix check_health response format
**File**: `gtmd/src/daemon.rs`

Spec expects:
```json
{"uptime_secs": 3600, "clients_connected": 2, "audio_backend": "rodio"}
```

Simplify to return a flat JSON object instead of the nested `HealthReport` struct.
