% gtm(1) gtm client manual
% prjctimg
% 2026

# NAME

gtm - terminal user interface and command-line client for the gtm music daemon

# SYNOPSIS

**gtm** [**\--socket**=*path*] [**\--cli**] [*command* [*args*]]

# DESCRIPTION

**gtm** is the client for **gtmd**(1). When invoked without the **\--cli** flag, it
opens a full-screen Terminal User Interface (TUI) with keyboard-driven
navigation. With **\--cli** (or **-c**), it acts as a command-line client for
scripting and headless control.

The TUI provides a built-in help buffer accessible with **?** that covers all
keybindings, configuration options, and setup instructions.

# TUI MODE (default)

The TUI provides two tabs navigated with **1** / **2** or **Tab** /
**Shift+Tab**:

## Library (1)

Browse tracks by category: All Tracks, Playlists, Favourites, Recent. Left
pane selects category, right pane lists tracks. Keys: **Tab** (toggle pane),
**j**/**k** or **Up**/**Down** (navigate), **Enter** (play), **/** (filter).

## Settings (2)

Adjust playback settings and open overlays. Left pane selects category
(Playback, Appearance, Sleep Timer, About), right pane shows options. Keys:
**Tab** (toggle pane), **j**/**k** (navigate), **Enter** (toggle/select).

## Global Keys

| Key | Action |
|-----|--------|
| `Tab` / `Shift+Tab` | Next / Previous tab |
| `1` / `2` | Switch to Library / Settings tab |
| `Space` | Play / Pause |
| `n` / `p` | Next / Previous track |
| `+` / `-` | Volume up / down |
| `m` | Toggle mute |
| `r` | Cycle repeat mode |
| `S` | Toggle shuffle |
| `s` | Stop |
| `.` / `,` | Seek forward / backward |
| `l` | Fetch lyrics for current track |
| `:` | Command mode |
| `?` | Toggle help |
| `q` / `Esc` | Quit |

# CLI MODE

With the **\--cli** (or **-c**) flag, **gtm** sends a single command to the
daemon and prints the result. Use **\--json** for machine-readable output.

## Playback

**play** *path* [*start_pos*]
:   Play a track by filesystem path or URL. Optionally start at a given
    position in seconds.

**play-pause**
:   Toggle between play and pause (smart: stopped → play, playing → pause,
    paused → resume).

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
:   Set the playback volume (0-100).

**mute**
:   Toggle mute.

**shuffle**
:   Toggle shuffle mode for the queue.

**repeat** {off|one|all}
:   Set repeat mode.

**crossfade** *enabled* [*duration_secs*]
:   Enable or disable crossfade between tracks. Optional duration in seconds
    (default: 3). Easing is picked in Settings.

## Queue

**queue**
:   Display the current playback queue.

**queue-add** *path* [*position*]
:   Add a track to the queue. Directories are scanned recursively for audio
    files. Without a position the tracks are queued to play next.

**queue-remove** *index*
:   Remove a track by index.

**queue-move** *from* *to*
:   Move a track between positions.

**queue-clear**
:   Clear the entire queue.

**queue-set** *paths*... *start_idx*
:   Replace the entire queue with the given paths, starting playback at
    *start_idx*.

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

**import-m3u** *path*
:   Import an M3U playlist file into the library.

**export-m3u** *playlist_id* *path*
:   Export a playlist to an M3U file.

**recent** *count*
:   Show recently added tracks.

**search** *query*
:   Search the library.

**lyrics** *query*
:   Fetch lyrics for an "Artist - Title" query via LRCLIB.

**check-health**
:   Check daemon connectivity and return version info.

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

## Spotify

**spotify** *connect* *token*
:   Link the account with an access token (metadata/playlist APIs).

**spotify** *disconnect*
:   Unlink the account and delete the stored token.

**spotify** *status*
:   Show the current link/playback status.

**spotify** *sync*
:   Re-sync all playlists from the Web API.

## Daemon

**status** [**\--stream**]
:   Show daemon status. With **\--stream**, stream elapsed time continuously.

**ping**
:   Ping the daemon.

**quit**
:   Shut down the daemon.

## Configuration

**config**
:   Open the config file in the default editor.

**sleep-timer** *minutes*
:   Set the sleep timer (minutes until playback fades out and stops).

**cancel-sleep-timer**
:   Cancel a running sleep timer.

**update-metadata** *track_id* *field* *value*
:   Edit metadata of a library track. Fields: title, artist, album, genre,
    year, track-number. Empty value clears the field.

# OPTIONS

**\--socket**, **-s** *path*
:   Path to the daemon's Unix socket.

**\--cli**, **-c**
:   Run in CLI mode instead of TUI.

**\--json**, **-j**
:   Output as JSON (CLI mode only).

**\--verbose**, **-v**
:   Enable verbose output (global).

**\--version**, **-V**
:   Show version information.

**\--help**, **-h**
:   Show help message.

# ENVIRONMENT

`XDG_RUNTIME_DIR`
:   Used to derive the default socket path.

# FILES

$XDG_RUNTIME_DIR/gtm/gtmd.sock
:   Default daemon IPC socket.

/tmp/gtm-$USER/gtm/gtmd.sock
:   Fallback socket path if $XDG_RUNTIME_DIR is not set.

$TMPDIR/gtm/gtmd.sock
:   Further fallback.

$HOME/.gtm/gtm/gtmd.sock
:   Final fallback.

# SEE ALSO

**gtmd**(1), **gtmd-ipc**(1)

# AUTHORS

prjctimg <prjctimg@outlook.com>

# BUGS

Report bugs to <https://github.com/prjctimg/gtm.rs/issues> or by email to
<prjctimg@outlook.com>.

# COPYRIGHT

Copyright (c) 2026 - present prjctimg.

This is free software released under the GPL-3.0 license. See the LICENSE
file for the full license text.