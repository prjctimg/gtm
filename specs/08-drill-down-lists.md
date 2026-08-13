# Spec 8: Drill-down lists show contents instead of blank

## Problem

When selecting an item from the Albums, Artists, or Playlists categories in the library
sidebar, the right pane shows a blank list instead of the filtered tracks.

## Root Cause

`filtered_tracks()` in `gtm/src/app.rs:847-854` applies the `browse_detail` filter:

```rust
if let Some(ref detail) = self.browse_detail {
    let detail_lower = detail.to_lowercase();
    tracks.retain(|t| {
        t.album.to_lowercase().contains(&detail_lower)
            || t.artist.to_lowercase().contains(&detail_lower)
            || t.title.to_lowercase().contains(&detail_lower)
    });
}
```

Three issues:

1. **Playlists**: For category 4 (Playlists), `browse_detail` is set to the playlist
   name (line 1918). But `filtered_tracks()` doesn't know about playlist membership —
   it does a text `contains()` search, which won't match track metadata against a
   playlist name. Playlist tracks must be fetched from the daemon's playlist API.

2. **Metadata mismatches**: The `tracks_cache` may not have the expected metadata
   populated. If tracks were scanned without full metadata extraction or if queued
   tracks with `id: 0` from `resolve_track()` are in the cache, the album/artist
   fields will be empty strings, causing no matches.

3. **No forced refresh**: After setting `browse_detail`, there's no explicit refresh
   of the track cache to ensure metadata is current.

## Files to Modify

- `gtm/src/app.rs`
- `gtm-core/src/ipc.rs` (maybe — for playlist tracks query)

## Implementation Steps

### 1. Fix playlist drill-down (category 4)

For playlists, we need to fetch tracks from the daemon's playlist API instead of
filtering the main cache.

**Add a `playlist_tracks_cache` to App:**

```rust
// After `playlist_cache`
pub playlist_tracks_cache: Vec<TrackInfo>,
```

**Add IPC for fetching playlist tracks** (if not already present):

In `gtm-core/src/ipc.rs`, check if there's a `GetPlaylistTracks` or similar request.
If not, add one:

```rust
GetPlaylistTracks { playlist_id: i64 },
// Response:
PlaylistTracks { tracks: Vec<TrackInfo> },
```

**In `app.rs`, update the Select handler for playlists (~line 1914-1920):**

```rust
} else if self.library_category == 4 {
    // Playlists: select playlist → fetch its tracks
    if self.scroll_offset < self.playlist_cache.len() {
        let playlist = &self.playlist_cache[self.scroll_offset];
        self.browse_detail = Some(playlist.name.clone());
        self.scroll_offset = 0;
        // Fetch playlist tracks from daemon
        let client = self.client.clone();
        let tx = self.ipc_tx.clone();
        let pid = playlist.id;
        tokio::spawn(async move {
            // Use existing or new IPC method to get tracks
            if let Ok(tracks) = client.library_get_playlist_tracks(pid).await {
                let _ = tx.send(IpcResult::PlaylistTracks(tracks));
            }
        });
    }
}
```

**In `filtered_tracks()`:**

```rust
if self.library_category == 4 && self.browse_detail.is_some() {
    // Return cached playlist tracks
    return self.playlist_tracks_cache.iter().collect();
}
```

### 2. Fix Albums and Artists drill-down

For Albums (category 2) and Artists (category 3), the existing `contains()` filter
should work IF the tracks have proper metadata:

- Add a forced library refresh when entering drill-down:

```rust
} else if self.library_category == 2 {
    let albums = self.unique_albums();
    if self.scroll_offset < albums.len() {
        self.browse_detail = Some(albums[self.scroll_offset].0.clone());
        self.scroll_offset = 0;
        // Refresh the track cache to ensure full metadata
        self.send_high(TuiCommand::RefreshLibrary);
    }
} else if self.library_category == 3 {
    let artists = self.unique_artists();
    if self.scroll_offset < artists.len() {
        self.browse_detail = Some(artists[self.scroll_offset].0.clone());
        self.scroll_offset = 0;
        self.send_high(TuiCommand::RefreshLibrary); // Same
    }
}
```

### 3. Improve display in detail view

In `gtm/src/ui.rs`, the detail view rendering (~line 765-816) should show a helpful
message when no tracks match:

```rust
let filtered = app.filtered_tracks();
if filtered.is_empty() {
    let msg = Paragraph::new("No tracks found for this selection")
        .alignment(Alignment::Center)
        .style(Style::default().fg(app.theme.fg_dim));
    right_lines = vec![Line::from("")];
    // Render message inside the content area
}
```

## Verification

1. Go to Albums category — select an album
2. Verify the right pane shows tracks belonging to that album
3. Go to Artists category — select an artist
4. Verify the right pane shows tracks by that artist
5. Go to Playlists category — select a playlist
6. Verify the right pane shows tracks in that playlist
7. Verify the "Back" action (`Esc`) returns to the category overview
