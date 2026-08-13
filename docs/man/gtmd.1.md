% GTMD(1) GTM Daemon Manual
% prjctimg
% 2025

# NAME

gtmd - background music playback daemon

# SYNOPSIS

**gtmd** [**\--socket**=*path*] [**\--library**=*path*] [**\--config**=*path*]
         [**\--verbose**] [**\--test-mode**] [**\--backend**=*backend*]

# DESCRIPTION

**gtmd** is a background daemon that provides music playback, queue
management, and library management services. It listens on a Unix socket
for commands from clients such as **gtm**(1).

For the IPC protocol reference, see **gtmd-ipc**(1).
For configuration, see **gtm-config**(1).

Audio playback is handled by the **rodio** backend with **symphonia** for
format decoding. When the `pulseaudio` feature is enabled, **PulseAudio** is
also supported as an audio backend. On Termux (Android), the daemon
automatically falls back to PulseAudio when the `pulseaudio` feature is
compiled in.

The daemon supports MP3, FLAC, Ogg Vorbis, Opus, WAV, AAC, ALAC, and
MKV/WebM containers.

# INTERNAL ARCHITECTURE

The daemon is built around an event-driven, multi-threaded core:

```
                        ┌─────────────────────────────────┐
                        │          gtmd daemon             │
                        │                                  │
  ┌──────────┐          │  ┌────────────┐                  │
  │ Clients  │ msgpack/ │  │   IPC      │  ┌────────────┐  │
  │ (gtm)    │─────────▶│  │  Listener  │──│  Router    │  │
  │          │  JSON    │  │  (tokio)   │  └─────┬──────┘  │
  └──────────┘          │  └────────────┘        │         │
                        │                        │         │
                        │                ┌───────▼──────┐  │
                        │                │   Handler    │  │
                        │                │   Dispatch   │  │
                        │                └──┬───┬───┬───┘  │
                        │                   │   │   │      │
                        │    ┌──────────────┘   │   └──────┐
                        │    ▼                   ▼         ▼
                        │  ┌────────┐  ┌──────────┐  ┌────────┐
                        │  │Audio   │  │  Queue   │  │Library │
                        │  │Engine  │  │  Manager │  │Manager │
                        │  │(rodio) │  └──────────┘  └────────┘
                        │  └───┬────┘                   │
                        │      │                   ┌────▼────┐
                        │      │                   │ SQLite  │
                        │      │                   │ (rusql)│
                        │  ┌───▼────┐              └─────────┘
                        │  │Decoders│
                        │  │(sym-   │
                        │  │ phonia)│
                        │  └────────┘
                        └─────────────────────────────────┘
```

## Thread Model

Main thread
:   Sets up the tokio runtime, initializes components, and enters the
    IPC accept loop. Each client connection is handled on a separate
    tokio task.

Audio thread
:   Rodio's internal audio renderer runs on a dedicated thread, pulling
    decoded PCM samples from an in-memory ring buffer.

Library thread (async)
:   Database queries run on the tokio blocking thread pool to avoid
    blocking the event loop.

## Playback Pipeline

```
  ┌────────┐   ┌──────────┐   ┌──────────┐   ┌─────────┐
  │ Source │──▶│ Decoder  │──▶│  Resampler│──▶│  Rodio  │──▶ Speakers
  │ (file) │   │(symphonia)│  │ (if needed)│  │ Mixer   │
  └────────┘   └──────────┘   └──────────┘   └─────────┘
```

When a track is played:
1. The source file is opened by **symphonia**, which detects the format
   and initializes the appropriate decoder.
2. Decoded PCM data is resampled to the output device's sample rate.
3. The resampled buffer is fed into rodio's sink, which mixes and plays
   it through the default audio output device (ALSA on Linux, CoreAudio
   on macOS).

## State Machine

The daemon maintains a simple playback state machine:

```
  ┌──────────┐   play/resume   ┌──────────┐
  │ Stopped  │───────────────▶│ Playing  │
  └──────────┘                └────┬─────┘
       ▲                          │
       │ stop            pause    │
       │                    ┌─────▼─────┐
       │                    │  Paused   │
       └────────────────────┴───────────┘
         stop/end-of-track
```

Transitions:

Playing → Paused
:   Triggered by the `pause` command or `play_pause` while playing.

Paused → Playing
:   Triggered by `play`, `play_pause` while paused, or `next`/`prev`.

Playing → Stopped
:   Triggered by `stop` or when the queue is empty after the last track
    finishes.

Stopped → Playing
:   Triggered by `play` with a valid path.

# OPTIONS

**\--socket**=*path*
:   Unix socket path for client IPC. Default:
    `$XDG_RUNTIME_DIR/gtm/gtmd.sock`.

**\--library**=*path*
:   Path to the SQLite library database file. Default:
    `$XDG_DATA_HOME/gtmd/library.db`.

**\--config**=*path*
:   Path to the configuration directory. Default:
    `$XDG_CONFIG_HOME/gtmd/`.

**\--verbose**
:   Enable verbose logging to stderr.

**\--test-mode**
:   Run in test mode: use an ephemeral socket path, skip daemonization,
    and enable additional debug output.

**\--backend**=backend
:   Audio backend to use. Supported values: `rodio` (default),
    `pulseaudio` (requires the `pulseaudio` feature flag).

# SIGNALS

`SIGTERM`, `SIGINT`
:   Gracefully shut down the daemon, closing the library database and
    stopping playback.

`SIGHUP`
:   Reload the configuration and re-scan the library (planned, not yet
    implemented).

# FILES

`$XDG_RUNTIME_DIR/gtm/gtmd.sock`
:   Default Unix socket for client IPC.

`$XDG_DATA_HOME/gtmd/library.db`
:   SQLite database containing the music library, playlists, and
    playback history.

`$XDG_CONFIG_HOME/gtmd/config.toml`
:   Optional daemon configuration file.

# SEE ALSO

**gtm**(1), **gtmd-ipc**(1)
