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

use gtm_core::client::DaemonClient;
use gtm_core::state::RepeatMode;

use crate::CliCommand;

pub fn run(socket: Option<String>, json: bool, cmd: &CliCommand) {
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
            CliCommand::Next => client
                .next()
                .await
                .map(|()| "ok".to_string())
                .map_err(|e| e.to_string()),
            CliCommand::Prev => client
                .prev()
                .await
                .map(|()| "ok".to_string())
                .map_err(|e| e.to_string()),
            CliCommand::Seek { position_secs } => client
                .seek(*position_secs)
                .await
                .map(|()| "ok".to_string())
                .map_err(|e| e.to_string()),
            CliCommand::Volume { volume } => client
                .set_volume(*volume)
                .await
                .map(|()| "ok".to_string())
                .map_err(|e| e.to_string()),
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
            CliCommand::QueueAdd { path, position } => client
                .queue_add(path, *position)
                .await
                .map(|()| "ok".to_string())
                .map_err(|e| e.to_string()),
            CliCommand::QueueAddMany { paths } => client
                .queue_add_many(paths.clone())
                .await
                .map(|()| "ok".to_string())
                .map_err(|e| e.to_string()),
            CliCommand::QueueAddFolder { path } => client
                .queue_add_dir(path)
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
            CliCommand::Status => {
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
