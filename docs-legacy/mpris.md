# MPRIS — Media Player Remote Interfacing Specification

This document outlines how `gtmd` (the daemon) can become an MPRIS-compliant
media player so that desktop environments (GNOME, KDE, etc.), media key
keyboards, and D-Bus aware tools can discover and control it.

---

## What is MPRIS?

MPRIS is a freedesktop.org D-Bus interface specification
([spec](https://specifications.freedesktop.org/mpris-spec/latest/)) that
standardises how media players expose their state and accept control commands
over the session D-Bus.

A player that is **MPRIS-compliant** registers two D-Bus objects:

| Object path | Interface | Purpose |
|---|---|---|
| `/org/mpris/MediaPlayer2` | `org.mpris.MediaPlayer2` | Root — identity, quit, raise |
| `/org/mpris/MediaPlayer2` | `org.mpris.MediaPlayer2.Player` | Playback — play/pause/next/seek/metadata |

The bus name is `org.mpris.MediaPlayer2.<instance>`, where `<instance>` is
typically the program name (e.g. `gtm`). Other media players on the same bus
each have their own unique bus name, which is how desktop shell widgets discover
them.

---

## Implementation Strategy

### 1. Add a D-Bus binding

`gtmd` needs to speak the D-Bus protocol. Options (in order of preference):

| Option | Approach | Notes |
|---|---|---|
| **libdbus-1** | Call C `libdbus-1` directly via `{.importc.}` | Zero new deps; `libdbus-1` is installed on virtually every Linux desktop |
| **nimdbus** | Pure-Nim D-Bus library | Avoids C interop but adds a dependency |
| **dbus_nim** | Thin wrapper over libdbus-1 | Nimble package, maintained |

**Recommendation**: use raw `libdbus-1` FFI for minimum dependency overhead,
exposing only the subset of D-Bus that MPRIS needs (name request, method
handling, property get/set, signal emission).

### 2. Register on the session bus

On startup, `gtmd` calls:

```
dbus_bus_request_name(conn, "org.mpris.MediaPlayer2.gtm",
                      DBUS_NAME_FLAG_DO_NOT_QUEUE, error)
```

### 3. Expose D-Bus object paths and interfaces

#### `/org/mpris/MediaPlayer2` — `org.mpris.MediaPlayer2`

| Method/Property | When called | Maps to daemon IPC |
|---|---|---|
| `Quit` | User quits from desktop shell | `quit` |
| `Raise` | User clicks player in shell | (optional — focus TUI) |
| `Identity` (property) | Shell queries player name | Return `"gtm"` |
| `DesktopEntry` (property) | Shell looks up .desktop file | Return `"gtm"` |
| `SupportedUriSchemes` | Shell checks protocol support | Return `["file", "http", "https"]` |
| `SupportedMimeTypes` | Shell checks format support | Return `["audio/flac", "audio/mpeg", …]` |

#### `/org/mpris/MediaPlayer2` — `org.mpris.MediaPlayer2.Player`

| Method | Maps to daemon IPC |
|---|---|
| `Play` | `resume` |
| `Pause` | `pause` |
| `PlayPause` | `toggle_pause` |
| `Stop` | `stop` |
| `Next` | `next` |
| `Previous` | `prev` |
| `Seek(x: Int64)` | `seek` (microseconds → sec) |
| `SetPosition(TrackId, x: Int64)` | `seek` (absolute) |
| `OpenUri(uri: String)` | `load_file` |

| Property | Source in daemon state | Notes |
|---|---|---|
| `PlaybackStatus` | `state` field | `"Playing"`, `"Paused"`, or `"Stopped"` |
| `LoopStatus` | `repeat` field | `"None"`, `"Playlist"`, or `"Track"` |
| `Rate` | hardcoded `1.0` | No playback speed change planned |
| `Metadata` | `track_title`, `track_channel`, `track_album`, `duration`, `track_path` | See metadata map below |
| `Volume` | `volume` field | `0.0` – `1.0` (normalised) |
| `Position` | `time_pos` from events | Microseconds |
| `MinimumRate` / `MaximumRate` | hardcoded `1.0` / `1.0` | |
| `CanGoNext` / `CanGoPrevious` | `true` / `true` | |
| `CanPlay` / `CanPause` | `true` / `true` | Depends on audio backend |
| `CanSeek` | `true` | |
| `CanControl` | `true` | |

| Signal | When fired |
|---|---|
| `PropertiesChanged` | On any property change (playback state, volume, metadata, etc.) |
| `Seeked(x: Int64)` | After seek completes |

### 4. Metadata mapping

| MPRIS metadata key | Daemon state field |
|---|---|
| `mpris:trackid` | Hash of `track_path` |
| `mpris:length` | `duration * 1000000` (microseconds) |
| `mpris:artUrl` | (optional — could point to local album art) |
| `xesam:title` | `track_title` |
| `xesam:artist` | `track_channel` as `["Artist"]` array |
| `xesam:album` | `track_album` |
| `xesam:url` | `track_path` |

### 5. Integration point in gtmd

The daemon event loop currently lives in `src/daemon.nim`. The D-Bus
integration should:

1. Initialise a `dbus_connection` in `initDaemon()` / `runDaemon()`
2. Register the two MPRIS interface handler tables (`DBusObjectPathVTable`)
3. **Dispatch**: each iteration of the main `select()` loop should also call
   `dbus_connection_read_write_dispatch()` with a 0 ms timeout
4. **Property updates**: after every state change (track change, volume,
   play/pause) emit `PropertiesChanged` on the `Player` interface
5. **Incoming method calls**: map directly to the same internal IPC dispatch
   table that the Unix socket handler uses — reuse the existing command
   implementations

### 6. Thread safety

`libdbus-1` is **not** re-entrant and must only be called from the main
thread. Since `gtmd` already does all I/O on the main thread (single-threaded
`select()` loop), this is safe. The audio decode thread(s) must not touch the
D-Bus connection.

---

## Testing

| Test | What to verify |
|---|---|
| Bus registration | `dbus-send --session --dest=org.freedesktop.DBus --type=method_call --print-reply /org/freedesktop/DBus org.freedesktop.DBus.ListNames` includes `org.mpris.MediaPlayer2.gtm` |
| Property read | `dbus-send --session --dest=org.mpris.MediaPlayer2.gtm --type=method_call --print-reply /org/mpris/MediaPlayer2 org.freedesktop.DBus.Properties.Get string:"org.mpris.MediaPlayer2.Player" string:"PlaybackStatus"` |
| Method call | `dbus-send --session --dest=org.mpris.MediaPlayer2.gtm --type=method_call /org/mpris/MediaPlayer2 org.mpris.MediaPlayer2.Player.Play` |
| Desktop shell widget | Launch `gtmd` and check that GNOME/KDE media controls show the player |

---

## References

- [MPRIS 2.2 specification](https://specifications.freedesktop.org/mpris-spec/latest/)
- [D-Bus specification](https://dbus.freedesktop.org/doc/dbus-specification.html)
- [libdbus-1 API reference](https://dbus.freedesktop.org/doc/api/html/)
