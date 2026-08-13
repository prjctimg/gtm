use std::fs;

use clap::{Parser, Subcommand};
use clap_complete::Shell;

/// GTM music player client
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
    Play {
        path: String,
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
    Repeat {
        mode: String,
    },
    Mute,
    Crossfade {
        enabled: bool,
        duration_secs: Option<u8>,
    },
    Queue,
    QueueAdd {
        path: String,
        position: Option<u128>,
    },
    QueueAddMany {
        paths: Vec<String>,
    },
    QueueAddFolder {
        path: String,
    },
    QueueRemove {
        index: u128,
    },
    QueueMove {
        from: u128,
        to: u128,
    },
    QueueClear,
    QueueSet {
        paths: Vec<String>,
        start_idx: u128,
    },
    Scan {
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
    ImportM3u {
        path: String,
    },
    Recent {
        count: u128,
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
    YtResolve {
        url: String,
    },
    Search {
        query: String,
    },
    Status,
    Ping,
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
        eprintln!("Usage: release-gen <completions|completions-gtm <shell>|completions-gtmd <shell>> [outdir]");
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
