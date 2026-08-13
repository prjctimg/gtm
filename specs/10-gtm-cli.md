# 10 — gtm-cli: CLI Controller

## Purpose

Lightweight binary that sends DaemonRequest via the Unix socket and reads DaemonResponse JSON.
No TUI, no Ratatui. Used for scripting, keybindings, and headless control.

Depends on: `gtm-core`, `clap`, `tokio`

## Subcommand Tree (clap derive)

```rust
use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "gtm", about = "Terminal music player CLI")]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,

    /// Path to daemon socket
    #[arg(long, short, global = true)]
    pub socket: Option<String>,

    /// Output as JSON (for scripting)
    #[arg(long, short, global = true)]
    pub json: bool,
}

#[derive(Subcommand)]
pub enum Command {
    // ─── Playback ───
    /// Play file(s) or URL(s)
    Play { targets: Vec<String> },
    /// Toggle play/pause
    PlayPause,
    /// Pause playback
    Pause,
    /// Stop playback
    Stop,
    /// Next track
    Next,
    /// Previous track
    Prev,
    /// Seek to position in seconds
    Seek { seconds: f64 },
    /// Get/set volume (0-100)
    Volume { level: Option<u8> },
    /// Toggle or set shuffle
    Shuffle { on: Option<bool> },
    /// Set repeat mode
    Repeat { mode: String },            // "off", "one", "all"
    /// Toggle mute
    Mute,

    // ─── Status ───
    /// Show daemon status
    Status,
    /// Show current track
    Now,

    // ─── Queue ───
    /// Queue management
    Queue {
        #[command(subcommand)]
        action: QueueCommand,
    },

    // ─── Library ───
    /// Library management
    Library {
        #[command(subcommand)]
        action: LibraryCommand,
    },

    // ─── Favourites ───
    /// Favourite management
    Favourite {
        #[command(subcommand)]
        action: FavouriteCommand,
    },

    // ─── Playlists ───
    /// Playlist management
    Playlist {
        #[command(subcommand)]
        action: PlaylistCommand,
    },

    // ─── Features ───
    /// Get/set crossfade
    Crossfade { secs: Option<u8> },
    /// Set sleep timer
    Sleep { minutes: u32 },
    /// Kill daemon
    Kill,
    /// Start daemon (fork into background)
    Daemon,

    // ─── Utility ───
    /// Get lyrics for current or specified track
    Lyrics { track_id: Option<i64> },
    /// Generate shell completions
    Completions { shell: clap_complete::Shell },
}

#[derive(Subcommand)]
pub enum QueueCommand {
    List,
    Clear,
    Add { path: String, position: Option<usize> },
    Remove { index: usize },
    Move { from: usize, to: usize },
}

#[derive(Subcommand)]
pub enum LibraryCommand {
    Scan { path: String },
    List { filter: Option<String> },
    Search { query: String },
    Recent { count: Option<usize> },
}

#[derive(Subcommand)]
pub enum FavouriteCommand {
    List,
    Add { id: i64 },
    Remove { id: i64 },
}

#[derive(Subcommand)]
pub enum PlaylistCommand {
    List,
    Create { name: String },
    Delete { id: i64 },
    Add { id: i64, track_ids: Vec<i64> },
}
```

## Main Dispatch

```rust
#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    // Determine socket path: CLI arg > XDG_RUNTIME_DIR > /tmp
    let socket_path = cli.socket
        .map(PathBuf::from)
        .unwrap_or_else(default_socket_path);

    let mut client = DaemonClient::new(socket_path);
    client.ensure_connected().await?;

    let request = build_request(&cli.command);
    let response = client.request(&request).await?;

    if cli.json {
        println!("{}", serde_json::to_string_pretty(&response)?);
    } else {
        print_human_readable(&response);
    }

    Ok(())
}

fn build_request(cmd: &Command) -> DaemonRequest {
    match cmd {
        // ... map each Command variant to DaemonRequest variant
    }
}
```

## Command → DaemonRequest Mapping

```
CLI Command                    → DaemonRequest
────────────────────────────────────────────────────
Play {targets}                 Play { path: targets[0] } (first target)
PlayPause                      PlayPause
Pause                          PlayPause (only if playing in human mode)
Stop                           Stop
Next                           Next
Prev                           Prev
Seek {seconds}                 Seek { position_secs: seconds }
Volume {Some(v)}              SetVolume { volume: v }
Volume {None}                 GetStatus (show volume)
Shuffle {Some(on)}            ToggleShuffle? (or two-step: query then set)
Repeat {mode}                  CycleRepeat { mode: parse_mode(mode) }
Mute                           ToggleMute
Status                         GetStatus
Now                            GetStatus (extract current_track)
Queue List                     Queue { action: QueueAction::List }
Queue Clear                    Queue { action: QueueAction::Clear }
Queue Add {path, pos}          Queue { action: QueueAction::Add { path, position } }
Queue Remove {index}           Queue { action: QueueAction::Remove { index } }
Queue Move {from, to}          Queue { action: QueueAction::Move { from, to } }
Library Scan {path}            Library { action: LibraryAction::Scan { path } }
Library List {filter}          Library { action: LibraryAction::GetTracks { filter, sort: None } }
Library Search {query}         Search { query }
Library Recent {count}         Library { action: LibraryAction::GetRecent { count: count.unwrap_or(10) } }
Favourite List                 GetFavourites
Favourite Add {id}             AddFavourite { track_id: id }
Favourite Remove {id}          RemoveFavourite { track_id: id }
Playlist List                  Library { action: LibraryAction::GetPlaylists }
Playlist Create {name}         Library { action: LibraryAction::CreatePlaylist { name } }
Playlist Delete {id}           Library { action: LibraryAction::DeletePlaylist { id } }
Playlist Add {id, tids}        Library { action: LibraryAction::AddToPlaylist { playlist_id: id, track_ids: tids } }
Crossfade {Some(s)}           Crossfade { enabled: true, duration_secs: s }
Crossfade {None}              GetStatus (show current crossfade)
Sleep {minutes}                Custom {"sleep", { "minutes": minutes }} (or extend DaemonRequest)
Kill                           Quit
Lyrics {Some(id)}              Custom {"lyrics", { "track_id": id }}
Lyrics {None}                  GetStatus → then Custom {"lyrics", { "track_id": current.id }}
```

## Human-readable output

```
status:
  State:     Playing
  Track:     Artist - Song Title
  Album:     Album Name (2024)
  Position:  2:34 / 4:20 (59%)
  Volume:    75%  🔀  🔁 All
  Queue:     5 tracks (cursor at 1)

queue list:
  1. [▶] Artist A - Song One                 5:01
  2.     Artist B - Song Two                 3:45
  3.     Artist C - Song Three               4:12
```

## Completions

```rust
#[cfg(feature = "completions")]
pub fn generate_completions(shell: clap_complete::Shell) {
    let mut cmd = Cli::command();
    let name = cmd.get_name().to_string();
    clap_complete::generate(shell, &mut cmd, name, &mut std::io::stdout());
}
```

## IPC Flow

```
┌──────────┐     connect     ┌──────────────────┐
│ gtm-cli  │ ───────────────▶│ Unix Socket       │
│          │                 │ /run/user/$UID/   │
│          │                 │ gtmd.socket       │
│          │                 │                   │
│          │  {"cmd":"play", │                   │
│          │   "path":"..."}─┼─────────────────▶│ gtmd
│          │                 │                   │
│          │  {"ok":true}    │                   │
│          │  ◀─────────────┼──────────────────│
│          │                 │                   │
└──────────┘                 └──────────────────┘

Implementation uses DaemonClient::request() — same as gtm-tui
but without the event polling loop (single request-response).
```

## File Structure

```
gtm-cli/
├── Cargo.toml
└── src/
    ├── main.rs        # Cli parser, dispatch, output formatting
    └── completions.rs # Shell completion generation (feature-gated)
```
