% GTMD-IPC(1) GTM IPC Protocol Manual
% prjctimg
% 2025

# NAME

gtmd-ipc - IPC protocol for the GTM music daemon

# DESCRIPTION

The GTM daemon (**gtmd**(1)) communicates with clients on a Unix socket
using a mixed JSON+binary protocol.  Commands are sent as newline-delimited
JSON **DaemonReq** objects.  The daemon responds synchronously with a
**DaemonRes** JSON object per request, interleaved with asynchronous
binary-framed **DaemonEvent** notifications.

# FRAMING

**Requests** are a single JSON line terminated with `\\n`:

```
{"Ping":null}
```

**Responses** are a single JSON line terminated with `\\n`:

```
{"Pong":null}
```

**Events** are binary frames with a 4-byte big-endian length prefix followed
by a bincode-encoded `WireFrame` payload.

The client distinguishes responses from events by the first byte:
- `0x7B` (`{`) — JSON response (read until `\\n`)
- anything else — binary event frame (read 4-byte length, then payload)

# COMMON RESPONSES

Many commands return:

```
{"Ok":{"version":<u32>}}
```

On error:

```
{"Error":{"version":<u32>,"message":"..."}}
```

# PLAYBACK COMMANDS

## Play

```
{"Play":{"path":"/path/to/file.opus","start_pos":0.0}}
```

Response: `{"Ok":{"version":1}}`

Emits: `PlaybackStarted` event.

## PlayPause

```
{"PlayPause":null}
```

Response: `{"Ok":{"version":1}}`

Toggles between playing and paused state.

## Pause

```
{"Pause":null}
```

Response: `{"Ok":{"version":1}}`

## Stop

```
{"Stop":null}
```

Response: `{"Ok":{"version":1}}`

## Next / Prev

```
{"Next":null}
{"Prev":null}
```

Response: `{"Ok":{"version":1}}`

## Seek

```
{"Seek":{"position_secs":30.0}}
```

Response: `{"Ok":{"version":1}}`

## SetVolume

```
{"SetVolume":{"volume":75}}
```

Response: `{"Ok":{"version":1}}`

Volume range: 0–100.

## ToggleShuffle

```
{"ToggleShuffle":null}
```

Response: `{"Ok":{"version":1}}`

## CycleRepeat

```
{"CycleRepeat":{"mode":"All"}}
```

Response: `{"Ok":{"version":1}}`

Modes: `Off`, `One`, `All`.

## ToggleMute

```
{"ToggleMute":null}
```

Response: `{"Ok":{"version":1}}`

## Crossfade

```
{"Crossfade":{"enabled":true,"duration_secs":3}}
```

Response: `{"Ok":{"version":1}}`

# QUEUE COMMANDS

## Queue List

```
{"Queue":{"action":"List"}}
```

Response:

```
{"QueueState":{"version":1,"tracks":[...],"cursor":0}}
```

## Queue Add

```
{"Queue":{"action":{"Add":{"path":"/path/to/file.opus","position":null}}}}
```

Response: `{"Ok":{"version":1}}`

## Queue AddMany

```
{"Queue":{"action":{"AddMany":{"paths":["/a.opus","/b.opus"]}}}}
```

Response: `{"Ok":{"version":1}}`

## Queue AddFolder

```
{"Queue":{"action":{"AddFolder":{"path":"/path/to/music/"}}}}
```

Response: `{"Ok":{"version":1}}`

## Queue Remove

```
{"Queue":{"action":{"Remove":{"index":2}}}}
```

Response: `{"Ok":{"version":1}}`

## Queue Move

```
{"Queue":{"action":{"Move":{"from":3,"to":1}}}}
```

Response: `{"Ok":{"version":1}}`

## Queue Clear

```
{"Queue":{"action":"Clear"}}
```

Response: `{"Ok":{"version":1}}`

## Queue Set

```
{"Queue":{"action":{"Set":{"paths":["/a.opus"],"start_idx":0}}}}
```

Response: `{"Ok":{"version":1}}`

# LIBRARY COMMANDS

## Library Scan

```
{"Library":{"action":{"Scan":{"path":"/path/to/music"}}}}
```

Response: `{"Ok":{"version":1}}`

## Library GetTracks

```
{"Library":{"action":{"GetTracks":{"filter":null,"sort":null}}}}
```

Response:

```
{"Tracks":{"version":1,"tracks":[...]}}
```

## Library GetPlaylists

```
{"Library":{"action":"GetPlaylists"}}
```

Response:

```
{"Playlists":{"version":1,"playlists":[...]}}
```

## Library CreatePlaylist

```
{"Library":{"action":{"CreatePlaylist":{"name":"My Mix"}}}}
```

Response: `{"Ok":{"version":1}}`

## Library DeletePlaylist

```
{"Library":{"action":{"DeletePlaylist":{"id":1}}}}
```

Response: `{"Ok":{"version":1}}`

## Library AddToPlaylist

```
{"Library":{"action":{"AddToPlaylist":{"playlist_id":1,"track_ids":[1,2,3]}}}}
```

Response: `{"Ok":{"version":1}}`

## Library GetRecent

```
{"Library":{"action":{"GetRecent":{"count":10}}}}
```

Response:

```
{"Tracks":{"version":1,"tracks":[...]}}
```

# DISCOVERY COMMANDS

## Search

```
{"Search":{"query":"jazz"}}
```

Response:

```
{"Tracks":{"version":1,"tracks":[...]}}
```

## GetFavourites

```
{"GetFavourites":null}
```

Response:

```
{"Tracks":{"version":1,"tracks":[...]}}
```

## AddFavourite / RemoveFavourite

```
{"AddFavourite":{"track_id":42}}
{"RemoveFavourite":{"track_id":42}}
```

Response: `{"Ok":{"version":1}}`

## YtSearch

```
{"YtSearch":{"query":"lofi jazz","filter":null}}
```

Response: `{"Ok":{"version":1}}` (results are polled via YtSearchPoll)

## YtSearchPoll

```
{"YtSearchPoll":null}
```

Response:

```
{"YtSearchResults":{"version":1,"results":[...]}}
```

## YtSearchCancel

```
{"YtSearchCancel":null}
```

Response: `{"Ok":{"version":1}}`

## YtResolveStream

```
{"YtResolveStream":{"url":"https://youtube.com/watch?v=..."}}
```

Response:

```
{"StreamInfo":{"version":1,"info":{...}}}
```

# SYSTEM COMMANDS

## GetStatus

```
{"GetStatus":null}
```

Response:

```
{"Status":{"version":1,"state":{"version":0,"status":"Stopped",...}}}
```

## Ping

```
{"Ping":null}
```

Response: `{"Pong":null}`

## Quit

```
{"Quit":null}
```

Response: `{"Ok":{"version":1}}`

# EVENTS

Events are received asynchronously as binary bincode frames:

```
DaemonEvent::PlaybackStarted { track, auto_advanced, time_pos, duration }
DaemonEvent::PlaybackPaused
DaemonEvent::PlaybackStopped
DaemonEvent::TrackEnded
DaemonEvent::PositionChanged { time_pos }
DaemonEvent::DurationChanged { duration }
DaemonEvent::VolumeChanged { volume }
DaemonEvent::QueueChanged { queue, cursor }
DaemonEvent::RepeatModeChanged { mode }
DaemonEvent::ShuffleChanged { enabled }
DaemonEvent::SleepTimerTick { remaining_secs }
```

# SEE ALSO

**gtmd**(1), **gtm**(1)
