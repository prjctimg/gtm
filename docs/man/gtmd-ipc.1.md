% gtmd-ipc(1) gtmd IPC protocol manual
% prjctimg
% 2026

# NAME

gtmd-ipc - IPC protocol for the gtm music daemon

# DESCRIPTION

The gtm daemon (**gtmd**(1)) communicates with clients over Unix domain sockets
using a mixed JSON+binary protocol. Commands are sent as newline-delimited JSON
objects with an explicit `cmd` field. The daemon responds synchronously with a
JSON response per request, interleaved with asynchronous JSON and binary event
notifications.

Commands are dispatched on the daemon via `cmd` string matching; params are
deserialized from the envelope's `params` key. Each command variant carries
its own params structure — the envelope `params` is a flat object with no
wrapper key.

The daemon's protocol version is `3` (`gtm-core/src/ipc.rs:16`). Framing is
newline-delimited JSON for commands/responses; binary MessagePack frames for
the dedicated pulse socket event stream.

Two sockets are used:

| Socket | Path | Purpose |
|--------|------|---------|
| **Command** | `$XDG_RUNTIME_DIR/gtm/gtmd.sock` | JSON commands, responses, and JSON events |
| **Pulse** | `$XDG_RUNTIME_DIR/gtm/gtmd.pulse` | Binary event stream (MessagePack, read-only) |

If `$XDG_RUNTIME_DIR` is not set, the daemon falls back in order:
/tmp/gtm-$USER/gtm/gtmd.sock, `$TMPDIR/gtm/gtmd.sock`, `$HOME/.gtm/gtm/gtmd.sock`.

# FRAMING

## Commands (client → daemon)

Each command is a single JSON line terminated with `\n`:

```json
{"id": 1, "cmd": "play", "path": "/music/song.mp3", "start_pos": 0.0}
```

Fields:

- `id` (uint64, required): Monotonically increasing sequence number. Used to
  correlate responses. MUST be `0` for the handshake command only.
- `cmd` (string, required): The command name. Dispatch is via
  `DaemonReq::parse_cmd`.
- Additional fields: command-specific parameters. The envelope's `params` key
  holds a flat object — the nested `action` enum for `Queue`/`Library` carries
  its fields at the top level per wire spec.

## Responses (daemon → client)

Each response is a single JSON line terminated with `\n`:

```json
{"id": 1, "ok": true, "state": {"volume": 80}}
```

Fields:

- `id` (uint64, required): Echoes the `id` from the matching request.
- `ok` (boolean, required): `true` on success, `false` on error.
- `error` (string, optional): Human-readable error message when `ok` is `false`.
- Additional fields: command-specific response data encoded via
  `DaemonRes::to_wire(id)`.

## Events (daemon → client, JSON)

Events are delivered as individual JSON objects on the command socket,
interleaved with responses. Clients distinguish events from responses by
checking for the `event` field (events) versus the `id` field (responses).

```json
{"event": "playback_started", "track": {"title": "Song"}, "time_pos": 0.0, "duration": 240.0}
```

## Events (daemon → client, binary / MessagePack)

The pulse socket delivers the same events in a compact binary format for
high-frequency position updates. Frame format:

```
[4 bytes: payload length, big-endian uint32][payload bytes]
```

The payload is a MessagePack-encoded array of event objects. Each event is a
MessagePack map with at minimum an `event` string field, matching the full JSON
event schema.

The client distinguishes JSON responses from binary events by the first byte:

- `0x7B` (`{`): JSON response (read until `\n`)
- anything else: binary event frame (read 4-byte length, then payload)

Maximum JSON line length: 1,048,576 bytes (1 MiB) (`gtmd/src/daemon.rs:618`).
Maximum binary frame: 16,777,216 bytes (16 MiB) (`gtm-core/src/client.rs:1122`).

# HANDSHAKE

The first message a client sends MUST be a handshake command. The daemon
responds with its version and daemon identifier.

```json
{"id": 0, "cmd": "handshake", "version": 3, "client": "gtm", "client_version": "0.1.8"}
```

Daemon response:

```json
{"id": 0, "ok": true, "version": 3, "daemon": "gtmd-rs", "daemon_version": "0.1.8"}
```

If the client's protocol version exceeds the daemon's, the daemon responds
with `ok: false` and the client MUST disconnect.

# COMMAND ENVELOPE

Every command follows this envelope:

```json
{"id": <uint64>, "cmd": "<name>", ...params}
```

The daemon responds with:

```json
{"id": <uint64>, "ok": true, ...data}
{"id": <uint64>, "ok": false, "error": "<message>"}
```

# PLAYBACK COMMANDS

## play

Load a track by path and begin playback.

```json
{"id": 1, "cmd": "play", "path": "/path/to/file.opus", "start_pos": 0.0}
```

Response: `{"id": 1, "ok": true}`. Emits: `playback_started` event.

## play_pause

Smart toggle: stopped → play, playing → pause, paused → resume.

```json
{"id": 2, "cmd": "play_pause"}
```

Response: `{"id": 2, "ok": true}`.

## pause

```json
{"id": 3, "cmd": "pause"}
```

Response: `{"id": 3, "ok": true}`.

## stop

```json
{"id": 4, "cmd": "stop"}
```

Response: `{"id": 4, "ok": true}`.

## next / prev

```json
{"id": 5, "cmd": "next"}
{"id": 6, "cmd": "prev"}
```

Response: `{"id": 5, "ok": true}`.

## seek

```json
{"id": 7, "cmd": "seek", "position_secs": 30.0}
```

Response: `{"id": 7, "ok": true}`.

## set_volume

Volume range: 0-100.

```json
{"id": 8, "cmd": "set_volume", "volume": 75}
```

Response: `{"id": 8, "ok": true}`.

## get_volume

```json
{"id": 9, "cmd": "get_volume"}
```

Response: `{"id": 9, "ok": true, "volume": 75}`.

## toggle_shuffle

```json
{"id": 10, "cmd": "toggle_shuffle"}
```

Response: `{"id": 10, "ok": true}`.

## cycle_repeat

Modes: `"off"`, `"one"`, `"all"`.

```json
{"id": 11, "cmd": "cycle_repeat", "mode": "all"}
```

Response: `{"id": 11, "ok": true}`.

## toggle_mute

```json
{"id": 12, "cmd": "toggle_mute"}
```

Response: `{"id": 12, "ok": true}`.

## crossfade

```json
{"id": 13, "cmd": "crossfade", "enabled": true, "duration_secs": 3}
```

Response: `{"id": 13, "ok": true}`.

# AUDIO EFFECT COMMANDS

## set_eq_preset

```json
{"id": 14, "cmd": "set_eq_preset", "preset": "rock"}
```

Response: `{"id": 14, "ok": true}`.

## set_eq_enabled

```json
{"id": 15, "cmd": "set_eq_enabled", "enabled": true}
```

Response: `{"id": 15, "ok": true}`.

## set_reverb

```json
{"id": 16, "cmd": "set_reverb", "enabled": true, "room_size": 0.7}
```

Response: `{"id": 16, "ok": true}`.

## list_eq_presets

```json
{"id": 17, "cmd": "list_eq_presets"}
```

Response: `{"id": 17, "ok": true, "presets": ["flat", "rock", "pop", "jazz"]}`.

# QUEUE COMMANDS

All queue operations are sub-commands dispatched through the `queue` command
with an `action` field.

## queue list

```json
{"id": 20, "cmd": "queue", "action": "list"}
```

Response: `{"id": 20, "ok": true, "queue": [{"title": "...", "path": "..."}], "cursor": 0}`.

## queue add

```json
{"id": 21, "cmd": "queue", "action": "add", "path": "/path/to/file.opus"}
```

## queue remove

```json
{"id": 24, "cmd": "queue", "action": "remove", "index": 2}
```

## queue move

```json
{"id": 25, "cmd": "queue", "action": "move", "from": 3, "to": 1}
```

## queue clear

```json
{"id": 26, "cmd": "queue", "action": "clear"}
```

## queue set

```json
{"id": 27, "cmd": "queue", "action": "set", "paths": ["/a.opus"], "start_idx": 0}
```

# LIBRARY COMMANDS

All library operations are sub-commands dispatched through the `library`
command with an `action` field.

## library scan

```json
{"id": 30, "cmd": "library", "action": "scan", "path": "/path/to/music"}
```

Runs asynchronously. No `custom`/`scan_done` event emitted.

## library get_tracks

```json
{"id": 31, "cmd": "library", "action": "get_tracks", "filter": null, "sort": null}
```

Response: `{"id": 31, "ok": true, "tracks": [{"id": "abc", "title": "...", "artist": "...", "path": "...", "duration": 240.0}]}`.

## library get_playlists

```json
{"id": 32, "cmd": "library", "action": "get_playlists"}
```

Response: `{"id": 32, "ok": true, "playlists": [{"id": 1, "name": "My Playlist", "track_count": 15}]}`.

## library create_playlist

```json
{"id": 33, "cmd": "library", "action": "create_playlist", "name": "My Mix"}
```

## library delete_playlist

```json
{"id": 34, "cmd": "library", "action": "delete_playlist", "id": 1}
```

## library add_to_playlist

```json
{"id": 35, "cmd": "library", "action": "add_to_playlist", "playlist_id": 1, "track_ids": [1, 2, 3]}
```

## library get_recent

```json
{"id": 36, "cmd": "library", "action": "get_recent", "count": 10}
```

Response: `{"id": 36, "ok": true, "tracks": [...]}`.

## library remove_track

```json
{"id": 37, "cmd": "library", "action": "remove_track", "id": 42}
```

Response: `{"id": 37, "ok": true}`.

## library update_metadata

```json
{"id": 38, "cmd": "library", "action": "update_metadata", "track_id": 42, "title": "New Title"}
```

Response: `{"id": 38, "ok": true}`.

## library sync_covers

```json
{"id": 39, "cmd": "library", "action": "sync_covers"}
```

Runs asynchronously. Emits `metadata_changed` event on completion.

## library sync_lyrics

```json
{"id": 40, "cmd": "library", "action": "sync_lyrics"}
```

Runs asynchronously. Emits `metadata_changed` event on completion.

## library sync_metadata

```json
{"id": 41, "cmd": "library", "action": "sync_metadata", "path": "/path/to/file.mp3"}
```

Runs asynchronously. Emits `metadata_changed` event on completion.

## library sync_status

```json
{"id": 42, "cmd": "library", "action": "sync_status"}
```

Response: `{"id": 42, "ok": true, "report": {"running": false, "kind": "covers", "synced": 15, "total": 15}}`.

# SEARCH AND FAVOURITES

## search

```json
{"id": 41, "cmd": "search", "query": "jazz"}
```

Response: `{"id": 41, "ok": true, "tracks": [...]}`.

Extended parameters: none (the wire accepts **only** `query`; `fuzzy`,
`ignore_diacritics`, `fields` are not supported).

## get_favourites

```json
{"id": 42, "cmd": "get_favourites"}
```

Response: `{"id": 42, "ok": true, "tracks": [...]}`.

## add_favourite / remove_favourite

```json
{"id": 43, "cmd": "add_favourite", "track_id": 42}
{"id": 44, "cmd": "remove_favourite", "track_id": 42}
```

# YOUTUBE COMMANDS

## yt_search

```json
{"id": 45, "cmd": "yt_search", "query": "lofi jazz"}
```

Emits `custom` events with `name: "yt_search_partial"` or `"yt_search_done"`.

## yt_search_poll

```json
{"id": 46, "cmd": "yt_search_poll"}
```

Response: `{"id": 46, "ok": true, "results": [{"title": "...", "url": "...", "duration": 240, "channel": "..."}]}`.

## yt_search_cancel

```json
{"id": 47, "cmd": "yt_search_cancel"}
```

## yt_resolve_stream

```json
{"id": 48, "cmd": "yt_resolve_stream", "url": "https://youtube.com/watch?v=..."}
```

## yt_download

```json
{"id": 49, "cmd": "yt_download", "url": "https://youtube.com/watch?v=..."}
```

## yt_download_poll

```json
{"id": 50, "cmd": "yt_download_poll"}
```

Response: `{"id": 50, "ok": true, "progress": 0.75, "status": "downloading"}`.

## yt_cancel_download

```json
{"id": 51, "cmd": "yt_cancel_download", "url": "https://youtube.com/watch?v=..."}
```

## yt_fetch_playlist

```json
{"id": 52, "cmd": "yt_fetch_playlist", "url": "https://youtube.com/playlist?list=..."}
```

## yt_fetch_playlist_poll

```json
{"id": 53, "cmd": "yt_fetch_playlist_poll"}
```

## yt_set_config

```json
{"id": 54, "cmd": "yt_set_config", "cookie_source": "/path/to/cookies.txt", "js_runtime": "deno", "download_dir": null, "max_concurrent": 4}
```

# COVER ART AND LYRICS

## get_cover_art

```json
{"id": 55, "cmd": "get_cover_art", "track_id": 42}
```

Response: `{"id": 55, "ok": true, "data": "<base64-encoded PNG>"}`.

## get_lyrics

```json
{"id": 56, "cmd": "get_lyrics", "track_id": 42}
```

Response: `{"id": 56, "ok": true, "lyrics": {"synced": true, "lines": [{"time": 0.0, "text": "..."}]}}`.

# AUDIO EFFECTS

## set_sleep_timer / cancel_sleep_timer

```json
{"id": 57, "cmd": "set_sleep_timer", "minutes": 30}
{"id": 58, "cmd": "cancel_sleep_timer"}
```

# LOUDNESS COMPENSATION

## set_loudness_mode

Modes: `"off"`, `"track"`, `"album"`, `"auto"`.

```json
{"id": 60, "cmd": "set_loudness_mode", "mode": "auto"}
```

Response: `{"id": 60, "ok": true}`.

## scan_loudness

```json
{"id": 61, "cmd": "scan_loudness", "track_ids": null, "force": false}
```

Runs asynchronously. Emits `loudness_scan_progress` and `loudness_scan_done`.

## set_pre_gain

```json
{"id": 62, "cmd": "set_pre_gain", "pre_gain_db": -14.0}
```

Response: `{"id": 62, "ok": true}`.

# GAPLESS PLAYBACK

## set_gapless

```json
{"id": 63, "cmd": "set_gapless", "enabled": true}
```

Response: `{"id": 63, "ok": true}`.

# DYNAMIC MODE

## set_dynamic_mode

```json
{"id": 64, "cmd": "set_dynamic_mode", "enabled": true, "min_queue_remaining": 3, "max_history": 50}
```

Response: `{"id": 64, "ok": true}`.

# SCROBBLING

## set_scrobble

```json
{"id": 65, "cmd": "set_scrobble", "enabled": true, "api_key": "...", "session_token": "..."}
```

Response: `{"id": 65, "ok": true}`.

# SYSTEM COMMANDS

## get_status

```json
{"id": 70, "cmd": "get_status"}
```

Response: `{"id": 70, "ok": true, "state": {"uptime_secs": 3600, "clients_connected": 2, "audio_backend": "rodio"}}`.

## check_health

```json
{"id": 71, "cmd": "check_health"}
```

Response: `{"id": 71, "ok": true, "report": {"uptime_secs": 3600, "clients_connected": 2, "audio_backend": "rodio"}}`.

## ping

```json
{"id": 99, "cmd": "ping"}
```

Response: `{"id": 99, "ok": true}`.

## quit

```json
{"id": 100, "cmd": "quit"}
```

Response: `{"id": 100, "ok": true}`.

The daemon persists state and closes the connection.

# EVENTS

Events are daemon-to-client notifications about state changes. They are
delivered as JSON objects on the command socket and as MessagePack binary
frames on the pulse socket.

## Playback Lifecycle

- `playback_started`: track begins playing. Fields: `track` (TrackInfo),
  `auto_advanced` (bool), `time_pos` (float64), `duration` (float64).
- `playback_paused`: playback paused. Fields: `time_pos`.
- `playback_stopped`: playback explicitly stopped.
- `track_ended`: track reached end of file naturally.

## Position and Duration

- `position_changed`: playback position updated. Fields: `time_pos` (float64).
- `duration_changed`: track duration resolved. Fields: `duration` (float64).

## Volume

- `volume_changed`: volume level changed. Fields: `volume` (uint8, 0-100).

## Playback Mode

- `shuffle_changed`: shuffle toggled. Fields: `enabled` (bool).
- `repeat_mode_changed`: repeat mode changed. Fields: `mode` (string).

## Queue

- `queue_changed`: queue modified. Fields: `queue` (array), `cursor`.
- `queue_index_changed`: cursor moved. Fields: `index`.

## Audio Effects

- `crossfade_changed`: fields: `enabled`, `duration_secs`.
- `eq_preset_changed`: fields: `preset`.
- `eq_enabled_changed`: fields: `enabled`.
- `reverb_changed`: fields: `enabled`, `room_size`.

## Sleep Timer

- `sleep_timer_tick`: emitted every second. Fields: `remaining_secs`.
- `sleep_timer_expired`: timer reached zero.

## Loudness Compensation

- `loudness_mode_changed`: fields: `mode`.
- `loudness_scan_progress`: fields: `tracks_remaining`, `tracks_total`, `current_track`.
- `loudness_scan_done`: fields: `scanned`, `failed`.
- `pre_gain_changed`: fields: `pre_gain_db`.

## Gapless Playback

- `gapless_changed`: fields: `enabled`.

## Dynamic Mode

- `dynamic_mode_changed`: fields: `enabled`, `min_queue_remaining`, `max_history`.

## Scrobbling

- `scrobble_config_changed`: fields: `enabled`.
- `scrobble_sent`: fields: `track`, `timestamp`.
- `scrobble_error`: fields: `track_id`, `error`.

## Library Organization

- `library_organized`: fields: `moves_succeeded`, `moves_failed`.

## System

- `heartbeat`: emitted at least every 30 seconds during active playback.
- `custom`: extensible event type with `name` sub-type field. Known names:
  `daemon_quitting`, `backend_error`, `audio_error`, `scan_done`.

# SEE ALSO

**gtmd**(1), **gtm**(1)

# AUTHORS

prjctimg <prjctimg@outlook.com>

# BUGS

Report bugs to <https://github.com/prjctimg/gtm.rs/issues> or by email to
<prjctimg@outlook.com>.

# COPYRIGHT

Copyright (c) 2026 prjctimg.

This is free software released under the GPL-3.0 license. See the LICENSE
file for the full license text.