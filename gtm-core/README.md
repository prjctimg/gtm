# gtm-core

Shared types for the gtm workspace:

- IPC protocol: `DaemonRequest`, `DaemonResponse`, `DaemonEvent`, `QueueAction`, `LibraryAction`
- Binary wire framing: `encode_frame` / `decode_frame`
- Track and playlist models: `TrackInfo`, `Playlist`, `LrcLine`, `LrcData`, `RepeatMode`
- Daemon state: `DaemonState`, `PlaybackStatus`, `CrossfadeConfig`

Every other crate in the workspace depends on this one.