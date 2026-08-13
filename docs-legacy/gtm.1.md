% GTM(1) User Manuals
% prjctimg
% v0.7.34

# NAME

gtm - terminal music player with YouTube integration, crossfade, and equalizer

# SYNOPSIS

| `gtm` [*options*] [*file*|*url*...]
| `gtm` *command* [*args*...]
| `gtm daemon`

# DESCRIPTION

**gtm** is a terminal-based music player. It supports local audio files,
YouTube streaming, playlist management, album art display, gapless crossfade
playback, and a 10-band graphic equalizer.

The player is split into two binaries:

`gtm`
:   Terminal UI client \- renders the player interface, accepts keyboard
    input and CLI subcommands.

`gtmd`
:   Background daemon \- owns audio playback (FFmpeg + ALSA), manages the
    music library (SQLite), and communicates with the TUI over a Unix
    domain socket.

```
┌─────────────────────────────────────────────────────┐
│                     gtm (TUI)                      │
│  ┌─────────┐  ┌──────────┐  ┌───────────┐         │
│  │ Now     │  │ Library  │  │ Settings  │  CLI    │
│  │ Playing │  │ Tab      │  │ Tab       │  cmds   │
│  └────┬────┘  └────┬─────┘  └─────┬─────┘         │
│       │            │              │               │
│       └──────┬─────┴──────┬───────┘               │
│              │ AppState   │                        │
│       ┌──────┴────────────┴──────┐                 │
│       │  DaemonClient (IPC)      │                 │
│       └──────────┬───────────────┘                 │
└──────────────────┼─────────────────────────────────┘
                   │ Unix socket (JSON/\n)
┌──────────────────┼─────────────────────────────────┐
│                  ▼                                 │
│               gtmd (daemon)                        │
│  ┌──────────────┴──────────────┐                   │
│  │  Daemon state + cmd handler │                   │
│  └──────────────┬──────────────┘                   │
│  ┌──────────────┴──────────────┐                   │
│  │  AudioBackend (FFmpeg/ALSA) │                   │
│  └─────────────────────────────┘                   │
│  ┌─────────────────────────────┐                   │
│  │  SQLite library.db          │                   │
│  └─────────────────────────────┘                   │
│  ┌─────────────────────────────┐                   │
│  │  yt-dlp (search/stream/dl)  │                   │
│  └─────────────────────────────┘                   │
└─────────────────────────────────────────────────────┘
```

# OPTIONS

`--debug`
:   Enable debug logging to stderr.

`--help`, `-h`
:   Display help and exit.

`--version`, `-v`
:   Display version and exit.

# COMMANDS

When invoked with a subcommand, **gtm** connects to the daemon and
operates in headless mode:

`play` <path|url>
:   Play a file or URL via daemon

`pause`
:   Toggle play/pause

`stop`
:   Stop playback

`next`
:   Skip to next track

`prev`
:   Go to previous track

`volume` [level]
:   Get/set volume (0–100)

`shuffle` [on/off]
:   Toggle shuffle mode

`repeat` [mode]
:   Set repeat mode (0=none, 1=all, 2=one)

`sleep` [minutes]
:   Set sleep timer in minutes

`status`
:   Show current playback status

`now`
:   Show current track info

`kill`
:   Stop the daemon process

`daemon`
:   Start the background daemon process (auto-started by the TUI).

`help`
:   Show help and usage information

`--version`, `-v`
:   Show version information

# EXAMPLES

Play a local file:

```bash
gtm play ~/music/track.flac
```

Play a YouTube URL:

```bash
gtm play https://youtube.com/watch?v=dQw4w9WgXcQ
```

Toggle play/pause:

```bash
gtm pause
```

Skip to next track:

```bash
gtm next
```

Set volume to 75%:

```bash
gtm volume 75
```

Query current volume:

```bash
gtm volume
```

Enable shuffle:

```bash
gtm shuffle on
```

Set repeat mode to repeat one:

```bash
gtm repeat 2
```

Set sleep timer (30 minutes):

```bash
gtm sleep 30
```

Show playback status:

```bash
gtm status
```

Show current track:

```bash
gtm now
```

Query status programmatically with jq:

```bash
gtm status | jq '.volume, .state'
```

Start the daemon in background and run commands:

```bash
gtm daemon &
gtm play ~/music/song.ogg
gtm next
```

# TUI KEYBINDINGS

The following commands are available in the TUI. Most have default
keybindings; these can be remapped via the configuration file.

# FILES

`~/.config/gtm/config.json`
:   User configuration file (theme, volume, crossfade, keybindings, etc.).

`~/.config/gtm/config.jsonc`
:   Optional JSONC configuration file (overrides config.json).

`$XDG_DATA_HOME/gtm/gtm.db`
:   SQLite database (music library, playlists, queue, playback state).

`$XDG_RUNTIME_DIR/gtm/gtmd.sock`
:   Unix socket for TUI\-daemon communication.

`$XDG_RUNTIME_DIR/gtm/gtmd.pid`
:   PID file for the running daemon instance.

# ENVIRONMENT

`HOME`
:   Used to locate the configuration directory.

`XDG_RUNTIME_DIR`
:   Used for the daemon socket and PID file.

`XDG_CONFIG_HOME`
:   Used for the config file location (default: `~/.config`).

`XDG_DATA_HOME`
:   Used for the library database and download directory.

`TERM`
:   Terminal type for true\-color detection and Kitty protocol support.

# BUGS

Report bugs and feature requests at:
<https://github.com/prjctimg/gtm/issues>

# SEE ALSO

`gtmd`(1), `ffplay`(1), `yt-dlp`(1)
