// Copyright (c) 2026
// Author: prjctimg <prjctimg@outlook.com>
//
// This is free software released under the GPL-3.0 license.

use std::fs::File;
use std::io::BufReader;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};
use std::time::{Duration, Instant};

use pulseaudio::protocol;
use pulseaudio::{Client, PlaybackSource};

use crate::audio::backend::{AudioError, AudioEvent, AudioResult};
use crate::audio::buffer::{
    BUFFER_CAPACITY_SAMPLES, DecodeControl, PREBUFFER_SAMPLES, PREBUFFER_SAMPLES_REDUCED,
    RingBufferInner, SharedRingBuffer,
};
use crate::audio::decoder::DecodeThread;
use crate::audio::eq::{EqGains, EqSource, PreGain, PreGainSource, ReverbSource};
use crate::audio::mixer::{Mixer, STREAM_PREBUFFER_TIMEOUT};
use crate::audio::symphonia::{StreamingReopen, SymphoniaSource};
use crate::audio::wave::WaveformShared;
use crate::shared::global::{EqPreset, ReverbConfig};
use crate::shared::{MAX_VOLUME, volume_from_ratio, volume_ratio};
use rodio::Source;

struct PaPlaybackSource {
    ring: SharedRingBuffer,
    volume: Arc<AtomicU8>,
    /// Consecutive polls that returned empty while the decode thread was still
    /// feeding the ring, collapsed into one report per burst rather than
    /// one per poll.
    underrun_burst: u32,
}

impl PlaybackSource for PaPlaybackSource {
    fn poll_read(self: Pin<&mut Self>, _cx: &mut Context<'_>, buf: &mut [u8]) -> Poll<usize> {
        let self_ = self.get_mut();
        let vol = volume_ratio(self_.volume.load(Ordering::Relaxed));
        let float_count = buf.len() / 4;
        let mut written = 0usize;

        for _i in 0..float_count {
            match self_.ring.pop() {
                Some(s) => {
                    // Producer caught up; a previously empty burst is over.
                    if self_.underrun_burst > 0 {
                        log::debug!(
                            "pa underrun cleared after {} empty polls",
                            self_.underrun_burst
                        );
                        self_.underrun_burst = 0;
                    }
                    let scaled = s * vol;
                    let offset = written * 4;
                    buf[offset..offset + 4].copy_from_slice(&scaled.to_le_bytes());
                    written += 1;
                }
                None => {
                    if self_.ring.is_finished() {
                        break;
                    }
                    if written > 0 {
                        return Poll::Ready(written * 4);
                    }
                    // Ring empty but decoding: report once per burst so stutter
                    // is visible in diagnostics without spamming the log.
                    self_.underrun_burst += 1;
                    if self_.underrun_burst == 1 || self_.underrun_burst.is_multiple_of(500) {
                        log::warn!(
                            "pa underrun: ring empty while decoding, {} consecutive polls",
                            self_.underrun_burst
                        );
                    }
                    return Poll::Pending;
                }
            }
        }

        if written == 0 && self_.ring.is_finished() {
            Poll::Ready(0)
        } else {
            Poll::Ready(written * 4)
        }
    }
}

struct PaStreamState {
    stream: pulseaudio::PlaybackStream,
    ring: SharedRingBuffer,
    stream_volume: Arc<AtomicU8>,
    control: Option<Arc<DecodeControl>>,
    decode_handle: Option<std::thread::JoinHandle<()>>,
    name: String,
    sample_rate: u32,
    channels: u16,
}

impl PaStreamState {
    fn stream_params(sample_rate: u32, channels: u16) -> protocol::PlaybackStreamParams {
        let sample_rate = sample_rate.clamp(22050, 192000);
        let channels = if channels == 1 { 1 } else { 2 };
        protocol::PlaybackStreamParams {
            sample_spec: protocol::SampleSpec {
                format: protocol::SampleFormat::Float32Le,
                channels: channels as u8,
                sample_rate,
            },
            channel_map: if channels == 1 {
                protocol::ChannelMap::mono()
            } else {
                protocol::ChannelMap::stereo()
            },
            cvolume: Some(protocol::ChannelVolume::muted(channels as u8)),
            buffer_attr: protocol::stream::BufferAttr {
                max_length: u32::MAX,
                // ~2s buffer and ~1.5s pre-buffer, scaled to the track rate so
                // the timing stays constant regardless of the source rate.
                target_length: sample_rate * (channels as u32) * 4 * 2,
                pre_buffering: sample_rate * (channels as u32) * 4 * 3 / 2,
                minimum_request_length: 1024,
                fragment_size: u32::MAX,
            },
            ..Default::default()
        }
    }

    fn create_stream(
        client: &Client,
        name: &str,
        ring: &SharedRingBuffer,
        stream_volume: &Arc<AtomicU8>,
        sample_rate: u32,
        channels: u16,
    ) -> AudioResult<pulseaudio::PlaybackStream> {
        let source = PaPlaybackSource {
            ring: ring.clone(),
            volume: stream_volume.clone(),
            underrun_burst: 0,
        };
        let params = Self::stream_params(sample_rate, channels);
        let stream = tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current()
                .block_on(client.create_playback_stream(params, source))
        })
        .map_err(|e| AudioError::OutputError(format!("PA create stream '{name}': {e}")))?;
        Ok(stream)
    }

    fn new(client: &Client, name: &str, _mixer_volume: &Arc<AtomicU8>) -> AudioResult<Self> {
        let ring = Arc::new(RingBufferInner::new(BUFFER_CAPACITY_SAMPLES));
        let stream_volume = Arc::new(AtomicU8::new(0));
        let stream = Self::create_stream(client, name, &ring, &stream_volume, 44100, 2)?;
        Ok(Self {
            stream,
            ring,
            stream_volume,
            control: None,
            decode_handle: None,
            name: name.to_string(),
            sample_rate: 44100,
            channels: 2,
        })
    }

    /// Re-opens the playback stream when the source rate or channel count
    /// differs from what is currently configured. PulseAudio mixes formats on
    /// the server, so a close-to-native rate avoids extra resampling and fixes
    /// pitch drift when a track's rate is not 44.1k (previously every stream
    /// was hard-coded to 44100 Hz).
    fn reconfigure(&mut self, client: &Client, sample_rate: u32, channels: u16) -> AudioResult<()> {
        let sample_rate = sample_rate.clamp(22050, 192000);
        let channels = if channels == 1 { 1 } else { 2 };
        if sample_rate == self.sample_rate && channels == self.channels {
            return Ok(());
        }
        let stream = Self::create_stream(
            client,
            &self.name,
            &self.ring,
            &self.stream_volume,
            sample_rate,
            channels,
        )?;
        self.stream = stream;
        self.sample_rate = sample_rate;
        self.channels = channels;
        log::info!(
            "pa: stream '{}' reopened at {} Hz / {} ch",
            self.name,
            sample_rate,
            channels
        );
        Ok(())
    }

    fn stop_decode(&mut self) {
        if let Some(ctrl) = &self.control {
            ctrl.signal_stop();
        }
        if let Some(h) = self.decode_handle.take() {
            let _ = h.join();
        }
        self.control = None;
    }

    fn cork(&self) {
        let _ = tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current().block_on(self.stream.cork())
        });
    }

    fn uncork(&self) {
        let _ = tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current().block_on(self.stream.uncork())
        });
    }

    fn flush(&self) {
        let _ = tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current().block_on(self.stream.flush())
        });
    }
}

pub struct PulseAudioMixer {
    _client: Client,
    stream_a: PaStreamState,
    stream_b: PaStreamState,
    is_a_active: bool,

    playing: Arc<AtomicBool>,
    position: Arc<Mutex<f64>>,
    duration: Arc<Mutex<f64>>,
    start_time: Arc<Mutex<Option<Instant>>>,
    start_pos: Arc<Mutex<f64>>,
    last_reported_pos: f64,

    crossfade_start: Option<Instant>,
    crossfade_duration: f64,
    /// Duration of the source staged in `standby`; promoted on a crossfade.
    standby_duration: f64,
    pending_pause: bool,
    pause_fade_start: Option<Instant>,
    stored_volume: u8,
    user_volume: Arc<AtomicU8>,

    eq_gains: EqGains,
    eq_enabled: Arc<AtomicBool>,
    pre_gain: PreGain,
    reverb_enabled: Arc<AtomicBool>,
    reverb_room_size: Arc<Mutex<f32>>,
    spectrum: Arc<Mutex<Vec<f32>>>,
}

impl PulseAudioMixer {
    pub fn new() -> AudioResult<Self> {
        let client = Client::from_env(c"gtm")
            .map_err(|e| AudioError::OutputError(format!("PulseAudio client: {e}")))?;

        let mixer_volume = Arc::new(AtomicU8::new(MAX_VOLUME));

        let stream_a = PaStreamState::new(&client, "gtm-a", &mixer_volume)?;
        let stream_b = PaStreamState::new(&client, "gtm-b", &mixer_volume)?;

        Ok(Self {
            _client: client,
            stream_a,
            stream_b,
            is_a_active: true,
            playing: Arc::new(AtomicBool::new(false)),
            position: Arc::new(Mutex::new(0.0)),
            duration: Arc::new(Mutex::new(0.0)),
            start_time: Arc::new(Mutex::new(None)),
            start_pos: Arc::new(Mutex::new(0.0)),
            last_reported_pos: f64::NEG_INFINITY,
            crossfade_start: None,
            crossfade_duration: 0.0,
            standby_duration: 0.0,
            pending_pause: false,
            pause_fade_start: None,
            stored_volume: MAX_VOLUME,
            user_volume: Arc::new(AtomicU8::new(MAX_VOLUME)),
            eq_gains: EqGains::new_flat(),
            eq_enabled: Arc::new(AtomicBool::new(true)),
            pre_gain: PreGain::default(),
            reverb_enabled: Arc::new(AtomicBool::new(false)),
            reverb_room_size: Arc::new(Mutex::new(0.3)),
            spectrum: Arc::new(Mutex::new(Vec::new())),
        })
    }

    fn active(&self) -> &PaStreamState {
        if self.is_a_active {
            &self.stream_a
        } else {
            &self.stream_b
        }
    }

    fn active_mut(&mut self) -> &mut PaStreamState {
        if self.is_a_active {
            &mut self.stream_a
        } else {
            &mut self.stream_b
        }
    }

    fn standby(&self) -> &PaStreamState {
        if self.is_a_active {
            &self.stream_b
        } else {
            &self.stream_a
        }
    }

    fn standby_mut(&mut self) -> &mut PaStreamState {
        if self.is_a_active {
            &mut self.stream_b
        } else {
            &mut self.stream_a
        }
    }

    fn decode(path: &str) -> AudioResult<Box<dyn Source<Item = f32> + Send>> {
        let file = File::open(path).map_err(|e| AudioError::OpenFailed(e.to_string()))?;
        let reader = BufReader::new(file);
        if let Ok(source) = rodio::Decoder::new(reader) {
            return Ok(Box::new(source));
        }
        SymphoniaSource::from_file(path, 0.0)
    }

    fn probe_duration(path: &str) -> AudioResult<f64> {
        let source = Self::decode(path)?;
        Ok(source
            .total_duration()
            .map(|d| d.as_secs_f64())
            .unwrap_or(0.0))
    }

    /// Swap the prepared standby stream into the active slot without ever
    /// silencing the outgoing stream before the incoming one is ready.
    ///
    /// Load paths stage their decode thread and prebuffer in the standby ring
    /// first. Only after that has succeeded does this stop the old feed, take
    /// the incoming control/handle, and move it to the active slot. A selected
    /// track therefore cannot leave the output silent while it is opened,
    /// probed, or primed.
    fn promote_prepared_standby(&mut self, start_pos: f64, duration: Option<f64>, audible: bool) {
        // Stop the outgoing decode only now; its stream was still draining the
        // existing ring up to this point.
        self.active_mut().stop_decode();
        Self::set_stream_volume(self.active(), 0);
        self.active().cork();

        // The incoming handles must be taken before the toggle: afterwards
        // `standby_mut()` refers to the outgoing stream.
        let control = self.standby_mut().control.take();
        let handle = self.standby_mut().decode_handle.take();

        self.is_a_active = !self.is_a_active;
        self.active_mut().control = control;
        self.active_mut().decode_handle = handle;
        let vol = if audible {
            volume_from_ratio(volume_ratio(self.user_volume.load(Ordering::SeqCst)))
        } else {
            0
        };
        Self::set_stream_volume(self.active(), vol);
        self.active().uncork();

        // The old active stream is the new standby: discard whatever was
        // queued and leave it corked until the next staged load.
        self.standby().flush();
        self.standby().cork();
        Self::set_stream_volume(self.standby(), 0);
        Self::reset_ring(self.standby());

        if let Some(duration) = duration {
            *self.duration.lock().unwrap() = duration;
        }
        *self.position.lock().unwrap() = start_pos;
        *self.start_time.lock().unwrap() = None;
        *self.start_pos.lock().unwrap() = start_pos;
        self.playing.store(false, Ordering::SeqCst);
        self.crossfade_start = None;
        self.standby_duration = 0.0;
    }

    fn wrap_source(
        &self,
        source: Box<dyn Source<Item = f32> + Send>,
    ) -> Box<dyn Source<Item = f32> + Send> {
        // Pre-gain first, so the EQ and reverb see the level it produced --
        // which is the whole point of calling it pre-gain. Boxed so the two
        // branches below keep one concrete type to share.
        let source: Box<dyn Source<Item = f32> + Send> =
            Box::new(PreGainSource::new(source, self.pre_gain.clone()));
        let source = if self.eq_enabled.load(Ordering::Relaxed) {
            Box::new(EqSource::new(source, self.eq_gains.clone()))
                as Box<dyn Source<Item = f32> + Send>
        } else {
            source
        };
        if self.reverb_enabled.load(Ordering::Relaxed) {
            let room_size = *self.reverb_room_size.lock().unwrap();
            Box::new(ReverbSource::new(
                source,
                room_size,
                self.reverb_enabled.clone(),
            )) as Box<dyn Source<Item = f32> + Send>
        } else {
            source
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn start_decode(
        path: &str,
        ring: &SharedRingBuffer,
        eq_gains: &EqGains,
        eq_enabled: &Arc<AtomicBool>,
        pre_gain: &PreGain,
        reverb_enabled: &Arc<AtomicBool>,
        reverb_room_size: &Arc<Mutex<f32>>,
        spectrum: &Arc<Mutex<Vec<f32>>>,
        prebuffer_samples: usize,
    ) -> AudioResult<(Arc<DecodeControl>, std::thread::JoinHandle<()>)> {
        let control = Arc::new(DecodeControl::new());
        let thread = DecodeThread::new(
            path.to_string(),
            ring.clone(),
            control.clone(),
            eq_gains.clone(),
            eq_enabled.clone(),
            pre_gain.clone(),
            reverb_enabled.clone(),
            reverb_room_size.clone(),
            spectrum.clone(),
            WaveformShared::default(),
            prebuffer_samples,
        );
        let handle = thread.spawn().map_err(AudioError::DecodeError)?;

        let start = Instant::now();
        let timeout = Duration::from_secs(5);
        while !control.ready.load(Ordering::Acquire) && start.elapsed() < timeout {
            if control.finished.load(Ordering::Acquire) {
                return Err(AudioError::DecodeError("stream could not be opened".into()));
            }
            if !control.running.load(Ordering::Acquire) {
                return Err(AudioError::DecodeError(
                    "decode thread exited before prebuffer".into(),
                ));
            }
            std::thread::sleep(Duration::from_millis(5));
        }

        Ok((control, handle))
    }

    #[allow(clippy::too_many_arguments)]
    fn start_decode_reader(
        reader: Box<dyn std::io::Read + Send>,
        reopen: Option<Box<dyn StreamingReopen>>,
        ring: &SharedRingBuffer,
        eq_gains: &EqGains,
        eq_enabled: &Arc<AtomicBool>,
        pre_gain: &PreGain,
        reverb_enabled: &Arc<AtomicBool>,
        reverb_room_size: &Arc<Mutex<f32>>,
        spectrum: &Arc<Mutex<Vec<f32>>>,
    ) -> AudioResult<(Arc<DecodeControl>, std::thread::JoinHandle<()>)> {
        let control = Arc::new(DecodeControl::new());
        let thread = DecodeThread::new_reader(
            reader,
            reopen,
            ring.clone(),
            control.clone(),
            eq_gains.clone(),
            eq_enabled.clone(),
            pre_gain.clone(),
            reverb_enabled.clone(),
            reverb_room_size.clone(),
            spectrum.clone(),
            WaveformShared::default(),
            PREBUFFER_SAMPLES_REDUCED,
        );
        let handle = thread.spawn().map_err(AudioError::DecodeError)?;

        // Same readiness contract as `start_decode_reader` on `AudioMixer`:
        // a live reader that fails to probe exits early with `finished` set
        // (and `ready` never set), so this wait returns immediately instead
        // of stalling playback for the whole timeout.
        let start = Instant::now();
        let timeout = Duration::from_secs(5);
        while !control.ready.load(Ordering::Acquire) && start.elapsed() < timeout {
            if control.finished.load(Ordering::Acquire) {
                return Err(AudioError::DecodeError(
                    "live stream could not be opened".into(),
                ));
            }
            if !control.running.load(Ordering::Acquire) {
                return Err(AudioError::DecodeError(
                    "decode thread exited before prebuffer".into(),
                ));
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        if !control.ready.load(Ordering::Acquire) {
            return Err(AudioError::DecodeError(
                "live stream did not become ready in time".into(),
            ));
        }

        Ok((control, handle))
    }

    /// Same as [`Self::start_decode_reader`] for a provider-decoded source
    /// (Spotify). The wait is bounded but not fatal: a slow first packet is a
    /// late start, and failing the play would make it a dead button.
    #[allow(clippy::too_many_arguments)]
    fn start_decode_stream(
        source: Box<dyn Source<Item = f32> + Send>,
        ring: &SharedRingBuffer,
        eq_gains: &EqGains,
        eq_enabled: &Arc<AtomicBool>,
        pre_gain: &PreGain,
        reverb_enabled: &Arc<AtomicBool>,
        reverb_room_size: &Arc<Mutex<f32>>,
        spectrum: &Arc<Mutex<Vec<f32>>>,
    ) -> AudioResult<(Arc<DecodeControl>, std::thread::JoinHandle<()>)> {
        let control = Arc::new(DecodeControl::new());
        let thread = DecodeThread::new_stream(
            source,
            ring.clone(),
            control.clone(),
            eq_gains.clone(),
            eq_enabled.clone(),
            pre_gain.clone(),
            reverb_enabled.clone(),
            reverb_room_size.clone(),
            spectrum.clone(),
            WaveformShared::default(),
            PREBUFFER_SAMPLES_REDUCED,
        );
        let handle = thread.spawn().map_err(AudioError::DecodeError)?;

        let start = Instant::now();
        let timeout = STREAM_PREBUFFER_TIMEOUT;
        while !control.ready.load(Ordering::Acquire) && start.elapsed() < timeout {
            if control.finished.load(Ordering::Acquire) {
                return Err(AudioError::DecodeError(
                    "stream ended before producing audio".into(),
                ));
            }
            if !control.running.load(Ordering::Acquire) {
                return Err(AudioError::DecodeError(
                    "decode thread exited before prebuffer".into(),
                ));
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        if !control.ready.load(Ordering::Acquire) {
            log::warn!(
                "provider stream produced no audio within {}s; starting anyway",
                timeout.as_secs()
            );
        }

        Ok((control, handle))
    }

    fn set_stream_volume(stream: &PaStreamState, vol: u8) {
        stream.stream_volume.store(vol, Ordering::Relaxed);
    }

    /// Drop samples and the end marker left by the previous source before a
    /// new decode thread starts filling this ring.
    fn reset_ring(stream: &PaStreamState) {
        stream.ring.flush();
        stream.ring.set_finished(false);
    }
}

impl Mixer for PulseAudioMixer {
    fn load_active(&mut self, path: &str, start_pos: f64) -> AudioResult<()> {
        let dur = Self::probe_duration(path)?;

        // Stage in the standby stream while the outgoing track keeps playing.
        self.standby_mut().stop_decode();
        Self::reset_ring(self.standby());
        self.standby().cork();
        self.standby().flush();
        Self::set_stream_volume(self.standby(), 0);

        let (control, handle) = Self::start_decode(
            path,
            &self.standby().ring,
            &self.eq_gains,
            &self.eq_enabled,
            &self.pre_gain,
            &self.reverb_enabled,
            &self.reverb_room_size,
            &self.spectrum,
            PREBUFFER_SAMPLES,
        )?;

        let client = self._client.clone();
        self.standby_mut().reconfigure(
            &client,
            control.sample_rate.load(Ordering::Relaxed),
            control.channels.load(Ordering::Relaxed),
        )?;
        self.standby_mut().control = Some(control);
        self.standby_mut().decode_handle = Some(handle);
        self.standby().cork();
        Self::set_stream_volume(self.standby(), 0);

        self.promote_prepared_standby(start_pos, (dur > 0.0).then_some(dur), false);
        Ok(())
    }

    fn load_active_decoded(
        &mut self,
        source: Box<dyn Source<Item = f32> + Send>,
        start_pos: f64,
    ) -> AudioResult<()> {
        // A decoded source still decodes lazily, so run it through the same
        // prebuffering stream path as a provider source.
        let duration = source
            .total_duration()
            .map(|d| d.as_secs_f64())
            .unwrap_or(0.0);
        self.load_active_stream(source, start_pos, duration)
    }

    fn load_active_reader(
        &mut self,
        reader: Box<dyn std::io::Read + Send>,
        reopen: Option<Box<dyn StreamingReopen>>,
        start_pos: f64,
    ) -> AudioResult<()> {
        // Open and prime the transport in the standby ring while the outgoing
        // track remains audible.
        self.standby_mut().stop_decode();
        Self::reset_ring(self.standby());
        self.standby().cork();
        self.standby().flush();
        Self::set_stream_volume(self.standby(), 0);

        let (control, handle) = Self::start_decode_reader(
            reader,
            reopen,
            &self.standby().ring,
            &self.eq_gains,
            &self.eq_enabled,
            &self.pre_gain,
            &self.reverb_enabled,
            &self.reverb_room_size,
            &self.spectrum,
        )?;

        let client = self._client.clone();
        self.standby_mut().reconfigure(
            &client,
            control.sample_rate.load(Ordering::Relaxed),
            control.channels.load(Ordering::Relaxed),
        )?;
        self.standby_mut().control = Some(control);
        self.standby_mut().decode_handle = Some(handle);
        self.standby().cork();
        Self::set_stream_volume(self.standby(), 0);

        // Live streams have no meaningful total duration; zero it so the
        // Finished stall-guard is permanently disabled for this source and a
        // genuine EOF ends it via the ring's `finished` flag instead.
        self.promote_prepared_standby(start_pos, Some(0.0), false);
        Ok(())
    }

    fn load_active_stream(
        &mut self,
        source: Box<dyn Source<Item = f32> + Send>,
        start_pos: f64,
        duration_secs: f64,
    ) -> AudioResult<()> {
        // Prime the provider source in the standby ring before stopping the
        // outgoing stream. A slow provider start therefore cannot silence the
        // track that is already playing.
        self.standby_mut().stop_decode();
        Self::reset_ring(self.standby());
        self.standby().cork();
        self.standby().flush();
        Self::set_stream_volume(self.standby(), 0);

        let (control, handle) = Self::start_decode_stream(
            source,
            &self.standby().ring,
            &self.eq_gains,
            &self.eq_enabled,
            &self.pre_gain,
            &self.reverb_enabled,
            &self.reverb_room_size,
            &self.spectrum,
        )?;

        let client = self._client.clone();
        self.standby_mut().reconfigure(
            &client,
            control.sample_rate.load(Ordering::Relaxed),
            control.channels.load(Ordering::Relaxed),
        )?;
        self.standby_mut().control = Some(control);
        self.standby_mut().decode_handle = Some(handle);
        self.standby().cork();
        Self::set_stream_volume(self.standby(), 0);

        // A provider track has a real length, so keep it for the Finished
        // stall-guard and position reporting; zero it only if unknown.
        let duration = if duration_secs > 0.0 {
            duration_secs
        } else {
            0.0
        };
        self.promote_prepared_standby(start_pos, Some(duration), false);
        Ok(())
    }

    fn dropped_samples(&self) -> u64 {
        // Both streams share the counter the decode thread bumps; the active
        // one is the only one that can have dropped anything this run.
        [Some(&self.stream_a), Some(&self.stream_b)]
            .into_iter()
            .flatten()
            .filter_map(|s| s.control.as_ref())
            .map(|c| c.dropped_samples())
            .max()
            .unwrap_or(0)
    }
    fn load_standby(&mut self, path: &str) -> AudioResult<()> {
        self.standby_mut().stop_decode();
        Self::reset_ring(self.standby());

        self.standby().cork();
        self.standby().flush();
        Self::set_stream_volume(self.standby(), 0);
        self.standby_duration = Self::probe_duration(path).unwrap_or(0.0);

        let (control, handle) = Self::start_decode(
            path,
            &self.standby().ring,
            &self.eq_gains,
            &self.eq_enabled,
            &self.pre_gain,
            &self.reverb_enabled,
            &self.reverb_room_size,
            &self.spectrum,
            PREBUFFER_SAMPLES,
        )?;

        let client = self._client.clone();
        self.standby_mut().reconfigure(
            &client,
            control.sample_rate.load(Ordering::Relaxed),
            control.channels.load(Ordering::Relaxed),
        )?;
        self.standby_mut().control = Some(control);
        self.standby_mut().decode_handle = Some(handle);

        self.standby().uncork();
        Ok(())
    }

    fn load_standby_decoded(
        &mut self,
        source: Box<dyn Source<Item = f32> + Send>,
    ) -> AudioResult<()> {
        self.standby_mut().stop_decode();
        Self::reset_ring(self.standby());

        self.standby().cork();
        self.standby().flush();
        Self::set_stream_volume(self.standby(), 0);
        self.standby_duration = source
            .total_duration()
            .map(|d| d.as_secs_f64())
            .unwrap_or(0.0);

        let rate = source.sample_rate().get();
        let channels = source.channels().get();
        let client = self._client.clone();
        self.standby_mut().reconfigure(&client, rate, channels)?;
        let source = self.wrap_source(source);

        let ring = self.standby().ring.clone();
        let handle = std::thread::Builder::new()
            .name("gtm-pa-feed-std".into())
            .spawn(move || {
                for sample in source {
                    while !ring.push(sample) {
                        // Ring full (PA consumer drains it): nap instead of
                        // spinning to keep the feed thread off a hot core.
                        std::thread::sleep(std::time::Duration::from_micros(50));
                    }
                }
                ring.set_finished(true);
            })
            .expect("failed to spawn PA feed thread");

        self.standby_mut().control = Some(Arc::new(DecodeControl::new()));
        self.standby_mut().decode_handle = Some(handle);

        self.standby().uncork();
        Ok(())
    }

    fn standby_is_loaded(&self) -> bool {
        self.standby().ring.available() > 0 || !self.standby().ring.is_finished()
    }

    fn play(&mut self) -> AudioResult<()> {
        let ring = self.active().ring.clone();
        if ring.available() == 0 && !ring.is_finished() {
            return Ok(());
        }

        if self.pending_pause {
            self.pending_pause = false;
            self.pause_fade_start = None;
            self.active().uncork();
        }
        // Every `load_active_*` zeroes the active stream so a priming ring
        // cannot leak audio, and `stop` zeroes both. Restoring the volume only
        // on a resume-from-pause left a freshly loaded track scaled to silence
        // for its whole length: the callback still produced samples, but every
        // one of them was multiplied by zero.
        Self::set_stream_volume(self.active(), self.user_volume.load(Ordering::SeqCst));

        *self.start_time.lock().unwrap() = Some(Instant::now());
        self.playing.store(true, Ordering::SeqCst);
        Ok(())
    }

    fn pause(&mut self) -> AudioResult<()> {
        if !self.playing.load(Ordering::SeqCst) {
            return Ok(());
        }
        self.stored_volume = self.active().stream_volume.load(Ordering::Relaxed);
        self.pending_pause = true;
        self.pause_fade_start = Some(Instant::now());
        Ok(())
    }

    fn stop(&mut self) -> AudioResult<()> {
        Self::set_stream_volume(self.active(), 0);
        Self::set_stream_volume(self.standby(), 0);
        self.active().cork();
        self.standby().cork();
        self.active().flush();
        self.standby().flush();
        *self.position.lock().unwrap() = 0.0;
        self.playing.store(false, Ordering::SeqCst);
        *self.start_time.lock().unwrap() = None;
        *self.start_pos.lock().unwrap() = 0.0;
        self.crossfade_start = None;
        Ok(())
    }

    fn seek(&mut self, position_secs: f64) -> AudioResult<()> {
        let Some(ref ctrl) = self.active().control else {
            return Ok(());
        };
        ctrl.signal_seek(position_secs);
        let deadline = Instant::now() + Duration::from_secs(2);
        while ctrl.seeking.load(Ordering::Acquire) && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(5));
        }
        self.active().flush();
        *self.position.lock().unwrap() = position_secs;
        *self.start_time.lock().unwrap() = (position_secs > 0.0).then(Instant::now);
        *self.start_pos.lock().unwrap() = position_secs;
        Ok(())
    }

    fn set_volume(&mut self, volume: u8) -> AudioResult<()> {
        let vol = volume.min(MAX_VOLUME);
        self.user_volume.store(vol, Ordering::SeqCst);
        if !self.pending_pause {
            Self::set_stream_volume(self.active(), vol);
        }
        Ok(())
    }

    fn volume(&self) -> u8 {
        self.user_volume.load(Ordering::SeqCst)
    }

    fn is_playing(&self) -> bool {
        self.playing.load(Ordering::SeqCst) && !self.pending_pause
    }

    fn current_position(&self) -> f64 {
        let elapsed = self
            .start_time
            .lock()
            .unwrap()
            .map(|t| t.elapsed().as_secs_f64())
            .unwrap_or(0.0);
        let start = *self.start_pos.lock().unwrap();
        let total = *self.duration.lock().unwrap();
        (start + elapsed).min(total)
    }

    fn duration(&self) -> f64 {
        *self.duration.lock().unwrap()
    }

    fn active_remaining(&self) -> f64 {
        let total = *self.duration.lock().unwrap();
        if total <= 0.0 {
            return 0.0;
        }
        (total - self.current_position()).max(0.0)
    }

    fn start_crossfade(&mut self, duration_secs: f64) {
        if self.standby().ring.available() == 0 && self.standby().ring.is_finished() {
            return;
        }
        Self::set_stream_volume(self.standby(), 0);
        self.crossfade_start = Some(Instant::now());
        self.crossfade_duration = duration_secs.max(1.0);
    }

    fn is_crossfading(&self) -> bool {
        self.crossfade_start.is_some()
    }

    fn cancel_crossfade(&mut self) {
        let crossfading = self.crossfade_start.take().is_some();
        if !crossfading && self.standby().control.is_none() && self.standby().ring.available() == 0
        {
            return;
        }
        // Keep the outgoing stream, restore its user volume if a ramp had
        // started, and discard the partially faded-in standby.
        if crossfading {
            let vol = volume_from_ratio(volume_ratio(self.user_volume.load(Ordering::SeqCst)));
            Self::set_stream_volume(self.active(), vol);
            self.active().uncork();
        }
        self.standby().flush();
        Self::set_stream_volume(self.standby(), 0);
        self.standby().cork();
        self.standby_mut().stop_decode();
        Self::reset_ring(self.standby());
        self.standby_duration = 0.0;
    }

    fn force_complete_crossfade(&mut self) {
        if self.crossfade_start.is_none() {
            return;
        }
        self.swap_active_standby();
    }

    fn drop_active(&mut self) {
        if self.crossfade_start.is_none() {
            return;
        }
        self.swap_active_standby();
    }

    fn poll(&mut self) -> AudioResult<Option<AudioEvent>> {
        if self.crossfade_start.is_some() {
            self.step_crossfade();
        }
        if self.pending_pause {
            let fade_start = match self.pause_fade_start {
                Some(s) => s,
                None => return Ok(None),
            };
            const FADE_MS: f64 = 150.0;
            let elapsed = fade_start.elapsed().as_secs_f64() * 1000.0;
            if elapsed >= FADE_MS {
                self.pending_pause = false;
                self.pause_fade_start = None;
                self.active().cork();
                let paused_pos = self.current_position();
                *self.position.lock().unwrap() = paused_pos;
                *self.start_pos.lock().unwrap() = paused_pos;
                *self.start_time.lock().unwrap() = None;
                self.playing.store(false, Ordering::SeqCst);
            } else {
                let progress = elapsed / FADE_MS;
                let target =
                    volume_ratio(self.stored_volume.min(MAX_VOLUME)) * (1.0 - progress as f32);
                Self::set_stream_volume(self.active(), volume_from_ratio(target));
            }
        }

        let ring = self.active().ring.clone();
        let finished = ring.is_finished() && ring.available() == 0;
        if finished {
            if self.crossfade_start.is_some() {
                self.force_complete_crossfade();
                if self.active().ring.available() > 0 || !self.active().ring.is_finished() {
                    return Ok(None);
                }
            }
            if self.playing.load(Ordering::SeqCst) {
                let total = *self.duration.lock().unwrap();
                let pos = self.current_position();
                if total > 0.0 && pos < total - 0.5 {
                    return Ok(None);
                }
                self.playing.store(false, Ordering::SeqCst);
                return Ok(Some(AudioEvent::Finished));
            }
            return Ok(None);
        }

        if self.playing.load(Ordering::SeqCst) && !self.pending_pause {
            let pos = self.current_position();
            *self.position.lock().unwrap() = pos;
            if (pos - self.last_reported_pos).abs() >= 0.05 {
                self.last_reported_pos = pos;
                return Ok(Some(AudioEvent::Position(pos)));
            }
        }

        Ok(None)
    }

    fn set_eq_preset(&self, preset: &EqPreset) {
        self.eq_gains.apply_preset(preset);
    }

    fn set_eq_enabled(&self, enabled: bool) {
        self.eq_enabled.store(enabled, Ordering::Relaxed);
    }

    fn set_pre_gain(&self, db: f32) {
        self.pre_gain.set_db(db);
    }

    fn set_reverb(&self, config: &ReverbConfig) {
        self.reverb_enabled.store(config.enabled, Ordering::Relaxed);
        *self.reverb_room_size.lock().unwrap() = config.room_size;
    }

    fn current_peak_level(&self) -> f32 {
        if !self.playing.load(Ordering::SeqCst) {
            return 0.0;
        }
        volume_ratio(self.user_volume.load(Ordering::SeqCst))
    }
    fn current_spectrum(&self) -> Vec<f32> {
        self.spectrum.lock().unwrap().clone()
    }
}

impl PulseAudioMixer {
    /// Promote the standby that the crossfade just completed.
    fn swap_active_standby(&mut self) {
        let duration = self.standby_duration;
        self.promote_prepared_standby(0.0, Some(duration), true);
        *self.start_time.lock().unwrap() = Some(Instant::now());
        self.playing.store(true, Ordering::SeqCst);
    }

    fn step_crossfade(&mut self) -> bool {
        let start = match self.crossfade_start {
            Some(s) => s,
            None => return false,
        };
        let elapsed = start.elapsed().as_secs_f64();
        let progress = (elapsed / self.crossfade_duration).min(1.0);
        let eased_out = 1.0 - progress;
        let eased_in = progress;

        let a_vol = if self.is_a_active {
            eased_out
        } else {
            eased_in
        };
        let b_vol = if self.is_a_active {
            eased_in
        } else {
            eased_out
        };

        Self::set_stream_volume(&self.stream_a, volume_from_ratio(a_vol as f32));
        Self::set_stream_volume(&self.stream_b, volume_from_ratio(b_vol as f32));

        if progress >= 1.0 {
            self.swap_active_standby();
            return true;
        }
        false
    }
}
