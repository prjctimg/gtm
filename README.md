# gtm.rs 🦀

[![CI](https://img.shields.io/github/actions/workflow/status/prjctimg/gtm.rs/ci.yml)](https://github.com/prjctimg/gtm.rs/actions/workflows/ci.yml)
[![Version](https://img.shields.io/github/v/release/prjctimg/gtm.rs)](https://github.com/prjctimg/gtm.rs/releases/latest)
[![License](https://img.shields.io/github/license/prjctimg/gtm.rs)](https://github.com/prjctimg/gtm.rs/blob/main/LICENSE)
[![Rust](https://img.shields.io/badge/rustc-1.81+-orange.svg)](https://www.rust-lang.org)

> Feature rich terminal audio player with background playback support and
> YouTube/Spotify integration. Rust implementation of the
> [gtm spec](https://github.com/prjctimg/gtm.spec).

<img src="assets/screenshots/library.png" alt="gtm library view" width="640">

> Demo GIF placeholder — regenerate with VHS, see [`tapes/VHS.md`](tapes/VHS.md).

## Features

- **Background daemon** (`gtmd`) — playback survives terminal close; control it
  from the TUI, the CLI, or MPRIS.
- **YouTube, Spotify, Deezer** integration — search, download, and resolve
  tracks, sync Spotify playlists, and fetch missing metadata/lyrics/cover art.
- **Equalizer** — 16 presets plus per-band gain, headroom stage, and a
  spectrum visualizer.
- **Cover art** — rendered inline via the kitty/terminal image protocol.
- **Themes** — 12 built-in themes (light and dark) plus user TOML themes.
- **MPRIS** — media player controls via D-Bus (optional feature).
- **Termux / Android** — runs on Android via PulseAudio (optional feature).

## Install

You can use the shared [`install.sh`](https://github.com/prjctimg/gtm.spec/blob/main/install.sh) script for an interactive, hassle free installation. It asks which implementation (gtm.rs or gtm.nim) and which install type you want:

```bash
curl -fsSL https://raw.githubusercontent.com/prjctimg/gtm.spec/main/install.sh | bash
```

Or pick this implementation directly:

```bash
curl -fsSL https://raw.githubusercontent.com/prjctimg/gtm.spec/main/install.sh | bash -s -- --impl rust
```

### Pre-built binaries

Grab one for your target system from the [releases page](https://github.com/prjctimg/gtm.rs/releases/latest).

### Build from Source

Requires Rust 1.81+ and ALSA development headers (`libasound2-dev` on Debian/Ubuntu).

```bash
cargo build --release
sudo make install
```

To install just the client binary (for example to try the TUI against an
already-running daemon), or to install without system packages:

```bash
cargo install --path gtm
```

## Screenshots

<img src="assets/screenshots/command-palette.png" alt="gtm command palette" width="640">

## Crates

The audio player is split into separate crates, allowing you to easily pick which component of the player you want to tweak.

| Crate | Description |
|---|---|
| `gtm-core` | Shared types, IPC protocol, state machine, `DaemonClient` |
| `gtm-audio` | Audio playback backend (rodio + symphonia and fundsp) |
| `gtmd` | Daemon — manages queue, library, IPC socket |
| `gtm` | Client — TUI and CLI interface |
| `gtm-mpris` | MPRIS D-Bus interface (optional) |

## Documentation

- [Wiki](https://github.com/prjctimg/gtm.rs/wiki)
- [gtm.spec](https://github.com/prjctimg/gtm.spec)
- [gtm(1)](docs/man/gtm.1.md)
- [gtmd(1)](docs/man/gtmd.1.md)
- [gtmd-ipc(1)](docs/man/gtmd-ipc.1.md)

## Contributing

> [!note]
>
> This is a hobby project but is feature complete and stable enough to use as my daily driver.
> However its still largely a WIP.
>

See [CONTRIBUTING.md](CONTRIBUTING.md) for build instructions, the crate layout and the green gates (fmt / clippy / test).

---

> ## License 📜
>
> (c) 2025 - present, [prjctimg](https://prjctimg.me)
>
> This is free software, released under the GPL-3.0 license.
