# GTM IPC Protocol

The daemon listens on a Unix socket. Clients send newline-delimited JSON `DaemonReq` lines
and receive JSON `DaemonRes` responses (one line per response).  The daemon also pushes
binary-framed `DaemonEvent` notifications on the same connection (see *Events* below).

## Framing

**Requests** — a single JSON line terminated with `\n`:

```json
{"Ping":null}
```

**Responses** — a single JSON line terminated with `\n` (synchronously after each request):

```json
{"Pong":null}
```

**Events** — binary frames interleaved with responses, formatted as:

```
[4-byte big-endian payload length][bincode-encoded WireFrame]
```

Clients distinguish events from responses by inspecting the first byte:
- `0x7B` (`{`) → JSON response (read until `\n`)
- anything else → binary event frame (read 4 bytes for length, then payload)

---

## 1. Playback Commands

### Play a file

```json
{"Play":{"path":"/path/to/file.opus","start_pos":0.0}}
```

Response: `{"Ok":{"version":1}}`

### Toggle Play / Pause

```json
{"PlayPause":null}
```

Response: `{"Ok":{"version":1}}`

If a track is playing it will pause. If paused it will resume. If stopped and a
current track exists it will start from the beginning.

### Pause

```json
{"Pause":null}
```

Response: `{"Ok":{"version":1}}`

### Stop

```json
{"Stop":null}
```

Response: `{"Ok":{"version":1}}`

### Next track

```json
{"Next":null}
```

Response: `{"Ok":{"version":1}}`

Advances the queue cursor and plays the next track.

### Previous track

```json
{"Prev":null}
```

Response: `{"Ok":{"version":1}}`

Moves to the previous queue entry. If already at the first track, seeks to position 0.

### Seek

```json
{"Seek":{"position_secs":42.5}}
```

Response: `{"Ok":{"version":1}}`

### Set Volume

```json
{"SetVolume":{"volume":75}}
```

Volume range: 0–100. Response: `{"Ok":{"version":1}}`

### Toggle Shuffle

```json
{"ToggleShuffle":null}
```

Response: `{"Ok":{"version":1}}`

### Cycle Repeat

```json
{"CycleRepeat":{"mode":"One"}}
```

Mode values: `"Off"`, `"One"`, `"All"`. Response: `{"Ok":{"version":1}}`

### Toggle Mute

```json
{"ToggleMute":null}
```

Response: `{"Ok":{"version":1}}`

### Crossfade

```json
{"Crossfade":{"enabled":true,"duration_secs":7}}
```

Response: `{"Ok":{"version":1}}`

Duration is clamped to 0–30 seconds (default: 7s).

### Set EQ Preset

```json
{"SetEqPreset":{"preset":"Rock"}}
```

Preset values: `"Flat"`, `"Pop"`, `"Rock"`, `"Jazz"`, `"Classical"`, `"Bass"`, `"Vocal"`, `"Custom"`.
Response: `{"Ok":{"version":1}}`

---

## 2. Queue Commands

All queue commands use the `Queue` wrapper with an `action` field:

```json
{"Queue":{"action":{"List":null}}}
```

### List queue

```json
{"Queue":{"action":{"List":null}}}
```

Response:

```json
{"QueueState":{"version":1,"tracks":[...],"cursor":0}}
```

### Add single file

```json
{"Queue":{"action":{"Add":{"path":"/path/to/track.opus","position":null}}}}
```

If `position` is `null`, the track is appended. If a number, it is inserted at that index.
Response: `{"Ok":{"version":1}}`

### Add multiple files

```json
{"Queue":{"action":{"AddMany":{"paths":["/path/to/a.opus","/path/to/b.mp3"]}}}}
```

All tracks are appended. Response: `{"Ok":{"version":1}}`

### Add folder (recursive scan)

Scans for audio files (`mp3`, `flac`, `ogg`, `wav`, `m4a`, `aac`, `opus`, `wma`):

```json
{"Queue":{"action":{"AddFolder":{"path":"/home/user/Music/Album"}}}}
```

If the queue was empty and stopped, playback starts automatically.
Response: `{"Ok":{"version":1}}`

### Clear queue

```json
{"Queue":{"action":{"Clear":null}}}
```

Response: `{"Ok":{"version":1}}`

### Remove at index

```json
{"Queue":{"action":{"Remove":{"index":0}}}}
```

Response: `{"Ok":{"version":1}}`

### Move track

```json
{"Queue":{"action":{"Move":{"from":0,"to":2}}}}
```

Response: `{"Ok":{"version":1}}`

### Set queue (replace entire queue)

```json
{"Queue":{"action":{"Set":{"paths":["/path/to/a.opus","/path/to/b.flac"],"start_idx":0}}}}
```

Response: `{"Ok":{"version":1}}`

---

## 3. Library Commands

All library commands use the `Library` wrapper:

```json
{"Library":{"action":{"Scan":{"path":"/home/user/Music"}}}}
```

### Scan

```json
{"Library":{"action":{"Scan":{"path":"/home/user/Music"}}}}
```

### Get Tracks

```json
{"Library":{"action":{"GetTracks":{"filter":null,"sort":null}}}}
```

### Get Playlists

```json
{"Library":{"action":{"GetPlaylists":null}}}
```

### Create Playlist

```json
{"Library":{"action":{"CreatePlaylist":{"name":"Favourites"}}}}
```

### Delete Playlist

```json
{"Library":{"action":{"DeletePlaylist":{"id":1}}}}
```

### Add to Playlist

```json
{"Library":{"action":{"AddToPlaylist":{"playlist_id":1,"track_ids":[1,2,3]}}}}
```

### Import M3U

```json
{"Library":{"action":{"ImportM3u":{"path":"/home/user/playlist.m3u"}}}}
```

### Get Recent

```json
{"Library":{"action":{"GetRecent":{"count":10}}}}
```

---

## 4. Search & Favourites

### Search

```json
{"Search":{"query":"artist name"}}
```

### Get Favourites

```json
{"GetFavourites":null}
```

### Add Favourite

```json
{"AddFavourite":{"track_id":42}}
```

### Remove Favourite

```json
{"RemoveFavourite":{"track_id":42}}
```

---

## 5. YouTube Commands

### YouTube Search

```json
{"YtSearch":{"query":"lofi hip hop","filter":null}}
```

Optional filter: `"Song"`, `"Video"`, `"Playlist"`, `"Channel"`.

### YouTube Search Poll

```json
{"YtSearchPoll":null}
```

Retrieve results from an async YouTube search.

### YouTube Search Cancel

```json
{"YtSearchCancel":null}
```

### YouTube Resolve Stream

```json
{"YtResolveStream":{"url":"https://youtube.com/watch?v=..."}}
```

Resolve a YouTube URL to a playable stream.

---

## 6. System Commands

### Get Status

```json
{"GetStatus":null}
```

Response: `{"Status":{"version":1,"state":{...}}}`

Returns the full `DaemonState` including current track, position, volume, queue, etc.

### Ping

```json
{"Ping":null}
```

Response: `{"Pong":null}`

### Quit

```json
{"Quit":null}
```

Stops playback and exits the daemon process.

---

## Events

The daemon pushes binary-framed `DaemonEvent` notifications. Each frame is:

```
[4-byte BE length][bincode WireFrame]
```

Where `WireFrame` contains one or more `DaemonEvent` values:

| Event | Description |
|---|---|
| `PlaybackStarted` | A track began playing |
| `PlaybackPaused` | Playback was paused |
| `PlaybackStopped` | Playback was stopped |
| `TrackEnded` | The current track finished |
| `PositionChanged` | Playback position updated (frequent) |
| `DurationChanged` | Track duration detected |
| `VolumeChanged` | Volume was changed |
| `QueueChanged` | Queue contents changed |
| `QueueIndexChanged` | Queue cursor moved |
| `RepeatModeChanged` | Repeat mode changed |
| `ShuffleChanged` | Shuffle toggled |
| `SleepTimerTick` | Sleep timer countdown |
| `CrossfadeChanged` | Crossfade config toggled |
| `EqPresetChanged` | EQ preset changed |
| `Custom` | Application-defined event |

---

## Error Responses

Commands that fail return:

```json
{"Error":{"version":1,"message":"description of the error"}}
```

---

## Shell Examples

Using `socat` to interact with the daemon:

```bash
# Connect
socat - UNIX-CONNECT:/run/user/1000/gtmd.socket

# Ping
{"Ping":null}

# Play a file
{"Play":{"path":"/home/user/Music/track.opus","start_pos":0.0}}

# Get status
{"GetStatus":null}

# List queue
{"Queue":{"action":{"List":null}}}

# Quit daemon
{"Quit":null}
```

Using the `gtm` CLI:

```bash
# Status
gtm status

# Play
gtm play /path/to/track.opus

# Toggle play/pause
gtm play-pause

# Next track
gtm next

# Set volume
gtm volume 75

# Add folder to queue
gtm queue-add-folder /home/user/Music/Album

# List queue
gtm queue
```
