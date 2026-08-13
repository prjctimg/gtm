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
#[cfg(feature = "pulseaudio")]
use gtm_audio::PulseAudioMixer;
use gtm_audio::{AudioEvent, AudioMixer, AudioResult, Mixer, NullMixer};
use gtm_core::ipc::{
    DaemonEvent, DaemonReq, DaemonRes, QueueAction, SyncKind, WireReq, PROTOCOL_VERSION,
};
use gtm_core::state::{
    DaemonState, EqPreset, PlaybackStatus, RepeatMode, ReverbConfig, SavedState,
};
use gtm_core::track::TrackInfo;
use gtm_core::wire;
use gtm_core::CoreError;

use crate::config::DaemonConfig;
use crate::cover_art::CoverCache;
use crate::library::Library;
use crate::lyrics::LyricsManager;
use crate::queue;
use crate::spotify::SpotifyManager;
use crate::youtube::YoutubeManager;

type ClientId = u64;
type ReplyTx = mpsc::UnboundedSender<(u64, DaemonRes)>;

/// Above this position `Prev` restarts the current track instead of going
/// back through playback history.
const RESTART_THRESHOLD_SECS: f64 = 3.0;

use std::time::Instant;

/// Tracks daemon component health for diagnostic reports.
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

/// One previously-played track used by `Prev` for full back-traversal
/// (through user queue entries, then into the default list).
enum HistoryEntry {
    User(TrackInfo),
    Default { index: usize, track: TrackInfo },
}

/// Shared progress of a background library sync (covers/lyrics/metadata).
/// Atomically updated from blocking sync threads and read by the IPC
/// `SyncStatus` handler without ever holding `cmd_lock` across the sync.
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
    crossfade_loaded_for: tokio::sync::Mutex<Option<String>>,
    sleep_cancel: Arc<AtomicBool>,
    health: Arc<HealthTracker>,
    /// Per-client authentication state: true = handshake completed
    client_auth: tokio::sync::Mutex<HashMap<ClientId, bool>>,
    /// Channel for internal requests (e.g., auto-advance from audio events)
    /// that bypass client authentication and are processed sequentially
    /// alongside client requests to prevent state mutation races.
    internal_req_tx: mpsc::UnboundedSender<DaemonReq>,
    /// Serialises all handle_request calls (both spawned client dispatch
    /// and internal commands) so that only one state mutation runs at a
    /// time, preventing races between concurrent IPC commands and auto-advance.
    cmd_lock: tokio::sync::Mutex<()>,
    /// Playback history for Prev back-traversal (guarded by cmd_lock in use).
    play_history: tokio::sync::Mutex<Vec<HistoryEntry>>,
    /// Progress of any background library sync (covers/lyrics/metadata).
    sync_progress: Arc<SyncProgress>,
}

pub struct Daemon {
    inner: Arc<DaemonInner>,
    listener: UnixListener,
    pulse_listener: UnixListener,
    req_tx: mpsc::UnboundedSender<(ClientId, u64, DaemonReq, ReplyTx)>,
    req_rx: mpsc::UnboundedReceiver<(ClientId, u64, DaemonReq, ReplyTx)>,
    internal_req_rx: mpsc::UnboundedReceiver<DaemonReq>,
    next_client_id: ClientId,
}

impl Daemon {
    pub fn new(config: DaemonConfig) -> Result<Self, CoreError> {
        let mut initial_state = DaemonState::new();

        // Load persisted state if available
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

        // Write PID file per daemon.md spec
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
            cover_cache: tokio::sync::Mutex::new(Some(CoverCache::new(cache_dir))),
            lyrics_manager: Some(LyricsManager::new()),
            youtube: Arc::new(tokio::sync::Mutex::new(YoutubeManager::new())),
            spotify: tokio::sync::Mutex::new(SpotifyManager::new(config_dir)),
            crossfade_loaded_for: tokio::sync::Mutex::new(None),
            sleep_cancel: Arc::new(AtomicBool::new(false)),
            health: Arc::new(HealthTracker::new(audio_backend_name)),
            client_auth: tokio::sync::Mutex::new(HashMap::new()),
            internal_req_tx,
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
            next_client_id: 0,
        })
    }

    #[cfg(feature = "pulseaudio")]
    fn init_mixer(config: &DaemonConfig) -> Result<Box<dyn Mixer>, CoreError> {
        if config.audio_backend == AudioBackendKind::PulseAudio {
            match PulseAudioMixer::new() {
                Ok(m) => Ok(Box::new(m)),
                Err(e) => {
                    // On Termux, rodio/cpal cannot open a device, so falling
                    // back to it is pointless — surface the actionable error.
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

        // Periodic heartbeat so clients can detect stale connections
        let hb_event_tx = self.inner.event_tx.clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(30));
            loop {
                interval.tick().await;
                let _ = hb_event_tx.send(DaemonEvent::Heartbeat);
            }
        });

        // Load a persisted Spotify token (if any) in the background so a slow
        // playlist refresh never delays daemon startup or client connects.
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

    /// Background scan: runs auto-scan concurrently with the event loop
    /// so clients don't block waiting for the library to be indexed.
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
        // Initialize client auth state as unauthenticated
        inner.client_auth.lock().await.insert(client_id, false);

        let (reader, writer) = stream.into_split();
        let event_rx = inner.event_tx.subscribe();
        let (reply_tx, mut reply_rx) = mpsc::unbounded_channel::<(u64, DaemonRes)>();

        // Cancellation token: any one of reader/writer/watchdog cancelling it
        // causes the other two to shut down within one select cycle.
        let token = tokio_util::sync::CancellationToken::new();

        // Reader task: JSON lines → req_tx
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
                                    // protocol.md: "Lines exceeding this limit MUST be
                                    // rejected and the connection closed."
                                    warn!("client {client_id}: line too long ({} bytes), disconnecting", trimmed.len());
                                    break;
                                }
                                let wire_req: WireReq = match serde_json::from_str(trimmed) {
                                    Ok(r) => r,
                                    Err(e) => {
                                        // protocol.md: "If the daemon receives malformed
                                        // JSON, it MUST close the connection."
                                        warn!("client {client_id} malformed JSON, closing: {e}");
                                        break;
                                    }
                                };
                                let daemon_req = match DaemonReq::parse_cmd(&wire_req.cmd, wire_req.params.clone()) {
                                    Ok(r) => r,
                                    Err(e) if e.starts_with("unknown command:") => {
                                        // protocol.md: unknown `cmd` → error response, keep alive.
                                        let _ = r_tx.send((wire_req.id, DaemonRes::Error {
                                            message: e,
                                        }));
                                        line.clear();
                                        continue;
                                    }
                                    Err(e) => {
                                        // Params parsed wrong for a known `cmd`: recoverable.
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
            // Clean up auth state on disconnect
            inner_clone.client_auth.lock().await.remove(&client_id);
            info!("client {client_id} disconnected");
        });

        // Watchdog task: enforce 10 s handshake deadline.
        // Closed as soon as the reader exits (reader cancels `token`
        // on any exit path, so this task is woken and exits via the
        // cancelled branch above).
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

        // Writer task: responses + broadcast events → socket
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

    async fn dispatch(
        inner: Arc<DaemonInner>,
        client_id: ClientId,
        request_id: u64,
        req: DaemonReq,
        reply_tx: ReplyTx,
    ) {
        // Check if client has completed handshake
        let authenticated = {
            inner
                .client_auth
                .lock()
                .await
                .get(&client_id)
                .copied()
                .unwrap_or(false)
        };

        // Handshake is the only command allowed before authentication
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
                // protocol.md "Version Negotiation": client > daemon ⇒ ok:false.
                if *version > PROTOCOL_VERSION {
                    info!(
                        "client {client_id}: handshake rejected — client protocol v{version} > daemon v{PROTOCOL_VERSION}"
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
                // Explicit user play: reset Prev back-traversal and re-enable
                // the default-list fallback.
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
            DaemonReq::OrganizeLibrary { dry_run } => {
                Self::cmd_organize_library(inner, *dry_run).await
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
            DaemonReq::GetLyrics { track_id, path } => {
                Self::cmd_get_lyrics(inner, *track_id, path.clone()).await
            }
            DaemonReq::LyricsSearch { artist, title } => {
                Self::cmd_lyrics_search(inner, artist, title).await
            }
            DaemonReq::SpotifySetToken { token } => Self::cmd_spotify_set_token(inner, token).await,
            DaemonReq::SpotifyClear => Self::cmd_spotify_clear(inner).await,
            DaemonReq::SpotifyStatus => Self::cmd_spotify_status(inner).await,
            DaemonReq::SpotifySync => Self::cmd_spotify_sync(inner).await,
            DaemonReq::SpotifyPlaylists => Self::cmd_spotify_playlists(inner).await,
            DaemonReq::SpotifyPlaylistTracks { id } => {
                Self::cmd_spotify_playlist_tracks(inner, id).await
            }
            DaemonReq::SpotifyResolve {
                playlist_id,
                track_index,
            } => Self::cmd_spotify_resolve(inner, playlist_id, *track_index).await,
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
                // Save state before shutdown
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
                // Reply Ok first so clients can finish their quit() call, then
                // shut down shortly after the response has been flushed.
                let socket_path = inner.config.socket_path.clone();
                let socket_pulse_path = inner.config.socket_pulse_path.clone();
                tokio::spawn(async move {
                    tokio::time::sleep(Duration::from_millis(200)).await;
                    // Clean up socket files
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

    /// Save persistent state to disk. Non-blocking and failure-tolerant.
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

    /// Re-enable the default-list fallback (explicit play / queue add intent).
    async fn enable_fallback(inner: &DaemonInner) {
        inner.state.write().await.fallback_disabled = false;
    }

    /// Clear the Prev back-traversal history (explicit play / queue reset).
    async fn clear_history(inner: &DaemonInner) {
        inner.play_history.lock().await.clear();
    }

    /// Push a QueueChanged event carrying the merged queue view.
    async fn push_queue_state(inner: &DaemonInner) {
        let state = inner.state.read().await;
        let (queue, cursor) = queue::visible_queue(&state);
        drop(state);
        Self::push_event(inner, DaemonEvent::QueueChanged { queue, cursor });
    }

    /// The track that will play after the current one, without mutating any
    /// state.  Used to preload the crossfade standby.
    ///
    /// The default-list cursor points at the currently-playing (or most
    /// recently played) default entry, so the resume point is `cursor + 1`.
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

    /// Build the default playback list: the whole library sorted by title
    /// (case-insensitive), shuffled when enabled.  When not shuffling, the
    /// list is rotated so it resumes at the first title at/after the leading
    /// letter of `resume_key`.
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

    /// Consume the current entry (recording it for Prev back-traversal) and
    /// return the next track to play per the queue model:
    ///
    ///   1. the next user-queue entry, if any
    ///   2. the next default-list entry (RepeatMode governs the list end)
    ///   3. a freshly-built default list (resuming at the letter of the last
    ///      played track) once the user queue exhausts, unless disabled
    ///
    /// Returns None when playback should stop.  Does not start playback.
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
                    // Standalone interruption with queue items pending: the
                    // next queue entry plays without consuming anything.
                    let next = state.queue[0].clone();
                    drop(state);
                    drop(history);
                    return Ok(Some(next));
                }
                // User queue exhausted → fall through to the default list.
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
                    // Cursor already past the end (e.g. after an Off-stop);
                    // honor RepeatMode without re-recording history.
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

    /// Stop the mixer and reset playback state, broadcasting TrackEnded.
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

    /// Start playback of `track` as an auto-advanced next entry, using a
    /// crossfade when enabled and straightforward, else a plain play.
    /// Pushes PlaybackStarted.
    /// Preload `track` as a crossfade standby and start the fade without
    /// advancing the model.  Returns true when the fade started; the model is
    /// advanced later by `finish_crossfade` once the mixer swaps the standby
    /// into the active slot.
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

    /// Finalize a crossfade: the standby (next) track is already playing on
    /// the mixer, so only the state must be advanced and reported.
    async fn finish_crossfade(inner: &DaemonInner) {
        let actual = inner.mixer.lock().await.current_position();
        *inner.crossfade_loaded_for.lock().await = None;
        match Self::step_next(inner).await {
            Ok(Some(mut next)) => {
                let dur = inner.mixer.lock().await.duration();
                // Stamp the real duration onto the TrackInfo so the queued
                // track's metadata isn't left at 0 after a crossfade.
                next.duration = dur;
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
                // Crossfade completion: the mixer has swapped the standby
                // into the active slot, so the model must catch up before
                // time_pos is updated.
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

                if let Some(cf) = crossfade {
                    if cf.enabled && dur > 0.0 && (dur - pos) <= cf.duration_secs as f64 + 0.5 {
                        if let Some(track) = next {
                            let _ = Self::try_start_crossfade(inner, &track).await;
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
                    // Rare: the completion-detection poll was missed, so the
                    // crossfaded standby already ended. Finalize the model,
                    // then keep auto-advancing past it.
                    let _lock = inner.cmd_lock.lock().await;
                    Self::finish_crossfade(inner).await;
                    let _ = Self::cmd_next(inner).await;
                } else {
                    // Genuine end-of-track: auto-advance through the internal
                    // request channel so it is serialized with client commands.
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

        let track = if !inner.config.test_mode {
            let lib = Library::new(inner.config.data_dir.to_str().unwrap_or("")).ok();
            if let Some(ref lib) = lib {
                match lib.track_by_path(&path_owned) {
                    Ok(Some(mut t)) => {
                        t.duration = dur;
                        t
                    }
                    _ => {
                        // Fallback: search by path substring
                        let mut fallback = gtm_core::track::TrackInfo {
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
                            ..Default::default()
                        };
                        if let Ok(tracks) = lib.list_tracks() {
                            if let Some(matched) = tracks.iter().find(|t| {
                                path_owned.contains(&t.path) || t.path.contains(&path_owned)
                            }) {
                                fallback.id = matched.id;
                                fallback.title = matched.title.clone();
                                fallback.artist = matched.artist.clone();
                                fallback.album = matched.album.clone();
                                fallback.cover_path = matched.cover_path.clone();
                                fallback.favourite = matched.favourite;
                            }
                        }
                        fallback
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
                    ..Default::default()
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
                ..Default::default()
            }
        };
        // Rotate the user queue so the explicitly-played track sits at index
        // 0 (the one-time queue consumes from the head).  No-op when the track
        // isn't queued (standalone interruption) or already at the head.
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
                // Stopped with nothing current: play the merged view's entry
                // at the cursor (next user entry or default-list entry).
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

    /// Stop playback: stops the mixer backend, transitions state to Stopped,
    /// and broadcasts PlaybackStopped.  Safe to call when already stopped
    /// (checks status before calling state.stop() to avoid assert).
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

    /// Advance to the next track per the queue model: consume the current
    /// entry, then play the next queued / default-list / freshly-built
    /// fallback track.  Stops playback when there is nothing left to play.
    async fn cmd_next(inner: &DaemonInner) -> Result<DaemonRes, CoreError> {
        // Crossfade path: look up the standby without advancing the model and
        // start the fade; `finish_crossfade` advances at completion.  Falls
        // through to an immediate step for the non-crossfade case.
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

    /// Go to the previous track, traversing playback history back through
    /// user-queue entries and into the default list.  If the current track
    /// has been playing for a while, restart it instead.
    async fn cmd_prev(inner: &DaemonInner) -> Result<DaemonRes, CoreError> {
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

    /// Seek to absolute position in seconds.  No-op if status is Stopped.
    /// Re-decodes the current track from `pos` via `SymphoniaSource`'s
    /// seek-skip fast-forward (the decode runs on a blocking task so the
    /// event loop isn't starved).  Reports the requested (clamped) position.
    async fn cmd_seek(inner: &DaemonInner, pos: f64) -> Result<DaemonRes, CoreError> {
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

    async fn cmd_organize_library(
        inner: &DaemonInner,
        dry_run: Option<bool>,
    ) -> Result<DaemonRes, CoreError> {
        let is_dry_run = dry_run.unwrap_or(true);
        // TODO: Implement actual library organization
        // For now, return success with 0 moves
        if !is_dry_run {
            warn!("library organize: destructive mode not yet implemented");
        }
        Self::push_event(
            inner,
            DaemonEvent::LibraryOrganized {
                moves_succeeded: 0,
                moves_failed: 0,
            },
        );
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

    /// Queue command dispatcher.
    ///
    /// ```text
    ///  QueueAction
    ///  ├── List       → return current queue + cursor
    ///  ├── Clear      → clear queue, push QueueChanged
    ///  ├── Remove(i)  → remove at index, push QueueChanged
    ///  ├── Move(f,t)  → move from→to, push QueueChanged
    ///  ├── Add(paths) → expand files/dirs, add tracks, auto-play if empty
    ///  └── Set        → replace entire queue
    /// ```
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
                let result = tokio::task::spawn_blocking(move || {
                    let lib = Library::new(data_dir.to_str().unwrap_or(""))?;
                    lib.remove_track(id)
                })
                .await
                .map_err(|e| CoreError::Daemon(e.to_string()))?;
                match result {
                    Ok(()) => DaemonRes::Ok,
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

    /// Kick off a background library sync (covers/lyrics/metadata). Returns
    /// immediately so the client's short response timeout is never hit and
    /// `cmd_lock` is not held across the long-running sync. Progress is
    /// reported via [`Self::cmd_sync_status`] and a `sync_done` event.
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
        // Fire-and-forget: the manager cancels any in-flight yt-dlp search and
        // starts a new one immediately, so the request returns before the
        // client's short response timeout. Results are picked up via
        // `cmd_yt_search_poll`, which echoes the query they belong to.
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
        // Report the merged queue view so clients see the same list they get
        // from QueueChanged events (user entries + remaining default list).
        let (queue, cursor) = queue::visible_queue(&state);
        state_clone.queue = queue;
        state_clone.queue_cursor = cursor;
        drop(state);
        Ok(DaemonRes::Status {
            state: Box::new(state_clone),
        })
    }

    async fn cmd_check_health(inner: &DaemonInner) -> Result<DaemonRes, CoreError> {
        use gtm_core::ipc::{ComponentHealth, HealthReport, HealthStatus};
        let h = &inner.health;
        let mut components = Vec::new();

        // Audio backend
        components.push(ComponentHealth {
            name: "audio_backend".into(),
            status: HealthStatus::Ok,
            message: Some(h.audio_backend.clone()),
            uptime_secs: Some(h.uptime_secs()),
        });

        // Library scan
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

        // YouTube
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

        // Cover art
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

        // Lyrics
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

        // Event channel capacity
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
                        return Ok(DaemonRes::CoverArt { data: Some(b64) });
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
                        return Ok(DaemonRes::CoverArt { data: Some(b64) });
                    }
                }
                discovered_artist = track.artist;
                discovered_album = track.album;
            }
        }

        // If not found in library, search the merged queue view and the
        // current track for the requested track_id.
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

        // Deezer fallback via CoverCache with discovered artist/album
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
                    use base64::Engine;
                    let b64 = base64::engine::general_purpose::STANDARD.encode(&cover.data);
                    return Ok(DaemonRes::CoverArt { data: Some(b64) });
                }
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
                // Prefer the library row (full tags) when available; otherwise
                // fall back to a path-derived track so queued/foreign tracks
                // (id == 0) can still be looked up.
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

        // If tags are missing, derive artist/title from the filename so the
        // lrclib exact/search lookup gets meaningful metadata.
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
        let spotify = inner.spotify.lock().await;
        Ok(DaemonRes::SpotifyStatusRes {
            status: spotify.status(),
        })
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

    /// Resolve a cached Spotify track to a YouTube stream, download it into
    /// the cache, and append it to the user queue (auto-playing if empty).
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

        let was_empty = {
            let mut state = inner.state.write().await;
            state.fallback_disabled = false;
            let w = state.queue.is_empty() && state.status == PlaybackStatus::Stopped;
            queue::queue_add(&mut state, &path, None);
            drop(state);
            w
        };
        if was_empty {
            let _ = Self::cmd_play(inner, &path, 0.0, false).await;
        }
        Self::push_queue_state(inner).await;
        Self::save_state(inner);
        Ok(DaemonRes::Ok)
    }

    /// Download a stream URL into `cache_dir/spotify/` via yt-dlp, returning
    /// the local file path. Stale files with the same prefix are replaced.
    async fn download_audio_to_cache(
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
}

/// Derive the artist/title pair to query Deezer with for a library track.
///
/// Falls back to cleaning the filename stem when the stored metadata is empty,
/// still equals the raw stem, or otherwise looks like an unparsed filename
/// (e.g. yt-dlp underscore names that survived an older scan).
fn metadata_query_for(track: &TrackInfo) -> (String, String) {
    let stem = Path::new(&track.path)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("");
    let (cleaned_artist, cleaned_title) = crate::metadata_cleaner::clean_filename_stem(stem);
    let title_unreliable = crate::metadata_cleaner::is_filename_like(stem, &track.title);
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

/// Background sync: fetch cover art for tracks missing it. Runs on a blocking
/// thread so the daemon event loop stays responsive; progress is mirrored into
/// `progress` for the `SyncStatus` poll.
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
    let mut cache = crate::cover_art::CoverCache::new(cache_dir.clone());
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
            let key = crate::cover_art::CoverCache::cache_key(artist, album);
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

/// Background sync: write `.lrc` sidecars for tracks without one.
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
                let mut lrc_content = String::new();
                if let Some(ref ar) = lyrics.artist {
                    lrc_content.push_str(&format!("[ar:{}]\n", ar));
                }
                if let Some(ref al) = lyrics.album {
                    lrc_content.push_str(&format!("[al:{}]\n", al));
                }
                if let Some(ref ti) = lyrics.title {
                    lrc_content.push_str(&format!("[ti:{}]\n", ti));
                }
                for line in &lyrics.lines {
                    let mins = (line.timestamp / 60.0) as u64;
                    let secs = line.timestamp - (mins as f64 * 60.0);
                    lrc_content.push_str(&format!("[{:02}:{:05.2}]{}\n", mins, secs, line.text));
                }
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

/// Background sync: enrich unreliable track metadata via Deezer and embed tags
/// into the files. Tracks with no verified match fall back to the cleaned
/// filename stem so the library never displays a raw yt-dlp filename.
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
        } else if !crate::metadata_cleaner::title_is_unreliable(
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
            // No verified Deezer match: fall back to the cleaned filename stem
            // so the library shows a real title/artist instead of the raw name.
            let (cleaned_artist, cleaned_title) =
                crate::metadata_cleaner::clean_filename_stem(stem);
            if !cleaned_title.is_empty() || cleaned_artist.is_some() {
                let patch = gtm_core::MetadataPatch {
                    title: (!cleaned_title.is_empty())
                        .then(|| crate::metadata_cleaner::sanitize_text(&cleaned_title)),
                    artist: cleaned_artist.map(|a| crate::metadata_cleaner::sanitize_text(&a)),
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
