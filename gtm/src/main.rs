//! GTM music player — single binary, TUI + CLI modes.
//!
//! ```text
//! ┌──────────────────────────────────────────────────────────┐
//! │                     gtm (this binary)                     │
//! │                                                          │
//! │  gtm              → TUI mode (ratatui + crossterm)       │
//! │  gtm -c           → CLI mode (prints help)               │
//! │  gtm play <path>  → CLI mode (direct subcommand)         │
//! │  gtm --version    → prints version                       │
//! │                                                          │
//! │  Both modes communicate with gtmd via Unix socket IPC    │
//! │  over JSON lines (DaemonClient in gtm-core).             │
//! └──────────────────────────────────────────────────────────┘
//! ```

mod app;
mod cli;
mod keymap;
mod ui;

use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(
    name = "gtm",
    version = option_env!("CARGO_PKG_VERSION").unwrap_or("0.1.0"),
    about = "GTM music player"
)]
struct Args {
    #[arg(long, short, help = "Run in CLI mode instead of TUI")]
    cli: bool,

    #[arg(long, short, global = true, help = "Daemon socket path")]
    socket: Option<String>,

    #[arg(long, short, global = true, help = "Output as JSON (CLI mode only)")]
    json: bool,

    #[command(subcommand)]
    command: Option<CliCommand>,
}

#[derive(Subcommand)]
enum CliCommand {
    Play { path: String, start_pos: Option<f64> },
    PlayPause,
    Pause,
    Stop,
    Next,
    Prev,
    Seek { position_secs: f64 },
    Volume { volume: u8 },
    Shuffle,
    Repeat { mode: String },
    Mute,
    Crossfade { enabled: bool, duration_secs: Option<u8> },
    Queue,
    QueueAdd { path: String, position: Option<u128> },
    QueueAddMany { paths: Vec<String> },
    QueueAddFolder { path: String },
    QueueRemove { index: u128 },
    QueueMove { from: u128, to: u128 },
    QueueClear,
    QueueSet { paths: Vec<String>, start_idx: u128 },
    Scan { path: String },
    Tracks { filter: Option<String>, sort: Option<String> },
    Playlists,
    CreatePlaylist { name: String },
    DeletePlaylist { id: i64 },
    AddToPlaylist { playlist_id: i64, track_ids: Vec<i64> },
    ImportM3u { path: String },
    Recent { count: u128 },
    Favourites,
    FavouriteAdd { track_id: i64 },
    FavouriteRemove { track_id: i64 },
    YtSearch { query: String, filter: Option<String> },
    YtPoll,
    YtCancel,
    YtResolve { url: String },
    Search { query: String },
    Status,
    Ping,
    Quit,
}

// Dispatch logic:
//
//   ┌──────────┐     subcommand? ─yes──→ cli::run()  ──→ DaemonClient IPC
//   │  args    │
//   │  parse   │     --cli flag? ──yes──→ print help
//   │          │
//   └──────────┘     no args ───────────→ ui::run_tui() → TUI event loop
//
fn main() {
    let args = Args::parse();

    if let Some(ref cmd) = args.command {
        // A subcommand was given → run in CLI mode, dispatch directly
        cli::run(args.socket, args.json, cmd);
    } else if args.cli {
        // --cli flag with no subcommand → print CLI help
        let mut cmd = Args::command();
        cmd.print_help().unwrap();
        println!();
    } else {
        // No subcommand, no --cli → launch the TUI
        let res = ui::run_tui(args.socket);
        if let Err(e) = res {
            eprintln!("Error: {e}");
            std::process::exit(1);
        }
    }
}
