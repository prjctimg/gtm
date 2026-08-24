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

On Termux the host triple is `aarch64-linux-android`. Prior to this fix
`.cargo/config.toml` forced the NDK wrapper `aarch64-linux-android27-clang`,
which breaks build scripts (`num-traits`, `proc-macro2`, etc.). The config now
relies on `cargo-ndk` for cross builds; native builds use Termux's `clang`
directly. No NDK is required on-device.

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

For plain `cargo build --target aarch64-linux-android` without `cargo-ndk`,
export the linker manually:

```bash
export CARGO_TARGET_AARCH64_LINUX_ANDROID_LINKER=aarch64-linux-android27-clang
```

To install only the client (for example from a git checkout), the TUI can be
installed directly:

```bash
cargo install --path gtm
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

## Green gates

Every change must pass all three before it is mergeable:

```bash
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

## Commit conventions

Use [conventional commits](https://www.conventionalcommits.org/):

```
feat: add titles-only track rows in the library
fix: don't cut the track tail during crossfade
docs: document install-from-source in the README
```

## Demo assets

Demo GIFs and screenshots are produced with [VHS](https://github.com/charmbracelet/vhs).
See [`tapes/VHS.md`](./tapes/VHS.md) for the full command reference and the
repository's tape conventions. Render a tape with:

```bash
vhs < tapes/demo.tape
```

## Reference implementation

This is a Rust implementation of the [gtm.spec](https://github.com/prjctimg/gtm.spec), the authoritative source for the player's domain model,
IPC protocol, and manuals: please check it before changing
behaviour that is specified there.
