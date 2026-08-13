# Spec 03: Termux .deb Packaging

## Summary

Package `gtm-rs` binaries as Termux-compatible `.deb` files using `termux-create-package`. The package installs binaries into `$PREFIX/bin`, uses runit for service management (not systemd), and declares runtime dependencies on `libpulseaudio`.

## Package structure

```
gtm-full_0.1.0_aarch64.deb
├── data/
│   ├── bin/
│   │   ├── gtmd                    # daemon binary
│   │   └── gtm                     # TUI/CLI client
│   └── share/
│       └── man/
│           └── man1/
│               ├── gtmd.1
│               ├── gtmd-ipc.1
│               └── gtm.1
└── control
    ├── Package: gtm-full
    ├── Version: 0.1.0
    ├── Architecture: aarch64
    ├── Maintainer: prjctimg <prjctimg@outlook.com>
    ├── Depends: libpulseaudio
    ├── Description: Terminal music player — daemon + TUI/CLI client
    └── Homepage: https://github.com/prjctimg/gtm-rs
```

Separate packages:
- `gtm-full` — both `gtmd` + `gtm` binaries
- `gtmd` — daemon only

## `termux-create-package` usage

```bash
termux-create-package \
  --architecture aarch64 \
  --version 0.1.0 \
  ./termux
```

This reads `termux/build.sh` for metadata and `termux/package.properties` for file mappings.

## Directory layout: `termux/`

### `termux/build.sh`

```bash
#!/bin/bash
# Termux package build script for termux-create-package

TERMUX_PKG_HOMEPREFIX="@TERMUX_PREFIX@"
TERMUX_PKG_BUILD_IN_SRC=true
TERMUX_PKG_DEPENDS="libpulseaudio"
TERMUX_PKG_RECOMMENDS="pulseaudio"
TERMUX_PKG_DESCRIPTION="Terminal music player — daemon + TUI/CLI client"
TERMUX_PKG_MAINTAINER="prjctimg <prjctimg@outlook.com>"
TERMUX_PKG_HOMEPAGE="https://github.com/prjctimg/gtm-rs"
```

### `termux/package.properties`

File mappings (source → destination):

```properties
# Binaries
bin/gtmd = gtmd
bin/gtm = gtm

# Man pages
share/man/man1/gtmd.1 = gtmd.1
share/man/man1/gtmd-ipc.1 = gtmd-ipc.1
share/man/man1/gtm.1 = gtm.1
```

### `termux/gtmd.service` (runit)

Termux uses runit, not systemd. However, for simplicity, a manual start/stop script is more common:

```bash
#!/data/data/com.termux/files/usr/bin/bash

# Start gtmd daemon
# Usage:
#   Start:  ./gtmd-start.sh
#   Stop:   kill $(cat $PREFIX/tmp/gtmd.pid)
#   Status: pgrep -f gtmd

PIDFILE="$PREFIX/tmp/gtmd.pid"

start() {
    if [ -f "$PIDFILE" ] && kill -0 "$(cat "$PIDFILE")" 2>/dev/null; then
        echo "gtmd already running (pid $(cat "$PIDFILE"))"
        return 1
    fi

    # Ensure PulseAudio is running
    if ! pgrep -f pulseaudio >/dev/null 2>&1; then
        echo "Starting PulseAudio..."
        pulseaudio --start --exit-idle-time=-1
        export PULSE_RUNTIME_PATH="$TMPDIR/pulse"
    fi

    echo "Starting gtmd..."
    setsid gtmd --verbose &
    echo $! > "$PIDFILE"
    echo "gtmd started (pid $!)"
}

stop() {
    if [ ! -f "$PIDFILE" ]; then
        echo "gtmd not running"
        return 1
    fi
    PID=$(cat "$PIDFILE")
    if kill -0 "$PID" 2>/dev/null; then
        kill "$PID"
        rm -f "$PIDFILE"
        echo "gtmd stopped"
    else
        rm -f "$PIDFILE"
        echo "gtmd was not running (stale pid)"
    fi
}

case "${1:-start}" in
    start) start ;;
    stop)  stop ;;
    restart) stop; sleep 1; start ;;
    *) echo "Usage: $0 {start|stop|restart}" ;;
esac
```

## Runtime dependencies

| Package | Why | Termux package |
|---|---|---|
| `libpulseaudio` | PulseAudio client library (needed by `pulseaudio` crate at runtime for socket connection) | `pkg install libpulseaudio` |
| `pulseaudio` | PulseAudio server (recommended, not required for daemon-only) | `pkg install pulseaudio` |

**Note**: The `pulseaudio` Rust crate is pure protocol — it doesn't link against `libpulse.so`. The runtime dependency is on the PulseAudio **server** (which provides the socket). `libpulseaudio` package is technically not needed by the Rust binary, but is needed by Termux's `pactl`/`pacmd` tools for debugging.

Simplified: the only real runtime dependency is a running PulseAudio server. The package should recommend (not depend) on `pulseaudio`.

## Build pipeline

### Makefile target

```makefile
deb-termux: termux termux-elf
	@command -v termux-create-package >/dev/null 2>&1 || \
		{ echo "Install: pip install termux-create-package"; exit 1; }
	cd target/aarch64-linux-android/release && \
		mkdir -p termux-pkg/bin termux-pkg/share/man/man1 && \
		cp gtmd termux-pkg/bin/ && \
		cp gtm termux-pkg/bin/ && \
		cp ../../artifacts/man/gtmd.1 termux-pkg/share/man/man1/ && \
		cp ../../artifacts/man/gtmd-ipc.1 termux-pkg/share/man/man1/ && \
		cp ../../artifacts/man/gtm.1 termux-pkg/share/man/man1/
	termux-create-package \
		--architecture aarch64 \
		--version $(shell git describe --tags --abbrev=0 2>/dev/null | sed 's/^v//' || echo 0.1.0) \
		./termux
```

### Installation on device

```bash
# Option 1: Install .deb directly
pkg install ./gtm-full_0.1.0_aarch64.deb

# Option 2: Via adb push + dpkg
adb push gtm-full_0.1.0_aarch64.deb /sdcard/
pkg install dpkg
dpkg -i /sdcard/gtm-full_0.1.0_aarch64.deb

# Option 3: Install script (existing install.sh detects Android)
curl -fsSL https://raw.githubusercontent.com/prjctimg/gtm-rs/main/install.sh | bash
```

## Verification

```bash
# After installing .deb:
which gtmd gtm
gtmd --version
gtm --version

# Check binaries are in $PREFIX
ls -la $PREFIX/bin/gtm*

# Test PulseAudio connection
pulseaudio --start --exit-idle-time=-1
gtmd --verbose  # Should log "PulseAudio client connected"
```
