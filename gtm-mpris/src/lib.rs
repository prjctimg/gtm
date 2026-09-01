// Copyright (c) 2026
// Author: prjctimg <prjctimg@outlook.com>
// MPRIS D-Bus interface implementation
//
// This is free software released under the GPL-3.0 license.

// The zbus macro-generated trampoline for the `metadata` property getter
// elides a lifetime on `HashMap<String, Value>`, producing a spurious lint.
#![allow(mismatched_lifetime_syntaxes)]

use std::collections::HashMap;
use std::sync::Arc;

use tokio::sync::{RwLock, broadcast};
use tracing::{info, warn};
use zbus::connection::Builder;
use zbus::interface;
use zvariant::{ObjectPath, Value};

use gtm_core::ipc::{DaemonEvent, DaemonReq};
use gtm_core::state::{DaemonState, PlaybackStatus, RepeatMode};

const BUS_NAME: &str = "org.mpris.MediaPlayer2.gtm";
const OBJECT_PATH: &str = "/org/mpris/MediaPlayer2";
const PROPERTY_IFACE: &str = "org.freedesktop.DBus.Properties";

/// Handle to shut down the MPRIS server.
pub struct MprisHandle {
    cancel: tokio::sync::broadcast::Sender<()>,
    #[allow(dead_code)]
    task: tokio::task::JoinHandle<()>,
}

impl MprisHandle {
    pub fn shutdown(&self) {
        let _ = self.cancel.send(());
    }
}

struct MediaPlayer2;

#[interface(name = "org.mpris.MediaPlayer2")]
impl MediaPlayer2 {
    async fn raise(&self) {}

    async fn quit(&self) {}

    #[zbus(property)]
    async fn can_quit(&self) -> bool {
        false
    }

    #[zbus(property)]
    async fn can_raise(&self) -> bool {
        false
    }

    #[zbus(property)]
    async fn has_track_list(&self) -> bool {
        false
    }

    #[zbus(property)]
    async fn identity(&self) -> &str {
        "gtm"
    }

    #[zbus(property)]
    async fn desktop_entry(&self) -> &str {
        "gtm"
    }

    #[zbus(property)]
    async fn supported_uri_schemes(&self) -> Vec<String> {
        vec!["file".into(), "http".into(), "https".into()]
    }

    #[zbus(property)]
    async fn supported_mime_types(&self) -> Vec<String> {
        vec![
            "audio/mpeg".into(),
            "audio/ogg".into(),
            "audio/flac".into(),
            "audio/x-wav".into(),
            "audio/aac".into(),
            "audio/mp4".into(),
            "audio/x-opus".into(),
            "audio/webm".into(),
        ]
    }
}

/// Commands a remote MPRIS client can issue, mapped to daemon IPC requests.
enum UserCommand {
    PlayPause,
    Play,
    Pause,
    Stop,
    Next,
    Prev,
    /// Relative seek offset in microseconds.
    Seek(i64),
    SetPosition(String, i64),
    SetLoopStatus(String),
    SetVolume(f64),
    SetShuffle(bool),
}

struct Player {
    state: Arc<RwLock<DaemonState>>,
    cmd_tx: tokio::sync::mpsc::UnboundedSender<UserCommand>,
}

impl Player {
    fn new(
        state: Arc<RwLock<DaemonState>>,
        cmd_tx: tokio::sync::mpsc::UnboundedSender<UserCommand>,
    ) -> Self {
        Self { state, cmd_tx }
    }

    fn loop_status_str(repeat: RepeatMode) -> &'static str {
        match repeat {
            RepeatMode::Off => "None",
            RepeatMode::One => "Track",
            RepeatMode::All => "Playlist",
        }
    }
}

#[interface(name = "org.mpris.MediaPlayer2.Player")]
impl Player {
    async fn next(&self) {
        let _ = self.cmd_tx.send(UserCommand::Next);
    }

    async fn previous(&self) {
        let _ = self.cmd_tx.send(UserCommand::Prev);
    }

    async fn pause(&self) {
        let _ = self.cmd_tx.send(UserCommand::Pause);
    }

    async fn play_pause(&self) {
        let _ = self.cmd_tx.send(UserCommand::PlayPause);
    }

    async fn stop(&self) {
        let _ = self.cmd_tx.send(UserCommand::Stop);
    }

    async fn play(&self) {
        let state = self.state.read().await;
        let idle = state.status == PlaybackStatus::Stopped
            && state.current_track.is_none()
            && state.queue.is_empty();
        drop(state);
        // No track to play: no-op, like the spec asks.
        if idle {
            return;
        }
        let _ = self.cmd_tx.send(UserCommand::Play);
    }

    async fn seek(&self, offset: i64) {
        let _ = self.cmd_tx.send(UserCommand::Seek(offset));
    }

    async fn set_position(&self, track_id: ObjectPath<'_>, position: i64) {
        let _ = self
            .cmd_tx
            .send(UserCommand::SetPosition(track_id.to_string(), position));
    }

    async fn open_uri(&self, _uri: &str) {}

    #[zbus(property)]
    async fn playback_status(&self) -> String {
        let state = self.state.read().await;
        match state.status {
            PlaybackStatus::Playing => "Playing".into(),
            PlaybackStatus::Paused => "Paused".into(),
            PlaybackStatus::Stopped => "Stopped".into(),
        }
    }

    #[zbus(property)]
    async fn loop_status(&self) -> String {
        let state = self.state.read().await;
        Self::loop_status_str(state.repeat).to_string()
    }

    #[zbus(property)]
    async fn set_loop_status(&self, status: &str) {
        let _ = self
            .cmd_tx
            .send(UserCommand::SetLoopStatus(status.to_string()));
    }

    #[zbus(property)]
    async fn rate(&self) -> f64 {
        1.0
    }

    #[zbus(property)]
    async fn set_rate(&self, _rate: f64) {}

    #[zbus(property)]
    async fn shuffle(&self) -> bool {
        let state = self.state.read().await;
        state.shuffle
    }

    #[zbus(property)]
    async fn set_shuffle(&self, shuffle: bool) {
        let state = self.state.read().await;
        if state.shuffle != shuffle {
            let _ = self.cmd_tx.send(UserCommand::SetShuffle(shuffle));
        }
    }

    #[zbus(property)]
    async fn metadata(&self) -> HashMap<String, Value> {
        let state = self.state.read().await;
        let mut meta = HashMap::new();
        if let Some(ref track) = state.current_track {
            let track_id = ObjectPath::try_from(format!("/gtm/track/{}", track.id))
                .unwrap_or(ObjectPath::try_from("/gtm/track/0").unwrap());
            meta.insert("mpris:trackid".into(), Value::ObjectPath(track_id));
            meta.insert(
                "mpris:length".into(),
                Value::I64((track.duration * 1_000_000.0) as i64),
            );
            if !track.title.is_empty() {
                meta.insert("xesam:title".into(), Value::Str(track.title.clone().into()));
            }
            if !track.artist.is_empty() {
                meta.insert(
                    "xesam:artist".into(),
                    Value::Array(vec![Value::Str(track.artist.clone().into())].into()),
                );
            }
            if !track.album.is_empty() {
                meta.insert("xesam:album".into(), Value::Str(track.album.clone().into()));
            }
            if let Some(ref path) = track.cover_path {
                meta.insert(
                    "mpris:artUrl".into(),
                    Value::Str(format!("file://{path}").into()),
                );
            }
            if let Some(track_num) = track.track_number {
                meta.insert("xesam:trackNumber".into(), Value::I32(track_num));
            }
            if let Some(year) = track.year {
                meta.insert(
                    "xesam:contentCreated".into(),
                    Value::Str(format!("{year}-01-01T00:00:00").into()),
                );
            }
        }
        meta
    }

    #[zbus(property)]
    async fn volume(&self) -> f64 {
        let state = self.state.read().await;
        state.volume as f64 / 100.0
    }

    #[zbus(property)]
    async fn set_volume(&self, volume: f64) {
        let _ = self.cmd_tx.send(UserCommand::SetVolume(volume));
    }

    #[zbus(property)]
    async fn position(&self) -> i64 {
        let state = self.state.read().await;
        (state.time_pos * 1_000_000.0) as i64
    }

    #[zbus(property)]
    async fn minimum_rate(&self) -> f64 {
        1.0
    }

    #[zbus(property)]
    async fn maximum_rate(&self) -> f64 {
        1.0
    }

    #[zbus(property)]
    async fn can_go_next(&self) -> bool {
        true
    }

    #[zbus(property)]
    async fn can_go_previous(&self) -> bool {
        true
    }

    #[zbus(property)]
    async fn can_play(&self) -> bool {
        true
    }

    #[zbus(property)]
    async fn can_pause(&self) -> bool {
        true
    }

    #[zbus(property)]
    async fn can_seek(&self) -> bool {
        true
    }

    #[zbus(property)]
    async fn can_control(&self) -> bool {
        true
    }
}

/// Start the MPRIS D-Bus server on the session bus. Subscribes to the daemon
/// event stream and forwards state changes as `PropertiesChanged` signals;
/// MPRIS client commands are forwarded to the daemon as `DaemonReq`s through
/// `req_tx` (the daemon's internal request channel).
pub async fn start(
    state: Arc<RwLock<DaemonState>>,
    event_rx: broadcast::Receiver<DaemonEvent>,
    req_tx: tokio::sync::mpsc::UnboundedSender<DaemonReq>,
) -> Result<MprisHandle, Box<dyn std::error::Error + Send + Sync>> {
    let (cmd_tx, mut cmd_rx) = tokio::sync::mpsc::unbounded_channel::<UserCommand>();
    let (cancel_tx, _) = tokio::sync::broadcast::channel::<()>(1);

    let player = Player::new(state.clone(), cmd_tx);
    let media_player2 = MediaPlayer2;

    let conn = Builder::session()?
        .name(BUS_NAME)?
        .serve_at(OBJECT_PATH, media_player2)?
        .serve_at(OBJECT_PATH, player)?
        .build()
        .await?;

    info!("mpris: registered on session bus as {BUS_NAME}");

    let cancel_rx = cancel_tx.subscribe();
    let task = tokio::spawn(async move {
        let mut cancel_rx = cancel_rx;

        // Forward MPRIS client commands to the daemon as IPC requests.
        let cmd_state = state.clone();
        let cmd_req_tx = req_tx;
        let cmd_task = tokio::spawn(async move {
            while let Some(cmd) = cmd_rx.recv().await {
                match cmd {
                    UserCommand::PlayPause => {
                        let _ = cmd_req_tx.send(DaemonReq::PlayPause);
                    }
                    UserCommand::Play => {
                        let _ = cmd_req_tx.send(DaemonReq::PlayPause);
                    }
                    UserCommand::Pause => {
                        let _ = cmd_req_tx.send(DaemonReq::Pause);
                    }
                    UserCommand::Stop => {
                        let _ = cmd_req_tx.send(DaemonReq::Stop);
                    }
                    UserCommand::Next => {
                        let _ = cmd_req_tx.send(DaemonReq::Next);
                    }
                    UserCommand::Prev => {
                        let _ = cmd_req_tx.send(DaemonReq::Prev);
                    }
                    UserCommand::Seek(offset) => {
                        let pos = cmd_state.read().await.time_pos;
                        let target = pos + offset as f64 / 1_000_000.0;
                        let _ = cmd_req_tx.send(DaemonReq::Seek {
                            position_secs: target.max(0.0),
                        });
                    }
                    UserCommand::SetPosition(track_id, position) => {
                        let current = cmd_state.read().await;
                        let matches = current
                            .current_track
                            .as_ref()
                            .map(|t| format!("/gtm/track/{}", t.id))
                            == Some(track_id);
                        drop(current);
                        if matches {
                            let _ = cmd_req_tx.send(DaemonReq::Seek {
                                position_secs: position as f64 / 1_000_000.0,
                            });
                        }
                    }
                    UserCommand::SetLoopStatus(status) => {
                        let mode = match status.as_str() {
                            "Track" => RepeatMode::One,
                            "Playlist" => RepeatMode::All,
                            _ => RepeatMode::Off,
                        };
                        let _ = cmd_req_tx.send(DaemonReq::CycleRepeat { mode });
                    }
                    UserCommand::SetVolume(volume) => {
                        let vol = (volume.clamp(0.0, 1.0) * 100.0) as u8;
                        let _ = cmd_req_tx.send(DaemonReq::SetVolume { volume: vol });
                    }
                    UserCommand::SetShuffle(enabled) => {
                        if cmd_state.read().await.shuffle != enabled {
                            let _ = cmd_req_tx.send(DaemonReq::ToggleShuffle);
                        }
                    }
                }
            }
        });

        // Apply daemon events to the shared state and notify clients.
        let mut event_rx = event_rx;
        loop {
            tokio::select! {
                _ = cancel_rx.recv() => {
                    info!("mpris: shutting down");
                    break;
                }
                result = event_rx.recv() => {
                    match result {
                        Ok(event) => {
                            apply_event(&state, &event).await;
                            emit_changes(&conn, &event).await;
                        }
                        Err(broadcast::error::RecvError::Lagged(n)) => {
                            warn!("mpris: event stream lagged by {n}");
                        }
                        Err(broadcast::error::RecvError::Closed) => {
                            info!("mpris: event stream closed");
                            break;
                        }
                    }
                }
            }
        }

        cmd_task.abort();
    });

    Ok(MprisHandle {
        cancel: cancel_tx,
        task,
    })
}

async fn apply_event(state: &Arc<RwLock<DaemonState>>, event: &DaemonEvent) {
    let mut s = state.write().await;
    match event {
        DaemonEvent::PlaybackStarted {
            track,
            time_pos,
            duration,
            ..
        } => {
            s.status = PlaybackStatus::Playing;
            s.current_track = Some(track.clone());
            s.time_pos = *time_pos;
            s.duration = *duration;
        }
        DaemonEvent::PlaybackPaused { time_pos } => {
            s.status = PlaybackStatus::Paused;
            s.time_pos = *time_pos;
        }
        DaemonEvent::PlaybackStopped | DaemonEvent::TrackEnded => {
            s.status = PlaybackStatus::Stopped;
            s.current_track = None;
            s.time_pos = 0.0;
        }
        DaemonEvent::PositionChanged { time_pos } => {
            s.time_pos = *time_pos;
        }
        DaemonEvent::VolumeChanged { volume } => {
            s.volume = *volume;
        }
        DaemonEvent::ShuffleChanged { enabled } => {
            s.shuffle = *enabled;
        }
        DaemonEvent::RepeatModeChanged { mode } => {
            s.repeat = *mode;
        }
        DaemonEvent::DurationChanged { duration } => {
            s.duration = *duration;
        }
        _ => {}
    }
}

async fn emit_changes(conn: &zbus::Connection, event: &DaemonEvent) {
    let names: &[&str] = match event {
        DaemonEvent::PlaybackStarted { .. } => {
            &["PlaybackStatus", "Metadata", "CanGoNext", "CanGoPrevious"]
        }
        DaemonEvent::PlaybackPaused { .. } | DaemonEvent::PlaybackStopped => {
            &["PlaybackStatus", "CanPlay", "CanPause"]
        }
        DaemonEvent::TrackEnded => &["PlaybackStatus", "Metadata", "Position"],
        DaemonEvent::VolumeChanged { .. } => &["Volume"],
        DaemonEvent::ShuffleChanged { .. } => &["Shuffle"],
        DaemonEvent::RepeatModeChanged { .. } => &["LoopStatus"],
        DaemonEvent::DurationChanged { .. } => &["Metadata"],
        _ => return,
    };
    let mut changed = HashMap::new();
    for name in names {
        changed.insert((*name).to_string(), Value::new(0u8));
    }
    let invalidated: Vec<String> = Vec::new();
    if let Err(e) = conn
        .emit_signal(
            None::<&str>,
            OBJECT_PATH,
            PROPERTY_IFACE,
            "PropertiesChanged",
            &(changed, invalidated),
        )
        .await
    {
        warn!("mpris: failed to emit PropertiesChanged: {e}");
    }
}
