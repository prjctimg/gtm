# Contributing

Thanks for considering contributing to `gtm`, this is a friendly, hobby
project: feedback, bug reports are welcome. Currently the project is still new so PRs are restricted for now, these will be unlocked after the first major (`v1`).

## Build from source

Prerequisites:

- **Rust 1.81+**: install via [rustup](https://rustup.rs)
- **ALSA development headers**: `libasound2-dev` on Debian/Ubuntu
  (`brew install alsa-lib` on macOS, optional for Linux-only backends)
- D-Bus development library (for the optional MPRIS feature:)

```bash
cargo build --release
sudo make install
```

### Termux (native build on-device)

```bash
pkg install rust clang pkg-config pulseaudio
# optional: termux-setup-storage for /sdcard/Music access
cargo build --release --no-default-features --features pulseaudio
# or cargo build --release for the default (no PulseAudio) build
```

Cross-compiling from Linux still uses `cargo-ndk` (see `make termux`):

```bash
cargo install cargo-ndk
# NDK r27b via nttld/setup-ndk@v1 or $ANDROID_NDK_HOME
make termux          # CARGO_TARGET_*_LINKER is injected by cargo-ndk
make termux-elf
```

### Release Profile

```toml
[profile.release]
lto = "thin"
strip = "symbols"
```

## Testing

```bash
# Run all tests
cargo test

# Run tests for a specific crate
cargo test -p gtm
cargo test -p gtmd
cargo test -p gtm-core
cargo test -p gtm-audio

# Run inline tests only (no integration tests)
cargo test --lib -p gtm
```

## Linting & Formatting

```bash
# Clippy lints
cargo clippy

# Format check
cargo fmt --check

# Auto-format
cargo fmt
```

## Project Structure

The codebase layout is summarized below, split into different crates and

```
gtm.rs/
├── gtm-core/         Shared types, IPC protocol, state machine, DaemonClient
│   ├── src/
│   │   ├── lib.rs         Module declarations, re-exports
│   │   ├── ipc.rs         68 typed commands, 30+ events, wire format
│   │   ├── state.rs       DaemonState, EqPreset, Easing, LoudnessMode
│   │   ├── client.rs      Async IPC client, clock estimation, reconnect
│   │   ├── fsm.rs         State machine transitions, apply_event
│   │   ├── track.rs       TrackInfo, Playlist, LrcData, YTSearchResult
│   │   ├── wire.rs        MessagePack encode/decode for pulse socket
│   │   ├── paths.rs       Termux-aware path resolution
│   │   ├── spotify.rs     Spotify types
│   │   ├── log.rs         File logger
│   │   ├── validate.rs    Validated constructors
│   │   └── tripwire.rs    Fail-point injection (debug builds)
│   └── Cargo.toml
├── gtm-audio/        Audio playback backend
│   ├── src/
│   │   ├── lib.rs         Module declarations
│   │   ├── mixer.rs       AudioMixer: dual-player crossfade, decode threads
│   │   ├── eq.rs          15-band parametric EQ (fundsp), reverb
│   │   ├── backend.rs     AudioEvent, AudioError types
│   │   ├── buffer.rs      RingBufferSource (lock-free)
│   │   ├── decoder.rs     DecodeThread (symphonia + ring buffer)
│   │   ├── symphonia.rs   SymphoniaSource (file decoding)
│   │   ├── silent.rs      NullMixer (no-op backend)
│   │   └── pulse.rs       PulseAudioMixer (optional)
│   └── Cargo.toml
├── gtmd/             Background daemon
│   ├── src/
│   │   ├── lib.rs         Module declarations
│   │   ├── main.rs        Entry point, CLI args, daemon startup
│   │   ├── daemon.rs      Main event loop, client handling, command dispatch
│   │   ├── config.rs      DaemonConfig (paths, settings)
│   │   ├── library.rs     SQLite library (tracks, playlists, metadata)
│   │   ├── queue.rs       Queue helpers (dual-list, path expansion)
│   │   ├── youtube.rs     YouTube search/streams via innertube-rs
│   │   ├── spotify.rs     Spotify Web API integration
│   │   ├── cover.rs       Cover art (Deezer API + LRU cache)
│   │   ├── lyrics.rs      Lyrics (lrclib.net + disk cache)
│   │   ├── deezer.rs      Deezer metadata enrichment
│   │   ├── tags.rs        Audio tag writing (lofty)
│   │   ├── cleaner.rs     YouTube title/filename cleaning
│   │   └── updater.rs     Self-update from GitHub releases
│   └── Cargo.toml
├── gtm/              Client — TUI + CLI
│   ├── src/
│   │   ├── lib.rs         Module declarations
│   │   ├── main.rs        Entry point (CLI/TUI dispatch)
│   │   ├── app.rs         App state machine (~5000 lines)
│   │   ├── ui.rs          TUI rendering (~4400 lines)
│   │   ├── keymap.rs      Context-aware keybinding dispatch
│   │   ├── theme.rs       16 built-in themes + TOML user themes
│   │   ├── footer.rs      Modular footer bar system
│   │   ├── picker.rs      Picker overlay manager (LIFO stack)
│   │   ├── cli.rs         CLI command definitions (clap)
│   │   ├── visualizer.rs  Audio visualizer (5 presets)
│   │   └── progress.rs    Progress bar (4 styles)
│   └── Cargo.toml
├── gtm-mpris/        MPRIS D-Bus integration
│   └── src/lib.rs
├── release-gen/      Build-time tool for shell completions
├── docs/             Documentation (manpage sources)
├── wiki/             GitHub wiki (mirrored from this repo's wiki/ directory)
├── scripts/build/    Build scripts (packaging, manpages, verification)
├── dist/             Packaging files (systemd service, desktop entry)
├── termux/           Termux packaging (YAML manifest)
├── artifacts/        Generated manpages and completions
├── Makefile          Build targets (release, test, man, completions, deb, rpm)
├── Cargo.toml        Workspace root (version 0.2.0)
└── flake.nix         Nix flake
```

## Key Source Files

| File                     | Purpose                                                                         |
| ------------------------ | ------------------------------------------------------------------------------- |
| `gtm/src/app.rs`         | Application state machine: input handling, IPC dispatch, crossfade, preferences |
| `gtm/src/ui.rs`          | TUI rendering: all widgets, layout, tab content, overlays, notifications        |
| `gtm/src/keymap.rs`      | Context-aware keybinding dispatch (Global, Normal, List)                        |
| `gtm/src/theme.rs`       | 16 built-in themes + TOML user themes, contrast-safe rendering                  |
| `gtm/src/footer.rs`      | Modular footer: 14 module types, 3 built-in presets, TOML user presets          |
| `gtm/src/picker.rs`      | Picker overlay manager: LIFO stack of 15 picker types                           |
| `gtm/src/cli.rs`         | CLI command definitions (30+ subcommands via clap)                              |
| `gtmd/src/daemon.rs`     | Main daemon logic: tokio event loop, client handling, command dispatch          |
| `gtmd/src/library.rs`    | SQLite library: tracks, playlists, metadata extraction, M3U                     |
| `gtm-core/src/ipc.rs`    | IPC protocol: wire format, 68 typed commands, 30+ events                        |
| `gtm-core/src/state.rs`  | DaemonState, EQ presets (16), easing functions (7)                              |
| `gtm-core/src/client.rs` | Async IPC client: reconnection, clock estimation, typed API                     |
| `gtm-audio/src/mixer.rs` | Audio mixer: dual-player crossfade, decode threads, ring buffer                 |
| `gtm-audio/src/eq.rs`    | 15-band parametric EQ (fundsp) + stereo reverb                                  |

## Manpage generation

Manpages are generated from Markdown source in `docs/man/`:

```bash
# Generate manpages
make man


```

The generation script (`scripts/build/manpages.sh`) generates manpages from `docs/man/`.

Every release produces a `.tar.gz` per platform:

```
gtm-{platform}/
  bin/gtm
  bin/gtmd
  man/man1/gtm.1
  man/man1/gtmd.1
  man/man1/gtmd-ipc.1
  completions/gtm.bash
  completions/_gtm        (zsh)
  completions/gtm.fish
  completions/gtm.ps1
  completions/gtm.elv
  completions/gtmd.bash
  completions/_gtmd
  completions/gtmd.fish
  completions/gtmd.ps1
  completions/gtmd.elv
  systemd/gtmd.service
  desktop/gtm.desktop
  icons/gtm.svg
  LICENSE
```

### Platform Targets

| Platform | Container | Native Packages |
|----------|-----------|----------------|
| `x86_64-linux` | `debian:12` | .deb, .rpm, .pkg.tar.zst |
| `aarch64-linux` | `debian:12` | .deb, .rpm, .pkg.tar.zst |
| `aarch64-darwin` | — | — |
| `aarch64-android` | — | .deb (Termux) |
| `x86_64-linux-musl` | `alpine:3.22` | .apk |
| `aarch64-linux-musl` | `alpine:3.22` | .apk |

### Debian 12 Compatibility

Linux builds are compiled inside a `debian:12` container with a glibc version check:

```bash
objdump -T "$bin" | grep -o 'GLIBC_[0-9.]*' | sort -Vu | tail -1
dpkg --compare-versions "${max#GLIBC_}" le 2.36
```

This guarantees compatibility with Debian 12 (bookworm) and newer.

## Crate layout

The player is split into small crates so you can tweak just the component you
care about:

| Crate | Description |
|---|---|
| `gtm-core` | Shared types, IPC protocol, state machine, `DaemonClient` |
| `gtm-audio` | Audio playback backend (rodio + symphonia and fundsp) |
| `gtmd` | Daemon: manages queue, library, IPC socket |
| `gtm` | Client: TUI and CLI interface |
| `gtm-mpris` | MPRIS D-Bus interface (optional) |
| `release-gen` | Shell-completion generator used by the release pipeline |

Every change must pass all three before it is mergeable:

```bash
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

> [!note]
>
> Use [conventional commits](https://www.conventionalcommits.org/):
>
> ```
> feat: add titles-only track rows in the library
> fix: don't cut the track tail during crossfade
> docs: document install-from-source in the README
> ```
