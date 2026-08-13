# Spec 3: User-configurable cookies.txt path

## Problem

The YouTube settings UI shows "Cookie Source" and "Cookie File" entries, but only
`cookie_source` is plumbed through the IPC protocol. There is no way to specify a
custom cookies.txt file path. Users who want to use a specific cookies file (e.g.,
exported from a browser extension) have no way to configure it.

## Root Cause

1. `DaemonReq::YtSetConfig` in `gtm-core/src/ipc.rs:224` has `cookie_source` but no
   `cookie_file` field.
2. `YoutubeManager` in `gtmd/src/youtube.rs` doesn't accept or use a custom cookie
   file path.
3. The dispatch of `YtSetConfig` in `gtmd/src/daemon.rs` doesn't pass a cookie file
   path to the youtube manager.
4. The TUI settings panel displays "Cookie File" in `gtm/src/ui.rs:1093` but has no
   interactive handling for it in the key handler.

## Files to Modify

- `gtm-core/src/ipc.rs` — add `cookie_file` field to `YtSetConfig`
- `gtmd/src/youtube.rs` — store and use custom cookie file path
- `gtmd/src/daemon.rs` — propagate `cookie_file` to `YoutubeManager`
- `gtm/src/app.rs` — handle setting changes for cookie file
- `gtm/src/ui.rs` — make "Cookie File" interactive

## Implementation Steps

### 1. IPC Protocol: Add cookie_file field

In `gtm-core/src/ipc.rs`, modify `YtSetConfig`:

```rust
YtSetConfig {
    cookie_source: Option<String>,
    cookie_file: Option<String>,  // NEW
    js_runtime: Option<String>,
    download_dir: Option<String>,
    max_concurrent: Option<u32>,
},
```

Add `cookie_file` to the serialization/deserialization logic if manual.

### 2. Daemon: YoutubeManager

In `gtmd/src/youtube.rs`, add a field to `YoutubeManager`:

```rust
pub struct YoutubeManager {
    client: Client,
    data_dir: PathBuf,
    cookie_file: Option<PathBuf>,  // NEW
}
```

When invoking yt-dlp for downloads or search, pass `--cookies` if set:

```rust
let mut cmd = tokio::process::Command::new("yt-dlp");
if let Some(ref cookie_path) = self.cookie_file {
    cmd.arg("--cookies").arg(cookie_path);
}
// ... rest of args
```

Add a setter method:

```rust
pub fn set_cookie_file(&mut self, path: Option<String>) {
    self.cookie_file = path.map(PathBuf::from);
}
```

### 3. Daemon: Dispatch YtSetConfig

In `gtmd/src/daemon.rs`, find the `YtSetConfig` handler and add:

```rust
DaemonReq::YtSetConfig {
    cookie_source,
    cookie_file,  // NEW
    js_runtime,
    download_dir,
    max_concurrent,
} => {
    let mut yt = inner.youtube.lock().await;
    if let Some(cf) = cookie_file {
        yt.set_cookie_file(cf);
    }
    // ... existing cookie_source handling
    return Ok(DaemonRes::Ok);
}
```

### 4. TUI: Make "Cookie File" interactive

In `gtm/src/ui.rs`, settings YouTube category (~line 1093):
- The "Cookie File" entry currently shows `[ (none) ]`
- When selected and Enter is pressed, open a text input to accept a file path

In `gtm/src/app.rs`, settings handling:
- When the cookie file option is selected and confirmed, send
  `YtSetConfig { cookie_file: Some(path), .. }` to the daemon
- Store the path locally for display in the settings UI

### 5. Default behavior

When `cookie_file` is `None`, yt-dlp's own cookie discovery mechanism applies
(browser integration, netscape cookies file in default locations, etc.),
preserving backward compatibility.

## Verification

1. Go to Settings → YouTube tab
2. Select "Cookie File" and press Enter
3. Enter a path to a valid cookies.txt file (e.g., `/home/user/.cookies/youtube.txt`)
4. The setting should persist and show the configured path
5. Perform a YouTube search — yt-dlp should use the specified cookies file
6. Set it back to empty/None — yt-dlp should fall back to default behavior
