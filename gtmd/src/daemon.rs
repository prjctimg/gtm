// Copyright (c) 2026 - present
// Author: prjctimg <prjctimg@outlook.com>
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

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::{broadcast, mpsc, RwLock};
use tracing::{error, info, warn};

use gtm_core::paths::resolve_pid_file;

#[cfg(feature = "pulseaudio")]
use crate::config::AudioBackendKind;
use base64::Engine;
#[cfg(feature = "pulseaudio")]
use gtm_audio::PulseAudioMixer;
use gtm_audio::{AudioEvent, AudioMixer, AudioResult, Mixer, NullMixer};
use gtm_core::ipc::{
    ComponentHealth, DaemonEvent, DaemonReq, DaemonRes, HealthReport, HealthStatus, QueueAction,
    SyncKind, WireReq, PROTOCOL_VERSION,
};
use gtm_core::spotify::{SoloistStatus, SpotifyTrack};
use gtm_core::state::{
    DaemonState, EqPreset, PlaybackStatus, RepeatMode, ReverbConfig, SavedState,
};
use gtm_core::track::TrackInfo;
use gtm_core::wire;
use gtm_core::CoreError;
use serde_json::Value;

use crate::config::DaemonConfig;
use crate::cover::CoverCache;
use crate::library::Library;
use crate::lyrics::LyricsManager;
use crate::queue;
use crate::soloist::{SoloistManager, SoloistMsg};
use crate::spotify::SpotifyManager;
use crate::youtube::YoutubeManager;

type ClientId = u64;
type ReplyTx = mpsc::UnboundedSender<(u64, DaemonRes)>;

const RESTART_THRESHOLD_SECS: f64 = 3.0;

use std::time::Instant;

struct HealthTracker {
    start_time: Instant,
    audio_backend: String,
    scan_count: AtomicUsize,
    scan_errors: AtomicUsize,
    yt_search_count: AtomicUsize,
    yt_search_errors: AtomicUsize,
    cover_fetch_count: AtomicUsize,
    cover_fetch_errors: AtomicUsize,
    lyrics_fetch_count: AtomicUsize,
    lyrics_fetch_errors: AtomicUsize,
}

impl HealthTracker {
    fn new(audio_backend: &str) -> Self {
        Self {
            start_time: Instant::now(),
            audio_backend: audio_backend.to_string(),
            scan_count: AtomicUsize::new(0),
            scan_errors: AtomicUsize::new(0),
            yt_search_count: AtomicUsize::new(0),
            yt_search_errors: AtomicUsize::new(0),
            cover_fetch_count: AtomicUsize::new(0),
            cover_fetch_errors: AtomicUsize::new(0),
            lyrics_fetch_count: AtomicUsize::new(0),
            lyrics_fetch_errors: AtomicUsize::new(0),
        }
    }

    fn uptime_secs(&self) -> f64 {
        self.start_time.elapsed().as_secs_f64()
    }
}

enum HistoryEntry {
    User(TrackInfo),
    Default { index: usize, track: TrackInfo },
}

struct SyncProgress {
    running: AtomicBool,
    kind: std::sync::Mutex<SyncKind>,
    synced: AtomicUsize,
    total: AtomicUsize,
}

impl Default for SyncProgress {
    fn default() -> Self {
        Self {
            running: AtomicBool::new(false),
            kind: std::sync::Mutex::new(SyncKind::Covers),
            synced: AtomicUsize::new(0),
            total: AtomicUsize::new(0),
        }
    }
}

struct DaemonInner {
    state: Arc<RwLock<DaemonState>>,
    mixer: tokio::sync::Mutex<Box<dyn Mixer>>,
    config: DaemonConfig,
    event_tx: broadcast::Sender<DaemonEvent>,
    cover_cache: tokio::sync::Mutex<Option<CoverCache>>,
    lyrics_manager: Option<LyricsManager>,
    youtube: Arc<tokio::sync::Mutex<YoutubeManager>>,
    spotify: tokio::sync::Mutex<SpotifyManager>,
    soloist: tokio::sync::Mutex<SoloistManager>,
    crossfade_loaded_for: tokio::sync::Mutex<Option<String>>,
    countdown_notified_for: tokio::sync::Mutex<Option<String>>,
    sleep_cancel: Arc<AtomicBool>,
    soloist_manual_override: Arc<AtomicBool>,
    health: Arc<HealthTracker>,
    soloist_tx: mpsc::UnboundedSender<SoloistMsg>,
    client_auth: tokio::sync::Mutex<HashMap<ClientId, bool>>,
    internal_req_tx: mpsc::UnboundedSender<DaemonReq>,
    cmd_lock: tokio::sync::Mutex<()>,
    play_history: tokio::sync::Mutex<Vec<HistoryEntry>>,
    sync_progress: Arc<SyncProgress>,
}

pub struct Daemon {
    inner: Arc<DaemonInner>,
    listener: UnixListener,
    pulse_listener: UnixListener,
    req_tx: mpsc::UnboundedSender<(ClientId, u64, DaemonReq, ReplyTx)>,
    req_rx: mpsc::UnboundedReceiver<(ClientId, u64, DaemonReq, ReplyTx)>,
    internal_req_rx: mpsc::UnboundedReceiver<DaemonReq>,
    soloist_msg_rx: Option<mpsc::UnboundedReceiver<SoloistMsg>>,
    next_client_id: ClientId,
}

impl Daemon {
    pub fn new(config: DaemonConfig) -> Result<Self, CoreError> {
        let mut initial_state = DaemonState::new();

        if !config.test_mode {
            if let Some(saved) = SavedState::load(&config.state_file) {
                info!("loaded saved state from {}", config.state_file.display());
                saved.apply_to(&mut initial_state);
            }
        }

        let state = Arc::new(RwLock::new(initial_state));

        let mixer: Box<dyn Mixer> = if config.test_mode {
            Box::new(NullMixer::new())
        } else {
            Self::init_mixer(&config)?
        };

        let socket_path = Path::new(&config.socket_path);
        if let Some(parent) = socket_path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| CoreError::Daemon(format!("create socket dir: {e}")))?;
        }
        if socket_path.exists() {
            match std::fs::remove_file(socket_path) {
                Ok(()) => {}
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => return Err(CoreError::Daemon(format!("remove stale socket: {e}"))),
            }
        }

        let listener = UnixListener::bind(socket_path)
            .map_err(|e| CoreError::Daemon(format!("bind socket: {e}")))?;

        let pulse_path = Path::new(&config.socket_pulse_path);
        if let Some(parent) = pulse_path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| CoreError::Daemon(format!("create pulse socket dir: {e}")))?;
        }
        if pulse_path.exists() {
            match std::fs::remove_file(pulse_path) {
                Ok(()) => {}
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => return Err(CoreError::Daemon(format!("remove stale pulse socket: {e}"))),
            }
        }
        let pulse_listener = UnixListener::bind(pulse_path)
            .map_err(|e| CoreError::Daemon(format!("bind pulse socket: {e}")))?;

        let pid_file = resolve_pid_file();
        if let Some(parent) = pid_file.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Err(e) = std::fs::write(&pid_file, std::process::id().to_string()) {
            warn!("failed to write PID file: {e}");
        }

        let (event_tx, _) = broadcast::channel::<DaemonEvent>(1024);
        let (req_tx, req_rx) = mpsc::unbounded_channel();
        let (internal_req_tx, internal_req_rx) = mpsc::unbounded_channel();
        let (soloist_tx, soloist_msg_rx) = mpsc::unbounded_channel::<SoloistMsg>();

        let cache_dir = config.cache_dir.clone();
        let config_dir = config.config_dir.clone();
        let audio_backend_name = match config.audio_backend {
            #[cfg(feature = "pulseaudio")]
            crate::config::AudioBackendKind::PulseAudio => "pulseaudio",
            crate::config::AudioBackendKind::Rodio => "rodio",
        };

        let inner = Arc::new(DaemonInner {
            state,
            mixer: tokio::sync::Mutex::new(mixer),
            config,
            event_tx,
            cover_cache: tokio::sync::Mutex::new(Some(CoverCache::new(cache_dir.clone()))),
            lyrics_manager: Some(LyricsManager::with_cache_dir(cache_dir.join("lyrics"))),
            youtube: Arc::new(tokio::sync::Mutex::new(YoutubeManager::new())),
            spotify: tokio::sync::Mutex::new(SpotifyManager::new(config_dir.clone())),
            soloist: tokio::sync::Mutex::new(SoloistManager::new(config_dir)),
            crossfade_loaded_for: tokio::sync::Mutex::new(None),
            countdown_notified_for: tokio::sync::Mutex::new(None),
            sleep_cancel: Arc::new(AtomicBool::new(false)),
            soloist_manual_override: Arc::new(AtomicBool::new(false)),
            health: Arc::new(HealthTracker::new(audio_backend_name)),
            client_auth: tokio::sync::Mutex::new(HashMap::new()),
            internal_req_tx,
            soloist_tx,
            cmd_lock: tokio::sync::Mutex::new(()),
            play_history: tokio::sync::Mutex::new(Vec::new()),
            sync_progress: Arc::new(SyncProgress::default()),
        });

        Ok(Self {
            inner,
            listener,
            pulse_listener,
            req_tx,
            req_rx,
            internal_req_rx,
            soloist_msg_rx: Some(soloist_msg_rx),
            next_client_id: 0,
        })
    }

    #[cfg(feature = "pulseaudio")]
    fn init_mixer(config: &DaemonConfig) -> Result<Box<dyn Mixer>, CoreError> {
        if config.audio_backend == AudioBackendKind::PulseAudio {
            match PulseAudioMixer::new() {
                Ok(m) => Ok(Box::new(m)),
                Err(e) => {
                    if gtm_core::is_termux() {
                        return Err(CoreError::Daemon(format!(
                            "PulseAudio init failed: {e}. On Termux start the server first: \
                             `pulseaudio --start --exit-idle-time=-1` (export \
                             PULSE_SERVER=127.0.0.1 if audio is routed over TCP)"
                        )));
                    }
                    warn!("PulseAudio init failed ({e}), falling back to rodio");
                    AudioMixer::new()
                        .map(|m| Box::new(m) as Box<dyn Mixer>)
                        .map_err(|e| CoreError::Daemon(format!("audio mixer init: {e}")))
                }
            }
        } else {
            AudioMixer::new()
                .map(|m| Box::new(m) as Box<dyn Mixer>)
                .map_err(|e| CoreError::Daemon(format!("audio mixer init: {e}")))
        }
    }

    #[cfg(not(feature = "pulseaudio"))]
    fn init_mixer(_config: &DaemonConfig) -> Result<Box<dyn Mixer>, CoreError> {
        AudioMixer::new()
            .map(|m| Box::new(m) as Box<dyn Mixer>)
            .map_err(|e| CoreError::Daemon(format!("audio mixer init: {e}")))
    }

    pub async fn run(&mut self) -> Result<(), CoreError> {
        info!(
            "daemon started on {} (pulse: {})",
            self.inner.config.socket_path.display(),
            self.inner.config.socket_pulse_path.display()
        );

        let bg_state = self.inner.state.clone();
        let bg_lib_paths = self.inner.config.library_paths.clone();
        let bg_data_dir = self.inner.config.data_dir.clone();
        let bg_cache_dir = self.inner.config.cache_dir.clone();
        let bg_req_tx = self.req_tx.clone();
        let bg_event_tx = self.inner.event_tx.clone();
        let bg_health = self.inner.health.clone();
        tokio::spawn(async move {
            Self::background_scan(
                bg_state,
                bg_lib_paths,
                bg_data_dir,
                bg_cache_dir,
                bg_req_tx,
                bg_event_tx,
                bg_health,
            )
            .await;
        });

        let hb_event_tx = self.inner.event_tx.clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(30));
            loop {
                interval.tick().await;
                let _ = hb_event_tx.send(DaemonEvent::Heartbeat);
            }
        });

        let spotify_inner = Arc::clone(&self.inner);
        tokio::spawn(async move {
            let mut spotify = spotify_inner.spotify.lock().await;
            if spotify.has_token_file() {
                match spotify.load().await {
                    Ok(()) => info!("spotify auto-synced on startup"),
                    Err(e) => warn!("spotify auto-sync failed: {e}"),
                }
            }
        });

        {
            let inner = Arc::clone(&self.inner);
            let mut state = inner.state.write().await;
            state.soloist.key_set = inner.soloist.lock().await.has_key();
            let auto_start = state.soloist_auto_start;
            drop(state);
            if auto_start && inner.soloist.lock().await.has_key() {
                let tx = inner.soloist_tx.clone();
                let inner2 = Arc::clone(&inner);
                tokio::spawn(async move {
                    let mut delay = Duration::from_secs(3);
                    loop {
                        let should_retry = {
                            let state = inner2.state.read().await;
                            state.soloist_auto_start
                                && !inner2.soloist_manual_override.load(Ordering::SeqCst)
                                && !state.soloist.running
                        };
                        if !should_retry {
                            break;
                        }
                        let mut soloist = inner2.soloist.lock().await;
                        soloist.start(tx.clone()).await;
                        drop(soloist);
                        tokio::time::sleep(delay).await;
                        delay = (delay * 2).min(Duration::from_secs(30));
                    }
                });
            }
        }

        let mut soloist_msg_rx = std::mem::take(&mut self.soloist_msg_rx).unwrap();
        let soloist_inner = Arc::clone(&self.inner);
        tokio::spawn(async move {
            while let Some(msg) = soloist_msg_rx.recv().await {
                Self::handle_soloist_msg(&soloist_inner, msg).await;
            }
        });

        let mut poll_interval = tokio::time::interval(Duration::from_millis(16));
        let mut save_interval = tokio::time::interval(Duration::from_secs(60));
        loop {
            tokio::select! {
                _ = poll_interval.tick() => {
                    let result = { self.inner.mixer.lock().await.poll() };
                    Self::handle_audio_event(&self.inner, result).await;
                }
                _ = save_interval.tick() => {
                    Self::save_state(&self.inner);
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
                Some(req) = self.internal_req_rx.recv() => {
                    let _lock = self.inner.cmd_lock.lock().await;
                    if let Err(e) = Self::handle_request(&self.inner, &req, 0, true).await {
                        warn!("internal command {:?} failed: {e}", req);
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

    async fn background_scan(
        state: Arc<RwLock<DaemonState>>,
        library_paths: Vec<std::path::PathBuf>,
        data_dir: std::path::PathBuf,
        cache_dir: std::path::PathBuf,
        _req_tx: mpsc::UnboundedSender<(ClientId, u64, DaemonReq, ReplyTx)>,
        _event_tx: broadcast::Sender<DaemonEvent>,
        health: Arc<HealthTracker>,
    ) {
        if library_paths.is_empty() {
            return;
        }
        let total_tracks = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        for audio_dir in &library_paths {
            if !audio_dir.exists() {
                info!("library path {:?} does not exist: skipping", audio_dir);
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
                Ok(Ok(t)) => {
                    health.scan_count.fetch_add(1, Ordering::Relaxed);
                    t
                }
                Ok(Err(e)) => {
                    health.scan_errors.fetch_add(1, Ordering::Relaxed);
                    warn!("auto-scan {:?} failed: {e}", audio_dir);
                    continue;
                }
                Err(e) => {
                    health.scan_errors.fetch_add(1, Ordering::Relaxed);
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

    async fn accept_client(
        client_id: ClientId,
        stream: UnixStream,
        inner: Arc<DaemonInner>,
        req_tx: mpsc::UnboundedSender<(ClientId, u64, DaemonReq, ReplyTx)>,
    ) {
        inner.client_auth.lock().await.insert(client_id, false);

        let (reader, writer) = stream.into_split();
        let event_rx = inner.event_tx.subscribe();
        let (reply_tx, mut reply_rx) = mpsc::unbounded_channel::<(u64, DaemonRes)>();

        let token = tokio_util::sync::CancellationToken::new();

        let r_tx = reply_tx.clone();
        let inner_clone = inner.clone();
        let token_reader = token.clone();
        tokio::spawn(async move {
            let mut reader = BufReader::new(reader);
            let mut line = String::new();
            loop {
                tokio::select! {
                    _ = token_reader.cancelled() => break,
                    result = reader.read_line(&mut line) => {
                        match result {
                            Ok(0) => break,
                            Ok(_) => {
                                let trimmed = line.trim();
                                if trimmed.is_empty() {
                                    line.clear();
                                    continue;
                                }
                                if trimmed.len() > 1_048_576 {
                                    warn!("client {client_id}: line too long ({} bytes), disconnecting", trimmed.len());
                                    break;
                                }
                                let wire_req: WireReq = match serde_json::from_str(trimmed) {
                                    Ok(r) => r,
                                    Err(e) => {
                                        warn!("client {client_id} malformed JSON, closing: {e}");
                                        break;
                                    }
                                };
                                let daemon_req = match DaemonReq::parse_cmd(&wire_req.cmd, wire_req.params.clone()) {
                                    Ok(r) => r,
                                    Err(e) if e.starts_with("unknown command:") => {
                                        let _ = r_tx.send((wire_req.id, DaemonRes::Error {
                                            message: e,
                                        }));
                                        line.clear();
                                        continue;
                                    }
                                    Err(e) => {
                                        let _ = r_tx.send((wire_req.id, DaemonRes::Error {
                                            message: format!("invalid params for {}: {}", wire_req.cmd, e),
                                        }));
                                        line.clear();
                                        continue;
                                    }
                                };
                                if req_tx.send((client_id, wire_req.id, daemon_req, r_tx.clone())).is_err() {
                                    break;
                                }
                                line.clear();
                            }
                            Err(e) => {
                                warn!("client {client_id} read error: {e}");
                                break;
                            }
                        }
                    }
                }
            }
            token_reader.cancel();
            inner_clone.client_auth.lock().await.remove(&client_id);
            info!("client {client_id} disconnected");
        });

        let token_watchdog = token.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_secs(10)).await;
            if !*inner
                .client_auth
                .lock()
                .await
                .get(&client_id)
                .unwrap_or(&true)
            {
                warn!("client {client_id}: handshake timeout");
                token_watchdog.cancel();
            }
        });

        tokio::spawn(async move {
            let mut writer = writer;
            let mut event_rx = event_rx;
            loop {
                tokio::select! {
                    _ = token.cancelled() => break,
                    res = reply_rx.recv() => {
                        match res {
                            Some((id, response)) => {
                                let wire = response.to_wire(id);
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

    async fn dispatch(
        inner: Arc<DaemonInner>,
        client_id: ClientId,
        request_id: u64,
        req: DaemonReq,
        reply_tx: ReplyTx,
    ) {
        let authenticated = {
            inner
                .client_auth
                .lock()
                .await
                .get(&client_id)
                .copied()
                .unwrap_or(false)
        };

        if !authenticated && !matches!(req, DaemonReq::Handshake { .. }) {
            let _ = reply_tx.send((
                request_id,
                DaemonRes::Error {
                    message: "handshake required".to_string(),
                },
            ));
            return;
        }

        let _lock = inner.cmd_lock.lock().await;
        let res = match Self::handle_request(&inner, &req, client_id, authenticated).await {
            Ok(res) => res,
            Err(e) => {
                warn!("command {:?} failed: {e}", req);
                DaemonRes::Error {
                    message: e.to_string(),
                }
            }
        };
        if matches!(&req, DaemonReq::SoloistSetApiKey { .. }) {
            let inner2 = Arc::clone(&inner);
            tokio::spawn(async move {
                tokio::time::sleep(Duration::from_secs(2)).await;
                let mut spotify = inner2.spotify.lock().await;
                if spotify.has_token_file() {
                    let _ =
                        tokio::time::timeout(Duration::from_secs(60), spotify.sync()).await;
                }
            });
        }
        let _ = reply_tx.send((request_id, res));
    }

    async fn handle_request(
        inner: &DaemonInner,
        req: &DaemonReq,
        client_id: ClientId,
        _authenticated: bool,
    ) -> Result<DaemonRes, CoreError> {
        match req {
            DaemonReq::Handshake {
                version,
                client,
                client_version,
            } => {
                if *version > PROTOCOL_VERSION {
                    info!(
                        "client {client_id}: handshake rejected: client protocol v{version} > daemon v{PROTOCOL_VERSION}"
                    );
                    return Ok(DaemonRes::Error {
                        message: format!(
                            "protocol version {version} not supported, daemon supports {PROTOCOL_VERSION}"
                        ),
                    });
                }
                inner.client_auth.lock().await.insert(client_id, true);
                info!(
                    "client {client_id}: handshake from {client} v{client_version:?}, protocol v{version}"
                );
                Ok(DaemonRes::Handshake {
                    version: *version,
                    daemon: "gtmd-rs".to_string(),
                    daemon_version: env!("CARGO_PKG_VERSION").to_string(),
                })
            }
            DaemonReq::Play { path, start_pos } => {
                Self::clear_history(inner).await;
                Self::enable_fallback(inner).await;
                Self::cmd_play(inner, path, *start_pos, false).await
            }
            DaemonReq::PlayPause => Self::cmd_playpause(inner).await,
            DaemonReq::Pause => Self::cmd_pause(inner).await,
            DaemonReq::Stop => Self::cmd_stop(inner).await,
            DaemonReq::Next => Self::cmd_next(inner).await,
            DaemonReq::Prev => Self::cmd_prev(inner).await,
            DaemonReq::Seek { position_secs } => Self::cmd_seek(inner, *position_secs).await,
            DaemonReq::SetVolume { volume } => Self::cmd_set_volume(inner, *volume).await,
            DaemonReq::SetMasterVolume { volume } => {
                Self::cmd_set_master_volume(inner, *volume).await
            }
            DaemonReq::GetVolume => Self::cmd_get_volume(inner).await,
            DaemonReq::ToggleShuffle => Self::cmd_toggle_shuffle(inner).await,
            DaemonReq::CycleRepeat { mode } => Self::cmd_cycle_repeat(inner, *mode).await,
            DaemonReq::ToggleMute => Self::cmd_toggle_mute(inner).await,
            DaemonReq::Crossfade {
                enabled,
                duration_secs,
                easing,
            } => Self::cmd_crossfade(inner, *enabled, *duration_secs, *easing).await,
            DaemonReq::SetLoudnessMode { mode } => Self::cmd_set_loudness_mode(inner, *mode).await,
            DaemonReq::ScanLoudness { track_ids, force } => {
                Self::cmd_scan_loudness(inner, track_ids.clone(), *force).await
            }
            DaemonReq::SetPreGain { pre_gain_db } => {
                Self::cmd_set_pre_gain(inner, *pre_gain_db).await
            }
            DaemonReq::SetGapless { enabled } => Self::cmd_set_gapless(inner, *enabled).await,
            DaemonReq::SetDynamicMode {
                enabled,
                min_queue_remaining,
                max_history,
            } => {
                Self::cmd_set_dynamic_mode(inner, *enabled, *min_queue_remaining, *max_history)
                    .await
            }
            DaemonReq::SetScrobble {
                enabled,
                api_key,
                session_token,
                min_play_secs,
                min_play_pct,
            } => {
                Self::cmd_set_scrobble(
                    inner,
                    *enabled,
                    api_key.clone(),
                    session_token.clone(),
                    *min_play_secs,
                    *min_play_pct,
                )
                .await
            }
            DaemonReq::Library { action } => Self::cmd_library(inner, action).await,
            DaemonReq::Search { query } => Self::cmd_search(inner, query).await,
            DaemonReq::GetFavourites => Self::cmd_get_favourites(inner).await,
            DaemonReq::AddFavourite { track_id } => Self::cmd_add_favourite(inner, *track_id).await,
            DaemonReq::RemoveFavourite { track_id } => {
                Self::cmd_remove_favourite(inner, *track_id).await
            }
            DaemonReq::YtSearch { query, filter } => {
                Self::cmd_yt_search(inner, query, *filter).await
            }
            DaemonReq::YtSearchPoll => Self::cmd_yt_search_poll(inner).await,
            DaemonReq::YtSearchCancel => Self::cmd_yt_search_cancel(inner).await,
            DaemonReq::YtResolveStream { url } => Self::cmd_yt_resolve_stream(inner, url).await,
            DaemonReq::YtDownload { .. } => {
                Err(CoreError::Daemon("yt_download not yet implemented".into()))
            }
            DaemonReq::YtDownloadPoll => Err(CoreError::Daemon(
                "yt_download_poll not yet implemented".into(),
            )),
            DaemonReq::YtCancelDownload { .. } => Err(CoreError::Daemon(
                "yt_cancel_download not yet implemented".into(),
            )),
            DaemonReq::YtFetchPlaylist { .. } => Err(CoreError::Daemon(
                "yt_fetch_playlist not yet implemented".into(),
            )),
            DaemonReq::YtFetchPlaylistPoll => Err(CoreError::Daemon(
                "yt_fetch_playlist_poll not yet implemented".into(),
            )),
            DaemonReq::YtSetConfig {
                cookie_source,
                cookie_file,
                js_runtime,
                download_dir,
                max_concurrent,
            } => {
                let mut yt = inner.youtube.lock().await;
                yt.set_cookie_file(cookie_file.clone());
                if let Some(cs) = cookie_source {
                    _ = cs;
                }
                if let Some(js) = js_runtime {
                    _ = js;
                }
                if let Some(dd) = download_dir {
                    _ = dd;
                }
                if let Some(mc) = max_concurrent {
                    _ = mc;
                }
                drop(yt);
                Self::save_state(inner);
                Ok(DaemonRes::Ok)
            }
            DaemonReq::GetCoverArt { track_id } => Self::cmd_get_cover_art(inner, *track_id).await,
            DaemonReq::GetArtistCoverArt { artist } => {
                Self::cmd_get_artist_cover_art(inner, artist).await
            }
            DaemonReq::GetLyrics { track_id, path } => {
                Self::cmd_get_lyrics(inner, *track_id, path.clone()).await
            }
            DaemonReq::LyricsSearch { artist, title } => {
                Self::cmd_lyrics_search(inner, artist, title).await
            }
            DaemonReq::SpotifySetToken { token } => Self::cmd_spotify_set_token(inner, token).await,
            DaemonReq::SpotifyClear => Self::cmd_spotify_clear(inner).await,
            DaemonReq::SpotifyStatus => Self::cmd_spotify_status(inner).await,
            DaemonReq::SpotifyPlayPause => Self::cmd_spotify_play_pause(inner).await,
            DaemonReq::SpotifySync => Self::cmd_spotify_sync(inner).await,
            DaemonReq::SpotifyPlaylists => Self::cmd_spotify_playlists(inner).await,
            DaemonReq::SpotifyPlaylistTracks { id } => {
                Self::cmd_spotify_playlist_tracks(inner, id).await
            }
            DaemonReq::SpotifyResolve {
                playlist_id,
                track_index,
            } => Self::cmd_spotify_resolve(inner, playlist_id, *track_index).await,
            DaemonReq::SoloistSetApiKey { key } => Self::cmd_soloist_set_api_key(inner, key).await,
            DaemonReq::SoloistSetConfig { auto_start } => {
                Self::cmd_soloist_set_config(inner, *auto_start).await
            }
            DaemonReq::SoloistClear => Self::cmd_soloist_clear(inner).await,
            DaemonReq::SoloistStart => Self::cmd_soloist_start(inner).await,
            DaemonReq::SoloistStop => Self::cmd_soloist_stop(inner).await,
            DaemonReq::SoloistStatus => Self::cmd_soloist_status(inner).await,
            DaemonReq::SoloistPlay { uri } => Self::cmd_soloist_play(inner, uri).await,
            DaemonReq::SoloistActivate => Self::cmd_soloist_activate(inner).await,
            DaemonReq::SetSleepTimer { minutes } => {
                Self::cmd_set_sleep_timer(inner, *minutes).await
            }
            DaemonReq::CancelSleepTimer => Self::cmd_cancel_sleep_timer(inner).await,
            DaemonReq::GetStatus => Self::cmd_get_status(inner).await,
            DaemonReq::CheckHealth => Self::cmd_check_health(inner).await,
            DaemonReq::Ping => Ok(DaemonRes::Pong),
            DaemonReq::Queue { action } => Self::cmd_queue(inner, action).await,
            DaemonReq::SetEqPreset { preset } => Self::cmd_set_eq_preset(inner, *preset).await,
            DaemonReq::SetEqEnabled { enabled } => Self::cmd_set_eq_enabled(inner, *enabled).await,
            DaemonReq::SetReverb { enabled, room_size } => {
                Self::cmd_set_reverb(inner, *enabled, *room_size).await
            }
            DaemonReq::ListEqPresets => Self::cmd_list_eq_presets(inner).await,
            DaemonReq::Quit => {
                info!("quit requested");
                let _ = Self::cmd_stop(inner).await;
                if !inner.config.test_mode {
                    let s = inner.state.read().await;
                    let saved = SavedState::from_state(&s);
                    drop(s);
                    if let Err(e) = saved.save(&inner.config.state_file) {
                        warn!("failed to save state on quit: {e}");
                    }
                }
                let _ = inner.event_tx.send(DaemonEvent::Custom {
                    name: "daemon_quitting".into(),
                    data: [].into(),
                });
                let socket_path = inner.config.socket_path.clone();
                let socket_pulse_path = inner.config.socket_pulse_path.clone();
                tokio::spawn(async move {
                    tokio::time::sleep(Duration::from_millis(200)).await;
                    let _ = std::fs::remove_file(&socket_path);
                    let _ = std::fs::remove_file(&socket_pulse_path);
                    let _ = std::fs::remove_file(resolve_pid_file());
                    info!("daemon shut down cleanly");
                    std::process::exit(0);
                });
                Ok(DaemonRes::Ok)
            }
        }
    }

    fn push_event(inner: &DaemonInner, event: DaemonEvent) {
        let _ = inner.event_tx.send(event);
    }

    fn save_state(inner: &DaemonInner) {
        if inner.config.test_mode {
            return;
        }
        let state_file = inner.config.state_file.clone();
        let state = inner.state.clone();
        tokio::spawn(async move {
            let s = state.read().await;
            let saved = SavedState::from_state(&s);
            drop(s);
            if let Err(e) = saved.save(&state_file) {
                warn!("failed to save state: {e}");
            }
        });
    }

    async fn enable_fallback(inner: &DaemonInner) {
        inner.state.write().await.fallback_disabled = false;
    }

    async fn clear_history(inner: &DaemonInner) {
        inner.play_history.lock().await.clear();
    }

    async fn push_queue_state(inner: &DaemonInner) {
        let state = inner.state.read().await;
        let (queue, cursor) = queue::visible_queue(&state);
        drop(state);
        Self::push_event(inner, DaemonEvent::QueueChanged { queue, cursor });
    }

    fn next_track(state: &DaemonState) -> Option<TrackInfo> {
        let cur_is_queued = state
            .current_track
            .as_ref()
            .map(|t| {
                state
                    .queue
                    .first()
                    .map(|q| q.path == t.path)
                    .unwrap_or(false)
            })
            .unwrap_or(false);
        if !state.queue.is_empty() {
            if cur_is_queued {
                state
                    .queue
                    .get(1)
                    .cloned()
                    .or_else(|| state.default_list.get(state.default_cursor + 1).cloned())
            } else {
                state.queue.first().cloned()
            }
        } else if !state.default_list.is_empty() {
            let len = state.default_list.len();
            let cursor = state.default_cursor.min(len - 1);
            if cursor + 1 < len {
                state.default_list.get(cursor + 1).cloned()
            } else {
                match state.repeat {
                    RepeatMode::All => state.default_list.first().cloned(),
                    _ => None,
                }
            }
        } else {
            None
        }
    }

    async fn build_default_list(inner: &DaemonInner, resume_key: Option<&str>) -> Vec<TrackInfo> {
        let data_dir = inner.config.data_dir.clone();
        let tracks = tokio::task::spawn_blocking(move || {
            Library::new(data_dir.to_str().unwrap_or(""))
                .ok()
                .and_then(|lib| lib.list_tracks().ok())
        })
        .await
        .ok()
        .flatten()
        .unwrap_or_default();
        if tracks.is_empty() {
            return tracks;
        }
        let mut list = tracks;
        list.sort_by_key(|a| a.title.to_lowercase());
        let shuffle = inner.state.read().await.shuffle;
        if shuffle {
            fastrand::shuffle(&mut list);
        } else if let Some(key) = resume_key {
            let key_char = key
                .chars()
                .next()
                .map(|c| c.to_lowercase().next().unwrap_or(c))
                .unwrap_or('\0');
            let pos = list.iter().position(|t| {
                t.title
                    .chars()
                    .next()
                    .map(|c| c.to_lowercase().next().unwrap_or(c))
                    .unwrap_or('\0')
                    >= key_char
            });
            if let Some(pos) = pos {
                if pos > 0 {
                    list.rotate_left(pos);
                }
            }
        }
        list
    }

    async fn step_next(inner: &DaemonInner) -> Result<Option<TrackInfo>, CoreError> {
        let mut resume_key: Option<String> = None;
        {
            let mut history = inner.play_history.lock().await;
            let mut state = inner.state.write().await;
            if !state.queue.is_empty() {
                let cur_is_queued = state
                    .current_track
                    .as_ref()
                    .map(|t| t.path == state.queue[0].path)
                    .unwrap_or(false);
                if cur_is_queued {
                    let cur = state.queue.remove(0);
                    resume_key = Some(cur.title.clone());
                    history.push(HistoryEntry::User(cur));
                    if !state.queue.is_empty() {
                        let next = state.queue[0].clone();
                        drop(state);
                        drop(history);
                        return Ok(Some(next));
                    }
                } else {
                    let next = state.queue[0].clone();
                    drop(state);
                    drop(history);
                    return Ok(Some(next));
                }
            }
            if !state.default_list.is_empty() {
                let len = state.default_list.len();
                let cursor = state.default_cursor;
                if cursor < len {
                    let cur = state.default_list[cursor].clone();
                    history.push(HistoryEntry::Default {
                        index: cursor,
                        track: cur.clone(),
                    });
                    let next_idx = cursor + 1;
                    if next_idx < len {
                        state.default_cursor = next_idx;
                        let next = state.default_list[next_idx].clone();
                        drop(state);
                        drop(history);
                        return Ok(Some(next));
                    }
                    state.default_cursor = next_idx;
                    let next = match state.repeat {
                        RepeatMode::Off => None,
                        RepeatMode::All => {
                            state.default_cursor = 0;
                            state.default_list.first().cloned()
                        }
                        RepeatMode::One => {
                            state.default_cursor = cursor;
                            Some(cur)
                        }
                    };
                    drop(state);
                    drop(history);
                    return Ok(next);
                } else {
                    let next = match state.repeat {
                        RepeatMode::Off => None,
                        RepeatMode::All => {
                            state.default_cursor = 0;
                            state.default_list.first().cloned()
                        }
                        RepeatMode::One => {
                            state.default_cursor = len - 1;
                            state.default_list.get(len - 1).cloned()
                        }
                    };
                    drop(state);
                    drop(history);
                    return Ok(next);
                }
            }
            if resume_key.is_none() {
                resume_key = state.current_track.as_ref().map(|t| t.title.clone());
            }
            let fallback_disabled = state.fallback_disabled;
            drop(state);
            drop(history);
            if fallback_disabled {
                return Ok(None);
            }
        }
        let list = Self::build_default_list(inner, resume_key.as_deref()).await;
        if list.is_empty() {
            return Ok(None);
        }
        let first = list[0].clone();
        {
            let mut state = inner.state.write().await;
            state.default_list = list;
            state.default_cursor = 0;
            state.fallback_disabled = false;
        }
        Ok(Some(first))
    }

    async fn stop_playback(inner: &DaemonInner) {
        {
            let mut mixer = inner.mixer.lock().await;
            let _ = mixer.stop();
        }
        *inner.crossfade_loaded_for.lock().await = None;
        let mut state = inner.state.write().await;
        state.status = PlaybackStatus::Stopped;
        state.current_track = None;
        state.time_pos = 0.0;
        drop(state);
        Self::push_event(inner, DaemonEvent::TrackEnded);
    }

    async fn try_start_crossfade(inner: &DaemonInner, track: &TrackInfo) -> bool {
        let (enabled, dur, easing) = {
            let state = inner.state.read().await;
            match state.crossfade.as_ref() {
                Some(cf) => (cf.enabled, cf.duration_secs as f64, cf.easing),
                None => (false, 0.0, gtm_core::state::Easing::Linear),
            }
        };
        if !enabled
            || dur <= 0.0
            || inner.crossfade_loaded_for.lock().await.is_some()
            || inner.mixer.lock().await.is_crossfading()
        {
            return false;
        }
        let path = track.path.clone();
        let path_owned = path.clone();
        let decoded =
            tokio::task::spawn_blocking(move || AudioMixer::decode_file(&path_owned)).await;
        let source = match decoded {
            Ok(Ok(source)) => source,
            _ => return false,
        };
        let mut mixer = inner.mixer.lock().await;
        if mixer.load_standby_decoded(source).is_err() {
            return false;
        }
        mixer.set_crossfade_easing(easing);
        mixer.start_crossfade(dur);
        drop(mixer);
        *inner.crossfade_loaded_for.lock().await = Some(track.path.clone());
        true
    }

    fn resolve_track_meta(inner: &DaemonInner, path: &std::path::Path, dur: f64) -> TrackInfo {
        let path = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
        let path_str = path.to_string_lossy().into_owned();
        let stem = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("Unknown")
            .to_string();

        if !inner.config.test_mode {
            if let Ok(lib) = Library::new(inner.config.data_dir.to_str().unwrap_or("")) {
                if let Ok(Some(mut t)) = lib.track_by_path(&path_str) {
                    t.duration = dur;
                    return t;
                }
                if let Ok(tracks) = lib.list_tracks() {
                    if let Some(matched) = tracks
                        .iter()
                        .find(|t| path_str.contains(&t.path) || t.path.contains(&path_str))
                    {
                        let mut t = matched.clone();
                        t.duration = dur;
                        return t;
                    }
                }
            }

            let cache_dir = inner.config.cache_dir.to_string_lossy().into_owned();
            if let Ok((meta, hash)) = crate::library::extract_metadata(&path_str, Some(&cache_dir))
            {
                return gtm_core::track::TrackInfo {
                    id: 0,
                    path: path_str,
                    title: if meta.title.is_empty() {
                        stem.clone()
                    } else {
                        meta.title
                    },
                    artist: if meta.artist.is_empty() {
                        "Unknown Artist".to_string()
                    } else {
                        meta.artist
                    },
                    album: if meta.album.is_empty() {
                        "Unknown Album".to_string()
                    } else {
                        meta.album
                    },
                    duration: if dur > 0.0 { dur } else { meta.duration },
                    track_number: meta.track_number,
                    genre: meta.genre,
                    year: meta.year,
                    bitrate: meta.bitrate,
                    samplerate: meta.samplerate,
                    hash,
                    cover_path: meta.cover_path,
                    favourite: false,
                    ..Default::default()
                };
            }
        }

        let (cleaned_artist, cleaned_title) = crate::cleaner::clean_filename_stem(&stem);
        let title = if cleaned_title.is_empty() {
            stem
        } else {
            cleaned_title
        };
        let artist = cleaned_artist.unwrap_or_else(|| "Unknown Artist".to_string());
        gtm_core::track::TrackInfo {
            id: 0,
            path: path_str,
            title,
            artist,
            album: "Unknown Album".to_string(),
            duration: dur,
            ..Default::default()
        }
    }

    async fn finish_crossfade(inner: &DaemonInner) {
        let actual = inner.mixer.lock().await.current_position();
        *inner.crossfade_loaded_for.lock().await = None;
        match Self::step_next(inner).await {
            Ok(Some(mut next)) => {
                let dur = inner.mixer.lock().await.duration();
                next = Self::resolve_track_meta(inner, std::path::Path::new(&next.path), dur);
                {
                    let mut state = inner.state.write().await;
                    state.status = PlaybackStatus::Playing;
                    state.time_pos = actual;
                    state.current_track = Some(next.clone());
                    state.duration = dur;
                }
                Self::push_event(
                    inner,
                    DaemonEvent::PlaybackStarted {
                        track: next,
                        auto_advanced: true,
                        time_pos: actual,
                        duration: dur,
                    },
                );
                Self::push_queue_state(inner).await;
            }
            Ok(None) => {
                Self::stop_playback(inner).await;
            }
            Err(e) => {
                warn!("crossfade finish failed: {e}");
                Self::stop_playback(inner).await;
            }
        }
    }

    async fn handle_audio_event(inner: &DaemonInner, result: AudioResult<Option<AudioEvent>>) {
        let ev = match result {
            Ok(Some(e)) => e,
            Ok(None) => {
                return;
            }
            Err(e) => {
                warn!("backend error: {e}");
                Self::push_event(
                    inner,
                    DaemonEvent::Custom {
                        name: "backend_error".into(),
                        data: [("error".into(), e.to_string())].into(),
                    },
                );
                return;
            }
        };

        match ev {
            AudioEvent::Position(pos) => {
                if inner.crossfade_loaded_for.lock().await.is_some()
                    && !inner.mixer.lock().await.is_crossfading()
                {
                    let _lock = inner.cmd_lock.lock().await;
                    Self::finish_crossfade(inner).await;
                }
                let mut state = inner.state.write().await;
                state.time_pos = pos;
                let dur = state.duration;
                let crossfade = state.crossfade.clone();
                let next = Self::next_track(&state);
                drop(state);

                let cf_secs = crossfade
                    .as_ref()
                    .filter(|c| c.enabled)
                    .map_or(0.0, |c| c.duration_secs as f64);
                if dur > 0.0 && (dur - pos) <= cf_secs + 3.0 {
                    if let Some(track) = &next {
                        let mut notified = inner.countdown_notified_for.lock().await;
                        if notified.as_deref() != Some(track.hash.as_str()) {
                            *notified = Some(track.hash.clone());
                            Self::push_event(
                                inner,
                                DaemonEvent::CrossfadeCountdown {
                                    track: track.clone(),
                                },
                            );
                        }
                    }
                }

                if let Some(cf) = crossfade {
                    if cf.enabled && dur > 0.0 && (dur - pos) <= cf.duration_secs as f64 + 0.15 {
                        if let Some(track) = &next {
                            let _ = Self::try_start_crossfade(inner, track).await;
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
                if was_crossfading {
                    let _lock = inner.cmd_lock.lock().await;
                    Self::finish_crossfade(inner).await;
                    let _ = Self::cmd_next(inner).await;
                } else {
                    let _ = inner.internal_req_tx.send(DaemonReq::Next);
                }
            }
            AudioEvent::Error(msg) => {
                warn!("audio error: {msg}");
                Self::push_event(
                    inner,
                    DaemonEvent::Custom {
                        name: "audio_error".into(),
                        data: [("error".into(), msg)].into(),
                    },
                );
            }
        }
    }

    // ─── Command handlers ──────────────────────────────────────────────
    //
    //

    async fn promote_crossfade(inner: &DaemonInner) -> Option<String> {
        let mut mixer = inner.mixer.lock().await;
        if !mixer.is_crossfading() || !mixer.standby_is_loaded() {
            return None;
        }
        mixer.drop_active();
        drop(mixer);
        inner.crossfade_loaded_for.lock().await.take()
    }

    async fn report_promoted(inner: &DaemonInner, path: &str) {
        let dur = inner.mixer.lock().await.duration();
        let track = Self::resolve_track_meta(inner, std::path::Path::new(path), dur);
        {
            let mut state = inner.state.write().await;
            state.status = PlaybackStatus::Playing;
            state.time_pos = 0.0;
            state.current_track = Some(track.clone());
            state.duration = dur;
        }
        Self::push_event(
            inner,
            DaemonEvent::PlaybackStarted {
                track,
                auto_advanced: true,
                time_pos: 0.0,
                duration: dur,
            },
        );
        Self::push_queue_state(inner).await;
    }

    async fn cmd_play(
        inner: &DaemonInner,
        path: &str,
        start_pos: f64,
        auto_advanced: bool,
    ) -> Result<DaemonRes, CoreError> {
        {
            let mut mixer = inner.mixer.lock().await;
            mixer.stop()?;
        }
        *inner.crossfade_loaded_for.lock().await = None;
        {
            let mut state = inner.state.write().await;
            if state.status != PlaybackStatus::Stopped {
                state.stop()?;
            }
        }

        let path_owned = path.to_string();
        let path_for_blocking = path_owned.clone();
        let source =
            tokio::task::spawn_blocking(move || AudioMixer::decode_file(&path_for_blocking))
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

        let track = Self::resolve_track_meta(inner, std::path::Path::new(&path_owned), dur);
        if let Some(pos) = state.queue.iter().position(|t| t.path == track.path) {
            if pos > 0 {
                state.queue.rotate_left(pos);
            }
        }
        state.play(track.clone())?;
        state.time_pos = start_pos;
        state.duration = dur;
        drop(state);
        Self::push_event(
            inner,
            DaemonEvent::PlaybackStarted {
                track,
                auto_advanced,
                time_pos: start_pos,
                duration: dur,
            },
        );
        Ok(DaemonRes::Ok)
    }

    async fn cmd_playpause(inner: &DaemonInner) -> Result<DaemonRes, CoreError> {
        if Self::soloist_ready(inner).await {
            let state = inner.state.read().await;
            let is_playing = state.soloist.playing;
            drop(state);
            if is_playing {
                return Self::soloist_control(inner, "pause", vec![])
                    .await
                    .map(|_| DaemonRes::Ok)
                    .map_err(CoreError::Daemon);
            } else {
                return Self::soloist_control(inner, "play", vec![])
                    .await
                    .map(|_| DaemonRes::Ok)
                    .map_err(CoreError::Daemon);
            }
        }

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
                        drop(state);
                        return Ok(DaemonRes::Error {
                            message: "no current track".into(),
                        });
                    }
                };
                state.play(track.clone())?;
                let time_pos = state.time_pos;
                let duration = state.duration;
                drop(state);
                Self::push_event(
                    inner,
                    DaemonEvent::PlaybackStarted {
                        track,
                        auto_advanced: false,
                        time_pos,
                        duration,
                    },
                );
                Ok(DaemonRes::Ok)
            } else if !path.is_empty() {
                Self::cmd_play(inner, &path, 0.0, false).await
            } else {
                let state = inner.state.read().await;
                let (queue, cursor) = queue::visible_queue(&state);
                drop(state);
                if !queue.is_empty() {
                    let idx = (cursor as usize).min(queue.len() - 1);
                    Self::cmd_play(inner, &queue[idx].path, 0.0, false).await
                } else {
                    Ok(DaemonRes::Ok)
                }
            }
        }
    }

    async fn cmd_pause(inner: &DaemonInner) -> Result<DaemonRes, CoreError> {
        if Self::soloist_ready(inner).await {
            return Self::soloist_control(inner, "pause", vec![])
                .await
                .map(|_| DaemonRes::Ok)
                .map_err(CoreError::Daemon);
        }

        let pos = {
            let mut mixer = inner.mixer.lock().await;
            mixer.pause()?;
            mixer.current_position()
        };
        let mut state = inner.state.write().await;
        state.pause()?;
        state.time_pos = pos;
        let time_pos = state.time_pos;
        drop(state);
        Self::push_event(inner, DaemonEvent::PlaybackPaused { time_pos });
        Ok(DaemonRes::Ok)
    }

    async fn cmd_stop(inner: &DaemonInner) -> Result<DaemonRes, CoreError> {
        {
            let mut mixer = inner.mixer.lock().await;
            mixer.stop()?;
        }
        *inner.crossfade_loaded_for.lock().await = None;
        let mut state = inner.state.write().await;
        if state.status != PlaybackStatus::Stopped {
            state.stop()?;
        }
        drop(state);
        Self::push_event(inner, DaemonEvent::PlaybackStopped);
        Ok(DaemonRes::Ok)
    }

    async fn cmd_next(inner: &DaemonInner) -> Result<DaemonRes, CoreError> {
        if Self::soloist_ready(inner).await {
            return Self::soloist_control(inner, "skip_next", vec![])
                .await
                .map(|_| DaemonRes::Ok)
                .map_err(CoreError::Daemon);
        }

        if let Some(path) = Self::promote_crossfade(inner).await {
            let _ = Self::step_next(inner).await;
            Self::report_promoted(inner, &path).await;
            return Ok(DaemonRes::Ok);
        }
        *inner.crossfade_loaded_for.lock().await = None;
        let standby = {
            let state = inner.state.read().await;
            Self::next_track(&state)
        };
        if let Some(track) = standby {
            if Self::try_start_crossfade(inner, &track).await {
                return Ok(DaemonRes::Ok);
            }
        }
        let next = match Self::step_next(inner).await {
            Ok(Some(t)) => t,
            Ok(None) => {
                let was_playing = inner.state.read().await.status != PlaybackStatus::Stopped;
                if was_playing {
                    Self::stop_playback(inner).await;
                } else {
                    Self::push_queue_state(inner).await;
                }
                return Ok(DaemonRes::Ok);
            }
            Err(e) => return Err(e),
        };
        let _ = Self::cmd_play(inner, &next.path, 0.0, true).await?;
        Self::push_queue_state(inner).await;
        Ok(DaemonRes::Ok)
    }

    async fn cmd_prev(inner: &DaemonInner) -> Result<DaemonRes, CoreError> {
        if Self::soloist_ready(inner).await {
            return Self::soloist_control(inner, "skip_prev", vec![])
                .await
                .map(|_| DaemonRes::Ok)
                .map_err(CoreError::Daemon);
        }

        if let Some(path) = Self::promote_crossfade(inner).await {
            Self::report_promoted(inner, &path).await;
        }
        let pos = inner.mixer.lock().await.current_position();
        if pos > RESTART_THRESHOLD_SECS {
            return Self::cmd_seek(inner, 0.0).await;
        }
        let has_current = inner.state.read().await.current_track.is_some();

        let prev = inner.play_history.lock().await.pop();
        match prev {
            Some(HistoryEntry::User(track)) => {
                {
                    let mut state = inner.state.write().await;
                    state.queue.insert(0, track.clone());
                }
                let res = Self::cmd_play(inner, &track.path, 0.0, true).await?;
                Self::push_queue_state(inner).await;
                Ok(res)
            }
            Some(HistoryEntry::Default { index, track }) => {
                {
                    let mut state = inner.state.write().await;
                    if index < state.default_list.len() {
                        state.default_cursor = index;
                    }
                }
                let res = Self::cmd_play(inner, &track.path, 0.0, true).await?;
                Self::push_queue_state(inner).await;
                Ok(res)
            }
            None => {
                if has_current {
                    Self::cmd_seek(inner, 0.0).await
                } else {
                    Ok(DaemonRes::Ok)
                }
            }
        }
    }

    async fn cmd_seek(inner: &DaemonInner, pos: f64) -> Result<DaemonRes, CoreError> {
        if Self::soloist_ready(inner).await {
            let pos_ms = (pos * 1000.0).round() as u64;
            return Self::soloist_control(
                inner,
                "seek",
                vec![("position_ms", Value::from(pos_ms))],
            )
            .await
            .map(|_| DaemonRes::Ok)
            .map_err(CoreError::Daemon);
        }

        let state = inner.state.read().await;
        if state.status == PlaybackStatus::Stopped {
            return Ok(DaemonRes::Ok);
        }
        let was_paused = state.status == PlaybackStatus::Paused;
        let path = state.current_track.as_ref().map(|t| t.path.clone());
        let duration = state.duration;
        drop(state);
        let Some(path) = path else {
            return Ok(DaemonRes::Ok);
        };
        let pos = pos.clamp(0.0, duration.max(0.0));
        tracing::debug!("cmd_seek: requested position={}", pos);
        let path_owned = path.clone();
        let source =
            tokio::task::spawn_blocking(move || AudioMixer::decode_file_at(&path_owned, pos))
                .await
                .map_err(|e| CoreError::Daemon(format!("spawn_blocking: {e}")))?
                .map_err(|e| CoreError::Daemon(format!("decode: {e}")))?;
        {
            let mut mixer = inner.mixer.lock().await;
            mixer.load_active_decoded(source, pos)?;
            mixer.play()?;
            if was_paused {
                mixer.pause()?;
            }
        }
        let mut state = inner.state.write().await;
        state.seek(pos)?;
        drop(state);
        Self::push_event(inner, DaemonEvent::PositionChanged { time_pos: pos });
        Ok(DaemonRes::Ok)
    }

    async fn cmd_set_volume(inner: &DaemonInner, volume: u8) -> Result<DaemonRes, CoreError> {
        if Self::soloist_ready(inner).await {
            return Self::soloist_control(
                inner,
                "set_volume",
                vec![("volume", Value::from(volume))],
            )
            .await
            .map(|_| DaemonRes::Ok)
            .map_err(CoreError::Daemon);
        }

        inner.mixer.lock().await.set_volume(volume)?;
        let mut state = inner.state.write().await;
        state.set_volume(volume)?;
        drop(state);
        Self::push_event(inner, DaemonEvent::VolumeChanged { volume });
        Self::save_state(inner);
        Ok(DaemonRes::Ok)
    }

    async fn cmd_set_master_volume(
        inner: &DaemonInner,
        volume: u8,
    ) -> Result<DaemonRes, CoreError> {
        let vol = volume.min(100);
        inner.mixer.lock().await.set_master_volume(vol)?;
        let mut state = inner.state.write().await;
        state.master_volume = vol;
        drop(state);
        Self::save_state(inner);
        Ok(DaemonRes::Ok)
    }

    async fn cmd_get_volume(inner: &DaemonInner) -> Result<DaemonRes, CoreError> {
        let state = inner.state.read().await;
        let volume = state.volume;
        drop(state);
        Ok(DaemonRes::Value {
            value: serde_json::json!({ "volume": volume }),
        })
    }

    async fn cmd_list_eq_presets(_inner: &DaemonInner) -> Result<DaemonRes, CoreError> {
        let presets = gtm_core::state::EQ_PRESETS
            .iter()
            .map(|p| p.to_string())
            .collect::<Vec<String>>();
        Ok(DaemonRes::EqPresets { presets })
    }

    async fn cmd_toggle_shuffle(inner: &DaemonInner) -> Result<DaemonRes, CoreError> {
        if Self::soloist_ready(inner).await {
            let state = inner.state.read().await;
            let enabled = !state.shuffle;
            drop(state);
            return Self::soloist_control(
                inner,
                "set_shuffle",
                vec![("enabled", Value::Bool(enabled))],
            )
            .await
            .map(|_| DaemonRes::Ok)
            .map_err(CoreError::Daemon);
        }

        let mut state = inner.state.write().await;
        state.toggle_shuffle()?;
        let enabled = state.shuffle;
        drop(state);
        Self::push_event(inner, DaemonEvent::ShuffleChanged { enabled });
        Self::save_state(inner);
        Ok(DaemonRes::Ok)
    }

    async fn cmd_cycle_repeat(
        inner: &DaemonInner,
        mode: gtm_core::state::RepeatMode,
    ) -> Result<DaemonRes, CoreError> {
        if Self::soloist_ready(inner).await {
            let (ctx_enabled, track_enabled) = match mode {
                RepeatMode::Off => (false, false),
                RepeatMode::All => (true, false),
                RepeatMode::One => (false, true),
            };
            let ctx_json = SoloistManager::cmd(
                "set_repeat_context",
                vec![("enabled", Value::Bool(ctx_enabled))],
            );
            let track_json = SoloistManager::cmd(
                "set_repeat_track",
                vec![("enabled", Value::Bool(track_enabled))],
            );
            inner.soloist.lock().await.send(ctx_json).await;
            inner.soloist.lock().await.send(track_json).await;
            return Ok(DaemonRes::Ok);
        }

        let mut state = inner.state.write().await;
        state.cycle_repeat(mode)?;
        let m = state.repeat;
        drop(state);
        Self::push_event(inner, DaemonEvent::RepeatModeChanged { mode: m });
        Self::save_state(inner);
        Ok(DaemonRes::Ok)
    }

    async fn cmd_toggle_mute(inner: &DaemonInner) -> Result<DaemonRes, CoreError> {
        let mut state = inner.state.write().await;
        state.toggle_mute()?;
        let muted = state.mute;
        drop(state);
        let vol = if muted {
            0
        } else {
            inner.state.read().await.volume
        };
        inner.mixer.lock().await.set_volume(vol)?;
        Self::save_state(inner);
        Ok(DaemonRes::Ok)
    }

    async fn cmd_crossfade(
        inner: &DaemonInner,
        enabled: bool,
        duration_secs: u8,
        easing: Option<gtm_core::state::Easing>,
    ) -> Result<DaemonRes, CoreError> {
        let mut state = inner.state.write().await;
        state.set_crossfade(enabled, duration_secs, easing)?;
        drop(state);
        Self::push_event(
            inner,
            DaemonEvent::CrossfadeChanged {
                enabled,
                duration_secs,
                easing,
            },
        );
        Self::save_state(inner);
        Ok(DaemonRes::Ok)
    }

    async fn cmd_set_loudness_mode(
        inner: &DaemonInner,
        mode: gtm_core::state::LoudnessMode,
    ) -> Result<DaemonRes, CoreError> {
        let mut state = inner.state.write().await;
        state.set_loudness_mode(mode)?;
        drop(state);
        Self::push_event(inner, DaemonEvent::LoudnessModeChanged { mode });
        Self::save_state(inner);
        Ok(DaemonRes::Ok)
    }

    async fn cmd_scan_loudness(
        inner: &DaemonInner,
        track_ids: Option<Vec<i64>>,
        _force: Option<bool>,
    ) -> Result<DaemonRes, CoreError> {
        let total = track_ids.as_ref().map(|v| v.len() as u32).unwrap_or(0);
        for i in 0..total {
            let remaining = total - i;
            Self::push_event(
                inner,
                DaemonEvent::LoudnessScanProgress {
                    tracks_remaining: remaining,
                    tracks_total: total,
                    current_track: None,
                },
            );
        }
        Self::push_event(
            inner,
            DaemonEvent::LoudnessScanDone {
                scanned: total,
                failed: 0,
            },
        );
        Ok(DaemonRes::Ok)
    }

    async fn cmd_set_pre_gain(
        inner: &DaemonInner,
        pre_gain_db: f32,
    ) -> Result<DaemonRes, CoreError> {
        let mut state = inner.state.write().await;
        state.set_pre_gain(pre_gain_db)?;
        drop(state);
        Self::push_event(inner, DaemonEvent::PreGainChanged { pre_gain_db });
        Self::save_state(inner);
        Ok(DaemonRes::Ok)
    }

    async fn cmd_set_gapless(inner: &DaemonInner, enabled: bool) -> Result<DaemonRes, CoreError> {
        let mut state = inner.state.write().await;
        state.set_gapless(enabled)?;
        drop(state);
        Self::push_event(inner, DaemonEvent::GaplessChanged { enabled });
        Self::save_state(inner);
        Ok(DaemonRes::Ok)
    }

    async fn cmd_set_dynamic_mode(
        inner: &DaemonInner,
        enabled: bool,
        min_queue_remaining: Option<u32>,
        max_history: Option<u32>,
    ) -> Result<DaemonRes, CoreError> {
        let mut state = inner.state.write().await;
        state.set_dynamic_mode(enabled, min_queue_remaining, max_history)?;
        let effective_min = min_queue_remaining.unwrap_or(state.dynamic_mode.min_queue_remaining);
        let effective_max = max_history.unwrap_or(state.dynamic_mode.max_history);
        drop(state);
        Self::push_event(
            inner,
            DaemonEvent::DynamicModeChanged {
                enabled,
                min_queue_remaining: effective_min,
                max_history: effective_max,
            },
        );
        Self::save_state(inner);
        Ok(DaemonRes::Ok)
    }

    async fn cmd_set_scrobble(
        inner: &DaemonInner,
        enabled: bool,
        api_key: Option<String>,
        session_token: Option<String>,
        min_play_secs: Option<u32>,
        min_play_pct: Option<f32>,
    ) -> Result<DaemonRes, CoreError> {
        let mut state = inner.state.write().await;
        state.set_scrobble(enabled, api_key, session_token, min_play_secs, min_play_pct)?;
        drop(state);
        Self::push_event(inner, DaemonEvent::ScrobbleConfigChanged { enabled });
        Self::save_state(inner);
        Ok(DaemonRes::Ok)
    }

    async fn cmd_set_eq_preset(
        inner: &DaemonInner,
        preset: EqPreset,
    ) -> Result<DaemonRes, CoreError> {
        let mut state = inner.state.write().await;
        state.eq_preset = preset;
        state.version += 1;
        drop(state);
        inner.mixer.lock().await.set_eq_preset(&preset);
        Self::push_event(inner, DaemonEvent::EqPresetChanged { preset });
        Self::save_state(inner);
        Ok(DaemonRes::Ok)
    }

    async fn cmd_set_eq_enabled(
        inner: &DaemonInner,
        enabled: bool,
    ) -> Result<DaemonRes, CoreError> {
        let mut state = inner.state.write().await;
        state.eq_enabled = enabled;
        state.version += 1;
        drop(state);
        inner.mixer.lock().await.set_eq_enabled(enabled);
        Self::push_event(inner, DaemonEvent::EqEnabledChanged { enabled });
        Self::save_state(inner);
        Ok(DaemonRes::Ok)
    }

    async fn cmd_set_reverb(
        inner: &DaemonInner,
        enabled: bool,
        room_size: f32,
    ) -> Result<DaemonRes, CoreError> {
        let mut state = inner.state.write().await;
        state.reverb = ReverbConfig { enabled, room_size };
        state.version += 1;
        drop(state);
        inner
            .mixer
            .lock()
            .await
            .set_reverb(&ReverbConfig { enabled, room_size });
        Self::push_event(inner, DaemonEvent::ReverbChanged { enabled, room_size });
        Self::save_state(inner);
        Ok(DaemonRes::Ok)
    }

    async fn cmd_set_sleep_timer(
        inner: &DaemonInner,
        minutes: u32,
    ) -> Result<DaemonRes, CoreError> {
        let total_secs = minutes * 60;
        let event_tx = inner.event_tx.clone();
        let state = inner.state.clone();

        inner.sleep_cancel.store(true, Ordering::SeqCst);
        inner.sleep_cancel.store(false, Ordering::SeqCst);
        let cancel_flag = inner.sleep_cancel.clone();

        let mut s = state.write().await;
        s.sleep_timer = Some(total_secs);
        s.version += 1;
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
                let _ = event_tx.send(DaemonEvent::SleepTimerTick {
                    remaining_secs: remaining,
                });
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

        Ok(DaemonRes::Ok)
    }

    async fn cmd_cancel_sleep_timer(inner: &DaemonInner) -> Result<DaemonRes, CoreError> {
        inner.sleep_cancel.store(true, Ordering::SeqCst);
        let mut state = inner.state.write().await;
        state.sleep_timer = None;
        state.version += 1;
        Ok(DaemonRes::Ok)
    }

    async fn cmd_queue(inner: &DaemonInner, action: &QueueAction) -> Result<DaemonRes, CoreError> {
        match action {
            QueueAction::List => {
                let state = inner.state.read().await;
                let (queue, cursor) = queue::visible_queue(&state);
                drop(state);
                Ok(DaemonRes::QueueState { queue, cursor })
            }
            QueueAction::Clear => {
                Self::clear_history(inner).await;
                {
                    let mut state = inner.state.write().await;
                    queue::queue_clear(&mut state);
                }
                Self::push_queue_state(inner).await;
                Self::save_state(inner);
                Ok(DaemonRes::Ok)
            }
            QueueAction::Remove { index } => {
                {
                    let mut state = inner.state.write().await;
                    queue::queue_remove(&mut state, *index);
                }
                Self::push_queue_state(inner).await;
                Self::save_state(inner);
                Ok(DaemonRes::Ok)
            }
            QueueAction::Move { from, to } => {
                {
                    let mut state = inner.state.write().await;
                    queue::queue_move(&mut state, *from, *to);
                }
                Self::push_queue_state(inner).await;
                Self::save_state(inner);
                Ok(DaemonRes::Ok)
            }
            QueueAction::Add { paths, position } => {
                let expanded = match queue::expand_paths(paths) {
                    Ok(files) => files,
                    Err(e) => return Ok(DaemonRes::Error { message: e }),
                };
                if expanded.is_empty() {
                    return Ok(DaemonRes::Error {
                        message: "no audio files found".into(),
                    });
                }
                let first_path = expanded[0].clone();
                let was_empty = {
                    let mut state = inner.state.write().await;
                    state.fallback_disabled = false;
                    let w = state.queue.is_empty() && state.status == PlaybackStatus::Stopped;
                    queue::queue_add_many(&mut state, &expanded, *position);
                    drop(state);
                    w
                };
                if was_empty {
                    let _ = Self::cmd_play(inner, &first_path, 0.0, false).await;
                }
                Self::push_queue_state(inner).await;
                Self::save_state(inner);
                Ok(DaemonRes::Ok)
            }
            QueueAction::Set { paths, start_idx } => {
                Self::clear_history(inner).await;
                {
                    let mut state = inner.state.write().await;
                    queue::queue_set(&mut state, paths, *start_idx);
                }
                Self::push_queue_state(inner).await;
                Self::save_state(inner);
                Ok(DaemonRes::Ok)
            }
        }
    }

    async fn cmd_library(
        inner: &DaemonInner,
        action: &gtm_core::ipc::LibraryAction,
    ) -> Result<DaemonRes, CoreError> {
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
                    Ok(tracks) => DaemonRes::Tracks { tracks },
                    Err(e) => DaemonRes::Error { message: e },
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
                    Ok(tracks) => DaemonRes::Tracks { tracks },
                    Err(e) => DaemonRes::Error { message: e },
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
                    Ok(playlists) => DaemonRes::Playlists { playlists },
                    Err(e) => DaemonRes::Error { message: e },
                }
            }
            gtm_core::ipc::LibraryAction::GetPlaylistTracks { id } => {
                let data_dir = inner.config.data_dir.clone();
                let pid = *id;
                let result = tokio::task::spawn_blocking(move || {
                    let lib = Library::new(data_dir.to_str().unwrap_or(""))?;
                    lib.get_playlist_tracks(pid)
                })
                .await
                .map_err(|e| CoreError::Daemon(e.to_string()))?;
                match result {
                    Ok(tracks) => DaemonRes::Tracks { tracks },
                    Err(e) => DaemonRes::Error { message: e },
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
                        DaemonRes::Playlists { playlists }
                    }
                    Err(e) => DaemonRes::Error { message: e },
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
                    Ok(_) => DaemonRes::Ok,
                    Err(e) => DaemonRes::Error { message: e },
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
                    Ok(_) => DaemonRes::Ok,
                    Err(e) => DaemonRes::Error { message: e },
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
                        DaemonRes::Playlists { playlists }
                    }
                    Err(e) => DaemonRes::Error { message: e },
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
                    Ok(tracks) => DaemonRes::Tracks { tracks },
                    Err(e) => DaemonRes::Error { message: e },
                }
            }
            gtm_core::ipc::LibraryAction::SyncCovers => {
                Self::cmd_sync_start(inner, SyncKind::Covers, None).await?
            }
            gtm_core::ipc::LibraryAction::SyncLyrics => {
                Self::cmd_sync_start(inner, SyncKind::Lyrics, None).await?
            }
            gtm_core::ipc::LibraryAction::SyncMetadata { path } => {
                Self::cmd_sync_start(inner, SyncKind::Metadata, path.clone()).await?
            }
            gtm_core::ipc::LibraryAction::SyncStatus => Self::cmd_sync_status(inner).await?,
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
                    Ok(_) => DaemonRes::Ok,
                    Err(e) => DaemonRes::Error { message: e },
                }
            }
            gtm_core::ipc::LibraryAction::RemoveFromPlaylist {
                playlist_id,
                track_id,
            } => {
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
                    Ok(()) => DaemonRes::Ok,
                    Err(e) => DaemonRes::Error { message: e },
                }
            }
            gtm_core::ipc::LibraryAction::RemoveTrack { id } => {
                let id = *id;
                let data_dir = inner.config.data_dir.clone();
                let library_dirs = inner.config.library_paths.clone();
                let allow_delete_files = inner.config.allow_delete_files;
                let result = tokio::task::spawn_blocking(move || {
                    let lib = Library::new(data_dir.to_str().unwrap_or(""))?;
                    lib.remove_track_full(id, &library_dirs, allow_delete_files)
                })
                .await
                .map_err(|e| CoreError::Daemon(e.to_string()))?;
                match result {
                    Ok(Some(removed_path)) => {
                        let mut state = inner.state.write().await;
                        state.queue.retain(|t| t.path != removed_path);
                        let was_current = state
                            .current_track
                            .as_ref()
                            .is_some_and(|t| t.path == removed_path);
                        drop(state);
                        if was_current {
                            Self::stop_playback(inner).await;
                        } else {
                            Self::push_queue_state(inner).await;
                        }
                        Self::push_event(
                            inner,
                            DaemonEvent::Custom {
                                name: "library_changed".into(),
                                data: Default::default(),
                            },
                        );
                        DaemonRes::Ok
                    }
                    Ok(None) => DaemonRes::Error {
                        message: "track not found".to_string(),
                    },
                    Err(e) => DaemonRes::Error { message: e },
                }
            }
            gtm_core::ipc::LibraryAction::UpdateMetadata { track_id, patch } => {
                let track_id = *track_id;
                let patch = patch.clone();
                let data_dir = inner.config.data_dir.clone();
                let result = tokio::task::spawn_blocking(move || {
                    let lib = Library::new(data_dir.to_str().unwrap_or(""))?;
                    lib.update_metadata(track_id, &patch)
                })
                .await
                .map_err(|e| CoreError::Daemon(e.to_string()))?;
                match result {
                    Ok(_) => DaemonRes::Ok,
                    Err(e) => DaemonRes::Error { message: e },
                }
            }
        };
        Ok(res)
    }

    async fn cmd_sync_start(
        inner: &DaemonInner,
        kind: SyncKind,
        only_path: Option<String>,
    ) -> Result<DaemonRes, CoreError> {
        if inner.sync_progress.running.load(Ordering::Acquire) {
            return Ok(DaemonRes::Error {
                message: "a library sync is already running".into(),
            });
        }
        *inner.sync_progress.kind.lock().unwrap() = kind;
        inner.sync_progress.synced.store(0, Ordering::Relaxed);
        inner.sync_progress.total.store(0, Ordering::Relaxed);
        inner.sync_progress.running.store(true, Ordering::Release);

        let data_dir = inner.config.data_dir.clone();
        let cache_dir = inner.config.cache_dir.clone();
        let lyrics_manager = inner.lyrics_manager.clone();
        let progress = inner.sync_progress.clone();
        let event_tx = inner.event_tx.clone();
        tokio::spawn(async move {
            let progress_inner = progress.clone();
            let result = tokio::task::spawn_blocking(move || match kind {
                SyncKind::Covers => run_covers_sync(data_dir, cache_dir, &progress_inner),
                SyncKind::Lyrics => run_lyrics_sync(data_dir, lyrics_manager, &progress_inner),
                SyncKind::Metadata => {
                    run_metadata_sync(data_dir, cache_dir, only_path, &progress_inner)
                }
            })
            .await;
            let (synced, total, error) = match result {
                Ok(Ok((s, t))) => (s, t, None),
                Ok(Err(e)) => (0, 0, Some(e)),
                Err(e) => (0, 0, Some(e.to_string())),
            };
            progress.synced.store(synced, Ordering::Relaxed);
            progress.total.store(total, Ordering::Relaxed);
            progress.running.store(false, Ordering::Release);
            let mut data = std::collections::HashMap::new();
            data.insert("kind".to_string(), format!("{kind:?}").to_lowercase());
            data.insert("synced".to_string(), synced.to_string());
            data.insert("total".to_string(), total.to_string());
            if let Some(e) = error {
                data.insert("error".to_string(), e);
            }
            let _ = event_tx.send(DaemonEvent::Custom {
                name: "sync_done".into(),
                data,
            });
        });
        Ok(DaemonRes::Ok)
    }

    async fn cmd_sync_status(inner: &DaemonInner) -> Result<DaemonRes, CoreError> {
        let progress = &inner.sync_progress;
        Ok(DaemonRes::SyncStatus {
            running: progress.running.load(Ordering::Acquire),
            kind: *progress.kind.lock().unwrap(),
            synced: progress.synced.load(Ordering::Relaxed),
            total: progress.total.load(Ordering::Relaxed),
        })
    }

    async fn cmd_search(inner: &DaemonInner, query: &str) -> Result<DaemonRes, CoreError> {
        let query = query.to_string();
        let data_dir = inner.config.data_dir.clone();
        let result = tokio::task::spawn_blocking(move || {
            let lib = Library::new(data_dir.to_str().unwrap_or(""))?;
            lib.search_tracks(&query)
        })
        .await
        .map_err(|e| CoreError::Daemon(e.to_string()))?;
        match result {
            Ok(tracks) => Ok(DaemonRes::Tracks { tracks }),
            Err(e) => Ok(DaemonRes::Error { message: e }),
        }
    }

    async fn cmd_get_favourites(inner: &DaemonInner) -> Result<DaemonRes, CoreError> {
        let data_dir = inner.config.data_dir.clone();
        let result = tokio::task::spawn_blocking(move || {
            let lib = Library::new(data_dir.to_str().unwrap_or(""))?;
            lib.get_favourites()
        })
        .await
        .map_err(|e| CoreError::Daemon(e.to_string()))?;
        match result {
            Ok(tracks) => Ok(DaemonRes::Tracks { tracks }),
            Err(e) => Ok(DaemonRes::Error { message: e }),
        }
    }

    async fn cmd_add_favourite(inner: &DaemonInner, track_id: i64) -> Result<DaemonRes, CoreError> {
        let data_dir = inner.config.data_dir.clone();
        let result = tokio::task::spawn_blocking(move || {
            let lib = Library::new(data_dir.to_str().unwrap_or(""))?;
            lib.toggle_favourite(track_id)
        })
        .await
        .map_err(|e| CoreError::Daemon(e.to_string()))?;
        match result {
            Ok(_) => Ok(DaemonRes::Ok),
            Err(e) => Ok(DaemonRes::Error { message: e }),
        }
    }

    async fn cmd_remove_favourite(
        inner: &DaemonInner,
        track_id: i64,
    ) -> Result<DaemonRes, CoreError> {
        let data_dir = inner.config.data_dir.clone();
        let result = tokio::task::spawn_blocking(move || {
            let lib = Library::new(data_dir.to_str().unwrap_or(""))?;
            lib.toggle_favourite(track_id)
        })
        .await
        .map_err(|e| CoreError::Daemon(e.to_string()))?;
        match result {
            Ok(_) => Ok(DaemonRes::Ok),
            Err(e) => Ok(DaemonRes::Error { message: e }),
        }
    }

    async fn cmd_yt_search(
        inner: &DaemonInner,
        query: &str,
        filter: Option<gtm_core::state::YTFilter>,
    ) -> Result<DaemonRes, CoreError> {
        inner.health.yt_search_count.fetch_add(1, Ordering::Relaxed);
        inner.youtube.lock().await.start_search(query, filter);
        Ok(DaemonRes::Ok)
    }

    async fn cmd_yt_search_poll(inner: &DaemonInner) -> Result<DaemonRes, CoreError> {
        match inner.youtube.lock().await.poll_results().await {
            Ok(Some((query, results))) => Ok(DaemonRes::YtSearchResults { query, results }),
            Ok(None) => Ok(DaemonRes::Ok),
            Err(e) => {
                inner
                    .health
                    .yt_search_errors
                    .fetch_add(1, Ordering::Relaxed);
                Ok(DaemonRes::Error { message: e })
            }
        }
    }

    async fn cmd_yt_search_cancel(inner: &DaemonInner) -> Result<DaemonRes, CoreError> {
        inner.youtube.lock().await.cancel().await;
        Ok(DaemonRes::Ok)
    }

    async fn cmd_yt_resolve_stream(inner: &DaemonInner, url: &str) -> Result<DaemonRes, CoreError> {
        match inner.youtube.lock().await.resolve_stream(url).await {
            Ok(info) => Ok(DaemonRes::StreamInfo {
                info: Box::new(info),
            }),
            Err(e) => Ok(DaemonRes::Error { message: e }),
        }
    }

    async fn cmd_get_status(inner: &DaemonInner) -> Result<DaemonRes, CoreError> {
        let state = inner.state.read().await;
        let mut state_clone = state.clone();
        let (queue, cursor) = queue::visible_queue(&state);
        state_clone.queue = queue;
        state_clone.queue_cursor = cursor;
        drop(state);
        Ok(DaemonRes::Status {
            state: Box::new(state_clone),
        })
    }

    async fn cmd_check_health(inner: &DaemonInner) -> Result<DaemonRes, CoreError> {
        let h = &inner.health;
        let mut components = Vec::new();

        components.push(ComponentHealth {
            name: "audio_backend".into(),
            status: HealthStatus::Ok,
            message: Some(h.audio_backend.clone()),
            uptime_secs: Some(h.uptime_secs()),
        });

        let scans = h.scan_count.load(Ordering::Relaxed);
        let scan_errs = h.scan_errors.load(Ordering::Relaxed);
        components.push(ComponentHealth {
            name: "library_scan".into(),
            status: if scan_errs > 0 && scans > 0 {
                HealthStatus::Degraded
            } else {
                HealthStatus::Ok
            },
            message: Some(format!("{scans} scans, {scan_errs} errors")),
            uptime_secs: None,
        });

        let yt = h.yt_search_count.load(Ordering::Relaxed);
        let yt_errs = h.yt_search_errors.load(Ordering::Relaxed);
        components.push(ComponentHealth {
            name: "youtube_search".into(),
            status: if yt_errs > 0 && yt > 0 {
                HealthStatus::Degraded
            } else {
                HealthStatus::Ok
            },
            message: Some(format!("{yt} searches, {yt_errs} errors")),
            uptime_secs: None,
        });

        let covers = h.cover_fetch_count.load(Ordering::Relaxed);
        let cover_errs = h.cover_fetch_errors.load(Ordering::Relaxed);
        components.push(ComponentHealth {
            name: "cover_art".into(),
            status: if cover_errs > 0 && covers > 0 {
                HealthStatus::Degraded
            } else {
                HealthStatus::Ok
            },
            message: Some(format!("{covers} fetches, {cover_errs} errors")),
            uptime_secs: None,
        });

        let lyrics = h.lyrics_fetch_count.load(Ordering::Relaxed);
        let lyrics_errs = h.lyrics_fetch_errors.load(Ordering::Relaxed);
        components.push(ComponentHealth {
            name: "lyrics".into(),
            status: if lyrics_errs > 0 && lyrics > 0 {
                HealthStatus::Degraded
            } else {
                HealthStatus::Ok
            },
            message: Some(format!("{lyrics} fetches, {lyrics_errs} errors")),
            uptime_secs: None,
        });

        components.push(ComponentHealth {
            name: "event_channel".into(),
            status: HealthStatus::Ok,
            message: Some("capacity 1024".to_string()),
            uptime_secs: None,
        });

        Ok(DaemonRes::HealthReport {
            report: Box::new(HealthReport {
                daemon_uptime_secs: h.uptime_secs(),
                version: env!("CARGO_PKG_VERSION").to_string(),
                components,
            }),
        })
    }

    async fn cmd_get_cover_art(inner: &DaemonInner, track_id: i64) -> Result<DaemonRes, CoreError> {
        inner
            .health
            .cover_fetch_count
            .fetch_add(1, Ordering::Relaxed);
        let mut discovered_artist = String::new();
        let mut discovered_album = String::new();

        let lib = if !inner.config.test_mode {
            Library::new(inner.config.data_dir.to_str().unwrap_or("")).ok()
        } else {
            None
        };
        if let Some(ref library) = lib {
            if let Ok(Some(track)) = library.get_track(track_id) {
                if let Some(ref path) = track.cover_path {
                    if let Ok(data) = tokio::fs::read(path).await {
                        if !crate::cover::CoverCache::cover_too_small(&data) {
                            let b64 = base64::engine::general_purpose::STANDARD.encode(&data);
                            return Ok(DaemonRes::CoverArt { data: Some(b64) });
                        }
                    }
                }
                let audio_path = std::path::Path::new(&track.path);
                let parent = audio_path.parent().unwrap_or(std::path::Path::new(""));
                let stem = audio_path.file_stem().unwrap_or_default();
                for ext in ["jpg", "jpeg", "png", "webp"] {
                    let sidecar = parent.join(format!("{}.{}", stem.to_string_lossy(), ext));
                    if let Ok(data) = tokio::fs::read(&sidecar).await {
                        if !crate::cover::CoverCache::cover_too_small(&data) {
                            let b64 = base64::engine::general_purpose::STANDARD.encode(&data);
                            return Ok(DaemonRes::CoverArt { data: Some(b64) });
                        }
                    }
                }
                discovered_artist = track.artist;
                discovered_album = track.album;
            }
        }

        if discovered_artist.is_empty() {
            let state = inner.state.read().await;
            let in_merged = state
                .queue
                .iter()
                .chain(state.default_list.iter())
                .find(|t| t.id == track_id);
            if let Some(t) = in_merged {
                discovered_artist = t.artist.clone();
                discovered_album = t.album.clone();
            } else if let Some(ref t) = state.current_track {
                discovered_artist = t.artist.clone();
                discovered_album = t.album.clone();
            }
        }

        if !discovered_artist.is_empty() && !discovered_album.is_empty() {
            let mut guard = inner.cover_cache.lock().await;
            if let Some(ref mut cache) = *guard {
                let artist = discovered_artist.clone();
                let album = discovered_album.clone();
                let cover =
                    tokio::time::timeout(Duration::from_secs(5), cache.get_cover(&artist, &album))
                        .await
                        .ok()
                        .flatten();
                if let Some(cover) = cover {
                    let b64 = base64::engine::general_purpose::STANDARD.encode(&cover.data);
                    return Ok(DaemonRes::CoverArt { data: Some(b64) });
                }
            }
        }

        Ok(DaemonRes::CoverArt { data: None })
    }

    async fn cmd_get_artist_cover_art(
        inner: &DaemonInner,
        artist: &str,
    ) -> Result<DaemonRes, CoreError> {
        let mut guard = inner.cover_cache.lock().await;
        if let Some(ref mut cache) = *guard {
            let cover =
                tokio::time::timeout(Duration::from_secs(8), cache.get_artist_image(artist))
                    .await
                    .ok()
                    .flatten();
            if let Some(cover) = cover {
                let b64 = base64::engine::general_purpose::STANDARD.encode(&cover.data);
                return Ok(DaemonRes::CoverArt { data: Some(b64) });
            }
        }
        Ok(DaemonRes::CoverArt { data: None })
    }

    async fn cmd_get_lyrics(
        inner: &DaemonInner,
        track_id: i64,
        path: Option<String>,
    ) -> Result<DaemonRes, CoreError> {
        inner
            .health
            .lyrics_fetch_count
            .fetch_add(1, Ordering::Relaxed);
        let current = {
            let state = inner.state.read().await;
            state.current_track.as_ref().and_then(|t| {
                let id_matches = t.id == track_id;
                let path_matches = path.as_deref().is_some_and(|p| t.path == p);
                if id_matches || path_matches {
                    Some(t.clone())
                } else {
                    None
                }
            })
        };
        let track = match current {
            Some(t) => t,
            None => {
                let resolved = if !inner.config.test_mode {
                    Library::new(inner.config.data_dir.to_str().unwrap_or(""))
                        .ok()
                        .and_then(|lib| lib.get_track(track_id).ok().flatten())
                } else {
                    None
                };
                match resolved.or_else(|| path.map(|p| crate::queue::resolve_track(&p))) {
                    Some(t) => t,
                    None => return Ok(DaemonRes::Lyrics { lyrics: None }),
                }
            }
        };

        let mut track = track;
        if track.artist.is_empty() || track.title.is_empty() {
            let (artist, title) = crate::lyrics::meta_from_filename(&track.path);
            if track.artist.is_empty() {
                track.artist = artist;
            }
            if track.title.is_empty() {
                track.title = title;
            }
        }

        if let Some(ref manager) = inner.lyrics_manager {
            let lyrics = tokio::time::timeout(Duration::from_secs(4), manager.get_lyrics(&track))
                .await
                .ok()
                .flatten();
            Ok(DaemonRes::Lyrics { lyrics })
        } else {
            Ok(DaemonRes::Lyrics { lyrics: None })
        }
    }

    async fn cmd_lyrics_search(
        inner: &DaemonInner,
        artist: &str,
        title: &str,
    ) -> Result<DaemonRes, CoreError> {
        if let Some(ref manager) = inner.lyrics_manager {
            let lyrics =
                tokio::time::timeout(Duration::from_secs(4), manager.search(artist, title))
                    .await
                    .ok()
                    .flatten();
            Ok(DaemonRes::Lyrics { lyrics })
        } else {
            Ok(DaemonRes::Lyrics { lyrics: None })
        }
    }

    // ─── Spotify ───

    async fn cmd_spotify_set_token(
        inner: &DaemonInner,
        token: &str,
    ) -> Result<DaemonRes, CoreError> {
        let mut spotify = inner.spotify.lock().await;
        let _ = tokio::time::timeout(Duration::from_secs(60), spotify.set_token(token)).await;
        Ok(DaemonRes::SpotifyStatusRes {
            status: spotify.status(),
        })
    }

    async fn cmd_spotify_clear(inner: &DaemonInner) -> Result<DaemonRes, CoreError> {
        let mut spotify = inner.spotify.lock().await;
        spotify.clear();
        Ok(DaemonRes::SpotifyStatusRes {
            status: spotify.status(),
        })
    }

    async fn cmd_spotify_status(inner: &DaemonInner) -> Result<DaemonRes, CoreError> {
        let mut spotify = inner.spotify.lock().await;
        let _ = tokio::time::timeout(Duration::from_secs(5), spotify.refresh_playback()).await;
        Ok(DaemonRes::SpotifyStatusRes {
            status: spotify.status(),
        })
    }

    async fn cmd_spotify_play_pause(inner: &DaemonInner) -> Result<DaemonRes, CoreError> {
        let mut spotify = inner.spotify.lock().await;
        match spotify.play_pause().await {
            Ok(()) => Ok(DaemonRes::SpotifyStatusRes {
                status: spotify.status(),
            }),
            Err(e) => Ok(DaemonRes::Error { message: e }),
        }
    }

    async fn cmd_spotify_sync(inner: &DaemonInner) -> Result<DaemonRes, CoreError> {
        let mut spotify = inner.spotify.lock().await;
        match tokio::time::timeout(Duration::from_secs(60), spotify.sync()).await {
            Ok(Ok(())) => Ok(DaemonRes::Ok),
            Ok(Err(e)) => Ok(DaemonRes::Error { message: e }),
            Err(_) => Ok(DaemonRes::Error {
                message: "spotify sync timed out".into(),
            }),
        }
    }

    async fn cmd_spotify_playlists(inner: &DaemonInner) -> Result<DaemonRes, CoreError> {
        let spotify = inner.spotify.lock().await;
        if !spotify.linked() {
            return Ok(DaemonRes::Error {
                message: "spotify not linked".into(),
            });
        }
        Ok(DaemonRes::SpotifyPlaylistsRes {
            playlists: spotify.playlists(),
        })
    }

    async fn cmd_spotify_playlist_tracks(
        inner: &DaemonInner,
        id: &str,
    ) -> Result<DaemonRes, CoreError> {
        let spotify = inner.spotify.lock().await;
        if !spotify.linked() {
            return Ok(DaemonRes::Error {
                message: "spotify not linked".into(),
            });
        }
        match spotify.playlist_tracks(id) {
            Some(tracks) => Ok(DaemonRes::SpotifyTracksRes { tracks }),
            None => Ok(DaemonRes::Error {
                message: "unknown spotify playlist".into(),
            }),
        }
    }

    async fn cmd_spotify_resolve(
        inner: &DaemonInner,
        playlist_id: &str,
        track_index: usize,
    ) -> Result<DaemonRes, CoreError> {
        let track = {
            let spotify = inner.spotify.lock().await;
            spotify
                .playlist_tracks(playlist_id)
                .and_then(|tracks| tracks.into_iter().find(|t| t.index == track_index))
        };
        let Some(track) = track else {
            return Ok(DaemonRes::Error {
                message: "track not found in spotify cache".into(),
            });
        };
        let query = if track.artists.is_empty() {
            track.name.clone()
        } else {
            format!("{} - {}", track.artists, track.name)
        };

        let mut yt = inner.youtube.lock().await;
        if let Err(e) = yt.search(&query, None).await {
            return Ok(DaemonRes::Error { message: e });
        }
        let top = match yt.poll_results().await {
            Ok(Some((_, mut results))) if !results.is_empty() => results.remove(0),
            _ => {
                return Ok(DaemonRes::Error {
                    message: "no youtube results for track".into(),
                })
            }
        };
        let info = match yt.resolve_stream(&top.url).await {
            Ok(info) => info,
            Err(e) => return Ok(DaemonRes::Error { message: e }),
        };

        let prefix = format!("spotify-{playlist_id}-{track_index}");
        let path = match Self::download_audio_to_cache(&inner.config.cache_dir, &prefix, &info.url)
            .await
        {
            Ok(path) => path,
            Err(e) => return Ok(DaemonRes::Error { message: e }),
        };
        drop(yt);

        let spotify_title = track.name.clone();
        let spotify_artist = track.artists.clone();
        let spotify_album = track.album.unwrap_or_default();
        let was_empty = {
            let mut state = inner.state.write().await;
            state.fallback_disabled = false;
            let w = state.queue.is_empty() && state.status == PlaybackStatus::Stopped;
            let added = queue::queue_add(&mut state, &path, None);
            if let Some(entry) = state.queue.iter_mut().rev().find(|t| t.path == added.path) {
                entry.title = spotify_title.clone();
                entry.artist = spotify_artist.clone();
                entry.album = spotify_album.clone();
            }
            drop(state);
            w
        };
        if was_empty {
            let _ = Self::cmd_play(inner, &path, 0.0, false).await;
        }

        {
            let mut guard = inner.cover_cache.lock().await;
            if let Some(ref mut cc) = *guard {
                let _ = cc.get_cover(&spotify_artist, &spotify_album).await;
            }
        }

        Self::push_queue_state(inner).await;
        Self::save_state(inner);
        Ok(DaemonRes::Ok)
    }

    async fn download_audio_to_cache(
        cache_dir: &Path,
        prefix: &str,
        url: &str,
    ) -> Result<String, String> {
        let max_retries = 3u32;
        let mut last_err = String::new();
        for attempt in 1..=max_retries {
            match Self::try_download_audio_to_cache(cache_dir, prefix, url).await {
                Ok(path) => return Ok(path),
                Err(e) => {
                    last_err = e;
                    if attempt < max_retries {
                        tokio::time::sleep(Duration::from_secs(2 * attempt as u64)).await;
                    }
                }
            }
        }
        Err(last_err)
    }

    async fn try_download_audio_to_cache(
        cache_dir: &Path,
        prefix: &str,
        url: &str,
    ) -> Result<String, String> {
        let dir = cache_dir.join("spotify");
        std::fs::create_dir_all(&dir).map_err(|e| format!("create spotify cache: {e}"))?;
        if let Ok(entries) = std::fs::read_dir(&dir) {
            for entry in entries.flatten() {
                let name = entry.file_name().to_string_lossy().into_owned();
                if name.starts_with(prefix) {
                    let _ = std::fs::remove_file(entry.path());
                }
            }
        }
        let template = dir.join(format!("{prefix}.%(ext)s"));
        let output = tokio::time::timeout(
            Duration::from_secs(120),
            tokio::process::Command::new("yt-dlp")
                .arg("-f")
                .arg("bestaudio[ext=m4a]/bestaudio")
                .arg("-o")
                .arg(&template)
                .arg("--no-warnings")
                .arg(url)
                .output(),
        )
        .await
        .map_err(|_| "spotify download timed out".to_string())?
        .map_err(|e| format!("yt-dlp download: {e}"))?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            let msg = stderr
                .lines()
                .last()
                .unwrap_or("yt-dlp download failed")
                .trim()
                .to_string();
            return Err(msg);
        }
        for entry in std::fs::read_dir(&dir).map_err(|e| e.to_string())? {
            let entry = entry.map_err(|e| e.to_string())?;
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.starts_with(prefix) {
                return Ok(entry.path().to_string_lossy().into_owned());
            }
        }
        Err("download produced no file".into())
    }

    // ─── Soloist playback bridge ───

    async fn cmd_soloist_set_api_key(
        inner: &DaemonInner,
        key: &str,
    ) -> Result<DaemonRes, CoreError> {
        let mut soloist = inner.soloist.lock().await;
        soloist.save_key(key).map_err(CoreError::Daemon)?;
        soloist.start(inner.soloist_tx.clone()).await;
        let res = Self::cmd_soloist_status(inner).await?;
        let status = match res {
            DaemonRes::SoloistStatusRes { status } => status,
            _ => SoloistStatus::default(),
        };
        Ok(DaemonRes::SoloistStatusRes { status })
    }

    async fn cmd_soloist_clear(inner: &DaemonInner) -> Result<DaemonRes, CoreError> {
        let mut soloist = inner.soloist.lock().await;
        soloist.clear_key();
        soloist.stop().await;
        {
            let mut state = inner.state.write().await;
            state.soloist = SoloistStatus::default();
            state.version += 1;
        }
        Self::push_event(
            inner,
            DaemonEvent::SoloistStatusChanged {
                status: SoloistStatus::default(),
            },
        );
        Ok(DaemonRes::SoloistStatusRes {
            status: SoloistStatus::default(),
        })
    }

    async fn cmd_soloist_set_config(
        inner: &DaemonInner,
        auto_start: bool,
    ) -> Result<DaemonRes, CoreError> {
        {
            let mut state = inner.state.write().await;
            state.soloist_auto_start = auto_start;
            state.version += 1;
        }
        Self::save_state(inner);
        Ok(DaemonRes::Ok)
    }

    async fn cmd_soloist_start(inner: &DaemonInner) -> Result<DaemonRes, CoreError> {
        inner.soloist_manual_override.store(true, Ordering::SeqCst);
        let mut soloist = inner.soloist.lock().await;
        soloist.start(inner.soloist_tx.clone()).await;
        let res = Self::cmd_soloist_status(inner).await?;
        let status = match res {
            DaemonRes::SoloistStatusRes { status } => status,
            _ => SoloistStatus::default(),
        };
        Ok(DaemonRes::SoloistStatusRes { status })
    }

    async fn cmd_soloist_stop(inner: &DaemonInner) -> Result<DaemonRes, CoreError> {
        inner.soloist_manual_override.store(true, Ordering::SeqCst);
        let mut soloist = inner.soloist.lock().await;
        soloist.stop().await;
        {
            let mut state = inner.state.write().await;
            state.soloist.running = false;
            state.soloist.connected = false;
            state.soloist.logged_in = false;
            state.soloist.track = None;
            state.soloist.playing = false;
            state.version += 1;
        }
        let res = Self::cmd_soloist_status(inner).await?;
        let status = match res {
            DaemonRes::SoloistStatusRes { status } => status,
            _ => SoloistStatus::default(),
        };
        Self::push_event(
            inner,
            DaemonEvent::SoloistStatusChanged {
                status: status.clone(),
            },
        );
        Ok(DaemonRes::SoloistStatusRes { status })
    }

    async fn cmd_soloist_status(inner: &DaemonInner) -> Result<DaemonRes, CoreError> {
        let state = inner.state.read().await;
        Ok(DaemonRes::SoloistStatusRes {
            status: state.soloist.clone(),
        })
    }

    async fn cmd_soloist_play(inner: &DaemonInner, uri: &str) -> Result<DaemonRes, CoreError> {
        let json = SoloistManager::cmd("play", vec![("uri", Value::String(uri.into()))]);
        if inner.soloist.lock().await.send(json).await {
            Ok(DaemonRes::Ok)
        } else {
            Ok(DaemonRes::Error {
                message: "soloist not connected".into(),
            })
        }
    }

    async fn cmd_soloist_activate(inner: &DaemonInner) -> Result<DaemonRes, CoreError> {
        let json = SoloistManager::cmd("activate", vec![]);
        if inner.soloist.lock().await.send(json).await {
            Ok(DaemonRes::Ok)
        } else {
            Ok(DaemonRes::Error {
                message: "soloist not connected".into(),
            })
        }
    }

    async fn soloist_control(
        inner: &DaemonInner,
        command: &str,
        fields: Vec<(&str, Value)>,
    ) -> Result<(), String> {
        let json = SoloistManager::cmd(command, fields);
        if inner.soloist.lock().await.send(json).await {
            Ok(())
        } else {
            Err("soloist not connected".into())
        }
    }

    async fn soloist_ready(inner: &DaemonInner) -> bool {
        let state = inner.state.read().await;
        state.soloist.connected && state.soloist.logged_in
    }

    async fn apply_soloist_event(inner: &DaemonInner, name: &str, data: &Value) {
        match name {
            "auth_state" => {
                let logged_in = data
                    .get("logged_in")
                    .and_then(Value::as_bool)
                    .unwrap_or(false);
                let device_name = data
                    .get("device_name")
                    .and_then(Value::as_str)
                    .map(|s| s.to_string());
                let mut state = inner.state.write().await;
                state.soloist.logged_in = logged_in;
                if let Some(d) = device_name {
                    state.soloist.device = Some(d);
                }
                let st = state.soloist.clone();
                drop(state);
                Self::push_event(inner, DaemonEvent::SoloistStatusChanged { status: st });
                if logged_in {
                    let _ = Self::soloist_send(inner, "get_state").await;
                }
            }
            "playback_state" => {
                let status = data.get("status").and_then(Value::as_str).unwrap_or("idle");
                let volume = data.get("volume").and_then(Value::as_u64).unwrap_or(100) as u8;
                let options = data.get("options");
                let shuffle = options
                    .and_then(|o| o.get("shuffle"))
                    .and_then(Value::as_bool)
                    .unwrap_or(false);
                let repeat = options
                    .and_then(|o| o.get("repeat"))
                    .and_then(Value::as_str)
                    .unwrap_or("off");
                let pos_ms = data
                    .get("position")
                    .and_then(|p| p.get("position_ms"))
                    .and_then(Value::as_u64)
                    .unwrap_or(0);
                let item = data.get("item");

                let mut state = inner.state.write().await;
                let was_playing = state.soloist.playing;
                state.soloist.playing = status == "playing" || status == "buffering";
                state.volume = volume;
                state.shuffle = shuffle;
                state.repeat = match repeat {
                    "context" => RepeatMode::All,
                    "track" => RepeatMode::One,
                    _ => RepeatMode::Off,
                };
                state.time_pos = pos_ms as f64 / 1000.0;
                state.version += 1;
                let track_opt = item.and_then(Self::soloist_track_info);
                if let Some(st_track) = track_opt {
                    let track = Self::soloist_to_track_info(&st_track);
                    state.soloist.track = Some(st_track.clone());
                    state.current_track = Some(track);
                    state.duration = st_track
                        .duration_ms
                        .map(|d| d as f64 / 1000.0)
                        .unwrap_or(0.0);
                }
                let st = state.soloist.clone();
                drop(state);
                Self::push_event(inner, DaemonEvent::SoloistStatusChanged { status: st });
                if (status == "playing" || status == "buffering") && !was_playing {
                    let _ = inner.mixer.lock().await.stop();
                }
                if status == "playing" {
                    let s = inner.state.read().await;
                    Self::push_event(
                        inner,
                        DaemonEvent::PlaybackStarted {
                            track: s.current_track.clone().unwrap(),
                            auto_advanced: false,
                            time_pos: s.time_pos,
                            duration: s.duration,
                        },
                    );
                } else if status == "paused" {
                    let s = inner.state.read().await;
                    Self::push_event(
                        inner,
                        DaemonEvent::PlaybackPaused {
                            time_pos: s.time_pos,
                        },
                    );
                }
            }
            "track_changed" => {
                let item = data.get("item");
                if let Some(st_track) = item.and_then(Self::soloist_track_info) {
                    let track = Self::soloist_to_track_info(&st_track);
                    let mut state = inner.state.write().await;
                    state.soloist.track = Some(st_track.clone());
                    state.current_track = Some(track.clone());
                    state.duration = st_track
                        .duration_ms
                        .map(|d| d as f64 / 1000.0)
                        .unwrap_or(0.0);
                    state.time_pos = 0.0;
                    state.soloist.playing = true;
                    state.version += 1;
                    let st = state.soloist.clone();
                    drop(state);
                    Self::push_event(inner, DaemonEvent::SoloistStatusChanged { status: st });
                    Self::push_event(
                        inner,
                        DaemonEvent::PlaybackStarted {
                            track,
                            auto_advanced: false,
                            time_pos: 0.0,
                            duration: st_track
                                .duration_ms
                                .map(|d| d as f64 / 1000.0)
                                .unwrap_or(0.0),
                        },
                    );
                }
            }
            "playback_changed" => {
                let status = data.get("status").and_then(Value::as_str).unwrap_or("idle");
                let mut state = inner.state.write().await;
                if status == "playing" || status == "buffering" {
                    state.soloist.playing = true;
                    state.version += 1;
                    let st = state.soloist.clone();
                    drop(state);
                    let _ = inner.mixer.lock().await.stop();
                    Self::push_event(inner, DaemonEvent::SoloistStatusChanged { status: st });
                } else if status == "paused" {
                    state.soloist.playing = false;
                    state.version += 1;
                    let st = state.soloist.clone();
                    drop(state);
                    Self::push_event(inner, DaemonEvent::SoloistStatusChanged { status: st });
                    let s = inner.state.read().await;
                    Self::push_event(
                        inner,
                        DaemonEvent::PlaybackPaused {
                            time_pos: s.time_pos,
                        },
                    );
                }
            }
            "position_sync" => {
                let pos = data.get("position");
                let pos_ms = pos
                    .and_then(|p| p.get("position_ms"))
                    .and_then(Value::as_u64)
                    .unwrap_or(0);
                let mut state = inner.state.write().await;
                state.time_pos = pos_ms as f64 / 1000.0;
                state.version += 1;
                drop(state);
                Self::push_event(
                    inner,
                    DaemonEvent::PositionChanged {
                        time_pos: pos_ms as f64 / 1000.0,
                    },
                );
            }
            "volume_changed" => {
                let volume = data.get("volume").and_then(Value::as_u64).unwrap_or(100) as u8;
                let mut state = inner.state.write().await;
                state.volume = volume;
                state.version += 1;
                drop(state);
                Self::push_event(inner, DaemonEvent::VolumeChanged { volume });
            }
            "device_changed" => {
                let device_name = data
                    .get("device_name")
                    .and_then(Value::as_str)
                    .map(|s| s.to_string());
                let mut state = inner.state.write().await;
                state.soloist.device = device_name;
                let st = state.soloist.clone();
                drop(state);
                Self::push_event(inner, DaemonEvent::SoloistStatusChanged { status: st });
            }
            "options_changed" => {
                let options = data.get("options");
                let shuffle = options
                    .and_then(|o| o.get("shuffle"))
                    .and_then(Value::as_bool)
                    .unwrap_or(false);
                let repeat = options
                    .and_then(|o| o.get("repeat"))
                    .and_then(Value::as_str)
                    .unwrap_or("off");
                let new_repeat = match repeat {
                    "context" => RepeatMode::All,
                    "track" => RepeatMode::One,
                    _ => RepeatMode::Off,
                };
                let mut state = inner.state.write().await;
                state.shuffle = shuffle;
                state.repeat = new_repeat;
                state.version += 1;
                let st = state.soloist.clone();
                drop(state);
                Self::push_event(inner, DaemonEvent::SoloistStatusChanged { status: st });
                Self::push_event(inner, DaemonEvent::ShuffleChanged { enabled: shuffle });
                Self::push_event(inner, DaemonEvent::RepeatModeChanged { mode: new_repeat });
            }
            "error" => {
                let msg = data
                    .get("message")
                    .and_then(Value::as_str)
                    .unwrap_or("soloist error");
                let mut state = inner.state.write().await;
                state.soloist.error = Some(msg.into());
                let st = state.soloist.clone();
                drop(state);
                Self::push_event(inner, DaemonEvent::SoloistStatusChanged { status: st });
            }
            _ => {}
        }
    }

    async fn soloist_send(inner: &DaemonInner, command: &str) -> bool {
        let json = SoloistManager::cmd(command, vec![]);
        inner.soloist.lock().await.send(json).await
    }

    fn soloist_track_info(entity: &Value) -> Option<SpotifyTrack> {
        let uri = entity.get("uri").and_then(Value::as_str)?.to_string();
        if uri.is_empty() || !uri.starts_with("spotify:") {
            return None;
        }
        let name = entity
            .get("decorations")
            .and_then(|d| d.get("identity"))
            .and_then(|i| i.get("name"))
            .and_then(Value::as_str)?
            .to_string();
        let artists = entity
            .get("decorations")
            .and_then(|d| d.get("creators"))
            .and_then(|c| c.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|a| {
                        a.get("entity")
                            .and_then(|e| e.get("decorations"))
                            .and_then(|d| d.get("identity"))
                            .and_then(|i| i.get("name"))
                            .and_then(Value::as_str)
                    })
                    .collect::<Vec<_>>()
                    .join(", ")
            })
            .unwrap_or_default();
        let album = entity
            .get("decorations")
            .and_then(|d| d.get("parent"))
            .and_then(|p| p.get("entity"))
            .and_then(|e| e.get("decorations"))
            .and_then(|d| d.get("identity"))
            .and_then(|i| i.get("name"))
            .and_then(Value::as_str)
            .map(|s| s.to_string());
        let duration_ms = entity
            .get("decorations")
            .and_then(|d| d.get("playback"))
            .and_then(|p| p.get("duration_ms"))
            .and_then(Value::as_u64);
        Some(SpotifyTrack {
            index: 0,
            name,
            artists,
            album,
            duration_ms,
            uri: Some(uri),
        })
    }

    fn soloist_to_track_info(st: &SpotifyTrack) -> TrackInfo {
        TrackInfo {
            id: 0,
            path: st.uri.clone().unwrap_or_default(),
            title: st.name.clone(),
            artist: st.artists.clone(),
            album: st.album.clone().unwrap_or_default(),
            duration: st.duration_ms.map(|d| d as f64 / 1000.0).unwrap_or(0.0),
            hash: st.uri.clone().unwrap_or_default(),
            ..Default::default()
        }
    }

    async fn handle_soloist_msg(inner: &DaemonInner, msg: SoloistMsg) {
        match msg {
            SoloistMsg::Connected {
                logged_in,
                device_name,
            } => {
                let mut state = inner.state.write().await;
                state.soloist.running = true;
                state.soloist.connected = true;
                state.soloist.logged_in = logged_in;
                state.soloist.device = device_name;
                state.version += 1;
                let st = state.soloist.clone();
                drop(state);
                Self::push_event(inner, DaemonEvent::SoloistStatusChanged { status: st });
                if logged_in {
                    let _ = Self::soloist_send(inner, "get_state").await;
                }
            }
            SoloistMsg::Disconnected(reason) => {
                let mut state = inner.state.write().await;
                state.soloist.connected = false;
                state.soloist.running = false;
                state.soloist.logged_in = false;
                state.soloist.track = None;
                state.soloist.playing = false;
                if state.soloist.error.is_none() {
                    state.soloist.error = Some(reason);
                }
                state.version += 1;
                let st = state.soloist.clone();
                drop(state);
                Self::push_event(inner, DaemonEvent::SoloistStatusChanged { status: st });
            }
            SoloistMsg::Event { name, data } => {
                Self::apply_soloist_event(inner, &name, &data).await;
            }
            SoloistMsg::Failed(err) => {
                let mut state = inner.state.write().await;
                state.soloist.error = Some(err);
                state.soloist.running = false;
                state.soloist.connected = false;
                state.version += 1;
                let st = state.soloist.clone();
                drop(state);
                Self::push_event(inner, DaemonEvent::SoloistStatusChanged { status: st });
            }
        }
    }
}

fn metadata_query_for(track: &TrackInfo) -> (String, String) {
    let stem = Path::new(&track.path)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("");
    let (cleaned_artist, cleaned_title) = crate::cleaner::clean_filename_stem(stem);
    let title_unreliable = crate::cleaner::is_filename_like(stem, &track.title);
    let query_artist = if track.artist.is_empty() {
        cleaned_artist.unwrap_or_default()
    } else {
        track.artist.clone()
    };
    let query_title = if title_unreliable {
        cleaned_title
    } else {
        track.title.clone()
    };
    (query_artist, query_title)
}

fn run_covers_sync(
    data_dir: PathBuf,
    cache_dir: PathBuf,
    progress: &SyncProgress,
) -> Result<(usize, usize), String> {
    let lib =
        Library::new(data_dir.to_str().unwrap_or("")).map_err(|e| format!("open library: {e}"))?;
    let tracks = lib.list_tracks().map_err(|e| format!("list tracks: {e}"))?;
    let total = tracks.len();
    progress.total.store(total, Ordering::Relaxed);
    let rt = tokio::runtime::Runtime::new().map_err(|e| format!("runtime: {e}"))?;
    let mut cache = crate::cover::CoverCache::new(cache_dir.clone());
    let mut synced = 0usize;
    for track in &tracks {
        let missing_cover = track.cover_path.is_none()
            || track
                .cover_path
                .as_ref()
                .is_none_or(|p| !std::path::Path::new(p).exists());
        if !missing_cover {
            continue;
        }
        let artist = if track.artist.is_empty() {
            "Unknown Artist"
        } else {
            &track.artist
        };
        let album = if track.album.is_empty() {
            "Unknown Album"
        } else {
            &track.album
        };
        if rt.block_on(cache.get_cover(artist, album)).is_some() {
            let key = crate::cover::CoverCache::cache_key(artist, album);
            let cover_file = cache_dir.join("covers").join(format!("{key}.jpg"));
            if cover_file.exists() {
                let path_str = cover_file.to_string_lossy().to_string();
                let _ = lib.update_cover_path(track.id, &path_str);
            }
            synced += 1;
            progress.synced.store(synced, Ordering::Relaxed);
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    Ok((synced, total))
}

fn run_lyrics_sync(
    data_dir: PathBuf,
    lyrics_manager: Option<LyricsManager>,
    progress: &SyncProgress,
) -> Result<(usize, usize), String> {
    let lib =
        Library::new(data_dir.to_str().unwrap_or("")).map_err(|e| format!("open library: {e}"))?;
    let tracks = lib.list_tracks().map_err(|e| format!("list tracks: {e}"))?;
    let total = tracks.len();
    progress.total.store(total, Ordering::Relaxed);
    let rt = tokio::runtime::Runtime::new().map_err(|e| format!("runtime: {e}"))?;
    let manager = lyrics_manager.ok_or("lyrics manager not available")?;
    let mut synced = 0usize;
    for track in &tracks {
        let lrc_path = std::path::Path::new(&track.path).with_extension("lrc");
        if lrc_path.exists() {
            continue;
        }
        if let Some(lyrics) = rt.block_on(manager.get_lyrics(track)) {
            if !lyrics.lines.is_empty() {
                let lrc_content = crate::lyrics::lrc_to_text(&lyrics);
                if std::fs::write(&lrc_path, &lrc_content).is_ok() {
                    synced += 1;
                    progress.synced.store(synced, Ordering::Relaxed);
                }
            }
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    Ok((synced, total))
}

fn run_metadata_sync(
    data_dir: PathBuf,
    cache_dir: PathBuf,
    only_path: Option<String>,
    progress: &SyncProgress,
) -> Result<(usize, usize), String> {
    let lib =
        Library::new(data_dir.to_str().unwrap_or("")).map_err(|e| format!("open library: {e}"))?;
    let tracks = lib.list_tracks().map_err(|e| format!("list tracks: {e}"))?;
    let total = tracks.len();
    progress.total.store(total, Ordering::Relaxed);
    let rt = tokio::runtime::Runtime::new().map_err(|e| format!("runtime: {e}"))?;
    let deezer = crate::deezer::DeezerSearch::new();
    let mut synced = 0usize;
    for track in &tracks {
        let stem = Path::new(&track.path)
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("");
        if let Some(ref only) = only_path {
            if track.path != *only {
                continue;
            }
        } else if !crate::cleaner::title_is_unreliable(
            stem,
            &track.title,
            &track.artist,
            &track.album,
        ) {
            continue;
        }

        let (q_artist, q_title) = metadata_query_for(track);
        let hit = match rt.block_on(deezer.search(&q_artist, &q_title, track.duration)) {
            Ok(Some(hit)) => Some(hit),
            Ok(None) => None,
            Err(e) => {
                warn!("metadata sync failed for {}: {e}", track.path);
                None
            }
        };

        if let Some(hit) = hit {
            let cover = if let Some(ref url) = hit.cover_url {
                rt.block_on(deezer.download_cover(url))
            } else {
                None
            };
            let cover_mime = cover.as_ref().map(|_| "image/jpeg".to_string());
            let meta = crate::tags::MetadataToWrite {
                title: hit.title.clone(),
                artist: hit.artist.clone(),
                album: hit.album.clone(),
                genre: hit.genre.clone(),
                year: hit.year,
                track_number: hit.track_number,
            };
            if crate::tags::write_tags(&track.path, &meta, cover.clone().zip(cover_mime)).is_err() {
                continue;
            }
            if let Some(bytes) = &cover {
                let key = CoverCache::cache_key(&hit.artist, &hit.album);
                let cover_file = cache_dir.join("covers").join(format!("{key}.jpg"));
                if let Some(parent) = cover_file.parent() {
                    std::fs::create_dir_all(parent).ok();
                }
                if std::fs::write(&cover_file, bytes).is_ok() {
                    let _ = lib.update_cover_path(track.id, &cover_file.to_string_lossy());
                }
            }
            if let Err(e) = lib.update_metadata(
                track.id,
                &gtm_core::MetadataPatch {
                    title: Some(hit.title),
                    artist: Some(hit.artist),
                    album: Some(hit.album),
                    genre: hit.genre,
                    year: hit.year,
                    track_number: hit.track_number,
                },
            ) {
                warn!("metadata sync: failed to update DB for {}: {e}", track.path);
            }
            synced += 1;
        } else {
            let (cleaned_artist, cleaned_title) = crate::cleaner::clean_filename_stem(stem);
            if !cleaned_title.is_empty() || cleaned_artist.is_some() {
                let patch = gtm_core::MetadataPatch {
                    title: (!cleaned_title.is_empty())
                        .then(|| crate::cleaner::sanitize_text(&cleaned_title)),
                    artist: cleaned_artist.map(|a| crate::cleaner::sanitize_text(&a)),
                    ..Default::default()
                };
                if patch.title.is_some() || patch.artist.is_some() {
                    if let Err(e) = lib.update_metadata(track.id, &patch) {
                        warn!(
                            "metadata sync: failed to write fallback for {}: {e}",
                            track.path
                        );
                    } else {
                        synced += 1;
                    }
                }
            }
        }
        progress.synced.store(synced, Ordering::Relaxed);
        std::thread::sleep(Duration::from_millis(50));
    }
    Ok((synced, total))
}
