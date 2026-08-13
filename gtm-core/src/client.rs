// Copyright (c) 2025 - present
// Author: prjctimg <prjctimg@outlook.com>
// IPC client: async daemon communication over Unix sockets
//
// This is free software released under the GPL-3.0 license.

use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::UnixStream;
use tokio::sync::{mpsc, oneshot, Mutex};

use crate::ipc::{DaemonEvent, DaemonReq, DaemonRes, LibraryAction, QueueAction};
use crate::state::{DaemonState, EqPreset, RepeatMode, YTFilter};
use crate::wire;
use crate::CoreError;
use crate::Result;

struct PendingRequest {
    req: DaemonReq,
    response_tx: Option<oneshot::Sender<Result<DaemonRes>>>,
}

#[derive(Clone)]
pub struct DaemonClient {
    cmd_tx: mpsc::UnboundedSender<PendingRequest>,
    events: Arc<Mutex<Vec<DaemonEvent>>>,
    connected: Arc<AtomicBool>,
    /// Clock-skewing state: base position and time for local position estimation.
    /// Updated on playback start/pause/stop from event stream.
    base_pos: Arc<Mutex<f64>>,
    base_time: Arc<Mutex<Option<Instant>>>,
    is_playing: Arc<AtomicBool>,
}

impl DaemonClient {
    pub async fn connect(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref().to_owned();
        let mut last_err = None;
        for i in 0..10 {
            match UnixStream::connect(&path).await {
                Ok(stream) => {
                    let (reader, writer) = stream.into_split();
                    let (cmd_tx, cmd_rx) = mpsc::unbounded_channel();
                    let events: Arc<Mutex<Vec<DaemonEvent>>> =
                        Arc::new(Mutex::new(Vec::new()));
                    let connected = Arc::new(AtomicBool::new(true));

                    let worker = IpcWorker {
                        reader,
                        writer,
                        cmd_rx,
                        events: events.clone(),
                        connected: connected.clone(),
                        buf: Vec::with_capacity(4096),
                        socket_path: path.clone(),
                        last_event_time: Instant::now(),
                        consecutive_failures: 0,
                    };
                    tokio::spawn(worker.run());

                    // Connect to pulse socket for dedicated event stream
                    let pulse_path = {
                        let mut p = path.clone();
                        p.set_extension("pulse");
                        p
                    };
                    let events_pulse = events.clone();
                    tokio::spawn(async move {
                        pulse_reader(&pulse_path, events_pulse).await;
                    });

                    return Ok(Self {
                        cmd_tx,
                        events,
                        connected,
                        base_pos: Arc::new(Mutex::new(0.0)),
                        base_time: Arc::new(Mutex::new(None)),
                        is_playing: Arc::new(AtomicBool::new(false)),
                    });
                }
                Err(e) => {
                    last_err = Some(e);
                    tokio::time::sleep(Duration::from_millis(50 * (i + 1))).await;
                }
            }
        }
        Err(CoreError::Daemon(format!(
            "connect to {} failed after 10 retries: {}",
            path.display(),
            last_err.map(|e| e.to_string()).unwrap_or_default()
        )))
    }

    pub fn is_connected(&self) -> bool {
        self.connected.load(Ordering::Acquire)
    }

    pub async fn drain(&self) -> Vec<DaemonEvent> {
        let mut events = self.events.lock().await;
        let cap = events.len().min(1000);
        let drained: Vec<DaemonEvent> = events.drain(..cap).collect();
        self.apply_clock_events(&drained).await;
        drained
    }

    async fn apply_clock_events(&self, evs: &[DaemonEvent]) {
        let mut base_pos = self.base_pos.lock().await;
        let mut base_time = self.base_time.lock().await;
        for ev in evs {
            match ev {
                DaemonEvent::PlaybackStarted { time_pos, .. } => {
                    *base_pos = *time_pos;
                    *base_time = Some(Instant::now());
                    self.is_playing.store(true, Ordering::Release);
                }
                DaemonEvent::PlaybackPaused { time_pos } => {
                    *base_pos = *time_pos;
                    *base_time = None;
                    self.is_playing.store(false, Ordering::Release);
                }
                DaemonEvent::PositionChanged { time_pos } => {
                    *base_pos = *time_pos;
                    *base_time = Some(Instant::now());
                }
                DaemonEvent::PlaybackStopped | DaemonEvent::TrackEnded => {
                    *base_pos = 0.0;
                    *base_time = None;
                    self.is_playing.store(false, Ordering::Release);
                }
                _ => {}
            }
        }
    }

    /// Compute estimated playback position using local clock skewing.
    /// Returns the position in seconds, or 0.0 if unknown.
    pub async fn estimated_position(&self) -> f64 {
        let base_pos = *self.base_pos.lock().await;
        if self.is_playing.load(Ordering::Acquire) {
            if let Some(base_time) = *self.base_time.lock().await {
                let elapsed = base_time.elapsed().as_secs_f64();
                return base_pos + elapsed;
            }
        }
        base_pos
    }

    /// Seed the clock-skewing state from a full daemon state snapshot
    /// (e.g. after `GetStatus` on reconnect).  This ensures the position
    /// estimate is correct before the first event arrives.
    pub async fn seed_clock_from_state(&self, state: &crate::state::DaemonState) {
        let is_playing = state.status == crate::state::PlaybackStatus::Playing;
        *self.base_pos.lock().await = state.time_pos;
        *self.base_time.lock().await = if is_playing { Some(Instant::now()) } else { None };
        self.is_playing.store(is_playing, Ordering::Release);
    }

    async fn send_raw(&self, req: DaemonReq) -> Result<DaemonRes> {
        let (tx, rx) = oneshot::channel();
        self.cmd_tx
            .send(PendingRequest {
                req,
                response_tx: Some(tx),
            })
            .map_err(|_| CoreError::Daemon("IPC worker died".into()))?;
        rx.await
            .map_err(|_| CoreError::Daemon("IPC worker response dropped".into()))?
    }

    async fn send_ok(&self, req: DaemonReq) -> Result<u32> {
        match self.send_raw(req).await? {
            DaemonRes::Ok { version } => Ok(version),
            DaemonRes::Error { message, .. } => Err(CoreError::Daemon(message)),
            _ => Err(CoreError::Daemon("unexpected response".into())),
        }
    }

    // ─── Playback ───

    pub async fn play(&self, path: &str, start_pos: f64) -> Result<u32> {
        self.send_ok(DaemonReq::Play {
            path: path.into(),
            start_pos,
        })
        .await
    }

    pub async fn play_pause(&self) -> Result<u32> {
        self.send_ok(DaemonReq::PlayPause).await
    }

    pub async fn pause(&self) -> Result<u32> {
        self.send_ok(DaemonReq::Pause).await
    }

    pub async fn stop(&self) -> Result<u32> {
        self.send_ok(DaemonReq::Stop).await
    }

    pub async fn next(&self) -> Result<u32> {
        self.send_ok(DaemonReq::Next).await
    }

    pub async fn prev(&self) -> Result<u32> {
        self.send_ok(DaemonReq::Prev).await
    }

    pub async fn seek(&self, position_secs: f64) -> Result<u32> {
        *self.base_pos.lock().await = position_secs;
        *self.base_time.lock().await = Some(Instant::now());
        self.send_ok(DaemonReq::Seek { position_secs })
            .await
    }

    pub async fn set_volume(&self, volume: u8) -> Result<u32> {
        self.send_ok(DaemonReq::SetVolume { volume }).await
    }

    pub async fn toggle_shuffle(&self) -> Result<u32> {
        self.send_ok(DaemonReq::ToggleShuffle).await
    }

    pub async fn cycle_repeat(&self, mode: RepeatMode) -> Result<u32> {
        self.send_ok(DaemonReq::CycleRepeat { mode }).await
    }

    pub async fn toggle_mute(&self) -> Result<u32> {
        self.send_ok(DaemonReq::ToggleMute).await
    }

    pub async fn set_eq_preset(&self, preset: EqPreset) -> Result<u32> {
        self.send_ok(DaemonReq::SetEqPreset { preset }).await
    }

    pub async fn set_eq_enabled(&self, enabled: bool) -> Result<u32> {
        self.send_ok(DaemonReq::SetEqEnabled { enabled }).await
    }

    pub async fn set_reverb(&self, enabled: bool, room_size: f32) -> Result<u32> {
        self.send_ok(DaemonReq::SetReverb { enabled, room_size }).await
    }

    pub async fn crossfade(&self, enabled: bool, duration_secs: u8) -> Result<u32> {
        self.send_ok(DaemonReq::Crossfade {
            enabled,
            duration_secs,
        })
        .await
    }

    pub async fn set_crossfade_easing(&self, easing: crate::state::Easing) -> Result<u32> {
        self.send_ok(DaemonReq::SetCrossfadeEasing { easing }).await
    }

    pub async fn set_sleep_timer(&self, minutes: u32) -> Result<u32> {
        self.send_ok(DaemonReq::SetSleepTimer { minutes }).await
    }

    pub async fn cancel_sleep_timer(&self) -> Result<u32> {
        self.send_ok(DaemonReq::CancelSleepTimer).await
    }

    // ─── Queue ───

    pub async fn queue_list(&self) -> Result<DaemonRes> {
        self.send_raw(DaemonReq::Queue {
            action: QueueAction::List,
        })
        .await
    }

    pub async fn queue_add(&self, path: &str, position: Option<u128>) -> Result<u32> {
        self.send_ok(DaemonReq::Queue {
            action: QueueAction::Add {
                path: path.into(),
                position,
            },
        })
        .await
    }

    pub async fn queue_add_many(&self, paths: Vec<String>) -> Result<u32> {
        self.send_ok(DaemonReq::Queue {
            action: QueueAction::AddMany { paths },
        })
        .await
    }

    pub async fn queue_add_dir(&self, path: &str) -> Result<u32> {
        self.send_ok(DaemonReq::Queue {
            action: QueueAction::AddFolder { path: path.into() },
        })
        .await
    }

    pub async fn queue_clear(&self) -> Result<u32> {
        self.send_ok(DaemonReq::Queue {
            action: QueueAction::Clear,
        })
        .await
    }

    pub async fn queue_rm(&self, index: u128) -> Result<u32> {
        self.send_ok(DaemonReq::Queue {
            action: QueueAction::Remove { index },
        })
        .await
    }

    pub async fn queue_move(&self, from: u128, to: u128) -> Result<u32> {
        self.send_ok(DaemonReq::Queue {
            action: QueueAction::Move { from, to },
        })
        .await
    }

    pub async fn queue_set(&self, paths: Vec<String>, start_idx: u128) -> Result<u32> {
        self.send_ok(DaemonReq::Queue {
            action: QueueAction::Set { paths, start_idx },
        })
        .await
    }

    // ─── Library ───

    pub async fn library_scan(&self, path: &str) -> Result<u32> {
        self.send_ok(DaemonReq::Library {
            action: LibraryAction::Scan { path: path.into() },
        })
        .await
    }

    pub async fn library_get_tracks(
        &self,
        filter: Option<String>,
        sort: Option<String>,
    ) -> Result<DaemonRes> {
        self.send_raw(DaemonReq::Library {
            action: LibraryAction::GetTracks { filter, sort },
        })
        .await
    }

    pub async fn library_get_playlists(&self) -> Result<DaemonRes> {
        self.send_raw(DaemonReq::Library {
            action: LibraryAction::GetPlaylists,
        })
        .await
    }

    pub async fn library_create_playlist(&self, name: &str) -> Result<u32> {
        self.send_ok(DaemonReq::Library {
            action: LibraryAction::CreatePlaylist { name: name.into() },
        })
        .await
    }

    pub async fn library_delete_playlist(&self, id: i64) -> Result<u32> {
        self.send_ok(DaemonReq::Library {
            action: LibraryAction::DeletePlaylist { id },
        })
        .await
    }

    pub async fn library_add_to_playlist(
        &self,
        playlist_id: i64,
        track_ids: Vec<i64>,
    ) -> Result<u32> {
        self.send_ok(DaemonReq::Library {
            action: LibraryAction::AddToPlaylist {
                playlist_id,
                track_ids,
            },
        })
        .await
    }

    pub async fn library_import_m3u(&self, path: &str) -> Result<u32> {
        self.send_ok(DaemonReq::Library {
            action: LibraryAction::ImportM3u { path: path.into() },
        })
        .await
    }

    pub async fn library_export_m3u(&self, playlist_id: i64, path: &str) -> Result<u32> {
        self.send_ok(DaemonReq::Library {
            action: LibraryAction::ExportM3u { playlist_id, path: path.into() },
        })
        .await
    }

    pub async fn library_get_recent(&self, count: u128) -> Result<DaemonRes> {
        self.send_raw(DaemonReq::Library {
            action: LibraryAction::GetRecent { count },
        })
        .await
    }

    pub async fn library_sync_covers(&self) -> Result<DaemonRes> {
        self.send_raw(DaemonReq::Library {
            action: LibraryAction::SyncCovers,
        })
        .await
    }

    pub async fn library_sync_lyrics(&self) -> Result<DaemonRes> {
        self.send_raw(DaemonReq::Library {
            action: LibraryAction::SyncLyrics,
        })
        .await
    }

    pub async fn library_remove_from_playlist(&self, playlist_id: i64, track_id: i64) -> Result<u32> {
        self.send_ok(DaemonReq::Library {
            action: LibraryAction::RemoveFromPlaylist { playlist_id, track_id },
        })
        .await
    }

    pub async fn library_remove_track(&self, id: i64) -> Result<u32> {
        self.send_ok(DaemonReq::Library {
            action: LibraryAction::RemoveTrack { id },
        })
        .await
    }

    pub async fn library_update_metadata(
        &self, track_id: i64,
        title: Option<String>, artist: Option<String>,
        album: Option<String>, genre: Option<String>,
        year: Option<i32>, track_number: Option<i32>,
    ) -> Result<u32> {
        self.send_ok(DaemonReq::Library {
            action: LibraryAction::UpdateMetadata {
                track_id, title, artist, album, genre, year, track_number,
            },
        })
        .await
    }

    // ─── Search / Favourites ───

    pub async fn search(&self, query: &str) -> Result<DaemonRes> {
        self.send_raw(DaemonReq::Search {
            query: query.into(),
        })
        .await
    }

    pub async fn get_favourites(&self) -> Result<DaemonRes> {
        self.send_raw(DaemonReq::GetFavourites).await
    }

    pub async fn add_favourite(&self, track_id: i64) -> Result<u32> {
        self.send_ok(DaemonReq::AddFavourite { track_id }).await
    }

    pub async fn remove_favourite(&self, track_id: i64) -> Result<u32> {
        self.send_ok(DaemonReq::RemoveFavourite { track_id }).await
    }

    // ─── YouTube ───

    pub async fn yt_search(&self, query: &str, filter: Option<YTFilter>) -> Result<DaemonRes> {
        self.send_raw(DaemonReq::YtSearch {
            query: query.into(),
            filter,
        })
        .await
    }

    pub async fn yt_search_poll(&self) -> Result<DaemonRes> {
        self.send_raw(DaemonReq::YtSearchPoll).await
    }

    pub async fn yt_search_cancel(&self) -> Result<u32> {
        self.send_ok(DaemonReq::YtSearchCancel).await
    }

    pub async fn yt_resolve_stream(&self, url: &str) -> Result<DaemonRes> {
        self.send_raw(DaemonReq::YtResolveStream { url: url.into() })
            .await
    }

    // ─── System ───

    pub async fn get_status(&self) -> Result<DaemonState> {
        let res = self.send_raw(DaemonReq::GetStatus).await?;
        match res {
            DaemonRes::Status { state, .. } => Ok(*state),
            DaemonRes::Error { message, .. } => Err(CoreError::Daemon(message)),
            _ => Err(CoreError::Daemon(format!("unexpected response: {res:?}"))),
        }
    }

    pub async fn ping(&self) -> Result<()> {
        let res = self.send_raw(DaemonReq::Ping).await?;
        match res {
            DaemonRes::Pong => Ok(()),
            DaemonRes::Error { message, .. } => Err(CoreError::Daemon(message)),
            _ => Err(CoreError::Daemon(format!("unexpected response: {res:?}"))),
        }
    }

    pub async fn quit(&self) -> Result<u32> {
        self.send_ok(DaemonReq::Quit).await
    }

    pub async fn get_cover_art(&self, track_id: i64) -> Result<Option<String>> {
        let res = self.send_raw(DaemonReq::GetCoverArt { track_id }).await?;
        match res {
            DaemonRes::CoverArt { data, .. } => Ok(data),
            DaemonRes::Error { message, .. } => Err(CoreError::Daemon(message)),
            _ => Err(CoreError::Daemon(format!("unexpected response: {res:?}"))),
        }
    }

    pub async fn get_lyrics(&self, track_id: i64) -> Result<Option<crate::track::LrcData>> {
        let res = self.send_raw(DaemonReq::GetLyrics { track_id }).await?;
        match res {
            DaemonRes::Lyrics { lyrics, .. } => Ok(lyrics),
            DaemonRes::Error { message, .. } => Err(CoreError::Daemon(message)),
            _ => Err(CoreError::Daemon(format!("unexpected response: {res:?}"))),
        }
    }
}

enum Frame {
    Response(DaemonRes),
    #[allow(dead_code)]
    Event(DaemonEvent),
}

struct IpcWorker {
    reader: tokio::net::unix::OwnedReadHalf,
    writer: tokio::net::unix::OwnedWriteHalf,
    cmd_rx: mpsc::UnboundedReceiver<PendingRequest>,
    events: Arc<Mutex<Vec<DaemonEvent>>>,
    connected: Arc<AtomicBool>,
    buf: Vec<u8>,
    socket_path: std::path::PathBuf,
    last_event_time: Instant,
    consecutive_failures: u32,
}

const MAX_CONSECUTIVE_FAILURES: u32 = 3;

impl IpcWorker {
    async fn run(mut self) {
        let mut tmp = [0u8; 4096];
        loop {
            // Health check: only force reconnect after MAX_CONSECUTIVE_FAILURES
            // timeouts, to tolerate brief daemon stalls during prev/next.
            if self.last_event_time.elapsed() > Duration::from_secs(10) {
                self.consecutive_failures += 1;
                if self.consecutive_failures >= MAX_CONSECUTIVE_FAILURES {
                    crate::log::log(&format!(
                        "IPC worker: no events for 10s ({} consecutive), forcing reconnect",
                        self.consecutive_failures
                    ));
                    self.reconnect().await;
                    self.consecutive_failures = 0;
                } else {
                    crate::log::log(&format!(
                        "IPC worker: no events for 10s ({}/{} failures), waiting",
                        self.consecutive_failures, MAX_CONSECUTIVE_FAILURES
                    ));
                }
                self.last_event_time = Instant::now();
                continue;
            }

            // Process ONE pending request (if any), then drain events so
            // events are never starved by a burst of commands.
            if let Ok(pending) = self.cmd_rx.try_recv() {
                if let Err(e) = self.send_request(pending).await {
                    crate::log::log(&format!("IPC worker send error: {e}"));
                    self.reconnect().await;
                    continue;
                }
            }

            // Read from socket with a small timeout so we can check for requests
            match self.read_with_timeout(&mut tmp).await {
                Ok(true) => {
                    self.last_event_time = Instant::now();
                    self.consecutive_failures = 0;
                    // Parse all complete frames
                    while let Some(frame) = self.parse().await {
                        match frame {
                            Frame::Response(_) => {
                                crate::log::log("IPC worker: unexpected response with no pending request");
                                self.reconnect().await;
                                continue;
                            }
                            Frame::Event(_) => {}
                        }
                    }
                }
                Ok(false) => {} // timeout, loop back to check for requests
                Err(e) => {
                    crate::log::log(&format!("IPC worker read error: {e}"));
                    self.reconnect().await;
                    continue;
                }
            }
        }
    }

    async fn reconnect(&mut self) {
        self.connected.store(false, Ordering::Release);
        for i in 0..30 {
            tokio::time::sleep(Duration::from_millis(200 * (i + 1))).await;
            match tokio::net::UnixStream::connect(&self.socket_path).await {
                Ok(stream) => {
                    let (reader, writer) = stream.into_split();
                    self.reader = reader;
                    self.writer = writer;
                    self.buf.clear();
                    self.connected.store(true, Ordering::Release);
                    crate::log::log("IPC worker reconnected");
                    return;
                }
                Err(e) => {
                    crate::log::log(&format!("IPC worker reconnect attempt {i} failed: {e}"));
                }
            }
        }
        crate::log::log("IPC worker: giving up after 30 reconnect attempts");
    }

    async fn read_with_timeout(&mut self, tmp: &mut [u8; 4096]) -> Result<bool> {
        match tokio::time::timeout(Duration::from_millis(50), self.reader.read(tmp)).await {
            Ok(Ok(n)) => {
                if n == 0 {
                    Err(CoreError::Daemon("connection closed".into()))
                } else {
                    self.buf.extend_from_slice(&tmp[..n]);
                    if self.buf.len() > 16_777_216 {
                        self.buf.clear();
                        return Err(CoreError::Daemon("buffer exceeded 16MB".into()));
                    }
                    Ok(true)
                }
            }
            Ok(Err(e)) => Err(CoreError::Daemon(format!("read error: {e}"))),
            Err(_) => Ok(false),
        }
    }

    async fn send_request(&mut self, pending: PendingRequest) -> Result<()> {
        let mut line = serde_json::to_string(&pending.req)?;
        line.push('\n');
        match tokio::time::timeout(Duration::from_secs(5), self.writer.write_all(line.as_bytes())).await {
            Ok(Ok(())) => {}
            Ok(Err(e)) => return Err(CoreError::Daemon(format!("write error: {e}"))),
            Err(_) => return Err(CoreError::Daemon("write timeout".into())),
        }
        match tokio::time::timeout(Duration::from_secs(5), self.writer.flush()).await {
            Ok(Ok(())) => {}
            Ok(Err(e)) => return Err(CoreError::Daemon(format!("flush error: {e}"))),
            Err(_) => return Err(CoreError::Daemon("flush timeout".into())),
        }

        let response = self.read_response().await?;
        if let Some(tx) = pending.response_tx {
            let _ = tx.send(Ok(response));
        }
        Ok(())
    }

    async fn read_response(&mut self) -> Result<DaemonRes> {
        loop {
            if let Some(Frame::Response(res)) = self.parse().await {
                return Ok(res);
            }
            let mut tmp = [0u8; 4096];
            match tokio::time::timeout(Duration::from_secs(15), self.reader.read(&mut tmp)).await {
                Ok(Ok(n)) => {
                    if n == 0 {
                        return Err(CoreError::Daemon("connection closed".into()));
                    }
                    self.buf.extend_from_slice(&tmp[..n]);
                }
                Ok(Err(e)) => return Err(CoreError::Daemon(format!("read error: {e}"))),
                Err(_) => return Err(CoreError::Daemon("response timeout".into())),
            }
        }
    }

    async fn parse(&mut self) -> Option<Frame> {
        if self.buf.is_empty() {
            return None;
        }
        let pos = self.buf.iter().position(|&b| b == b'\n')?;
        let line = self.buf[..pos].to_vec();
        self.buf.drain(..=pos);
        if let Ok(res) = serde_json::from_slice::<DaemonRes>(&line) {
            return Some(Frame::Response(res));
        }
        if let Ok(event) = serde_json::from_slice::<DaemonEvent>(&line) {
            let mut events = self.events.lock().await;
            events.push(event);
        }
        None
    }
}

/// Background reader task for the dedicated pulse socket.
///
/// Continuously reads bincode-encoded DaemonEvent frames from the pulse
/// socket and pushes them into the shared event queue.
async fn pulse_reader(pulse_path: &std::path::Path, events: Arc<Mutex<Vec<DaemonEvent>>>) {
    let stream = match UnixStream::connect(pulse_path).await {
        Ok(s) => s,
        Err(e) => {
            crate::log::log(&format!("pulse connect failed: {e}"));
            return;
        }
    };
    let mut reader = stream;
    let mut buf = Vec::with_capacity(4096);
    loop {
        let mut tmp = [0u8; 4096];
        let n = match reader.read(&mut tmp).await {
            Ok(0) => break,
            Ok(n) => n,
            Err(e) => {
                crate::log::log(&format!("pulse read error: {e}"));
                break;
            }
        };
        buf.extend_from_slice(&tmp[..n]);
        loop {
            let (frame, consumed) = match wire::decode(&buf) {
                Ok(Some((f, c))) => (f, c),
                Ok(None) => break,
                Err(e) => {
                    crate::log::log(&format!("pulse decode error: {e}"));
                    buf.clear();
                    break;
                }
            };
            buf.drain(..consumed as usize);
            let mut evs = events.lock().await;
            evs.extend(frame.events);
        }
    }
}
