## Spec: Spotify Integration Fixes

### Problem
The Spotify integration had several issues:
1. Pasting a token into the Spotify/Soloist input fields didn't work
2. Resolved tracks lost their metadata (title, artist, album from Spotify)
3. Cover art wasn't fetched immediately after resolving a Spotify track

### Changes

#### 1. Bracketed Paste Support
- Enable `EnableBracketedPaste` in terminal setup (`ui.rs`)
- Disable on teardown
- Handle `Event::Paste` in the event loop (`app.rs`)
- Route pasted text to `SpotifySearch` (token input), `EditMetadata`, and
  text-input pickers (query fields)

#### 2. Track Metadata on Resolve
In `cmd_spotify_resolve` (`daemon.rs`):
- After `queue_add`, patch the queued entry with Spotify title/artist/album
- Use the canonical path from `queue_add`'s return value to find the entry

#### 3. Immediate Cover Fetch
- After queuing, call `cover_cache.get_cover(artist, album)` inline
- This pre-populates the cover cache so the TUI has cover art immediately

### Verification
- Paste a Spotify token, verify it appears in the input field
- Resolve a Spotify playlist track, verify title/artist/album display correctly
- Verify cover art loads after resolving a Spotify track
