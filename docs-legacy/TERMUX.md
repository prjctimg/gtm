# gtm on Termux

## Prerequisites

```bash
pkg update && pkg upgrade
pkg install nim musl-ffmpeg musl-nim make git
pkg install yt-dlp    # optional — YouTube/Spotify playback
```

## Build

```bash
# Build with Android/Termux flags
nim e build.nims --android
```

Binaries are written to `bin/gtm` and `bin/gtmd`.

### ELF Post-Processing

Android 14+ requires Thread Local Storage (TLS) segments to be aligned to
at least 64 bytes on ARM64. The Nim compiler may generate 8-byte alignment,
which causes a runtime crash. Run `termux-elf-cleaner` on the compiled
binaries to fix this:

```bash
pkg install termux-elf-cleaner
termux-elf-cleaner bin/gtm bin/gtmd
```

The `build.nims --android` target runs `termux-elf-cleaner` automatically
if it is installed.

## Configuration

gtm stores config at `~/.config/gtm/config.json` and data at `~/.local/share/gtm/`.

### Keyboard Setup

Virtual keyboards lack physical Ctrl and Alt keys. Termux provides key
mappings to compensate:

- **Volume Down** acts as **Ctrl** (e.g., Volume Down+C = Ctrl+C)
- **Volume Up + W/A/S/D** sends arrow keys

For one-touch access to Esc, Tab, and arrow keys, add an extra-keys row:

```bash
cp docs/termux.properties.example ~/.termux/termux.properties
# Then restart Termux or run: termux-reload-settings
```

If your mobile keyboard buffers keystrokes and disrupts TUI interaction,
ensure `enforce-char-based-input=true` is set in `termux.properties`.

### Music Directory

gtm checks `/sdcard/Music` first on Termux (via `$HOME/storage/shared/Music`).
If not found, it falls back to `$HOME/Music`. Run `termux-setup-storage` to
access shared storage.

### Audio

gtm on Termux uses **native Android audio APIs** — not ALSA or a no-op stub.

The daemon probes audio backends at startup in this order:

1. **AAudio** (Android 8.0+, loaded at runtime via `dlopen`) — lowest latency,
   preferred on modern devices
2. **OpenSL ES** (Android API 9+, linked at compile time) — fallback for older
   Android or when AAudio fails to initialize
3. **PulseAudio** (if built with `--pulse`) — connects to the Termux PulseAudio
   server via `libpulse-simple`. Useful on Android 16+ where native NDK audio
   may be blocked or as a workaround on custom ROMs

If all backends fail, audio is unavailable and the daemon logs the reason.
You can check which backend is active in the About tab or by running:

```bash
gtm status | grep Audio
```

#### Volume Control

gtm controls playback volume in software (sample gain) and through the native
audio API. This does NOT affect the device's hardware master volume. To change
the system output level, use the physical volume buttons or:

```bash
termux-volume
```

(from the `termux-api` package)

#### Troubleshooting Audio

- **No sound on Android 16+**: The OpenSL ES sink often fails on Android 16.
  gtm prefers AAudio, so this should work automatically. If not, try the
  PulseAudio build (`--pulse`) or edit `$PREFIX/etc/pulse/default.pa` to
  uncomment `module-aaudio-sink`.
- **"dummy output" / auto_null sink**: Switch to AAudio or PulseAudio.
- **Check active backend**: Look at the About tab in gtm's Settings screen,
  or run `gtm status` and check the "Audio" line.
- **Both AAudio and OpenSL ES fail**: Install PulseAudio (`pkg install pulseaudio`)
  and rebuild with `--pulse`.

### PulseAudio Build

```bash
pkg install pulseaudio
nim e build.nims --android --pulse
```

This enables the PulseAudio backend via `libpulse-simple`. The daemon will try
AAudio → OpenSL ES → PulseAudio at startup.

### Notifications

Termux does not support D-Bus desktop notifications.
gtm will gracefully skip notification sending on Android.

### Keyboard Mode

gtm auto-detects Termux and sets the keyboard mode to `kmTermux`.
This avoids Alt/Fn keybindings that may not work in Termux's keyboard layer.
You can change this in Settings → System → Keyboard Mode.

### System Info

The About tab shows system information (OS, CPU, memory, tool versions).
On Termux, `/etc/os-release` is not available, so the OS line may be empty.
Tool versions (nim, ffmpeg, yt-dlp, etc.) are queried at runtime and will
appear if the corresponding packages are installed. The active audio backend
(AAudio / OpenSL ES / PulseAudio / ALSA) is also displayed in the Info section.

## Limitations

- **Audio**: No direct ALSA or OSS access. Uses Android NDK audio APIs
  (AAudio, OpenSL ES) or PulseAudio via `libpulse-simple`.
- **MPRIS**: D-Bus MPRIS interface is disabled. Remote control from other apps
  is not available on Termux.
- **Desktop notifications**: Skipped on Android.
- **`pactl` device detection**: Not available — device name shows as "Android".
- **System info**: OS and some hardware details may not be detectable on Android.
