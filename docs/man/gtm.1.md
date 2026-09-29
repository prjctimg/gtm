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

The TUI provides a built-in help buffer accessible with **?** listing every
default keybinding, grouped by topic. For configuration options and setup, see
**gtm config**(1) and the project documentation.

# TUI MODE (default)

The TUI opens on the **Library** view; there are no tabs. Three panes are
available and **Tab** / **Shift+Tab** cycles between them, or focus them
directly with **[** / **]**.

## Library

Browse tracks by one of 13 sidebar categories: All Tracks, Liked, Albums,
Artists, Playlists, Spotify, Radio, Most Played, Recently Played, Recently
Added, Genres, Folders, Top Charts. The left pane selects the category, the
centre pane lists its contents. Keys: **j**/**k** or **Up**/**Down**
(navigate), **Enter** (drill down or play), **/** (contextual search),
**Alt+S** (cycle sort in the List context).

## Settings

Settings is a floating picker opened with **Alt+,**, not a tab. The left pane
selects a category, the right pane shows its options. Keys: **j**/**k**
(navigate), **Enter** (toggle/select). The System category renders 17 rows.

### Lyrics view

Press **l** to fetch lyrics for the current track (LRCLIB, an `.lrc`/`.srt`/
timed `.json` sidecar next to the audio file, or the offline cache). When
timestamps are available the active line is highlighted and auto-follows the
playback position; enhanced-LRC sources light up per word. **Tab** moves focus
into the lyrics pane, where **j**/**k**, **PageUp**/**PageDown**, **Home**/
**End** scroll manually. **[** and **]** shift the lyric timing by ±0.1 s per
press, clamped to ±120 s, so early/late sync can be corrected. Untimed lyrics
are shown as untimestamped lines and never get a highlight.

## Global Keys

| Key | Action |
|-----|--------|
| `Space` | Play / Pause |
| `n` / `p` | Next / Previous track |
| `+` / `-` | Volume up / down (5 per press) |
| `m` | Toggle mute |
| `r` | Cycle repeat mode |
| `S` | Toggle shuffle |
| `s` | Stop |
| `.` / `,` | Seek forward / backward (±5 s per press) |
| `Alt+r` | Radio Browser (custom + top stations, tags/countries, search) |
| `Alt+T` | Cycle theme |
| `Alt+S` | Cycle library sort order |
| `Alt+O` | Play an HTTP(S) stream URL |
| `Alt+p` | Podcast feeds |
| `Alt+b` | Progress bar style |
| `Alt+1` | Toggle mono |
| `Alt+/` | Search the library |
| `Alt+l` | Add the current track to a Spotify playlist |
| `Alt+,` | Settings |
| `Alt+.` | Pick a library to show |
| `Alt+a` | About |
| `Alt+c` | Theme picker |
| `Alt+e` | Equalizer |
| `Alt+n` | Notifications |
| `Alt+q` | Queue |
| `Alt+s` | Search Spotify (requires linking) |
| `Alt+v` | Visualizer preset |
| `Alt+x` | Setup walkthrough |
| `Alt+y` | Search YouTube |
| `Alt+z` | Sleep timer |
| `l` | Fetch lyrics for current track |
| `:` | Command mode |
| `?` | Toggle help |
| `Q` / `Ctrl+Q` | Quit and stop the daemon |
| `q` | Quit the client |
| `Esc` | Close the top overlay, or pop a drill-down step |

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

**mono**
:   Toggle mono downmix.

**love**
:   Love the current track on Last.fm. Also immediate-scrobbles the play
    session.

**unlove**
:   Remove the Last.fm love flag from the current track.

**scrobble**
:   Toggle Last.fm scrobbling for this session and report the new state.

**shuffle**
:   Toggle shuffle mode for the queue.

**repeat** {off|one|all}
:   Set repeat mode.

**crossfade** *enabled* [*duration_secs*]
:   Enable or disable crossfade between tracks. Optional duration in seconds
    (default: 7, clamped to 1–30). Crossfade is skipped for Spotify tracks
    (`spotify:` URIs), which librespot cannot pre-decode.

## Queue

**queue**
:   Display the current playback queue.

**queue-add** *path*... [`--position` *index*]
:   Add tracks to the queue. Directories are scanned recursively for audio
    files. Without `--position` the tracks are inserted play-next, right
    after the current entry. Indices are positions in the merged view (the
    user queue first, then the default library list).

**queue-remove** *index*
:   Remove a track by index. Out-of-range indices are silently ignored.

**queue-move** *from* *to*
:   Move a track between positions in the merged view.

**queue-clear**
:   Clear the entire queue.

**queue-set** *paths*... `--start-idx` *index*
:   Replace the entire queue with the given paths. `--start-idx` is required
    but currently ignored — playback always starts at index 0.

## Library

**scan** *path*
:   Scan a directory for music files. There is no file watcher: the daemon
    scans its library paths once at startup, and after that you must run
    **scan** or restart the daemon. Startup also appends newly found tracks
    to the playback queue.

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
:   Show recently played tracks.

**metadata-sync** [*path*]
:   Probe tags for a single file, or for every library track. Scanning itself
    never contacts a provider. The gate is per-track: once a track qualifies
    and Deezer answers, title, artist, album, genre, year, track number and
    `album_id` are written wholesale to both the file tags and the database.

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

**yt-search** *query*
:   Search YouTube. A query starting with `http://` / `https://` is resolved
    as a direct URL instead. Provider prefixes select another source:
    `scsearch:` (SoundCloud), `bilisearch:` (Bilibili), `mcsearch:` (Mixcloud).
    Any other query is sent as `<query> official audio` so the top hits are
    single tracks. Additional providers can be registered with
    `GTM_YT_HOSTS`.

## Spotify

**spotify** *connect* *token*
:   Link the account with an access token (metadata/playlist APIs).

**spotify** *login* [*client_id*] [*port*]
:   Run the OAuth PKCE browser flow to link the account. The callback is served
    on a loopback port (default 8990, `$GTM_SPOTIFY_PORT`).

    *client_id* is accepted and ignored. The Spotify app is fixed, because
    Connect only plays audio for a token issued by the client id the session
    registers as, and a self-registered app is not one it recognises — a link
    made with one browses the whole library and plays nothing.

**spotify** *disconnect*
:   Unlink the account and delete the stored token.

**spotify** *status*
:   Show the current link/playback status.

**spotify** *sync*
:   Re-sync all playlists from the Web API.

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
:   Interactive source setup. Without a *service* argument (`spotify` or
    `lastfm`), a picker opens and every unconfigured source is
    walked through in turn. OAuth steps open your browser: Spotify captures
    the loopback callback automatically, and Last.fm just waits for you to
    click *Allow* (its desktop flow sends nothing back). With **\--cli**, run the
    plain terminal wizard instead of the TUI. The daemon is started
    automatically if it is not already running.

## Daemon

**status** [**\--stream**] [**\--cover**] [**\--lyrics**]
:   Show daemon status. With **\--stream**, stream elapsed time continuously.
    **\--cover** renders the current track's cover art as a half-block grid, and
    **\--lyrics** prints the time-synced lyric line for the current position;
    both work with and without **\--stream**, and with **\--stream** each is
    fetched once per track rather than once per tick.

**ping**
:   Ping the daemon.

**quit**
:   Shut down the daemon.

## Configuration

**config** [`--reset`] [`--validate`]
:   Open the config file in the default editor, creating it on first run.
    `--reset` restores defaults, `--validate` reports parse errors.

    Enum values are case-sensitive and several have no serde rename:
    `progress_style` takes `SeekHead` / `Classic` / `Dots` / `TrueGradient`
    and `track_sort` takes `Recents` / `RecentlyAdded` / `Alphabetical` /
    `Artist` / `Album`. An invalid value does not fail just that key — the
    entire file is discarded and defaults are used, silently. Validate before
    relying on a hand-edited file.

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
:   Edit metadata in the **library database**; the audio file is not
    rewritten. Fields: title, artist, album, genre, year, track-number. An
    empty value clears the four text fields, but `year` and `track-number`
    must parse as an integer and cannot be cleared. No range validation is
    applied here; the 1000–9999 / >0 checks apply only during metadata-sync
    write-back to file tags.

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

`XDG_CONFIG_HOME`, `XDG_DATA_HOME`, `XDG_CACHE_HOME`
:   Config, data and cache roots (default `~/.config`, `~/.local/share`,
    `~/.cache`).

`GTM_THEME_MODE`
:   Force OS-theme detection to `dark` / `light`. Honored only when
    `theme_mode = "auto"`.

`GTM_NERD_FONTS`
:   Set to `0` / `false` / `no` to force plain-ASCII glyphs.

`GTM_YT_HOSTS`
:   Register extra YouTube-family search providers.

`GTM_SPOTIFY_PORT`
:   Override the Spotify OAuth callback port (default 8990). There is no
    Last.fm equivalent: its desktop flow has no callback.

`GTK_THEME`, `XDG_STATE_HOME`
:   Probed for the OS theme (a trailing `-dark` means dark) and the Omarchy
    `colors.toml` respectively.

`RUST_LOG`
:   Daemon log verbosity, e.g. `RUST_LOG=gtm=debug`. Overrides `--verbose`.

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

$XDG_CONFIG_HOME/gtm/footer.toml
:   Optional user footer presets. Built-in presets are **Default**, **Minimal**
    and **Full**; the Default footer shows the platform icon (`System`) instead
    of the audio output device or backend — add the `Device` or `Backend`
    module to a preset when that detail is needed. A legacy `middle` key still
    parses so old files load, but its modules are discarded; use `left`/`right`.
    Individual modules cannot be toggled from the TUI.

$XDG_CONFIG_HOME/gtm/themes/*.toml
:   User theme files. All 18 colour fields are required — a file missing any of
    them is skipped silently, so the theme never appears in the picker. A user
    theme whose `name` matches a built-in replaces it.

$XDG_CONFIG_HOME/gtm/config.toml
:   Client preferences, re-read every 120 frames when the file's mtime
    changes. See **config**(1) above for the enum values that silently reset
    the file when mistyped.

# SEE ALSO

**gtmd**(1), **gtmd-ipc**(1)

# AUTHORS

prjctimg <prjctimg@outlook.com>

# BUGS

Report bugs to <https://github.com/prjctimg/gtm/issues> or by email to
<prjctimg@outlook.com>.

# COPYRIGHT

Copyright (c) 2026 prjctimg.

This is free software released under the GPL-3.0 license. See the LICENSE
file for the full license text.