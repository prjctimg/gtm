// Copyright (c) 2025 - present
// Author: prjctimg <prjctimg@outlook.com>
// CLI mode: dispatches subcommands to the daemon via IPC
//
// This is free software released under the GPL-3.0 license.

//! CLI mode: dispatches subcommands to the daemon via IPC.
//!
//! ```text
//!  gtm play <path>
//!  gtm next
//!  gtm queue
//!  gtm status
//!       │
//!       ▼
//!  ┌────────────────────────┐
//!  │  DaemonClient::connect │  → Unix socket → gtmd
//!  │  serde_json over pipe  │
//!  └────────┬───────────────┘
//!           │ IPC request
//!           ▼
//!  ┌────────────────────────┐
//!  │  command match arms    │  Each CliCommand → client.method()
//!  │  Print result (or JSON)│
//!  └────────────────────────┘
//! ```

use std::path::PathBuf;

use clap::{Parser, Subcommand};
use gtm_core::client::DaemonClient;
use gtm_core::state::RepeatMode;

#[derive(Parser)]
#[command(
    name = "gtm",
    version = option_env!("CARGO_PKG_VERSION").unwrap_or("0.1.0"),
    about = "GTM music player"
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
}

pub fn run(socket: Option<String>, json: bool, verbose: bool, cmd: &CliCommand) {
    // `gtm config` edits the config file locally — no daemon needed.
    if matches!(cmd, CliCommand::Config) {
        if let Err(e) = open_config_in_editor() {
            eprintln!("error: {e}");
            std::process::exit(1);
        }
        return;
    }

    let rt = tokio::runtime::Runtime::new().unwrap();
    let result: Result<String, String> = rt.block_on(async {
        let socket_path = socket
            .map(PathBuf::from)
            .unwrap_or_else(gtm_core::resolve_command_socket);

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
            CliCommand::Crossfade {
                enabled,
                duration_secs,
            } => {
                let dur = duration_secs.unwrap_or(7);
                client
                    .crossfade(*enabled, dur, None)
                    .await
                    .map(|()| "ok".to_string())
                    .map_err(|e| e.to_string())
            }
            CliCommand::Queue => {
                let res = client.queue_list().await.map_err(|e| e.to_string())?;
                if json {
                    serde_json::to_string_pretty(&res).map_err(|e| e.to_string())
                } else {
                    Ok(format!("{res:?}"))
                }
            }
            CliCommand::QueueAdd { paths, position } => client
                .queue_add_many(paths.clone(), *position)
                .await
                .map(|()| "ok".to_string())
                .map_err(|e| e.to_string()),
            CliCommand::QueueRemove { index } => client
                .queue_rm(*index)
                .await
                .map(|()| "ok".to_string())
                .map_err(|e| e.to_string()),
            CliCommand::QueueMove { from, to } => client
                .queue_move(*from, *to)
                .await
                .map(|()| "ok".to_string())
                .map_err(|e| e.to_string()),
            CliCommand::QueueClear => client
                .queue_clear()
                .await
                .map(|()| "ok".to_string())
                .map_err(|e| e.to_string()),
            CliCommand::QueueSet { paths, start_idx } => client
                .queue_set(paths.clone(), *start_idx)
                .await
                .map(|()| "ok".to_string())
                .map_err(|e| e.to_string()),
            CliCommand::Scan { path } => client
                .library_scan(path)
                .await
                .map(|()| "ok".to_string())
                .map_err(|e| e.to_string()),
            CliCommand::Tracks { filter, sort } => {
                let res = client
                    .library_get_tracks(filter.clone(), sort.clone())
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
                    .library_get_playlists()
                    .await
                    .map_err(|e| e.to_string())?;
                if json {
                    serde_json::to_string_pretty(&res).map_err(|e| e.to_string())
                } else {
                    Ok(format!("{res:?}"))
                }
            }
            CliCommand::CreatePlaylist { name } => client
                .library_create_playlist(name)
                .await
                .map(|()| "ok".to_string())
                .map_err(|e| e.to_string()),
            CliCommand::DeletePlaylist { id } => client
                .library_delete_playlist(*id)
                .await
                .map(|()| "ok".to_string())
                .map_err(|e| e.to_string()),
            CliCommand::AddToPlaylist {
                playlist_id,
                track_ids,
            } => client
                .library_add_to_playlist(*playlist_id, track_ids.clone())
                .await
                .map(|()| "ok".to_string())
                .map_err(|e| e.to_string()),
            CliCommand::ImportM3u { path } => client
                .library_import_m3u(path)
                .await
                .map(|()| "ok".to_string())
                .map_err(|e| e.to_string()),
            CliCommand::ExportM3u { playlist_id, path } => client
                .library_export_m3u(*playlist_id, path)
                .await
                .map(|()| "ok".to_string())
                .map_err(|e| e.to_string()),
            CliCommand::Recent { count } => {
                let res = client
                    .library_get_recent(*count)
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
                    .library_sync_metadata(path.clone())
                    .await
                    .map_err(|e| e.to_string())?;
                let deadline = std::time::Instant::now() + std::time::Duration::from_secs(1800);
                loop {
                    tokio::time::sleep(std::time::Duration::from_millis(400)).await;
                    let st = client
                        .library_sync_status()
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
                let res = client.get_favourites().await.map_err(|e| e.to_string())?;
                if json {
                    serde_json::to_string_pretty(&res).map_err(|e| e.to_string())
                } else {
                    Ok(format!("{res:?}"))
                }
            }
            CliCommand::FavouriteAdd { track_id } => client
                .add_favourite(*track_id)
                .await
                .map(|()| "ok".to_string())
                .map_err(|e| e.to_string()),
            CliCommand::FavouriteRemove { track_id } => client
                .remove_favourite(*track_id)
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
            CliCommand::YtSearch { query, filter } => {
                let filter = filter.as_ref().and_then(|f| {
                    serde_json::from_str::<gtm_core::state::YTFilter>(&format!("\"{f}\"")).ok()
                });
                let res = client
                    .yt_search(query, filter)
                    .await
                    .map_err(|e| e.to_string())?;
                if json {
                    serde_json::to_string_pretty(&res).map_err(|e| e.to_string())
                } else {
                    Ok(format!("{res:?}"))
                }
            }
            CliCommand::YtPoll => {
                let res = client.yt_search_poll().await.map_err(|e| e.to_string())?;
                if json {
                    serde_json::to_string_pretty(&res).map_err(|e| e.to_string())
                } else {
                    Ok(format!("{res:?}"))
                }
            }
            CliCommand::YtCancel => client
                .yt_search_cancel()
                .await
                .map(|()| "ok".to_string())
                .map_err(|e| e.to_string()),
            CliCommand::YtResolve { url } => {
                let res = client
                    .yt_resolve_stream(url)
                    .await
                    .map_err(|e| e.to_string())?;
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
                    .lyrics_search(&artist, &title)
                    .await
                    .map_err(|e| e.to_string())?;
                if json {
                    serde_json::to_string_pretty(&lyrics).map_err(|e| e.to_string())
                } else {
                    match lyrics {
                        Some(l) => {
                            if l.lines.is_empty() {
                                let artist_str = if let Some(ref a) = l.artist {
                                    format!("{a} — ")
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
                    // Stream mode: continuously print status every second
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
                        print!(
                            "\rStream: {} | {}s / {}s | {}%   ",
                            track, elapsed, dur, vol
                        );
                        use std::io::Write;
                        std::io::stdout().flush().ok();
                        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                    }
                } else {
                    let state = client.get_status().await.map_err(|e| e.to_string())?;
                    if json {
                        serde_json::to_string_pretty(&state).map_err(|e| e.to_string())
                    } else {
                        let status_str = match state.status {
                            gtm_core::state::PlaybackStatus::Playing => "\x1b[32m▶ Playing\x1b[0m",
                            gtm_core::state::PlaybackStatus::Paused => "\x1b[33m⏸ Paused\x1b[0m",
                            gtm_core::state::PlaybackStatus::Stopped => "\x1b[31m⏹ Stopped\x1b[0m",
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
                                    format!("{} — {}", t.artist, title)
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
                        "\x1b[1mGTM Health Report\x1b[0m (v{})\n\
                         Daemon uptime: {:.0}s\n",
                        report.version, report.daemon_uptime_secs
                    );
                    for c in &report.components {
                        let icon = match c.status {
                            gtm_core::ipc::HealthStatus::Ok => "\x1b[32m✓\x1b[0m",
                            gtm_core::ipc::HealthStatus::Degraded => "\x1b[33m⚠\x1b[0m",
                            gtm_core::ipc::HealthStatus::Error => "\x1b[31m✗\x1b[0m",
                        };
                        out += &format!("  {icon} \x1b[1m{}\x1b[0m", c.name);
                        if let Some(ref msg) = c.message {
                            out += &format!(" — {msg}");
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
            CliCommand::Config => Ok("config opened".to_string()),
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

/// Open the config file (`~/.config/gtm/prefs.json`) in the user's default
/// editor, creating it with defaults if it does not exist yet.
///
/// Editor resolution order: `$VISUAL` → `$EDITOR` → first installed editor
/// from a common list. Values may include arguments (e.g. `code --wait`).
fn open_config_in_editor() -> Result<(), String> {
    let path = crate::app::ensure_prefs_file();
    let editor = pick_editor().ok_or_else(|| {
        format!(
            "no editor found — set $VISUAL or $EDITOR to open {}",
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
            if let Some(program) = parts.first() {
                if command_exists(program) {
                    return Some(parts);
                }
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
