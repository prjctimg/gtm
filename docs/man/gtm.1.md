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
(YouTube, Playback, System, Spotify), right pane shows options. Keys:
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
| `Alt+R` | Top radio stations |
| `Alt+T` | Radio Browser (browse tags / countries) |
| `Alt+O` | Play an HTTP(S) stream URL |
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
    position in seconds. An `http://` or `https://` URL is treated as an
    internet stream.

**stream** *url*
:   Play an HTTP(S) stream. The URL may also point to an M3U/PLS playlist,
    which is fetched and resolved server-side; remaining playlist entries are
    queued behind the first so **next** rotates through them. Live stream
    titles (ICMP/Shoutcast `StreamTitle`) appear in the playing view.

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

**speed** [*rate*]
:   Set the playback speed (1.0 = normal). Without an argument, prints the
    current speed.

**shuffle**
:   Toggle shuffle mode for the queue.

**repeat** {off|one|all}
:   Set repeat mode.

**crossfade** *enabled* [*duration_secs*]
:   Enable or disable crossfade between tracks. Optional duration in seconds
    (default: 3).

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

**playlist-dedup** *playlist_id*
:   Remove duplicate track entries from a playlist.

**playlist-doctor** *playlist_id*
:   Remove playlist entries whose audio file is missing on disk.

**playlist-sort** *playlist_id* [`--field` *title|artist|album|date*]
:   Reorder a playlist's tracks in place. Defaults to `title`.

**import-playlist** *path* `--format` *m3u8|pls*
:   Import a playlist file (M3U8 or PLS) into the library. Defaults to M3U8.

**export-playlist** *playlist_id* *path* `--format` *m3u8|pls*
:   Export a playlist to an M3U8 or PLS file. Defaults to M3U8.

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

**spotify** *login* [*client_id*] [*port*]
:   Run the OAuth PKCE browser flow to link the account. The client id is
    taken from the argument, the keychain, or an interactive prompt. The
    callback is served on a loopback port (default 8990,
    `$GTM_SPOTIFY_PORT`).

**spotify** *disconnect*
:   Unlink the account and delete the stored token.

**spotify** *status*
:   Show the current link/playback status.

**spotify** *sync*
:   Re-sync all playlists from the Web API.

## Subsonic / Navidrome

**subsonic** *configure* *server* *username* [*password*]
:   Save server credentials and verify the connection. Omit *password* for
    an interactive prompt.

**subsonic** *clear*
:   Forget stored Subsonic credentials.

**subsonic** *status*
:   Show the current Subsonic configuration state.

**subsonic** *ping*
:   Ping the server.

**subsonic** *search* *query*
:   Search the server's index.

**subsonic** *play* *track_id*
:   Play a track by its server-side id.

## Podcast

**podcast** *add* *url*
:   Subscribe to a podcast feed (RSS/Atom URL).

**podcast** *remove* *feed_id*
:   Unsubscribe from a feed.

**podcast** *list*
:   List subscribed feeds.

**podcast** *episodes* *feed_id*
:   List episodes of a feed.

**podcast** *refresh* [*feed_id*]
:   Refresh all feeds (or a single one) from the network.

**podcast** *play* *feed_id* *episode_index*
:   Play an episode by its zero-based index in the feed.

**podcast** *status*
:   Show podcast state.

## Radio

**radio** *search* *query* *limit*
:   Search radio-browser.info for stations by name or tag.

**radio** *top* *limit*
:   List the top-rated stations.

**radio** *tags* *limit*
:   List the most-used station tags on radio-browser.info.

**radio** *tag* *tag* *limit*
:   List stations carrying a tag.

**radio** *countries* *limit*
:   List the available station countries.

**radio** *country* *country* *limit*
:   List stations from a country.

**radio** *play* *station_id* [*station_name*]
:   Play a station by its radio-browser id, optionally with a display name.

**radio** *list*
:   List locally stored custom stations (see *add* below). Each is referenced
    by a `custom:N` id where *N* is its 1-based index.

**radio** *add* *name* *url*
:   Store a custom station URL so **radio play** `custom:N` and the TUI can
    open it from any machine. Stations live in `$XDG_CONFIG_HOME/gtm/radios.toml`.

**radio** *rm* *selector*
:   Remove a custom station by `custom:N` index or by exact name.

## Setup

**setup** [*service*] [**\--cli**]
:   Interactive source setup. Without a *service* argument (`spotify`,
    `lastfm`, or `subsonic`), a picker opens and every unconfigured source is
    walked through in turn. OAuth steps (Spotify, Last.fm) open your browser
    and capture the callback response automatically; Last.fm's loopback
    capture falls back to pasting the token on stdin. With **\--cli**, run the
    plain terminal wizard instead of the TUI. The daemon is started
    automatically if it is not already running.

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

**low-power** [`--set` *on|off*]
:   Toggle low-power mode (pauses playback and eases up on background work).
    Use `--set on|off` to force a state instead of toggling. With no flag,
    prints the current state.

**audio-devices**
:   List available audio output devices.

**set-audio-device** *name*
:   Switch the audio output device (use `default` for the system default).
    Switching restarts the output and stops playback.

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

$XDG_CONFIG_HOME/gtm/radios.toml
:   Custom radio stations added with **radio add** (defaults to
    `~/.config/gtm/radios.toml`).

# SEE ALSO

**gtmd**(1), **gtmd-ipc**(1)

# AUTHORS

prjctimg <prjctimg@outlook.com>

# BUGS

Report bugs to <https://github.com/prjctimg/gtm.rs/issues> or by email to
<prjctimg@outlook.com>.

# COPYRIGHT

Copyright (c) 2026 prjctimg.

This is free software released under the GPL-3.0 license. See the LICENSE
file for the full license text.