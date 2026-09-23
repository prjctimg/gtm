// Copyright (c) 2026
// Author: prjctimg <prjctimg@outlook.com>
// librespot-backed Spotify streaming.

use std::collections::VecDeque;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use gtm::audio::{
    SPECTRUM_BINS, SpectrumAnalyzer, WAVEFORM_DECIM, WAVEFORM_FRESHNESS, WaveformShared,
};
use gtm::shared::spotify::LIBRESPOT_CLIENT_ID;
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
use tracing::info;

// A single librespot [`Session`] + [`Player`] pair is created lazily on the
// first streamed track and reused afterwards. Decoded audio is pushed by a
// custom librespot `Sink` through a bounded std channel; on the rodio side a
// [`PcmStreamSource`] pulls from that channel, feeds the spectrum analyzer
// used for visualizer levels, and implements `rodio::Source` so it can be
// handed to the existing audio mixer chain (`load_active_decoded`) —
// EQ, reverb, volume, and output routing all behave exactly like local
// files.

/// Bounded channel capacity: each packet is ~23 ms of stereo audio, so this
/// buffers roughly 1.5 s — enough to ride out network jitter without
/// unbounded memory use. When rodio's queue is full the sink blocks, which
/// naturally pauses the librespot decoder (backpressure).
const CHANNEL_CAPACITY: usize = 64;

/// Spectrum levels older than this are treated as expired so the visualizer
/// falls back to silence instead of freezing on the last frame.
const SPECTRUM_FRESHNESS: Duration = Duration::from_millis(300);

/// Hard ceiling for the librespot session handshake. Without it, a rejected
/// access token or unreachable access points make librespot retry across up
/// to 6 APs (token auth performs a double connect per attempt), which can
/// stall the whole IPC reply past its budget and surface as a misleading
/// "IPC response timeout". Failing fast returns a readable error instead.
const STREAM_CONNECT_TIMEOUT: Duration = Duration::from_secs(20);

type SpectrumShared = Arc<Mutex<(std::time::Instant, Vec<f32>)>>;

fn new_spectrum_shared() -> SpectrumShared {
    Arc::new(Mutex::new((
        std::time::Instant::now() - SPECTRUM_FRESHNESS - SPECTRUM_FRESHNESS,
        Vec::new(),
    )))
}

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
        // Snapshot the target under a short lock, then block outside of it so
        // a stalled consumer never wedges target swaps or event handling.
        let target = self.0.lock().unwrap().clone();
        if let Some(target) = target
            && target.tx.send(buf).is_err()
        {
            // Receiver gone (track switched/stop); not fatal for the
            // decoder thread — the player is being replaced anyway.
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
pub struct PcmStreamSource {
    rx: std::sync::mpsc::Receiver<Vec<f32>>,
    pending: VecDeque<f32>,
    analyzer: SpectrumAnalyzer,
    levels: [f32; SPECTRUM_BINS],
    spectrum_out: SpectrumShared,
    wave_out: WaveformShared,
    wave_left: Option<f32>,
    wave_frame: usize,
    channels: u16,
    sample_rate: u32,
    total_duration: Option<Duration>,
}

impl PcmStreamSource {
    fn new(
        rx: std::sync::mpsc::Receiver<Vec<f32>>,
        spectrum_out: SpectrumShared,
        wave_out: WaveformShared,
        duration_secs: f64,
    ) -> Self {
        wave_out.set_stereo(true);
        Self {
            rx,
            pending: VecDeque::with_capacity(4096),
            analyzer: SpectrumAnalyzer::new(44_100.0),
            levels: [0.0; SPECTRUM_BINS],
            spectrum_out,
            wave_out,
            wave_left: None,
            wave_frame: 0,
            channels: 2,
            sample_rate: 44_100,
            total_duration: Some(Duration::from_secs_f64(duration_secs)),
        }
    }

    /// Feed refilled samples through the mono spectrum analyzer (left
    /// channel) and publish fresh band levels when an FFT window completes.
    /// Every `WAVEFORM_DECIM`th stereo frame is also tapped into the shared
    /// waveform ring for the Wave/Stereo visualizer modes.
    fn refill(&mut self, chunk: Vec<f32>) {
        for (i, s) in chunk.into_iter().enumerate() {
            let even = i % 2 == 0;
            if even {
                self.wave_left = Some(s);
                if self.analyzer.push(s, &mut self.levels) {
                    let mut out = self.spectrum_out.lock().unwrap();
                    out.0 = std::time::Instant::now();
                    out.1 = self.levels.to_vec();
                }
            } else if let Some(l) = self.wave_left.take() {
                self.wave_frame += 1;
                if self.wave_frame.is_multiple_of(WAVEFORM_DECIM) {
                    self.wave_out.push_frame(l, s);
                }
            }
            self.pending.push_back(s);
        }
    }

    pub fn take_spectrum(&self) -> Vec<f32> {
        self.spectrum_out.lock().unwrap().1.clone()
    }
}

impl Iterator for PcmStreamSource {
    type Item = f32;

    fn next(&mut self) -> Option<f32> {
        loop {
            if let Some(s) = self.pending.pop_front() {
                return Some(s);
            }
            match self.rx.recv_timeout(Duration::from_millis(100)) {
                Ok(chunk) => self.refill(chunk),
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => continue,
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
    spectrum: SpectrumShared,
    waveform: WaveformShared,
    current_uri: Option<String>,
    /// Access token the current session was created with. `load` reconnects
    /// the session whenever the daemon supplies a fresh token (rspotify
    /// transparently refreshes it), so an expired access token never leaves a
    /// stale librespot session silently producing no audio.
    session_token: Option<String>,
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
            spectrum: new_spectrum_shared(),
            waveform: WaveformShared::default(),
            current_uri: None,
            session_token: None,
        }
    }

    /// Latest waveform ring produced by the active stream, alongside its
    /// stereo flag. Empty when nothing streamed recently so the UI decays to
    /// rest.
    pub fn waveform_snapshot(&self) -> (Vec<f32>, bool) {
        self.waveform.snapshot(WAVEFORM_FRESHNESS)
    }

    /// Latest visualizer band levels produced by the active stream. Empty
    /// when nothing streamed recently so the UI falls back to silence.
    pub fn spectrum_snapshot(&self) -> Vec<f32> {
        let guard = self.spectrum.lock().unwrap();
        if guard.0.elapsed() <= SPECTRUM_FRESHNESS {
            guard.1.clone()
        } else {
            Vec::new()
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
        {
            let mut s = self.spectrum.lock().unwrap();
            s.1.clear();
        }
    }

    /// Create the librespot session and player on first use, and reconnect
    /// with a fresh access token whenever the incoming token differs from the
    /// one the current session was established with. This keeps playback
    /// working past a token expiry instead of leaving a stale session that
    /// silently stops producing audio.
    async fn ensure_session(&mut self, token: &str, config_dir: &Path) -> Result<(), String> {
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

        let session_config = SessionConfig {
            client_id: LIBRESPOT_CLIENT_ID.to_string(),
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
        self.current_uri = None;
        {
            let mut s = self.spectrum.lock().unwrap();
            s.1.clear();
        }
        self.waveform.clear();
    }

    /// Start streaming `uri` and return the rodio source to hand to the
    /// mixer. Any previous stream is torn down first.
    pub async fn load(
        &mut self,
        uri: &str,
        start_ms: u32,
        duration_secs: f64,
        token: &str,
        config_dir: &Path,
    ) -> Result<PcmStreamSource, String> {
        self.ensure_session(token, config_dir).await?;
        let parsed = SpotifyUri::from_uri(uri).map_err(|e| format!("bad spotify uri: {e}"))?;

        self.clear_target();
        let (tx, rx) = std::sync::mpsc::sync_channel::<Vec<f32>>(CHANNEL_CAPACITY);
        *self.target.lock().unwrap() = Some(StreamTarget {
            uri: uri.to_string(),
            tx,
        });

        self.current_uri = Some(uri.to_string());
        {
            let mut s = self.spectrum.lock().unwrap();
            s.1.clear();
        }
        self.waveform.clear();
        self.player
            .as_ref()
            .expect("session ensured")
            .load(parsed, true, start_ms);

        Ok(PcmStreamSource::new(
            rx,
            self.spectrum.clone(),
            self.waveform.clone(),
            duration_secs,
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
}
