# Spec 07 — Better CLI Logging

Status: **Planned** — verbose output with contextual feedback, --stream flag for status.

Green gate: `cargo fmt --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`

---

## 7.1 — Verbose output format

When `verbose: true` is passed, each CLI command now shows contextual feedback instead of just "ok".

Example: `gtm next` outputs:
```
Now Playing: [Track title]
[Artist name] - [album name]
```

The output format preserves tabs and indentation, and colors the output.

---

## 7.2 — --stream flag

Add `--stream` flag to `gtm status` command:
- Streams elapsed track time continuously (instead of one-shot)
- Updates now playing info all from CLI (no TUI needed)
- Output format:
  ```
  Stream: [Track title] - [Artist name] | [elapsed]s / [duration]s | [volume]%
  ```

---

## 7.3 — All commands with verbose output

Each command that can modify or query state should show contextual feedback:
- `play` → "Playing: [title] by [artist] - [album]"
- `pause` → "Paused: [title] by [artist]"
- `next` → "Now Playing: [title] by [artist] - [album]"
- `prev` → "Now Playing: [title] by [artist] - [album]"
- `seek` → "Seeking to [time]s — new position: [current]s"
- `volume` → "Volume set to [X]%"
- `shuffle` → "Shuffle: [On/Off]"
- `repeat` → "Repeat: [mode]"
- `mute` → "Mute: [On/Off]"
- `crossfade` → "Crossfade: [enabled/disabled] (duration: [X]s)"
- `queue` → "Queue: [N] tracks, cursor [M]/[N]"
- `queue_add` → "Added [N] track(s) to queue"
- `queue_remove` → "Removed track from queue"
- `queue_move` → "Moved track from [A] to [B]"
- `queue_clear` → "Queue cleared"
- `queue_set` → "Queue set to [N] tracks"
- `scan` → "Scanned [N] tracks in [path]"
- `tracks` → "Found [N] tracks matching [filter]"
- `playlists` → "Found [N] playlists"
- `create_playlist` → "Created playlist [name]"
- `delete_playlist` → "Deleted playlist [id]"
- `add_to_playlist` → "Added [N] track(s) to playlist [id]"
- `import_m3u` → "Imported M3U from [path]"
- `export_m3u` → "Exported M3U to [path]"
- `recent` → "Recent [N] tracks"
- `metadata_sync` → "Synced metadata: [N]/[N] tracks"
- `favourites` → "Found [N] favourite tracks"
- `favourite_add` → "Added track [id] to favourites"
- `favourite_remove` → "Removed track [id] from favourites"
- `yt_search` → "Search: [query]"
- `yt_poll` → "Streaming poll: [result]"
- `yt_cancel` → "Poll cancelled"
- `yt_resolve` → "Resolved: [url]"
- `lyrics` → "Found lyrics for [query]"
- `search` → "Search results: [N] matches"
- `status` → Full status with optional stream
- `check_health` → Health report with uptime
- `ping` → "Ping: [result]"
- `quit` → "Exiting"
- `config` → "Config opened"
