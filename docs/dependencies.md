# Dependencies

gtm is built on the Rust open-source ecosystem. This page lists the direct
runtime and development dependencies declared in `gtm/Cargo.toml` and
`gtmd/Cargo.toml`, grouped by what they do. Versions are pinned in those
manifests; transitive dependencies are resolved by Cargo and can be inspected
with `cargo tree`.

## Terminal interface

- [ratatui](https://ratatui.rs) — immediate-mode terminal UI, layout, and
  wrapping.
- [crossterm](https://github.com/crossterm-rs/crossterm) — terminal input,
  alternate screen, and bracketed paste.
- [tui-big-text](https://crates.io/crates/tui-big-text) — the big-text widget
  used for the logo and large headings (the ratatui big-text crate).
- [tachyonfx](https://crates.io/crates/tachyonfx) — terminal UI effects and
  transitions.
- [ratatui-image](https://crates.io/crates/ratatui-image) — cover art over the
  Kitty, Sixel, iTerm2, and half-block protocols.
- [image](https://crates.io/crates/image) — image decoding, resizing, and
  format handling.
- [color-thief](https://crates.io/crates/color-thief) — cover-art palette
  extraction for reactive themes.

## Audio and media

- [rodio](https://crates.io/crates/rodio) — audio output and playback.
- [symphonia](https://crates.io/crates/symphonia) — audio demuxing, decoding,
  and metadata.
- [symphonia-adapter-libopus](https://crates.io/crates/symphonia-adapter-libopus)
  — Opus decoding for podcast and radio streams.
- [fundsp](https://crates.io/crates/fundsp) — DSP building blocks for the
  equalizer, reverb, and crossfade paths.
- [rustfft](https://crates.io/crates/rustfft) — FFTs for the spectrum display.
- [lofty](https://crates.io/crates/lofty) — audio tags, cover art, and text
  metadata.
- [librespot-core](https://github.com/librespot-org/librespot) and
  [librespot-playback](https://github.com/librespot-org/librespot) — Spotify
  Connect playback and local audio streaming.
- [rspotify](https://github.com/ramsayleung/rspotify) — Spotify Web API client
  for search, playlists, and account metadata.
- [innertube-rs](https://crates.io/crates/innertube-rs) — YouTube/InnerTube
  search and resolution for free-tier playback.
- [pulseaudio](https://crates.io/crates/pulseaudio) — optional native
  PulseAudio output backend.

## Networking and async

- [tokio](https://tokio.rs) — asynchronous runtime, sockets, timers, and
  subprocesses.
- [tokio-util](https://crates.io/crates/tokio-util) — codecs and utilities for
  async streams.
- [futures](https://crates.io/crates/futures) — stream and future combinators.
- [reqwest](https://crates.io/crates/reqwest) — HTTP client with rustls and
  streaming support.
- [async-trait](https://crates.io/crates/async-trait) — async trait methods for
  the provider interfaces.

## Data, storage, and serialization

- [serde](https://serde.rs) and
  [serde_json](https://crates.io/crates/serde_json) — IPC and provider JSON.
- [toml](https://crates.io/crates/toml) — configuration files.
- [rmp-serde](https://crates.io/crates/rmp-serde) — compact MessagePack
  persistence.
- [quick-xml](https://crates.io/crates/quick-xml) — RSS, podcast, and XML
  feeds.
- [rusqlite](https://crates.io/crates/rusqlite) — bundled SQLite library
  index.
- [walkdir](https://crates.io/crates/walkdir) — recursive library scans.
- [lru](https://crates.io/crates/lru) — bounded in-memory caches.
- [uuid](https://crates.io/crates/uuid) — stable identifiers.
- [chrono](https://crates.io/crates/chrono) — dates and timestamps.
- [regex](https://crates.io/crates/regex) — parsing titles, feed text, and
  provider responses.
- [urlencoding](https://crates.io/crates/urlencoding) — query and path
  encoding.
- [base64](https://crates.io/crates/base64) — encoded image and wire payloads.
- [hex](https://crates.io/crates/hex), [md5](https://crates.io/crates/md5),
  and [sha2](https://crates.io/crates/sha2) — hashes and cache keys.
- [bincode](https://crates.io/crates/bincode) — test-only serialization
  fixtures.

## Platform integration

- [zbus](https://crates.io/crates/zbus) and
  [zvariant](https://crates.io/crates/zvariant) — optional MPRIS D-Bus
  integration.
- [keyring](https://crates.io/crates/keyring) — operating-system credential
  storage.
- [webbrowser](https://crates.io/crates/webbrowser) — browser-based OAuth and
  device-code flows.
- [dirs](https://crates.io/crates/dirs) — XDG and platform configuration
  paths.
- [libc](https://crates.io/crates/libc) — POSIX process and socket details.
- [rpassword](https://crates.io/crates/rpassword) — hidden credential prompts.

## CLI, configuration, and diagnostics

- [clap](https://crates.io/crates/clap) and
  [clap_complete](https://crates.io/crates/clap_complete) — command-line
  parsing, help, and shell completions.
- [color-eyre](https://crates.io/crates/color-eyre) — user-facing error
  reports.
- [thiserror](https://crates.io/crates/thiserror) — typed library errors.
- [tracing](https://crates.io/crates/tracing),
  [tracing-subscriber](https://crates.io/crates/tracing-subscriber), and
  [log](https://crates.io/crates/log) — structured logs and log bridging.
- [fastrand](https://crates.io/crates/fastrand),
  [getrandom](https://crates.io/crates/getrandom), and
  [rustc-hash](https://crates.io/crates/rustc-hash) — randomized identifiers,
  jitter, and the optional debug-fail test switch.

## External programs and services

- [yt-dlp](https://github.com/yt-dlp/yt-dlp) — external extractor/downloader
  for YouTube and yt-dlp-supported sites.
- [FFmpeg](https://ffmpeg.org) — optional external converter used when tagging
  media that the built-in muxers cannot write.
- [LRCLIB](https://lrclib.net) — synced and plain lyrics.
- [Radio Browser](https://www.radio-browser.info) and
  [cliamp](https://www.cliamp.stream) — internet radio directories.
- [Last.fm](https://www.last.fm) — scrobbling and loved tracks.
- [Spotify](https://developer.spotify.com) and
  [YouTube](https://www.youtube.com) — streaming providers.
