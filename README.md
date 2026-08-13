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
| `gtm-cli` | Command-line client for the daemon |
| `gtm-tui` | Terminal UI client (ratatui) |
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
- `gtm-cli` — `completions`: generates shell completions
- `gtm-core` — `debug-fail`: enables debug/test failure injection

## Usage

```bash
# Start the daemon
gtmd

# CLI client
gtm status
gtm play /path/to/track.opus
gtm next

# Terminal UI
gtm-tui
```

## IPC Protocol

See [`docs/ipc-protocol.md`](docs/ipc-protocol.md) for the full protocol reference.

## License

GPL-3.0-only
