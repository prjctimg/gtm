% GTMD(1) User Manuals
% prjctimg
% v0.7.34

# NAME

gtmd - background music playback daemon (IPC server)

# SYNOPSIS

`gtmd` [*options*]

# DESCRIPTION

**gtmd** is a standalone audio playback daemon. It plays audio via
FFmpeg + ALSA, maintains a music library in SQLite, and exposes a
JSON\-over\-Unix\-socket IPC interface. It supports:

- Local audio file playback (FLAC, MP3, Ogg, WAV, AAC, Opus, WMA)
- YouTube streaming (via yt-dlp)
- Gapless crossfade with configurable duration and curve
- 10-band graphic equalizer with presets
- Playlist and favourites management
- MPRIS D-Bus interface (when built with dbus)
- Sleep timer and idle shutdown
- Background directory scanning

```
┌─────────────────────────────────────────────────────┐
│                  gtmd (daemon)                      │
│                                                     │
│  ┌──────────────┐     select() loop (16ms)         │
│  │  Unix socket  │◄─── read cmds, write resp+events │
│  └──────┬───────┘                                   │
│         ▼                                           │
│  ┌──────────────┐                                   │
│  │ parseCmd()   │──► executeCommand()               │
│  └──────────────┘                                   │
│         ▼                                           │
│  ┌──────────────────────┐     ┌─────────────────┐   │
│  │ AudioBackend          │     │ SQLite library   │   │
│  │  ├─ MixerBackend      │     │  ├─ tracks       │   │
│  │  └─ FfmpegBackend     │     │  ├─ playlists    │   │
│  │  pollEvents() ──►     │     │  ├─ favourites   │   │
│  │  events → broadcast   │     │  ├─ downloads    │   │
│  └──────────────────────┘     │  └─ trash        │   │
│  ┌──────────────────────┐     └─────────────────┘   │
│  │ yt-dlp processes     │                            │
│  │  ├─ search           │                            │
│  │  ├─ stream resolve   │                            │
│  │  ├─ download         │                            │
│  │  └─ playlist fetch   │                            │
│  └──────────────────────┘                            │
└─────────────────────────────────────────────────────┘
```

# OPTIONS

`--debug`
:   Enable debug logging.

`--help`
:   Display help and exit.

# IPC TRANSPORT

| Field | Value |
|---|---|
| Socket family | `AF_UNIX` / `SOCK_STREAM` |
| Socket path | `$XDG_RUNTIME_DIR/gtm/gtmd.sock`
| Framing | Newline\-delimited JSON |
| Encoding | UTF-8 |

## Message format

Request (client → daemon):

```json
{"cmd": "<command>", "arg1": value1, ...}
```

Response (daemon → client):

```json
{"ok": true, "field1": value1, ...}
```

Events (daemon → client, unsolicited):

```json
{"events": [{"kind": 1, ...}, ...]}
```

# COMMANDS

All commands use the wire format `{"cmd": "<name>", ...}`.
Every response includes `"ok"` (boolean) unless noted.

## Playback Control

#### `play`

Request: `{"cmd": "play"}`

Response: `{"ok": true}`

#### `pause`

Request: `{"cmd": "pause"}`

Response: `{"ok": true}`

#### `toggle_pause`

Request: `{"cmd": "toggle_pause"}`

Response: `{"ok": true}`

#### `stop`

Request: `{"cmd": "stop"}`

Response: `{"ok": true}`

#### `seek`

Request: `{"cmd": "seek"}`

Response: `{"ok": true}`

#### `next`

Request: `{"cmd": "next"}`

Response: `{"ok": true}`

#### `prev`

Request: `{"cmd": "prev"}`

Response: `{"ok": true}`

#### `load_file`

Request: `{"cmd": "load_file"}`

Response: `{"ok": true}`

#### `resume`

Request: `{"cmd": "resume"}`

Response: `{"ok": true}`

#### `status`

Request: `{"cmd": "status"}`

Response: `{"ok": true, ...}`

#### `now_playing`

Request: `{"cmd": "now_playing"}`

Response: `{"ok": true, ...}`

#### `get_state`

Request: `{"cmd": "get_state"}`

Response: `{"ok": true, ...}`

#### `ping`

Request: `{"cmd": "ping"}`

Response: `{"ok": true}`

## Playlists

#### `create_playlist`

Request: `{"cmd": "create_playlist"}`

Response: `{"ok": true}`

#### `delete_playlist`

Request: `{"cmd": "delete_playlist"}`

Response: `{"ok": true}`

#### `rename_playlist`

Request: `{"cmd": "rename_playlist"}`

Response: `{"ok": true}`

#### `add_to_playlist`

Request: `{"cmd": "add_to_playlist"}`

Response: `{"ok": true}`

#### `remove_from_playlist`

Request: `{"cmd": "remove_from_playlist"}`

Response: `{"ok": true}`

#### `list_playlists`

Request: `{"cmd": "list_playlists"}`

Response: `{"ok": true, ...}`

#### `get_playlist_tracks`

Request: `{"cmd": "get_playlist_tracks"}`

Response: `{"ok": true, ...}`

## Queue

#### `queue_add`

Request: `{"cmd": "queue_add"}`

Response: `{"ok": true}`

#### `queue_remove`

Request: `{"cmd": "queue_remove"}`

Response: `{"ok": true}`

#### `queue_remove_path`

Request: `{"cmd": "queue_remove_path"}`

Response: `{"ok": true}`

#### `queue_clear`

Request: `{"cmd": "queue_clear"}`

Response: `{"ok": true}`

#### `queue_validate`

Request: `{"cmd": "queue_validate"}`

Response: `{"ok": true}`

#### `queue_list`

Request: `{"cmd": "queue_list"}`

Response: `{"ok": true, ...}`

#### `queue_set_cursor`

Request: `{"cmd": "queue_set_cursor"}`

Response: `{"ok": true}`

## Crossfade

#### `prepare_next`

Request: `{"cmd": "prepare_next"}`

Response: `{"ok": true}`

#### `crossfade`

Request: `{"cmd": "crossfade"}`

Response: `{"ok": true}`

#### `set_crossfade_duration`

Request: `{"cmd": "set_crossfade_duration"}`

Response: `{"ok": true}`

#### `set_crossfade_curve`

Request: `{"cmd": "set_crossfade_curve"}`

Response: `{"ok": true}`

## Library

#### `get_library`

Request: `{"cmd": "get_library"}`

Response: `{"ok": true, ...}`

#### `add_track`

Request: `{"cmd": "add_track"}`

Response: `{"ok": true}`

#### `update_track_path`

Request: `{"cmd": "update_track_path"}`

Response: `{"ok": true}`

#### `scan`

Request: `{"cmd": "scan"}`

Response: `{"ok": true}`

#### `delete_track`

Request: `{"cmd": "delete_track"}`

Response: `{"ok": true}`

#### `restore_track`

Request: `{"cmd": "restore_track"}`

Response: `{"ok": true}`

#### `permanent_delete_trash`

Request: `{"cmd": "permanent_delete_trash"}`

Response: `{"ok": true}`

#### `list_trash`

Request: `{"cmd": "list_trash"}`

Response: `{"ok": true, ...}`

#### `purge_trash`

Request: `{"cmd": "purge_trash"}`

Response: `{"ok": true}`

## Lifecycle

#### `quit`

Request: `{"cmd": "quit"}`

Response: `{"ok": true}`

## Volume

#### `set_volume`

Request: `{"cmd": "set_volume"}`

Response: `{"ok": true}`

#### `get_volume`

Request: `{"cmd": "get_volume"}`

Response: `{"ok": true, ...}`

## Cover Art & Lyrics

#### `get_cover_art`

Request: `{"cmd": "get_cover_art"}`

Response: `{"ok": true, ...}`

#### `get_lyrics`

Request: `{"cmd": "get_lyrics"}`

Response: `{"ok": true, ...}`

#### `search_lyrics`

Request: `{"cmd": "search_lyrics"}`

Response: `{"ok": true, ...}`

## YouTube

#### `yt_search`

Request: `{"cmd": "yt_search"}`

Response: `{"ok": true}`

#### `yt_search_poll`

Request: `{"cmd": "yt_search_poll"}`

Response: `{"ok": true, ...}`

#### `yt_search_cancel`

Request: `{"cmd": "yt_search_cancel"}`

Response: `{"ok": true}`

#### `yt_resolve_stream`

Request: `{"cmd": "yt_resolve_stream"}`

Response: `{"ok": true}`

#### `yt_resolve_stream_poll`

Request: `{"cmd": "yt_resolve_stream_poll"}`

Response: `{"ok": true}`

#### `yt_download`

Request: `{"cmd": "yt_download"}`

Response: `{"ok": true}`

#### `yt_download_poll`

Request: `{"cmd": "yt_download_poll"}`

Response: `{"ok": true}`

#### `yt_cancel_download`

Request: `{"cmd": "yt_cancel_download"}`

Response: `{"ok": true}`

#### `yt_list_downloads`

Request: `{"cmd": "yt_list_downloads"}`

Response: `{"ok": true, ...}`

#### `yt_fetch_playlist`

Request: `{"cmd": "yt_fetch_playlist"}`

Response: `{"ok": true}`

#### `yt_fetch_playlist_poll`

Request: `{"cmd": "yt_fetch_playlist_poll"}`

Response: `{"ok": true, ...}`

#### `yt_set_config`

Request: `{"cmd": "yt_set_config"}`

Response: `{"ok": true}`

#### `yt_get_search_history`

Request: `{"cmd": "yt_get_search_history"}`

Response: `{"ok": true, ...}`

#### `yt_clear_search_history`

Request: `{"cmd": "yt_clear_search_history"}`

Response: `{"ok": true}`

## State

#### `get_full_state`

Request: `{"cmd": "get_full_state"}`

Response: `{"ok": true, ...}`

#### `get_state`

Request: `{"cmd": "get_state"}`

Response: `{"ok": true, ...}`

#### `get_volume`

Request: `{"cmd": "get_volume"}`

Response: `{"ok": true, ...}`

## Spotify

#### `sp_set_config`

Request: `{"cmd": "sp_set_config"}`

Response: `{"ok": true}`

#### `sp_list_downloads`

Request: `{"cmd": "sp_list_downloads"}`

Response: `{"ok": true, ...}`

## Equalizer

#### `set_eq_band`

Request: `{"cmd": "set_eq_band"}`

Response: `{"ok": true}`

#### `set_eq_preset`

Request: `{"cmd": "set_eq_preset"}`

Response: `{"ok": true}`

#### `list_eq_presets`

Request: `{"cmd": "list_eq_presets"}`

Response: `{"ok": true, ...}`

## Playback Mode

#### `set_shuffle`

Request: `{"cmd": "set_shuffle"}`

Response: `{"ok": true}`

#### `set_repeat`

Request: `{"cmd": "set_repeat"}`

Response: `{"ok": true}`

#### `set_sleep_timer`

Request: `{"cmd": "set_sleep_timer"}`

Response: `{"ok": true}`

## Favourites

#### `add_favourite`

Request: `{"cmd": "add_favourite"}`

Response: `{"ok": true}`

#### `remove_favourite`

Request: `{"cmd": "remove_favourite"}`

Response: `{"ok": true}`

#### `get_favourites`

Request: `{"cmd": "get_favourites"}`

Response: `{"ok": true, ...}`

# EXAMPLES

### Play a track (socat):

```bash
echo '{"cmd":"play"}' | socat - UNIX-CONNECT:$XDG_RUNTIME_DIR/gtm/gtmd.sock
# {"ok":true}
```

### Load and start a local file (socat):

```bash
echo '{"cmd":"load_file","path":"/home/user/music/track.flac"}' | \
  socat - UNIX-CONNECT:$XDG_RUNTIME_DIR/gtm/gtmd.sock
# {"ok":true}
```

### Query volume (socat):

```bash
echo '{"cmd":"get_volume"}' | socat - UNIX-CONNECT:$XDG_RUNTIME_DIR/gtm/gtmd.sock
# {"ok":true,"volume":80}
```

### Set volume (socat):

```bash
echo '{"cmd":"set_volume","volume":60}' | socat - UNIX-CONNECT:$XDG_RUNTIME_DIR/gtm/gtmd.sock
# {"ok":true}
```

### Listen for events (daemon keeps connection open):

```bash
socat - UNIX-CONNECT:$XDG_RUNTIME_DIR/gtm/gtmd.sock <<'EOF'
{"cmd":"play"}
EOF
# Response:
# {"ok":true}
# Incoming events as playback progresses:
# {"events":[{"kind":1,"state":"playing","track_path":"/home/user/music/track.flac","time_pos":0,"duration":253.0}]}
# {"events":[{"kind":5,"time_pos":15.2}]}
# {"events":[{"kind":5,"time_pos":30.5}]}
```

### Scripted interaction from a shell script:

```bash
#!/bin/sh
SOCK=$XDG_RUNTIME_DIR/gtm/gtmd.sock

# Send command and read one response line
send_cmd() {
  echo "$1" | socat - UNIX-CONNECT:"$SOCK" 2>/dev/null | head -1
}

# Start playback
send_cmd '{"cmd":"play"}'

# Set volume
send_cmd '{"cmd":"set_volume","volume":60}'

# Get current track info
send_cmd '{"cmd":"now_playing"}'
```

### Programmatic access from Nim:

```nim
import std/net, std/json

let sock = newSocket()
sock.connect("$XDG_RUNTIME_DIR/gtm/gtmd.sock", Port(0))  # AF_UNIX
sock.send("{\"cmd\":\"ping\"}\n")
let resp = sock.recvLine()
echo parseJson(resp)
# {"ok": true}
```

# EVENTS

Events are pushed asynchronously in JSON arrays:

```json
{"events": [{"kind": 0, ...}, ...]}
```

| Kind | Name | Extra fields | Description |
|---|---|---|---|
| `0` | `evNone` |  | No event (placeholder) |
| `1` | `evPlaybackStarted` | state, track_path, track_title, track_channel, time_pos, duration | Playback has started. Extra: state, track_path, track_title, track_channel, time_pos, duration |
| `2` | `evPlaybackPaused` | state | Playback was paused. Extra: state |
| `3` | `evPlaybackStopped` | state | Playback was stopped. Extra: state |
| `4` | `evTrackEnded` | reason | Current track reached end-of-file. Extra: reason |
| `5` | `evPositionChanged` | time_pos | Playback position changed. Extra: time_pos |
| `6` | `evDurationChanged` | duration | Track duration changed. Extra: duration |
| `7` | `evVolumeChanged` | volume | Volume changed. Extra: volume |
| `8` | `evMetadataChanged` | event | Track metadata changed. Extra: event |
| `9` | `evError` |  | Audio backend error occurred |
| `10` | `evCustomEvent` | event, (type-specific) | Custom event. Extra: event, plus type-specific fields |

# FILES

`$XDG_DATA_HOME/gtm/gtm.db`
:   SQLite database (schema: tracks, artists, albums, playlists, favourites, downloads, trash, playback_state).

`$XDG_RUNTIME_DIR/gtm/gtmd.sock`
:   Unix domain socket for IPC.

`$XDG_RUNTIME_DIR/gtm/gtmd.pid`
:   PID file for singleton enforcement.

# ENVIRONMENT

`XDG_RUNTIME_DIR`
:   Used for the daemon socket and PID file.

`XDG_DATA_HOME`
:   Used for the library database and download directory.

# BUGS

Report bugs and feature requests at:
<https://github.com/prjctimg/gtm/issues>

# SEE ALSO

`gtm`(1), `ffplay`(1), `yt-dlp`(1)
