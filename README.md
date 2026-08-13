# gtm.rs 🦀

[![CI](https://img.shields.io/github/actions/workflow/status/prjctimg/gtm.rs/ci.yml)](https://github.com/prjctimg/gtm.rs/actions/workflows/ci.yml)
[![Version](https://img.shields.io/github/v/release/prjctimg/gtm.rs)](https://github.com/prjctimg/gtm.rs/releases/latest)
[![License](https://img.shields.io/github/license/prjctimg/gtm.rs)](https://github.com/prjctimg/gtm.rs/blob/main/LICENSE)
[![Rust](https://img.shields.io/badge/rustc-1.81+-orange.svg)](https://www.rust-lang.org)

> Feature rich terminal audio player 📻  with background playback support and YouTube/Spotify integration. Rust implementation of the
> [gtm spec](https://github.com/prjctimg/gtm.spec).

<img src="assets/screenshots/library.png" alt="gtm library view" width="640">

## Features

- **Background playback** - reattach to the client from anywhere in the terminal
- **YouTube, Spotify, Deezer** integration — search, download, and resolve
  tracks, sync Spotify playlists, and fetch missing metadata/lyrics/cover art.
- **Equalizer** — 16 presets plus per-band gain, headroom stage, and a
  spectrum visualizer.
- **Cover art** — rendered inline via the kitty/terminal image protocol.
- **Zero configuration** - Sane defaults,fully customizable via TOML.Has 12 built-in themes and various widget styles.
- **MPRIS** — media player controls via D-Bus.

## Install

```bash
curl -fsSL https://raw.githubusercontent.com/prjctimg/gtm.spec/main/install.sh | bash -s -- --impl rust
```

### GitHub Release

> [!note]
>
> Each `gtm-{platform}.tar.gz` bundles both binaries
(`bin/gtm`, `bin/gtmd`), man pages, shell completions, and the `systemd` user service.
>
> Substitute `x86_64-linux` for `aarch64-linux`, `aarch64-linux-musl` (musl), `aarch64-darwin` or `aarch64-android` for as needed.
>

Grab one from the [releases page](https://github.com/prjctimg/gtm.rs/releases/latest).

```bash
curl -fsSLO https://github.com/prjctimg/gtm.rs/releases/latest/download/gtm-x86_64-linux.tar.gz
tar xzf gtm-x86_64-linux.tar.gz
cd gtm-x86_64-linux

sudo install -Dm755 bin/gtm  /usr/local/bin/gtm
sudo install -Dm755 bin/gtmd /usr/local/bin/gtmd
sudo install -Dm644 man/man1/gtm.1 man/man1/gtmd.1 man/man1/gtmd-ipc.1 /usr/local/share/man/man1/
sudo install -Dm644 systemd/gtmd.service /usr/local/lib/systemd/user/gtmd.service
sudo install -Dm644 completions/gtm.bash completions/gtmd.bash /usr/local/share/bash-completion/completions/
sudo install -Dm644 completions/_gtm completions/_gtmd /usr/local/share/zsh/vendor-completions/
sudo install -Dm644 completions/gtm.fish completions/gtmd.fish /usr/local/share/fish/vendor_completions.d/

systemctl --user daemon-reload
systemctl --user enable --now gtmd
```

### Build from Source

Requires Rust 1.81+ and ALSA development headers (`libasound2-dev` on Debian/Ubuntu).

```bash
cargo build --release
sudo make install
```

## Screenshots

<img src="assets/screenshots/command-palette.png" alt="gtm command palette" width="640">

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
