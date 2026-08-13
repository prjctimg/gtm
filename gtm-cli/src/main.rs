use std::path::PathBuf;

use clap::{Parser, Subcommand};
use gtm_core::client::DaemonClient;
use gtm_core::state::RepeatMode;

#[derive(Parser)]
#[command(name = "gtm", about = "GTM CLI client")]
struct Cli {
    #[arg(
        long,
        default_value = "/run/user/1000/gtmd.socket",
        help = "Daemon socket path"
    )]
    socket: PathBuf,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    // ─── Playback ───
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
        mode: RepeatMode,
    },
    Mute,
    Crossfade {
        enabled: bool,
        duration_secs: Option<u8>,
    },

    // ─── Queue ───
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

    // ─── Library ───
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

    // ─── Favourites ───
    Favourites,
    FavouriteAdd {
        track_id: i64,
    },
    FavouriteRemove {
        track_id: i64,
    },

    // ─── YouTube ───
    YtSearch {
        query: String,
        filter: Option<String>,
    },
    YtPoll,
    YtCancel,
    YtResolve {
        url: String,
    },

    // ─── Search ───
    Search {
        query: String,
    },

    // ─── System ───
    Status,
    Ping,
    Quit,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cli = Cli::parse();
    let mut client = DaemonClient::connect(&cli.socket).await?;

    match &cli.command {
        Command::Play { path, start_pos } => {
            let pos = start_pos.unwrap_or(0.0);
            let v = client.play(path, pos).await?;
            println!("ok version={v}");
        }
        Command::PlayPause => {
            let v = client.play_pause().await?;
            println!("ok version={v}");
        }
        Command::Pause => {
            let v = client.pause().await?;
            println!("ok version={v}");
        }
        Command::Stop => {
            let v = client.stop().await?;
            println!("ok version={v}");
        }
        Command::Next => {
            let v = client.next().await?;
            println!("ok version={v}");
        }
        Command::Prev => {
            let v = client.prev().await?;
            println!("ok version={v}");
        }
        Command::Seek { position_secs } => {
            let v = client.seek(*position_secs).await?;
            println!("ok version={v}");
        }
        Command::Volume { volume } => {
            let v = client.set_volume(*volume).await?;
            println!("ok version={v}");
        }
        Command::Shuffle => {
            let v = client.toggle_shuffle().await?;
            println!("ok version={v}");
        }
        Command::Repeat { mode } => {
            let v = client.cycle_repeat(*mode).await?;
            println!("ok version={v}");
        }
        Command::Mute => {
            let v = client.toggle_mute().await?;
            println!("ok version={v}");
        }
        Command::Crossfade {
            enabled,
            duration_secs,
        } => {
            let dur = duration_secs.unwrap_or(5);
            let v = client.crossfade(*enabled, dur).await?;
            println!("ok version={v}");
        }
        Command::Queue => {
            let res = client.queue_list().await?;
            println!("{}", serde_json::to_string_pretty(&res)?);
        }
        Command::QueueAdd { path, position } => {
            let v = client.queue_add(path, *position).await?;
            println!("ok version={v}");
        }
        Command::QueueAddMany { paths } => {
            let v = client.queue_add_many(paths.clone()).await?;
            println!("ok version={v}");
        }
        Command::QueueAddFolder { path } => {
            let v = client.queue_add_folder(path).await?;
            println!("ok version={v}");
        }
        Command::QueueRemove { index } => {
            let v = client.queue_remove(*index).await?;
            println!("ok version={v}");
        }
        Command::QueueMove { from, to } => {
            let v = client.queue_move(*from, *to).await?;
            println!("ok version={v}");
        }
        Command::QueueClear => {
            let v = client.queue_clear().await?;
            println!("ok version={v}");
        }
        Command::QueueSet { paths, start_idx } => {
            let v = client.queue_set(paths.clone(), *start_idx).await?;
            println!("ok version={v}");
        }
        Command::Scan { path } => {
            let v = client.library_scan(path).await?;
            println!("ok version={v}");
        }
        Command::Tracks { filter, sort } => {
            let res = client
                .library_get_tracks(filter.clone(), sort.clone())
                .await?;
            println!("{}", serde_json::to_string_pretty(&res)?);
        }
        Command::Playlists => {
            let res = client.library_get_playlists().await?;
            println!("{}", serde_json::to_string_pretty(&res)?);
        }
        Command::CreatePlaylist { name } => {
            let v = client.library_create_playlist(name).await?;
            println!("ok version={v}");
        }
        Command::DeletePlaylist { id } => {
            let v = client.library_delete_playlist(*id).await?;
            println!("ok version={v}");
        }
        Command::AddToPlaylist {
            playlist_id,
            track_ids,
        } => {
            let v = client
                .library_add_to_playlist(*playlist_id, track_ids.clone())
                .await?;
            println!("ok version={v}");
        }
        Command::ImportM3u { path } => {
            let v = client.library_import_m3u(path).await?;
            println!("ok version={v}");
        }
        Command::Recent { count } => {
            let res = client.library_get_recent(*count).await?;
            println!("{}", serde_json::to_string_pretty(&res)?);
        }
        Command::Favourites => {
            let res = client.get_favourites().await?;
            println!("{}", serde_json::to_string_pretty(&res)?);
        }
        Command::FavouriteAdd { track_id } => {
            let v = client.add_favourite(*track_id).await?;
            println!("ok version={v}");
        }
        Command::FavouriteRemove { track_id } => {
            let v = client.remove_favourite(*track_id).await?;
            println!("ok version={v}");
        }
        Command::Search { query } => {
            let res = client.search(query).await?;
            println!("{}", serde_json::to_string_pretty(&res)?);
        }
        Command::YtSearch { query, filter } => {
            let filter = filter.as_ref().and_then(|f| {
                serde_json::from_str::<gtm_core::state::YTFilter>(&format!("\"{f}\"")).ok()
            });
            let res = client.yt_search(query, filter).await?;
            println!("{}", serde_json::to_string_pretty(&res)?);
        }
        Command::YtPoll => {
            let res = client.yt_search_poll().await?;
            println!("{}", serde_json::to_string_pretty(&res)?);
        }
        Command::YtCancel => {
            let v = client.yt_search_cancel().await?;
            println!("ok version={v}");
        }
        Command::YtResolve { url } => {
            let res = client.yt_resolve_stream(url).await?;
            println!("{}", serde_json::to_string_pretty(&res)?);
        }
        Command::Status => {
            let state = client.get_status().await?;
            println!("{}", serde_json::to_string_pretty(&state)?);
        }
        Command::Ping => {
            client.ping().await?;
            println!("pong");
        }
        Command::Quit => {
            let v = client.quit().await?;
            println!("ok version={v}");
        }
    }
    Ok(())
}
