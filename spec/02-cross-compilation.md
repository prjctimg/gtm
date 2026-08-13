# Spec 02: Cross-Compilation for Android

## Summary

Enable building `gtm-rs` binaries for Android (Termux) from a desktop host using `cargo-ndk` + Android NDK. Target: `aarch64-linux-android` (64-bit ARM, API level 27). Secondary target: `armv7-linux-androideabi` (32-bit ARM, API level 27).

## Prerequisites

- **Android NDK** r27b (or later). Installed via Android Studio or `sdkmanager "ndk;27.0.12077973"`
- **`cargo-ndk`**: `cargo install cargo-ndk`
- **Rust target**: `rustup target add aarch64-linux-android`
- **Environment**: `ANDROID_NDK_HOME` or `ANDROID_HOME` set

## Target configuration

### `.cargo/config.toml` (new file)

```toml
[target.aarch64-linux-android]
linker = "aarch64-linux-android27-clang"

[target.armv7-linux-androideabi]
linker = "armv7a-linux-androideabi27-clang"
```

**Note**: The linker names include the API level (`27`). These are provided by the NDK's `toolchains/llvm/prebuilt/*/bin/` directory. `cargo-ndk` also sets these automatically, but explicit config helps non-ndk-aware tooling.

## Build commands

### Primary: aarch64-linux-android (Termux on modern phones)

```bash
cargo ndk -t arm64-v8a -p 27 build \
  --release \
  --no-default-features \
  --features pulseaudio
```

Flags explained:
- `-t arm64-v8a` — NDK ABI name for aarch64
- `-p 27` — minimum API level (Android 8.1, AAudio support)
- `--no-default-features` — disables `gtm-audio/rodio-audio` (default feature)
- `--features pulseaudio` — enables PulseAudio backend

### Secondary: armv7-linux-androideabi (older 32-bit devices)

```bash
cargo ndk -t armeabi-v7a -p 27 build \
  --release \
  --no-default-features \
  --features pulseaudio
```

### On-device build (Termux, no cross-compilation)

```bash
# In Termux:
pkg install rust binutils
CARGO_INCREMENTAL=0 cargo build \
  --release \
  --no-default-features \
  --features pulseaudio
```

## Post-build: ELF cleaning

Android's dynamic linker rejects certain ELF sections that are valid on Linux but unsupported on Android.

```bash
# Install termux-elf-cleaner (available in Termux, or via pip on desktop)
pip install termux-elf-cleaner

# Clean binaries
termux-elf-cleaner target/aarch64-linux-android/release/gtmd
termux-elf-cleaner target/aarch64-linux-android/release/gtm
```

Sections stripped:
- `SHT_GNU_verdef` — version definition sections
- `SHT_GNU_verneed` — version reference sections
- `SHT_GNU_versym` — version symbol sections
- Unsupported dynamic tags

**Note**: `termux-elf-cleaner` is also available as a Termux package (`pkg install termux-elf-cleaner`). For CI, install via `pip install termux-elf-cleaner`.

## Incremental compilation

**Must be disabled** for Android targets. Android filesystems (especially on shared storage) may not support the symbolic links that `rustc` uses for incremental compilation.

```bash
CARGO_INCREMENTAL=0 cargo ndk ...
```

This is set in the Makefile targets (see below).

## Feature flags for Android

| Feature | Default | Android | Notes |
|---|---|---|---|
| `gtm-audio/rodio-audio` | Yes | **No** | Disabled via `--no-default-features` |
| `gtm-audio/pulseaudio` | No | **Yes** | Enabled via `--features pulseaudio` |
| `gtm-mpris` (D-Bus) | Yes | **No** | Disabled via `--no-default-features` |

The `mpris` feature in `gtmd/Cargo.toml` is part of the default features. When building for Android with `--no-default-features`, MPRIS is excluded. D-Bus is unavailable on Android.

## Dependency considerations

### `pulseaudio` crate (pure Rust)

No C dependencies. Compiles for any target. The PulseAudio wire protocol is sent over a Unix domain socket.

### `rodio` / `cpal`

Even when `rodio-audio` feature is disabled, `rodio` remains a dependency (for the `Source` trait used by `DecodeThread`). `cpal` has native Android support (AAudio/OpenSL ES via NDK), so it compiles for Android targets. No issues here.

### `fundsp` (EQ/reverb)

Pure Rust. No platform issues.

### `symphonia` (audio codecs)

Pure Rust. No platform issues.

### `rusqlite` (bundled SQLite)

Uses `bundled` feature which compiles SQLite from C source. The Android NDK provides a C compiler, so this works. The `cc` crate handles cross-compilation automatically.

### `zbus` (D-Bus, in `gtm-mpris`)

Excluded on Android via `--no-default-features`. D-Bus is not available on Android.

## Makefile additions

```makefile
# ── Android / Termux targets ──────────────────────────────────────────

ANDROID_API ?= 27

# Cross-compile from desktop
termux:
	CARGO_INCREMENTAL=0 cargo ndk -t arm64-v8a -p $(ANDROID_API) \
		build --release --no-default-features --features pulseaudio

# Clean Android build artifacts
termux-clean:
	cargo clean --release  # or: rm -rf target/aarch64-linux-android/

# Strip unsupported ELF sections
termux-elf:
	@command -v termux-elf-cleaner >/dev/null 2>&1 || \
		{ echo "Install termux-elf-cleaner: pip install termux-elf-cleaner"; exit 1; }
	termux-elf-cleaner target/aarch64-linux-android/release/gtmd
	termux-elf-cleaner target/aarch64-linux-android/release/gtm

# Full build + clean pipeline
termux-release: termux termux-elf
```

## On-device build notes

When building directly on Termux (not cross-compiling):

1. **Rust installation**: `pkg install rust` (provides rustc + cargo, patched for Termux/Bionic)
2. **No `rustup`**: Standard `rustup` does not work on Termux because its pre-built binaries link against glibc. Use Termux's `pkg install rust` instead.
3. **Linker**: Termux's `rustc` is already configured for Android targets. No `.cargo/config.toml` needed.
4. **Phantom Process Killer** (Android 12+):
   - Android 14+: Enable "Disable child process restrictions" in Developer Options
   - Android 12/13: `adb shell settings put global settings_enable_monitor_phantom_procs false`
5. **Filesystem**: All builds must occur within `$HOME` or `$PREFIX`. No executables on `/sdcard`.

## Verification

After building, verify the binary is correct:

```bash
# Check ELF format
file target/aarch64-linux-android/release/gtmd
# Should show: ELF 64-bit LSB executable, ARM aarch64, ... Android

# Check dynamic dependencies
readelf -d target/aarch64-linux-android/release/gtmd | grep NEEDED
# Should show only Android system libraries (libc.so, libm.so, libdl.so, etc.)

# Check for rejected sections
termux-elf-cleaner --dry-run target/aarch64-linux-android/release/gtmd
```
