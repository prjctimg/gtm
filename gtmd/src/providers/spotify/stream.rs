// Copyright (c) 2026
// Author: prjctimg <prjctimg@outlook.com>
// librespot-backed Spotify streaming.

use std::collections::VecDeque;
use std::path::Path;
use std::sync::atomic::{AtomicU16, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use librespot_connect::{ConnectConfig, Spirc};
use librespot_core::SessionConfig;
use librespot_core::authentication::Credentials;
use librespot_core::cache::Cache;
use librespot_core::config::DeviceType;
use librespot_core::session::Session;
use librespot_core::spotify_uri::SpotifyUri;
use librespot_playback::audio_backend::{Sink as LibrespotSink, SinkError, SinkResult};
use librespot_playback::config::PlayerConfig;
use librespot_playback::convert::Converter;
use librespot_playback::decoder::AudioPacket;
use librespot_playback::mixer::{Mixer, VolumeGetter};
use librespot_playback::player::{Player, PlayerEvent};
use librespot_playback::{NUM_CHANNELS, SAMPLE_RATE};
use tracing::{info, warn};

use gtm::shared::global::MAX_VOLUME;
use gtm::shared::ipc::DaemonEvent;

use super::LIBRESPOT_CLIENT_ID;

// A single librespot [`Session`] + [`Player`] pair is created lazily on the
// first streamed track and reused afterwards. Decoded audio is pushed by a
// custom librespot `Sink` through a bounded std channel and drained by a
// [`PcmStreamSource`].
//
// The blocking half of that pairing is deliberate and is librespot's own
// contract: `Sink::write` runs on a player thread documented as blocking, and
// every official backend blocks there, so the bounded send is what applies
// backpressure to the decoder. The drain side must then be a thread that is
// allowed to wait — the mixer does that via `load_active_stream`, which feeds
// its decode thread and gives the output callback a ring-buffer view. Handing
// this source to rodio directly would park the callback inside `recv_timeout`
// for as long as the network takes, underrun the device, and then be evicted
// from the mix for good the first time it yielded `None`.

/// Bounded channel capacity: each packet is ~23 ms of stereo audio, so this
/// buffers roughly 1.5 s — enough to ride out network jitter without
/// unbounded memory use. When the queue is full the sink blocks, which
/// naturally pauses the librespot decoder (backpressure).
const CHANNEL_CAPACITY: usize = 64;

/// How long the drain thread waits for the next packet before re-checking the
/// silence watchdog. Short enough that a stall is noticed promptly, long enough
/// that an idle stream is not a busy loop.
const POLL: Duration = Duration::from_millis(100);

/// How long a freshly loaded stream may stay silent before it is reported.
/// A cold librespot connect plus the first packets can take a while, so this is
/// generous; exceeding it means the session registered but never delivers
/// audio. Reported, not fatal — see [`PcmStreamSource::stalled_for`].
const STARTUP_GRACE: Duration = Duration::from_secs(25);

/// How long an already-playing stream may stay silent before it is reported.
/// Long enough to ride out a network hiccup without cutting the track short.
const STALL_TIMEOUT: Duration = Duration::from_secs(45);

/// Hard ceiling for the librespot session handshake. Without it, a rejected
/// access token or unreachable access points make librespot retry across up
/// to 6 APs (token auth performs a double connect per attempt), which can
/// stall the whole IPC reply past its budget and surface as a misleading
/// "IPC response timeout". Failing fast returns a readable error instead.
const STREAM_CONNECT_TIMEOUT: Duration = Duration::from_secs(20);

// ---------------------------------------------------------------------------
// Sink side: librespot audio thread -> bounded channel
// ---------------------------------------------------------------------------

#[derive(Clone)]
struct StreamTarget {
    uri: String,
    tx: std::sync::mpsc::SyncSender<Vec<f32>>,
    stat: Arc<StreamStat>,
}

type SharedTarget = Arc<Mutex<Option<StreamTarget>>>;

/// Counters shared between the librespot sink and the drain side.
///
/// A Spotify track can report itself as playing while the mixer never receives
/// a sample, and the two possible causes — librespot never delivering, and the
/// delivery being dropped between the sink and the ring — look identical from
/// the UI. These make the difference visible in one log line.
#[derive(Default)]
struct StreamStat {
    packets: AtomicU64,
    samples: AtomicU64,
    first_at: Mutex<Option<std::time::Instant>>,
    loaded_at: Mutex<Option<std::time::Instant>>,
}

impl StreamStat {
    fn note_packet(&self, samples: usize) {
        self.packets.fetch_add(1, Ordering::Relaxed);
        self.samples.fetch_add(samples as u64, Ordering::Relaxed);
        let mut first = self.first_at.lock().unwrap();
        if first.is_none() {
            *first = Some(std::time::Instant::now());
        }
    }

    /// One line summarising a load: how long the first packet took, and how much
    /// audio actually reached the drain side. `None` means nothing ever did.
    fn report(&self, uri: &str) {
        let loaded = *self.loaded_at.lock().unwrap();
        let first = *self.first_at.lock().unwrap();
        let packets = self.packets.load(Ordering::Relaxed);
        let samples = self.samples.load(Ordering::Relaxed);
        let latency = match (loaded, first) {
            (Some(l), Some(f)) => format!("{:?}", f.saturating_duration_since(l)),
            _ => "never".to_string(),
        };
        info!("spotify stream {uri}: {packets} packets / {samples} samples, first after {latency}");
    }
}

struct ChannelSink(SharedTarget);

impl LibrespotSink for ChannelSink {
    fn write(&mut self, packet: AudioPacket, _converter: &mut Converter) -> SinkResult<()> {
        let samples = packet
            .samples()
            .map_err(|e| SinkError::OnWrite(e.to_string()))?;
        let mut buf = Vec::with_capacity(samples.len());
        for sample in samples {
            buf.push((*sample as f32).clamp(-1.0, 1.0));
        }
        // Snapshot the target under a short lock, then wait outside of it so a
        // full queue never blocks target swaps or event handling.
        let target = self.0.lock().unwrap().clone();
        let Some(target) = target else {
            // librespot keeps writing until the track ends, and treats an
            // `Ok` write as healthy, so dropping here is how a target race
            // turns into silence that reports itself as playing. Say so.
            warn!(
                "spotify sink has no target — dropping {} samples",
                buf.len()
            );
            return Ok(());
        };
        target.stat.note_packet(buf.len());
        // `write` runs on librespot's player thread, which every official
        // backend blocks in, so the bounded send is the sanctioned shape — the
        // same one librespot's jackaudio backend uses. Blocking here applies
        // backpressure to the decoder, which is the point; it is only wrong on
        // the consumer side, which is why the mixer drains this on a thread
        // that is allowed to wait.
        if target.tx.send(buf).is_err() {
            // Receiver gone (track replaced or stopped). librespot reads this
            // as a healthy write and keeps going, so log it rather than let
            // it look like a normal end of stream.
            warn!("spotify sink receiver gone — dropping packet");
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Source side: bounded channel -> rodio mixer chain
// ---------------------------------------------------------------------------

/// Rodio-compatible source pulling decoded f32 samples from a streaming
/// track. Ends only after the sender disconnects *and* all buffered samples
/// are consumed, which lets the mixer emit its normal end-of-track event and
/// advance the queue.
///
/// Its `next()` blocks while waiting for the provider, so it is drained by the
/// mixer's decode thread and never by the output callback. Spectral and
/// waveform analysis therefore live in that thread too, alongside every other
/// source, instead of here.
pub struct PcmStreamSource {
    rx: std::sync::mpsc::Receiver<Vec<f32>>,
    pending: VecDeque<f32>,
    channels: u16,
    sample_rate: u32,
    total_duration: Option<Duration>,
    /// When `load` handed this source to the mixer.
    loaded_at: std::time::Instant,
    /// When a sample last arrived, once one has.
    last_sample_at: Option<std::time::Instant>,
    /// When the silence watchdog last fired, so it logs the transition once
    /// rather than on every poll.
    stalled_at: Option<std::time::Instant>,
    /// The URI this source was loaded for, and the shared target registry, so
    /// the end-of-track summary can be attributed to the right track even after
    /// the target has been replaced.
    uri: String,
    target: SharedTarget,
    stat: Arc<StreamStat>,
}

impl PcmStreamSource {
    fn new(
        rx: std::sync::mpsc::Receiver<Vec<f32>>,
        duration_secs: f64,
        uri: String,
        target: SharedTarget,
        stat: Arc<StreamStat>,
    ) -> Self {
        Self {
            rx,
            pending: VecDeque::with_capacity(CHANNEL_CAPACITY * 64),
            channels: NUM_CHANNELS as u16,
            sample_rate: SAMPLE_RATE,
            total_duration: Some(Duration::from_secs_f64(duration_secs)),
            loaded_at: std::time::Instant::now(),
            last_sample_at: None,
            stalled_at: None,
            uri,
            target,
            stat,
        }
    }

    /// Record that the stream has been silent, and log the transition into it.
    ///
    /// A stall no longer ends the source. The previous behaviour returned
    /// `None`, and rodio evicts a source from the mix the moment it yields
    /// `None` — so one slow start became permanent silence, with the UI still
    /// reporting a playing track. Waiting is correct: a transient network gap
    /// resolves on its own, and the caller keeps the track. The watchdog only
    /// makes the condition visible.
    fn stalled_for(&mut self) {
        if self.stalled_at.is_some() {
            return;
        }
        let idle = match self.last_sample_at {
            Some(at) => at.elapsed(),
            None => self.loaded_at.elapsed(),
        };
        let budget = if self.last_sample_at.is_some() {
            STALL_TIMEOUT
        } else {
            STARTUP_GRACE
        };
        if idle >= budget {
            self.stalled_at = Some(std::time::Instant::now());
            warn!(
                "spotify stream {} silent for {}s ({} packets from librespot); still waiting",
                self.uri,
                idle.as_secs(),
                self.stat.packets.load(Ordering::Relaxed)
            );
        }
    }
}

impl Drop for PcmStreamSource {
    fn drop(&mut self) {
        // Reported when the track ends or is replaced, which is the only point
        // where the totals are meaningful.
        let uri = self
            .target
            .lock()
            .unwrap()
            .as_ref()
            .filter(|t| t.uri == self.uri)
            .map(|t| t.uri.clone());
        if let Some(uri) = uri {
            self.stat.report(&uri);
        }
    }
}

impl Iterator for PcmStreamSource {
    type Item = f32;

    fn next(&mut self) -> Option<f32> {
        loop {
            if let Some(s) = self.pending.pop_front() {
                self.last_sample_at = Some(std::time::Instant::now());
                return Some(s);
            }
            // `recv_timeout` blocks, so this must never run on the output
            // callback: a network wait there underruns the device. The mixer
            // drains this source on a decode thread and hands rodio a
            // ring-buffer view instead.
            match self.rx.recv_timeout(POLL) {
                Ok(chunk) => {
                    self.last_sample_at = Some(std::time::Instant::now());
                    self.pending.extend(chunk);
                }
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => self.stalled_for(),
                // Real end of stream: the event pump dropped the sender, or the
                // track was replaced. Returning `None` here is what lets the
                // ring drain and the queue advance.
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => return None,
            }
        }
    }
}

impl rodio::Source for PcmStreamSource {
    fn current_span_len(&self) -> Option<usize> {
        None
    }
    fn channels(&self) -> std::num::NonZeroU16 {
        std::num::NonZeroU16::new(self.channels).expect("channels > 0")
    }
    fn sample_rate(&self) -> std::num::NonZeroU32 {
        std::num::NonZeroU32::new(self.sample_rate).expect("sample_rate > 0")
    }
    fn total_duration(&self) -> Option<Duration> {
        self.total_duration
    }
}

// ---------------------------------------------------------------------------
// Manager: session + player lifecycle shared across tracks
// ---------------------------------------------------------------------------

/// Everything needed to establish a librespot session, grouped so the load path
/// stays under the argument ceiling as fields are added.
///
/// No client id: the session registers as [`LIBRESPOT_CLIENT_ID`], which is
/// also the app the OAuth flow authorizes against. It used to be a separate
/// field whose documented purpose was to be *different* from the app that
/// minted `token` — that is the conflation login5 refuses, and it only ever
/// traded one failure for the other.
pub struct SessionSpec<'a> {
    pub token: &'a str,
    pub config_dir: &'a Path,
    /// The mixer's current level, announced to Connect as the device's.
    pub volume: u8,
}

/// The volume Connect announces for this device.
///
/// Connect keeps its own level and mirrors it back, so it is wired to the
/// daemon's real volume rather than a private one: a second volume state would
/// drift from the mixer's and the first Connect-issued change would silently
/// take the device to a level the UI never showed.
struct ConnectVolume(AtomicU16);

impl ConnectVolume {
    fn new(volume: u8) -> Self {
        Self(AtomicU16::new(volume.min(MAX_VOLUME) as u16))
    }
}

impl Mixer for ConnectVolume {
    fn open(_: librespot_playback::mixer::MixerConfig) -> Result<Self, librespot_core::Error> {
        Ok(Self::new(MAX_VOLUME))
    }
    fn volume(&self) -> u16 {
        self.0.load(Ordering::Relaxed)
    }
    fn set_volume(&self, volume: u16) {
        self.0
            .store(volume.min(MAX_VOLUME as u16), Ordering::Relaxed);
    }
}

/// Always reports unity attenuation; volume control lives in the rodio chain.
struct VolumeOne;

impl VolumeGetter for VolumeOne {
    fn attenuation_factor(&self) -> f64 {
        1.0
    }
}

/// How many consecutive refused track loads count as a provider problem rather
/// than a run of unavailable tracks. Three is past the point where a bad
/// playlist explains it, and low enough that the user is told within a track or
/// two of pressing play.
const LOAD_REFUSALS_REPORTED: u32 = 3;

/// Consecutive refused track loads, so a provider refusing *every* request is
/// distinguishable from a playlist of individually unavailable tracks.
///
/// Reports once per streak: a session that keeps getting refused should not
/// re-notify on every track, and a track that plays clears the streak so a
/// later failure is reported again.
struct RefusalStreak {
    count: u32,
    reported: bool,
}

impl RefusalStreak {
    fn new() -> Self {
        Self {
            count: 0,
            reported: false,
        }
    }

    /// Record one refusal; `true` when this is the one to report.
    fn refused(&mut self) -> bool {
        self.count += 1;
        if self.reported || self.count < LOAD_REFUSALS_REPORTED {
            return false;
        }
        self.reported = true;
        true
    }

    fn played(&mut self) {
        self.count = 0;
        self.reported = false;
    }
}

/// Release the channel for `track_id`, but only when it is still the loaded
/// target. A late event for a track already replaced must not strand the
/// replacement by ending its channel.
fn drop_target(target: &SharedTarget, track_id: &SpotifyUri) {
    let mut guard = target.lock().unwrap();
    if let Ok(uri) = track_id.to_uri()
        && let Some(t) = guard.as_ref()
        && t.uri == uri
    {
        guard.take();
    }
}

pub struct StreamManager {
    session: Option<Session>,
    player: Option<Arc<Player>>,
    target: SharedTarget,
    current_uri: Option<String>,
    /// Access token the current session was created with. `load` reconnects
    /// the session whenever the daemon supplies a fresh token (rspotify
    /// transparently refreshes it), so an expired access token never leaves a
    /// stale librespot session silently producing no audio.
    session_token: Option<String>,
    /// Where provider-level failures are reported. Held as a sender rather than
    /// the daemon itself so the event pump can never reach into daemon state.
    notify: tokio::sync::broadcast::Sender<DaemonEvent>,
    /// Handle to the registered Connect device. Dropped without `shutdown` it
    /// stays registered with Spotify, so teardown has to close it explicitly.
    spirc: Option<Spirc>,
}

impl Default for StreamManager {
    fn default() -> Self {
        Self::new(tokio::sync::broadcast::Sender::new(1))
    }
}

impl StreamManager {
    pub fn new(notify: tokio::sync::broadcast::Sender<DaemonEvent>) -> Self {
        Self {
            session: None,
            player: None,
            target: Arc::new(Mutex::new(None)),
            current_uri: None,
            session_token: None,
            notify,
            spirc: None,
        }
    }

    /// URI currently loaded into the stream player, if any.
    pub fn playing_uri(&self) -> Option<&str> {
        self.current_uri.as_deref()
    }

    /// A partner-API bearer token for the endpoints the official clients call,
    /// which the Web API has no equivalent of — lyrics being the one that
    /// matters here.
    ///
    /// Reuses the session the stream already established, so this costs
    /// nothing while audio is playing. `None` when no session is up: a lyrics
    /// lookup must not bring a Connect device into existence on its own, so
    /// before the first track plays there is simply no token and the caller
    /// falls back.
    pub async fn spclient_token(&self) -> Option<String> {
        self.session.as_ref()?.spclient().client_token().await.ok()
    }

    fn clear_target(&self) {
        // Dropping the sender makes the rodio source drain its buffer and
        // then end, which triggers the mixer's normal end-of-track path.
        self.target.lock().unwrap().take();
    }

    /// Halt any in-flight stream decoding and drop buffered audio.
    pub fn reset(&mut self) {
        self.clear_target();
        if let Some(player) = &self.player {
            player.stop();
        }
        self.current_uri = None;
    }

    /// Create the librespot session and player on first use, and reconnect
    /// with a fresh access token whenever the incoming token differs from the
    /// one the current session was established with. This keeps playback
    /// working past a token expiry instead of leaving a stale session that
    /// silently stops producing audio.
    async fn ensure_session(&mut self, spec: &SessionSpec<'_>) -> Result<(), String> {
        let SessionSpec {
            token,
            config_dir,
            volume,
        } = *spec;
        if self.player.is_some() && self.session_token.as_deref() == Some(token) {
            return Ok(());
        }
        self.teardown_session();

        let cache = Cache::new(
            Some(config_dir.to_path_buf()),
            None::<std::path::PathBuf>,
            None::<std::path::PathBuf>,
            None,
        )
        .map_err(|e| format!("spotify cache: {e}"))?;

        // The device id is left to librespot so it is a fresh UUID per session.
        // A fixed one is shared by every gtm install, which Connect registration
        // has to disambiguate and which makes a reconnect look like a
        // re-registration.
        let session_config = SessionConfig {
            client_id: LIBRESPOT_CLIENT_ID.to_string(),
            ..Default::default()
        };

        // Logged before connecting because the two failure modes are
        // indistinguishable afterwards: a session that connects cleanly and then
        // has every track load refused is a device that never registered with
        // Spotify Connect, not a bad token.
        info!("librespot session: client id {LIBRESPOT_CLIENT_ID}");

        let session = Session::new(session_config, Some(cache));
        // Bound the handshake hard — see STREAM_CONNECT_TIMEOUT.
        let player = Player::new(
            PlayerConfig::default(),
            session.clone(),
            Box::new(VolumeOne),
            {
                let target = self.target.clone();
                move || Box::new(ChannelSink(target)) as Box<dyn LibrespotSink>
            },
        );

        // Connect and register in one step, because `Spirc::new` connects the
        // session itself. Connecting first and then registering connected
        // twice, and the second attempt failed with "Session is not connected"
        // because it did not own the connection it was handed.
        //
        // Registration is what makes playback possible at all: Connect serves
        // the audio item and refuses a device it has never seen with
        // `FaultyRequest(BAD_REQUEST)`. The session still authenticates and the
        // player still loads the track, so the only symptom is silence —
        // librespot emits `Unavailable`, the source is released, the queue moves
        // on. The handle is kept so teardown can unregister rather than leave a
        // ghost device behind for the next session to collide with.
        let mixer = Arc::new(ConnectVolume::new(volume));
        let registered = tokio::time::timeout(
            STREAM_CONNECT_TIMEOUT,
            Spirc::new(
                ConnectConfig {
                    name: "gtm".to_string(),
                    device_type: DeviceType::Computer,
                    initial_volume: mixer.volume(),
                    is_group: false,
                    disable_volume: false,
                    volume_steps: 64,
                },
                session.clone(),
                Credentials::with_access_token(token),
                player.clone(),
                mixer,
            ),
        )
        .await
        .map_err(|_| {
            format!(
                "spotify connect timed out after {}s — check network / access-point reachability",
                STREAM_CONNECT_TIMEOUT.as_secs()
            )
        })?;
        let (spirc, spirc_task) = registered.map_err(|e| Self::connect_error(&e.to_string()))?;
        tokio::spawn(spirc_task);
        info!("librespot session connected and registered as a connect device");

        // Event pump: end-of-track / unavailable mark the channel as
        // finished so the rodio source drains out and the mixer advances the
        // queue exactly like a local file would. Stopped events are excluded
        // so loading a new track does not clear the replacement target.
        let events = player.get_player_event_channel();
        let target = self.target.clone();
        let notify = self.notify.clone();
        tokio::spawn(async move {
            let mut events = events;
            let mut streak = RefusalStreak::new();
            while let Some(event) = events.recv().await {
                match event {
                    PlayerEvent::EndOfTrack { track_id, .. } => {
                        streak.played();
                        drop_target(&target, &track_id);
                    }
                    PlayerEvent::Unavailable { track_id, .. } => {
                        drop_target(&target, &track_id);
                        if streak.refused() {
                            warn!(
                                "spotify refused {} consecutive track loads — is the \
                                 connect device registered?",
                                LOAD_REFUSALS_REPORTED
                            );
                            // The token is not the usual culprit and saying so
                            // sends people re-linking for nothing: the common
                            // cause is a device Spotify never registered, which
                            // is a Connect registration failure, not a
                            // credential one.
                            let _ = notify.send(DaemonEvent::ProviderError {
                                provider: "spotify".to_string(),
                                message: format!(
                                    "Spotify refused {LOAD_REFUSALS_REPORTED} track loads in a row. \
                                     This is usually a device that failed to register with \
                                     Spotify Connect rather than a bad login — check the \
                                     daemon log for the connect registration line."
                                ),
                            });
                        }
                    }
                    // Anything that actually starts playing clears the streak.
                    PlayerEvent::Playing { .. } => streak.played(),
                    _ => {}
                }
            }
        });

        self.spirc = Some(spirc);
        self.session = Some(session);
        self.player = Some(player);
        self.session_token = Some(token.to_string());
        Ok(())
    }

    /// Turn a librespot connect/registration failure into something actionable.
    ///
    /// A rejected access token is the common case and the raw error says only
    /// "invalid request", so it is named explicitly rather than leaving the user
    /// to guess which of the two things went wrong.
    fn connect_error(msg: &str) -> String {
        let low = msg.to_ascii_lowercase();
        if ["login", "token", "auth", "credential", "unauthor"]
            .iter()
            .any(|k| low.contains(k))
        {
            format!("spotify connect rejected — re-link your Spotify account: {msg}")
        } else {
            format!("spotify connect: {msg}")
        }
    }

    /// Drop the current librespot session and player so a fresh one can be
    /// established (e.g. with a renewed access token).
    fn teardown_session(&mut self) {
        self.clear_target();
        // Before the player, so Connect sees the device go quiet rather than
        // losing a player it is still holding.
        if let Some(spirc) = self.spirc.take()
            && let Err(e) = spirc.shutdown()
        {
            warn!("spotify connect shutdown: {e}");
        }
        if let Some(player) = &self.player {
            player.stop();
        }
        if let Some(session) = self.session.take() {
            session.shutdown();
        }
        self.player = None;
        self.session_token = None;
        self.current_uri = None;
    }

    /// Start streaming `uri` and return the rodio source to hand to the
    /// mixer. Any previous stream is torn down first.
    ///
    /// `token` must have been minted by [`LIBRESPOT_CLIENT_ID`], the same app the
    /// session registers as. Connect refuses any other pairing: a
    /// self-registered app is answered `BAD_REQUEST` for not being a recognised
    /// playback client, and someone else's id is answered
    /// `INVALID_CREDENTIALS` because login5 requires the id to match the app
    /// that issued the credential. See [`super::SpotifyManager::client_id`].
    pub async fn load(
        &mut self,
        uri: &str,
        start_ms: u32,
        duration_secs: f64,
        spec: &SessionSpec<'_>,
    ) -> Result<PcmStreamSource, String> {
        self.ensure_session(spec).await?;
        let parsed = SpotifyUri::from_uri(uri).map_err(|e| format!("bad spotify uri: {e}"))?;

        self.clear_target();
        let (tx, rx) = std::sync::mpsc::sync_channel::<Vec<f32>>(CHANNEL_CAPACITY);
        let stat = Arc::new(StreamStat::default());
        *stat.loaded_at.lock().unwrap() = Some(std::time::Instant::now());
        *self.target.lock().unwrap() = Some(StreamTarget {
            uri: uri.to_string(),
            tx,
            stat: stat.clone(),
        });

        self.current_uri = Some(uri.to_string());
        self.player
            .as_ref()
            .expect("session ensured")
            .load(parsed, true, start_ms);

        Ok(PcmStreamSource::new(
            rx,
            duration_secs,
            uri.to_string(),
            self.target.clone(),
            stat,
        ))
    }

    /// Tear down the whole librespot stack (used at daemon shutdown).
    pub fn shutdown(&mut self) {
        self.reset();
        if let Some(session) = self.session.take() {
            session.shutdown();
        }
        self.player = None;
        self.session_token = None;
    }

    /// Resume the librespot player after a pause. The mixer is the transport
    /// authority, so this only needs to tell the player to keep feeding.
    pub fn resume(&mut self) {
        if let Some(player) = self.player.as_ref() {
            player.play();
        }
    }

    /// Pause the librespot player. Pausing the mixer alone only backpressures
    /// the decoder; pausing the player also stops the network stream.
    pub fn pause(&mut self) {
        if let Some(player) = self.player.as_ref() {
            player.pause();
        }
    }

    /// Seek within the loaded track. Returns false when there is no live
    /// session, so the caller can fall back to reloading the stream.
    pub fn seek(&mut self, pos_ms: u32) -> bool {
        let Some(player) = self.player.as_ref() else {
            return false;
        };
        if player.is_invalid() {
            return false;
        }
        player.seek(pos_ms);
        true
    }

    /// True when the librespot session is gone or the player went stale, in
    /// which case the next load must rebuild it.
    pub fn is_dead(&self) -> bool {
        match self.player.as_ref() {
            None => true,
            Some(p) => p.is_invalid(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_target() -> SharedTarget {
        Arc::new(Mutex::new(None))
    }

    fn test_uri() -> String {
        "spotify:track:4cOdK2wGLETKBW3PvgPWqT".to_string()
    }

    /// The source must not report end-of-stream while its sender is still
    /// alive. rodio evicts a source from the mix the first time its iterator
    /// yields `None`, so treating a momentary gap as the end would drop a
    /// track that is merely between packets — and the previous code did exactly
    /// that after a 25s silence budget.
    #[test]
    fn empty_channel_is_not_end_of_stream() {
        let (tx, rx) = std::sync::mpsc::sync_channel::<Vec<f32>>(CHANNEL_CAPACITY);
        let mut source = PcmStreamSource::new(
            rx,
            180.0,
            test_uri(),
            test_target(),
            Arc::new(StreamStat::default()),
        );
        // Keep the sender alive: the receiver must block, not end.
        let handle = std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(120));
            tx.send(vec![0.5, -0.5]).unwrap();
            // Hold the sender open past the first sample's delivery.
            std::thread::sleep(std::time::Duration::from_millis(200));
        });
        assert_eq!(source.next(), Some(0.5));
        assert_eq!(source.next(), Some(-0.5));
        handle.join().unwrap();
    }

    /// A real end of stream — sender dropped — must still end the source, or
    /// the ring never drains and the queue stops advancing.
    #[test]
    fn dropped_sender_ends_the_source() {
        let (tx, rx) = std::sync::mpsc::sync_channel::<Vec<f32>>(CHANNEL_CAPACITY);
        tx.send(vec![0.25, 0.75]).unwrap();
        drop(tx);
        let mut source = PcmStreamSource::new(
            rx,
            180.0,
            test_uri(),
            test_target(),
            Arc::new(StreamStat::default()),
        );
        assert_eq!(source.next(), Some(0.25));
        assert_eq!(source.next(), Some(0.75));
        assert_eq!(source.next(), None);
    }

    /// The format reported to rodio must be librespot's own, not a guess: the
    /// decode thread builds its EQ and resampler from these values.
    #[test]
    fn format_matches_librespot() {
        let (_tx, rx) = std::sync::mpsc::sync_channel::<Vec<f32>>(CHANNEL_CAPACITY);
        let source = PcmStreamSource::new(
            rx,
            180.0,
            test_uri(),
            test_target(),
            Arc::new(StreamStat::default()),
        );
        use rodio::Source;
        assert_eq!(source.sample_rate().get(), SAMPLE_RATE);
        assert_eq!(source.channels().get(), NUM_CHANNELS as u16);
        assert_eq!(source.total_duration(), Some(Duration::from_secs(180)));
    }

    /// A stall is reported, never fatal.
    #[test]
    fn stall_is_reported_not_fatal() {
        let (_tx, rx) = std::sync::mpsc::sync_channel::<Vec<f32>>(CHANNEL_CAPACITY);
        let mut source = PcmStreamSource::new(
            rx,
            180.0,
            test_uri(),
            test_target(),
            Arc::new(StreamStat::default()),
        );
        source.loaded_at = std::time::Instant::now() - STARTUP_GRACE - Duration::from_secs(1);
        source.stalled_for();
        assert!(source.stalled_at.is_some(), "stall should be recorded");
        // Reported once, not re-logged on every poll.
        let first = source.stalled_at;
        source.stalled_for();
        assert_eq!(source.stalled_at, first);
    }

    fn target_for(uri: &str) -> SharedTarget {
        let target = Arc::new(Mutex::new(None));
        let (tx, _rx) = std::sync::mpsc::sync_channel::<Vec<f32>>(CHANNEL_CAPACITY);
        *target.lock().unwrap() = Some(StreamTarget {
            uri: uri.to_string(),
            tx,
            stat: Arc::new(StreamStat::default()),
        });
        target
    }

    fn loaded() -> SpotifyUri {
        SpotifyUri::from_uri(&test_uri()).expect("valid uri")
    }

    /// The normal path: a track that loads and plays ends by releasing the
    /// channel, which is what lets the ring drain and the queue advance.
    #[test]
    fn end_of_track_releases_the_channel() {
        let target = target_for(&test_uri());
        drop_target(&target, &loaded());
        assert!(target.lock().unwrap().is_none());
    }

    /// A refusal for the loaded track must also release it, or the source waits
    /// out its stall budget on a channel nobody will ever feed.
    #[test]
    fn refusal_releases_the_channel() {
        let target = target_for(&test_uri());
        drop_target(&target, &loaded());
        assert!(target.lock().unwrap().is_none());
    }

    /// A late event for a track that has already been replaced must not strand
    /// the replacement by ending its channel.
    #[test]
    fn a_stale_event_leaves_the_replacement_alone() {
        let target = target_for("spotify:track:55Lz7vmtisJ6BBvuIR8t7U");
        drop_target(&target, &loaded());
        let held = target.lock().unwrap();
        assert_eq!(
            held.as_ref().map(|t| t.uri.as_str()),
            Some("spotify:track:55Lz7vmtisJ6BBvuIR8t7U"),
            "the replacement target must survive an event for the old track"
        );
    }

    /// The threshold that turns per-track refusals into a report about the
    /// provider. Below it, an unavailable playlist must stay quiet.
    #[test]
    fn a_single_refusal_is_not_a_provider_failure() {
        let mut streak = RefusalStreak::new();
        for _ in 0..(LOAD_REFUSALS_REPORTED - 1) {
            assert!(
                !streak.refused(),
                "an isolated unavailable track is not a failure"
            );
        }
        assert!(
            streak.refused(),
            "the streak should report once it reaches the threshold"
        );
    }

    /// One report per streak: a session that keeps being refused must not
    /// re-notify on every track.
    #[test]
    fn a_streak_reports_only_once() {
        let mut streak = RefusalStreak::new();
        assert!(!streak.refused());
        assert!(!streak.refused());
        assert!(streak.refused());
        for _ in 0..5 {
            assert!(!streak.refused(), "already reported for this streak");
        }
    }

    /// A track that plays clears the streak, so a later failure is diagnosed
    /// again rather than staying latched off for the rest of the session.
    #[test]
    fn playing_clears_the_streak() {
        let mut streak = RefusalStreak::new();
        for _ in 0..LOAD_REFUSALS_REPORTED {
            streak.refused();
        }
        streak.played();
        assert!(!streak.refused(), "the streak restarts after a track plays");
    }
}
