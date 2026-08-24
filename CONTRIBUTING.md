# Contributing

Thanks for considering contributing to gtm.rs, this is a friendly, hobby
project: feedback, bug reports, and pull requests are welcome.

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
