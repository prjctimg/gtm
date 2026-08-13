# gtm.rs

[![Release](https://github.com/prjctimg/gtm.rs/actions/workflows/release.yml/badge.svg?branch=main)](https://github.com/prjctimg/gtm.rs/actions/workflows/release.yml)
[![GitHub Release](https://img.shields.io/github/v/release/prjctimg/gtm.rs)](https://github.com/prjctimg/gtm.rs/releases/latest)
[![License](https://img.shields.io/badge/license-GPLv3-blue.svg)](LICENSE)
[![Rust](https://img.shields.io/badge/rustc-1.81+-orange.svg)](https://www.rust-lang.org)

Terminal music player with daemon-client architecture. Rust implementation of the GTM protocol.

## Quick Install

```bash
curl -fsSL https://raw.githubusercontent.com/prjctimg/gtm.rs/main/install.sh | bash
```

## Build from Source

Requires Rust 1.81+ and ALSA development headers (`libasound2-dev` on Debian/Ubuntu).

```bash
cargo build --release
sudo make install
```

## Crates

| Crate | Description |
|---|---|
| `gtm-core` | Shared types, IPC protocol, state machine, `DaemonClient` |
| `gtm-audio` | Audio playback backend (rodio + symphonia) |
| `gtmd` | Daemon — manages queue, library, IPC socket |
| `gtm` | Client — TUI and CLI interface |
| `gtm-mpris` | MPRIS D-Bus interface (optional) |

## Documentation

- [Wiki](https://github.com/prjctimg/gtm.rs/wiki)
- [Protocol docs](https://github.com/prjctimg/gtm.spec)
- [Configuration](https://github.com/prjctimg/gtm.spec/blob/main/man/gtm-config.1.md)
- [TUI keybindings](https://github.com/prjctimg/gtm.spec/blob/main/man/gtm-keybindings.1.md)

## License

GPL-3.0-only

---

> ## License 📜
>
> (c) 2025 - present, [prjctimg](https://prjctimg.me)
>
> This is free software, released under the GPL-3.0 license.

---
