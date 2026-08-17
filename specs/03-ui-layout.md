## Spec: UI Layout & Display Fixes

### Problem
Several visual issues degraded the user experience:
1. Cover art wasn't centered horizontally in the now-playing pane
2. Lists showed redundant "N tracks" counts inline
3. Track info rendered even when the pane was too small
4. Liked list showed total library count instead of liked count

### Changes

#### Cover Art Centering (`ui.rs`)
- Compute `left_pad` as `(inner.width - COVER_W - 2) / 2`
- Apply horizontal offset when drawing cover block
- Adjust `info_area` chunk index from `hchunks[2]` to `hchunks[3]`

#### Remove Inlined Descriptions (`ui.rs`)
- Albums, Artists, Playlists list items: remove `" {:>4} tracks"` format
- Spotify playlist list items: same removal

#### Liked List Count (`ui.rs`)
- Replace `app.filtered_tracks().len()` with
  `app.tracks_cache.iter().filter(|t| t.favourite).count()`

#### Track Info Height Check (`ui.rs`)
- Gate `render_track_info_in_pane` on
  `left_track_info_area.height >= track_info_block_height()`

### Verification
- Cover art appears horizontally centered in the left pane
- Album/artist/playlist lists show names only (no counts)
- Liked list shows the correct favourite count
- Track info panel disappears when the window is too small
