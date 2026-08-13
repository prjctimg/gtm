% GTM(1) GTM CLI Client Manual
% prjctimg
% 2025

# NAME

gtm - command-line interface for the GTM music daemon

# SYNOPSIS

**gtm** [*socket-path*] *command* [*args*]

# DESCRIPTION

**gtm** is a CLI client for the **gtmd**(1) music daemon. It communicates
with the daemon over a Unix socket using a bincode-based IPC protocol.
Commands control playback, queue management, library browsing, and more.

# ARCHITECTURE

The following diagram shows the communication flow between **gtm** and the
daemon:

```
                         ┌──────────────────────────────────────┐
                         │            gtmd daemon              │
                         │                                      │
                         │  ┌─────────┐  ┌──────────────────┐  │
  ┌──────────┐           │  │  IPC    │  │  Command Router  │  │
  │  gtm-cli │  bincode  │  │  Socket │──│  (dispatch to    │  │
  │          │──────────▶│  │  Listener│  │   handler)      │  │
  │  ┌──────┐│  request  │  └────┬────┘  └────────┬─────────┘  │
  │  │ IPC  ││           │       │                │            │
  │  │ Send ││           │       │          ┌─────▼──────┐     │
  │  └──┬───┘│           │       │          │  Playback  │     │
  │     │    │           │       │          │  Manager   │     │
  │  ┌──▼───┐│           │       │          └─────┬──────┘     │
  │  │ IPC  ││           │       │                │            │
  │  │ Recv ││◀──────────│───────┘          ┌─────▼──────┐     │
  │  └──────┘│  response │                  │  Queue     │     │
  └──────────┘           │                  │  Manager   │     │
                         │                  └─────┬──────┘     │
                         │                        │            │
                         │                  ┌─────▼──────┐     │
                         │                  │  Library   │     │
                         │                  │  Manager   │     │
                         │                  └────────────┘     │
                         └──────────────────────────────────────┘
```

## Data Flow

1. **gtm** serializes a command (e.g. `Play`, `QueueAdd`) into a bincode
   packet and writes it to the Unix socket.
2. **gtmd** 's IPC listener accepts the connection, reads the packet, and
   routes it to the appropriate handler.
3. The handler executes the operation (playback, queue mutation, library
   query, etc.) and writes a response back through the socket.
4. **gtm** reads the response and displays it to the user.

# COMMANDS

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
:   Set repeat mode. Off disables repeating, One repeats the current
    track, All repeats the entire queue.

**crossfade** *enabled* [*duration_secs*]
:   Enable or disable crossfade between tracks. Optionally set the
    crossfade duration in seconds.

## Queue

**queue**
:   Display the current playback queue.

**queue-add** *path* [*position*]
:   Add a track to the queue. Optionally specify a position index.

**queue-add-many** *paths*...
:   Add multiple tracks to the queue at once.

**queue-add-folder** *path*
:   Add all tracks in a folder to the queue.

**queue-remove** *index*
:   Remove a track from the queue by its index.

**queue-move** *from* *to*
:   Move a track from one queue position to another.

**queue-clear**
:   Clear the entire queue.

**queue-set** *paths*... *start_idx*
:   Replace the entire queue with a new set of paths.

## Library

**scan** *path*
:   Scan a directory for music files and add them to the library.

**tracks** [*filter*] [*sort*]
:   List tracks in the library. Optionally filter by string and sort by
    field (e.g. `title`, `artist`, `album`).

**recent** *count*
:   Show the most recently added tracks.

**search** *query*
:   Search the library for tracks matching a query.

**favourites**
:   List all favourite tracks.

**favourite-add** *track_id*
:   Add a track to favourites.

**favourite-remove** *track_id*
:   Remove a track from favourites.

## Playlists

**playlists**
:   List all saved playlists.

**create-playlist** *name*
:   Create a new empty playlist.

**delete-playlist** *id*
:   Delete a playlist by its ID.

**add-to-playlist** *playlist_id* *track_ids*...
:   Add one or more tracks to a playlist.

**import-m3u** *path*
:   Import an M3U playlist file.

## YouTube

**yt-search** *query* [*filter*]
:   Search YouTube for tracks matching a query.

**yt-poll**
:   Poll for pending YouTube search results.

**yt-cancel**
:   Cancel an in-progress YouTube search.

**yt-resolve** *url*
:   Resolve a YouTube stream URL to a playable audio stream.

## Daemon

**status**
:   Show daemon status (playing, paused, stopped, current track, volume,
    etc.).

**ping**
:   Ping the daemon to verify it is running.

**quit**
:   Tell the daemon to shut down gracefully.

# OPTIONS

**--socket** *path*
:   Path to the daemon's Unix socket. Default:
    `/run/user/1000/gtmd.socket`.

# ENVIRONMENT

`XDG_RUNTIME_DIR`
:   Used to derive the default socket path when `--socket` is not given.

# FILES

`/run/user/1000/gtmd.socket`
:   Default daemon IPC socket.

# SEE ALSO

**gtmd**(1)
