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
use std::sync::Arc;
use std::time::Duration;

use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::{broadcast, mpsc, RwLock};
use tracing::{error, info, warn};

use gtm_audio::{AudioEvent, AudioMixer, AudioResult, Mixer, NullMixer};
use gtm_core::ipc::{DaemonEvent, DaemonReq, DaemonRes, QueueAction};
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
type ReplyTx = mpsc::UnboundedSender<DaemonRes>;

pub struct Daemon {
    pub state: Arc<RwLock<DaemonState>>,
    pub mixer: Box<dyn Mixer>,
    pub listener: UnixListener,
    pub pulse_listener: UnixListener,
    pub config: DaemonConfig,
    pub event_tx: broadcast::Sender<DaemonEvent>,
    pub library: Option<Library>,
    pub cover_cache: Option<CoverCache>,
    pub lyrics_manager: Option<LyricsManager>,
    pub youtube: YoutubeManager,
    req_tx: mpsc::UnboundedSender<(ClientId, DaemonReq, ReplyTx)>,
    req_rx: mpsc::UnboundedReceiver<(ClientId, DaemonReq, ReplyTx)>,
    next_client_id: ClientId,
    crossfade_loaded_for: Option<String>,
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
        let library = if !config.test_mode {
            Library::new(config.data_dir.to_str().unwrap_or("")).ok()
        } else {
            None
        };

        Ok(Self {
            state,
            mixer,
            listener,
            pulse_listener,
            config,
            event_tx,
            req_tx,
            req_rx,
            next_client_id: 0,
            crossfade_loaded_for: None,
            library,
            cover_cache: Some(CoverCache::new(cache_dir)),
            lyrics_manager: Some(LyricsManager::new()),
            youtube: YoutubeManager::new(),
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
            self.config.socket_path.display(),
            self.config.socket_pulse_path.display()
        );

        // Kick off background auto-scan so clients can connect immediately
        let bg_state = self.state.clone();
        let bg_lib_paths = self.config.library_paths.clone();
        let bg_data_dir = self.config.data_dir.clone();
        let bg_cache_dir = self.config.cache_dir.clone();
        let bg_req_tx = self.req_tx.clone();
        let bg_event_tx = self.event_tx.clone();
        tokio::spawn(async move {
            Self::background_scan(bg_state, bg_lib_paths, bg_data_dir, bg_cache_dir, bg_req_tx, bg_event_tx).await;
        });

        let mut poll_interval = tokio::time::interval(Duration::from_millis(16));
        loop {
            tokio::select! {
                _ = poll_interval.tick() => {
                    let result = self.mixer.poll();
                    self.handle_audio_event(result).await;
                }
                result = self.listener.accept() => {
                    match result {
                        Ok((stream, _addr)) => {
                            self.accept_client(stream).await;
                        }
                        Err(e) => {
                            error!("accept failed: {e}");
                        }
                    }
                }
                result = self.pulse_listener.accept() => {
                    match result {
                        Ok((stream, _addr)) => {
                            self.accept_pulse_client(stream);
                        }
                        Err(e) => {
                            error!("pulse accept failed: {e}");
                        }
                    }
                }
                Some((client_id, req, reply_tx)) = self.req_rx.recv() => {
                    self.dispatch(client_id, req, reply_tx).await;
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
        _req_tx: mpsc::UnboundedSender<(ClientId, DaemonReq, ReplyTx)>,
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
    async fn accept_client(&mut self, stream: UnixStream) {
        let client_id = self.next_client_id;
        self.next_client_id += 1;

        let (reader, writer) = stream.into_split();
        let req_tx = self.req_tx.clone();
        let event_rx = self.event_tx.subscribe();
        let (reply_tx, mut reply_rx) = mpsc::unbounded_channel();

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
                        let req: DaemonReq = match serde_json::from_str(trimmed) {
                            Ok(r) => r,
                            Err(e) => {
                                warn!("client {client_id} bad request: {e}");
                                continue;
                            }
                        };
                        if req_tx.send((client_id, req, r_tx.clone())).is_err() {
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
                            Some(response) => {
                                let line = match serde_json::to_string(&response) {
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
    fn accept_pulse_client(&self, stream: UnixStream) {
        let event_rx = self.event_tx.subscribe();
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

    async fn dispatch(&mut self, _client_id: ClientId, req: DaemonReq, reply_tx: ReplyTx) {
        let res = match self.handle_request(&req).await {
            Ok(res) => res,
            Err(e) => {
                warn!("command {:?} failed: {e}", req);
                DaemonRes::Error {
                    version: self.state.read().await.version as u32,
                    message: e.to_string(),
                }
            }
        };
        let _ = reply_tx.send(res);
    }

    async fn handle_request(&mut self, req: &DaemonReq) -> Result<DaemonRes, CoreError> {
        match req {
            DaemonReq::Play { path, start_pos } => self.cmd_play(path, *start_pos, false).await,
            DaemonReq::PlayPause => self.cmd_playpause().await,
            DaemonReq::Pause => self.cmd_pause().await,
            DaemonReq::Stop => self.cmd_stop().await,
            DaemonReq::Next => self.cmd_next().await,
            DaemonReq::Prev => self.cmd_prev().await,
            DaemonReq::Seek { position_secs } => self.cmd_seek(*position_secs).await,
            DaemonReq::SetVolume { volume } => self.cmd_set_volume(*volume).await,
            DaemonReq::ToggleShuffle => self.cmd_toggle_shuffle().await,
            DaemonReq::CycleRepeat { mode } => self.cmd_cycle_repeat(*mode).await,
            DaemonReq::ToggleMute => self.cmd_toggle_mute().await,
            DaemonReq::Crossfade {
                enabled,
                duration_secs,
            } => self.cmd_crossfade(*enabled, *duration_secs).await,
            DaemonReq::SetCrossfadeEasing { easing } => self.cmd_set_crossfade_easing(*easing).await,
            DaemonReq::Queue { action } => self.cmd_queue(action).await,
            DaemonReq::Library { action } => self.cmd_library(action).await,
            DaemonReq::Search { query } => self.cmd_search(query).await,
            DaemonReq::GetFavourites => self.cmd_get_favourites().await,
            DaemonReq::AddFavourite { track_id } => self.cmd_add_favourite(*track_id).await,
            DaemonReq::RemoveFavourite { track_id } => self.cmd_remove_favourite(*track_id).await,
            DaemonReq::YtSearch { query, filter } => self.cmd_yt_search(query, *filter).await,
            DaemonReq::YtSearchPoll => self.cmd_yt_search_poll().await,
            DaemonReq::YtSearchCancel => self.cmd_yt_search_cancel().await,
            DaemonReq::YtResolveStream { url } => self.cmd_yt_resolve_stream(url).await,
            DaemonReq::GetStatus => self.cmd_get_status().await,
            DaemonReq::SetEqPreset { preset } => self.cmd_set_eq_preset(*preset).await,
            DaemonReq::SetEqEnabled { enabled } => self.cmd_set_eq_enabled(*enabled).await,
            DaemonReq::SetReverb { enabled, room_size } => self.cmd_set_reverb(*enabled, *room_size).await,
        DaemonReq::GetCoverArt { track_id }           => self.cmd_get_cover_art(*track_id).await,
        DaemonReq::GetLyrics { track_id }             => self.cmd_get_lyrics(*track_id).await,
            DaemonReq::Ping => Ok(DaemonRes::Pong),
            DaemonReq::Quit => {
                info!("quit requested");
                let _ = self.cmd_stop().await;
                let _ = self.event_tx.send(DaemonEvent::Custom {
                    name: "daemon_quitting".into(),
                    data: [].into(),
                });
                tokio::time::sleep(Duration::from_millis(50)).await;
                let _ = std::fs::remove_file(&self.config.socket_path);
                let pulse_path = format!("{}.pulse", self.config.socket_path.display());
                let _ = std::fs::remove_file(&pulse_path);
                std::process::exit(0);
            }
        }
    }

    fn push_event(&self, event: DaemonEvent) {
        let _ = self.event_tx.send(event);
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
    async fn handle_audio_event(&mut self, result: AudioResult<Option<AudioEvent>>) {
        let ev = match result {
            Ok(Some(e)) => e,
            Ok(None) => {
                return;
            }
            Err(e) => {
                warn!("backend error: {e}");
                let _ = self.event_tx.send(DaemonEvent::Custom {
                    name: "backend_error".into(),
                    data: [("error".into(), e.to_string())].into(),
                });
                return;
            }
        };

        match ev {
            AudioEvent::Position(pos) => {
                let mut state = self.state.write().await;
                state.time_pos = pos;
                let dur = state.duration;
                let crossfade = state.crossfade.clone();
                let cur_path = state.current_track.as_ref().map(|t| t.path.clone());
                let queue_len = state.queue.len();
                drop(state);

                // Crossfade logic: when the track is within (duration_secs + 0.5s)
                // of the end, load the next track on the standby player and start
                // fading.  The standby player was already advanced (queue_cursor + 1)
                // so it's ready to go.
                if let Some(cf) = crossfade {
                    if cf.enabled
                        && dur > 0.0
                        && queue_len > 0
                        && !self.mixer.is_crossfading()
                        && self.crossfade_loaded_for.is_none()
                        && (dur - pos) <= cf.duration_secs as f64 + 0.5
                    {
                        // Determine which track comes next (considering repeat-all)
                        let next_path = {
                            let s = self.state.read().await;
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
                                if self.mixer.load_standby_decoded(source).is_ok() {
                                    self.mixer.set_crossfade_easing(cf.easing);
                                    self.mixer.start_crossfade(cf.duration_secs as f64);
                                    self.crossfade_loaded_for = cur_path.clone();
                                    if let Ok(mut s) = self.state.try_write() {
                                        let _ = s.advance_queue(1);
                                        let idx = s.queue_cursor;
                                        drop(s);
                                        self.push_event(DaemonEvent::QueueIndexChanged { index: idx });
                                    }
                                }
                            }
                        }
                    }
                }

                }
                AudioEvent::Duration(dur) => {
                let mut state = self.state.write().await;
                state.duration = dur;
                drop(state);
                self.push_event(DaemonEvent::DurationChanged { duration: dur });
            }
            // Track finished — if we were crossfading the next track is already
            // playing on the swapped player.  Emit PlaybackStarted so the
            // client knows which track is now active.
            AudioEvent::Finished => {
                let was_crossfading = self.crossfade_loaded_for.is_some();
                self.crossfade_loaded_for = None;
                if was_crossfading {
                    let actual = self.mixer.current_position();
                    let mut state = self.state.write().await;
                    state.status = PlaybackStatus::Playing;
                    state.time_pos = actual;
                    // Queue cursor was already advanced when crossfade started.
                    if state.queue_cursor < state.queue.len() as u128 {
                        state.current_track =
                            Some(state.queue[state.queue_cursor as usize].clone());
                    }
                    let track = state.current_track.clone();
                    drop(state);
                    if let Some(t) = track {
                        self.push_event(DaemonEvent::PlaybackStarted {
                            track: t.clone(),
                            auto_advanced: true,
                            time_pos: actual,
                            duration: t.duration as f64,
                        });
                    }
                } else {
                    // No crossfade — try advancing to the next track.
                    // Don't push TrackEnded yet; cmd_next() will either
                    // push PlaybackStarted (crossfade or play) or we push
                    // TrackEnded if there's no next track.
                    let mut state = self.state.write().await;
                    state.status = PlaybackStatus::Stopped;
                    state.time_pos = 0.0;
                    state.current_track = None;
                    drop(state);
                    let res = self.cmd_next().await;
                    // If cmd_next didn't start playback (end of queue, error),
                    // notify the client.
                    if res.is_err() || self.state.read().await.status == PlaybackStatus::Stopped {
                        self.push_event(DaemonEvent::TrackEnded);
                    }
                }
            }
            AudioEvent::Error(msg) => {
                warn!("audio error: {msg}");
                self.push_event(DaemonEvent::Custom {
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
    async fn cmd_play(&mut self, path: &str, start_pos: f64, auto_advanced: bool) -> Result<DaemonRes, CoreError> {
        self.mixer.stop()?;
        self.crossfade_loaded_for = None;
        {
            let mut state = self.state.write().await;
            if state.status != PlaybackStatus::Stopped {
                state.stop()?;
            }
        }

        // Decode in a blocking thread so the daemon event loop stays responsive
        let path_owned = path.to_string();
        let path_for_blocking = path_owned.clone();
        let source = tokio::task::spawn_blocking(move || {
            AudioMixer::decode_file(&path_for_blocking)
        })
        .await
        .map_err(|e| CoreError::Daemon(format!("spawn_blocking: {e}")))?
        .map_err(|e| CoreError::Daemon(format!("decode: {e}")))?;

        self.mixer.load_active_decoded(source, start_pos)?;
        self.mixer.play()?;
        let dur = self.mixer.duration();
        let mut state = self.state.write().await;

        // Look up real metadata from the library if available
        let track = if let Some(ref lib) = self.library {
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
        };
        state.play(track.clone())?;
        state.time_pos = start_pos;
        state.duration = dur;
        let version = state.version as u32;
        drop(state);
        self.push_event(DaemonEvent::PlaybackStarted {
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
    async fn cmd_playpause(&mut self) -> Result<DaemonRes, CoreError> {
        let is_playing = self.mixer.is_playing();
        if is_playing {
            self.cmd_pause().await
        } else {
            let state = self.state.read().await;
            let is_paused = state.status == PlaybackStatus::Paused;
            let path = state
                .current_track
                .as_ref()
                .map(|t| t.path.clone())
                .unwrap_or_default();
            drop(state);

            if is_paused && !path.is_empty() {
                // Resume from paused — unpause backend without reloading
                self.mixer.play()?;
                let mut state = self.state.write().await;
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
                self.push_event(DaemonEvent::PlaybackStarted {
                    track,
                    auto_advanced: false,
                    time_pos,
                    duration,
                });
                Ok(DaemonRes::Ok { version })
            } else if !path.is_empty() {
                self.cmd_play(&path, 0.0, false).await
            } else {
                // Stopped with no current track — try first track in queue
                let state = self.state.read().await;
                let queue = state.queue.clone();
                let cursor = state.queue_cursor as usize;
                drop(state);
                if !queue.is_empty() {
                    let idx = cursor.min(queue.len() - 1);
                    self.cmd_play(&queue[idx].path, 0.0, false).await
                } else {
                    let version = self.state.read().await.version as u32;
                    Ok(DaemonRes::Ok { version })
                }
            }
        }
    }

    async fn cmd_pause(&mut self) -> Result<DaemonRes, CoreError> {
        self.mixer.pause()?;
        let mut state = self.state.write().await;
        state.pause()?;
        state.time_pos = self.mixer.current_position();
        let version = state.version as u32;
        let time_pos = state.time_pos;
        drop(state);
        self.push_event(DaemonEvent::PlaybackPaused { time_pos });
        Ok(DaemonRes::Ok { version })
    }

    /// Stop playback: stops the mixer backend, transitions state to Stopped,
    /// and broadcasts PlaybackStopped.  Safe to call when already stopped
    /// (checks status before calling state.stop() to avoid assert).
    async fn cmd_stop(&mut self) -> Result<DaemonRes, CoreError> {
        self.mixer.stop()?;
        self.crossfade_loaded_for = None;
        let mut state = self.state.write().await;
        if state.status != PlaybackStatus::Stopped {
            state.stop()?;
        }
        let version = state.version as u32;
        drop(state);
        self.push_event(DaemonEvent::PlaybackStopped);
        Ok(DaemonRes::Ok { version })
    }

    /// Advance to next track in the queue.  Advances cursor via
    /// state.advance_queue(1), then plays the track at the new cursor.
    /// Returns Ok if no next track (already at end).
    async fn cmd_next(&mut self) -> Result<DaemonRes, CoreError> {
        let mut state = self.state.write().await;
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
        if crossfade_enabled && crossfade_dur > 0.0 && self.crossfade_loaded_for.is_none() && !self.mixer.is_crossfading() {
            let path = track.path.clone();
            let path_owned = path.clone();
            let decoded = tokio::task::spawn_blocking(move || {
                AudioMixer::decode_file(&path_owned)
            })
            .await;
            if let Ok(Ok(source)) = decoded {
                if self.mixer.load_standby_decoded(source).is_ok() {
                    self.mixer.set_crossfade_easing(crossfade_easing);
                    self.mixer.start_crossfade(crossfade_dur);
                    self.crossfade_loaded_for = Some(
                        self.state.read().await.current_track.as_ref().map(|t| t.path.clone())
                            .unwrap_or_default()
                    );
                    self.push_event(DaemonEvent::QueueIndexChanged { index: idx });
                    let dur = self.mixer.duration();
                    {
                        let mut st = self.state.write().await;
                        st.status = PlaybackStatus::Playing;
                        st.current_track = Some(track.clone());
                        st.time_pos = 0.0;
                        st.duration = dur;
                    }
                    self.push_event(DaemonEvent::PlaybackStarted {
                        track,
                        auto_advanced: true,
                        time_pos: 0.0,
                        duration: dur,
                    });
                    return Ok(DaemonRes::Ok { version: self.state.read().await.version as u32 });
                }
            }
        }
        self.crossfade_loaded_for = None;
        let path = track.path.clone();
        let res = self.cmd_play(&path, 0.0, true).await?;
        self.push_event(DaemonEvent::QueueIndexChanged { index: idx });
        Ok(res)
    }

    /// Go to previous track.  If cursor is at 0 or queue is empty,
    /// seek to beginning of current track instead.
    async fn cmd_prev(&mut self) -> Result<DaemonRes, CoreError> {
        let mut state = self.state.write().await;
        if state.queue.is_empty() || state.queue_cursor == 0 {
            drop(state);
            return self.cmd_seek(0.0).await;
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
        if crossfade_enabled && crossfade_dur > 0.0 && self.crossfade_loaded_for.is_none() && !self.mixer.is_crossfading() {
            let path = track.path.clone();
            let path_owned = path.clone();
            let decoded = tokio::task::spawn_blocking(move || {
                AudioMixer::decode_file(&path_owned)
            })
            .await;
            if let Ok(Ok(source)) = decoded {
                if self.mixer.load_standby_decoded(source).is_ok() {
                    self.mixer.set_crossfade_easing(crossfade_easing);
                    self.mixer.start_crossfade(crossfade_dur);
                    self.crossfade_loaded_for = Some(
                        self.state.read().await.current_track.as_ref().map(|t| t.path.clone())
                            .unwrap_or_default()
                    );
                    self.push_event(DaemonEvent::QueueIndexChanged { index: idx });
                    let dur = self.mixer.duration();
                    {
                        let mut st = self.state.write().await;
                        st.status = PlaybackStatus::Playing;
                        st.current_track = Some(track.clone());
                        st.time_pos = 0.0;
                        st.duration = dur;
                    }
                    self.push_event(DaemonEvent::PlaybackStarted {
                        track,
                        auto_advanced: true,
                        time_pos: 0.0,
                        duration: dur,
                    });
                    return Ok(DaemonRes::Ok { version: self.state.read().await.version as u32 });
                }
            }
        }
        self.crossfade_loaded_for = None;
        let path = track.path.clone();
        let res = self.cmd_play(&path, 0.0, true).await?;
        self.push_event(DaemonEvent::QueueIndexChanged { index: idx });
        Ok(res)
    }

    /// Seek to absolute position in seconds.  Errors if status is Stopped.
    /// Reports the *actual* position (mixer.current_position()) in the event,
    /// which may differ from the requested pos due to clamping.
    async fn cmd_seek(&mut self, pos: f64) -> Result<DaemonRes, CoreError> {
        let state = self.state.read().await;
        if state.status == PlaybackStatus::Stopped {
            return Err(CoreError::Daemon("cannot seek while stopped".into()));
        }
        drop(state);
        self.mixer.seek(pos)?;
        let actual = self.mixer.current_position();
        let mut state = self.state.write().await;
        state.seek(actual)?;
        let version = state.version as u32;
        drop(state);
        self.push_event(DaemonEvent::PositionChanged { time_pos: actual });
        Ok(DaemonRes::Ok { version })
    }

    async fn cmd_set_volume(&mut self, volume: u8) -> Result<DaemonRes, CoreError> {
        self.mixer.set_volume(volume)?;
        let mut state = self.state.write().await;
        state.set_volume(volume)?;
        let version = state.version as u32;
        drop(state);
        self.push_event(DaemonEvent::VolumeChanged { volume });
        Ok(DaemonRes::Ok { version })
    }

    async fn cmd_toggle_shuffle(&mut self) -> Result<DaemonRes, CoreError> {
        let mut state = self.state.write().await;
        state.toggle_shuffle()?;
        let enabled = state.shuffle;
        let version = state.version as u32;
        drop(state);
        self.push_event(DaemonEvent::ShuffleChanged { enabled });
        Ok(DaemonRes::Ok { version })
    }

    async fn cmd_cycle_repeat(
        &mut self,
        mode: gtm_core::state::RepeatMode,
    ) -> Result<DaemonRes, CoreError> {
        let mut state = self.state.write().await;
        state.cycle_repeat(mode)?;
        let m = state.repeat;
        let version = state.version as u32;
        drop(state);
        self.push_event(DaemonEvent::RepeatModeChanged { mode: m });
        Ok(DaemonRes::Ok { version })
    }

    async fn cmd_toggle_mute(&mut self) -> Result<DaemonRes, CoreError> {
        let mut state = self.state.write().await;
        state.toggle_mute()?;
        let muted = state.mute;
        let version = state.version as u32;
        drop(state);
        if muted {
            self.mixer.set_volume(0)?;
        } else {
            let vol = self.state.read().await.volume;
            self.mixer.set_volume(vol)?;
        }
        Ok(DaemonRes::Ok { version })
    }

    async fn cmd_crossfade(
        &mut self,
        enabled: bool,
        duration_secs: u8,
    ) -> Result<DaemonRes, CoreError> {
        let mut state = self.state.write().await;
        state.set_crossfade(enabled, duration_secs)?;
        let version = state.version as u32;
        drop(state);
        self.push_event(DaemonEvent::CrossfadeChanged {
            enabled,
            duration_secs,
        });
        Ok(DaemonRes::Ok { version })
    }

    async fn cmd_set_crossfade_easing(
        &mut self,
        easing: gtm_core::state::Easing,
    ) -> Result<DaemonRes, CoreError> {
        let mut state = self.state.write().await;
        if let Some(ref mut cf) = state.crossfade {
            cf.easing = easing;
        }
        state.version += 1;
        let version = state.version as u32;
        drop(state);
        Ok(DaemonRes::Ok { version })
    }

    async fn cmd_set_eq_preset(
        &mut self,
        preset: EqPreset,
    ) -> Result<DaemonRes, CoreError> {
        let mut state = self.state.write().await;
        state.eq_preset = preset;
        state.version += 1;
        let version = state.version as u32;
        drop(state);
        self.mixer.set_eq_preset(&preset);
        self.push_event(DaemonEvent::EqPresetChanged { preset });
        Ok(DaemonRes::Ok { version })
    }

    async fn cmd_set_eq_enabled(
        &mut self,
        enabled: bool,
    ) -> Result<DaemonRes, CoreError> {
        let mut state = self.state.write().await;
        state.eq_enabled = enabled;
        state.version += 1;
        let version = state.version as u32;
        drop(state);
        self.mixer.set_eq_enabled(enabled);
        self.push_event(DaemonEvent::EqEnabledChanged { enabled });
        Ok(DaemonRes::Ok { version })
    }

    async fn cmd_set_reverb(
        &mut self,
        enabled: bool,
        room_size: f32,
    ) -> Result<DaemonRes, CoreError> {
        let mut state = self.state.write().await;
        state.reverb = ReverbConfig { enabled, room_size };
        state.version += 1;
        let version = state.version as u32;
        drop(state);
        self.mixer.set_reverb(&ReverbConfig { enabled, room_size });
        self.push_event(DaemonEvent::ReverbChanged { enabled, room_size });
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
    async fn cmd_queue(&mut self, action: &QueueAction) -> Result<DaemonRes, CoreError> {
        match action {
            QueueAction::List => {
                let state = self.state.read().await;
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
                let mut state = self.state.write().await;
                queue::queue_clear(&mut state);
                let version = state.version as u32;
                let queue = state.queue.clone();
                let cursor = state.queue_cursor;
                drop(state);
                self.push_event(DaemonEvent::QueueChanged { queue, cursor });
                return Ok(DaemonRes::Ok { version });
            }
            QueueAction::Remove { index } => {
                let mut state = self.state.write().await;
                queue::queue_remove(&mut state, *index);
                let version = state.version as u32;
                let queue = state.queue.clone();
                let cursor = state.queue_cursor;
                drop(state);
                self.push_event(DaemonEvent::QueueChanged { queue, cursor });
                return Ok(DaemonRes::Ok { version });
            }
            QueueAction::Move { from, to } => {
                let mut state = self.state.write().await;
                queue::queue_move(&mut state, *from, *to);
                let version = state.version as u32;
                let queue = state.queue.clone();
                let cursor = state.queue_cursor;
                drop(state);
                self.push_event(DaemonEvent::QueueChanged { queue, cursor });
                return Ok(DaemonRes::Ok { version });
            }
            QueueAction::Add { path, position } => {
                let was_empty;
                {
                    let mut state = self.state.write().await;
                    was_empty = state.queue.is_empty() && state.status == PlaybackStatus::Stopped;
                    queue::queue_add(&mut state, path, *position);
                    let queue = state.queue.clone();
                    let cursor = state.queue_cursor;
                    drop(state);
                    self.push_event(DaemonEvent::QueueChanged { queue, cursor });
                    if was_empty {
                        // Auto-play; if it fails (e.g. file missing), still
                        // report success for the queue operation.
                        let _ = self.cmd_play(path, 0.0, false).await;
                    }
                }
                let version = self.state.read().await.version as u32;
                Ok(DaemonRes::Ok { version })
            }
            QueueAction::AddMany { paths } => {
                let was_empty;
                let first_path;
                {
                    let mut state = self.state.write().await;
                    was_empty = state.queue.is_empty() && state.status == PlaybackStatus::Stopped;
                    queue::queue_add_many(&mut state, paths);
                    first_path = paths[0].clone();
                    let queue = state.queue.clone();
                    let cursor = state.queue_cursor;
                    drop(state);
                    self.push_event(DaemonEvent::QueueChanged { queue, cursor });
                    if was_empty {
                        let _ = self.cmd_play(&first_path, 0.0, false).await;
                    }
                }
                let version = self.state.read().await.version as u32;
                Ok(DaemonRes::Ok { version })
            }
            QueueAction::AddFolder { path } => {
                let paths = queue::scan_audio_files(path);
                if paths.is_empty() {
                    return Ok(DaemonRes::Error {
                        version: self.state.read().await.version as u32,
                        message: "no audio files found in folder".into(),
                    });
                }
                let was_empty;
                let first_path;
                {
                    let mut state = self.state.write().await;
                    was_empty = state.queue.is_empty() && state.status == PlaybackStatus::Stopped;
                    queue::queue_add_many(&mut state, &paths);
                    first_path = paths[0].clone();
                    let queue = state.queue.clone();
                    let cursor = state.queue_cursor;
                    drop(state);
                    self.push_event(DaemonEvent::QueueChanged { queue, cursor });
                    // If queue was empty and stopped, auto-play the first track
                    if was_empty {
                        return self.cmd_play(&first_path, 0.0, false).await;
                    }
                }
                let version = self.state.read().await.version as u32;
                Ok(DaemonRes::Ok { version })
            }
            QueueAction::Set { paths, start_idx } => {
                let mut state = self.state.write().await;
                queue::queue_set(&mut state, paths, *start_idx);
                let version = state.version as u32;
                let queue = state.queue.clone();
                let cursor = state.queue_cursor;
                drop(state);
                self.push_event(DaemonEvent::QueueChanged { queue, cursor });
                Ok(DaemonRes::Ok { version })
            }
        }
    }

    async fn cmd_library(
        &mut self,
        action: &gtm_core::ipc::LibraryAction,
    ) -> Result<DaemonRes, CoreError> {
        let version = self.state.read().await.version as u32;
        let res = match action {
            gtm_core::ipc::LibraryAction::Scan { path } => {
                let audio_dir = path.clone();
                let data_dir = self.config.data_dir.clone();
                let cache_dir = self.config.cache_dir.to_string_lossy().to_string();
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
                let data_dir = self.config.data_dir.clone();
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
                let data_dir = self.config.data_dir.clone();
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
                let data_dir = self.config.data_dir.clone();
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
                let data_dir = self.config.data_dir.clone();
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
                let data_dir = self.config.data_dir.clone();
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
                let data_dir = self.config.data_dir.clone();
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
                let data_dir = self.config.data_dir.clone();
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
                let data_dir = self.config.data_dir.clone();
                let cache_dir = self.config.cache_dir.clone();
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
            gtm_core::ipc::LibraryAction::ExportM3u { playlist_id, path } => {
                let playlist_id = *playlist_id;
                let export_path = path.clone();
                let data_dir = self.config.data_dir.clone();
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
                let data_dir = self.config.data_dir.clone();
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
                let data_dir = self.config.data_dir.clone();
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
                let data_dir = self.config.data_dir.clone();
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

    async fn cmd_search(&mut self, query: &str) -> Result<DaemonRes, CoreError> {
        let version = self.state.read().await.version as u32;
        let query = query.to_string();
        let data_dir = self.config.data_dir.clone();
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

    async fn cmd_get_favourites(&mut self) -> Result<DaemonRes, CoreError> {
        let version = self.state.read().await.version as u32;
        let data_dir = self.config.data_dir.clone();
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

    async fn cmd_add_favourite(&mut self, track_id: i64) -> Result<DaemonRes, CoreError> {
        let version = self.state.read().await.version as u32;
        let data_dir = self.config.data_dir.clone();
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

    async fn cmd_remove_favourite(&mut self, track_id: i64) -> Result<DaemonRes, CoreError> {
        let version = self.state.read().await.version as u32;
        let data_dir = self.config.data_dir.clone();
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
        &mut self,
        query: &str,
        filter: Option<gtm_core::state::YTFilter>,
    ) -> Result<DaemonRes, CoreError> {
        let version = self.state.read().await.version as u32;
        match self.youtube.search(query, filter).await {
            Ok(()) => Ok(DaemonRes::Ok { version }),
            Err(e) => Ok(DaemonRes::Error {
                version,
                message: e,
            }),
        }
    }

    async fn cmd_yt_search_poll(&mut self) -> Result<DaemonRes, CoreError> {
        let version = self.state.read().await.version as u32;
        match self.youtube.poll_results().await {
            Ok(Some(results)) => Ok(DaemonRes::YtSearchResults { version, results }),
            Ok(None) => Ok(DaemonRes::Ok { version }),
            Err(e) => Ok(DaemonRes::Error {
                version,
                message: e,
            }),
        }
    }

    async fn cmd_yt_search_cancel(&mut self) -> Result<DaemonRes, CoreError> {
        let version = self.state.read().await.version as u32;
        self.youtube.cancel().await;
        Ok(DaemonRes::Ok { version })
    }

    async fn cmd_yt_resolve_stream(&mut self, url: &str) -> Result<DaemonRes, CoreError> {
        let version = self.state.read().await.version as u32;
        match self.youtube.resolve_stream(url).await {
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

    async fn cmd_get_status(&mut self) -> Result<DaemonRes, CoreError> {
        let state = self.state.read().await;
        let version = state.version as u32;
        let state_clone = state.clone();
        drop(state);
        Ok(DaemonRes::Status {
            version,
            state: Box::new(state_clone),
        })
    }

    async fn cmd_get_cover_art(&mut self, track_id: i64) -> Result<DaemonRes, CoreError> {
        let mut discovered_artist = String::new();
        let mut discovered_album = String::new();

        // Try embedded cover / sidecar from library first
        if let Some(ref library) = self.library {
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
            let state = self.state.read().await;
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
            if let Some(ref mut cache) = self.cover_cache {
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

    async fn cmd_get_lyrics(&mut self, track_id: i64) -> Result<DaemonRes, CoreError> {
        let track = {
            let state = self.state.read().await;
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
        } else if let Some(ref library) = self.library {
            match library.get_track(track_id) {
                Ok(Some(t)) => t,
                _ => {
                    return Ok(DaemonRes::Lyrics { version: u32::MAX, lyrics: None });
                }
            }
        } else {
            return Ok(DaemonRes::Lyrics { version: u32::MAX, lyrics: None });
        };

        if let Some(ref manager) = self.lyrics_manager {
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
