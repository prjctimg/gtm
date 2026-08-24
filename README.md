# gtm.rs

![](./logo.png)

[![Crates.io](https://img.shields.io/crates/v/gtm)](https://crates.io/crates/gtm)
[![Crates.io downloads](https://img.shields.io/crates/d/gtm)](https://crates.io/crates/gtm)
[![Docs.rs](https://docs.rs/gtm/badge.svg)](https://docs.rs/gtm)
[![CI](https://img.shields.io/github/actions/workflow/status/prjctimg/gtm.rs/ci.yml)](https://github.com/prjctimg/gtm.rs/actions/workflows/ci.yml)
[![License](https://img.shields.io/github/license/prjctimg/gtm.rs)](https://github.com/prjctimg/gtm.rs/blob/main/LICENSE)

A terminal music player with background playback and YouTube/Spotify
integration. Rust implementation of the [gtm spec](https://github.com/prjctimg/gtm.spec).

## Why another (terminal) audio player ?

I wanted a player that keeps playing in the background while I work in the
terminal, handles local files *and* YouTube/Spotify in one place, and sounds
good through a real EQ instead of a flat toggle. gtm.rs is that.

## Features

- **Background playback**: reattach to the client from anywhere in the terminal
- **YouTube, Spotify, Deezer**: search & download from YouTube, sync
  Spotify playlists, and fetch missing metadata/lyrics/cover art via Deezer.
- **Crossfade**: gapless-ish transitions with duration and easing options.
- **Lyrics**: automatic fetch from LRCLIB (default provider)
- **Metadata sync**: backfill missing tags, cover art, and lyrics for local
  files from the TUI or the CLI (via `gtm lyrics` / `gtm metadata sync`)
- **Equalizer**: 16 presets plus per-band gain, headroom stage, and a spectrum
  visualizer
- **Playlist management**: Import/export `m3u8` playlists
- **Sleep timer**
- **Cover art**: rendered inline via the kitty/terminal image protocol
- **Zero configuration**: sane defaults, fully customizable via TOML,
- **16 built-in themes** : and various widget styles for the visualizer and progress indicator. Supports transparent mode.
- **Reactive theming**: accent colors extracted from the current track
- **MPRIS**: media player controls via D-Bus (planned)

## Install

```bash
# stable (latest)
curl -fsSL https://raw.githubusercontent.com/prjctimg/gtm.rs/main/install.sh | bash
# nightly
curl -fsSL https://raw.githubusercontent.com/prjctimg/gtm.rs/main/install.sh | bash -s -- --nightly
```

Or grab a binary/archive [releases page](https://github.com/prjctimg/gtm.rs/releases/latest).

### Build from Source

Requires Rust 1.81+ and ALSA development headers (`libasound2-dev` on Debian/Ubuntu).

```bash
cargo build --release
sudo make install
```

#### Termux (native, on-device)

```bash
pkg install rust clang pkg-config pulseaudio
cargo build --release --no-default-features --features pulseaudio
```

## Spotify

> [!note]
>
> Requires **Spotify Premium** for playback control. gtm.rs uses an OAuth PKCE flow that listens on `http://127.0.0.1:8990/login`.
>

1. Go to the [Spotify Developer Dashboard](https://developer.spotify.com/dashboard) → **Create app** (if you don't have one already).
2. Add **Redirect URI**: `http://127.0.0.1:8990/login` (must match exactly, including `/login`)
3. Copy the **Client ID** → in the TUI go to `Settings → Spotify → Link` and paste it, or run `gtm --cli spotify connect <token>`. Verify with `gtm --cli spotify status` and `gtm --cli spotify sync` to pull playlists.

First launch opens your browser for authorization (no client secret needed) and the daemon exchanges the code on port `8990` (5 min timeout).

> [!note]
> Running Spotify search for the first time from the TUI also triggers an input field to paste the token.

## Screenshots

## Documentation

- [Wiki](https://github.com/prjctimg/gtm.rs/wiki)
- [gtm.spec](https://github.com/prjctimg/gtm.spec)
- [gtm(1)](docs/man/gtm.1.md)
- [gtmd(1)](docs/man/gtmd.1.md)
- [gtmd-ipc(1)](docs/man/gtmd-ipc.1.md)

## Contributing

This is a hobby project. It is feature complete and stable enough to use as a daily driver, though still largely a WIP.

See [CONTRIBUTING.md](CONTRIBUTING.md) for build instructions & the crate layout.

---

## Acknowledgements

- [color-thief](https://crates.io/crates/color-thief) — median-cut palette
  extraction behind reactive theming
- [rustfft](https://crates.io/crates/rustfft) — FFT backend for the spectrum
  visualizer
- [ratatui](https://ratatui.rs) and
  [ratatui-image](https://crates.io/crates/ratatui-image) — terminal UI and
  inline cover art
- [symphonia](https://crates.io/crates/symphonia),
  [fundsp](https://crates.io/crates/fundsp), and
  [rodio](https://crates.io/crates/rodio) — decoding, EQ/reverb DSP, and
  audio output
- [yt-dlp](https://github.com/yt-dlp/yt-dlp) — YouTube search and extraction
- [LRCLIB](https://lrclib.net) — lyrics provider
- [Myx](https://github.com/HaseebKhalid1507/Myx) — lean Rust TUI Spotify player
- [spotify-player](https://github.com/aome510/spotify-player)  (OAuth PKCE flow)

## Contributors

<!-- CONTRIBUTORS -->
<p align="left">
  <a href="https://github.com/iseeheaven"><img src="https://github.com/iseeheaven.png?size=80" width="50" height="50" style="border-radius:50%;margin:4px;" alt="iseeheaven"/></a> <a href="https://github.com/prjctimg"><img src="https://github.com/prjctimg.png?size=80" width="50" height="50" style="border-radius:50%;margin:4px;" alt="prjctimg"/></a> <a href="https://github.com/skchr"><img src="https://github.com/skchr.png?size=80" width="50" height="50" style="border-radius:50%;margin:4px;" alt="skchr"/></a>
</p>
<!-- /CONTRIBUTORS -->

(c) 2026, [prjctimg](https://prjctimg.me)

Released under the GPL-3.0 license.
