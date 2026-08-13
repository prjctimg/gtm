# gtm-rs

[![Release](https://github.com/skchr/gtm-rs/actions/workflows/release.yml/badge.svg?branch=main)](https://github.com/skchr/gtm-rs/actions/workflows/release.yml)
[![License](https://img.shields.io/badge/license-GPLv3-blue.svg)](LICENSE)
[![Rust](https://img.shields.io/badge/rustc-1.81+-orange.svg)](https://www.rust-lang.org)
[![Size](https://img.shields.io/github/repo-size/skchr/gtm-rs)](https://github.com/skchr/gtm-rs)

A modular terminal-based music player daemon and client suite written in Rust.

## Crate Overview

| Crate | Description |
|---|---|
| `gtm-core` | Shared types, IPC protocol, state machine, and `DaemonClient` |
| `gtm-audio` | Audio playback backend (rodio + symphonia) |
| `gtmd` | Daemon — manages queue, library, IPC socket |
| `gtm` | Client — Terminal UI (TUI) and command-line interface |
| `gtm-mpris` | MPRIS D-Bus interface (optional) |

## Build

Requires Rust 1.81+.

```bash
# Build everything
cargo build --release

# Run tests
cargo test --workspace
```

### Feature flags

- `gtmd` — `mpris` (default): enables the MPRIS D-Bus interface
- `gtm` — `completions`: generates shell completions
- `gtm-core` — `debug-fail`: enables debug/test failure injection

## Usage

```bash
# Start the daemon
gtmd

# Terminal UI (default, no args)
gtm

# CLI mode (-c flag)
gtm -c status
gtm -c play /path/to/track.opus
gtm -c next
```

## IPC Protocol

See [`docs/ipc-protocol.md`](docs/ipc-protocol.md) for the full protocol reference.

## License

GPL-3.0-only
