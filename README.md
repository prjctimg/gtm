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

Requires Rust 1.81+, ALSA development headers (Linux, `libasound2-dev` on Debian/Ubuntu).

```bash
# Build everything
cargo build --release

# Run tests
cargo test --workspace
```

### Feature flags

- `gtmd` — `mpris` (default): enables the MPRIS D-Bus interface
- `gtm-core` — `debug-fail`: enables debug/test failure injection

## Install

### From source

```bash
make && sudo make install
```

Or manually:

```bash
cargo build --release
sudo install -Dm 0755 target/release/gtmd /usr/local/bin/gtmd
sudo install -Dm 0755 target/release/gtm  /usr/local/bin/gtm
```

### Debian / Ubuntu (.deb)

Requires [`cargo-deb`](https://crates.io/crates/cargo-deb).

```bash
make deb
sudo dpkg -i target/debian/gtm-full_*.deb
```

### RPM-based (Fedora, RHEL)

Requires `rpmbuild`.

```bash
make rpm
sudo rpm -i ~/rpmbuild/RPMS/*/gtm-*.rpm
```

### Nix

```bash
nix build
./result/bin/gtmd
```

### Systemd user service

After installation, enable the daemon for your user session:

```bash
systemctl --user enable --now gtmd.service
```

## Usage

```bash
# Start the daemon
gtmd

# Terminal UI (default, no args)
gtm

# CLI mode (--cli flag)
gtm --cli status
gtm --cli play /path/to/track.opus
gtm --cli next
```

## IPC Protocol

See [`docs/ipc-protocol.md`](docs/ipc-protocol.md) for the full protocol reference.

## License

GPL-3.0-only
