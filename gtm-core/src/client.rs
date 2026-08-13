// Copyright (c) 2025 - present
// Author: prjctimg <prjctimg@outlook.com>
// IPC client: async daemon communication over Unix sockets
//
// This is free software released under the GPL-3.0 license.

use std::collections::HashMap;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::UnixStream;
use tokio::sync::{mpsc, oneshot, Mutex};

use crate::ipc::{DaemonEvent, DaemonReq, DaemonRes, LibraryAction, QueueAction, WireEvent, WireReq, WireRes, PROTOCOL_VERSION};
use crate::state::{self, DaemonState, EqPreset, PlaybackStatus, RepeatMode, ReverbConfig, SavedState, YTFilter};
use crate::track;
use crate::wire;
use crate::CoreError;
use crate::Result;

// Socket location constants for GTM Protocol v1
const DEFAULT_RUNTIME_DIR: &str = "XDG_RUNTIME_DIR";
const DEFAULT_TMP_DIR: &str = "TMPDIR";
const DEFAULT_HOME: &str = "HOME";

fn get_socket_path() -> Result<std::path::PathBuf> {
    let home = std::env::var(DEFAULT_HOME).unwrap_or_else(|_| "/tmp".into());

    // Try XDG_RUNTIME_DIR first
    if let Ok(runtime) = std::env::var(DEFAULT_RUNTIME_DIR) {
        let path = std::path::PathBuf::from(runtime).join("gtm");
        let _ = std::fs::create_dir_all(&path);
        return Ok(path);
    }

    // Fallbacks will be set up by caller
    Ok(std::path::PathBuf::from("/tmp"))
}

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

                    let heartbeat_at = Arc::new(std::sync::Mutex::new(Instant::now()));
                    let hb_pulse = heartbeat_at.clone();

                    let worker = IpcWorker {
                        reader,
                        writer,
                        cmd_rx,
                        events: events.clone(),
                        connected: connected.clone(),
                        buf: Vec::with_capacity(4096),
                        socket_path: path.clone(),
                        last_event_time: Instant::now(),
                        last_heartbeat_at: heartbeat_at,
                        consecutive_failures: 0,
                        pending: HashMap::new(),
                        next_id: 0,
                        handshake_sent: false,
                        authenticated: Arc::new(AtomicBool::new(false)),
                    };
                    // Spawn worker before constructing the client handle so
                    // we can issue the mandatory handshake as id=0 here.
                    let client = Self {
                        cmd_tx: cmd_tx.clone(),
                        events: events.clone(),
                        connected: connected.clone(),
                        base_pos: Arc::new(Mutex::new(0.0)),
                        base_time: Arc::new(Mutex::new(None)),
                        is_playing: Arc::new(AtomicBool::new(false)),
                    };
                    tokio::spawn(worker.run());

                    // protocol.md "Handshake": first message after connect.
                    // Worker assigns id=0 to the first request queued, so the
                    // handshake naturally gets id=0 as required.
                    let hres = client.send_raw(DaemonReq::Handshake {
                        version: PROTOCOL_VERSION,
                        client: "gtm-rs".to_string(),
                        client_version: Some(env!("CARGO_PKG_VERSION").to_string()),
                    }).await?;
                    match hres {
                        DaemonRes::Handshake { version, daemon, daemon_version } => {
                            if version > PROTOCOL_VERSION {
                                return Err(CoreError::Daemon(format!(
                                    "daemon {daemon} {daemon_version} speaks protocol v{version} \
                                     which is newer than client v{PROTOCOL_VERSION}"
                                )));
                            }
                            connected.store(true, Ordering::Release);
                        }
                        DaemonRes::Error { message, .. } => {
                            return Err(CoreError::Daemon(format!("handshake rejected: {message}")));
                        }
                        other => {
                            return Err(CoreError::Daemon(format!(
                                "unexpected handshake response: {other:?}"
                            )));
                        }
                    }

                    // Connect to pulse socket for dedicated event stream
                    let pulse_path = {
                        let mut p = path.clone();
                        p.set_extension("pulse");
                        p
                    };
                    let events_pulse = events.clone();
                    tokio::spawn(async move {
                        pulse_reader(&pulse_path, events_pulse, hb_pulse).await;
                    });

                    return Ok(client);
                }
                Err(e) => {
                    last_err = Some(e);
                    // Exponential backoff: 50ms, 100ms, 150ms, ... up to 500ms
                    let delay = (50 * (i + 1)).min(500);
                    tokio::time::sleep(Duration::from_millis(delay)).await;
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
    pub async fn seed_clock_from_state(&self, state: &DaemonState) {
        let is_playing = state.status == PlaybackStatus::Playing;
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
        tokio::time::timeout(Duration::from_secs(5), rx)
            .await
            .map_err(|_| CoreError::Daemon("IPC response timeout".into()))?
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

    pub async fn set_crossfade_easing(&self, easing: state::Easing) -> Result<u32> {
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
        &self,
        track_id: i64,
        title: Option<String>,
        artist: Option<String>,
        album: Option<String>,
        genre: Option<String>,
        year: Option<i32>,
        track_number: Option<i32>,
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

    pub async fn get_lyrics(&self, track_id: i64) -> Result<Option<track::LrcData>> {
        let res = self.send_raw(DaemonReq::GetLyrics { track_id }).await?;
        match res {
            DaemonRes::Lyrics { lyrics, .. } => Ok(lyrics),
            DaemonRes::Error { message, .. } => Err(CoreError::Daemon(message)),
            _ => Err(CoreError::Daemon(format!("unexpected response: {res:?}"))),
        }
    }

    pub async fn check_health(&self) -> Result<crate::ipc::HealthReport> {
        let res = self.send_raw(DaemonReq::CheckHealth).await?;
        match res {
            DaemonRes::HealthReport { report, .. } => Ok(*report),
            DaemonRes::Error { message, .. } => Err(CoreError::Daemon(message)),
            _ => Err(CoreError::Daemon(format!("unexpected response: {res:?}"))),
        }
    }
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
    last_heartbeat_at: Arc<std::sync::Mutex<Instant>>,
    consecutive_failures: u32,
    pending: HashMap<u64, (String, oneshot::Sender<Result<DaemonRes>>)>,
    next_id: u64,
    handshake_sent: bool,
    authenticated: Arc<AtomicBool>,
}

const MAX_CONSECUTIVE_FAILURES: u32 = 5;
const HEARTBEAT_TIMEOUT_SECS: u64 = 30;
const HANDSHAKE_TIMEOUT_SECS: u64 = 10;

impl IpcWorker {
    async fn run(mut self) {
        let mut tmp = [0u8; 4096];
        loop {
            // Heartbeat check: if no heartbeat received within timeout,
            // the daemon or connection is stale — force reconnect immediately.
            if self.last_heartbeat_at.lock().unwrap().elapsed() > Duration::from_secs(HEARTBEAT_TIMEOUT_SECS) {
                crate::log::log(&format!(
                    "IPC worker: no heartbeat for {}s, forcing reconnect",
                    HEARTBEAT_TIMEOUT_SECS,
                ));
                self.fail_all_pending("heartbeat timeout");
                self.reconnect().await;
                *self.last_heartbeat_at.lock().unwrap() = Instant::now();
                self.last_event_time = Instant::now();
                continue;
            }

            // Health check: only force reconnect after MAX_CONSECUTIVE_FAILURES
            // timeouts, to tolerate brief daemon stalls during prev/next.
            if self.last_event_time.elapsed() > Duration::from_secs(30) {
                self.consecutive_failures += 1;
                if self.consecutive_failures >= MAX_CONSECUTIVE_FAILURES {
                    crate::log::log(&format!(
                        "IPC worker: no events for 30s ({} consecutive), forcing reconnect",
                        self.consecutive_failures
                    ));
                    self.fail_all_pending("daemon not responding");
                    self.reconnect().await;
                    self.consecutive_failures = 0;
                } else {
                    crate::log::log(&format!(
                        "IPC worker: no events for 30s ({}/{} failures), waiting",
                        self.consecutive_failures, MAX_CONSECUTIVE_FAILURES
                    ));
                }
                self.last_event_time = Instant::now();
                continue;
            }

            // Drain ALL pending requests from the channel and send them
            // without waiting for individual responses. This is the key fix:
            // previously we blocked on read_response() after each send,
            // causing commands to queue up for 15 seconds.
            let mut sent_any = false;
            while let Ok(pending) = self.cmd_rx.try_recv() {
                let id = self.next_id;
                self.next_id = self.next_id.wrapping_add(1);
                if let Err(e) = self.send_request_by_id(id, &pending).await {
                    crate::log::log(&format!("IPC worker send error: {e}"));
                    if let Some(tx) = pending.response_tx {
                        let _ = tx.send(Err(CoreError::Daemon("send failed".into())));
                    }
                    self.fail_all_pending("send failed");
                    self.reconnect().await;
                    break;
                }
                if let Some(tx) = pending.response_tx {
                    let cmd = pending.req.cmd_name().to_string();
                    self.pending.insert(id, (cmd, tx));
                }
                sent_any = true;
            }
            if sent_any && !self.pending.is_empty() {
                if let Err(e) = tokio::time::timeout(
                    Duration::from_secs(5),
                    self.writer.flush(),
                ).await {
                    crate::log::log(&format!("IPC worker flush error: {e}"));
                    self.fail_all_pending("flush failed");
                    self.reconnect().await;
                    continue;
                }
            }

            // Read from socket with a small timeout so we can loop back
            // to check for requests.
            match self.read_with_timeout(&mut tmp).await {
                Ok(true) => {
                    self.last_event_time = Instant::now();
                    self.consecutive_failures = 0;
                    // Parse all complete frames, dispatching responses by ID
                    while self.parse_next().await {}
                }
                Ok(false) => {} // timeout, loop back to check for requests
                Err(e) => {
                    crate::log::log(&format!("IPC worker read error: {e}"));
                    self.fail_all_pending("read error");
                    self.reconnect().await;
                    continue;
                }
            }
        }
    }

    fn fail_all_pending(&mut self, reason: &str) {
        for (_, (_, tx)) in self.pending.drain() {
            let _ = tx.send(Err(CoreError::Daemon(reason.into())));
        }
    }

    async fn reconnect(&mut self) {
        self.connected.store(false, Ordering::Release);
        let mut attempt = 0u32;
        loop {
            let delay_ms = (100u64 * 2u64.saturating_pow(attempt.min(10))).min(10_000);
            tokio::time::sleep(Duration::from_millis(delay_ms)).await;
            match tokio::net::UnixStream::connect(&self.socket_path).await {
                Ok(stream) => {
                    let (reader, writer) = stream.into_split();
                    self.reader = reader;
                    self.writer = writer;
                    self.buf.clear();
                    self.connected.store(true, Ordering::Release);
                    crate::log::log(&format!(
                        "IPC worker reconnected after {attempt} attempts"
                    ));
                    // Reset handshake tracking after reconnect
                    self.handshake_sent = false;
                    self.authenticated.store(false, Ordering::Release);
                    return;
                }
                Err(e) => {
                    attempt += 1;
                    if attempt % 10 == 0 {
                        crate::log::log(&format!(
                            "IPC worker reconnect attempt {attempt} failed: {e}"
                        ));
                    }
                }
            }
        }
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

    async fn send_request_by_id(&mut self, id: u64, pending: &PendingRequest) -> Result<()> {
        let mut line = serde_json::to_string(&WireReq {
            id,
            cmd: pending.req.cmd_name(),
        })?;
        line.push('\n');
        self.writer.write_all(line.as_bytes()).await?;
        self.writer.flush().await?;
        Ok(())
    }
        Ok(())
    }

    async fn parse_next(&mut self) -> bool {
        if self.buf.is_empty() {
            return false;
        }
        let pos = match self.buf.iter().position(|&b| b == b'\n') {
            Some(p) => p,
            None => return false,
        };
        let line = self.buf[..pos].to_vec();
        self.buf.drain(..=pos);
        if let Ok(wire_res) = serde_json::from_slice::<WireRes>(&line) {
            if let Some((cmd, tx)) = self.pending.remove(&wire_res.id) {
                // protocol.md: responses do not echo `cmd`; reconstruct the
                // typed DaemonRes from the original request's cmd string so
                // callers can match on typed variants.
                let response = DaemonRes::from_wire(&cmd, &wire_res);
                let _ = tx.send(Ok(response));
            }
            return true;
        }
        if let Ok(wire_event) = serde_json::from_slice::<WireEvent>(&line) {
            let event = deserialize_daemon_event(&wire_event.event, wire_event.data);
            if let Some(event) = event {
                if matches!(event, DaemonEvent::Heartbeat) {
                    *self.last_heartbeat_at.lock().unwrap() = Instant::now();
                }
                let mut events = self.events.lock().await;
                events.push(event);
            }
        }
        true
    }
}

fn deserialize_daemon_event(tag: &str, data: serde_json::Value) -> Option<DaemonEvent> {
    let mut obj = match data {
        serde_json::Value::Object(o) => o,
        _ => return None,
    };
    obj.insert("event".to_string(), serde_json::Value::String(tag.to_string()));
    serde_json::from_value(serde_json::Value::Object(obj)).ok()
}

async fn pulse_reader(
    pulse_path: &std::path::Path,
    events: Arc<Mutex<Vec<DaemonEvent>>>,
    last_heartbeat_at: Arc<std::sync::Mutex<Instant>>,
) {
    let mut buf = Vec::with_capacity(4096);
    let mut attempt = 0u32;
    loop {
        let stream = match UnixStream::connect(pulse_path).await {
            Ok(s) => s,
            Err(e) => {
                attempt += 1;
                if attempt > 30 {
                    crate::log::log(&format!(
                        "pulse: giving up after {attempt} reconnect attempts"
                    ));
                    return;
                }
                crate::log::log(&format!(
                    "pulse connect attempt {attempt} failed: {e}"
                ));
                tokio::time::sleep(Duration::from_millis((200 * attempt.min(30)) as u64)).await;
                continue;
            }
        };
        attempt = 0;
        buf.clear();
        let mut reader = stream;
        loop {
            let mut tmp = [0u8; 4096];
            let n = match reader.read(&mut tmp).await {
                Ok(0) => {
                    crate::log::log("pulse: connection closed, reconnecting");
                    break;
                }
                Ok(n) => n,
                Err(e) => {
                    crate::log::log(&format!("pulse read error: {e}, reconnecting"));
                    break;
                }
            };
            buf.extend_from_slice(&tmp[..n]);
            loop {
                let (decoded, consumed) = match wire::decode(&buf) {
                    Ok(Some((e, c))) => (e, c),
                    Ok(None) => break,
                    Err(e) => {
                        crate::log::log(&format!("pulse decode error: {e}"));
                        buf.clear();
                        break;
                    }
                };
                buf.drain(..consumed as usize);
                if decoded.iter().any(|e| matches!(e, DaemonEvent::Heartbeat)) {
                    *last_heartbeat_at.lock().unwrap() = Instant::now();
                }
                let mut evs = events.lock().await;
                evs.extend(decoded);
            }
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
}