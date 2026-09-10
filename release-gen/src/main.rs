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
    /// Set playback speed (1.0 = normal)
    Speed {
        /// Playback rate multiplier
        rate: Option<f32>,
    },
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
    PlaylistDedup {
        playlist_id: i64,
    },
    PlaylistDoctor {
        playlist_id: i64,
    },
    PlaylistSort {
        playlist_id: i64,
        #[arg(long, value_name = "FIELD", default_value = "title")]
        field: String,
    },
    AddToPlaylist {
        playlist_id: i64,
        track_ids: Vec<i64>,
    },
    /// Import a playlist file (M3U8 or PLS)
    ImportPlaylist {
        #[arg(value_name = "FILE", value_hint = clap::ValueHint::FilePath)]
        path: String,
        #[arg(long, value_name = "FORMAT", value_parser = ["m3u8", "pls"], default_value = "m3u8")]
        format: String,
    },
    /// Export a playlist to a playlist file (M3U8 or PLS)
    ExportPlaylist {
        playlist_id: i64,
        #[arg(value_name = "FILE", value_hint = clap::ValueHint::FilePath)]
        path: String,
        #[arg(long, value_name = "FORMAT", value_parser = ["m3u8", "pls"], default_value = "m3u8")]
        format: String,
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
    /// Toggle low-power mode (pause playback, ease off background work)
    LowPower {
        /// Force on/off instead of toggling state
        #[arg(long, value_name = "on|off")]
        set: Option<bool>,
    },
    /// List available audio output devices
    AudioDevices,
    /// Switch audio output device ("default" restores the system default; switching stops playback)
    SetAudioDevice {
        /// Output device name
        #[arg(value_name = "NAME")]
        name: String,
    },
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
    /// Subsonic/Navidrome server access
    #[command(subcommand)]
    Subsonic(SubsonicAction),
    /// Podcast feed management and playback
    #[command(subcommand)]
    Podcast(PodcastAction),
    /// Internet radio via radio-browser.info
    #[command(subcommand)]
    Radio(RadioAction),
    /// Walk through setting up integration sources (Spotify, Last.fm,
    /// Subsonic/Navidrome). With no SERVICE argument every unconfigured
    /// source is visited; OAuth steps launch your browser and capture the
    /// response.
    Setup {
        /// Service to configure: spotify | lastfm | subsonic
        #[arg(value_name = "SERVICE")]
        service: Option<String>,
        /// Run the plain terminal wizard instead of the TUI
        #[arg(long)]
        cli: bool,
    },
}

#[derive(Subcommand)]
enum SpotifyAction {
    /// Link the account with an access token (metadata/playlist APIs)
    Connect {
        /// Spotify OAuth access token
        token: String,
    },
    /// Run the OAuth browser flow to link the account
    Login {
        /// Spotify Client ID (dialog prompt when absent)
        client_id: Option<String>,
        /// Loopback port for the OAuth callback (default 8990)
        port: Option<u16>,
    },
    /// Unlink the account and delete the stored token
    Disconnect,
    /// Show the current link/playback status
    Status,
    /// Re-sync all playlists from the Web API
    Sync,
}

#[derive(Subcommand)]
enum SubsonicAction {
    /// Save server credentials and verify the connection
    Configure {
        /// Server URL, e.g. https://music.example.com/rest
        server: String,
        /// Subsonic username
        username: String,
        /// Password (interactive prompt when absent)
        password: Option<String>,
    },
    /// Forget stored Subsonic credentials
    Clear,
    /// Show the current Subsonic configuration state
    Status,
    /// Ping the server
    Ping,
    /// Search the server's index
    Search { query: String },
    /// Play a track by its server-side id
    Play { track_id: String },
}

#[derive(Subcommand)]
enum PodcastAction {
    /// Subscribe to a podcast feed
    Add {
        /// Feed URL (RSS/Atom)
        url: String,
    },
    /// Unsubscribe from a feed
    Remove { feed_id: String },
    /// List subscribed feeds
    List,
    /// List episodes of a feed
    Episodes { feed_id: String },
    /// Refresh all feeds (or one) from the network
    Refresh { feed_id: Option<String> },
    /// Play an episode from a feed
    Play {
        feed_id: String,
        /// Zero-based episode index
        episode_index: usize,
    },
    /// Show podcast state
    Status,
}

#[derive(Subcommand)]
enum RadioAction {
    /// Search for stations by name/tag
    Search {
        query: String,
        /// Maximum number of results
        limit: u16,
    },
    /// List the top-rated stations
    Top { limit: u16 },
    /// Play a station by its radio-browser id
    Play {
        station_id: String,
        /// Optional display name
        station_name: Option<String>,
    },
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

    // Copy install.sh to artifacts for release packaging
    if let Err(e) = fs::copy("../../install.sh", format!("{outdir}/install.sh")) {
        eprintln!("Warning: failed to copy install.sh: {e}");
    }

    println!("Generated completions in {comp_dir}/");
}
