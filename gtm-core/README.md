# gtm-core

Shared types for the gtm workspace. Every crate depends on this.

## Contents

- **`ipc.rs`** — `DaemonRequest`, `DaemonResponse`, `DaemonEvent` enums, `QueueAction`, `LibraryAction`
- **`wire.rs`** — Binary frame encoding/decoding (`encode_frame`, `decode_frame`)
- **`track.rs`** — `TrackInfo`, `Playlist`, `LrcLine`, `LrcData`, `RepeatMode`
- **`state.rs`** — `DaemonState`, `PlaybackStatus`, `CrossfadeConfig`

## Dependencies

`serde`, `serde_json`, `bincode`, `thiserror`, `chrono`, `uuid`
