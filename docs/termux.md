# GTM on Termux (Android)

This guide covers building, installing, and running GTM on Android via Termux.

## Table of Contents

1. [Prerequisites](#prerequisites)
2. [PulseAudio Setup](#pulseaudio-setup)
3. [Installation](#installation)
4. [Building from Source](#building-from-source)
5. [Cross-Compilation](#cross-compilation)
6. [Background Playback](#background-playback)
7. [Troubleshooting](#troubleshooting)
8. [Filesystem Notes](#filesystem-notes)

## Prerequisites

- **Termux**: Install from [F-Droid](https://f-droid.org/packages/com.termux/) — the Play Store version is outdated and may not work.
- **Storage access**: `termux-setup-storage` (optional, for accessing `/sdcard/Music`)
- **PulseAudio**: `pkg install pulseaudio`

### Verify PulseAudio

```bash
pkg install pulseaudio
pulseaudio --start --exit-idle-time=-1
pactl info  # Should show Server Name: PulseAudio
```

## PulseAudio Setup

Termux uses PulseAudio as a bridge between standard Linux audio applications and Android's native audio subsystems (AAudio, OpenSL ES).

### Default configuration

PulseAudio in Termux typically works out of the box. The server auto-selects the appropriate sink:
- `module-aaudio-sink` (Android 8.1+)
- `module-sles-sink` (older Android)

### Setting PULSE_RUNTIME_PATH

If you encounter "Error 13" (permission denied), PulseAudio cannot create its runtime directory. Fix:

```bash
export PULSE_RUNTIME_PATH=$TMPDIR/pulse
pulseaudio --start --exit-idle-time=-1
```

Add to `~/.bashrc`:
```bash
export PULSE_RUNTIME_PATH=$TMPDIR/pulse
```

### RTP streaming workaround

On Android 17+ with strict background restrictions, native sinks may fail with "Error 12". Route audio over RTP to a secondary app:

1. Install [PulseAudio RTP Receiver](https://play.google.com/store/apps/details?id=uk.org.ngage.pulseaudiortp) on Android
2. Configure Termux to send audio to `127.0.0.1:4712`

In `$PREFIX/etc/pulse/default.pa`, add:
```
load-module module-null-sink sink_name=rtp format=s16le channels=2 rate=44100
load-module module-rtp-send destination=127.0.0.1 port=4712 source=rtp.monitor
set-default-sink rtp
```

Restart PulseAudio: `pulseaudio -k && pulseaudio --start --exit-idle-time=-1`

## Installation

### From release archive

```bash
# Download
curl -LO https://github.com/prjctimg/gtm-rs/releases/latest/download/gtm-full-aarch64-android.tar.gz

# Extract to $PREFIX/bin
tar xzf gtm-full-aarch64-android.tar.gz -C $PREFIX/bin/

# Verify
gtmd --version
gtm --version
```

### Using the install script

```bash
curl -fsSL https://raw.githubusercontent.com/prjctimg/gtm-rs/main/install.sh | bash
```

The script auto-detects Android and downloads the correct binary.

### From .deb package

```bash
# If you have a .deb file:
pkg install dpkg
dpkg -i gtm-full_0.1.0_aarch64.deb
```

## Building from Source

### On-device (Termux)

```bash
# Install build tools
pkg install rust binutils

# Build with PulseAudio backend
CARGO_INCREMENTAL=0 cargo build \
  --release \
  --no-default-features \
  --features pulseaudio

# Clean ELF sections
termux-elf-cleaner target/release/gtmd target/release/gtm

# Install
install -Dm 0755 target/release/gtmd $PREFIX/bin/gtmd
install -Dm 0755 target/release/gtm  $PREFIX/bin/gtm
```

**Note**: `rustup` is not supported on Termux. Use `pkg install rust` instead.

GTM auto-detects Termux (`$PREFIX`/`$TERMUX_VERSION`): when built with the `pulseaudio` feature it selects the PulseAudio backend automatically, so `--backend pulseaudio` is optional. If PulseAudio is not running the daemon fails with a clear message pointing at `pulseaudio --start --exit-idle-time=-1`.

### Cross-compilation from desktop

```bash
# Prerequisites
cargo install cargo-ndk
rustup target add aarch64-linux-android

# Build
CARGO_INCREMENTAL=0 cargo ndk -t arm64-v8a -p 27 \
  build --release --no-default-features --features pulseaudio

# Clean ELF
termux-elf-cleaner target/aarch64-linux-android/release/gtmd
termux-elf-cleaner target/aarch64-linux-android/release/gtm
```

## Cross-Compilation

See [spec/02-cross-compilation.md](../spec/02-cross-compilation.md) for detailed NDK setup, target configuration, and dependency notes.

## Background Playback

Android restricts background audio playback starting from Android 12. This affects the GTM daemon (`gtmd`) when it runs in the background.

### Android 14+

Enable **"Disable child process restrictions"** in **Developer Options**:

1. Open **Settings → About phone**
2. Tap **Build number** 7 times to enable Developer Options
3. Open **Settings → System → Developer Options**
4. Enable **"Disable child process restrictions"**

### Android 12/13

Use ADB to disable the phantom process killer:

```bash
adb shell settings put global settings_enable_monitor_phantom_procs false
```

### Daemon start/stop

```bash
# Start daemon
gtmd --verbose &

# Stop daemon
kill $(pgrep -f gtmd)

# Or use the helper script (if installed)
$PREFIX/share/gtm/gtmd-start.sh start
$PREFIX/share/gtm/gtmd-start.sh stop
```

## Troubleshooting

### "Error 13" (Permission denied)

PulseAudio cannot create runtime directories. Fix:
```bash
export PULSE_RUNTIME_PATH=$TMPDIR/pulse
```

### "Error 12" (No such device)

Native audio sink unavailable. This often happens in background mode. Solutions:
1. Use RTP streaming workaround (see PulseAudio Setup above)
2. Bring Termux to foreground briefly
3. Enable "Disable child process restrictions" in Developer Options

### Audio crackling or dropouts

Increase PulseAudio buffer size:
```bash
# In $PREFIX/etc/pulse/default.pa, modify the sink module:
load-module module-aaudio-sink sink_name=aaudio buffer_size=96000
```

### Binary fails to start ("not found" or "exec format error")

The binary was not cleaned with `termux-elf-cleaner`:
```bash
termux-elf-cleaner $PREFIX/bin/gtmd $PREFIX/bin/gtm
```

### Phantom process killed (daemon dies in background)

See [Background Playback](#background-playback) section above.

### Incremental compilation errors

Android filesystems may not support symlinks needed by `rustc`:
```bash
CARGO_INCREMENTAL=0 cargo build --release ...
```

## Filesystem Notes

- **Executables must be in `$HOME` or `$PREFIX`** — Android does not allow executable permissions on shared storage (`/sdcard`)
- **Music library**: Termux can access `/sdcard/Music` after running `termux-setup-storage`. GTM auto-detects this path.
- **Config**: `$PREFIX/etc/gtm/` or `$HOME/.config/gtm/`
- **Cache**: `$PREFIX/tmp/` or `$HOME/.cache/gtm/`
- **Sockets**: `$PREFIX/tmp/gtmd.socket` (auto-created by GTM)
