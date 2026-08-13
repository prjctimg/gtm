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

fn default_socket() -> PathBuf {
    if let Ok(runtime) = std::env::var("XDG_RUNTIME_DIR") {
        PathBuf::from(runtime).join("gtmd.socket")
    } else if let Ok(tmpdir) = std::env::var("TMPDIR") {
        PathBuf::from(tmpdir).join("gtmd.socket")
    } else {
        std::env::temp_dir().join("gtmd.socket")
    }
}

pub fn run(socket: Option<String>, json: bool, cmd: &CliCommand) {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let result: Result<String, String> = rt.block_on(async {
        let socket_path = socket.map(PathBuf::from).unwrap_or_else(default_socket);

        let client = DaemonClient::connect(&socket_path)
            .await
            .map_err(|e| format!("Failed to connect to daemon at {socket_path:?}: {e}"))?;

        match cmd {
            CliCommand::Play { path, start_pos } => {
                let pos = start_pos.unwrap_or(0.0);
                client
                    .play(path, pos)
                    .await
                    .map(|v| format!("ok version={v}"))
                    .map_err(|e| e.to_string())
            }
            CliCommand::PlayPause => client
                .play_pause()
                .await
                .map(|v| format!("ok version={v}"))
                .map_err(|e| e.to_string()),
            CliCommand::Pause => client
                .pause()
                .await
                .map(|v| format!("ok version={v}"))
                .map_err(|e| e.to_string()),
            CliCommand::Stop => client
                .stop()
                .await
                .map(|v| format!("ok version={v}"))
                .map_err(|e| e.to_string()),
            CliCommand::Next => client
                .next()
                .await
                .map(|v| format!("ok version={v}"))
                .map_err(|e| e.to_string()),
            CliCommand::Prev => client
                .prev()
                .await
                .map(|v| format!("ok version={v}"))
                .map_err(|e| e.to_string()),
            CliCommand::Seek { position_secs } => client
                .seek(*position_secs)
                .await
                .map(|v| format!("ok version={v}"))
                .map_err(|e| e.to_string()),
            CliCommand::Volume { volume } => client
                .set_volume(*volume)
                .await
                .map(|v| format!("ok version={v}"))
                .map_err(|e| e.to_string()),
            CliCommand::Shuffle => client
                .toggle_shuffle()
                .await
                .map(|v| format!("ok version={v}"))
                .map_err(|e| e.to_string()),
            CliCommand::Repeat { mode } => {
                let mode: RepeatMode = mode.parse().map_err(|e: String| e)?;
                client
                    .cycle_repeat(mode)
                    .await
                    .map(|v| format!("ok version={v}"))
                    .map_err(|e| e.to_string())
            }
            CliCommand::Mute => client
                .toggle_mute()
                .await
                .map(|v| format!("ok version={v}"))
                .map_err(|e| e.to_string()),
            CliCommand::Crossfade {
                enabled,
                duration_secs,
            } => {
                let dur = duration_secs.unwrap_or(7);
                client
                    .crossfade(*enabled, dur)
                    .await
                    .map(|v| format!("ok version={v}"))
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
                .map(|v| format!("ok version={v}"))
                .map_err(|e| e.to_string()),
            CliCommand::QueueAddMany { paths } => client
                .queue_add_many(paths.clone())
                .await
                .map(|v| format!("ok version={v}"))
                .map_err(|e| e.to_string()),
            CliCommand::QueueAddFolder { path } => client
                .queue_add_dir(path)
                .await
                .map(|v| format!("ok version={v}"))
                .map_err(|e| e.to_string()),
            CliCommand::QueueRemove { index } => client
                .queue_rm(*index)
                .await
                .map(|v| format!("ok version={v}"))
                .map_err(|e| e.to_string()),
            CliCommand::QueueMove { from, to } => client
                .queue_move(*from, *to)
                .await
                .map(|v| format!("ok version={v}"))
                .map_err(|e| e.to_string()),
            CliCommand::QueueClear => client
                .queue_clear()
                .await
                .map(|v| format!("ok version={v}"))
                .map_err(|e| e.to_string()),
            CliCommand::QueueSet { paths, start_idx } => client
                .queue_set(paths.clone(), *start_idx)
                .await
                .map(|v| format!("ok version={v}"))
                .map_err(|e| e.to_string()),
            CliCommand::Scan { path } => client
                .library_scan(path)
                .await
                .map(|v| format!("ok version={v}"))
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
                .map(|v| format!("ok version={v}"))
                .map_err(|e| e.to_string()),
            CliCommand::DeletePlaylist { id } => client
                .library_delete_playlist(*id)
                .await
                .map(|v| format!("ok version={v}"))
                .map_err(|e| e.to_string()),
            CliCommand::AddToPlaylist {
                playlist_id,
                track_ids,
            } => client
                .library_add_to_playlist(*playlist_id, track_ids.clone())
                .await
                .map(|v| format!("ok version={v}"))
                .map_err(|e| e.to_string()),
            CliCommand::ImportM3u { path } => client
                .library_import_m3u(path)
                .await
                .map(|v| format!("ok version={v}"))
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
                .map(|v| format!("ok version={v}"))
                .map_err(|e| e.to_string()),
            CliCommand::FavouriteRemove { track_id } => client
                .remove_favourite(*track_id)
                .await
                .map(|v| format!("ok version={v}"))
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
                .map(|v| format!("ok version={v}"))
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
            CliCommand::Status => {
                let state = client.get_status().await.map_err(|e| e.to_string())?;
                if json {
                    serde_json::to_string_pretty(&state).map_err(|e| e.to_string())
                } else {
                    Ok(format!("{state:?}"))
                }
            }
            CliCommand::Ping => {
                client.ping().await.map_err(|e| e.to_string())?;
                Ok("pong".into())
            }
            CliCommand::Quit => client
                .quit()
                .await
                .map(|v| format!("ok version={v}"))
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
