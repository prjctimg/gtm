use std::path::Path;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::UnixStream;

use crate::ipc::{DaemonEvent, DaemonReq, DaemonRes, LibraryAction, QueueAction};
use crate::state::{DaemonState, RepeatMode, YTFilter};
use crate::wire;
use crate::CoreError;
use crate::Result;

enum Frame {
    Response(DaemonRes),
    Event(DaemonEvent),
}

pub struct DaemonClient {
    reader: tokio::net::unix::OwnedReadHalf,
    writer: tokio::net::unix::OwnedWriteHalf,
    buf: Vec<u8>,
    event_queue: Vec<DaemonEvent>,
    connected: bool,
}

impl DaemonClient {
    pub async fn connect(path: impl AsRef<Path>) -> Result<Self> {
        let stream = UnixStream::connect(path).await?;
        let (reader, writer) = stream.into_split();
        Ok(Self {
            reader,
            writer,
            buf: Vec::with_capacity(4096),
            event_queue: Vec::new(),
            connected: true,
        })
    }

    pub fn is_connected(&self) -> bool {
        self.connected
    }

    pub fn drain_events(&mut self) -> Vec<DaemonEvent> {
        std::mem::take(&mut self.event_queue)
    }

    async fn send_raw(&mut self, req: &DaemonReq) -> Result<DaemonRes> {
        let mut line = serde_json::to_string(req)?;
        line.push('\n');
        self.writer.write_all(line.as_bytes()).await?;
        self.writer.flush().await?;
        self.read_response().await
    }

    async fn read_response(&mut self) -> Result<DaemonRes> {
        loop {
            if let Some(frame) = self.try_parse() {
                match frame {
                    Frame::Response(res) => return Ok(res),
                    Frame::Event(ev) => self.event_queue.push(ev),
                }
            }
            let mut tmp = [0u8; 4096];
            let n = self.reader.read(&mut tmp).await.map_err(|e| {
                self.connected = false;
                e
            })?;
            if n == 0 {
                self.connected = false;
                return Err(CoreError::Daemon("connection closed".into()));
            }
            self.buf.extend_from_slice(&tmp[..n]);
        }
    }

    fn try_parse(&mut self) -> Option<Frame> {
        if self.buf.is_empty() {
            return None;
        }
        if self.buf[0] == b'{' {
            let pos = self.buf.iter().position(|&b| b == b'\n')?;
            let line = self.buf[..pos].to_vec();
            self.buf.drain(..=pos);
            let res: DaemonRes = serde_json::from_slice(&line).ok()?;
            return Some(Frame::Response(res));
        }
        if self.buf.len() < 4 {
            return None;
        }
        let len = u32::from_be_bytes([self.buf[0], self.buf[1], self.buf[2], self.buf[3]]) as usize;
        if self.buf.len() < 4 + len {
            return None;
        }
        let frame: wire::WireFrame = bincode::deserialize(&self.buf[4..4 + len]).ok()?;
        self.buf.drain(..4 + len);
        frame.events.into_iter().next().map(Frame::Event)
    }

    async fn send_ok(&mut self, req: &DaemonReq) -> Result<u32> {
        let res = self.send_raw(req).await?;
        match res {
            DaemonRes::Ok { version } => Ok(version),
            DaemonRes::Error { message, .. } => Err(CoreError::Daemon(message)),
            _ => Err(CoreError::Daemon(format!("unexpected response: {res:?}"))),
        }
    }

    // ─── Playback ───

    pub async fn play(&mut self, path: &str, start_pos: f64) -> Result<u32> {
        self.send_ok(&DaemonReq::Play {
            path: path.into(),
            start_pos,
        })
        .await
    }

    pub async fn play_pause(&mut self) -> Result<u32> {
        self.send_ok(&DaemonReq::PlayPause).await
    }

    pub async fn pause(&mut self) -> Result<u32> {
        self.send_ok(&DaemonReq::Pause).await
    }

    pub async fn stop(&mut self) -> Result<u32> {
        self.send_ok(&DaemonReq::Stop).await
    }

    pub async fn next(&mut self) -> Result<u32> {
        self.send_ok(&DaemonReq::Next).await
    }

    pub async fn prev(&mut self) -> Result<u32> {
        self.send_ok(&DaemonReq::Prev).await
    }

    pub async fn seek(&mut self, position_secs: f64) -> Result<u32> {
        self.send_ok(&DaemonReq::Seek { position_secs }).await
    }

    pub async fn set_volume(&mut self, volume: u8) -> Result<u32> {
        self.send_ok(&DaemonReq::SetVolume { volume }).await
    }

    pub async fn toggle_shuffle(&mut self) -> Result<u32> {
        self.send_ok(&DaemonReq::ToggleShuffle).await
    }

    pub async fn cycle_repeat(&mut self, mode: RepeatMode) -> Result<u32> {
        self.send_ok(&DaemonReq::CycleRepeat { mode }).await
    }

    pub async fn toggle_mute(&mut self) -> Result<u32> {
        self.send_ok(&DaemonReq::ToggleMute).await
    }

    pub async fn crossfade(&mut self, enabled: bool, duration_secs: u8) -> Result<u32> {
        self.send_ok(&DaemonReq::Crossfade {
            enabled,
            duration_secs,
        })
        .await
    }

    // ─── Queue ───

    pub async fn queue_list(&mut self) -> Result<DaemonRes> {
        self.send_raw(&DaemonReq::Queue {
            action: QueueAction::List,
        })
        .await
    }

    pub async fn queue_add(&mut self, path: &str, position: Option<u128>) -> Result<u32> {
        self.send_ok(&DaemonReq::Queue {
            action: QueueAction::Add {
                path: path.into(),
                position,
            },
        })
        .await
    }

    pub async fn queue_add_many(&mut self, paths: Vec<String>) -> Result<u32> {
        self.send_ok(&DaemonReq::Queue {
            action: QueueAction::AddMany { paths },
        })
        .await
    }

    pub async fn queue_add_folder(&mut self, path: &str) -> Result<u32> {
        self.send_ok(&DaemonReq::Queue {
            action: QueueAction::AddFolder { path: path.into() },
        })
        .await
    }

    pub async fn queue_clear(&mut self) -> Result<u32> {
        self.send_ok(&DaemonReq::Queue {
            action: QueueAction::Clear,
        })
        .await
    }

    pub async fn queue_remove(&mut self, index: u128) -> Result<u32> {
        self.send_ok(&DaemonReq::Queue {
            action: QueueAction::Remove { index },
        })
        .await
    }

    pub async fn queue_move(&mut self, from: u128, to: u128) -> Result<u32> {
        self.send_ok(&DaemonReq::Queue {
            action: QueueAction::Move { from, to },
        })
        .await
    }

    pub async fn queue_set(&mut self, paths: Vec<String>, start_idx: u128) -> Result<u32> {
        self.send_ok(&DaemonReq::Queue {
            action: QueueAction::Set { paths, start_idx },
        })
        .await
    }

    // ─── Library ───

    pub async fn library_scan(&mut self, path: &str) -> Result<u32> {
        self.send_ok(&DaemonReq::Library {
            action: LibraryAction::Scan { path: path.into() },
        })
        .await
    }

    pub async fn library_get_tracks(
        &mut self,
        filter: Option<String>,
        sort: Option<String>,
    ) -> Result<DaemonRes> {
        self.send_raw(&DaemonReq::Library {
            action: LibraryAction::GetTracks { filter, sort },
        })
        .await
    }

    pub async fn library_get_playlists(&mut self) -> Result<DaemonRes> {
        self.send_raw(&DaemonReq::Library {
            action: LibraryAction::GetPlaylists,
        })
        .await
    }

    pub async fn library_create_playlist(&mut self, name: &str) -> Result<u32> {
        self.send_ok(&DaemonReq::Library {
            action: LibraryAction::CreatePlaylist { name: name.into() },
        })
        .await
    }

    pub async fn library_delete_playlist(&mut self, id: i64) -> Result<u32> {
        self.send_ok(&DaemonReq::Library {
            action: LibraryAction::DeletePlaylist { id },
        })
        .await
    }

    pub async fn library_add_to_playlist(
        &mut self,
        playlist_id: i64,
        track_ids: Vec<i64>,
    ) -> Result<u32> {
        self.send_ok(&DaemonReq::Library {
            action: LibraryAction::AddToPlaylist {
                playlist_id,
                track_ids,
            },
        })
        .await
    }

    pub async fn library_import_m3u(&mut self, path: &str) -> Result<u32> {
        self.send_ok(&DaemonReq::Library {
            action: LibraryAction::ImportM3u { path: path.into() },
        })
        .await
    }

    pub async fn library_get_recent(&mut self, count: u128) -> Result<DaemonRes> {
        self.send_raw(&DaemonReq::Library {
            action: LibraryAction::GetRecent { count },
        })
        .await
    }

    // ─── Search / Favourites ───

    pub async fn search(&mut self, query: &str) -> Result<DaemonRes> {
        self.send_raw(&DaemonReq::Search {
            query: query.into(),
        })
        .await
    }

    pub async fn get_favourites(&mut self) -> Result<DaemonRes> {
        self.send_raw(&DaemonReq::GetFavourites).await
    }

    pub async fn add_favourite(&mut self, track_id: i64) -> Result<u32> {
        self.send_ok(&DaemonReq::AddFavourite { track_id }).await
    }

    pub async fn remove_favourite(&mut self, track_id: i64) -> Result<u32> {
        self.send_ok(&DaemonReq::RemoveFavourite { track_id }).await
    }

    // ─── YouTube ───

    pub async fn yt_search(&mut self, query: &str, filter: Option<YTFilter>) -> Result<DaemonRes> {
        self.send_raw(&DaemonReq::YtSearch {
            query: query.into(),
            filter,
        })
        .await
    }

    pub async fn yt_search_poll(&mut self) -> Result<DaemonRes> {
        self.send_raw(&DaemonReq::YtSearchPoll).await
    }

    pub async fn yt_search_cancel(&mut self) -> Result<u32> {
        self.send_ok(&DaemonReq::YtSearchCancel).await
    }

    pub async fn yt_resolve_stream(&mut self, url: &str) -> Result<DaemonRes> {
        self.send_raw(&DaemonReq::YtResolveStream { url: url.into() })
            .await
    }

    // ─── System ───

    pub async fn get_status(&mut self) -> Result<DaemonState> {
        let res = self.send_raw(&DaemonReq::GetStatus).await?;
        match res {
            DaemonRes::Status { state, .. } => Ok(*state),
            DaemonRes::Error { message, .. } => Err(CoreError::Daemon(message)),
            _ => Err(CoreError::Daemon(format!("unexpected response: {res:?}"))),
        }
    }

    pub async fn ping(&mut self) -> Result<()> {
        let res = self.send_raw(&DaemonReq::Ping).await?;
        match res {
            DaemonRes::Pong => Ok(()),
            DaemonRes::Error { message, .. } => Err(CoreError::Daemon(message)),
            _ => Err(CoreError::Daemon(format!("unexpected response: {res:?}"))),
        }
    }

    pub async fn quit(&mut self) -> Result<u32> {
        self.send_ok(&DaemonReq::Quit).await
    }
}
