// Copyright (c) 2026
// Author: prjctimg <prjctimg@outlook.com>
// librespot-backed Spotify streaming.

use std::collections::VecDeque;
use std::path::Path;
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
}

type SharedTarget = Arc<Mutex<Option<StreamTarget>>>;

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
}

impl PcmStreamSource {
    fn new(rx: std::sync::mpsc::Receiver<Vec<f32>>, duration_secs: f64) -> Self {
        Self {
            rx,
            pending: VecDeque::with_capacity(CHANNEL_CAPACITY * 64),
            channels: NUM_CHANNELS as u16,
            sample_rate: SAMPLE_RATE,
            total_duration: Some(Duration::from_secs_f64(duration_secs)),
            loaded_at: std::time::Instant::now(),
            last_sample_at: None,
            stalled_at: None,
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
                "spotify stream silent for {}s after starting; still waiting",
                idle.as_secs()
            );
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

/// Always reports unity attenuation; volume control lives in the rodio chain.
struct VolumeOne;

impl VolumeGetter for VolumeOne {
    fn attenuation_factor(&self) -> f64 {
        1.0
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
    /// Client id the session registered with. A session is only reusable for
    /// the app that minted its token, so this is part of the reuse check.
    session_client_id: Option<String>,
}

impl Default for StreamManager {
    fn default() -> Self {
        Self::new()
    }
}

impl StreamManager {
    pub fn new() -> Self {
        Self {
            session: None,
            player: None,
            target: Arc::new(Mutex::new(None)),
            current_uri: None,
            session_token: None,
            session_client_id: None,
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
    /// with a fresh access token whenever the incoming token differs from the
    /// one the current session was established with. This keeps playback
    /// working past a token expiry instead of leaving a stale session that
    /// silently stops producing audio.
    async fn ensure_session(
        &mut self,
        token: &str,
        client_id: &str,
        config_dir: &Path,
    ) -> Result<(), String> {
        if self.player.is_some()
            && self.session_token.as_deref() == Some(token)
            && self.session_client_id.as_deref() == Some(client_id)
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

        let session_config = SessionConfig {
            client_id: client_id.to_string(),
            device_id: "gtm-rs-stream".to_string(),
            ..Default::default()
        };

        let session = Session::new(session_config, Some(cache));
        // Bound the handshake hard — see STREAM_CONNECT_TIMEOUT.
        let connected = tokio::time::timeout(
            STREAM_CONNECT_TIMEOUT,
            session.connect(Credentials::with_access_token(token), true),
        )
        .await
        .map_err(|_| {
            format!(
                "spotify connect timed out after {}s — check network / access-point reachability",
                STREAM_CONNECT_TIMEOUT.as_secs()
            )
        })?;
        connected.map_err(|e| {
            let msg = e.to_string();
            let low = msg.to_ascii_lowercase();
            if ["login", "token", "auth", "credential"]
                .iter()
                .any(|k| low.contains(k))
            {
                // A rejected access token (missing `streaming` scope, expired,
                // or issued for a different client) is the common failure;
                // point at the fix instead of the raw low-level error.
                format!(
                    "spotify connect rejected — re-link your Spotify account \
                     (re-authorize, leaving the client id empty for the default app): {msg}"
                )
            } else {
                format!("spotify connect: {msg}")
            }
        })?;
        info!("librespot session connected");

        let player = Player::new(
            PlayerConfig::default(),
            session.clone(),
            Box::new(VolumeOne),
            {
                let target = self.target.clone();
                move || Box::new(ChannelSink(target)) as Box<dyn LibrespotSink>
            },
        );

        // Event pump: end-of-track / unavailable mark the channel as
        // finished so the rodio source drains out and the mixer advances the
        // queue exactly like a local file would. Stopped events are excluded
        // so loading a new track does not clear the replacement target.
        let events = player.get_player_event_channel();
        let target = self.target.clone();
        tokio::spawn(async move {
            let mut events = events;
            while let Some(event) = events.recv().await {
                match event {
                    PlayerEvent::EndOfTrack { track_id, .. }
                    | PlayerEvent::Unavailable { track_id, .. } => {
                        let mut guard = target.lock().unwrap();
                        if let Ok(uri) = track_id.to_uri()
                            && let Some(t) = guard.as_ref()
                            && t.uri == uri
                        {
                            guard.take();
                        }
                    }
                    _ => {}
                }
            }
        });

        self.session = Some(session);
        self.player = Some(player);
        self.session_token = Some(token.to_string());
        self.session_client_id = Some(client_id.to_string());
        Ok(())
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
        self.session_client_id = None;
        self.current_uri = None;
    }

    /// Start streaming `uri` and return the rodio source to hand to the
    /// mixer. Any previous stream is torn down first.
    ///
    /// `client_id` must be the app that minted `token` (see
    /// [`SpotifyManager::streaming_client_id`]); librespot presents it when it
    /// registers the session, and a mismatch connects without streaming.
    pub async fn load(
        &mut self,
        uri: &str,
        start_ms: u32,
        duration_secs: f64,
        token: &str,
        client_id: &str,
        config_dir: &Path,
    ) -> Result<PcmStreamSource, String> {
        self.ensure_session(token, client_id, config_dir).await?;
        let parsed = SpotifyUri::from_uri(uri).map_err(|e| format!("bad spotify uri: {e}"))?;

        self.clear_target();
        let (tx, rx) = std::sync::mpsc::sync_channel::<Vec<f32>>(CHANNEL_CAPACITY);
        *self.target.lock().unwrap() = Some(StreamTarget {
            uri: uri.to_string(),
            tx,
        });

        self.current_uri = Some(uri.to_string());
        self.player
            .as_ref()
            .expect("session ensured")
            .load(parsed, true, start_ms);

        Ok(PcmStreamSource::new(rx, duration_secs))
    }

    /// Tear down the whole librespot stack (used at daemon shutdown).
    pub fn shutdown(&mut self) {
        self.reset();
        if let Some(session) = self.session.take() {
            session.shutdown();
        }
        self.player = None;
        self.session_token = None;
        self.session_client_id = None;
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

    /// The source must not report end-of-stream while its sender is still
    /// alive. rodio evicts a source from the mix the first time its iterator
    /// yields `None`, so treating a momentary gap as the end would drop a
    /// track that is merely between packets — and the previous code did exactly
    /// that after a 25s silence budget.
    #[test]
    fn empty_channel_is_not_end_of_stream() {
        let (tx, rx) = std::sync::mpsc::sync_channel::<Vec<f32>>(CHANNEL_CAPACITY);
        let mut source = PcmStreamSource::new(rx, 180.0);
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
        let mut source = PcmStreamSource::new(rx, 180.0);
        assert_eq!(source.next(), Some(0.25));
        assert_eq!(source.next(), Some(0.75));
        assert_eq!(source.next(), None);
    }

    /// The format reported to rodio must be librespot's own, not a guess: the
    /// decode thread builds its EQ and resampler from these values.
    #[test]
    fn format_matches_librespot() {
        let (_tx, rx) = std::sync::mpsc::sync_channel::<Vec<f32>>(CHANNEL_CAPACITY);
        let source = PcmStreamSource::new(rx, 180.0);
        use rodio::Source;
        assert_eq!(source.sample_rate().get(), SAMPLE_RATE);
        assert_eq!(source.channels().get(), NUM_CHANNELS as u16);
        assert_eq!(source.total_duration(), Some(Duration::from_secs(180)));
    }

    /// A stall is reported, never fatal.
    #[test]
    fn stall_is_reported_not_fatal() {
        let (_tx, rx) = std::sync::mpsc::sync_channel::<Vec<f32>>(CHANNEL_CAPACITY);
        let mut source = PcmStreamSource::new(rx, 180.0);
        source.loaded_at = std::time::Instant::now() - STARTUP_GRACE - Duration::from_secs(1);
        source.stalled_for();
        assert!(source.stalled_at.is_some(), "stall should be recorded");
        // Reported once, not re-logged on every poll.
        let first = source.stalled_at;
        source.stalled_for();
        assert_eq!(source.stalled_at, first);
    }
}
