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

use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::{broadcast, mpsc, RwLock};
use tracing::{error, info, warn};

use gtm_audio::{AudioEvent, AudioMixer, AudioResult, Mixer, NullMixer};
use gtm_core::ipc::{DaemonEvent, DaemonReq, DaemonRes, QueueAction};
use gtm_core::state::{DaemonState, PlaybackStatus};
use gtm_core::wire;
use gtm_core::CoreError;

use crate::config::DaemonConfig;
use crate::queue;

type ClientId = u64;
type ReplyTx = mpsc::UnboundedSender<DaemonRes>;

pub struct Daemon {
    pub state: Arc<RwLock<DaemonState>>,
    pub mixer: Box<dyn Mixer>,
    pub listener: UnixListener,
    pub config: DaemonConfig,
    pub event_tx: broadcast::Sender<DaemonEvent>,
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
        if socket_path.exists() {
            std::fs::remove_file(socket_path)
                .map_err(|e| CoreError::Daemon(format!("remove stale socket: {e}")))?;
        }

        let listener = UnixListener::bind(socket_path)
            .map_err(|e| CoreError::Daemon(format!("bind socket: {e}")))?;

        let (event_tx, _) = broadcast::channel(256);
        let (req_tx, req_rx) = mpsc::unbounded_channel();

        Ok(Self {
            state,
            mixer,
            listener,
            config,
            event_tx,
            req_tx,
            req_rx,
            next_client_id: 0,
            crossfade_loaded_for: None,
        })
    }

    /// Main daemon event loop — multiplexes three sources:
    ///
    ///   1. **new connections**  — accept_client() spawns per-client read/write tasks
    ///   2. **IPC requests**     — dispatch() → handle_request() → cmd_*()
    ///   3. **audio events**     — handle_audio_event() updates state, pushes events
    pub async fn run(&mut self) -> Result<(), CoreError> {
        info!("daemon started on {}", self.config.socket_path.display());

        loop {
            tokio::select! {
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
                Some((client_id, req, reply_tx)) = self.req_rx.recv() => {
                    self.dispatch(client_id, req, reply_tx).await;
                }
                result = std::future::ready(self.mixer.poll()) => {
                    self.handle_audio_event(result).await;
                }
            }
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
                                let line = serde_json::to_string(&response).unwrap() + "\n";
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
                                let frame = match wire::encode(&[event]) {
                                    Ok(f) => f,
                                    Err(e) => {
                                        warn!("encode event: {e}");
                                        continue;
                                    }
                                };
                                if writer.write_all(&frame).await.is_err() {
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

    async fn dispatch(&mut self, _client_id: ClientId, req: DaemonReq, reply_tx: ReplyTx) {
        let res = match self.handle_request(&req).await {
            Ok(res) => res,
            Err(e) => DaemonRes::Error {
                version: self.state.read().await.version as u32,
                message: e.to_string(),
            },
        };
        let _ = reply_tx.send(res);
    }

    async fn handle_request(&mut self, req: &DaemonReq) -> Result<DaemonRes, CoreError> {
        match req {
            DaemonReq::Play { path, start_pos } => self.cmd_play(path, *start_pos).await,
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
            DaemonReq::Ping => Ok(DaemonRes::Pong),
            DaemonReq::Quit => {
                info!("quit requested");
                self.cmd_stop().await?;
                // In a production daemon, this would break the event loop.
                // For now we stop playback and let the process exit.
                std::process::exit(0);
            }
        }
    }

    async fn push_event(&self, event: DaemonEvent) {
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
                tokio::task::yield_now().await;
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
                            if self.mixer.load_standby(path).is_ok() {
                                self.mixer.start_crossfade(cf.duration_secs as f64);
                                self.crossfade_loaded_for = cur_path.clone();
                                // Advance the queue cursor to the next track
                                if let Ok(mut s) = self.state.try_write() {
                                    let _ = s.advance_queue(1);
                                    let idx = s.queue_cursor;
                                    drop(s);
                                    self.push_event(DaemonEvent::QueueIndexChanged { index: idx })
                                        .await;
                                }
                            }
                        }
                    }
                }

                self.push_event(DaemonEvent::PositionChanged { time_pos: pos })
                    .await;
            }
            AudioEvent::Duration(dur) => {
                let mut state = self.state.write().await;
                state.duration = dur;
                drop(state);
                self.push_event(DaemonEvent::DurationChanged { duration: dur })
                    .await;
            }
            // Track finished — if we were crossfading the next track is already
            // playing on the standby player, so don't auto-advance.
            AudioEvent::Finished => {
                let was_crossfading = self.crossfade_loaded_for.is_some();
                self.crossfade_loaded_for = None;
                let mut state = self.state.write().await;
                state.status = PlaybackStatus::Stopped;
                state.time_pos = 0.0;
                drop(state);
                self.push_event(DaemonEvent::TrackEnded).await;
                if !was_crossfading {
                    let _ = self.cmd_next().await;
                }
            }
            AudioEvent::Error(msg) => {
                warn!("audio error: {msg}");
                self.push_event(DaemonEvent::Custom {
                    name: "audio_error".into(),
                    data: [("error".into(), msg)].into(),
                })
                .await;
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
    async fn cmd_play(&mut self, path: &str, start_pos: f64) -> Result<DaemonRes, CoreError> {
        self.mixer.stop()?;
        self.crossfade_loaded_for = None;
        {
            let mut state = self.state.write().await;
            if state.status != PlaybackStatus::Stopped {
                state.stop()?;
            }
        }

        self.mixer.load_active(path, start_pos)?;
        self.mixer.play()?;
        let dur = self.mixer.duration();
        let mut state = self.state.write().await;
        let track = gtm_core::track::TrackInfo {
            id: 0,
            path: path.to_string(),
            title: String::new(),
            artist: String::new(),
            album: String::new(),
            duration: dur,
            track_number: None,
            genre: String::new(),
            year: None,
            bitrate: None,
            samplerate: None,
            hash: String::new(),
            cover_path: None,
            favourite: false,
        };
        state.play(track.clone())?;
        state.time_pos = start_pos;
        state.duration = dur;
        let version = state.version as u32;
        drop(state);
        self.push_event(DaemonEvent::PlaybackStarted {
            track,
            auto_advanced: false,
            time_pos: start_pos,
            duration: dur,
        })
        .await;
        Ok(DaemonRes::Ok { version })
    }

    /// Smart play/pause toggle:
    ///
    ///   - If mixer is playing → pause
    ///   - If paused → resume (no reload, just unpause backend)
    ///   - If stopped with a current track → play from beginning
    ///   - If stopped with no track → no-op
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
                let track = state.current_track.clone().unwrap();
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
                })
                .await;
                Ok(DaemonRes::Ok { version })
            } else if !path.is_empty() {
                self.cmd_play(&path, 0.0).await
            } else {
                let version = self.state.read().await.version as u32;
                Ok(DaemonRes::Ok { version })
            }
        }
    }

    async fn cmd_pause(&mut self) -> Result<DaemonRes, CoreError> {
        self.mixer.pause()?;
        let mut state = self.state.write().await;
        state.pause()?;
        state.time_pos = self.mixer.current_position();
        let version = state.version as u32;
        drop(state);
        self.push_event(DaemonEvent::PlaybackPaused).await;
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
        self.push_event(DaemonEvent::PlaybackStopped).await;
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
        drop(state);
        self.crossfade_loaded_for = None;
        let path = track.path.clone();
        let res = self.cmd_play(&path, 0.0).await?;
        self.push_event(DaemonEvent::QueueIndexChanged { index: idx })
            .await;
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
        drop(state);
        self.crossfade_loaded_for = None;
        let path = track.path.clone();
        let res = self.cmd_play(&path, 0.0).await?;
        self.push_event(DaemonEvent::QueueIndexChanged { index: idx })
            .await;
        Ok(res)
    }

    /// Seek to absolute position in seconds.  Errors if status is Stopped.
    /// Reports the *actual* position (mixer.current_position()) in the event,
    /// which may differ from the requested pos due to clamping.
    async fn cmd_seek(&mut self, pos: f64) -> Result<DaemonRes, CoreError> {
        let state = self.state.read().await;
        if state.status == PlaybackStatus::Stopped {
            return Err(CoreError::Daemon(
                "cannot seek while stopped".into(),
            ));
        }
        drop(state);
        self.mixer.seek(pos)?;
        let actual = self.mixer.current_position();
        let mut state = self.state.write().await;
        state.seek(actual)?;
        let version = state.version as u32;
        drop(state);
        self.push_event(DaemonEvent::PositionChanged { time_pos: actual })
            .await;
        Ok(DaemonRes::Ok { version })
    }

    async fn cmd_set_volume(&mut self, volume: u8) -> Result<DaemonRes, CoreError> {
        self.mixer.set_volume(volume)?;
        let mut state = self.state.write().await;
        state.set_volume(volume)?;
        let version = state.version as u32;
        drop(state);
        self.push_event(DaemonEvent::VolumeChanged { volume }).await;
        Ok(DaemonRes::Ok { version })
    }

    async fn cmd_toggle_shuffle(&mut self) -> Result<DaemonRes, CoreError> {
        let mut state = self.state.write().await;
        state.toggle_shuffle()?;
        let enabled = state.shuffle;
        let version = state.version as u32;
        drop(state);
        self.push_event(DaemonEvent::ShuffleChanged { enabled })
            .await;
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
        self.push_event(DaemonEvent::RepeatModeChanged { mode: m })
            .await;
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
                self.push_event(DaemonEvent::QueueChanged { queue, cursor })
                    .await;
                return Ok(DaemonRes::Ok { version });
            }
            QueueAction::Remove { index } => {
                let mut state = self.state.write().await;
                queue::queue_remove(&mut state, *index);
                let version = state.version as u32;
                let queue = state.queue.clone();
                let cursor = state.queue_cursor;
                drop(state);
                self.push_event(DaemonEvent::QueueChanged { queue, cursor })
                    .await;
                return Ok(DaemonRes::Ok { version });
            }
            QueueAction::Move { from, to } => {
                let mut state = self.state.write().await;
                queue::queue_move(&mut state, *from, *to);
                let version = state.version as u32;
                let queue = state.queue.clone();
                let cursor = state.queue_cursor;
                drop(state);
                self.push_event(DaemonEvent::QueueChanged { queue, cursor })
                    .await;
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
                    self.push_event(DaemonEvent::QueueChanged { queue, cursor })
                        .await;
                    if was_empty {
                        return self.cmd_play(path, 0.0).await;
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
                    self.push_event(DaemonEvent::QueueChanged { queue, cursor })
                        .await;
                    if was_empty {
                        return self.cmd_play(&first_path, 0.0).await;
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
                    self.push_event(DaemonEvent::QueueChanged { queue, cursor })
                        .await;
                    // If queue was empty and stopped, auto-play the first track
                    if was_empty {
                        return self.cmd_play(&first_path, 0.0).await;
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
                self.push_event(DaemonEvent::QueueChanged { queue, cursor })
                    .await;
                Ok(DaemonRes::Ok { version })
            }
        }
    }

    async fn cmd_library(
        &mut self,
        _action: &gtm_core::ipc::LibraryAction,
    ) -> Result<DaemonRes, CoreError> {
        let version = self.state.read().await.version as u32;
        Ok(DaemonRes::Ok { version })
    }

    async fn cmd_search(&mut self, _query: &str) -> Result<DaemonRes, CoreError> {
        let version = self.state.read().await.version as u32;
        Ok(DaemonRes::Ok { version })
    }

    async fn cmd_get_favourites(&mut self) -> Result<DaemonRes, CoreError> {
        let version = self.state.read().await.version as u32;
        Ok(DaemonRes::Ok { version })
    }

    async fn cmd_add_favourite(&mut self, _track_id: i64) -> Result<DaemonRes, CoreError> {
        let version = self.state.read().await.version as u32;
        Ok(DaemonRes::Ok { version })
    }

    async fn cmd_remove_favourite(&mut self, _track_id: i64) -> Result<DaemonRes, CoreError> {
        let version = self.state.read().await.version as u32;
        Ok(DaemonRes::Ok { version })
    }

    async fn cmd_yt_search(
        &mut self,
        _query: &str,
        _filter: Option<gtm_core::state::YTFilter>,
    ) -> Result<DaemonRes, CoreError> {
        let version = self.state.read().await.version as u32;
        Ok(DaemonRes::Ok { version })
    }

    async fn cmd_yt_search_poll(&mut self) -> Result<DaemonRes, CoreError> {
        let version = self.state.read().await.version as u32;
        Ok(DaemonRes::Ok { version })
    }

    async fn cmd_yt_search_cancel(&mut self) -> Result<DaemonRes, CoreError> {
        let version = self.state.read().await.version as u32;
        Ok(DaemonRes::Ok { version })
    }

    async fn cmd_yt_resolve_stream(&mut self, _url: &str) -> Result<DaemonRes, CoreError> {
        let version = self.state.read().await.version as u32;
        Ok(DaemonRes::Ok { version })
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
}
