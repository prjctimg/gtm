# gtmd

Background audio daemon for [gtm](https://github.com/prjctimg/gtm.rs) — the
feature-rich cross-platform terminal audio player.

`gtmd` runs as a headless daemon: it owns the audio output (local files,
YouTube, Spotify, radio, podcasts), the library database, cover
artwork and lyrics caching, scrobbling, and the MPRIS D-Bus interface. The
`gtm` TUI/CLI connects to it over a Unix socket and drives playback through
the IPC protocol defined in `gtm::shared::ipc`.

This crate is intentionally thin: it depends on the `gtm` library crate for
the shared types, the audio mixer backends, and the MPRIS module, so the
daemon and the client can never drift apart in their wire protocol.

## Build

```sh
cargo build --release -p gtmd
```

The `gtmd` binary is normally installed together with `gtm`:

```sh
cargo install gtm gtmd
```