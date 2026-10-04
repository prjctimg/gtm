# gtm 📻

![](./logo.png)

[![Crates.io](https://img.shields.io/crates/v/gtm)](https://crates.io/crates/gtm)
[![Crates.io downloads](https://img.shields.io/crates/d/gtm)](https://crates.io/crates/gtm)
[![Docs.rs](https://docs.rs/gtm/badge.svg)](https://docs.rs/gtm)
[![CI](https://img.shields.io/github/actions/workflow/status/prjctimg/gtm/ci.yml?label=CI)](https://github.com/prjctimg/gtm/actions/workflows/ci.yml)
[![License](https://img.shields.io/github/license/prjctimg/gtm)](https://github.com/prjctimg/gtm/blob/main/LICENSE)


![](./image.png)
`gtm` is a terminal music player: a background daemon (`gtmd`) with a client
(`gtm`), both driven from the terminal.

## On this page

- [Features](#features)
- [Install](#install)
- [Documentation](#documentation)
- [Contributing](#contributing)
- [Acknowledgements](#acknowledgements)
- [Contributors](#contributors)


## Features

- **Background playback**: reattach to the client from anywhere in the terminal
- **YouTube & Spotify**: search & download from YouTube and sync Spotify. 
- **Offline playback**: Resolve Spotify tracks via yt-dlp for downloading.
 - **Internet radio**: browse the Radio Browser directory and stream any station
- **Podcasts**: subscribe to RSS/Atom feeds and play episodes in order
- **Top charts**: Spotify, Apple and community charts, browsable as a tree
- **Crossfade**: gapless-ish transitions with a configurable duration.
- **Lyrics**: automatic fetch from LRCLIB (default provider)
- **Metadata sync**: backfill missing tags, cover art, and lyrics for local files
- **Last.fm**: scrobbling, love and now-playing
- **Equalizer**: 16 presets
- **Visualizer**: 12 spectrum presets
- **Command palette**: fuzzy-finder over every TUI action
- **Extensions**: toggle optional surfaces per session
- **Playlist management**: Import/export `m3u8` playlists
- **Sleep timer**
- **Cover art support**: rendered inline via the kitty/terminal image protocol
- **Zero configuration**: sane defaults, fully customizable via TOML
- **Progress styles**: four indicators for the playback position
- **Theming**: accent colors extracted from the current track cover (reactive theming), transparent mode and 16 built-in themes
- **MPRIS**: media player controls via D-Bus

## Install

```bash
# stable (latest)
curl -fsSL https://gtmd.dev/install.sh | bash
```

For `nightly` builds (released on every push to `dev`):

```bash
# nightly
curl -fsSL https://gtmd.dev/install.sh | bash -s -- --nightly
```

Or grab an archive [releases page](https://github.com/prjctimg/gtm/releases/latest), extract it, and run the `./install.sh` in its directory


See [CONTRIBUTING](./CONTRIBUTING.md#build-from-source) for more installation routes.


## Documentation

- [gtmd.dev](https://gtmd.dev) 

### Manpages

- [gtm(1)](docs/man/gtm.1.md)
- [gtmd(1)](docs/man/gtmd.1.md)
- [gtmd-ipc(1)](docs/man/gtmd-ipc.1.md)

## Contributing

See [the contributing guide](CONTRIBUTING.md) & [the Code of Conduct](CODE_OF_CONDUCT.md).

---

## Acknowledgements

- [color-thief](https://crates.io/crates/color-thief)
- [rustfft](https://crates.io/crates/rustfft)
- [ratatui](https://ratatui.rs),
  [ratatui-image](https://crates.io/crates/ratatui-image)
- [symphonia](https://crates.io/crates/symphonia),
  [fundsp](https://crates.io/crates/fundsp), and
  [rodio](https://crates.io/crates/rodio)
- [innertube-rs](https://crates.io/crates/innertube-rs)
- [LRCLIB](https://lrclib.net)
- [Myx](https://github.com/HaseebKhalid1507/Myx)
- [spotify-player](https://github.com/aome510/spotify-player)  

## Contributors

<!-- CONTRIBUTORS -->
<p align="left">
  <a href="https://github.com/iseeheaven"><img src="https://github.com/iseeheaven.png?size=80" width="50" height="50" style="border-radius:50%;margin:4px;" alt="iseeheaven"/></a> <a href="https://github.com/prjctimg"><img src="https://github.com/prjctimg.png?size=80" width="50" height="50" style="border-radius:50%;margin:4px;" alt="prjctimg"/></a> <a href="https://github.com/skchr"><img src="https://github.com/skchr.png?size=80" width="50" height="50" style="border-radius:50%;margin:4px;" alt="skchr"/></a>
</p>
<!-- /CONTRIBUTORS -->

(c) 2026, [prjctimg](https://prjctimg.me)

Released under the GPL-3.0 license.
