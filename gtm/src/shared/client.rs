// Copyright (c) 2026
// Author: prjctimg <prjctimg@outlook.com>
// IPC client: async daemon communication over Unix sockets
//
// This is free software released under the GPL-3.0 license.

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::UnixStream;
use tokio::sync::{Mutex, mpsc, oneshot};

use crate::shared::CoreError;
use crate::shared::Result;
use crate::shared::global::{
    DaemonState, EqPreset, LoudnessMode, PlaybackStatus, RepeatMode, YTFilter,
};
use crate::shared::ipc::{
    CacheKind, DaemonEvent, DaemonReq, DaemonRes, HealthReport, LibraryAction, MetadataPatch,
    QueueAction, SyncKind, WireRes,
};
use crate::shared::log::log;
use crate::shared::playlist::PlaylistFormatKind;
use crate::shared::podcast::{PodcastEpisode, PodcastFeed, PodcastStatus};
use crate::shared::radio::{RadioCountry, RadioStation, RadioTag};
use crate::shared::spotify::{SpotifyPlaylist, SpotifyStatus, SpotifyTrack};
use crate::shared::track;
use crate::shared::wire;

/// Map a response that did not match the awaited variant into an error.
fn unexpected(res: &DaemonRes) -> CoreError {
    CoreError::Daemon(format!("unexpected response: {res:?}"))
}

/// Snapshot of a background library sync operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub struct LibrarySyncStatus {
    pub running: bool,
    pub kind: SyncKind,
    pub synced: usize,
    pub total: usize,
}

struct PendingRequest {
    req: DaemonReq,
    response_tx: Option<oneshot::Sender<Result<DaemonRes>>>,
}

#[derive(Clone)]
pub struct DaemonClient {
    cmd_tx: mpsc::UnboundedSender<PendingRequest>,
    events: Arc<Mutex<Vec<DaemonEvent>>>,
    /// Reusable buffer for draining events, avoiding per-frame allocation.
    drain_buf: Arc<Mutex<Vec<DaemonEvent>>>,
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
                    let events: Arc<Mutex<Vec<DaemonEvent>>> = Arc::new(Mutex::new(Vec::new()));
                    let connected = Arc::new(AtomicBool::new(true));

                    let heartbeat_at = Arc::new(std::sync::Mutex::new(Instant::now()));
                    let hb_pulse = heartbeat_at.clone();

                    let worker = IpcWorker {
                        reader,
                        writer,
                        cmd_rx,
                        connected: connected.clone(),
                        buf: Vec::with_capacity(4096),
                        socket_path: path.clone(),
                        last_heartbeat_at: heartbeat_at,
                        pending: HashMap::new(),
                        next_id: 0,
                    };
                    // Spawn worker before constructing the client handle so
                    // requests can flow immediately on connect.
                    let client = Self {
                        cmd_tx: cmd_tx.clone(),
                        events: events.clone(),
                        drain_buf: Arc::new(Mutex::new(Vec::with_capacity(256))),
                        connected: connected.clone(),
                        base_pos: Arc::new(Mutex::new(0.0)),
                        base_time: Arc::new(Mutex::new(None)),
                        is_playing: Arc::new(AtomicBool::new(false)),
                    };
                    tokio::spawn(worker.run());
                    connected.store(true, Ordering::Release);

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
                    // Exponential backoff: 50ms, 100ms, 200ms, ... up to 2s
                    let delay = 50u64.saturating_mul(1u64 << i).min(2000);
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
        let mut buf = self.drain_buf.lock().await;
        buf.clear();
        buf.extend(events.drain(..cap));
        drop(events);
        self.apply_clock_events(&buf).await;
        std::mem::take(&mut *buf)
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
        if self.is_playing.load(Ordering::Acquire)
            && let Some(base_time) = *self.base_time.lock().await
        {
            let elapsed = base_time.elapsed().as_secs_f64();
            return base_pos + elapsed;
        }
        base_pos
    }

    /// Seed the clock-skewing state from a full daemon state snapshot
    /// (e.g. after `GetStatus` on reconnect).  This ensures the position
    /// estimate is correct before the first event arrives.
    pub async fn seed_clock(&self, state: &DaemonState) {
        let is_playing = state.status == PlaybackStatus::Playing;
        *self.base_pos.lock().await = state.time_pos;
        *self.base_time.lock().await = if is_playing {
            Some(Instant::now())
        } else {
            None
        };
        self.is_playing.store(is_playing, Ordering::Release);
    }

    async fn send_raw(&self, req: DaemonReq) -> Result<DaemonRes> {
        let (tx, rx) = oneshot::channel();
        // Network-backed commands (yt-dlp resolve/download, feed refresh,
        // Spotify sync) legitimately take minutes; everything else must answer
        // fast so a wedged daemon surfaces quickly instead of hanging the UI.
        let timeout_secs = match &req {
            DaemonReq::YtResolveStream { .. }
            | DaemonReq::YtDownload { .. }
            | DaemonReq::YtSearch { .. }
            | DaemonReq::YtFetchPlaylist { .. }
            | DaemonReq::SpotifySync
            | DaemonReq::SpotifyResolve { .. }
            | DaemonReq::SpotifyResolveTrack { .. } => 200,
            _ => IPC_TIMEOUT_SECS,
        };
        self.cmd_tx
            .send(PendingRequest {
                req,
                response_tx: Some(tx),
            })
            .map_err(|_| CoreError::Daemon("IPC worker died".into()))?;
        tokio::time::timeout(Duration::from_secs(timeout_secs), rx)
            .await
            .map_err(|_| CoreError::Daemon("IPC response timeout".into()))?
            .map_err(|_| CoreError::Daemon("IPC worker response dropped".into()))?
    }

    async fn send_ok(&self, req: DaemonReq) -> Result<()> {
        let cmd = req.cmd_name().to_string();
        match self.send_raw(req).await? {
            DaemonRes::Ok => Ok(()),
            DaemonRes::Error { message, .. } => Err(CoreError::Daemon(message)),
            other => {
                let msg = format!("unexpected response to {cmd}: {other:?}");
                tracing::warn!("{msg}");
                Err(CoreError::Daemon(msg))
            }
        }
    }

    /// Send a request whose response is expected to be `DaemonRes::Playlists`
    /// (e.g. `CreatePlaylist`, `ImportPlaylist`). Returns the created/imported
    /// playlists so callers can attach tracks without a second round-trip.
    async fn send_playlists(&self, req: DaemonReq) -> Result<Vec<track::Playlist>> {
        let cmd = req.cmd_name().to_string();
        match self.send_raw(req).await? {
            DaemonRes::Playlists { playlists } => Ok(playlists),
            DaemonRes::Error { message, .. } => Err(CoreError::Daemon(message)),
            other => {
                let msg = format!("unexpected response to {cmd}: {other:?}");
                tracing::warn!("{msg}");
                Err(CoreError::Daemon(msg))
            }
        }
    }

    // ─── Playback ───

    pub async fn play(&self, path: &str, start_pos: f64) -> Result<()> {
        self.send_ok(DaemonReq::Play {
            path: path.into(),
            start_pos,
        })
        .await
    }

    pub async fn play_stream(&self, url: &str) -> Result<()> {
        self.send_ok(DaemonReq::PlayStream { url: url.into() })
            .await
    }

    pub async fn play_pause(&self) -> Result<()> {
        self.send_ok(DaemonReq::PlayPause).await
    }

    pub async fn pause(&self) -> Result<()> {
        self.send_ok(DaemonReq::Pause).await
    }

    pub async fn stop(&self) -> Result<()> {
        self.send_ok(DaemonReq::Stop).await
    }

    pub async fn next(&self) -> Result<()> {
        self.send_ok(DaemonReq::Next).await
    }

    pub async fn prev(&self) -> Result<()> {
        self.send_ok(DaemonReq::Prev).await
    }

    pub async fn seek(&self, position_secs: f64) -> Result<()> {
        *self.base_pos.lock().await = position_secs;
        *self.base_time.lock().await = Some(Instant::now());
        self.send_ok(DaemonReq::Seek { position_secs }).await
    }

    pub async fn set_volume(&self, volume: u8) -> Result<()> {
        self.send_ok(DaemonReq::SetVolume { volume }).await
    }

    pub async fn toggle_shuffle(&self) -> Result<()> {
        self.send_ok(DaemonReq::ToggleShuffle).await
    }

    pub async fn cycle_repeat(&self, mode: RepeatMode) -> Result<()> {
        self.send_ok(DaemonReq::CycleRepeat { mode }).await
    }

    pub async fn toggle_mute(&self) -> Result<()> {
        self.send_ok(DaemonReq::ToggleMute).await
    }

    pub async fn set_mono(&self, enabled: bool) -> Result<()> {
        self.send_ok(DaemonReq::SetMono { enabled }).await
    }

    pub async fn toggle_mono(&self) -> Result<()> {
        let st = self.get_status().await?;
        self.send_ok(DaemonReq::SetMono { enabled: !st.mono }).await
    }

    pub async fn set_eq_preset(&self, preset: EqPreset) -> Result<()> {
        self.send_ok(DaemonReq::SetEqPreset { preset }).await
    }

    pub async fn set_eq_enabled(&self, enabled: bool) -> Result<()> {
        self.send_ok(DaemonReq::SetEqEnabled { enabled }).await
    }

    pub async fn set_reverb(&self, enabled: bool, room_size: f32) -> Result<()> {
        self.send_ok(DaemonReq::SetReverb { enabled, room_size })
            .await
    }

    /// Set pitch-preserving playback rate (clamped to 0.25..=2.0 by the daemon).
    pub async fn set_speed(&self, rate: f32) -> Result<()> {
        self.send_ok(DaemonReq::SetSpeed { rate }).await
    }

    /// Current pitch-preserving playback rate.
    pub async fn speed(&self) -> Result<f32> {
        match self.send_raw(DaemonReq::GetSpeed).await? {
            DaemonRes::Value { value } => {
                Ok(value.get("speed").and_then(|v| v.as_f64()).unwrap_or(1.0) as f32)
            }
            _ => Err(CoreError::Daemon("unexpected response to get_speed".into())),
        }
    }

    /// Current low-power mode flag.
    pub async fn low_power(&self) -> Result<bool> {
        match self.send_raw(DaemonReq::GetLowPower).await? {
            DaemonRes::Value { value } => Ok(value
                .get("low_power")
                .and_then(|v| v.as_bool())
                .unwrap_or(false)),
            _ => Err(CoreError::Daemon(
                "unexpected response to get_low_power".into(),
            )),
        }
    }

    /// List available output device names.
    pub async fn list_audio_devices(&self) -> Result<Vec<String>> {
        match self.send_raw(DaemonReq::ListAudioDevices).await? {
            DaemonRes::Value { value } => Ok(value
                .get("devices")
                .and_then(|v| v.as_array())
                .map(|a| {
                    a.iter()
                        .filter_map(|x| x.as_str().map(String::from))
                        .collect()
                })
                .unwrap_or_default()),
            _ => Err(CoreError::Daemon(
                "unexpected response to list_audio_devices".into(),
            )),
        }
    }

    /// Switch the active output device (`None` restores the system default).
    /// This stops playback and restarts the audio output.
    pub async fn set_audio_device(&self, name: Option<String>) -> Result<()> {
        self.send_ok(DaemonReq::SetAudioDevice { name }).await
    }

    /// Switch the live cover-art provider used by the daemon without a
    /// restart. The TUI persists the choice in config.toml separately.
    pub async fn set_cover_provider(&self, provider: &str) -> Result<()> {
        self.send_ok(DaemonReq::SetCoverProvider {
            provider: provider.to_string(),
        })
        .await
    }

    /// Change the daemon's on-disk cover cache budget and prune to fit.
    pub async fn set_cover_cache(&self, bytes: u64) -> Result<()> {
        self.send_ok(DaemonReq::SetCoverCache { bytes }).await
    }

    /// Current cover cache usage, for the Settings display.
    pub async fn cover_cache_stat(&self) -> Result<(u64, u64, u64)> {
        let res = self.send_raw(DaemonReq::GetCoverCacheStat).await?;
        match res {
            DaemonRes::CoverCacheStat {
                disk_bytes,
                mem_bytes,
                cap_bytes,
            } => Ok((disk_bytes, mem_bytes, cap_bytes)),
            DaemonRes::Error { message, .. } => Err(CoreError::Daemon(message)),
            _ => Err(unexpected(&res)),
        }
    }

    pub async fn crossfade(&self, enabled: bool, duration_secs: u8) -> Result<()> {
        self.send_ok(DaemonReq::Crossfade {
            enabled,
            duration_secs,
        })
        .await
    }

    pub async fn set_loudness_mode(&self, mode: LoudnessMode) -> Result<()> {
        self.send_ok(DaemonReq::SetLoudnessMode { mode }).await
    }

    pub async fn scan_loudness(
        &self,
        track_ids: Option<Vec<i64>>,
        force: Option<bool>,
    ) -> Result<()> {
        self.send_ok(DaemonReq::ScanLoudness { track_ids, force })
            .await
    }

    pub async fn set_pre_gain(&self, pre_gain_db: f32) -> Result<()> {
        self.send_ok(DaemonReq::SetPreGain { pre_gain_db }).await
    }

    pub async fn set_gapless(&self, enabled: bool) -> Result<()> {
        self.send_ok(DaemonReq::SetGapless { enabled }).await
    }

    pub async fn set_dynamic_mode(
        &self,
        enabled: bool,
        min_queue_remaining: Option<u32>,
        max_history: Option<u32>,
    ) -> Result<()> {
        self.send_ok(DaemonReq::SetDynamicMode {
            enabled,
            min_queue_remaining,
            max_history,
        })
        .await
    }

    pub async fn set_scrobble(
        &self,
        enabled: bool,
        api_key: Option<String>,
        session_token: Option<String>,
        min_play_secs: Option<u32>,
        min_play_pct: Option<f32>,
    ) -> Result<()> {
        self.send_ok(DaemonReq::SetScrobble {
            enabled,
            api_key,
            session_token,
            min_play_secs,
            min_play_pct,
        })
        .await
    }

    pub async fn set_sleep_timer(&self, minutes: u32, stop_immediately: bool) -> Result<()> {
        self.send_ok(DaemonReq::SetSleepTimer {
            minutes,
            stop_immediately,
        })
        .await
    }

    pub async fn cancel_sleep_timer(&self) -> Result<()> {
        self.send_ok(DaemonReq::CancelSleepTimer).await
    }

    pub async fn set_low_power(&self, enabled: bool) -> Result<()> {
        self.send_ok(DaemonReq::SetLowPower { enabled }).await
    }

    pub async fn clear_cache(&self, what: CacheKind) -> Result<()> {
        self.send_ok(DaemonReq::ClearCache { what }).await
    }

    // ─── Search ───

    pub async fn search(&self, query: &str) -> Result<DaemonRes> {
        self.send_raw(DaemonReq::Search {
            query: query.into(),
        })
        .await
    }

    // ─── Namespaced accessors ───

    pub fn queue(&self) -> Queue<'_> {
        Queue { client: self }
    }

    pub fn library(&self) -> Library<'_> {
        Library { client: self }
    }

    pub fn yt(&self) -> Yt<'_> {
        Yt { client: self }
    }

    pub fn spotify(&self) -> Spotify<'_> {
        Spotify { client: self }
    }

    pub fn podcast(&self) -> Podcast<'_> {
        Podcast { client: self }
    }

    pub fn radio(&self) -> Radio<'_> {
        Radio { client: self }
    }

    pub fn charts(&self) -> Charts<'_> {
        Charts { client: self }
    }

    pub fn lastfm(&self) -> Lastfm<'_> {
        Lastfm { client: self }
    }

    pub fn favourites(&self) -> Favourites<'_> {
        Favourites { client: self }
    }

    pub fn art(&self) -> Art<'_> {
        Art { client: self }
    }

    pub fn lyrics(&self) -> Lyrics<'_> {
        Lyrics { client: self }
    }

    // ─── System ───

    pub async fn get_status(&self) -> Result<DaemonState> {
        let res = self.send_raw(DaemonReq::GetStatus).await?;
        match res {
            DaemonRes::Status { state, .. } => Ok(*state),
            DaemonRes::Error { message, .. } => Err(CoreError::Daemon(message)),
            _ => Err(unexpected(&res)),
        }
    }

    /// Lightweight status snapshot: identical to [`get_status`] but omits the
    /// full `default_list` (library) from the daemon state. Used by the TUI's
    /// periodic background refresh so the daemon does not re-serialize the
    /// whole library every second.
    pub async fn get_status_lite(&self) -> Result<DaemonState> {
        let res = self.send_raw(DaemonReq::GetStatusLite).await?;
        match res {
            DaemonRes::Status { state, .. } => Ok(*state),
            DaemonRes::Error { message, .. } => Err(CoreError::Daemon(message)),
            _ => Err(unexpected(&res)),
        }
    }

    pub async fn ping(&self) -> Result<()> {
        let res = self.send_raw(DaemonReq::Ping).await?;
        match res {
            DaemonRes::Pong => Ok(()),
            DaemonRes::Error { message, .. } => Err(CoreError::Daemon(message)),
            _ => Err(unexpected(&res)),
        }
    }

    pub async fn quit(&self) -> Result<()> {
        self.send_ok(DaemonReq::Quit).await
    }

    pub async fn check_health(&self) -> Result<HealthReport> {
        let res = self.send_raw(DaemonReq::CheckHealth).await?;
        match res {
            DaemonRes::HealthReport { report, .. } => Ok(*report),
            DaemonRes::Error { message, .. } => Err(CoreError::Daemon(message)),
            _ => Err(unexpected(&res)),
        }
    }
}

// ─── Handle structs: grouped IPC namespaces ───

pub struct Queue<'a> {
    client: &'a DaemonClient,
}

impl<'a> Queue<'a> {
    pub async fn list(&self) -> Result<DaemonRes> {
        self.client
            .send_raw(DaemonReq::Queue {
                action: QueueAction::List,
            })
            .await
    }

    pub async fn add(&self, path: &str, position: Option<u64>) -> Result<()> {
        self.client
            .send_ok(DaemonReq::Queue {
                action: QueueAction::Add {
                    paths: vec![path.into()],
                    position,
                },
            })
            .await
    }

    /// Add one or more paths (files or directories, auto-detected by the
    /// daemon) to the queue, optionally at a merged-view position.
    pub async fn add_many(&self, paths: Vec<String>, position: Option<u64>) -> Result<()> {
        self.client
            .send_ok(DaemonReq::Queue {
                action: QueueAction::Add { paths, position },
            })
            .await
    }

    pub async fn clear(&self) -> Result<()> {
        self.client
            .send_ok(DaemonReq::Queue {
                action: QueueAction::Clear,
            })
            .await
    }

    pub async fn remove(&self, index: u64) -> Result<()> {
        self.client
            .send_ok(DaemonReq::Queue {
                action: QueueAction::Remove { index },
            })
            .await
    }

    pub async fn reorder(&self, from: u64, to: u64) -> Result<()> {
        self.client
            .send_ok(DaemonReq::Queue {
                action: QueueAction::Move { from, to },
            })
            .await
    }

    pub async fn set(&self, paths: Vec<String>, start_idx: u64) -> Result<()> {
        self.client
            .send_ok(DaemonReq::Queue {
                action: QueueAction::Set { paths, start_idx },
            })
            .await
    }
}

pub struct Library<'a> {
    client: &'a DaemonClient,
}

impl<'a> Library<'a> {
    pub async fn scan(&self, path: &str) -> Result<()> {
        self.client
            .send_ok(DaemonReq::Library {
                action: LibraryAction::Scan { path: path.into() },
            })
            .await
    }

    pub async fn get_tracks(
        &self,
        filter: Option<String>,
        sort: Option<String>,
    ) -> Result<DaemonRes> {
        self.client
            .send_raw(DaemonReq::Library {
                action: LibraryAction::GetTracks { filter, sort },
            })
            .await
    }

    pub async fn get_playlists(&self) -> Result<DaemonRes> {
        self.client
            .send_raw(DaemonReq::Library {
                action: LibraryAction::GetPlaylists,
            })
            .await
    }

    pub async fn most_played(&self, limit: u64) -> Result<DaemonRes> {
        self.client
            .send_raw(DaemonReq::Library {
                action: LibraryAction::GetMostPlayed { limit },
            })
            .await
    }

    pub async fn recently_played(&self, limit: u64) -> Result<DaemonRes> {
        self.client
            .send_raw(DaemonReq::Library {
                action: LibraryAction::GetRecentlyPlayed { limit },
            })
            .await
    }

    pub async fn recently_added(&self, limit: u64) -> Result<DaemonRes> {
        self.client
            .send_raw(DaemonReq::Library {
                action: LibraryAction::GetRecentlyAdded { limit },
            })
            .await
    }

    pub async fn get_playlist_tracks(&self, playlist_id: i64) -> Result<DaemonRes> {
        self.client
            .send_raw(DaemonReq::Library {
                action: LibraryAction::GetPlaylistTracks { id: playlist_id },
            })
            .await
    }

    pub async fn create_playlist(&self, name: &str) -> Result<Vec<track::Playlist>> {
        self.client
            .send_playlists(DaemonReq::Library {
                action: LibraryAction::CreatePlaylist { name: name.into() },
            })
            .await
    }

    pub async fn delete_playlist(&self, id: i64) -> Result<()> {
        self.client
            .send_ok(DaemonReq::Library {
                action: LibraryAction::DeletePlaylist { id },
            })
            .await
    }

    pub async fn rename_playlist(&self, id: i64, name: &str) -> Result<()> {
        self.client
            .send_ok(DaemonReq::Library {
                action: LibraryAction::RenamePlaylist {
                    id,
                    name: name.into(),
                },
            })
            .await
    }

    pub async fn add_to_playlist(&self, playlist_id: i64, track_ids: Vec<i64>) -> Result<()> {
        self.client
            .send_ok(DaemonReq::Library {
                action: LibraryAction::AddToPlaylist {
                    playlist_id,
                    track_ids,
                },
            })
            .await
    }

    pub async fn import_playlist(
        &self,
        path: &str,
        format: PlaylistFormatKind,
    ) -> Result<Vec<track::Playlist>> {
        self.client
            .send_playlists(DaemonReq::Library {
                action: LibraryAction::ImportPlaylist {
                    path: path.into(),
                    format,
                },
            })
            .await
    }

    pub async fn export_playlist(
        &self,
        playlist_id: i64,
        path: &str,
        format: PlaylistFormatKind,
    ) -> Result<()> {
        self.client
            .send_ok(DaemonReq::Library {
                action: LibraryAction::ExportPlaylist {
                    playlist_id,
                    path: path.into(),
                    format,
                },
            })
            .await
    }

    pub async fn get_recent(&self, count: u64) -> Result<DaemonRes> {
        self.client
            .send_raw(DaemonReq::Library {
                action: LibraryAction::GetRecent { count },
            })
            .await
    }

    pub async fn sync_covers(&self) -> Result<()> {
        self.client
            .send_ok(DaemonReq::Library {
                action: LibraryAction::SyncCovers,
            })
            .await
    }

    pub async fn sync_lyrics(&self) -> Result<()> {
        self.client
            .send_ok(DaemonReq::Library {
                action: LibraryAction::SyncLyrics,
            })
            .await
    }

    /// Enrich unreliable track metadata via Deezer and embed tags into the
    /// files. With `path` given, only that track is processed. The daemon
    /// acknowledges immediately and runs the sync in the background; use
    /// [`Self::sync_status`] to poll for completion.
    pub async fn sync_metadata(&self, path: Option<String>) -> Result<()> {
        self.client
            .send_ok(DaemonReq::Library {
                action: LibraryAction::SyncMetadata { path },
            })
            .await
    }

    /// Poll the progress of a background library sync (covers/lyrics/metadata).
    pub async fn sync_status(&self) -> Result<LibrarySyncStatus> {
        let res = self
            .client
            .send_raw(DaemonReq::Library {
                action: LibraryAction::SyncStatus,
            })
            .await?;
        match res {
            DaemonRes::SyncStatus {
                running,
                kind,
                synced,
                total,
            } => Ok(LibrarySyncStatus {
                running,
                kind,
                synced,
                total,
            }),
            other => Err(CoreError::Daemon(format!(
                "unexpected response to library sync status: {other:?}"
            ))),
        }
    }

    pub async fn remove_from_playlist(&self, playlist_id: i64, track_id: i64) -> Result<()> {
        self.client
            .send_ok(DaemonReq::Library {
                action: LibraryAction::RemoveFromPlaylist {
                    playlist_id,
                    track_id,
                },
            })
            .await
    }

    pub async fn playlist_dedup(&self, playlist_id: i64) -> Result<u64> {
        self.library_removed_count(LibraryAction::PlaylistDedup { playlist_id })
            .await
    }

    pub async fn playlist_doctor(&self, playlist_id: i64) -> Result<u64> {
        self.library_removed_count(LibraryAction::PlaylistDoctor { playlist_id })
            .await
    }

    pub async fn playlist_sort(&self, playlist_id: i64, field: &str) -> Result<()> {
        self.client
            .send_ok(DaemonReq::Library {
                action: LibraryAction::PlaylistSort {
                    playlist_id,
                    field: field.into(),
                },
            })
            .await
    }

    /// Send a library action that returns a `{ "removed": n }` value,
    /// extracting `n`.
    async fn library_removed_count(&self, action: LibraryAction) -> Result<u64> {
        match self.client.send_raw(DaemonReq::Library { action }).await? {
            DaemonRes::Value { value } => {
                Ok(value.get("removed").and_then(|v| v.as_u64()).unwrap_or(0))
            }
            other => Err(CoreError::Daemon(format!(
                "unexpected response to playlist action: {other:?}"
            ))),
        }
    }

    pub async fn remove_track(&self, id: i64) -> Result<()> {
        self.client
            .send_ok(DaemonReq::Library {
                action: LibraryAction::RemoveTrack { id },
            })
            .await
    }

    pub async fn update_metadata(&self, track_id: i64, patch: MetadataPatch) -> Result<()> {
        self.client
            .send_ok(DaemonReq::Library {
                action: LibraryAction::UpdateMetadata { track_id, patch },
            })
            .await
    }
}

pub struct Yt<'a> {
    client: &'a DaemonClient,
}

impl<'a> Yt<'a> {
    pub async fn search(&self, query: &str, filter: Option<YTFilter>) -> Result<DaemonRes> {
        self.client
            .send_raw(DaemonReq::YtSearch {
                query: query.into(),
                filter,
            })
            .await
    }

    pub async fn poll(&self) -> Result<DaemonRes> {
        self.client.send_raw(DaemonReq::YtSearchPoll).await
    }

    pub async fn cancel(&self) -> Result<()> {
        self.client.send_ok(DaemonReq::YtSearchCancel).await
    }

    pub async fn resolve_stream(&self, url: &str) -> Result<DaemonRes> {
        self.client
            .send_raw(DaemonReq::YtResolveStream { url: url.into() })
            .await
    }

    /// Start a daemon-side yt-dlp download of `url`. Returns a download ID to
    /// poll with [`Yt::download_poll`].
    pub async fn download(
        &self,
        url: String,
        title: Option<String>,
        channel: Option<String>,
    ) -> Result<u64> {
        let res = self
            .client
            .send_raw(DaemonReq::YtDownload {
                url,
                title,
                channel,
            })
            .await?;
        match res {
            DaemonRes::Value { value } => value
                .get("id")
                .and_then(|v| v.as_u64())
                .ok_or_else(|| CoreError::Daemon("missing download id".into())),
            DaemonRes::Error { message, .. } => Err(CoreError::Daemon(message)),
            _ => Err(unexpected(&res)),
        }
    }

    /// Poll progress of the current daemon-side download.
    pub async fn download_poll(&self) -> Result<DaemonRes> {
        self.client.send_raw(DaemonReq::YtDownloadPoll).await
    }

    /// Cancel the current daemon-side download.
    pub async fn cancel_download(&self, url: String) -> Result<()> {
        self.client
            .send_ok(DaemonReq::YtCancelDownload { url })
            .await
    }

    /// Start a daemon-side yt-dlp flat-playlist fetch of `url`. Entries are
    /// collected with [`Yt::fetch_playlist_poll`].
    pub async fn fetch_playlist(&self, url: &str) -> Result<()> {
        self.client
            .send_ok(DaemonReq::YtFetchPlaylist { url: url.into() })
            .await
    }

    /// Poll for a finished playlist fetch.
    pub async fn fetch_playlist_poll(&self) -> Result<DaemonRes> {
        self.client.send_raw(DaemonReq::YtFetchPlaylistPoll).await
    }

    pub async fn set_config(
        &self,
        cookie_source: Option<String>,
        cookie_file: Option<String>,
        js_runtime: Option<String>,
        download_dir: Option<String>,
        max_concurrent: Option<u32>,
    ) -> Result<()> {
        self.client
            .send_ok(DaemonReq::YtSetConfig {
                cookie_source,
                cookie_file,
                js_runtime,
                download_dir,
                max_concurrent,
            })
            .await
    }
}

pub struct Spotify<'a> {
    client: &'a DaemonClient,
}

impl<'a> Spotify<'a> {
    /// Link a Spotify account from a token (plain access token or full JSON)
    /// and refresh the playlist cache.
    pub async fn set_token(&self, token: &str) -> Result<SpotifyStatus> {
        let res = self
            .client
            .send_raw(DaemonReq::SpotifySetToken {
                token: token.into(),
            })
            .await?;
        Self::status_from(res)
    }

    /// Start the OAuth PKCE link flow. Returns the authorize URL the user
    /// must open in a browser; completion is signalled via the
    /// `spotify_status_changed` daemon event. `port` selects the local
    /// redirect port so a previously-registered Spotify dashboard URI works.
    pub async fn oauth_start(&self, client_id: &str, port: u16) -> Result<String> {
        let res = self
            .client
            .send_raw(DaemonReq::SpotifyOauthStart {
                client_id: client_id.into(),
                port,
            })
            .await?;
        match res {
            DaemonRes::SpotifyOauthStarted { url } => Ok(url),
            DaemonRes::Error { message, .. } => Err(CoreError::Daemon(message)),
            _ => Err(unexpected(&res)),
        }
    }

    /// Abort a pending OAuth link flow (shuts down the callback server).
    pub async fn oauth_cancel(&self) -> Result<()> {
        self.client.send_ok(DaemonReq::SpotifyCancelOauth).await
    }

    /// Unlink the Spotify account and delete the token file.
    pub async fn clear(&self) -> Result<SpotifyStatus> {
        let res = self.client.send_raw(DaemonReq::SpotifyClear).await?;
        Self::status_from(res)
    }

    /// Current link status (linked user, playlist/track counts, last error).
    pub async fn status(&self) -> Result<SpotifyStatus> {
        let res = self.client.send_raw(DaemonReq::SpotifyStatus).await?;
        Self::status_from(res)
    }

    /// Toggle play/pause on the active Spotify device (Premium required).
    pub async fn play_pause(&self) -> Result<SpotifyStatus> {
        let res = self.client.send_raw(DaemonReq::SpotifyPlayPause).await?;
        Self::status_from(res)
    }

    /// Skip to the next track on the active Spotify device.
    pub async fn next(&self) -> Result<SpotifyStatus> {
        let res = self.client.send_raw(DaemonReq::SpotifyNext).await?;
        Self::status_from(res)
    }

    /// Skip to the previous track on the active Spotify device.
    pub async fn previous(&self) -> Result<SpotifyStatus> {
        let res = self.client.send_raw(DaemonReq::SpotifyPrevious).await?;
        Self::status_from(res)
    }

    /// Seek the active Spotify device to `pos_secs`.
    pub async fn seek(&self, pos_secs: u32) -> Result<SpotifyStatus> {
        let res = self
            .client
            .send_raw(DaemonReq::SpotifySeek { pos_secs })
            .await?;
        Self::status_from(res)
    }

    /// Toggle shuffle on the active Spotify device.
    pub async fn set_shuffle(&self, on: bool) -> Result<SpotifyStatus> {
        let res = self
            .client
            .send_raw(DaemonReq::SpotifyShuffle { on })
            .await?;
        Self::status_from(res)
    }

    /// Set repeat on the active Spotify device (`off`/`track`/`context`).
    pub async fn set_repeat(&self, mode: &str) -> Result<SpotifyStatus> {
        let res = self
            .client
            .send_raw(DaemonReq::SpotifyRepeat {
                mode: mode.to_string(),
            })
            .await?;
        Self::status_from(res)
    }

    /// Set the active Spotify device's volume (0-100).
    pub async fn set_volume(&self, percent: u8) -> Result<SpotifyStatus> {
        let res = self
            .client
            .send_raw(DaemonReq::SpotifyVolume { percent })
            .await?;
        Self::status_from(res)
    }

    /// Re-sync all playlists from the Spotify Web API.
    pub async fn sync(&self) -> Result<()> {
        self.client.send_ok(DaemonReq::SpotifySync).await
    }

    /// The cached playlist list (with tracks embedded).
    pub async fn playlists(&self) -> Result<Vec<SpotifyPlaylist>> {
        let res = self.client.send_raw(DaemonReq::SpotifyPlaylists).await?;
        match res {
            DaemonRes::SpotifyPlaylistsRes { playlists, .. } => Ok(playlists),
            DaemonRes::Error { message, .. } => Err(CoreError::Daemon(message)),
            _ => Err(unexpected(&res)),
        }
    }

    /// Cached tracks of a single playlist.
    pub async fn playlist_tracks(&self, id: &str) -> Result<Vec<SpotifyTrack>> {
        let res = self
            .client
            .send_raw(DaemonReq::SpotifyPlaylistTracks { id: id.into() })
            .await?;
        match res {
            DaemonRes::SpotifyTracksRes { tracks, .. } => Ok(tracks),
            DaemonRes::Error { message, .. } => Err(CoreError::Daemon(message)),
            _ => Err(unexpected(&res)),
        }
    }

    /// Resolve a Spotify playlist track to a playable local stream and append
    /// it to the user queue. With `play` it starts playing immediately
    /// (switching the active source) instead of only queueing.
    pub async fn resolve(&self, playlist_id: &str, track_index: usize, play: bool) -> Result<()> {
        self.client
            .send_ok(DaemonReq::SpotifyResolve {
                playlist_id: playlist_id.into(),
                track_index,
                play,
            })
            .await
    }

    /// Search the Spotify Web API for tracks matching a query.
    pub async fn search_web(&self, query: &str) -> Result<Vec<SpotifyTrack>> {
        let res = self
            .client
            .send_raw(DaemonReq::SpotifySearchWeb {
                query: query.into(),
            })
            .await?;
        match res {
            DaemonRes::SpotifyTracksRes { tracks, .. } => Ok(tracks),
            DaemonRes::Error { message, .. } => Err(CoreError::Daemon(message)),
            _ => Err(unexpected(&res)),
        }
    }

    /// Resolve a web-search album result to its full track list.
    pub async fn album_tracks(&self, uri: &str) -> Result<Vec<SpotifyTrack>> {
        let res = self
            .client
            .send_raw(DaemonReq::SpotifyAlbumTracks { uri: uri.into() })
            .await?;
        match res {
            DaemonRes::SpotifyTracksRes { tracks, .. } => Ok(tracks),
            DaemonRes::Error { message, .. } => Err(CoreError::Daemon(message)),
            _ => Err(unexpected(&res)),
        }
    }

    /// Resolve a web-search artist result to their top tracks.
    pub async fn artist_top_tracks(&self, uri: &str) -> Result<Vec<SpotifyTrack>> {
        let res = self
            .client
            .send_raw(DaemonReq::SpotifyArtistTopTracks { uri: uri.into() })
            .await?;
        match res {
            DaemonRes::SpotifyTracksRes { tracks, .. } => Ok(tracks),
            DaemonRes::Error { message, .. } => Err(CoreError::Daemon(message)),
            _ => Err(unexpected(&res)),
        }
    }

    /// Resolve a web-search playlist result to its full track list.
    pub async fn web_playlist_tracks(&self, uri: &str) -> Result<Vec<SpotifyTrack>> {
        let res = self
            .client
            .send_raw(DaemonReq::SpotifyWebPlaylistTracks { uri: uri.into() })
            .await?;
        match res {
            DaemonRes::SpotifyTracksRes { tracks, .. } => Ok(tracks),
            DaemonRes::Error { message, .. } => Err(CoreError::Daemon(message)),
            _ => Err(unexpected(&res)),
        }
    }

    /// Resolve a Spotify track (by metadata) to a playable stream and append
    /// it to the user queue. With a known `spotify:track:` URI on a Premium
    /// account the track streams natively via librespot instead of a YouTube
    /// match. With `play` it starts playing immediately (switching the active
    /// source) instead of only queueing.
    pub async fn resolve_track(
        &self,
        name: &str,
        artists: &str,
        album: &str,
        uri: Option<String>,
        play: bool,
    ) -> Result<()> {
        self.client
            .send_ok(DaemonReq::SpotifyResolveTrack {
                name: name.into(),
                artists: artists.into(),
                album: album.into(),
                uri,
                play,
            })
            .await
    }

    /// Play every track of a synced Spotify playlist. With `shuffle` the
    /// track order is randomised before enqueueing (i.e. dash `S`).
    pub async fn play_all(&self, playlist_id: &str, shuffle: bool) -> Result<()> {
        self.client
            .send_ok(DaemonReq::SpotifyPlayAll {
                playlist_id: playlist_id.into(),
                shuffle,
            })
            .await
    }

    /// Fetch the raw bytes of a Spotify album-cover URL as base64, so the
    /// client can render it without proxying through Spotify's CDN itself.
    pub async fn track_image(&self, image_url: &str) -> Result<Option<String>> {
        let res = self
            .client
            .send_raw(DaemonReq::SpotifyTrackImage {
                image_url: image_url.into(),
            })
            .await?;
        match res {
            DaemonRes::SpotifyImageRes { data, .. } => Ok(data),
            DaemonRes::Error { message, .. } => Err(CoreError::Daemon(message)),
            _ => Err(unexpected(&res)),
        }
    }

    fn status_from(res: DaemonRes) -> Result<SpotifyStatus> {
        match res {
            DaemonRes::SpotifyStatusRes { status, .. } => Ok(status),
            DaemonRes::Error { message, .. } => Err(CoreError::Daemon(message)),
            _ => Err(unexpected(&res)),
        }
    }
}

/// Client helpers for the Top Charts feature (provider-agnostic).
pub struct Charts<'a> {
    client: &'a DaemonClient,
}

impl<'a> Charts<'a> {
    /// List available chart sources (Spotify, Apple Music; new providers plug
    /// in via the `ChartProvider` trait and appear here automatically).
    pub async fn sources(&self) -> Result<Vec<crate::shared::chart::ChartSource>> {
        let res = self.client.send_raw(DaemonReq::ChartsSources).await?;
        match res {
            DaemonRes::ChartsSourcesRes { sources, .. } => Ok(sources),
            DaemonRes::Error { message, .. } => Err(CoreError::Daemon(message)),
            _ => Err(unexpected(&res)),
        }
    }

    /// List charts from a specific source or all configured sources.
    pub async fn list(
        &self,
        source_id: Option<String>,
    ) -> Result<Vec<crate::shared::chart::ChartPlaylist>> {
        let res = self
            .client
            .send_raw(DaemonReq::ChartsList { source_id })
            .await?;
        match res {
            DaemonRes::ChartsListRes { charts, .. } => Ok(charts),
            DaemonRes::Error { message, .. } => Err(CoreError::Daemon(message)),
            _ => Err(unexpected(&res)),
        }
    }

    /// Fetch tracks for a specific chart playlist.
    pub async fn tracks(
        &self,
        source_id: String,
        chart_id: String,
    ) -> Result<Vec<crate::shared::chart::ChartTrack>> {
        let res = self
            .client
            .send_raw(DaemonReq::ChartsTracks {
                source_id,
                chart_id,
            })
            .await?;
        match res {
            DaemonRes::ChartsTracksRes { tracks, .. } => Ok(tracks),
            DaemonRes::Error { message, .. } => Err(CoreError::Daemon(message)),
            _ => Err(unexpected(&res)),
        }
    }
}

/// Client helpers for podcast subscriptions and episodes.
pub struct Podcast<'a> {
    client: &'a DaemonClient,
}

impl<'a> Podcast<'a> {
    pub async fn add_feed(&self, url: &str) -> Result<Vec<PodcastFeed>> {
        let res = self
            .client
            .send_raw(DaemonReq::PodcastAddFeed { url: url.into() })
            .await?;
        match res {
            DaemonRes::PodcastFeedsRes { feeds, .. } => Ok(feeds),
            DaemonRes::Error { message, .. } => Err(CoreError::Daemon(message)),
            _ => Err(unexpected(&res)),
        }
    }

    pub async fn remove_feed(&self, feed_id: &str) -> Result<()> {
        self.client
            .send_ok(DaemonReq::PodcastRemoveFeed {
                feed_id: feed_id.into(),
            })
            .await
    }

    pub async fn feeds(&self) -> Result<Vec<PodcastFeed>> {
        let res = self.client.send_raw(DaemonReq::PodcastFeeds).await?;
        match res {
            DaemonRes::PodcastFeedsRes { feeds, .. } => Ok(feeds),
            DaemonRes::Error { message, .. } => Err(CoreError::Daemon(message)),
            _ => Err(unexpected(&res)),
        }
    }

    pub async fn episodes(&self, feed_id: &str) -> Result<(String, Vec<PodcastEpisode>)> {
        let res = self
            .client
            .send_raw(DaemonReq::PodcastEpisodes {
                feed_id: feed_id.into(),
            })
            .await?;
        match res {
            DaemonRes::PodcastEpisodesRes {
                feed_title,
                episodes,
                ..
            } => Ok((feed_title, episodes)),
            DaemonRes::Error { message, .. } => Err(CoreError::Daemon(message)),
            _ => Err(unexpected(&res)),
        }
    }

    pub async fn refresh(&self, feed_id: Option<&str>) -> Result<usize> {
        let res = self
            .client
            .send_raw(DaemonReq::PodcastRefresh {
                feed_id: feed_id.map(|s| s.to_string()),
            })
            .await?;
        match res {
            DaemonRes::PodcastFeedsRes { feeds, .. } => Ok(feeds.len()),
            DaemonRes::Value { value } => {
                Ok(value.get("refreshed").and_then(|v| v.as_u64()).unwrap_or(0) as usize)
            }
            DaemonRes::Error { message, .. } => Err(CoreError::Daemon(message)),
            _ => Err(unexpected(&res)),
        }
    }

    pub async fn status(&self) -> Result<PodcastStatus> {
        let res = self.client.send_raw(DaemonReq::PodcastStatus).await?;
        match res {
            DaemonRes::PodcastStatusRes { status, .. } => Ok(status),
            DaemonRes::Error { message, .. } => Err(CoreError::Daemon(message)),
            _ => Err(unexpected(&res)),
        }
    }

    pub async fn play(&self, feed_id: &str, episode_index: usize) -> Result<()> {
        self.client
            .send_ok(DaemonReq::PodcastPlay {
                feed_id: feed_id.into(),
                episode_index,
            })
            .await
    }
}

/// Client helpers for the Radio Browser directory.
pub struct Radio<'a> {
    client: &'a DaemonClient,
}

impl<'a> Radio<'a> {
    pub async fn search(&self, query: &str, limit: u16) -> Result<Vec<RadioStation>> {
        let res = self
            .client
            .send_raw(DaemonReq::RadioSearch {
                query: query.into(),
                limit,
            })
            .await?;
        match res {
            DaemonRes::RadioStationsRes { stations, .. } => Ok(stations),
            DaemonRes::Error { message, .. } => Err(CoreError::Daemon(message)),
            _ => Err(unexpected(&res)),
        }
    }

    pub async fn top(&self, limit: u16) -> Result<Vec<RadioStation>> {
        let res = self.client.send_raw(DaemonReq::RadioTop { limit }).await?;
        match res {
            DaemonRes::RadioStationsRes { stations, .. } => Ok(stations),
            DaemonRes::Error { message, .. } => Err(CoreError::Daemon(message)),
            _ => Err(unexpected(&res)),
        }
    }

    pub async fn play(&self, station_id: &str, station_name: &str) -> Result<()> {
        self.client
            .send_ok(DaemonReq::RadioPlay {
                station_id: station_id.into(),
                station_name: station_name.into(),
            })
            .await
    }

    pub async fn tags(&self, limit: u16) -> Result<Vec<RadioTag>> {
        let res = self.client.send_raw(DaemonReq::RadioTags { limit }).await?;
        match res {
            DaemonRes::RadioTagsRes { tags, .. } => Ok(tags),
            DaemonRes::Error { message, .. } => Err(CoreError::Daemon(message)),
            _ => Err(unexpected(&res)),
        }
    }

    pub async fn countries(&self, limit: u16) -> Result<Vec<RadioCountry>> {
        let res = self
            .client
            .send_raw(DaemonReq::RadioCountries { limit })
            .await?;
        match res {
            DaemonRes::RadioCountriesRes { countries, .. } => Ok(countries),
            DaemonRes::Error { message, .. } => Err(CoreError::Daemon(message)),
            _ => Err(unexpected(&res)),
        }
    }

    pub async fn stations_by_tag(&self, tag: &str, limit: u16) -> Result<Vec<RadioStation>> {
        let res = self
            .client
            .send_raw(DaemonReq::RadioByTag {
                tag: tag.into(),
                limit,
            })
            .await?;
        match res {
            DaemonRes::RadioStationsRes { stations, .. } => Ok(stations),
            DaemonRes::Error { message, .. } => Err(CoreError::Daemon(message)),
            _ => Err(unexpected(&res)),
        }
    }

    pub async fn stations_by_country(
        &self,
        country: &str,
        limit: u16,
    ) -> Result<Vec<RadioStation>> {
        let res = self
            .client
            .send_raw(DaemonReq::RadioByCountry {
                country: country.into(),
                limit,
            })
            .await?;
        match res {
            DaemonRes::RadioStationsRes { stations, .. } => Ok(stations),
            DaemonRes::Error { message, .. } => Err(CoreError::Daemon(message)),
            _ => Err(unexpected(&res)),
        }
    }
}

/// Last.fm scrobbling configuration and web-auth flow.
pub struct Lastfm<'a> {
    client: &'a DaemonClient,
}

impl<'a> Lastfm<'a> {
    /// Persist API credentials, optionally enable scrobbling, and (re)initialize
    /// the daemon-side manager. Blank key/secret/session values keep whatever
    /// is already configured or stored in the keychain.
    pub async fn set_config(
        &self,
        enabled: bool,
        api_key: Option<String>,
        api_secret: Option<String>,
        session_key: Option<String>,
        min_play_secs: Option<u32>,
        min_play_pct: Option<f32>,
    ) -> Result<()> {
        self.client
            .send_ok(DaemonReq::LastfmSetConfig {
                enabled,
                api_key,
                api_secret,
                session_key,
                min_play_secs,
                min_play_pct,
            })
            .await
    }

    /// Authorization URL the user opens to grant gtm access to their account.
    pub async fn auth_url(&self) -> Result<String> {
        let res = self.client.send_raw(DaemonReq::LastfmAuthUrl).await?;
        match res {
            DaemonRes::LastfmAuthUrlRes { url } => Ok(url),
            DaemonRes::Error { message, .. } => Err(CoreError::Daemon(message)),
            _ => Err(unexpected(&res)),
        }
    }

    /// Start the Last.fm OAuth browser flow. The daemon binds the callback
    /// port *before* returning the authorize URL (so the redirect never lands
    /// on a dead port), captures the returning `token`, exchanges it, and
    /// pushes a status event the TUI reacts to. Returns the authorize URL to
    /// open. Identical hook shape to `Spotify::oauth_start`.
    pub async fn oauth_start(&self, port: u16) -> Result<String> {
        let res = self
            .client
            .send_raw(DaemonReq::LastfmOauthStart { port })
            .await?;
        match res {
            DaemonRes::LastfmAuthUrlRes { url } => Ok(url),
            DaemonRes::Error { message, .. } => Err(CoreError::Daemon(message)),
            _ => Err(unexpected(&res)),
        }
    }

    /// Exchange a web-auth token for a (persisted) session key.
    pub async fn authenticate(&self, token: &str) -> Result<()> {
        self.client
            .send_ok(DaemonReq::LastfmAuthenticate {
                token: token.into(),
            })
            .await
    }

    pub async fn status(&self) -> Result<LastfmStatus> {
        let res = self.client.send_raw(DaemonReq::LastfmStatus).await?;
        match res {
            DaemonRes::LastfmStatusRes {
                enabled,
                api_key,
                session_token,
                ready,
                loved,
                error,
            } => Ok(LastfmStatus {
                enabled,
                api_key,
                session_token,
                ready,
                loved,
                error,
            }),
            DaemonRes::Error { message, .. } => Err(CoreError::Daemon(message)),
            _ => Err(unexpected(&res)),
        }
    }

    /// Unlink the account: clear the session and stored API credentials.
    pub async fn clear(&self) -> Result<()> {
        self.client.send_ok(DaemonReq::LastfmClear).await
    }

    /// Love the currently playing track on Last.fm. The daemon also submits an
    /// immediate scrobble for the active play session so a loved track is
    /// never lost on a quick skip.
    pub async fn love(&self) -> Result<()> {
        self.client.send_ok(DaemonReq::LastfmLove).await
    }

    /// Un-love the currently playing track on Last.fm.
    pub async fn unlove(&self) -> Result<()> {
        self.client.send_ok(DaemonReq::LastfmUnlove).await
    }
}

/// Last.fm configuration and link state.
#[derive(Debug, Clone)]
pub struct LastfmStatus {
    pub enabled: bool,
    pub api_key: Option<String>,
    pub session_token: Option<String>,
    pub ready: bool,
    pub loved: bool,
    /// Link failure surfaced through the status poll (callback timeout,
    /// token-exchange error); `None` when idle or linked.
    pub error: Option<String>,
}

pub struct Favourites<'a> {
    client: &'a DaemonClient,
}

impl<'a> Favourites<'a> {
    pub async fn list(&self) -> Result<DaemonRes> {
        self.client.send_raw(DaemonReq::GetFavourites).await
    }

    pub async fn add(&self, track_id: i64) -> Result<()> {
        self.client
            .send_ok(DaemonReq::AddFavourite { track_id })
            .await
    }

    pub async fn remove(&self, track_id: i64) -> Result<()> {
        self.client
            .send_ok(DaemonReq::RemoveFavourite { track_id })
            .await
    }
}

pub struct Art<'a> {
    client: &'a DaemonClient,
}

impl<'a> Art<'a> {
    pub async fn cover(&self, track_id: i64) -> Result<Option<String>> {
        self.cover_for(track_id, None).await
    }

    pub async fn cover_for(
        &self,
        track_id: i64,
        cover_path: Option<String>,
    ) -> Result<Option<String>> {
        let res = self
            .client
            .send_raw(DaemonReq::GetCoverArt {
                track_id,
                path: cover_path,
            })
            .await?;
        match res {
            DaemonRes::CoverArt { data, .. } => Ok(data),
            DaemonRes::Error { message, .. } => Err(CoreError::Daemon(message)),
            _ => Err(unexpected(&res)),
        }
    }

    pub async fn artist_cover(&self, artist: String) -> Result<Option<String>> {
        let res = self
            .client
            .send_raw(DaemonReq::GetArtistCoverArt { artist })
            .await?;
        match res {
            DaemonRes::CoverArt { data, .. } => Ok(data),
            DaemonRes::Error { message, .. } => Err(CoreError::Daemon(message)),
            _ => Err(unexpected(&res)),
        }
    }
}

pub struct Lyrics<'a> {
    client: &'a DaemonClient,
}

impl<'a> Lyrics<'a> {
    pub async fn get(&self, track_id: i64, path: Option<&str>) -> Result<Option<track::LrcData>> {
        let res = self
            .client
            .send_raw(DaemonReq::GetLyrics {
                track_id,
                path: path.map(str::to_string),
            })
            .await?;
        match res {
            DaemonRes::Lyrics { lyrics, .. } => Ok(lyrics),
            DaemonRes::Error { message, .. } => Err(CoreError::Daemon(message)),
            _ => Err(unexpected(&res)),
        }
    }

    /// Fetch lyrics for a free-form artist/title pair (no track id or path).
    pub async fn search(&self, artist: &str, title: &str) -> Result<Option<track::LrcData>> {
        let res = self
            .client
            .send_raw(DaemonReq::LyricsSearch {
                artist: artist.into(),
                title: title.into(),
            })
            .await?;
        match res {
            DaemonRes::Lyrics { lyrics, .. } => Ok(lyrics),
            DaemonRes::Error { message, .. } => Err(CoreError::Daemon(message)),
            _ => Err(unexpected(&res)),
        }
    }
}

struct IpcWorker {
    reader: tokio::net::unix::OwnedReadHalf,
    writer: tokio::net::unix::OwnedWriteHalf,
    cmd_rx: mpsc::UnboundedReceiver<PendingRequest>,
    connected: Arc<AtomicBool>,
    buf: Vec<u8>,
    socket_path: std::path::PathBuf,
    last_heartbeat_at: Arc<std::sync::Mutex<Instant>>,
    pending: HashMap<u64, (String, oneshot::Sender<Result<DaemonRes>>)>,
    next_id: u64,
}

const HEARTBEAT_TIMEOUT_SECS: u64 = 60;
/// Upper bound on how long a single IPC command may take before the client
/// reports the daemon as unresponsive. Connection liveness is handled by the
/// heartbeat timeout (see HEARTBEAT_TIMEOUT_SECS), so this only needs to
/// cover legitimate heavy operations (playlist rotation, playback startup,
/// metadata resolution) which can exceed a few seconds on large libraries.
const IPC_TIMEOUT_SECS: u64 = 30;

impl IpcWorker {
    async fn run(mut self) {
        let mut tmp = [0u8; 4096];
        loop {
            // Heartbeat check: if no heartbeat received within timeout,
            // the daemon or connection is stale: force reconnect immediately.
            if self.last_heartbeat_at.lock().unwrap().elapsed()
                > Duration::from_secs(HEARTBEAT_TIMEOUT_SECS)
            {
                log(&format!(
                    "IPC worker: no heartbeat for {}s, forcing reconnect",
                    HEARTBEAT_TIMEOUT_SECS,
                ));
                self.fail_all_pending("heartbeat timeout");
                self.reconnect().await;
                *self.last_heartbeat_at.lock().unwrap() = Instant::now();
                continue;
            }

            // Drain pending requests from the channel and send them.
            let mut sent_any = false;
            while let Ok(pending) = self.cmd_rx.try_recv() {
                let id = self.next_id;
                self.next_id = self.next_id.wrapping_add(1);
                if let Err(e) = self.send_by_id(id, &pending).await {
                    log(&format!("IPC worker send error: {e}"));
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
            if sent_any
                && !self.pending.is_empty()
                && let Err(e) =
                    tokio::time::timeout(Duration::from_secs(5), self.writer.flush()).await
            {
                log(&format!("IPC worker flush error: {e}"));
                self.fail_all_pending("flush failed");
                self.reconnect().await;
                continue;
            }

            // Read from socket with a small timeout so we can loop back
            // to check for requests.
            match self.read_with_timeout(&mut tmp).await {
                Ok(true) => {
                    // Parse all complete frames, dispatching responses by ID
                    while self.parse_next().await {}
                }
                Ok(false) => {} // timeout, loop back to check for requests
                Err(e) => {
                    log(&format!("IPC worker read error: {e}"));
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
                    self.pending.clear();
                    self.next_id = 0;
                    self.connected.store(true, Ordering::Release);
                    *self.last_heartbeat_at.lock().unwrap() = Instant::now();
                    log(&format!("IPC worker reconnected after {attempt} attempts"));
                    return;
                }
                Err(e) => {
                    attempt += 1;
                    if attempt.is_multiple_of(10) {
                        log(&format!(
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
                        // Bound memory without silently discarding in-flight
                        // data: drop only the fully-received lines at the
                        // front, preserving the incomplete trailing frame.
                        if let Some(last_nl) = self.buf.iter().rposition(|&b| b == b'\n') {
                            self.buf.drain(..=last_nl);
                        } else {
                            // Single oversized / unterminated frame: nothing
                            // safe to salvage, clear and report.
                            self.buf.clear();
                            return Err(CoreError::Daemon("buffer exceeded 16MB".into()));
                        }
                        log("IPC read buffer exceeded 16MB; dropped oldest lines");
                    }
                    Ok(true)
                }
            }
            Ok(Err(e)) => Err(CoreError::Daemon(format!("read error: {e}"))),
            Err(_) => Ok(false),
        }
    }

    async fn send_by_id(&mut self, id: u64, pending: &PendingRequest) -> Result<()> {
        let req_json = serde_json::to_string(&pending.req)?;
        let cmd = pending.req.cmd_name();
        let mut line = String::with_capacity(64 + req_json.len());
        line.push_str("{\"id\":");
        line.push_str(&id.to_string());
        line.push_str(",\"cmd\":\"");
        line.push_str(cmd);
        line.push_str("\",\"params\":");
        line.push_str(&req_json);
        line.push('}');
        line.push('\n');
        self.writer.write_all(line.as_bytes()).await?;
        self.writer.flush().await?;
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
        if let Ok(wire_res) = serde_json::from_slice::<WireRes>(&self.buf[..pos]) {
            self.buf.drain(..=pos);
            // Any well-formed daemon reply proves the connection is alive; the
            // pulse socket is not the only liveness signal anymore, so a dead
            // pulse reader alone can no longer fail in-flight requests with a
            // bogus "heartbeat timeout".
            *self.last_heartbeat_at.lock().unwrap() = Instant::now();
            if let Some((cmd, tx)) = self.pending.remove(&wire_res.id) {
                let response = DaemonRes::from_wire(&cmd, &wire_res);
                let _ = tx.send(Ok(response));
            }
            return true;
        }
        self.buf.drain(..=pos);
        true
    }
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
                // Never give up: the pulse socket is the heartbeat source, and
                // exiting here would guarantee a "heartbeat timeout" minutes
                // later during the next long operation (e.g. a YT download).
                // Backoff caps at 10s so recovery stays quick.
                if attempt.is_multiple_of(10) {
                    log(&format!("pulse connect attempt {attempt} failed: {e}"));
                }
                let backoff = 200u64
                    .saturating_mul(u64::from(attempt.min(50)))
                    .min(10_000);
                tokio::time::sleep(Duration::from_millis(backoff)).await;
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
                    log("pulse: connection closed, reconnecting");
                    break;
                }
                Ok(n) => n,
                Err(e) => {
                    log(&format!("pulse read error: {e}, reconnecting"));
                    break;
                }
            };
            buf.extend_from_slice(&tmp[..n]);
            loop {
                let (decoded, consumed) = match wire::decode(&buf) {
                    Ok(Some((e, c))) => (e, c),
                    Ok(None) => break,
                    Err(e) => {
                        log(&format!("pulse decode error: {e}"));
                        buf.clear();
                        break;
                    }
                };
                buf.drain(..consumed);
                // Any traffic on the pulse socket proves the daemon is alive,
                // not just explicit heartbeats — status/event bursts during a
                // long YT download keep liveness fresh on their own.
                *last_heartbeat_at.lock().unwrap() = Instant::now();
                let mut evs = events.lock().await;
                evs.extend(decoded);
            }
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
}
