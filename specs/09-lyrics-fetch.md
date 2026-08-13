# Spec 9: Lyrics not fetched when pressing 'l'

## Problem

Pressing 'l' to toggle the lyrics pane does not fetch lyrics for the currently
playing track. The pane shows "Press [l] to search" instead of fetching, or shows
"Fetching lyrics..." indefinitely.

## Root Cause

The `FetchLyrics` action in `gtm/src/app.rs:1820-1826`:

```rust
Some(KeyboardAction::FetchLyrics) => {
    self.show_lyrics = !self.show_lyrics;
    if self.show_lyrics && self.current_lyrics.is_none() {
        self.lyrics_fetching = true;
        self.send_high(TuiCommand::FetchLyrics);
    }
    self.dismiss_track_popup();
}
```

Three potential issues:

1. **Race condition**: `self.send_high(TuiCommand::FetchLyrics)` sends the command
   via the unbounded high-pri channel. The command handler at line 1322-1337 gets
   `track_id` from `self.state.current_track`. If the track was just started and
   the state hasn't been updated yet, `current_track` could be `None`, resulting in
   `track_id = 0`, which the daemon can't resolve.

2. **Silent failure in daemon**: The `GetLyrics` handler in the daemon calls
   `lyrics_manager.get_lyrics(track)` which tries sidecar file, then lrclib exact
   match, then lrclib search. If all fail, it returns `None`. The client receives
   `IpcResult::Lyrics(None)` which sets `current_lyrics = None` and
   `lyrics_fetching = false`. The UI then shows "Press [l] to search" (because
   `current_lyrics` is None and `lyrics_fetching` is false).

3. **Track ID zero**: When playing queued tracks (from Spec 4B), the track ID may
   be 0, causing the daemon to be unable to look up the track in the library.

## Files to Modify

- `gtm/src/app.rs`
- `gtmd/src/daemon.rs`
- `gtm/src/ui.rs`

## Implementation Steps

### 1. Fix FetchLyrics to handle track_id=0

In `gtm/src/app.rs`, `handle_command()` for `TuiCommand::FetchLyrics` (~line 1322):

```rust
TuiCommand::FetchLyrics => {
    let track_id = match self.state.current_track.as_ref().map(|t| t.id) {
        Some(id) if id > 0 => id,
        _ => {
            // Try to get track by path from the state
            if let Some(ref track) = self.state.current_track {
                // Fallback: send path directly, daemon resolves
                let path = track.path.clone();
                let client2 = self.client.clone();
                let ipc_tx2 = self.ipc_tx.clone();
                tokio::spawn(async move {
                    // Resolve track by path on daemon side
                    let result = client2.get_lyrics_for_path(&path).await;
                    let _ = ipc_tx2.send(IpcResult::Lyrics(result.unwrap_or(None)));
                });
                return;
            }
            self.notify("No track playing", NotificationKind::Warning);
            return;
        }
    };
    let client2 = self.client.clone();
    let ipc_tx2 = self.ipc_tx.clone();
    tokio::spawn(async move {
        let result = tokio::time::timeout(Duration::from_secs(5), client2.get_lyrics(track_id))
            .await;
        let _ = ipc_tx2.send(IpcResult::Lyrics(match result {
            Ok(r) => r.unwrap_or(None),
            Err(_) => None,
        }));
    });
}
```

### 2. Add lyrics-by-path method to daemon

In `gtm-core/src/client.rs`, add a new method:

```rust
pub async fn get_lyrics_for_path(&self, path: &str) -> Result<Option<LrcData>> {
    // Either add a new IPC request, or look up the track by path first
    // For now, look up track ID by path using the library
    let tracks_resp = self.library_get_tracks(None, None).await?;
    match tracks_resp {
        DaemonRes::Tracks { tracks, .. } => {
            if let Some(track) = tracks.iter().find(|t| t.path == path) {
                self.get_lyrics(track.id).await
            } else {
                Ok(None)
            }
        }
        _ => Ok(None),
    }
}
```

### 3. Add IPC request for lyrics-by-path

Alternatively, add a new IPC variant:

In `gtm-core/src/ipc.rs`:
```rust
GetLyricsByPath { path: String },
```

In `gtmd/src/daemon.rs`, handler:
```rust
DaemonReq::GetLyricsByPath { path } => {
    let lyrics_manager = inner.lyrics_manager.as_ref();
    match lyrics_manager {
        Some(manager) => {
            // Look up track in library
            let lib = Library::new(inner.config.data_dir.to_str().unwrap_or("")).ok();
            if let Some(ref lib) = lib {
                match lib.track_by_path(&path) {
                    Ok(Some(track)) => {
                        let result = manager.get_lyrics(&track).await;
                        Ok(DaemonRes::Lyrics { lyrics: result })
                    }
                    _ => {
                        // Create minimal track info and try anyway
                        let track = queue::resolve_track(&path);
                        let result = manager.get_lyrics(&track).await;
                        Ok(DaemonRes::Lyrics { lyrics: result })
                    }
                }
            } else {
                Ok(DaemonRes::Error { message: "library unavailable".into() })
            }
        }
        None => Ok(DaemonRes::Error { message: "lyrics manager unavailable".into() }),
    }
}
```

### 4. Improve UI feedback on fetch failure

In `gtm/src/ui.rs`, `render_lyrics_pane()` (~line 1616):

After the fetch completes with no results, show a more informative message:

```rust
let Some(ref lyrics) = app.current_lyrics else {
    let msg_text = if app.lyrics_fetching {
        let spinner = braille_spinner(app.scroll_offset);
        format!("Fetching lyrics... {}", spinner)
    } else {
        "No lyrics found for this track".to_string() // Changed from "Press [l] to search"
    };
    // ...
};
```

### 5. Auto-fetch on toggle-on

Ensure that when the user toggles lyrics ON (from off), and a track is already
playing, the fetch is triggered immediately. The existing code at line 1821-1825
already handles this, but add a guard to prevent redundant fetches:

```rust
Some(KeyboardAction::FetchLyrics) => {
    let was_visible = self.show_lyrics;
    self.show_lyrics = !self.show_lyrics;
    if self.show_lyrics && !was_visible && self.state.current_track.is_some() {
        // Only fetch if we don't already have lyrics for this track
        let current_id = self.state.current_track.as_ref().map(|t| t.id);
        if self.last_lyrics_track_id != current_id {
            self.current_lyrics = None;
            self.lyrics_fetching = true;
            self.last_lyrics_track_id = current_id;
            self.send_high(TuiCommand::FetchLyrics);
        }
    }
    self.dismiss_track_popup();
}
```

## Verification

1. Start the TUI with a track loaded (metadata with title + artist)
2. Press 'l' to toggle lyrics on
3. Should show "Fetching lyrics..." with a spinner
4. After a moment, lyrics should appear (or "No lyrics found" if unavailable)
5. Press 'l' again to hide lyrics
6. Press 'l' again — should not re-fetch (cached)
7. Start a different track — lyrics should auto-fetch if pane is visible
