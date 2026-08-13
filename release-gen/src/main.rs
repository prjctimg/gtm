use std::fs;

use clap::{Parser, Subcommand};
use clap_complete::Shell;
use clap_mangen::Man;

/// GTM CLI client
#[derive(Parser)]
#[command(name = "gtm")]
struct Cli {
    #[arg(long, default_value = "/run/user/1000/gtmd.socket", help = "Daemon socket path")]
    socket: String,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Play a track by path or URL
    Play { path: String, start_pos: Option<f64> },
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
    Seek { position_secs: f64 },
    /// Set volume (0-100)
    Volume { volume: u8 },
    /// Toggle shuffle mode
    Shuffle,
    /// Set repeat mode (Off, One, All)
    Repeat { mode: String },
    /// Toggle mute
    Mute,
    /// Set crossfade
    Crossfade { enabled: bool, duration_secs: Option<u8> },
    /// Show the current queue
    Queue,
    /// Add a track to the queue
    QueueAdd { path: String, position: Option<u128> },
    /// Add multiple tracks to the queue
    QueueAddMany { paths: Vec<String> },
    /// Add a folder of tracks to the queue
    QueueAddFolder { path: String },
    /// Remove a track from the queue by index
    QueueRemove { index: u128 },
    /// Move a track in the queue
    QueueMove { from: u128, to: u128 },
    /// Clear the queue
    QueueClear,
    /// Replace the queue with a new set of paths
    QueueSet { paths: Vec<String>, start_idx: u128 },
    /// Scan a directory for music files
    Scan { path: String },
    /// List tracks with optional filter and sort
    Tracks { filter: Option<String>, sort: Option<String> },
    /// List playlists
    Playlists,
    /// Create a new playlist
    CreatePlaylist { name: String },
    /// Delete a playlist
    DeletePlaylist { id: i64 },
    /// Add tracks to a playlist
    AddToPlaylist { playlist_id: i64, track_ids: Vec<i64> },
    /// Import an M3U file as a playlist
    ImportM3u { path: String },
    /// Get recently added tracks
    Recent { count: u128 },
    /// List favourite tracks
    Favourites,
    /// Add a track to favourites
    FavouriteAdd { track_id: i64 },
    /// Remove a track from favourites
    FavouriteRemove { track_id: i64 },
    /// Search YouTube
    YtSearch { query: String, filter: Option<String> },
    /// Poll for YouTube search results
    YtPoll,
    /// Cancel a YouTube search
    YtCancel,
    /// Resolve a YouTube stream URL
    YtResolve { url: String },
    /// Search the local library
    Search { query: String },
    /// Show daemon status
    Status,
    /// Ping the daemon
    Ping,
    /// Quit the daemon
    Quit,
}

/// GTM background audio daemon
#[derive(Parser, Debug)]
#[command(name = "gtmd")]
struct DaemonArgs {
    #[arg(long, help = "Unix socket path")]
    socket: Option<String>,
    #[arg(long, help = "Library database path")]
    library: Option<String>,
    #[arg(long, help = "Config directory path")]
    config: Option<String>,
    #[arg(short, long, help = "Enable verbose logging")]
    verbose: bool,
    #[arg(long, help = "Test mode (ephemeral socket, no daemonize)")]
    test_mode: bool,
    #[arg(long, help = "Audio backend (rodio)")]
    backend: Option<String>,
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 2 {
        eprintln!("Usage: release-gen <man-gtm|man-gtmd|completions-gtm <shell>|completions-gtmd <shell>|all <outdir>>");
        std::process::exit(1);
    }

    match args[1].as_str() {
        "man-gtm" => render_man::<Cli>("gtm", &mut std::io::stdout()),
        "man-gtmd" => render_man::<DaemonArgs>("gtmd", &mut std::io::stdout()),
        "completions-gtm" => {
            if args.len() < 3 {
                eprintln!("missing shell argument");
                std::process::exit(1);
            }
            let shell: Shell = args[2].parse().expect("invalid shell (bash, zsh, fish, powershell, elvish)");
            gen_completions::<Cli>("gtm", shell, &mut std::io::stdout());
        }
        "completions-gtmd" => {
            if args.len() < 3 {
                eprintln!("missing shell argument");
                std::process::exit(1);
            }
            let shell: Shell = args[2].parse().expect("invalid shell (bash, zsh, fish, powershell, elvish)");
            gen_completions::<DaemonArgs>("gtmd", shell, &mut std::io::stdout());
        }
        "all" => {
            let outdir = if args.len() >= 3 { &args[2] } else { "artifacts" };
            generate_all(outdir);
        }
        _ => {
            eprintln!("unknown command: {}", args[1]);
            std::process::exit(1);
        }
    }
}

fn render_man<T: Parser>(_name: &str, w: &mut impl std::io::Write) {
    let cmd = T::command();
    let man = Man::new(cmd);
    man.render(w).expect("render manpage");
}

fn gen_completions<T: Parser>(bin_name: &str, shell: Shell, w: &mut impl std::io::Write) {
    let mut cmd = T::command();
    clap_complete::generate(shell, &mut cmd, bin_name, w);
}

fn generate_all(outdir: &str) {
    let man_dir = format!("{outdir}/man");
    let comp_dir = format!("{outdir}/completions");
    fs::create_dir_all(&man_dir).expect("create man dir");
    fs::create_dir_all(&comp_dir).expect("create completions dir");

    // Manpages
    let mut buf: Vec<u8> = Vec::new();
    render_man::<Cli>("gtm", &mut buf);
    fs::write(format!("{man_dir}/gtm.1"), &buf).expect("write gtm manpage");
    buf.clear();

    render_man::<DaemonArgs>("gtmd", &mut buf);
    fs::write(format!("{man_dir}/gtmd.1"), &buf).expect("write gtmd manpage");
    buf.clear();

    // Completions
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
            "zsh" => "_zsh",   // zsh uses _ prefix
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

    println!("Generated artifacts in {outdir}/");
    println!("  man:  gtm.1, gtmd.1");
    println!("  completions: gtm.*, gtmd.*");
}
