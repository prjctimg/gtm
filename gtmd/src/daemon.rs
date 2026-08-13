// Copyright (c) 2025 - present
// Author: prjctimg <prjctimg@outlook.com>
// Daemon event loop, IPC command handlers, and audio event processing
//
// This is free software released under the GPL-3.0 license.

//! Daemon event loop, IPC command handlers, and audio event processing.
//!
//! ```text
//!  ┌─────────────────────────────────────────────────────────┐
//!  │  Daemon event loop (run)                               │
//!  │                                                         │
//!  │  tokio::select! waits on three sources:                 │
//!  │                                                         │
//!  │  ┌──────────────┐  ┌──────────────┐  ┌──────────────┐  │
//!  │  │ Unix socket  │  │ Request chan │  │ Audio mixer  │  │
//!  │  │ listener     │  │ (req_rx)     │  │ (poll)       │  │
//!  │  └──────┬───────┘  └──────┬───────┘  └──────┬───────┘  │
//!  │         │                 │                  │          │
//!  │         ▼                 ▼                  ▼          │
//!  │  accept_client()    dispatch()          handle_        │
//!  │  spawn read+write   → cmd_play,        audio_event()   │
//!  │  tasks per client      cmd_queue,      Position/Dur/   │
//!  │                        cmd_seek,       Finished/Error  │
//!  │                        cmd_stop etc.                   │
//!  └─────────────────────────────────────────────────────────┘
//! ```
//!
//! ## Per-client architecture
//!
//! Each connected client gets two spawned tokio tasks:
//!
//! ```text
//!  Client ──→ Reader task: read JSON lines, send to req_tx
//!           ──→ Writer task: recv responses + broadcast events,
//!               write JSON frames back to client
//! ```
//!
//! ## Audio event flow
//!
//! ```text
//!  AudioMixer::poll()  ──→  AudioEvent::Position(pos)
//!                        ──→  AudioEvent::Duration(dur)
//!                        ──→  AudioEvent::Finished
//!                        ──→  AudioEvent::Error(msg)
//!                                  │
//!                                  ▼
//!                           handle_audio_event()
//!                           → update state, push DaemonEvent
//!                           → crossfade trigger
//!                           → auto-advance on track end
//! ```

use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::{broadcast, mpsc, RwLock};
use tracing::{error, info, warn};

use gtm_audio::{AudioEvent, AudioMixer, AudioResult, Mixer, NullMixer};
use gtm_core::ipc::{DaemonEvent, DaemonReq, DaemonRes, QueueAction, WireReq, WireRes};
use gtm_core::state::{DaemonState, EqPreset, PlaybackStatus, ReverbConfig};
use gtm_core::wire;
use gtm_core::CoreError;

use crate::config::DaemonConfig;
use crate::cover_art::CoverCache;
use crate::library::Library;
use crate::lyrics::LyricsManager;
use crate::queue;
use crate::youtube::YoutubeManager;

type ClientId = u64;
type ReplyTx = mpsc::UnboundedSender<(u64, DaemonRes)>;

struct DaemonInner {
    state: Arc<RwLock<DaemonState>>,
    mixer: tokio::sync::Mutex<Box<dyn Mixer>>,
    config: DaemonConfig,
    event_tx: broadcast::Sender<DaemonEvent>,
    cover_cache: tokio::sync::Mutex<Option<CoverCache>>,
    lyrics_manager: Option<LyricsManager>,
    youtube: tokio::sync::Mutex<YoutubeManager>,
    crossfade_loaded_for: tokio::sync::Mutex<Option<String>>,
    sleep_cancel: Arc<AtomicBool>,
}

pub struct Daemon {
    inner: Arc<DaemonInner>,
    listener: UnixListener,
    pulse_listener: UnixListener,
    req_tx: mpsc::UnboundedSender<(ClientId, u64, DaemonReq, ReplyTx)>,
    req_rx: mpsc::UnboundedReceiver<(ClientId, u64, DaemonReq, ReplyTx)>,
    next_client_id: ClientId,
}

impl Daemon {
    pub fn new(config: DaemonConfig) -> Result<Self, CoreError> {
        let state = Arc::new(RwLock::new(DaemonState::new()));

        let mixer: Box<dyn Mixer> = if config.test_mode {
            Box::new(NullMixer::new())
        } else {
            Box::new(
                AudioMixer::new()
                    .map_err(|e| CoreError::Daemon(format!("audio mixer init: {e}")))?,
            )
        };

        let socket_path = Path::new(&config.socket_path);
        if let Some(parent) = socket_path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| CoreError::Daemon(format!("create socket dir: {e}")))?;
        }
        if socket_path.exists() {
            std::fs::remove_file(socket_path)
                .map_err(|e| CoreError::Daemon(format!("remove stale socket: {e}")))?;
        }

        let listener = UnixListener::bind(socket_path)
            .map_err(|e| CoreError::Daemon(format!("bind socket: {e}")))?;

        let pulse_path = Path::new(&config.socket_pulse_path);
        if let Some(parent) = pulse_path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| CoreError::Daemon(format!("create pulse socket dir: {e}")))?;
        }
        if pulse_path.exists() {
            std::fs::remove_file(pulse_path)
                .map_err(|e| CoreError::Daemon(format!("remove stale pulse socket: {e}")))?;
        }
        let pulse_listener = UnixListener::bind(pulse_path)
            .map_err(|e| CoreError::Daemon(format!("bind pulse socket: {e}")))?;

        let (event_tx, _) = broadcast::channel::<DaemonEvent>(1024);
        let (req_tx, req_rx) = mpsc::unbounded_channel();

        let cache_dir = config.cache_dir.clone();

        let inner = Arc::new(DaemonInner {
            state,
            mixer: tokio::sync::Mutex::new(mixer),
            config,
            event_tx,
            cover_cache: tokio::sync::Mutex::new(Some(CoverCache::new(cache_dir))),
            lyrics_manager: Some(LyricsManager::new()),
            youtube: tokio::sync::Mutex::new(YoutubeManager::new()),
            crossfade_loaded_for: tokio::sync::Mutex::new(None),
            sleep_cancel: Arc::new(AtomicBool::new(false)),
        });

        Ok(Self {
            inner,
            listener,
            pulse_listener,
            req_tx,
            req_rx,
            next_client_id: 0,
        })
    }

    /// Main daemon event loop — multiplexes three sources:
    ///
    ///   1. **new connections**  — accept_client() spawns per-client read/write tasks
    ///   2. **IPC requests**     — dispatch() → handle_request() → cmd_*()
    ///   3. **audio events**     — handle_audio_event() updates state, pushes events
    pub async fn run(&mut self) -> Result<(), CoreError> {
        info!(
            "daemon started on {} (pulse: {})",
            self.inner.config.socket_path.display(),
            self.inner.config.socket_pulse_path.display()
        );

        // Kick off background auto-scan so clients can connect immediately
        let bg_state = self.inner.state.clone();
        let bg_lib_paths = self.inner.config.library_paths.clone();
        let bg_data_dir = self.inner.config.data_dir.clone();
        let bg_cache_dir = self.inner.config.cache_dir.clone();
        let bg_req_tx = self.req_tx.clone();
        let bg_event_tx = self.inner.event_tx.clone();
        tokio::spawn(async move {
            Self::background_scan(bg_state, bg_lib_paths, bg_data_dir, bg_cache_dir, bg_req_tx, bg_event_tx).await;
        });

        let mut poll_interval = tokio::time::interval(Duration::from_millis(16));
        loop {
            tokio::select! {
                _ = poll_interval.tick() => {
                    let result = { self.inner.mixer.lock().await.poll() };
                    Self::handle_audio_event(&self.inner, result).await;
                }
                result = self.listener.accept() => {
                    match result {
                        Ok((stream, _addr)) => {
                            let client_id = self.next_client_id;
                            self.next_client_id += 1;
                            let inner = Arc::clone(&self.inner);
                            let req_tx = self.req_tx.clone();
                            tokio::spawn(async move {
                                Self::accept_client(client_id, stream, inner, req_tx).await;
                            });
                        }
                        Err(e) => {
                            error!("accept failed: {e}");
                        }
                    }
                }
                result = self.pulse_listener.accept() => {
                    match result {
                        Ok((stream, _addr)) => {
                            let inner = Arc::clone(&self.inner);
                            tokio::spawn(async move {
                                Self::accept_pulse_client(stream, &inner).await;
                            });
                        }
                        Err(e) => {
                            error!("pulse accept failed: {e}");
                        }
                    }
                }
                Some((client_id, request_id, req, reply_tx)) = self.req_rx.recv() => {
                    let inner = Arc::clone(&self.inner);
                    tokio::spawn(async move {
                        Self::dispatch(inner, client_id, request_id, req, reply_tx).await;
                    });
                }
            }
        }
    }

    /// Background scan: runs auto-scan concurrently with the event loop
    /// so clients don't block waiting for the library to be indexed.
    async fn background_scan(
        state: Arc<RwLock<DaemonState>>,
        library_paths: Vec<std::path::PathBuf>,
        data_dir: std::path::PathBuf,
        cache_dir: std::path::PathBuf,
        _req_tx: mpsc::UnboundedSender<(ClientId, u64, DaemonReq, ReplyTx)>,
        _event_tx: broadcast::Sender<DaemonEvent>,
    ) {
        if library_paths.is_empty() {
            return;
        }
        let total_tracks = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        for audio_dir in &library_paths {
            if !audio_dir.exists() {
                info!("library path {:?} does not exist — skipping", audio_dir);
                continue;
            }
            let audio_dir_str = audio_dir.to_string_lossy().to_string();
            let data_dir = data_dir.clone();
            let cache_dir_str = cache_dir.to_string_lossy().to_string();
            let total = total_tracks.clone();
            let result = tokio::task::spawn_blocking(move || {
                let lib = match Library::new(data_dir.to_str().unwrap_or("")) {
                    Ok(l) => l,
                    Err(e) => return Err(format!("Library::new: {e}")),
                };
                lib.scan_directory(&audio_dir_str, true, Some(&cache_dir_str))
                    .map_err(|e| format!("scan: {e}"))
            })
            .await;
            let tracks = match result {
                Ok(Ok(t)) => t,
                Ok(Err(e)) => {
                    warn!("auto-scan {:?} failed: {e}", audio_dir);
                    continue;
                }
                Err(e) => {
                    warn!("auto-scan task panicked for {:?}: {e}", audio_dir);
                    continue;
                }
            };
            let count = tracks.len();
            if count == 0 {
                info!("auto-scan found no new tracks in {:?}", audio_dir);
                continue;
            }
            info!("auto-scanned {} track(s) from {:?}", count, audio_dir);
            total.fetch_add(count, std::sync::atomic::Ordering::Relaxed);
            let mut s = state.write().await;
            for track in &tracks {
                s.queue.push(track.clone());
            }
            drop(s);
        }
    }

    /// Accept a new client connection and spawn two background tasks:
    ///
    ///   **Reader task** — reads JSON lines from the Unix socket,
    ///                    deserializes into DaemonReq, sends to req_tx.
    ///   **Writer task** — receives responses (via reply_rx) and broadcast
    ///                    events (via event_rx), writes JSON back to socket.
    async fn accept_client(
        client_id: ClientId,
        stream: UnixStream,
        inner: Arc<DaemonInner>,
        req_tx: mpsc::UnboundedSender<(ClientId, u64, DaemonReq, ReplyTx)>,
    ) {
        let (reader, writer) = stream.into_split();
        let event_rx = inner.event_tx.subscribe();
        let (reply_tx, mut reply_rx) = mpsc::unbounded_channel::<(u64, DaemonRes)>();

        // Reader task: JSON lines → req_tx
        let r_tx = reply_tx.clone();
        tokio::spawn(async move {
            let mut reader = BufReader::new(reader);
            let mut line = String::new();
            loop {
                line.clear();
                match reader.read_line(&mut line).await {
                    Ok(0) => break,
                    Ok(_) => {
                        let trimmed = line.trim();
                        if trimmed.is_empty() {
                            continue;
                        }
                        if trimmed.len() > 1_048_576 {
                            warn!("client {client_id}: line too long ({} bytes), disconnecting", trimmed.len());
                            break;
                        }
                        let wire_req: WireReq = match serde_json::from_str(trimmed) {
                            Ok(r) => r,
                            Err(e) => {
                                warn!("client {client_id} bad request: {e}");
                                continue;
                            }
                        };
                        if req_tx.send((client_id, wire_req.id, wire_req.req, r_tx.clone())).is_err() {
                            break;
                        }
                    }
                    Err(e) => {
                        warn!("client {client_id} read error: {e}");
                        break;
                    }
                }
            }
            info!("client {client_id} disconnected");
        });

        // Writer task: responses + broadcast events → socket
        tokio::spawn(async move {
            let mut writer = writer;
            let mut event_rx = event_rx;
            loop {
                // biased: responses take priority over events
                tokio::select! {
                    biased;
                    res = reply_rx.recv() => {
                        match res {
                            Some((id, response)) => {
                                let wire = WireRes { id, res: response };
                                let line = match serde_json::to_string(&wire) {
                                    Ok(s) => s + "\n",
                                    Err(e) => {
                                        warn!("serialize response: {e}");
                                        continue;
                                    }
                                };
                                if writer.write_all(line.as_bytes()).await.is_err()
                                    || writer.flush().await.is_err()
                                {
                                    break;
                                }
                            }
                            None => break,
                        }
                    }
                    event = event_rx.recv() => {
                        match event {
                            Ok(event) => {
                                let line = match serde_json::to_string(&event) {
                                    Ok(s) => s + "\n",
                                    Err(e) => {
                                        warn!("serialize event: {e}");
                                        continue;
                                    }
                                };
                                if writer.write_all(line.as_bytes()).await.is_err()
                                    || writer.flush().await.is_err()
                                {
                                    break;
                                }
                            }
                            Err(broadcast::error::RecvError::Lagged(n)) => {
                                warn!("event lagged by {n}");
                            }
                            Err(broadcast::error::RecvError::Closed) => break,
                        }
                    }
                }
            }
        });

        info!("client {client_id} connected");
    }

    /// Accept a dedicated pulse client connection.
    ///
    /// Pulse clients only receive binary-encoded events on a separate socket,
    /// never JSON command/response traffic. This keeps the event stream
    /// clean and avoids the heuristic first-byte sniffing.
    async fn accept_pulse_client(stream: UnixStream, inner: &DaemonInner) {
        let event_rx = inner.event_tx.subscribe();
        tokio::spawn(async move {
            let mut writer = stream;
            let mut event_rx = event_rx;
            loop {
                match event_rx.recv().await {
                    Ok(event) => {
                        let frame = match wire::encode(&[event]) {
                            Ok(f) => f,
                            Err(e) => {
                                warn!("pulse encode event: {e}");
                                continue;
                            }
                        };
                        if writer.write_all(&frame).await.is_err() {
                            break;
                        }
                    }
                    Err(broadcast::error::RecvError::Lagged(n)) => {
                        warn!("pulse client lagged by {n}");
                    }
                    Err(broadcast::error::RecvError::Closed) => break,
                }
            }
        });
    }

    async fn dispatch(inner: Arc<DaemonInner>, _client_id: ClientId, request_id: u64, req: DaemonReq, reply_tx: ReplyTx) {
        let res = match Self::handle_request(&inner, &req).await {
            Ok(res) => res,
            Err(e) => {
                warn!("command {:?} failed: {e}", req);
                DaemonRes::Error {
                    version: inner.state.read().await.version as u32,
                    message: e.to_string(),
                }
            }
        };
        let _ = reply_tx.send((request_id, res));
    }

    async fn handle_request(inner: &DaemonInner, req: &DaemonReq) -> Result<DaemonRes, CoreError> {
        match req {
            DaemonReq::Play { path, start_pos } => Self::cmd_play(inner, path, *start_pos, false).await,
            DaemonReq::PlayPause => Self::cmd_playpause(inner).await,
            DaemonReq::Pause => Self::cmd_pause(inner).await,
            DaemonReq::Stop => Self::cmd_stop(inner).await,
            DaemonReq::Next => Self::cmd_next(inner).await,
            DaemonReq::Prev => Self::cmd_prev(inner).await,
            DaemonReq::Seek { position_secs } => Self::cmd_seek(inner, *position_secs).await,
            DaemonReq::SetVolume { volume } => Self::cmd_set_volume(inner, *volume).await,
            DaemonReq::ToggleShuffle => Self::cmd_toggle_shuffle(inner).await,
            DaemonReq::CycleRepeat { mode } => Self::cmd_cycle_repeat(inner, *mode).await,
            DaemonReq::ToggleMute => Self::cmd_toggle_mute(inner).await,
            DaemonReq::Crossfade {
                enabled,
                duration_secs,
            } => Self::cmd_crossfade(inner, *enabled, *duration_secs).await,
            DaemonReq::SetCrossfadeEasing { easing } => Self::cmd_set_crossfade_easing(inner, *easing).await,
            DaemonReq::Queue { action } => Self::cmd_queue(inner, action).await,
            DaemonReq::Library { action } => Self::cmd_library(inner, action).await,
            DaemonReq::Search { query } => Self::cmd_search(inner, query).await,
            DaemonReq::GetFavourites => Self::cmd_get_favourites(inner).await,
            DaemonReq::AddFavourite { track_id } => Self::cmd_add_favourite(inner, *track_id).await,
            DaemonReq::RemoveFavourite { track_id } => Self::cmd_remove_favourite(inner, *track_id).await,
            DaemonReq::YtSearch { query, filter } => Self::cmd_yt_search(inner, query, *filter).await,
            DaemonReq::YtSearchPoll => Self::cmd_yt_search_poll(inner).await,
            DaemonReq::YtSearchCancel => Self::cmd_yt_search_cancel(inner).await,
            DaemonReq::YtResolveStream { url } => Self::cmd_yt_resolve_stream(inner, url).await,
            DaemonReq::GetStatus => Self::cmd_get_status(inner).await,
            DaemonReq::SetEqPreset { preset } => Self::cmd_set_eq_preset(inner, *preset).await,
            DaemonReq::SetEqEnabled { enabled } => Self::cmd_set_eq_enabled(inner, *enabled).await,
            DaemonReq::SetReverb { enabled, room_size } => Self::cmd_set_reverb(inner, *enabled, *room_size).await,
            DaemonReq::SetSleepTimer { minutes } => Self::cmd_set_sleep_timer(inner, *minutes).await,
            DaemonReq::CancelSleepTimer => Self::cmd_cancel_sleep_timer(inner).await,
            DaemonReq::GetCoverArt { track_id } => Self::cmd_get_cover_art(inner, *track_id).await,
            DaemonReq::GetLyrics { track_id } => Self::cmd_get_lyrics(inner, *track_id).await,
            DaemonReq::Ping => Ok(DaemonRes::Pong),
            DaemonReq::Quit => {
                info!("quit requested");
                let _ = Self::cmd_stop(inner).await;
                let _ = inner.event_tx.send(DaemonEvent::Custom {
                    name: "daemon_quitting".into(),
                    data: [].into(),
                });
                tokio::time::sleep(Duration::from_millis(50)).await;
                let _ = std::fs::remove_file(&inner.config.socket_path);
                let pulse_path = format!("{}.pulse", inner.config.socket_path.display());
                let _ = std::fs::remove_file(&pulse_path);
                std::process::exit(0);
            }
        }
    }

    fn push_event(inner: &DaemonInner, event: DaemonEvent) {
        let _ = inner.event_tx.send(event);
    }

    /// Process an audio event from the mixer backend.
    ///
    /// ```text
    ///  ┌──────────────┐
    ///  │ AudioEvent   │
    ///  ├──────────────┤
    ///  │ Position(p)  │──→ update state.time_pos
    ///  │              │──→ check crossfade trigger
    ///  │              │──→ push PositionChanged event
    ///  ├──────────────┤
    ///  │ Duration(d)  │──→ update state.duration, push DurationChanged
    ///  ├──────────────┤
    ///  │ Finished     │──→ mark Stopped, push TrackEnded
    ///  │              │──→ auto-advance → cmd_next() (unless crossfading)
    ///  ├──────────────┤
    ///  │ Error(msg)   │──→ log + push Custom event
    ///  └──────────────┘
    /// ```
    async fn handle_audio_event(inner: &DaemonInner, result: AudioResult<Option<AudioEvent>>) {
        let ev = match result {
            Ok(Some(e)) => e,
            Ok(None) => {
                return;
            }
            Err(e) => {
                warn!("backend error: {e}");
                Self::push_event(inner, DaemonEvent::Custom {
                    name: "backend_error".into(),
                    data: [("error".into(), e.to_string())].into(),
                });
                return;
            }
        };

        match ev {
            AudioEvent::Position(pos) => {
                let mut state = inner.state.write().await;
                state.time_pos = pos;
                let dur = state.duration;
                let crossfade = state.crossfade.clone();
                let cur_path = state.current_track.as_ref().map(|t| t.path.clone());
                let queue_len = state.queue.len();
                drop(state);

                if let Some(cf) = crossfade {
                    let should_crossfade = {
                        let mixer = inner.mixer.lock().await;
                        cf.enabled
                            && dur > 0.0
                            && queue_len > 0
                            && !mixer.is_crossfading()
                            && inner.crossfade_loaded_for.lock().await.is_none()
                            && (dur - pos) <= cf.duration_secs as f64 + 0.5
                    };
                    if should_crossfade {
                        let next_path = {
                            let s = inner.state.read().await;
                            if s.queue_cursor + 1 < s.queue.len() as u128 {
                                Some(s.queue[s.queue_cursor as usize + 1].path.clone())
                            } else if matches!(s.repeat, gtm_core::state::RepeatMode::All)
                                && !s.queue.is_empty()
                            {
                                Some(s.queue[0].path.clone())
                            } else {
                                None
                            }
                        };
                        if let Some(ref path) = next_path {
                            let path_owned = path.clone();
                            let decoded = tokio::task::spawn_blocking(move || {
                                AudioMixer::decode_file(&path_owned)
                            })
                            .await;
                            if let Ok(Ok(source)) = decoded {
                                let mut mixer = inner.mixer.lock().await;
                                if mixer.load_standby_decoded(source).is_ok() {
                                    mixer.set_crossfade_easing(cf.easing);
                                    mixer.start_crossfade(cf.duration_secs as f64);
                                    drop(mixer);
                                    *inner.crossfade_loaded_for.lock().await = cur_path.clone();
                                    if let Ok(mut s) = inner.state.try_write() {
                                        let _ = s.advance_queue(1);
                                        let idx = s.queue_cursor;
                                        drop(s);
                                        Self::push_event(inner, DaemonEvent::QueueIndexChanged { index: idx });
                                    }
                                }
                            }
                        }
                    }
                }
            }
            AudioEvent::Duration(dur) => {
                let mut state = inner.state.write().await;
                state.duration = dur;
                drop(state);
                Self::push_event(inner, DaemonEvent::DurationChanged { duration: dur });
            }
            AudioEvent::Finished => {
                let was_crossfading = inner.crossfade_loaded_for.lock().await.is_some();
                *inner.crossfade_loaded_for.lock().await = None;
                if was_crossfading {
                    let actual = {
                        let mixer = inner.mixer.lock().await;
                        mixer.current_position()
                    };
                    let mut state = inner.state.write().await;
                    state.status = PlaybackStatus::Playing;
                    state.time_pos = actual;
                    if state.queue_cursor < state.queue.len() as u128 {
                        state.current_track =
                            Some(state.queue[state.queue_cursor as usize].clone());
                    }
                    let track = state.current_track.clone();
                    drop(state);
                    if let Some(t) = track {
                        Self::push_event(inner, DaemonEvent::PlaybackStarted {
                            track: t.clone(),
                            auto_advanced: true,
                            time_pos: actual,
                            duration: t.duration as f64,
                        });
                    }
                } else {
                    let mut state = inner.state.write().await;
                    state.status = PlaybackStatus::Stopped;
                    state.time_pos = 0.0;
                    state.current_track = None;
                    drop(state);
                    let res = Self::cmd_next(inner).await;
                    if res.is_err() || inner.state.read().await.status == PlaybackStatus::Stopped {
                        Self::push_event(inner, DaemonEvent::TrackEnded);
                    }
                }
            }
            AudioEvent::Error(msg) => {
                warn!("audio error: {msg}");
                Self::push_event(inner, DaemonEvent::Custom {
                    name: "audio_error".into(),
                    data: [("error".into(), msg)].into(),
                });
            }
        }
    }

    // ─── Command handlers ──────────────────────────────────────────────
    //
    // Each cmd_* method implements one IPC command.  The pattern is:
    //
    //   1. Perform the action on the audio mixer
    //   2. Update the daemon state (behind RwLock)
    //   3. Push a DaemonEvent to the broadcast channel for all clients
    //   4. Return a DaemonRes to the requesting client

    /// Play a track from `path` (absolute or relative to daemon CWD),
    /// optionally starting at `start_pos` seconds.
    ///
    /// Stops any current playback first, then loads and plays the new track.
    /// Creates a minimal TrackInfo (metadata extraction is future work).
    async fn cmd_play(inner: &DaemonInner, path: &str, start_pos: f64, auto_advanced: bool) -> Result<DaemonRes, CoreError> {
        { let mut mixer = inner.mixer.lock().await; mixer.stop()?; }
        *inner.crossfade_loaded_for.lock().await = None;
        {
            let mut state = inner.state.write().await;
            if state.status != PlaybackStatus::Stopped {
                state.stop()?;
            }
        }

        let path_owned = path.to_string();
        let path_for_blocking = path_owned.clone();
        let source = tokio::task::spawn_blocking(move || {
            AudioMixer::decode_file(&path_for_blocking)
        })
        .await
        .map_err(|e| CoreError::Daemon(format!("spawn_blocking: {e}")))?
        .map_err(|e| CoreError::Daemon(format!("decode: {e}")))?;

        let dur = {
            let mut mixer = inner.mixer.lock().await;
            mixer.load_active_decoded(source, start_pos)?;
            mixer.play()?;
            mixer.duration()
        };
        let mut state = inner.state.write().await;

        let track = if !inner.config.test_mode {
            let lib = Library::new(inner.config.data_dir.to_str().unwrap_or("")).ok();
            if let Some(ref lib) = lib {
                match lib.track_by_path(&path_owned) {
                    Ok(Some(mut t)) => {
                        t.duration = dur;
                        t
                    }
                    _ => gtm_core::track::TrackInfo {
                        id: 0,
                        path: path_owned.clone(),
                        title: std::path::Path::new(&path_owned)
                            .file_stem()
                            .and_then(|s| s.to_str())
                            .unwrap_or("Unknown")
                            .to_string(),
                        artist: "Unknown Artist".to_string(),
                        album: "Unknown Album".to_string(),
                        duration: dur,
                        track_number: None,
                        genre: String::new(),
                        year: None,
                        bitrate: None,
                        samplerate: None,
                        hash: String::new(),
                        cover_path: None,
                        favourite: false,
                    },
                }
            } else {
                gtm_core::track::TrackInfo {
                    id: 0,
                    path: path_owned.clone(),
                    title: std::path::Path::new(&path_owned)
                        .file_stem()
                        .and_then(|s| s.to_str())
                        .unwrap_or("Unknown")
                        .to_string(),
                    artist: "Unknown Artist".to_string(),
                    album: "Unknown Album".to_string(),
                    duration: dur,
                    track_number: None,
                    genre: String::new(),
                    year: None,
                    bitrate: None,
                    samplerate: None,
                    hash: String::new(),
                    cover_path: None,
                    favourite: false,
                }
            }
        } else {
            gtm_core::track::TrackInfo {
                id: 0,
                path: path_owned.clone(),
                title: std::path::Path::new(&path_owned)
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or("Unknown")
                    .to_string(),
                artist: "Unknown Artist".to_string(),
                album: "Unknown Album".to_string(),
                duration: dur,
                track_number: None,
                genre: String::new(),
                year: None,
                bitrate: None,
                samplerate: None,
                hash: String::new(),
                cover_path: None,
                favourite: false,
            }
        };
        state.play(track.clone())?;
        state.time_pos = start_pos;
        state.duration = dur;
        let version = state.version as u32;
        drop(state);
        Self::push_event(inner, DaemonEvent::PlaybackStarted {
            track,
            auto_advanced,
            time_pos: start_pos,
            duration: dur,
        });
        Ok(DaemonRes::Ok { version })
    }

    /// Smart play/pause toggle:
    ///
    ///   - If mixer is playing → pause
    ///   - If paused → resume (no reload, just unpause backend)
    ///   - If stopped with a current track → play from beginning
    ///   - If stopped with no current track but queue is non-empty → play from queue cursor
    ///   - If stopped with no track and empty queue → no-op
    async fn cmd_playpause(inner: &DaemonInner) -> Result<DaemonRes, CoreError> {
        let is_playing = inner.mixer.lock().await.is_playing();
        if is_playing {
            Self::cmd_pause(inner).await
        } else {
            let state = inner.state.read().await;
            let is_paused = state.status == PlaybackStatus::Paused;
            let path = state
                .current_track
                .as_ref()
                .map(|t| t.path.clone())
                .unwrap_or_default();
            drop(state);

            if is_paused && !path.is_empty() {
                inner.mixer.lock().await.play()?;
                let mut state = inner.state.write().await;
                let track = match state.current_track.clone() {
                    Some(t) => t,
                    None => {
                        warn!("resume: current_track is None despite paused status");
                        let version = state.version as u32;
                        drop(state);
                        return Ok(DaemonRes::Error { version, message: "no current track".into() });
                    }
                };
                state.play(track.clone())?;
                let version = state.version as u32;
                let time_pos = state.time_pos;
                let duration = state.duration;
                drop(state);
                Self::push_event(inner, DaemonEvent::PlaybackStarted {
                    track,
                    auto_advanced: false,
                    time_pos,
                    duration,
                });
                Ok(DaemonRes::Ok { version })
            } else if !path.is_empty() {
                Self::cmd_play(inner, &path, 0.0, false).await
            } else {
                let state = inner.state.read().await;
                let queue = state.queue.clone();
                let cursor = state.queue_cursor as usize;
                drop(state);
                if !queue.is_empty() {
                    let idx = cursor.min(queue.len() - 1);
                    Self::cmd_play(inner, &queue[idx].path, 0.0, false).await
                } else {
                    let version = inner.state.read().await.version as u32;
                    Ok(DaemonRes::Ok { version })
                }
            }
        }
    }

    async fn cmd_pause(inner: &DaemonInner) -> Result<DaemonRes, CoreError> {
        let pos = {
            let mut mixer = inner.mixer.lock().await;
            mixer.pause()?;
            mixer.current_position()
        };
        let mut state = inner.state.write().await;
        state.pause()?;
        state.time_pos = pos;
        let version = state.version as u32;
        let time_pos = state.time_pos;
        drop(state);
        Self::push_event(inner, DaemonEvent::PlaybackPaused { time_pos });
        Ok(DaemonRes::Ok { version })
    }

    /// Stop playback: stops the mixer backend, transitions state to Stopped,
    /// and broadcasts PlaybackStopped.  Safe to call when already stopped
    /// (checks status before calling state.stop() to avoid assert).
    async fn cmd_stop(inner: &DaemonInner) -> Result<DaemonRes, CoreError> {
        { let mut mixer = inner.mixer.lock().await; mixer.stop()?; }
        *inner.crossfade_loaded_for.lock().await = None;
        let mut state = inner.state.write().await;
        if state.status != PlaybackStatus::Stopped {
            state.stop()?;
        }
        let version = state.version as u32;
        drop(state);
        Self::push_event(inner, DaemonEvent::PlaybackStopped);
        Ok(DaemonRes::Ok { version })
    }

    /// Advance to next track in the queue.  Advances cursor via
    /// state.advance_queue(1), then plays the track at the new cursor.
    /// Returns Ok if no next track (already at end).
    async fn cmd_next(inner: &DaemonInner) -> Result<DaemonRes, CoreError> {
        let mut state = inner.state.write().await;
        let track = match state.advance_queue(1)? {
            Some(t) => t.clone(),
            None => {
                let version = state.version as u32;
                return Ok(DaemonRes::Ok { version });
            }
        };
        let idx = state.queue_cursor;
        let crossfade_enabled = state.crossfade.as_ref().map(|c| c.enabled).unwrap_or(false);
        let crossfade_dur = state.crossfade.as_ref().map(|c| c.duration_secs as f64).unwrap_or(0.0);
        let crossfade_easing = state.crossfade.as_ref().map(|c| c.easing).unwrap_or(gtm_core::state::Easing::Linear);
        drop(state);
        if crossfade_enabled && crossfade_dur > 0.0 && inner.crossfade_loaded_for.lock().await.is_none() && !inner.mixer.lock().await.is_crossfading() {
            let path = track.path.clone();
            let path_owned = path.clone();
            let decoded = tokio::task::spawn_blocking(move || {
                AudioMixer::decode_file(&path_owned)
            })
            .await;
            if let Ok(Ok(source)) = decoded {
                let mut mixer = inner.mixer.lock().await;
                if mixer.load_standby_decoded(source).is_ok() {
                    mixer.set_crossfade_easing(crossfade_easing);
                    mixer.start_crossfade(crossfade_dur);
                    drop(mixer);
                    *inner.crossfade_loaded_for.lock().await = Some(
                        inner.state.read().await.current_track.as_ref().map(|t| t.path.clone())
                            .unwrap_or_default()
                    );
                    Self::push_event(inner, DaemonEvent::QueueIndexChanged { index: idx });
                    let dur = inner.mixer.lock().await.duration();
                    {
                        let mut st = inner.state.write().await;
                        st.status = PlaybackStatus::Playing;
                        st.current_track = Some(track.clone());
                        st.time_pos = 0.0;
                        st.duration = dur;
                    }
                    Self::push_event(inner, DaemonEvent::PlaybackStarted {
                        track,
                        auto_advanced: true,
                        time_pos: 0.0,
                        duration: dur,
                    });
                    return Ok(DaemonRes::Ok { version: inner.state.read().await.version as u32 });
                }
            }
        }
        *inner.crossfade_loaded_for.lock().await = None;
        let path = track.path.clone();
        let res = Self::cmd_play(inner, &path, 0.0, true).await?;
        Self::push_event(inner, DaemonEvent::QueueIndexChanged { index: idx });
        Ok(res)
    }

    /// Go to previous track.  If cursor is at 0 or queue is empty,
    /// seek to beginning of current track instead.
    async fn cmd_prev(inner: &DaemonInner) -> Result<DaemonRes, CoreError> {
        let mut state = inner.state.write().await;
        if state.queue.is_empty() || state.queue_cursor == 0 {
            drop(state);
            return Self::cmd_seek(inner, 0.0).await;
        }
        let track = match state.advance_queue(-1)? {
            Some(t) => t.clone(),
            None => {
                let version = state.version as u32;
                return Ok(DaemonRes::Ok { version });
            }
        };
        let idx = state.queue_cursor;
        let crossfade_enabled = state.crossfade.as_ref().map(|c| c.enabled).unwrap_or(false);
        let crossfade_dur = state.crossfade.as_ref().map(|c| c.duration_secs as f64).unwrap_or(0.0);
        let crossfade_easing = state.crossfade.as_ref().map(|c| c.easing).unwrap_or(gtm_core::state::Easing::Linear);
        drop(state);
        if crossfade_enabled && crossfade_dur > 0.0 && inner.crossfade_loaded_for.lock().await.is_none() && !inner.mixer.lock().await.is_crossfading() {
            let path = track.path.clone();
            let path_owned = path.clone();
            let decoded = tokio::task::spawn_blocking(move || {
                AudioMixer::decode_file(&path_owned)
            })
            .await;
            if let Ok(Ok(source)) = decoded {
                let mut mixer = inner.mixer.lock().await;
                if mixer.load_standby_decoded(source).is_ok() {
                    mixer.set_crossfade_easing(crossfade_easing);
                    mixer.start_crossfade(crossfade_dur);
                    drop(mixer);
                    *inner.crossfade_loaded_for.lock().await = Some(
                        inner.state.read().await.current_track.as_ref().map(|t| t.path.clone())
                            .unwrap_or_default()
                    );
                    Self::push_event(inner, DaemonEvent::QueueIndexChanged { index: idx });
                    let dur = inner.mixer.lock().await.duration();
                    {
                        let mut st = inner.state.write().await;
                        st.status = PlaybackStatus::Playing;
                        st.current_track = Some(track.clone());
                        st.time_pos = 0.0;
                        st.duration = dur;
                    }
                    Self::push_event(inner, DaemonEvent::PlaybackStarted {
                        track,
                        auto_advanced: true,
                        time_pos: 0.0,
                        duration: dur,
                    });
                    return Ok(DaemonRes::Ok { version: inner.state.read().await.version as u32 });
                }
            }
        }
        *inner.crossfade_loaded_for.lock().await = None;
        let path = track.path.clone();
        let res = Self::cmd_play(inner, &path, 0.0, true).await?;
        Self::push_event(inner, DaemonEvent::QueueIndexChanged { index: idx });
        Ok(res)
    }

    /// Seek to absolute position in seconds.  Errors if status is Stopped.
    /// Reports the *actual* position (mixer.current_position()) in the event,
    /// which may differ from the requested pos due to clamping.
    async fn cmd_seek(inner: &DaemonInner, pos: f64) -> Result<DaemonRes, CoreError> {
        let state = inner.state.read().await;
        if state.status == PlaybackStatus::Stopped {
            return Err(CoreError::Daemon("cannot seek while stopped".into()));
        }
        drop(state);
        let actual = {
            let mut mixer = inner.mixer.lock().await;
            mixer.seek(pos)?;
            mixer.current_position()
        };
        let mut state = inner.state.write().await;
        state.seek(actual)?;
        let version = state.version as u32;
        drop(state);
        Self::push_event(inner, DaemonEvent::PositionChanged { time_pos: actual });
        Ok(DaemonRes::Ok { version })
    }

    async fn cmd_set_volume(inner: &DaemonInner, volume: u8) -> Result<DaemonRes, CoreError> {
        inner.mixer.lock().await.set_volume(volume)?;
        let mut state = inner.state.write().await;
        state.set_volume(volume)?;
        let version = state.version as u32;
        drop(state);
        Self::push_event(inner, DaemonEvent::VolumeChanged { volume });
        Ok(DaemonRes::Ok { version })
    }

    async fn cmd_toggle_shuffle(inner: &DaemonInner) -> Result<DaemonRes, CoreError> {
        let mut state = inner.state.write().await;
        state.toggle_shuffle()?;
        let enabled = state.shuffle;
        let version = state.version as u32;
        drop(state);
        Self::push_event(inner, DaemonEvent::ShuffleChanged { enabled });
        Ok(DaemonRes::Ok { version })
    }

    async fn cmd_cycle_repeat(
        inner: &DaemonInner,
        mode: gtm_core::state::RepeatMode,
    ) -> Result<DaemonRes, CoreError> {
        let mut state = inner.state.write().await;
        state.cycle_repeat(mode)?;
        let m = state.repeat;
        let version = state.version as u32;
        drop(state);
        Self::push_event(inner, DaemonEvent::RepeatModeChanged { mode: m });
        Ok(DaemonRes::Ok { version })
    }

    async fn cmd_toggle_mute(inner: &DaemonInner) -> Result<DaemonRes, CoreError> {
        let mut state = inner.state.write().await;
        state.toggle_mute()?;
        let muted = state.mute;
        let version = state.version as u32;
        drop(state);
        let vol = if muted { 0 } else { inner.state.read().await.volume };
        inner.mixer.lock().await.set_volume(vol)?;
        Ok(DaemonRes::Ok { version })
    }

    async fn cmd_crossfade(
        inner: &DaemonInner,
        enabled: bool,
        duration_secs: u8,
    ) -> Result<DaemonRes, CoreError> {
        let mut state = inner.state.write().await;
        state.set_crossfade(enabled, duration_secs)?;
        let version = state.version as u32;
        drop(state);
        Self::push_event(inner, DaemonEvent::CrossfadeChanged {
            enabled,
            duration_secs,
        });
        Ok(DaemonRes::Ok { version })
    }

    async fn cmd_set_crossfade_easing(
        inner: &DaemonInner,
        easing: gtm_core::state::Easing,
    ) -> Result<DaemonRes, CoreError> {
        let mut state = inner.state.write().await;
        if let Some(ref mut cf) = state.crossfade {
            cf.easing = easing;
        }
        state.version += 1;
        let version = state.version as u32;
        drop(state);
        Ok(DaemonRes::Ok { version })
    }

    async fn cmd_set_eq_preset(
        inner: &DaemonInner,
        preset: EqPreset,
    ) -> Result<DaemonRes, CoreError> {
        let mut state = inner.state.write().await;
        state.eq_preset = preset;
        state.version += 1;
        let version = state.version as u32;
        drop(state);
        inner.mixer.lock().await.set_eq_preset(&preset);
        Self::push_event(inner, DaemonEvent::EqPresetChanged { preset });
        Ok(DaemonRes::Ok { version })
    }

    async fn cmd_set_eq_enabled(
        inner: &DaemonInner,
        enabled: bool,
    ) -> Result<DaemonRes, CoreError> {
        let mut state = inner.state.write().await;
        state.eq_enabled = enabled;
        state.version += 1;
        let version = state.version as u32;
        drop(state);
        inner.mixer.lock().await.set_eq_enabled(enabled);
        Self::push_event(inner, DaemonEvent::EqEnabledChanged { enabled });
        Ok(DaemonRes::Ok { version })
    }

    async fn cmd_set_reverb(
        inner: &DaemonInner,
        enabled: bool,
        room_size: f32,
    ) -> Result<DaemonRes, CoreError> {
        let mut state = inner.state.write().await;
        state.reverb = ReverbConfig { enabled, room_size };
        state.version += 1;
        let version = state.version as u32;
        drop(state);
        inner.mixer.lock().await.set_reverb(&ReverbConfig { enabled, room_size });
        Self::push_event(inner, DaemonEvent::ReverbChanged { enabled, room_size });
        Ok(DaemonRes::Ok { version })
    }

    async fn cmd_set_sleep_timer(inner: &DaemonInner, minutes: u32) -> Result<DaemonRes, CoreError> {
        let total_secs = minutes * 60;
        let event_tx = inner.event_tx.clone();
        let state = inner.state.clone();

        inner.sleep_cancel.store(true, Ordering::SeqCst);
        inner.sleep_cancel.store(false, Ordering::SeqCst);
        let cancel_flag = inner.sleep_cancel.clone();

        let mut s = state.write().await;
        s.sleep_timer = Some(total_secs);
        s.version += 1;
        let version = s.version as u32;
        drop(s);

        tokio::spawn(async move {
            for remaining in (1..=total_secs).rev() {
                if cancel_flag.load(Ordering::SeqCst) {
                    return;
                }
                {
                    let mut s = state.write().await;
                    s.sleep_timer = Some(remaining);
                }
                let _ = event_tx.send(DaemonEvent::SleepTimerTick { remaining_secs: remaining });
                tokio::time::sleep(Duration::from_secs(1)).await;
            }
            {
                let mut s = state.write().await;
                s.status = PlaybackStatus::Stopped;
                s.sleep_timer = None;
                s.version += 1;
            }
            let _ = event_tx.send(DaemonEvent::PlaybackStopped);
            let _ = event_tx.send(DaemonEvent::SleepTimerExpired);
        });

        Ok(DaemonRes::Ok { version })
    }

    async fn cmd_cancel_sleep_timer(inner: &DaemonInner) -> Result<DaemonRes, CoreError> {
        inner.sleep_cancel.store(true, Ordering::SeqCst);
        let mut state = inner.state.write().await;
        state.sleep_timer = None;
        state.version += 1;
        let version = state.version as u32;
        Ok(DaemonRes::Ok { version })
    }

    /// Queue command dispatcher.
    ///
    /// ```text
    ///  QueueAction
    ///  ├── List       → return current queue + cursor
    ///  ├── Clear      → clear queue, push QueueChanged
    ///  ├── Remove(i)  → remove at index, push QueueChanged
    ///  ├── Move(f,t)  → move from→to, push QueueChanged
    ///  ├── Add(path)  → add single track, auto-play if empty
    ///  ├── AddMany    → add multiple, auto-play first
    ///  ├── AddFolder  → scan folder, add all audio files, auto-play first
    ///  └── Set        → replace entire queue
    /// ```
    async fn cmd_queue(inner: &DaemonInner, action: &QueueAction) -> Result<DaemonRes, CoreError> {
        match action {
            QueueAction::List => {
                let state = inner.state.read().await;
                let version = state.version as u32;
                let tracks = state.queue.clone();
                let cursor = state.queue_cursor;
                drop(state);
                return Ok(DaemonRes::QueueState {
                    version,
                    tracks,
                    cursor,
                });
            }
            QueueAction::Clear => {
                let mut state = inner.state.write().await;
                queue::queue_clear(&mut state);
                let version = state.version as u32;
                let queue = state.queue.clone();
                let cursor = state.queue_cursor;
                drop(state);
                Self::push_event(inner, DaemonEvent::QueueChanged { queue, cursor });
                return Ok(DaemonRes::Ok { version });
            }
            QueueAction::Remove { index } => {
                let mut state = inner.state.write().await;
                queue::queue_remove(&mut state, *index);
                let version = state.version as u32;
                let queue = state.queue.clone();
                let cursor = state.queue_cursor;
                drop(state);
                Self::push_event(inner, DaemonEvent::QueueChanged { queue, cursor });
                return Ok(DaemonRes::Ok { version });
            }
            QueueAction::Move { from, to } => {
                let mut state = inner.state.write().await;
                queue::queue_move(&mut state, *from, *to);
                let version = state.version as u32;
                let queue = state.queue.clone();
                let cursor = state.queue_cursor;
                drop(state);
                Self::push_event(inner, DaemonEvent::QueueChanged { queue, cursor });
                return Ok(DaemonRes::Ok { version });
            }
            QueueAction::Add { path, position } => {
                let was_empty;
                {
                    let mut state = inner.state.write().await;
                    was_empty = state.queue.is_empty() && state.status == PlaybackStatus::Stopped;
                    queue::queue_add(&mut state, path, *position);
                    let queue = state.queue.clone();
                    let cursor = state.queue_cursor;
                    drop(state);
                    Self::push_event(inner, DaemonEvent::QueueChanged { queue, cursor });
                    if was_empty {
                        let _ = Self::cmd_play(inner, path, 0.0, false).await;
                    }
                }
                let version = inner.state.read().await.version as u32;
                Ok(DaemonRes::Ok { version })
            }
            QueueAction::AddMany { paths } => {
                let was_empty;
                let first_path;
                {
                    let mut state = inner.state.write().await;
                    was_empty = state.queue.is_empty() && state.status == PlaybackStatus::Stopped;
                    queue::queue_add_many(&mut state, paths);
                    first_path = paths[0].clone();
                    let queue = state.queue.clone();
                    let cursor = state.queue_cursor;
                    drop(state);
                    Self::push_event(inner, DaemonEvent::QueueChanged { queue, cursor });
                    if was_empty {
                        let _ = Self::cmd_play(inner, &first_path, 0.0, false).await;
                    }
                }
                let version = inner.state.read().await.version as u32;
                Ok(DaemonRes::Ok { version })
            }
            QueueAction::AddFolder { path } => {
                let paths = queue::scan_audio_files(path);
                if paths.is_empty() {
                    return Ok(DaemonRes::Error {
                        version: inner.state.read().await.version as u32,
                        message: "no audio files found in folder".into(),
                    });
                }
                let was_empty;
                let first_path;
                {
                    let mut state = inner.state.write().await;
                    was_empty = state.queue.is_empty() && state.status == PlaybackStatus::Stopped;
                    queue::queue_add_many(&mut state, &paths);
                    first_path = paths[0].clone();
                    let queue = state.queue.clone();
                    let cursor = state.queue_cursor;
                    drop(state);
                    Self::push_event(inner, DaemonEvent::QueueChanged { queue, cursor });
                    if was_empty {
                        return Self::cmd_play(inner, &first_path, 0.0, false).await;
                    }
                }
                let version = inner.state.read().await.version as u32;
                Ok(DaemonRes::Ok { version })
            }
            QueueAction::Set { paths, start_idx } => {
                let mut state = inner.state.write().await;
                queue::queue_set(&mut state, paths, *start_idx);
                let version = state.version as u32;
                let queue = state.queue.clone();
                let cursor = state.queue_cursor;
                drop(state);
                Self::push_event(inner, DaemonEvent::QueueChanged { queue, cursor });
                Ok(DaemonRes::Ok { version })
            }
        }
    }

    async fn cmd_library(
        inner: &DaemonInner,
        action: &gtm_core::ipc::LibraryAction,
    ) -> Result<DaemonRes, CoreError> {
        let version = inner.state.read().await.version as u32;
        let res = match action {
            gtm_core::ipc::LibraryAction::Scan { path } => {
                let audio_dir = path.clone();
                let data_dir = inner.config.data_dir.clone();
                let cache_dir = inner.config.cache_dir.to_string_lossy().to_string();
                let result = tokio::task::spawn_blocking(move || {
                    let lib = Library::new(data_dir.to_str().unwrap_or(""))?;
                    lib.scan_directory(&audio_dir, true, Some(&cache_dir))
                })
                .await
                .map_err(|e| CoreError::Daemon(e.to_string()))?;
                match result {
                    Ok(tracks) => DaemonRes::Tracks { version, tracks },
                    Err(e) => DaemonRes::Error {
                        version,
                        message: e,
                    },
                }
            }
            gtm_core::ipc::LibraryAction::GetTracks { filter: _, sort: _ } => {
                let data_dir = inner.config.data_dir.clone();
                let result = tokio::task::spawn_blocking(move || {
                    let lib = Library::new(data_dir.to_str().unwrap_or(""))?;
                    lib.list_tracks()
                })
                .await
                .map_err(|e| CoreError::Daemon(e.to_string()))?;
                match result {
                    Ok(tracks) => DaemonRes::Tracks { version, tracks },
                    Err(e) => DaemonRes::Error {
                        version,
                        message: e,
                    },
                }
            }
            gtm_core::ipc::LibraryAction::GetPlaylists => {
                let data_dir = inner.config.data_dir.clone();
                let result = tokio::task::spawn_blocking(move || {
                    let lib = Library::new(data_dir.to_str().unwrap_or(""))?;
                    lib.get_playlists()
                })
                .await
                .map_err(|e| CoreError::Daemon(e.to_string()))?;
                match result {
                    Ok(playlists) => DaemonRes::Playlists { version, playlists },
                    Err(e) => DaemonRes::Error {
                        version,
                        message: e,
                    },
                }
            }
            gtm_core::ipc::LibraryAction::CreatePlaylist { name } => {
                let name = name.clone();
                let data_dir = inner.config.data_dir.clone();
                let result = tokio::task::spawn_blocking(move || {
                    let lib = Library::new(data_dir.to_str().unwrap_or(""))?;
                    lib.create_playlist(&name)
                })
                .await
                .map_err(|e| CoreError::Daemon(e.to_string()))?;
                match result {
                    Ok(playlist) => {
                        let playlists = vec![playlist];
                        DaemonRes::Playlists { version, playlists }
                    }
                    Err(e) => DaemonRes::Error {
                        version,
                        message: e,
                    },
                }
            }
            gtm_core::ipc::LibraryAction::DeletePlaylist { id } => {
                let id = *id;
                let data_dir = inner.config.data_dir.clone();
                let result = tokio::task::spawn_blocking(move || {
                    let lib = Library::new(data_dir.to_str().unwrap_or(""))?;
                    lib.delete_playlist(id)
                })
                .await
                .map_err(|e| CoreError::Daemon(e.to_string()))?;
                match result {
                    Ok(_) => DaemonRes::Ok { version },
                    Err(e) => DaemonRes::Error {
                        version,
                        message: e,
                    },
                }
            }
            gtm_core::ipc::LibraryAction::AddToPlaylist {
                playlist_id,
                track_ids,
            } => {
                let playlist_id = *playlist_id;
                let track_ids = track_ids.clone();
                let data_dir = inner.config.data_dir.clone();
                let result = tokio::task::spawn_blocking(move || {
                    let lib = Library::new(data_dir.to_str().unwrap_or(""))?;
                    for tid in &track_ids {
                        lib.add_to_playlist(playlist_id, *tid)?;
                    }
                    Ok::<_, String>(())
                })
                .await
                .map_err(|e| CoreError::Daemon(e.to_string()))?;
                match result {
                    Ok(_) => DaemonRes::Ok { version },
                    Err(e) => DaemonRes::Error {
                        version,
                        message: e,
                    },
                }
            }
            gtm_core::ipc::LibraryAction::ImportM3u { path } => {
                let path = path.clone();
                let data_dir = inner.config.data_dir.clone();
                let result = tokio::task::spawn_blocking(move || {
                    let lib = Library::new(data_dir.to_str().unwrap_or(""))?;
                    lib.import_m3u(&path)
                })
                .await
                .map_err(|e| CoreError::Daemon(e.to_string()))?;
                match result {
                    Ok(playlist) => {
                        let playlists = vec![playlist];
                        DaemonRes::Playlists { version, playlists }
                    }
                    Err(e) => DaemonRes::Error {
                        version,
                        message: e,
                    },
                }
            }
            gtm_core::ipc::LibraryAction::GetRecent { count } => {
                let count = *count;
                let data_dir = inner.config.data_dir.clone();
                let result = tokio::task::spawn_blocking(move || {
                    let lib = Library::new(data_dir.to_str().unwrap_or(""))?;
                    lib.get_recent(count)
                })
                .await
                .map_err(|e| CoreError::Daemon(e.to_string()))?;
                match result {
                    Ok(tracks) => DaemonRes::Tracks { version, tracks },
                    Err(e) => DaemonRes::Error {
                        version,
                        message: e,
                    },
                }
            }
            gtm_core::ipc::LibraryAction::SyncCovers => {
                let data_dir = inner.config.data_dir.clone();
                let cache_dir = inner.config.cache_dir.clone();
                // Spawn blocking so the daemon event loop isn't starved.
                // Uses fresh Library + CoverCache to avoid connection issues.
                let result = tokio::task::spawn_blocking(move || {
                    let lib = Library::new(data_dir.to_str().unwrap_or(""))
                        .map_err(|e| format!("open library: {e}"))?;
                    let tracks = lib.list_tracks()
                        .map_err(|e| format!("list tracks: {e}"))?;
                    let rt = tokio::runtime::Runtime::new()
                        .map_err(|e| format!("runtime: {e}"))?;
                    let mut cache = crate::cover_art::CoverCache::new(cache_dir.clone());
                    let mut synced = 0usize;
                    for track in &tracks {
                        let missing_cover = track.cover_path.is_none()
                            || track.cover_path.as_ref().map_or(true, |p| !std::path::Path::new(p).exists());
                        if !missing_cover {
                            continue;
                        }
                        let artist = if track.artist.is_empty() { "Unknown Artist" } else { &track.artist };
                        let album = if track.album.is_empty() { "Unknown Album" } else { &track.album };
                        if rt.block_on(cache.get_cover(artist, album)).is_some() {
                            let key = crate::cover_art::CoverCache::cache_key(artist, album);
                            let cover_file = cache_dir.join("covers").join(format!("{key}.jpg"));
                            if cover_file.exists() {
                                let path_str = cover_file.to_string_lossy().to_string();
                                let _ = lib.update_cover_path(track.id, &path_str);
                            }
                            synced += 1;
                        }
                        std::thread::sleep(std::time::Duration::from_millis(50));
                    }
                    Ok::<(usize, usize), String>((synced, tracks.len()))
                })
                .await
                .map_err(|e| CoreError::Daemon(e.to_string()))?;
                match result {
                    Ok((synced, total)) => DaemonRes::SyncCoversResult { version, synced, total },
                    Err(e) => DaemonRes::Error { version, message: e },
                }
            }
            gtm_core::ipc::LibraryAction::SyncLyrics => {
                let lyrics_manager = inner.lyrics_manager.clone();
                let data_dir = inner.config.data_dir.clone();
                let result = tokio::task::spawn_blocking(move || {
                    let lib = Library::new(data_dir.to_str().unwrap_or(""))
                        .map_err(|e| format!("open library: {e}"))?;
                    let tracks = lib.list_tracks()
                        .map_err(|e| format!("list tracks: {e}"))?;
                    let rt = tokio::runtime::Runtime::new()
                        .map_err(|e| format!("runtime: {e}"))?;
                    let manager = lyrics_manager.ok_or("lyrics manager not available")?;
                    let mut synced = 0usize;
                    let total = tracks.len();
                    for track in &tracks {
                        let lrc_path = std::path::Path::new(&track.path).with_extension("lrc");
                        if lrc_path.exists() {
                            continue;
                        }
                        if let Some(lyrics) = rt.block_on(manager.get_lyrics(track)) {
                            if !lyrics.lines.is_empty() {
                                let mut lrc_content = String::new();
                                if let Some(ref ar) = lyrics.artist { lrc_content.push_str(&format!("[ar:{}]\n", ar)); }
                                if let Some(ref al) = lyrics.album { lrc_content.push_str(&format!("[al:{}]\n", al)); }
                                if let Some(ref ti) = lyrics.title { lrc_content.push_str(&format!("[ti:{}]\n", ti)); }
                                for line in &lyrics.lines {
                                    let mins = (line.timestamp / 60.0) as u64;
                                    let secs = line.timestamp - (mins as f64 * 60.0);
                                    lrc_content.push_str(&format!("[{:02}:{:05.2}]{}\n", mins, secs, line.text));
                                }
                                if std::fs::write(&lrc_path, &lrc_content).is_ok() {
                                    synced += 1;
                                }
                            }
                        }
                        std::thread::sleep(std::time::Duration::from_millis(100));
                    }
                    Ok::<(usize, usize), String>((synced, total))
                })
                .await
                .map_err(|e| CoreError::Daemon(e.to_string()))?;
                match result {
                    Ok((synced, total)) => DaemonRes::SyncLyricsResult { version, synced, total },
                    Err(e) => DaemonRes::Error { version, message: e },
                }
            }
            gtm_core::ipc::LibraryAction::ExportM3u { playlist_id, path } => {
                let playlist_id = *playlist_id;
                let export_path = path.clone();
                let data_dir = inner.config.data_dir.clone();
                let result = tokio::task::spawn_blocking(move || {
                    let lib = Library::new(data_dir.to_str().unwrap_or(""))?;
                    lib.export_m3u(playlist_id, &export_path)
                })
                .await
                .map_err(|e| CoreError::Daemon(e.to_string()))?;
                match result {
                    Ok(_) => DaemonRes::Ok { version },
                    Err(e) => DaemonRes::Error { version, message: e },
                }
            }
            gtm_core::ipc::LibraryAction::RemoveFromPlaylist { playlist_id, track_id } => {
                let playlist_id = *playlist_id;
                let track_id = *track_id;
                let data_dir = inner.config.data_dir.clone();
                let result = tokio::task::spawn_blocking(move || {
                    let lib = Library::new(data_dir.to_str().unwrap_or(""))?;
                    lib.remove_from_playlist(playlist_id, track_id)
                })
                .await
                .map_err(|e| CoreError::Daemon(e.to_string()))?;
                match result {
                    Ok(()) => DaemonRes::Ok { version },
                    Err(e) => DaemonRes::Error { version, message: e },
                }
            }
            gtm_core::ipc::LibraryAction::RemoveTrack { id } => {
                let id = *id;
                let data_dir = inner.config.data_dir.clone();
                let result = tokio::task::spawn_blocking(move || {
                    let lib = Library::new(data_dir.to_str().unwrap_or(""))?;
                    lib.remove_track(id)
                })
                .await
                .map_err(|e| CoreError::Daemon(e.to_string()))?;
                match result {
                    Ok(()) => DaemonRes::Ok { version },
                    Err(e) => DaemonRes::Error { version, message: e },
                }
            }
            gtm_core::ipc::LibraryAction::UpdateMetadata { track_id, title, artist, album, genre, year, track_number } => {
                let track_id = *track_id;
                let t = title.clone();
                let a = artist.clone();
                let al = album.clone();
                let g = genre.clone();
                let y = *year;
                let tn = *track_number;
                let data_dir = inner.config.data_dir.clone();
                let result = tokio::task::spawn_blocking(move || {
                    let lib = Library::new(data_dir.to_str().unwrap_or(""))?;
                    lib.update_metadata(track_id, t.as_deref(), a.as_deref(), al.as_deref(), g.as_deref(), y, tn)
                })
                .await
                .map_err(|e| CoreError::Daemon(e.to_string()))?;
                match result {
                    Ok(_) => DaemonRes::Ok { version },
                    Err(e) => DaemonRes::Error { version, message: e },
                }
            }
        };
        Ok(res)
    }

    async fn cmd_search(inner: &DaemonInner, query: &str) -> Result<DaemonRes, CoreError> {
        let version = inner.state.read().await.version as u32;
        let query = query.to_string();
        let data_dir = inner.config.data_dir.clone();
        let result = tokio::task::spawn_blocking(move || {
            let lib = Library::new(data_dir.to_str().unwrap_or(""))?;
            lib.search_tracks(&query)
        })
        .await
        .map_err(|e| CoreError::Daemon(e.to_string()))?;
        match result {
            Ok(tracks) => Ok(DaemonRes::Tracks { version, tracks }),
            Err(e) => Ok(DaemonRes::Error {
                version,
                message: e,
            }),
        }
    }

    async fn cmd_get_favourites(inner: &DaemonInner) -> Result<DaemonRes, CoreError> {
        let version = inner.state.read().await.version as u32;
        let data_dir = inner.config.data_dir.clone();
        let result = tokio::task::spawn_blocking(move || {
            let lib = Library::new(data_dir.to_str().unwrap_or(""))?;
            lib.get_favourites()
        })
        .await
        .map_err(|e| CoreError::Daemon(e.to_string()))?;
        match result {
            Ok(tracks) => Ok(DaemonRes::Tracks { version, tracks }),
            Err(e) => Ok(DaemonRes::Error {
                version,
                message: e,
            }),
        }
    }

    async fn cmd_add_favourite(inner: &DaemonInner, track_id: i64) -> Result<DaemonRes, CoreError> {
        let version = inner.state.read().await.version as u32;
        let data_dir = inner.config.data_dir.clone();
        let result = tokio::task::spawn_blocking(move || {
            let lib = Library::new(data_dir.to_str().unwrap_or(""))?;
            lib.toggle_favourite(track_id)
        })
        .await
        .map_err(|e| CoreError::Daemon(e.to_string()))?;
        match result {
            Ok(_) => Ok(DaemonRes::Ok { version }),
            Err(e) => Ok(DaemonRes::Error {
                version,
                message: e,
            }),
        }
    }

    async fn cmd_remove_favourite(inner: &DaemonInner, track_id: i64) -> Result<DaemonRes, CoreError> {
        let version = inner.state.read().await.version as u32;
        let data_dir = inner.config.data_dir.clone();
        let result = tokio::task::spawn_blocking(move || {
            let lib = Library::new(data_dir.to_str().unwrap_or(""))?;
            lib.toggle_favourite(track_id)
        })
        .await
        .map_err(|e| CoreError::Daemon(e.to_string()))?;
        match result {
            Ok(_) => Ok(DaemonRes::Ok { version }),
            Err(e) => Ok(DaemonRes::Error {
                version,
                message: e,
            }),
        }
    }

    async fn cmd_yt_search(
        inner: &DaemonInner,
        query: &str,
        filter: Option<gtm_core::state::YTFilter>,
    ) -> Result<DaemonRes, CoreError> {
        let version = inner.state.read().await.version as u32;
        match inner.youtube.lock().await.search(query, filter).await {
            Ok(()) => Ok(DaemonRes::Ok { version }),
            Err(e) => Ok(DaemonRes::Error {
                version,
                message: e,
            }),
        }
    }

    async fn cmd_yt_search_poll(inner: &DaemonInner) -> Result<DaemonRes, CoreError> {
        let version = inner.state.read().await.version as u32;
        match inner.youtube.lock().await.poll_results().await {
            Ok(Some(results)) => Ok(DaemonRes::YtSearchResults { version, results }),
            Ok(None) => Ok(DaemonRes::Ok { version }),
            Err(e) => Ok(DaemonRes::Error {
                version,
                message: e,
            }),
        }
    }

    async fn cmd_yt_search_cancel(inner: &DaemonInner) -> Result<DaemonRes, CoreError> {
        let version = inner.state.read().await.version as u32;
        inner.youtube.lock().await.cancel().await;
        Ok(DaemonRes::Ok { version })
    }

    async fn cmd_yt_resolve_stream(inner: &DaemonInner, url: &str) -> Result<DaemonRes, CoreError> {
        let version = inner.state.read().await.version as u32;
        match inner.youtube.lock().await.resolve_stream(url).await {
            Ok(info) => Ok(DaemonRes::StreamInfo {
                version,
                info: Box::new(info),
            }),
            Err(e) => Ok(DaemonRes::Error {
                version,
                message: e,
            }),
        }
    }

    async fn cmd_get_status(inner: &DaemonInner) -> Result<DaemonRes, CoreError> {
        let state = inner.state.read().await;
        let version = state.version as u32;
        let state_clone = state.clone();
        drop(state);
        Ok(DaemonRes::Status {
            version,
            state: Box::new(state_clone),
        })
    }

    async fn cmd_get_cover_art(inner: &DaemonInner, track_id: i64) -> Result<DaemonRes, CoreError> {
        let mut discovered_artist = String::new();
        let mut discovered_album = String::new();

        // Try embedded cover / sidecar from library first
        let lib = if !inner.config.test_mode {
            Library::new(inner.config.data_dir.to_str().unwrap_or("")).ok()
        } else {
            None
        };
        if let Some(ref library) = lib {
            if let Ok(Some(track)) = library.get_track(track_id) {
                if let Some(ref path) = track.cover_path {
                    if let Ok(data) = tokio::fs::read(path).await {
                        use base64::Engine;
                        let b64 = base64::engine::general_purpose::STANDARD.encode(&data);
                        return Ok(DaemonRes::CoverArt { version: u32::MAX, data: Some(b64) });
                    }
                }
                // Sidecar .jpg/.jpeg/.png/.webp next to the audio file
                let audio_path = std::path::Path::new(&track.path);
                let parent = audio_path.parent().unwrap_or(std::path::Path::new(""));
                let stem = audio_path.file_stem().unwrap_or_default();
                for ext in ["jpg", "jpeg", "png", "webp"] {
                    let sidecar = parent.join(format!("{}.{}", stem.to_string_lossy(), ext));
                    if let Ok(data) = tokio::fs::read(&sidecar).await {
                        use base64::Engine;
                        let b64 = base64::engine::general_purpose::STANDARD.encode(&data);
                        return Ok(DaemonRes::CoverArt { version: u32::MAX, data: Some(b64) });
                    }
                }
                discovered_artist = track.artist;
                discovered_album = track.album;
            }
        }

        // If not found in library, search the queue for the requested track_id
        if discovered_artist.is_empty() {
            let state = inner.state.read().await;
            if let Some(t) = state.queue.iter().find(|t| t.id == track_id) {
                discovered_artist = t.artist.clone();
                discovered_album = t.album.clone();
            } else if let Some(ref t) = state.current_track {
                discovered_artist = t.artist.clone();
                discovered_album = t.album.clone();
            }
        }

        // Deezer fallback via CoverCache with discovered artist/album
        if !discovered_artist.is_empty() && !discovered_album.is_empty() {
            let mut guard = inner.cover_cache.lock().await;
            if let Some(ref mut cache) = *guard {
                let artist = discovered_artist.clone();
                let album = discovered_album.clone();
                let cover = tokio::time::timeout(Duration::from_secs(5), cache.get_cover(&artist, &album)).await
                    .ok()
                    .flatten();
                if let Some(cover) = cover {
                    use base64::Engine;
                    let b64 = base64::engine::general_purpose::STANDARD.encode(&cover.data);
                    return Ok(DaemonRes::CoverArt { version: u32::MAX, data: Some(b64) });
                }
            }
        }

        Ok(DaemonRes::CoverArt { version: u32::MAX, data: None })
    }

    async fn cmd_get_lyrics(inner: &DaemonInner, track_id: i64) -> Result<DaemonRes, CoreError> {
        let track = {
            let state = inner.state.read().await;
            if let Some(ref t) = state.current_track {
                if t.id == track_id {
                    Some(t.clone())
                } else {
                    None
                }
            } else {
                None
            }
        };
        let track = if let Some(t) = track {
            t
        } else if !inner.config.test_mode {
            let lib = Library::new(inner.config.data_dir.to_str().unwrap_or("")).ok();
            if let Some(ref library) = lib {
                match library.get_track(track_id) {
                    Ok(Some(t)) => t,
                    _ => {
                        return Ok(DaemonRes::Lyrics { version: u32::MAX, lyrics: None });
                    }
                }
            } else {
                return Ok(DaemonRes::Lyrics { version: u32::MAX, lyrics: None });
            }
        } else {
            return Ok(DaemonRes::Lyrics { version: u32::MAX, lyrics: None });
        };

        if let Some(ref manager) = inner.lyrics_manager {
            let lyrics = tokio::time::timeout(
                Duration::from_secs(5),
                manager.get_lyrics(&track),
            )
            .await
            .ok()
            .flatten();
            Ok(DaemonRes::Lyrics { version: u32::MAX, lyrics })
        } else {
            Ok(DaemonRes::Lyrics { version: u32::MAX, lyrics: None })
        }
    }
}
