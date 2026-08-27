// Copyright (c) 2026
// Author: prjctimg <prjctimg@outlook.com>
// librespot-backed Spotify streaming.

use std::collections::VecDeque;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use gtm_audio::{SPECTRUM_BINS, SpectrumAnalyzer};
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
// handed to the existing gtm-audio mixer chain (`load_active_decoded`) —
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
    channels: u16,
    sample_rate: u32,
    total_duration: Option<Duration>,
}

impl PcmStreamSource {
    fn new(
        rx: std::sync::mpsc::Receiver<Vec<f32>>,
        spectrum_out: SpectrumShared,
        duration_secs: f64,
    ) -> Self {
        Self {
            rx,
            pending: VecDeque::new(),
            analyzer: SpectrumAnalyzer::new(44_100.0),
            levels: [0.0; SPECTRUM_BINS],
            spectrum_out,
            channels: 2,
            sample_rate: 44_100,
            total_duration: Some(Duration::from_secs_f64(duration_secs)),
        }
    }

    /// Feed refilled samples through the mono spectrum analyzer (left
    /// channel) and publish fresh band levels when an FFT window completes.
    fn refill(&mut self, chunk: Vec<f32>) {
        for (i, s) in chunk.into_iter().enumerate() {
            if i % 2 == 0 && self.analyzer.push(s, &mut self.levels) {
                let mut out = self.spectrum_out.lock().unwrap();
                out.0 = std::time::Instant::now();
                out.1 = self.levels.to_vec();
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
    current_uri: Option<String>,
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
            current_uri: None,
        }
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

    /// Create the librespot session and player once per daemon lifetime.
    async fn ensure_session(&mut self, token: &str, config_dir: &Path) -> Result<(), String> {
        if self.player.is_some() {
            return Ok(());
        }
        let cache = Cache::new(
            Some(config_dir.to_path_buf()),
            None::<std::path::PathBuf>,
            None::<std::path::PathBuf>,
            None,
        )
        .map_err(|e| format!("spotify cache: {e}"))?;

        let session_config = SessionConfig {
            client_id: gtm_core::spotify::LIBRESPOT_CLIENT_ID.to_string(),
            device_id: "gtm-rs-stream".to_string(),
            ..Default::default()
        };

        let session = Session::new(session_config, Some(cache));
        session
            .connect(Credentials::with_access_token(token), true)
            .await
            .map_err(|e| format!("spotify connect: {e}"))?;
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

        // Event pump: end-of-track / stop / unavailable mark the channel as
        // finished so the rodio source drains out and the mixer advances the
        // queue exactly like a local file would.
        let events = player.get_player_event_channel();
        let target = self.target.clone();
        tokio::spawn(async move {
            let mut events = events;
            while let Some(event) = events.recv().await {
                match event {
                    PlayerEvent::EndOfTrack { .. }
                    | PlayerEvent::Stopped { .. }
                    | PlayerEvent::Unavailable { .. } => {
                        target.lock().unwrap().take();
                    }
                    _ => {}
                }
            }
        });

        self.session = Some(session);
        self.player = Some(player);
        Ok(())
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
        *self.target.lock().unwrap() = Some(StreamTarget { tx });

        self.current_uri = Some(uri.to_string());
        {
            let mut s = self.spectrum.lock().unwrap();
            s.1.clear();
        }
        self.player
            .as_ref()
            .expect("session ensured")
            .load(parsed, true, start_ms);

        Ok(PcmStreamSource::new(
            rx,
            self.spectrum.clone(),
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
    }
}
