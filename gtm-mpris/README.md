# gtm-mpris

MPRIS D-Bus server for gtm. Exposes daemon playback state and controls via the
[MPRIS specification](https://specifications.freedesktop.org/mpris-spec/latest/),
allowing media keys, desktop environment lock screen controls, and tools like
`playerctl` to control gtm.

## Architecture

Serves two D-Bus interfaces on `org.mpris.MediaPlayer2.gtm`:

- **`org.mpris.MediaPlayer2`**: Identity, Quit, CanQuit
- **`org.mpris.MediaPlayer2.Player`**: PlaybackStatus, Metadata, Volume, Position, Seek, Next, Previous, PlayPause

Daemon events are bridged to D-Bus `PropertiesChanged` signals in real-time.

## Dependencies

`gtm-core`, `zbus` (tokio), `zvariant`, `tracing`

Optional: compile with `gtmd --features mpris` (enabled by default).
