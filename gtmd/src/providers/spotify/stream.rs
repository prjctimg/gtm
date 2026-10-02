// Copyright (c) 2026
// Author: prjctimg <prjctimg@outlook.com>
// librespot-backed Spotify streaming.

use std::collections::VecDeque;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use librespot_core::SessionConfig;
use librespot_core::authentication::Credentials;
use librespot_core::cache::Cache;
use librespot_core::session::Session;
use librespot_core::spotify_uri::SpotifyUri;
use librespot_playback::audio_backend::{Sink as LibrespotSink, SinkError, SinkResult};
use librespot_playback::config::PlayerConfig;
use librespot_playback::convert::Converter;
use librespot_playback::decoder::AudioPacket;
use librespot_playback::mixer::VolumeGetter;
use librespot_playback::player::{Player, PlayerEvent};
use librespot_playback::{NUM_CHANNELS, SAMPLE_RATE};
use tracing::{info, warn};

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
//
// The session connects to Spotify's access point and authenticates, and that
// is all. It deliberately does not register as a Spotify Connect device: that
// needed `Spirc`, whose constructor performs a `login5` transfer-token request
// this credential cannot satisfy (see [`StreamManager::ensure_session`]). The
// cost of dropping it is that gtm is not visible as a Connect target, so a
// phone cannot see or control it. Nothing on the audio path needs it — the
// player resolves each track through the Web API and pulls the audio from
// Spotify's CDN with the session's own `spclient`.

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
/// generous; exceeding it means the session connected but never delivers
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
    /// When a packet last reached the sink. The stall watchdog in the daemon
    /// polls this: the sink and the drain side both track silence locally, but
    /// only something outside this module can decide to tear the session down.
    last_packet_at: Mutex<Option<std::time::Instant>>,
}

impl StreamStat {
    fn note_packet(&self, samples: usize) {
        self.packets.fetch_add(1, Ordering::Relaxed);
        self.samples.fetch_add(samples as u64, Ordering::Relaxed);
        let now = std::time::Instant::now();
        *self.last_packet_at.lock().unwrap() = Some(now);
        let mut first = self.first_at.lock().unwrap();
        if first.is_none() {
            *first = Some(now);
        }
    }

    /// How long the sink has gone without delivering a packet, measured from
    /// the load when nothing ever arrived.
    fn silence(&self) -> Duration {
        match (
            *self.last_packet_at.lock().unwrap(),
            *self.loaded_at.lock().unwrap(),
        ) {
            (Some(at), _) => at.elapsed(),
            (None, Some(loaded)) => loaded.elapsed(),
            (None, None) => Duration::ZERO,
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
    /// The URI this source was loaded for, and the counters that belong to it,
    /// so the end-of-track summary is attributed to the right track even after
    /// the shared target has been replaced.
    uri: String,
    stat: Arc<StreamStat>,
}

impl PcmStreamSource {
    fn new(
        rx: std::sync::mpsc::Receiver<Vec<f32>>,
        duration_secs: f64,
        uri: String,
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
        // where the totals are meaningful. Attributed from this source's own
        // URI and counters: consulting the shared target here reported nothing
        // at all, because `EndOfTrack` clears that registry before the source
        // finishes draining.
        self.stat.report(&self.uri);
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
    /// Counters for the currently loaded track. Held beside the target rather
    /// than inside it so [`StreamManager::silence`] can still read them once the
    /// registry has been cleared for a track that ended.
    stat: Arc<Mutex<Option<Arc<StreamStat>>>>,
    /// Where provider-level failures are reported. Held as a sender rather than
    /// the daemon itself so the event pump can never reach into daemon state.
    notify: tokio::sync::broadcast::Sender<DaemonEvent>,
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
            stat: Arc::new(Mutex::new(None)),
            notify,
        }
    }

    /// URI currently loaded into the stream player, if any.
    pub fn playing_uri(&self) -> Option<&str> {
        self.current_uri.as_deref()
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
    /// whenever the incoming token differs from the one the current session was
    /// established with, or the session has gone stale. This keeps playback
    /// working past a token expiry instead of leaving a stale session that
    /// silently stops producing audio.
    ///
    /// The liveness test is what makes a reload able to recover. Spotify drops
    /// the access-point connection on its own schedule, and a player whose
    /// session is gone keeps reporting the loaded track while delivering
    /// nothing. Reusing it — which a token-only check did, since the token is
    /// still valid — left `PcmStreamSource` waiting on a channel nobody would
    /// ever feed, which is the permanent silence this guards against.
    ///
    /// The session connects to Spotify's access point and authenticates; it
    /// does **not** register as a Connect device. Registration is what required
    /// `Spirc`, and `Spirc::new` unconditionally calls `login5.auth_token()`
    /// (`librespot-connect-0.8.0/src/spirc.rs:220`) to pre-acquire a transfer
    /// token. That call posts an OAuth access token as
    /// `Login_method::StoredCredential` under the keymaster client id, the
    /// pairing `librespot-core` itself documents as unsupported — on
    /// `auth_token`: "This request will only work when the store credentials
    /// match the client-id". Spotify answers it `503 Service Unavailable`,
    /// which is the failure being fixed here. It is also the *only* call that
    /// failed: `client_token()` and the access-point handshake both succeed on
    /// the same token, which is why the log shows `Authenticated as '...' !`
    /// immediately before the 503.
    ///
    /// Registration is not on the audio path. `Player::load` goes
    /// `AudioItem::get_file` (the Web API) → `CdnUrl::resolve_audio` →
    /// `spclient().stream_from_cdn()`, and every one of those is session state
    /// that was already established. So the bytes still arrive, and what is
    /// given up is only visibility: gtm no longer appears as a Connect target,
    /// so a phone cannot see or control it.
    async fn ensure_session(&mut self, spec: &SessionSpec<'_>) -> Result<(), String> {
        let SessionSpec { token, config_dir } = *spec;
        if self.player.is_some() && self.session_token.as_deref() == Some(token) && !self.is_dead()
        {
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
        // A fixed one is shared by every gtm install, and the device id is part
        // of the `client_token` request, so a shared one would have every
        // install contending for one token.
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

        // Connect to the access point and authenticate. This is the whole of
        // what streaming needs: the session owns the AP connection, the
        // `client_token` and the authenticated identity that
        // `spclient().stream_from_cdn()` presents when it pulls the audio.
        //
        // There is no `Spirc` registration step any more. The old comment here
        // claimed "Connect serves the audio item and refuses a device it has
        // never seen with `FaultyRequest(BAD_REQUEST)`", and on that reasoning
        // registration was treated as a precondition for playback. Reading
        // librespot's own load path does not support it: `load_remote_track`
        // resolves the item through the Web API and the CDN through
        // `spclient`, and `Spirc` appears in neither. Registration bought
        // visibility (a phone could see and control gtm) and, in exchange, a
        // `login5` call this credential cannot satisfy.
        tokio::time::timeout(
            STREAM_CONNECT_TIMEOUT,
            session.connect(Credentials::with_access_token(token), true),
        )
        .await
        .map_err(|_| {
            format!(
                "spotify connect timed out after {}s — check network / access-point reachability",
                STREAM_CONNECT_TIMEOUT.as_secs()
            )
        })?
        .map_err(|e| Self::connect_error(&e.to_string()))?;
        info!("librespot session connected and authenticated");

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
                                "spotify refused {} consecutive track loads",
                                LOAD_REFUSALS_REPORTED
                            );
                            // This message used to blame Connect registration,
                            // which is now gone as a cause — the session never
                            // registers, so a device that failed to register
                            // cannot be what happened. What is left on this path
                            // is the account: `Unavailable` is what the CDN and
                            // Web API answer for a track the account may not
                            // play (no Premium, market restriction, region).
                            let _ = notify.send(DaemonEvent::ProviderError {
                                provider: "spotify".to_string(),
                                message: format!(
                                    "Spotify refused {LOAD_REFUSALS_REPORTED} track loads in a row. \
                                     The session connected, so this is the account rather than the \
                                     login — check that the account is Premium and available in \
                                     its market."
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

        self.session = Some(session);
        self.player = Some(player);
        self.session_token = Some(token.to_string());
        Ok(())
    }

    /// Turn a librespot connect failure into something actionable.
    ///
    /// A rejected access token is the common case and the raw error says only
    /// "invalid request", so it is named explicitly rather than leaving the user
    /// to guess which of the two things went wrong. Now that only
    /// `Session::connect` reports here, the two causes are a token Spotify will
    /// not accept and an access point we cannot reach, and a bare 503 means the
    /// latter — it was the *registration* call that answered 503 before, and
    /// that no longer happens here.
    fn connect_error(msg: &str) -> String {
        let low = msg.to_ascii_lowercase();
        if ["login", "token", "auth", "credential", "unauthor"]
            .iter()
            .any(|k| low.contains(k))
        {
            format!("spotify connect rejected — re-link your Spotify account: {msg}")
        } else if low.contains("503") || low.contains("service unavailable") {
            format!(
                "spotify connect: {msg} — Spotify's access point is refusing this client; \
                 try again, or set a Web API client id if you have not already"
            )
        } else {
            format!("spotify connect: {msg}")
        }
    }

    /// Drop the current librespot session and player so a fresh one can be
    /// established (e.g. with a renewed access token).
    fn teardown_session(&mut self) {
        self.clear_target();
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
    /// `token` must be minted by [`LIBRESPOT_CLIENT_ID`], the app the session
    /// authenticates as. That constraint is unchanged by dropping registration
    /// — it was never about Connect accepting the id, it is that
    /// `Session::connect` presents the token to the app that issued it, and a
    /// token minted by a different app is rejected there. See
    /// [`super::SpotifyManager::client_id`].
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
        *self.stat.lock().unwrap() = Some(stat.clone());
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

    /// How long the current stream has gone without a packet, or `None` when
    /// nothing is loaded.
    ///
    /// Read by the daemon's stall watchdog. A track whose position keeps
    /// advancing while this grows is the permanent-silence case: the mixer
    /// extrapolates off the wall clock, so a stalled stream looks like healthy
    /// playback everywhere except here and in the source's own log line.
    pub fn silence(&self) -> Option<Duration> {
        let stat = self.stat.lock().unwrap().clone();
        stat.map(|s| s.silence())
    }

    /// Packets the sink has accepted for the current track, for the watchdog's
    /// report line.
    pub fn packets(&self) -> Option<u64> {
        let stat = self.stat.lock().unwrap().clone();
        stat.map(|s| s.packets.load(Ordering::Relaxed))
    }

    /// Drop the session so the next load rebuilds it from a fresh connect.
    ///
    /// The recovery half of the stall watchdog. Reusing a session whose
    /// access-point connection Spotify has dropped is what turns a hiccup into
    /// permanent silence, so the caller has to be able to force the teardown
    /// that [`StreamManager::load`] only performs on a token or liveness change.
    pub fn rebuild(&mut self) {
        self.teardown_session();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_uri() -> String {
        "spotify:track:4cOdK2wGLETKBW3PvgPWqT".to_string()
    }

    /// A connect failure has to say which of the two things went wrong, because
    /// the raw librespot error does not: a token Spotify rejects and an access
    /// point that is not answering need opposite responses.
    #[test]
    fn connect_failures_are_classified() {
        // What Spotify actually says when it turns a token down.
        let rejected = StreamManager::connect_error("invalid token");
        assert!(rejected.contains("re-link"), "{rejected}");

        // "invalid request" names no credential, and a message that does not is
        // not evidence of one. Sending someone to re-link over a malformed
        // request is worse than saying what happened, so it falls through.
        let ambiguous = StreamManager::connect_error("invalid request");
        assert!(!ambiguous.contains("re-link"), "{ambiguous}");

        let unreachable =
            StreamManager::connect_error("Service unavailable { Response status code: 503 }");
        assert!(unreachable.contains("access point"), "{unreachable}");
        // A 503 is not a credential problem, and telling a user to re-link
        // sends them to do something that cannot help.
        assert!(!unreachable.contains("re-link"), "{unreachable}");

        let other = StreamManager::connect_error("some other failure");
        assert!(other.starts_with("spotify connect:"), "{other}");
    }

    /// The source must not report end-of-stream while its sender is still
    /// alive. rodio evicts a source from the mix the first time its iterator
    /// yields `None`, so treating a momentary gap as the end would drop a
    /// track that is merely between packets — and the previous code did exactly
    /// that after a 25s silence budget.
    #[test]
    fn empty_channel_is_not_end_of_stream() {
        let (tx, rx) = std::sync::mpsc::sync_channel::<Vec<f32>>(CHANNEL_CAPACITY);
        let mut source =
            PcmStreamSource::new(rx, 180.0, test_uri(), Arc::new(StreamStat::default()));
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
        let mut source =
            PcmStreamSource::new(rx, 180.0, test_uri(), Arc::new(StreamStat::default()));
        assert_eq!(source.next(), Some(0.25));
        assert_eq!(source.next(), Some(0.75));
        assert_eq!(source.next(), None);
    }

    /// The format reported to rodio must be librespot's own, not a guess: the
    /// decode thread builds its EQ and resampler from these values.
    #[test]
    fn format_matches_librespot() {
        let (_tx, rx) = std::sync::mpsc::sync_channel::<Vec<f32>>(CHANNEL_CAPACITY);
        let source = PcmStreamSource::new(rx, 180.0, test_uri(), Arc::new(StreamStat::default()));
        use rodio::Source;
        assert_eq!(source.sample_rate().get(), SAMPLE_RATE);
        assert_eq!(source.channels().get(), NUM_CHANNELS as u16);
        assert_eq!(source.total_duration(), Some(Duration::from_secs(180)));
    }

    /// A stall is reported, never fatal.
    #[test]
    fn stall_is_reported_not_fatal() {
        let (_tx, rx) = std::sync::mpsc::sync_channel::<Vec<f32>>(CHANNEL_CAPACITY);
        let mut source =
            PcmStreamSource::new(rx, 180.0, test_uri(), Arc::new(StreamStat::default()));
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

    /// Silence is measured from the load when nothing ever arrived, and from the
    /// last packet otherwise. The distinction matters: a source that has never
    /// been fed and one that stopped mid-track are different faults, and both
    /// are reported against the wrong baseline if the watchdog counts from the
    /// load in the second case.
    #[test]
    fn silence_counts_from_the_last_packet() {
        let stat = StreamStat::default();
        assert_eq!(stat.silence(), Duration::ZERO, "no load and no packets");

        *stat.loaded_at.lock().unwrap() = Some(std::time::Instant::now() - Duration::from_secs(60));
        assert!(
            stat.silence() >= Duration::from_secs(60),
            "a source that never delivered is timed from its load"
        );

        stat.note_packet(512);
        assert!(
            stat.silence() < Duration::from_secs(1),
            "a fresh packet resets the clock"
        );

        // The point of the whole watchdog: a stalled stream still reports a
        // growing silence rather than freezing at zero.
        std::thread::sleep(Duration::from_millis(20));
        assert!(stat.silence() >= Duration::from_millis(20));
    }

    /// The per-track summary is the only thing that separates "librespot never
    /// delivered" from "delivered but dropped before the ring", so it has to
    /// survive the target registry being cleared for an ended track. It used to
    /// consult that registry, which `EndOfTrack` empties before the source
    /// drains, so it reported nothing at all.
    #[test]
    fn the_summary_survives_the_target_being_cleared() {
        let target = target_for(&test_uri());
        let stat = Arc::new(StreamStat::default());
        *stat.loaded_at.lock().unwrap() = Some(std::time::Instant::now());
        stat.note_packet(64);
        let (_tx, rx) = std::sync::mpsc::sync_channel::<Vec<f32>>(CHANNEL_CAPACITY);
        let source = PcmStreamSource::new(rx, 180.0, test_uri(), stat);
        // The registry empties exactly as it does on `EndOfTrack`.
        drop_target(&target, &loaded());
        assert!(
            target.lock().unwrap().is_none(),
            "the target really is gone"
        );
        // Dropping reports through the source's own counters, so the ordering
        // between this and `EndOfTrack` no longer decides whether the track
        // ever gets a summary line.
        drop(source);
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
