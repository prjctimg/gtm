% GTM(1) GTM Client Manual
% prjctimg
% 2025

# NAME

gtm - terminal user interface and command-line client for the GTM music daemon

# SYNOPSIS

**gtm** [**\--socket**=*path*] [**\--cli**] [*command* [*args*]]

# DESCRIPTION

**gtm** is the client for the **gtmd**(1) music daemon.  When invoked without
the **\--cli** flag, it opens a full-screen Terminal User Interface (TUI) with
keyboard-driven navigation.  With **\--cli** (or **-c**), it acts as a
command-line client for scripting and headless control.

# TUI MODE (default)

The TUI provides three tabs navigated with **Tab** and **Shift+Tab**:

## Now Playing (1)
Shows current track info, cover art, a progress bar, volume gauge,
sleep timer, and control hints.
Keys: Space (play/pause), n (next), p (prev), +/- (volume), m (mute),
r (repeat), s (shuffle), h/l (seek).

## Library (2)
Browse tracks by category: All Tracks, Playlists, Favourites, Recent.
Left pane selects category, right pane lists tracks.
Keys: Tab (toggle pane), j/k or up/down (navigate), Enter (play),
/ (filter).

## Settings (3)
Adjust playback settings and open overlays.
Left pane selects category (Playback, Appearance, Sleep Timer, About),
right pane shows options.
Keys: Tab (toggle pane), j/k (navigate), Enter (toggle/select).

## Global Keys

| Key | Action |
|-----|--------|
| `Tab` / `Shift+Tab` | Next / Previous tab |
| `Space` | Play / Pause |
| `n` / `p` | Next / Previous track |
| `+` / `-` | Volume up / down |
| `m` | Toggle mute |
| `r` | Cycle repeat mode |
| `s` | Toggle shuffle |
| `:` | Command mode |
| `?` | Toggle help |
| `q` / `Esc` | Quit |

# CLI MODE

With the **\--cli** (or **-c**) flag, **gtm** sends a single command to the
daemon and prints the result.  Use **\--json** for machine-readable output.

## Playback

**play** *path* [*start_pos*]
:   Play a track by filesystem path or URL. Optionally start at a given
    position in seconds.

**play-pause**
:   Toggle between play and pause.

**pause**
:   Pause playback.

**stop**
:   Stop playback entirely.

**next**
:   Skip to the next track in the queue.

**prev**
:   Return to the previous track.

**seek** *position_secs*
:   Seek to a specific position in the current track (in seconds).

**volume** *volume*
:   Set the playback volume (0–100).

**mute**
:   Toggle mute.

**shuffle**
:   Toggle shuffle mode for the queue.

**repeat** {Off|One|All}
:   Set repeat mode.

**crossfade** *enabled* [*duration_secs*]
:   Enable or disable crossfade between tracks.

## Queue

**queue**
:   Display the current playback queue.

**queue-add** *path* [*position*]
:   Add a track to the queue.

**queue-add-many** *paths*...
:   Add multiple tracks at once.

**queue-add-folder** *path*
:   Add all tracks in a folder.

**queue-remove** *index*
:   Remove a track by index.

**queue-move** *from* *to*
:   Move a track between positions.

**queue-clear**
:   Clear the entire queue.

**queue-set** *paths*... *start_idx*
:   Replace the entire queue.

## Library

**scan** *path*
:   Scan a directory for music files.

**tracks** [*filter*] [*sort*]
:   List tracks in the library.

**playlists**
:   List saved playlists.

**create-playlist** *name*
:   Create a new playlist.

**delete-playlist** *id*
:   Delete a playlist by ID.

**add-to-playlist** *playlist_id* *track_ids*...
:   Add tracks to a playlist.

**recent** *count*
:   Show recently added tracks.

**search** *query*
:   Search the library.

## Favourites

**favourites**
:   List favourite tracks.

**favourite-add** *track_id*
:   Add a track to favourites.

**favourite-remove** *track_id*
:   Remove a track from favourites.

## YouTube

**yt-search** *query* [*filter*]
:   Search YouTube.

**yt-poll**
:   Poll for pending YouTube results.

**yt-cancel**
:   Cancel a YouTube search.

**yt-resolve** *url*
:   Resolve a YouTube URL to a playable stream.

## Daemon

**status**
:   Show daemon status.

**ping**
:   Ping the daemon.

**quit**
:   Shut down the daemon.

# OPTIONS

**\--socket**, **-s** *path*
:   Path to the daemon's Unix socket.

**\--cli**, **-c**
:   Run in CLI mode instead of TUI.

**\--json**, **-j**
:   Output as JSON (CLI mode only).

**\--version**, **-V**
:   Show version information.

**\--help**, **-h**
:   Show help message.

# ENVIRONMENT

`XDG_RUNTIME_DIR`
:   Used to derive the default socket path.

# FILES

`/run/user/1000/gtmd.socket`
:   Default daemon IPC socket.

# SEE ALSO

**gtmd**(1), **gtmd-ipc**(1)
