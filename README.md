# gtm.rs 🦀

[![Version](https://img.shields.io/github/v/release/prjctimg/gtm)](https://github.com/prjctimg/gtm.rs/releases/latest)
[![Rust](https://img.shields.io/badge/rustc-1.81+-orange.svg)](https://www.rust-lang.org)

Feature rich terminal audio player with background playback support and YouTube/Spotify integration. Rust implementation of the [gtm spec](https://github.com/prjctimg/gtm.spec).

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
- [gtm-config(1)](https://github.com/prjctimg/gtm.spec.wiki/Manpages)
- [gtm-keybindings(1)](https://github.com/prjctimg/gtm.spec.wiki/Manpages)

## Dependencies

The Rust implementation relies on the following crates:

| Crate | External dependencies |
|---|---|
| `gtm-core` | serde, serde_json, rmp-serde, chrono, libc, thiserror, uuid, tokio, tracing |
| `gtm-audio` | rodio, symphonia, symphonia-adapter-libopus, fundsp, thiserror, log, pulseaudio (optional) |
| `gtmd` | rusqlite, tokio, tokio-util, futures, reqwest, rspotify, clap, sha2, walkdir, symphonia, lru, hex, base64, dirs, fastrand, chrono, serde, serde_json, uuid, tracing, tracing-subscriber |
| `gtm` | ratatui, crossterm, tokio, clap, color-eyre, image, ratatui-image, base64, chrono, serde, serde_json, toml, tracing, tracing-subscriber |
| `gtm-mpris` | zbus, zvariant, serde, tracing |

External system libraries required at build time: ALSA development headers (`libasound2-dev` on Debian/Ubuntu) and, for the optional MPRIS feature, a D-Bus development library.

## Contributing

I'm currently unable to handle external contributions because I'm actively working on it and any bugs or issues you may notice  may well already be noted. Also I am doing this for fun and learning reasons.

Feel free to [fork off](https://github.com/prjctimg/gtm.rs/fork) though and on your way out don't forget to  checkout the [gtm spec](https://github.com/prjctimg/gtm.spec) for some domain specific notes on the reasons why the code is structured as it is.

---

> ## License 📜
>
> (c) 2025 - present, [prjctimg](https://prjctimg.me)
>
> This is free software, released under the GPL-3.0 license.

---
