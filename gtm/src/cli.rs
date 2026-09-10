// Copyright (c) 2026
// Author: prjctimg <prjctimg@outlook.com>
// CLI subcommand dispatch via daemon IPC
//
// This is free software released under the GPL-3.0 license.

use std::io::Write;
use std::path::PathBuf;

use clap::{Parser, Subcommand};
use gtm_core::client::DaemonClient;
use gtm_core::global::RepeatMode;
use gtm_core::playlist_fmt::PlaylistFormatKind;

use crate::footer::format_uptime;

/// Parse a CLI `--format` value into a [`PlaylistFormatKind`].
fn parse_format(s: &str) -> Result<PlaylistFormatKind, String> {
    match s.to_ascii_lowercase().as_str() {
        "m3u8" | "m3u" => Ok(PlaylistFormatKind::M3u8),
        "pls" => Ok(PlaylistFormatKind::Pls),
        _ => Err(format!("unknown playlist format: {s}")),
    }
}

#[derive(Parser)]
#[command(
    name = "gtm",
    version = option_env!("CARGO_PKG_VERSION").unwrap_or("0.1.0"),
    about = "gtm music player"
)]
pub struct Args {
    #[arg(long, short, help = "Run in CLI mode instead of TUI")]
    pub cli: bool,

    #[arg(long, short, global = true, help = "Verbose output")]
    pub verbose: bool,

    #[arg(long, short, global = true, help = "Daemon socket path")]
    pub socket: Option<String>,

    #[arg(long, short, global = true, help = "Output as JSON (CLI mode only)")]
    pub json: bool,

    #[command(subcommand)]
    pub command: Option<CliCommand>,
}

#[derive(Subcommand)]
pub enum CliCommand {
    /// Play an audio file or resume playback
    Play {
        #[arg(value_name = "PATH", value_hint = clap::ValueHint::FilePath)]
        path: String,
        #[arg(value_name = "SECONDS")]
        start_pos: Option<f64>,
    },
    /// Toggle play/pause
    PlayPause,
    /// Pause playback
    Pause,
    /// Stop playback
    Stop,
    /// Skip to next track
    Next,
    /// Skip to previous track
    Prev,
    /// Seek to a position in seconds
    Seek { position_secs: f64 },
    /// Set volume (0-100)
    Volume { volume: u8 },
    /// Toggle shuffle
    Shuffle,
    /// Set repeat mode (off, one, all)
    Repeat {
        #[arg(value_name = "MODE", value_parser = ["off", "one", "all"])]
        mode: String,
    },
    /// Toggle mute
    Mute,
    /// Set pitch-preserving playback speed (0.25-2.0, empty/omitted shows the current rate)
    Speed {
        #[arg(value_name = "RATE", value_parser = clap::value_parser!(f32))]
        rate: Option<f32>,
    },
    /// Enable/disable crossfade with optional duration
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
    /// Add audio files or directories to the queue
    QueueAdd {
        #[arg(value_name = "PATH", value_hint = clap::ValueHint::AnyPath, num_args = 1..)]
        paths: Vec<String>,

        #[arg(long, value_name = "INDEX")]
        position: Option<u64>,
    },
    /// Remove a track from the queue by index
    QueueRemove { index: u64 },
    /// Move a track within the queue
    QueueMove { from: u64, to: u64 },
    /// Clear the entire queue
    QueueClear,
    /// Replace the queue with the given tracks
    QueueSet {
        #[arg(value_name = "PATH", value_hint = clap::ValueHint::AnyPath, num_args = 1..)]
        paths: Vec<String>,
        #[arg(long, value_name = "INDEX")]
        start_idx: u64,
    },
    /// Scan a directory for audio files and add to library
    Scan {
        #[arg(value_name = "DIR", value_hint = clap::ValueHint::DirPath)]
        path: String,
    },
    /// List library tracks with optional filter and sort
    Tracks {
        filter: Option<String>,
        sort: Option<String>,
    },
    /// List playlists
    Playlists,
    /// Create a new playlist
    CreatePlaylist { name: String },
    /// Delete a playlist
    DeletePlaylist { id: i64 },
    /// Remove duplicate tracks from a playlist
    PlaylistDedup { playlist_id: i64 },
    /// Remove playlist entries whose audio file is missing on disk
    PlaylistDoctor { playlist_id: i64 },
    /// Sort a playlist in place (field: title, artist, album, date)
    PlaylistSort {
        playlist_id: i64,
        #[arg(long, value_name = "FIELD", default_value = "title")]
        field: String,
    },
    /// Add tracks to a playlist
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
    /// Show recently played tracks
    Recent { count: u64 },
    /// Sync metadata for a file or all library tracks
    MetadataSync {
        #[arg(value_name = "PATH", value_hint = clap::ValueHint::FilePath)]
        path: Option<String>,
    },
    /// List favourite tracks
    Favourites,
    /// Add a track to favourites
    FavouriteAdd { track_id: i64 },
    /// Remove a track from favourites
    FavouriteRemove { track_id: i64 },
    /// Search for lyrics (format: "artist - title")
    Lyrics { query: String },
    /// Search the library
    Search { query: String },
    /// Show daemon status
    Status {
        #[arg(long)]
        stream: bool,
    },
    /// Check daemon health
    CheckHealth,
    /// Ping the daemon
    Ping,
    /// Quit the daemon
    Quit,
    /// Manage config file
    Config {
        #[arg(long)]
        reset: bool,
        #[arg(long)]
        validate: bool,
    },
    /// Set a sleep timer in minutes
    SleepTimer { minutes: u32 },
    /// Cancel the current sleep timer
    CancelSleepTimer,
    /// Toggle low-power mode (pause playback, ease off background work)
    LowPower {
        #[arg(long, value_name = "on|off")]
        set: Option<bool>,
    },
    /// List available audio output devices
    AudioDevices,
    /// Switch audio output device ("default" restores the system default; switching stops playback)
    SetAudioDevice {
        #[arg(value_name = "NAME")]
        name: String,
    },
    /// Edit track metadata (field: title, artist, album, genre, year, track-number)
    UpdateMetadata {
        track_id: i64,
        #[arg(value_name = "FIELD")]
        field: String,
        #[arg(value_name = "VALUE")]
        value: String,
    },
    #[command(subcommand)]
    /// Spotify integration commands
    Spotify(SpotifyAction),
    #[command(subcommand)]
    /// Navidrome / Subsonic server integration
    Subsonic(SubsonicAction),
    #[command(subcommand)]
    /// Podcast subscriptions (RSS/Atom)
    Podcast(PodcastAction),
    #[command(subcommand)]
    /// Internet radio directory (Radio Browser)
    Radio(RadioAction),
    /// Walk through setting up integration sources that need credentials
    /// (Spotify, Last.fm, Subsonic/Navidrome). With no SERVICE argument every
    /// unconfigured source is visited; OAuth steps launch your browser and
    /// capture the response.
    Setup {
        /// Service to configure: spotify | lastfm | subsonic (default: all)
        #[arg(value_name = "SERVICE")]
        service: Option<String>,
    },
}

#[derive(Subcommand)]
pub enum SpotifyAction {
    /// Link a Spotify account with an access token
    Connect { token: String },
    /// Start the OAuth browser flow and wait for you to finish login
    Login {
        client_id: Option<String>,
        /// Local redirect port (defaults to 8990 or $GTM_SPOTIFY_PORT)
        #[clap(long)]
        port: Option<u16>,
    },
    /// Unlink the Spotify account
    Disconnect,
    /// Show Spotify connection status
    Status,
    /// Sync Spotify playlists to the library
    Sync,
}

#[derive(Subcommand)]
pub enum SubsonicAction {
    /// Save server credentials and validate the connection
    Configure {
        #[arg(value_name = "SERVER_URL")]
        server: String,
        #[arg(value_name = "USERNAME")]
        username: String,
        #[arg(value_name = "PASSWORD")]
        password: Option<String>,
    },
    /// Forget the saved server credentials
    Clear,
    /// Show connection status
    Status,
    /// Verify connectivity to the server
    Ping,
    /// Search artists, albums and songs
    Search {
        #[arg(value_name = "QUERY")]
        query: String,
    },
    /// Play a track found by search
    Play {
        #[arg(value_name = "TRACK_ID")]
        track_id: String,
    },
}

#[derive(Subcommand)]
pub enum PodcastAction {
    /// Subscribe to an RSS/Atom feed
    Add {
        #[arg(value_name = "URL")]
        url: String,
    },
    /// Unsubscribe from a feed
    Remove {
        #[arg(value_name = "FEED_ID")]
        feed_id: String,
    },
    /// List subscribed feeds
    List,
    /// Show episodes of a feed
    Episodes {
        #[arg(value_name = "FEED_ID")]
        feed_id: String,
    },
    /// Refresh feeds (optionally one by id)
    Refresh {
        #[arg(value_name = "FEED_ID")]
        feed_id: Option<String>,
    },
    /// Play an episode from a feed
    Play {
        #[arg(value_name = "FEED_ID")]
        feed_id: String,
        #[arg(value_name = "EPISODE_INDEX")]
        episode_index: usize,
    },
    /// Show podcast plugin status
    Status,
}

#[derive(Subcommand)]
pub enum RadioAction {
    /// Search the Radio Browser directory
    Search {
        #[arg(value_name = "QUERY")]
        query: String,
        #[arg(long, default_value_t = 25)]
        limit: u16,
    },
    /// Show top-voted stations
    Top {
        #[arg(long, default_value_t = 25)]
        limit: u16,
    },
    /// Play a station by its Radio Browser id
    Play {
        #[arg(value_name = "STATION_ID")]
        station_id: String,
        #[arg(value_name = "NAME", required = false)]
        station_name: Option<String>,
    },
}

pub fn run(socket: Option<String>, json: bool, verbose: bool, cmd: &CliCommand) {
    if let CliCommand::Config { reset, validate } = cmd {
        if *reset {
            let path = crate::app::ensure_prefs_file();
            let _ = std::fs::remove_file(&path);
            let path = crate::app::ensure_prefs_file();
            println!("config reset to defaults at {}", path.display());
        } else if *validate {
            let path = crate::app::ensure_prefs_file();
            match std::fs::read_to_string(&path) {
                Ok(contents) => match toml::from_str::<crate::app::Prefs>(&contents) {
                    Ok(_) => println!("config is valid"),
                    Err(e) => {
                        eprintln!("config error: {e}");
                        std::process::exit(1);
                    }
                },
                Err(e) => {
                    eprintln!("failed to read config: {e}");
                    std::process::exit(1);
                }
            }
        } else {
            if let Err(e) = open_config_in_editor() {
                eprintln!("error: {e}");
                std::process::exit(1);
            }
        }
        return;
    }

    let rt = tokio::runtime::Runtime::new().unwrap();
    let result: Result<String, String> = rt.block_on(async {
        let socket_path = socket.clone()
            .map(PathBuf::from)
            .unwrap_or_else(gtm_core::resolve_command_socket);

        // The setup wizard auto-starts the daemon so a fresh install can
        // register services without a separate daemon launch step.
        if let CliCommand::Setup { .. } = cmd {
            gtm_core::daemon_ctl::ensure_daemon_running(&socket_path).await?;
        }

        let client = DaemonClient::connect(&socket_path)
            .await
            .map_err(|e| format!("Failed to connect to daemon at {socket_path:?}: {e}"))?;

        match cmd {
            CliCommand::Play { path, start_pos } => {
                let pos = start_pos.unwrap_or(0.0);
                client
                    .play(path, pos)
                    .await
                    .map(|()| "ok".to_string())
                    .map_err(|e| e.to_string())
            }
            CliCommand::PlayPause => client
                .play_pause()
                .await
                .map(|()| "ok".to_string())
                .map_err(|e| e.to_string()),
            CliCommand::Pause => client
                .pause()
                .await
                .map(|()| "ok".to_string())
                .map_err(|e| e.to_string()),
            CliCommand::Stop => client
                .stop()
                .await
                .map(|()| "ok".to_string())
                .map_err(|e| e.to_string()),
            CliCommand::Next => {
                client.next().await.map_err(|e| e.to_string())?;
                if verbose {
                    match client.get_status().await {
                        Ok(state) => {
                            if let Some(ref t) = state.current_track {
                                Ok(format!(
                                    "Now Playing: {}\n{} - {}",
                                    t.title, t.artist, t.album
                                ))
                            } else {
                                Ok("Stopped".to_string())
                            }
                        }
                        Err(_) => Ok("ok".to_string()),
                    }
                } else {
                    Ok("ok".to_string())
                }
            }
            CliCommand::Prev => {
                client.prev().await.map_err(|e| e.to_string())?;
                if verbose {
                    match client.get_status().await {
                        Ok(state) => {
                            if let Some(ref t) = state.current_track {
                                Ok(format!(
                                    "Now Playing: {}\n{} - {}",
                                    t.title, t.artist, t.album
                                ))
                            } else {
                                Ok("Stopped".to_string())
                            }
                        }
                        Err(_) => Ok("ok".to_string()),
                    }
                } else {
                    Ok("ok".to_string())
                }
            }
            CliCommand::Seek { position_secs } => client
                .seek(*position_secs)
                .await
                .map(|()| "ok".to_string())
                .map_err(|e| e.to_string()),
            CliCommand::Volume { volume } => {
                client
                    .set_volume(*volume)
                    .await
                    .map_err(|e| e.to_string())?;
                if verbose {
                    Ok(format!("Volume: {}%", volume))
                } else {
                    Ok("ok".to_string())
                }
            }
            CliCommand::Shuffle => client
                .toggle_shuffle()
                .await
                .map(|()| "ok".to_string())
                .map_err(|e| e.to_string()),
            CliCommand::Repeat { mode } => {
                let mode: RepeatMode = mode.parse().map_err(|e: String| e)?;
                client
                    .cycle_repeat(mode)
                    .await
                    .map(|()| "ok".to_string())
                    .map_err(|e| e.to_string())
            }
            CliCommand::Mute => client
                .toggle_mute()
                .await
                .map(|()| "ok".to_string())
                .map_err(|e| e.to_string()),
            CliCommand::Speed { rate } => {
                match rate {
                    Some(r) => client
                        .set_speed(*r)
                        .await
                        .map(|()| format!("ok ({r:?}x)"))
                        .map_err(|e| e.to_string()),
                    None => {
                        let speed = client.speed().await.map_err(|e| e.to_string())?;
                        Ok(format!("speed: {speed:?}x"))
                    }
                }
            }
            CliCommand::Crossfade {
                enabled,
                duration_secs,
            } => {
                let dur = duration_secs.unwrap_or(7);
                client
                    .crossfade(*enabled, dur)
                    .await
                    .map(|()| "ok".to_string())
                    .map_err(|e| e.to_string())
            }
            CliCommand::Queue => {
                let res = client.queue().list().await.map_err(|e| e.to_string())?;
                if json {
                    serde_json::to_string_pretty(&res).map_err(|e| e.to_string())
                } else {
                    Ok(format!("{res:?}"))
                }
            }
            CliCommand::QueueAdd { paths, position } => client
                .queue()
                .add_many(paths.clone(), *position)
                .await
                .map(|()| "ok".to_string())
                .map_err(|e| e.to_string()),
            CliCommand::QueueRemove { index } => client
                .queue()
                .remove(*index)
                .await
                .map(|()| "ok".to_string())
                .map_err(|e| e.to_string()),
            CliCommand::QueueMove { from, to } => client
                .queue()
                .reorder(*from, *to)
                .await
                .map(|()| "ok".to_string())
                .map_err(|e| e.to_string()),
            CliCommand::QueueClear => client
                .queue()
                .clear()
                .await
                .map(|()| "ok".to_string())
                .map_err(|e| e.to_string()),
            CliCommand::QueueSet { paths, start_idx } => client
                .queue()
                .set(paths.clone(), *start_idx)
                .await
                .map(|()| "ok".to_string())
                .map_err(|e| e.to_string()),
            CliCommand::Scan { path } => client
                .library()
                .scan(path)
                .await
                .map(|()| "ok".to_string())
                .map_err(|e| e.to_string()),
            CliCommand::Tracks { filter, sort } => {
                let res = client
                    .library()
                    .get_tracks(filter.clone(), sort.clone())
                    .await
                    .map_err(|e| e.to_string())?;
                if json {
                    serde_json::to_string_pretty(&res).map_err(|e| e.to_string())
                } else {
                    Ok(format!("{res:?}"))
                }
            }
            CliCommand::Playlists => {
                let res = client
                    .library()
                    .get_playlists()
                    .await
                    .map_err(|e| e.to_string())?;
                if json {
                    serde_json::to_string_pretty(&res).map_err(|e| e.to_string())
                } else {
                    Ok(format!("{res:?}"))
                }
            }
            CliCommand::CreatePlaylist { name } => client
                .library()
                .create_playlist(name)
                .await
                .map(|playlists| format!("created {} playlist", playlists.len()))
                .map_err(|e| e.to_string()),
            CliCommand::DeletePlaylist { id } => client
                .library()
                .delete_playlist(*id)
                .await
                .map(|()| "ok".to_string())
                .map_err(|e| e.to_string()),
            CliCommand::PlaylistDedup { playlist_id } => client
                .library()
                .playlist_dedup(*playlist_id)
                .await
                .map(|removed| format!("removed {removed} duplicate entries"))
                .map_err(|e| e.to_string()),
            CliCommand::PlaylistDoctor { playlist_id } => client
                .library()
                .playlist_doctor(*playlist_id)
                .await
                .map(|removed| format!("removed {removed} broken entries"))
                .map_err(|e| e.to_string()),
            CliCommand::PlaylistSort {
                playlist_id,
                field,
            } => client
                .library()
                .playlist_sort(*playlist_id, field)
                .await
                .map(|()| "ok".to_string())
                .map_err(|e| e.to_string()),
            CliCommand::AddToPlaylist {
                playlist_id,
                track_ids,
            } => client
                .library()
                .add_to_playlist(*playlist_id, track_ids.clone())
                .await
                .map(|()| "ok".to_string())
                .map_err(|e| e.to_string()),
            CliCommand::ImportPlaylist { path, format } => {
                let format = parse_format(format)?;
                client
                    .library()
                    .import_playlist(path, format)
                    .await
                    .map(|playlists| format!("imported {} playlist", playlists.len()))
                    .map_err(|e| e.to_string())
            }
            CliCommand::ExportPlaylist {
                playlist_id,
                path,
                format,
            } => {
                let format = parse_format(format)?;
                client
                    .library()
                    .export_playlist(*playlist_id, path, format)
                    .await
                    .map(|()| "ok".to_string())
                    .map_err(|e| e.to_string())
            }
            CliCommand::Recent { count } => {
                let res = client
                    .library()
                    .get_recent(*count)
                    .await
                    .map_err(|e| e.to_string())?;
                if json {
                    serde_json::to_string_pretty(&res).map_err(|e| e.to_string())
                } else {
                    Ok(format!("{res:?}"))
                }
            }
            CliCommand::MetadataSync { path } => {
                client
                    .library()
                    .sync_metadata(path.clone())
                    .await
                    .map_err(|e| e.to_string())?;
                let deadline = std::time::Instant::now() + std::time::Duration::from_secs(1800);
                loop {
                    tokio::time::sleep(std::time::Duration::from_millis(400)).await;
                    let st = client
                        .library()
                        .sync_status()
                        .await
                        .map_err(|e| e.to_string())?;
                    if !st.running {
                        if json {
                            return serde_json::to_string_pretty(&st).map_err(|e| e.to_string());
                        }
                        return Ok(format!(
                            "Metadata synced: {}/{} tracks",
                            st.synced, st.total
                        ));
                    }
                    if std::time::Instant::now() >= deadline {
                        return Err("metadata sync timed out".to_string());
                    }
                }
            }
            CliCommand::Favourites => {
                let res = client
                    .favourites()
                    .list()
                    .await
                    .map_err(|e| e.to_string())?;
                if json {
                    serde_json::to_string_pretty(&res).map_err(|e| e.to_string())
                } else {
                    Ok(format!("{res:?}"))
                }
            }
            CliCommand::FavouriteAdd { track_id } => client
                .favourites()
                .add(*track_id)
                .await
                .map(|()| "ok".to_string())
                .map_err(|e| e.to_string()),
            CliCommand::FavouriteRemove { track_id } => client
                .favourites()
                .remove(*track_id)
                .await
                .map(|()| "ok".to_string())
                .map_err(|e| e.to_string()),
            CliCommand::Search { query } => {
                let res = client.search(query).await.map_err(|e| e.to_string())?;
                if json {
                    serde_json::to_string_pretty(&res).map_err(|e| e.to_string())
                } else {
                    Ok(format!("{res:?}"))
                }
            }
            CliCommand::Lyrics { query } => {
                let (artist, title) = match query.split_once(" - ") {
                    Some((a, t)) => (a.trim().to_string(), t.trim().to_string()),
                    None => (String::new(), query.trim().to_string()),
                };
                let lyrics = client
                    .lyrics()
                    .search(&artist, &title)
                    .await
                    .map_err(|e| e.to_string())?;
                if json {
                    serde_json::to_string_pretty(&lyrics).map_err(|e| e.to_string())
                } else {
                    match lyrics {
                        Some(l) => {
                            if l.lines.is_empty() {
                                let artist_str = if let Some(ref a) = l.artist {
                                    format!("{a}: ")
                                } else {
                                    String::new()
                                };
                                Ok(format!(
                                    "Found lyrics metadata for {}{} but no timed lines.",
                                    artist_str,
                                    l.title.as_ref().unwrap_or(&String::from("unknown"))
                                ))
                            } else {
                                let mut out = String::new();
                                if let Some(ref t) = l.title {
                                    out += &format!("{t}\n");
                                }
                                if let Some(ref a) = l.artist {
                                    out += &format!("{a}\n");
                                }
                                out += &"-".repeat(32);
                                out += "\n";
                                for line in &l.lines {
                                    if line.timestamp < 0.0 {
                                        out += &line.text;
                                        out += "\n";
                                    } else {
                                        let mm = (line.timestamp / 60.0) as u64;
                                        let ss = line.timestamp % 60.0;
                                        out += &format!("[{:02}:{:05.2}] {}\n", mm, ss, line.text);
                                    }
                                }
                                Ok(out)
                            }
                        }
                        None => Ok("No lyrics found.".to_string()),
                    }
                }
            }
            CliCommand::Status { stream } => {
                if *stream {
                    let mut last_track: Option<String> = None;
                    let mut lyrics: Option<gtm_core::track::LrcData> = None;
                    let mut first = true;
                    loop {
                        let state = client.get_status().await.map_err(|e| e.to_string())?;
                        let elapsed = state.time_pos as u64;
                        let dur = state.duration as u64;
                        let track = state.current_track.as_ref().map_or("No track".into(), |t| {
                            if t.artist.is_empty() {
                                t.title.clone()
                            } else {
                                format!("{} - {}", t.artist, t.title)
                            }
                        });
                        let vol = state.volume;
                        // (Re)fetch time-synced lyrics whenever the track changes.
                        let track_key = state.current_track.as_ref().map(|t| t.path.clone());
                        if track_key != last_track {
                            last_track = track_key;
                            lyrics = match &state.current_track {
                                Some(t) => client
                                    .lyrics()
                                    .search(&t.artist, &t.title)
                                    .await
                                    .ok()
                                    .flatten(),
                                None => None,
                            };
                        }
                        // Pick the active lyric line for the current position.
                        let active = lyrics.as_ref().and_then(|l| {
                            let pos = state.time_pos;
                            l.lines
                                .iter()
                                .rfind(|ln| ln.timestamp >= 0.0 && ln.timestamp <= pos)
                                .or_else(|| l.lines.iter().find(|ln| ln.timestamp < 0.0))
                                .map(|ln| ln.text.trim().to_string())
                        });
                        if !first {
                            print!("\x1b[1A");
                        }
                        first = false;
                        print!(
                            "\r\x1b[KStream: {} | {}s / {}s | {}%",
                            track, elapsed, dur, vol
                        );
                        if let Some(line) = active {
                            print!("\n\x1b[K  ♪ {}", line);
                        } else {
                            print!("\n\x1b[K");
                        }
                        std::io::stdout().flush().ok();
                        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                    }
                } else {
                    let state = client.get_status().await.map_err(|e| e.to_string())?;
                    if json {
                        serde_json::to_string_pretty(&state).map_err(|e| e.to_string())
                    } else {
                        let status_str = match state.status {
                            gtm_core::global::PlaybackStatus::Playing => "\x1b[32m▶ Playing\x1b[0m",
                            gtm_core::global::PlaybackStatus::Paused => "\x1b[33m⏸ Paused\x1b[0m",
                            gtm_core::global::PlaybackStatus::Stopped => "\x1b[31m⏹ Stopped\x1b[0m",
                        };
                        let track_str = state
                            .current_track
                            .as_ref()
                            .map(|t| {
                                let title = if t.title.is_empty() {
                                    std::path::Path::new(&t.path)
                                        .file_stem()
                                        .map(|s| s.to_string_lossy().to_string())
                                        .unwrap_or_else(|| "Unknown".into())
                                } else {
                                    t.title.clone()
                                };
                                if t.artist.is_empty() {
                                    title
                                } else {
                                    format!("{}: {}", t.artist, title)
                                }
                            })
                            .unwrap_or_else(|| "No track".into());
                        let vol_str = format!("\x1b[36m{}%\x1b[0m", state.volume);
                        let repeat_str = format!("{:?}", state.repeat);
                        let shuffle_str = if state.shuffle {
                            "\x1b[32mOn\x1b[0m"
                        } else {
                            "Off"
                        };
                        let queue_str = format!(
                            "{} tracks, cursor {}/{}",
                            state.queue.len(),
                            state.queue_cursor + 1,
                            state.queue.len().max(1)
                        );
                        let mute_str = if state.mute {
                            "\x1b[33mMuted\x1b[0m"
                        } else {
                            "Unmuted"
                        };
                        Ok(format!(
                            "\x1b[1mPlayback:\x1b[0m  {}\n\
                         \x1b[1mTrack:\x1b[0m    {}\n\
                         \x1b[1mVolume:\x1b[0m   {} ({})\n\
                         \x1b[1mRepeat:\x1b[0m   {}\n\
                         \x1b[1mShuffle:\x1b[0m  {}\n\
                         \x1b[1mQueue:\x1b[0m    {}",
                            status_str,
                            track_str,
                            vol_str,
                            mute_str,
                            repeat_str,
                            shuffle_str,
                            queue_str
                        ))
                    }
                }
            }
            CliCommand::Ping => {
                client.ping().await.map_err(|e| e.to_string())?;
                Ok("pong".into())
            }
            CliCommand::CheckHealth => {
                let report = client.check_health().await.map_err(|e| e.to_string())?;
                if json {
                    serde_json::to_string_pretty(&report).map_err(|e| e.to_string())
                } else {
                    let mut out = format!(
                        "\x1b[1mgtm Health Report\x1b[0m (v{})\n\
                         Daemon uptime: {}\n",
                        report.version,
                        format_uptime(report.daemon_uptime_secs)
                    );
                    for c in &report.components {
                        let icon = match c.status {
                            gtm_core::ipc::HealthStatus::Ok => "\x1b[32m✓\x1b[0m",
                            gtm_core::ipc::HealthStatus::Degraded => "\x1b[33m⚠\x1b[0m",
                            gtm_core::ipc::HealthStatus::Error => "\x1b[31m✗\x1b[0m",
                        };
                        out += &format!("  {icon} \x1b[1m{}\x1b[0m", c.name);
                        if let Some(ref msg) = c.message {
                            out += &format!(": {msg}");
                        }
                        if let Some(uptime) = c.uptime_secs {
                            out += &format!(" (uptime {:.0}s)", uptime);
                        }
                        out += "\n";
                    }
                    Ok(out)
                }
            }
            CliCommand::Quit => client
                .quit()
                .await
                .map(|()| "ok".to_string())
                .map_err(|e| e.to_string()),
            CliCommand::Config { .. } => unreachable!(),
            CliCommand::SleepTimer { minutes } => client
                .set_sleep_timer(*minutes)
                .await
                .map(|()| format!("sleep timer set for {minutes} min"))
                .map_err(|e| e.to_string()),
            CliCommand::CancelSleepTimer => client
                .cancel_sleep_timer()
                .await
                .map(|()| "sleep timer cancelled".to_string())
                .map_err(|e| e.to_string()),
            CliCommand::LowPower { set } => {
                match set {
                    Some(enabled) => {
                        client
                            .set_low_power(*enabled)
                            .await
                            .map(|()| format!("low-power {}", if *enabled { "on" } else { "off" }))
                    }
                    None => {
                        client
                            .low_power()
                            .await
                            .map(|on| format!("low-power {}", if on { "on" } else { "off" }))
                    }
                }
                .map_err(|e| e.to_string())
            }
            CliCommand::AudioDevices => {
                let devices = client.list_audio_devices().await.map_err(|e| e.to_string())?;
                if devices.is_empty() {
                    Ok("no output devices listed by this backend".to_string())
                } else {
                    Ok(devices.join("\n"))
                }
            }
            CliCommand::SetAudioDevice { name } => {
                if name == "default" {
                    client
                        .set_audio_device(None)
                        .await
                        .map(|()| "switched to default output device".to_string())
                        .map_err(|e| e.to_string())
                } else {
                    client
                        .set_audio_device(Some(name.clone()))
                        .await
                        .map(|()| format!("switched output device to '{name}'"))
                        .map_err(|e| e.to_string())
                }
            }
            CliCommand::UpdateMetadata {
                track_id,
                field,
                value,
            } => {
                let mut patch = gtm_core::ipc::MetadataPatch::default();
                match field.as_str() {
                    "title" => patch.title = Some(value.clone()),
                    "artist" => patch.artist = Some(value.clone()),
                    "album" => patch.album = Some(value.clone()),
                    "genre" => patch.genre = Some(value.clone()),
                    "year" => {
                        patch.year = Some(
                            value
                                .trim()
                                .parse::<i32>()
                                .map_err(|_| format!("invalid year: {value}"))?,
                        )
                    }
                    "track-number" | "track_number" => {
                        patch.track_number = Some(
                            value
                                .trim()
                                .parse::<i32>()
                                .map_err(|_| format!("invalid track number: {value}"))?,
                        )
                    }
                    other => {
                        return Err(format!(
                            "unknown field `{other}` (use title, artist, album, genre, year, \
                             track-number)"
                        ));
                    }
                }
                if patch == gtm_core::ipc::MetadataPatch::default() {
                    return Err("no field to update: pass a supported FIELD".to_string());
                }
                client
                    .library()
                    .update_metadata(*track_id, patch)
                    .await
                    .map(|()| "metadata updated".to_string())
                    .map_err(|e| e.to_string())
            }
            CliCommand::Spotify(action) => match action {
                SpotifyAction::Connect { token } => {
                    let st = client
                        .spotify()
                        .set_token(token)
                        .await
                        .map_err(|e| e.to_string())?;
                    Ok(format_spotify_status(&st))
                }
                SpotifyAction::Login { client_id, port } => spotify_login(&client, client_id.clone(), *port).await,
                SpotifyAction::Disconnect => {
                    let st = client.spotify().clear().await.map_err(|e| e.to_string())?;
                    Ok(format_spotify_status(&st))
                }
                SpotifyAction::Status => {
                    let st = client.spotify().status().await.map_err(|e| e.to_string())?;
                    Ok(format_spotify_status(&st))
                }
                SpotifyAction::Sync => client
                    .spotify()
                    .sync()
                    .await
                    .map(|()| "spotify playlists synced".to_string())
                    .map_err(|e| e.to_string()),
            },
            CliCommand::Subsonic(action) => match action {
                SubsonicAction::Configure { server, username, password } => {
                    let password = match password {
                        Some(p) => p.to_string(),
                        None => prompt("Password: ")?,
                    };
                    let st = client
                        .subsonic()
                        .configure(&server, &username, &password)
                        .await
                        .map_err(|e| e.to_string())?;
                    Ok(format_subsonic_status(&st))
                }
                SubsonicAction::Clear => client
                    .subsonic()
                    .clear()
                    .await
                    .map(|()| "subsonic credentials cleared".to_string())
                    .map_err(|e| e.to_string()),
                SubsonicAction::Status => {
                    let st = client.subsonic().status().await.map_err(|e| e.to_string())?;
                    Ok(format_subsonic_status(&st))
                }
                SubsonicAction::Ping => client
                    .subsonic()
                    .ping()
                    .await
                    .map(|()| "subsonic ping ok".to_string())
                    .map_err(|e| e.to_string()),
                SubsonicAction::Search { query } => {
                    let res = client
                        .subsonic()
                        .search(&query)
                        .await
                        .map_err(|e| e.to_string())?;
                    for a in &res.artists {
                        println!("artist  {}\t{}", a.id, a.name);
                    }
                    for a in &res.albums {
                        println!(
                            "album   {}\t{}\t{}",
                            a.id,
                            a.title,
                            if a.artist.is_empty() { "unknown" } else { &a.artist }
                        );
                    }
                    for t in &res.tracks {
                        println!(
                            "track   {}\t{} - {}",
                            t.id, t.artist, t.title
                        );
                    }
                    Ok(format!("{} artists, {} albums, {} songs", res.artists.len(), res.albums.len(), res.tracks.len()))
                }
                SubsonicAction::Play { track_id } => {
                    let track = gtm_core::subsonic::SubsonicTrack {
                        id: track_id.clone(),
                        title: track_id.clone(),
                        artist: String::new(),
                        album: String::new(),
                        duration_secs: 0,
                        ..Default::default()
                    };
                    client.subsonic().play(&track).await.map_err(|e| e.to_string())?;
                    Ok(format!("playing {track_id}"))
                }
            },
            CliCommand::Podcast(action) => match action {
                PodcastAction::Add { url } => {
                    let feeds = client
                        .podcast()
                        .add_feed(&url)
                        .await
                        .map_err(|e| e.to_string())?;
                    match feeds.first() {
                        Some(f) => Ok(format!(
                            "subscribed to {} ({})",
                            f.title,
                            f.episodes
                        )),
                        None => Err("feed returned no episodes".to_string()),
                    }
                }
                PodcastAction::Remove { feed_id } => client
                    .podcast()
                    .remove_feed(&feed_id)
                    .await
                    .map(|()| format!("removed feed {feed_id}"))
                    .map_err(|e| e.to_string()),
                PodcastAction::List => {
                    let feeds = client.podcast().feeds().await.map_err(|e| e.to_string())?;
                    for f in &feeds {
                        println!("{}\t{}\t{} episodes", f.id, f.title, f.episodes);
                    }
                    Ok(format!("{} feeds", feeds.len()))
                }
                PodcastAction::Episodes { feed_id } => {
                    let (title, eps) = client
                        .podcast()
                        .episodes(&feed_id)
                        .await
                        .map_err(|e| e.to_string())?;
                    for (i, e) in eps.iter().enumerate() {
                        let dur = e
                            .duration_secs
                            .map(|d| format_duration(d))
                            .unwrap_or_else(|| "-".into());
                        println!("{i}\t{dur}\t{}", e.title);
                    }
                    Ok(format!("{title}: {} episodes", eps.len()))
                }
                PodcastAction::Refresh { feed_id } => {
                    let n = client
                        .podcast()
                        .refresh(feed_id.as_deref())
                        .await
                        .map_err(|e| e.to_string())?;
                    Ok(format!("refreshed {n} feeds"))
                }
                PodcastAction::Play { feed_id, episode_index } => client
                    .podcast()
                    .play(&feed_id, *episode_index)
                    .await
                    .map(|()| format!("playing {feed_id}[/{episode_index}]"))
                    .map_err(|e| e.to_string()),
                PodcastAction::Status => {
                    let st = client.podcast().status().await.map_err(|e| e.to_string())?;
                    let err = st.error.as_deref().unwrap_or("none");
                    Ok(format!(
                        "podcast: {} feeds, {} episodes (last error: {err})",
                        st.feeds, st.episodes
                    ))
                }
            },
            CliCommand::Radio(action) => match action {
                RadioAction::Search { query, limit } => {
                    let stations = client
                        .radio()
                        .search(&query, *limit)
                        .await
                        .map_err(|e| e.to_string())?;
                    for s in &stations {
                        println!(
                            "{}\t{}\t{}\t{:.1} votes",
                            s.id, s.name, s.url_resolved, s.votes
                        );
                    }
                    Ok(format!("{} stations", stations.len()))
                }
                RadioAction::Top { limit } => {
                    let stations = client
                        .radio()
                        .top(*limit)
                        .await
                        .map_err(|e| e.to_string())?;
                    for s in &stations {
                        println!(
                            "{}\t{}\t{}\t{:.1} votes",
                            s.id, s.name, s.url_resolved, s.votes
                        );
                    }
                    Ok(format!("{} stations", stations.len()))
                }
                RadioAction::Play { station_id, station_name } => {
                    let name = station_name.clone().unwrap_or_else(|| "Radio".to_string());
                    client
                        .radio()
                        .play(&station_id, &name)
                        .await
                        .map(|()| format!("playing {name}"))
                        .map_err(|e| e.to_string())
                }
            },
            CliCommand::Setup { service } => setup_wizard(&client, service.as_deref()).await,
        }
    });

    match result {
        Ok(msg) => println!("{msg}"),
        Err(e) => {
            eprintln!("error: {e}");
            std::process::exit(1);
        }
    }
}

fn open_config_in_editor() -> Result<(), String> {
    let path = crate::app::ensure_prefs_file();
    let editor = pick_editor().ok_or_else(|| {
        format!(
            "no editor found: set $VISUAL or $EDITOR to open {}",
            path.display()
        )
    })?;

    let program = editor[0].clone();
    let args = &editor[1..];
    let status = std::process::Command::new(&program)
        .args(args)
        .arg(&path)
        .status()
        .map_err(|e| format!("failed to launch editor `{program}`: {e}"))?;

    if status.success() {
        println!("Opened config at {}", path.display());
        Ok(())
    } else {
        Err(format!("editor `{program}` exited with status {status}"))
    }
}

fn pick_editor() -> Option<Vec<String>> {
    for var in ["VISUAL", "EDITOR"] {
        if let Ok(val) = std::env::var(var) {
            let parts: Vec<String> = val.split_whitespace().map(String::from).collect();
            if let Some(program) = parts.first()
                && command_exists(program)
            {
                return Some(parts);
            }
        }
    }
    for name in ["vim", "nvim", "vi", "nano", "micro", "emacs", "ed"] {
        if command_exists(name) {
            return Some(vec![name.to_string()]);
        }
    }
    None
}

fn command_exists(name: &str) -> bool {
    std::env::var_os("PATH")
        .map(|paths| std::env::split_paths(&paths).any(|dir| dir.join(name).is_file()))
        .unwrap_or(false)
}

fn format_spotify_status(st: &gtm_core::spotify::SpotifyStatus) -> String {
    let mut out = if st.linked {
        format!("Linked as {}", st.user.as_deref().unwrap_or("(unknown)"))
    } else {
        "Disconnected".to_string()
    };
    if let Some(dev) = st.device.as_deref().filter(|d| !d.is_empty()) {
        out += &format!(" | device: {dev}");
    }
    if st.linked {
        if st.premium {
            out += if st.playing {
                " | playing ▶"
            } else {
                " | paused ❚❚"
            };
        } else {
            out += " | playback control needs Premium";
        }
        out += &format!(" | {} playlists, {} tracks", st.playlists, st.tracks);
    }
    if let Some(e) = st.error.as_deref() {
        out += &format!(" | error: {e}");
    }
    out
}

fn format_subsonic_status(st: &gtm_core::subsonic::SubsonicStatus) -> String {
    let mut out = if st.configured {
        format!(
            "Configured for {}@{}",
            st.user.as_deref().unwrap_or("?"),
            st.server.as_deref().unwrap_or("?")
        )
    } else {
        "Not configured".to_string()
    };
    if let Some(e) = st.error.as_deref() {
        out += &format!(" | error: {e}");
    }
    out
}

fn format_duration(secs: u64) -> String {
    let h = secs / 3600;
    let m = (secs % 3600) / 60;
    let s = secs % 60;
    if h > 0 {
        format!("{h}:{m:02}:{s:02}")
    } else {
        format!("{m}:{s:02}")
    }
}

fn prompt(msg: &str) -> Result<String, String> {
    let mut out = String::new();
    print!("{msg}");
    std::io::stdout().flush().map_err(|e| e.to_string())?;
    std::io::stdin()
        .read_line(&mut out)
        .map_err(|e| e.to_string())?;
    Ok(out.trim().to_string())
}

// ─── Source setup wizard (gtm setup) ───

/// Default loopback port used to capture the Last.fm authorization token.
const LASTFM_CALLBACK_PORT: u16 = 8991;

/// Walk through every (or one) source that needs credentials, running OAuth
/// browser steps where applicable. Returns a human-readable summary.
async fn setup_wizard(client: &DaemonClient, service: Option<&str>) -> Result<String, String> {
    let single = service.map(|s| s.trim().to_ascii_lowercase());
    match single.as_deref() {
        Some("spotify") => setup_spotify(client).await,
        Some("lastfm" | "last.fm") => setup_lastfm(client).await,
        Some("subsonic" | "navidrome") => setup_subsonic(client).await,
        Some(other) => Err(format!(
            "unknown service '{other}' (expected spotify, lastfm, or subsonic)"
        )),
        None => {
            let steps = [
                ("Spotify", setup_spotify(client).await),
                ("Last.fm", setup_lastfm(client).await),
                ("Subsonic/Navidrome", setup_subsonic(client).await),
            ];
            let mut lines = Vec::new();
            for (name, result) in steps {
                match result {
                    Ok(msg) => lines.push(format!("{name}: {msg}")),
                    Err(e) => lines.push(format!("{name}: error: {e}")),
                }
            }
            Ok(lines.join("\n"))
        }
    }
}

/// Weekly prompt → `true` for a yes-like answer.
fn confirm(msg: &str) -> Result<bool, String> {
    let ans = prompt(msg)?;
    Ok(matches!(
        ans.as_str(),
        "y" | "Y" | "yes" | "Yes" | "YES" | "true" | "1"
    ))
}

/// Masked input (falls back to a plain read when no tty is available).
fn masked_prompt(msg: &str) -> Result<String, String> {
    match rpassword::prompt_password(msg) {
        Ok(s) => Ok(s.trim().to_string()),
        Err(_) => prompt(msg),
    }
}

/// Collect a value, prefilled with the stored one; empty input keeps it.
fn value_or_default(label: &str, stored: Option<String>) -> Result<String, String> {
    match stored {
        Some(cur) => {
            println!("{label}: {cur} (stored; leave blank to keep)");
            let v = prompt("> ")?;
            Ok(if v.is_empty() { cur } else { v })
        }
        None => prompt(&format!("{label}: ")),
    }
}

/// Like [`value_or_default`] but the stored value is never echoed.
fn masked_or_default(label: &str, stored: Option<String>) -> Result<String, String> {
    match stored {
        Some(_) => {
            println!("{label}: (stored; leave blank to keep)");
            let v = masked_prompt("> ")?;
            Ok(if v.is_empty() {
                stored.unwrap()
            } else {
                v
            })
        }
        None => masked_prompt(&format!("{label}: ")),
    }
}

/// Spot integration: skipped when a Spotify account is already linked.
async fn setup_spotify(client: &DaemonClient) -> Result<String, String> {
    let st = client.spotify().status().await.map_err(|e| e.to_string())?;
    if st.linked {
        return Ok(format_spotify_status(&st));
    }
    if !confirm("Spotify is not linked. Link it now? [y/N] ")? {
        return Ok("not configured".to_string());
    }
    spotify_login(client, None, None).await
}

/// Run the Spotify OAuth PKCE link flow: resolve the client id (explicit arg >
/// keychain > masked prompt), open the authorize URL in the browser, and poll
/// the daemon until its loopback callback has captured the token.
async fn spotify_login(
    client: &DaemonClient,
    client_id: Option<String>,
    port: Option<u16>,
) -> Result<String, String> {
    let port = port
        .or_else(|| {
            std::env::var("GTM_SPOTIFY_PORT")
                .ok()
                .and_then(|v| v.parse().ok())
        })
        .unwrap_or(8990);
    // Resolve the client id: explicit arg > keychain > masked prompt (so a
    // locked keychain still lets the user log in).
    let client_id = match client_id {
        Some(c) => c,
        None => match gtm_core::secret::get_secret(gtm_core::secret::SPOTIFY_CLIENT_ID_KEY) {
            Some(c) => c,
            None => masked_prompt("Spotify Client ID: ")?,
        },
    };
    if client_id.trim().is_empty() {
        return Err("no Spotify client id provided".into());
    }
    gtm_core::secret::set_secret(gtm_core::secret::SPOTIFY_CLIENT_ID_KEY, &client_id);

    let url = client
        .spotify()
        .oauth_start(&client_id, port)
        .await
        .map_err(|e| e.to_string())?;
    println!("Open this URL in your browser to authorize gtm:\n{url}\n");
    let _ = webbrowser::open(&url);
    println!("Waiting for you to finish login in your browser…");
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(180);
    loop {
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        match client.spotify().status().await {
            Ok(st) if st.linked => return Ok(format_spotify_status(&st)),
            Ok(_) if std::time::Instant::now() < deadline => continue,
            Ok(_) => return Err("timed out waiting for Spotify login".to_string()),
            Err(e) => return Err(e.to_string()),
        }
    }
}

/// Last.fm integration: collect API credentials, open the web-auth URL, and
/// capture the token via a loopback callback (or a manual paste).
async fn setup_lastfm(client: &DaemonClient) -> Result<String, String> {
    let st = client.lastfm().status().await.map_err(|e| e.to_string())?;
    if st.ready {
        return Ok(format_lastfm_status(&st));
    }
    if !confirm("Last.fm is not linked. Set it up now? [y/N] ")? {
        return Ok("not configured".to_string());
    }

    println!("Create an API application (API key, secret, and callback URL) at:");
    println!("  https://www.last.fm/api/account/create");
    let api_key = value_or_default(
        "Last.fm API key",
        gtm_core::secret::get_secret(gtm_core::secret::LASTFM_API_KEY_KEY),
    )?;
    let api_secret = masked_or_default(
        "Last.fm API secret",
        gtm_core::secret::get_secret(gtm_core::secret::LASTFM_API_SECRET_KEY),
    )?;
    if api_key.trim().is_empty() || api_secret.trim().is_empty() {
        return Err("Last.fm API key and secret are required".into());
    }

    client
        .lastfm()
        .set_config(true, Some(api_key), Some(api_secret), None, None, None)
        .await
        .map_err(|e| e.to_string())?;

    let url = client.lastfm().auth_url().await.map_err(|e| e.to_string())?;
    println!("Open this URL in your browser to authorize gtm:\n{url}\n");
    let _ = webbrowser::open(&url);

    let token = capture_callback_token("Last.fm").await?;
    if token.trim().is_empty() {
        return Err("no Last.fm token provided — authorization not completed".into());
    }
    client
        .lastfm()
        .authenticate(token.trim())
        .await
        .map_err(|e| e.to_string())?;

    let st = client.lastfm().status().await.map_err(|e| e.to_string())?;
    Ok(format_lastfm_status(&st))
}

/// Subsonic/Navidrome integration: collect server credentials and validate.
async fn setup_subsonic(client: &DaemonClient) -> Result<String, String> {
    let st = client.subsonic().status().await.map_err(|e| e.to_string())?;
    if st.configured {
        let msg = format_subsonic_status(&st);
        if !confirm(&format!("{msg}. Reconfigure Subsonic? [y/N] "))? {
            return Ok(msg);
        }
    }
    let server = prompt("Subsonic/Navidrome server URL (https://host[:port]/rest): ")?;
    if server.trim().is_empty() {
        return Err("server URL is required".into());
    }
    let username = prompt("Username: ")?;
    let password = masked_prompt("Password: ")?;
    let st = client
        .subsonic()
        .configure(server.trim(), username.trim(), &password)
        .await
        .map_err(|e| e.to_string())?;
    Ok(format_subsonic_status(&st))
}

/// Wait for an OAuth token on a loopback callback port (`$GTM_LASTFM_PORT`,
/// default 8991) or accept a manual paste on stdin. Times out after 5 minutes.
async fn capture_callback_token(service: &str) -> Result<String, String> {
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt};

    let port = std::env::var("GTM_LASTFM_PORT")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(LASTFM_CALLBACK_PORT);
    let addr = format!("127.0.0.1:{port}");
    let listener = tokio::net::TcpListener::bind(&addr)
        .await
        .map_err(|e| format!("bind {service} callback server to {addr}: {e}"))?;
    println!(
        "Waiting for the {service} authorization callback on http://{addr} (5-minute timeout).\n\
         If your browser doesn't redirect there, paste the token from the address bar and press Enter."
    );
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(300);
    let mut stdin_line = String::new();
    let mut stdin_reader = tokio::io::BufReader::new(tokio::io::stdin());
    loop {
        stdin_line.clear();
        tokio::select! {
            accept = listener.accept() => {
                let (mut stream, _) = match accept {
                    Ok(pair) => pair,
                    Err(e) => return Err(format!("{service} callback accept: {e}")),
                };
                let line = {
                    let mut reader = tokio::io::BufReader::new(&mut stream);
                    let mut line = String::new();
                    let _ = reader.read_line(&mut line).await;
                    line
                };
                if let Some(token) = query_param(&line, "token") {
                    let body = format!("gtm {service} authorized. You can close this tab.");
                    let _ = stream
                        .write_all(
                            format!(
                                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: text/plain\r\nConnection: close\r\n\r\n{body}",
                                body.len()
                            )
                            .as_bytes(),
                        )
                        .await;
                    let _ = stream.flush().await;
                    return Ok(token);
                }
                let _ = stream
                    .write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
                    .await;
                let _ = stream.flush().await;
            }
            pasted = stdin_reader.read_line(&mut stdin_line) => {
                let _ = pasted;
                return Ok(stdin_line.trim().to_string());
            }
            _ = tokio::time::sleep_until(deadline) => {
                return Err(format!("timed out waiting for {service} authorization"));
            }
        }
    }
}

/// Extract a named query parameter from the first line of an HTTP request, a
/// bare path, or a URL, e.g. `"/login?token=abc&api_key=k"`.
fn query_param(line: &str, name: &str) -> Option<String> {
    let query = line.split_whitespace().find_map(|tok| tok.split_once('?').map(|(_, q)| q))?;
    for pair in query.split('&') {
        if let Some((k, v)) = pair.split_once('=') {
            if k == name && !v.is_empty() {
                return Some(v.to_string());
            }
        }
    }
    None
}

fn format_lastfm_status(st: &gtm_core::client::LastfmStatus) -> String {
    let mut out = if st.ready {
        if st.enabled {
            "Ready (scrobbling enabled)".to_string()
        } else {
            "Ready (scrobbling disabled)".to_string()
        }
    } else if st.api_key.is_some() {
        "API key set, not yet authorized".to_string()
    } else {
        "Not configured".to_string()
    };
    if let Some(sk) = st.session_token.as_deref().filter(|s| !s.is_empty()) {
        out += &format!(" | session {}", mask_credential(sk));
    }
    out
}

/// Keep a credential short: show only the first and last two characters.
fn mask_credential(s: &str) -> String {
    if s.chars().count() <= 6 {
        "****".to_string()
    } else {
        format!("{}…{}", &s[..2], &s[s.len() - 2..])
    }
}

#[cfg(test)]
mod cli_tests {
    use super::*;

    #[test]
    fn query_param_extracts_named_field() {
        assert_eq!(
            query_param("GET /?token=abc123&api_key=k2 HTTP/1.1", "token"),
            Some("abc123".to_string())
        );
        assert_eq!(
            query_param("GET /lastfm?api_key=k2&token=xyz HTTP/1.1", "token"),
            Some("xyz".to_string())
        );
        assert_eq!(
            query_param("http://127.0.0.1:8991/lastfm?token=qwe", "token"),
            Some("qwe".to_string())
        );
        assert_eq!(query_param("GET / HTTP/1.1", "token"), None);
        assert_eq!(query_param("GET /?code=abc HTTP/1.1", "token"), None);
        assert_eq!(query_param("", "token"), None);
    }

    #[test]
    fn mask_credential_hides_value() {
        assert_ne!(mask_credential("aVeryLongSecretValue"), "aVeryLongSecretValue");
        assert_eq!(mask_credential("abc"), "****");
    }
}
