use std::fs;

use clap::{Parser, Subcommand};
use clap_complete::Shell;

/// gtm music player client
#[derive(Parser)]
#[command(name = "gtm")]
struct Cli {
    #[arg(long, short, help = "Run in CLI mode instead of TUI")]
    cli: bool,
    #[arg(long, short, global = true, help = "Daemon socket path")]
    socket: Option<String>,
    #[arg(long, short, global = true, help = "Output as JSON")]
    json: bool,
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    /// Play a track at an optional start position
    Play {
        /// Path to the audio file
        #[arg(value_name = "PATH", value_hint = clap::ValueHint::FilePath)]
        path: String,
        #[arg(value_name = "SECONDS")]
        start_pos: Option<f64>,
    },
    PlayPause,
    Pause,
    Stop,
    Next,
    Prev,
    Seek {
        position_secs: f64,
    },
    Volume {
        volume: u8,
    },
    Shuffle,
    /// Cycle or set the repeat mode
    Repeat {
        #[arg(value_name = "MODE", value_parser = ["off", "one", "all"])]
        mode: String,
    },
    Mute,
    /// Toggle crossfade between tracks
    Crossfade {
        #[arg(
            value_name = "ENABLED",
            action = clap::ArgAction::Set,
            value_parser = clap::builder::BoolishValueParser::new()
        )]
        enabled: bool,
        duration_secs: Option<u8>,
    },
    /// Show the current queue
    Queue,
    /// Add one or more files or folders to the queue
    ///
    /// Directories are scanned recursively for audio files. Without a
    /// position the tracks are queued to play next.
    QueueAdd {
        /// File or folder paths to add
        #[arg(value_name = "PATH", value_hint = clap::ValueHint::AnyPath, num_args = 1..)]
        paths: Vec<String>,

        /// Insert at this merged-view index instead of "play next"
        #[arg(long, value_name = "INDEX")]
        position: Option<u64>,
    },
    QueueRemove {
        index: u64,
    },
    QueueMove {
        from: u64,
        to: u64,
    },
    QueueClear,
    /// Replace the queue with a set of tracks
    QueueSet {
        #[arg(value_name = "PATH", value_hint = clap::ValueHint::AnyPath, num_args = 1..)]
        paths: Vec<String>,
        /// Merged-view index of the entry to start playback at
        #[arg(long, value_name = "INDEX")]
        start_idx: u64,
    },
    /// Scan a directory for tracks
    Scan {
        #[arg(value_name = "DIR", value_hint = clap::ValueHint::DirPath)]
        path: String,
    },
    Tracks {
        filter: Option<String>,
        sort: Option<String>,
    },
    Playlists,
    CreatePlaylist {
        name: String,
    },
    DeletePlaylist {
        id: i64,
    },
    AddToPlaylist {
        playlist_id: i64,
        track_ids: Vec<i64>,
    },
    /// Import an M3U playlist file
    ImportM3u {
        #[arg(value_name = "FILE", value_hint = clap::ValueHint::FilePath)]
        path: String,
    },
    /// Export a playlist to an M3U file
    ExportM3u {
        playlist_id: i64,
        #[arg(value_name = "FILE", value_hint = clap::ValueHint::FilePath)]
        path: String,
    },
    Recent {
        count: u64,
    },
    /// Enrich unreliable track metadata via Deezer and embed tags into the files
    MetadataSync {
        /// Only sync this single track; otherwise all unreliable tracks
        #[arg(value_name = "PATH", value_hint = clap::ValueHint::FilePath)]
        path: Option<String>,
    },
    Favourites,
    FavouriteAdd {
        track_id: i64,
    },
    FavouriteRemove {
        track_id: i64,
    },
    YtSearch {
        query: String,
        filter: Option<String>,
    },
    YtPoll,
    YtCancel,
    /// Resolve a stream URL for playback
    YtResolve {
        #[arg(value_hint = clap::ValueHint::Url)]
        url: String,
    },
    /// Fetch lyrics for an "Artist - Title" query via lrclib
    Lyrics {
        /// Search query in the form "Artist - Title"
        query: String,
    },
    Search {
        query: String,
    },
    Status {
        /// Stream elapsed time continuously
        #[arg(long)]
        stream: bool,
    },
    CheckHealth,
    Ping,
    Quit,
    /// Open the config file in the default editor
    Config,
    /// Set or clear the sleep timer (minutes)
    SleepTimer {
        /// Minutes until playback fades out and stops
        minutes: u32,
    },
    /// Cancel a running sleep timer
    CancelSleepTimer,
    /// Edit metadata of a library track
    UpdateMetadata {
        /// Library track id
        track_id: i64,
        /// Field to change: title, artist, album, genre, year, track-number
        #[arg(value_name = "FIELD")]
        field: String,
        /// New value (or blank to clear)
        #[arg(value_name = "VALUE")]
        value: String,
    },
    /// Spotify account and playback control
    #[command(subcommand)]
    Spotify(SpotifyAction),
}

#[derive(Subcommand)]
enum SpotifyAction {
    /// Link the account with an access token (metadata/playlist APIs)
    Connect {
        /// Spotify OAuth access token
        token: String,
    },
    /// Unlink the account and delete the stored token
    Disconnect,
    /// Show the current link/playback status
    Status,
    /// Re-sync all playlists from the Web API
    Sync,
}

/// gtm background audio daemon
#[derive(Parser, Debug)]
#[command(name = "gtmd")]
struct DaemonArgs {
    #[arg(long, help = "Unix socket path", value_hint = clap::ValueHint::AnyPath)]
    socket: Option<String>,
    #[arg(long, help = "Library database path", value_hint = clap::ValueHint::FilePath)]
    library: Option<String>,
    #[arg(long, help = "Config directory path", value_hint = clap::ValueHint::DirPath)]
    config: Option<String>,
    #[arg(short, long, help = "Enable verbose logging")]
    verbose: bool,
    #[arg(long, help = "Test mode (ephemeral socket, no daemonize)")]
    test_mode: bool,
    #[arg(long, help = "Audio backend", value_parser = ["rodio", "pulseaudio"])]
    backend: Option<String>,
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 2 {
        eprintln!(
            "Usage: release-gen <completions|completions-gtm <shell>|completions-gtmd <shell>> [outdir]"
        );
        std::process::exit(1);
    }

    match args[1].as_str() {
        "completions-gtm" => {
            if args.len() < 3 {
                eprintln!("missing shell argument");
                std::process::exit(1);
            }
            let shell: Shell = args[2]
                .parse()
                .expect("invalid shell (bash, zsh, fish, powershell, elvish)");
            gen_completions::<Cli>("gtm", shell, &mut std::io::stdout());
        }
        "completions-gtmd" => {
            if args.len() < 3 {
                eprintln!("missing shell argument");
                std::process::exit(1);
            }
            let shell: Shell = args[2]
                .parse()
                .expect("invalid shell (bash, zsh, fish, powershell, elvish)");
            gen_completions::<DaemonArgs>("gtmd", shell, &mut std::io::stdout());
        }
        "completions" | "all" => {
            let outdir = if args.len() >= 3 {
                &args[2]
            } else {
                "artifacts"
            };
            generate_completions(outdir);
        }
        _ => {
            eprintln!("unknown command: {}", args[1]);
            std::process::exit(1);
        }
    }
}

fn gen_completions<T: Parser>(bin_name: &str, shell: Shell, w: &mut impl std::io::Write) {
    let mut cmd = T::command();
    clap_complete::generate(shell, &mut cmd, bin_name, w);
}

fn generate_completions(outdir: &str) {
    let comp_dir = format!("{outdir}/completions");
    fs::create_dir_all(&comp_dir).expect("create completions dir");

    let shells = [
        (Shell::Bash, "bash"),
        (Shell::Zsh, "zsh"),
        (Shell::Fish, "fish"),
        (Shell::PowerShell, "powershell"),
        (Shell::Elvish, "elvish"),
    ];

    for (shell, ext) in &shells {
        let ext = *ext;
        let suffix = match ext {
            "bash" => "bash",
            "zsh" => "_zsh",
            "fish" => "fish",
            "powershell" => "ps1",
            "elvish" => "elv",
            _ => "completion",
        };
        let name_suffix = match ext {
            "zsh" => format!("_{}", "gtm"),
            _ => format!("gtm.{}", suffix),
        };
        let mut buf: Vec<u8> = Vec::new();
        gen_completions::<Cli>("gtm", *shell, &mut buf);
        fs::write(format!("{comp_dir}/{name_suffix}"), &buf).expect("write gtm completion");

        buf.clear();
        let name_suffix2 = match ext {
            "zsh" => format!("_{}", "gtmd"),
            _ => format!("gtmd.{}", suffix),
        };
        gen_completions::<DaemonArgs>("gtmd", *shell, &mut buf);
        fs::write(format!("{comp_dir}/{name_suffix2}"), &buf).expect("write gtmd completion");
    }

    println!("Generated completions in {comp_dir}/");
}
