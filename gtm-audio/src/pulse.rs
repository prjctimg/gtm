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

use crate::backend::{AudioError, AudioEvent, AudioResult};
use crate::buffer::{DecodeControl, PREBUFFER_SAMPLES, RingBufferInner, SharedRingBuffer};
use crate::decoder::DecodeThread;
use crate::eq::{EqGains, EqSource, ReverbSource};
use crate::mixer::Mixer;
use crate::symphonia::SymphoniaSource;
use gtm_core::global::{EqPreset, ReverbConfig};
use rodio::Source;

struct PaPlaybackSource {
    ring: SharedRingBuffer,
    volume: Arc<AtomicU8>,
}

impl PlaybackSource for PaPlaybackSource {
    fn poll_read(self: Pin<&mut Self>, _cx: &mut Context<'_>, buf: &mut [u8]) -> Poll<usize> {
        let vol = self.volume.load(Ordering::Relaxed) as f32 / 100.0;
        let float_count = buf.len() / 4;
        let mut written = 0usize;

        for _i in 0..float_count {
            match self.ring.pop() {
                Some(s) => {
                    let scaled = s * vol;
                    let offset = written * 4;
                    buf[offset..offset + 4].copy_from_slice(&scaled.to_le_bytes());
                    written += 1;
                }
                None => {
                    if self.ring.is_finished() {
                        break;
                    }
                    if written > 0 {
                        return Poll::Ready(written * 4);
                    }
                    return Poll::Pending;
                }
            }
        }

        if written == 0 && self.ring.is_finished() {
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
}

impl PaStreamState {
    fn new(client: &Client, name: &str, _mixer_volume: &Arc<AtomicU8>) -> AudioResult<Self> {
        let ring = Arc::new(RingBufferInner::new(44100 * 2 * 3));
        let stream_volume = Arc::new(AtomicU8::new(0));

        let source = PaPlaybackSource {
            ring: ring.clone(),
            volume: stream_volume.clone(),
        };

        let params = protocol::PlaybackStreamParams {
            sample_spec: protocol::SampleSpec {
                format: protocol::SampleFormat::Float32Le,
                channels: 2,
                sample_rate: 44100,
            },
            channel_map: protocol::ChannelMap::stereo(),
            cvolume: Some(protocol::ChannelVolume::muted(2)),
            buffer_attr: protocol::stream::BufferAttr {
                max_length: u32::MAX,
                target_length: (44100 * 2 * 4 * 2) as u32, // ~2s buffer
                pre_buffering: (PREBUFFER_SAMPLES * 4) as u32,
                minimum_request_length: 1024,
                fragment_size: u32::MAX,
            },
            ..Default::default()
        };

        let stream = tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current()
                .block_on(client.create_playback_stream(params, source))
        })
        .map_err(|e| AudioError::OutputError(format!("PA create stream '{name}': {e}")))?;

        Ok(Self {
            stream,
            ring,
            stream_volume,
            control: None,
            decode_handle: None,
        })
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
    pending_pause: bool,
    pause_fade_start: Option<Instant>,
    stored_volume: u8,
    user_volume: Arc<AtomicU8>,

    eq_gains: EqGains,
    eq_enabled: Arc<AtomicBool>,
    reverb_enabled: Arc<AtomicBool>,
    reverb_room_size: Arc<Mutex<f32>>,
    spectrum: Arc<Mutex<Vec<f32>>>,
}

impl PulseAudioMixer {
    pub fn new() -> AudioResult<Self> {
        let client = Client::from_env(c"gtm")
            .map_err(|e| AudioError::OutputError(format!("PulseAudio client: {e}")))?;

        let mixer_volume = Arc::new(AtomicU8::new(100));

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
            pending_pause: false,
            pause_fade_start: None,
            stored_volume: 100,
            user_volume: Arc::new(AtomicU8::new(100)),
            eq_gains: EqGains::new_flat(),
            eq_enabled: Arc::new(AtomicBool::new(true)),
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

    fn start_decode(
        path: &str,
        ring: &SharedRingBuffer,
        eq_gains: &EqGains,
        eq_enabled: &Arc<AtomicBool>,
        reverb_enabled: &Arc<AtomicBool>,
        reverb_room_size: &Arc<Mutex<f32>>,
        spectrum: &Arc<Mutex<Vec<f32>>>,
    ) -> AudioResult<(Arc<DecodeControl>, std::thread::JoinHandle<()>)> {
        let control = Arc::new(DecodeControl::new());
        let thread = DecodeThread::new(
            path.to_string(),
            ring.clone(),
            control.clone(),
            eq_gains.clone(),
            eq_enabled.clone(),
            reverb_enabled.clone(),
            reverb_room_size.clone(),
            spectrum.clone(),
        );
        let handle = thread.spawn().map_err(AudioError::DecodeError)?;

        let start = Instant::now();
        let timeout = Duration::from_secs(5);
        while !control.ready.load(Ordering::Acquire) && start.elapsed() < timeout {
            if !control.running.load(Ordering::Acquire) {
                return Err(AudioError::DecodeError(
                    "decode thread exited before prebuffer".into(),
                ));
            }
            std::thread::sleep(Duration::from_millis(5));
        }

        Ok((control, handle))
    }

    fn set_stream_volume(stream: &PaStreamState, vol: u8) {
        stream.stream_volume.store(vol, Ordering::Relaxed);
    }

    fn swap_active_standby(&mut self) {
        let vol = self.get_mixer_volume();

        self.active_mut().stop_decode();

        self.is_a_active = !self.is_a_active;

        Self::set_stream_volume(&self.active(), vol);
        self.active().uncork();
        self.standby().flush();
        self.standby().cork();
        Self::set_stream_volume(&self.standby(), 0);

        self.active_mut().control = self.standby_mut().control.take();
        self.active_mut().decode_handle = self.standby_mut().decode_handle.take();

        *self.start_time.lock().unwrap() = Some(Instant::now());
        *self.start_pos.lock().unwrap() = 0.0;
        self.playing.store(true, Ordering::SeqCst);
    }

    fn get_mixer_volume(&self) -> u8 {
        self.active().stream_volume.load(Ordering::Relaxed)
    }
}

impl Mixer for PulseAudioMixer {
    fn load_active(&mut self, path: &str, start_pos: f64) -> AudioResult<()> {
        self.active_mut().stop_decode();

        self.active().cork();
        self.active().flush();
        Self::set_stream_volume(&self.active(), 0);

        let dur = Self::probe_duration(path)?;
        if dur > 0.0 {
            *self.duration.lock().unwrap() = dur;
        }

        let (control, handle) = Self::start_decode(
            path,
            &self.active().ring,
            &self.eq_gains,
            &self.eq_enabled,
            &self.reverb_enabled,
            &self.reverb_room_size,
            &self.spectrum,
        )?;

        self.active_mut().control = Some(control);
        self.active_mut().decode_handle = Some(handle);

        self.active().uncork();

        *self.position.lock().unwrap() = start_pos;
        *self.start_time.lock().unwrap() = None;
        *self.start_pos.lock().unwrap() = start_pos;
        self.playing.store(false, Ordering::SeqCst);
        self.crossfade_start = None;

        Ok(())
    }

    fn load_active_decoded(
        &mut self,
        source: Box<dyn Source<Item = f32> + Send>,
        start_pos: f64,
    ) -> AudioResult<()> {
        self.active_mut().stop_decode();

        self.active().cork();
        self.active().flush();
        Self::set_stream_volume(&self.active(), 0);

        if let Some(ref dur) = source.total_duration() {
            *self.duration.lock().unwrap() = dur.as_secs_f64();
        }

        let source = if self.eq_enabled.load(Ordering::Relaxed) {
            Box::new(EqSource::new(source, self.eq_gains.clone()))
        } else {
            source
        };
        let source = if self.reverb_enabled.load(Ordering::Relaxed) {
            let room_size = *self.reverb_room_size.lock().unwrap();
            Box::new(ReverbSource::new(
                source,
                room_size,
                self.reverb_enabled.clone(),
            ))
        } else {
            source
        };

        let ring = self.active().ring.clone();
        let handle = std::thread::Builder::new()
            .name("gtm-pa-feed".into())
            .spawn(move || {
                for sample in source {
                    while !ring.push(sample) {
                        std::thread::yield_now();
                    }
                }
                ring.set_finished(true);
            })
            .expect("failed to spawn PA feed thread");

        self.active_mut().control = Some(Arc::new(DecodeControl::new()));
        self.active_mut().decode_handle = Some(handle);

        self.active().uncork();

        *self.position.lock().unwrap() = start_pos;
        *self.start_time.lock().unwrap() = None;
        *self.start_pos.lock().unwrap() = start_pos;
        self.playing.store(false, Ordering::SeqCst);
        self.crossfade_start = None;

        Ok(())
    }

    fn load_standby(&mut self, path: &str) -> AudioResult<()> {
        self.standby_mut().stop_decode();

        self.standby().cork();
        self.standby().flush();
        Self::set_stream_volume(&self.standby(), 0);

        let (control, handle) = Self::start_decode(
            path,
            &self.standby().ring,
            &self.eq_gains,
            &self.eq_enabled,
            &self.reverb_enabled,
            &self.reverb_room_size,
            &self.spectrum,
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

        self.standby().cork();
        self.standby().flush();
        Self::set_stream_volume(&self.standby(), 0);

        let source = if self.eq_enabled.load(Ordering::Relaxed) {
            Box::new(EqSource::new(source, self.eq_gains.clone()))
        } else {
            source
        };
        let source = if self.reverb_enabled.load(Ordering::Relaxed) {
            let room_size = *self.reverb_room_size.lock().unwrap();
            Box::new(ReverbSource::new(
                source,
                room_size,
                self.reverb_enabled.clone(),
            ))
        } else {
            source
        };

        let ring = self.standby().ring.clone();
        let handle = std::thread::Builder::new()
            .name("gtm-pa-feed-std".into())
            .spawn(move || {
                for sample in source {
                    while !ring.push(sample) {
                        std::thread::yield_now();
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
            Self::set_stream_volume(&self.active(), self.stored_volume);
        }

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
        Self::set_stream_volume(&self.active(), 0);
        Self::set_stream_volume(&self.standby(), 0);
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
        *self.start_time.lock().unwrap() = (position_secs > 0.0).then(|| Instant::now());
        *self.start_pos.lock().unwrap() = position_secs;
        Ok(())
    }

    fn set_volume(&mut self, volume: u8) -> AudioResult<()> {
        let vol = volume.min(100);
        self.user_volume.store(vol, Ordering::SeqCst);
        if !self.pending_pause {
            Self::set_stream_volume(&self.active(), vol);
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
        Self::set_stream_volume(&self.standby(), 0);
        self.crossfade_start = Some(Instant::now());
        self.crossfade_duration = duration_secs.max(1.0);
    }

    fn is_crossfading(&self) -> bool {
        self.crossfade_start.is_some()
    }

    fn force_complete_crossfade(&mut self) {
        if self.crossfade_start.is_none() {
            return;
        }
        self.crossfade_start = None;
        let vol = self.active().stream_volume.load(Ordering::Relaxed);

        self.active_mut().stop_decode();
        Self::set_stream_volume(&self.active(), 0);
        self.active().cork();

        self.is_a_active = !self.is_a_active;

        Self::set_stream_volume(&self.active(), vol);
        self.active().uncork();

        self.standby().flush();
        Self::set_stream_volume(&self.standby(), 0);
        self.standby().cork();

        self.active_mut().control = self.standby_mut().control.take();
        self.active_mut().decode_handle = self.standby_mut().decode_handle.take();

        *self.start_time.lock().unwrap() = Some(Instant::now());
        *self.start_pos.lock().unwrap() = 0.0;
        self.playing.store(true, Ordering::SeqCst);
    }

    fn drop_active(&mut self) {
        if self.crossfade_start.is_none() {
            return;
        }
        self.crossfade_start = None;
        let vol = self.active().stream_volume.load(Ordering::Relaxed);

        self.active_mut().stop_decode();
        Self::set_stream_volume(&self.active(), 0);
        self.active().cork();

        self.is_a_active = !self.is_a_active;

        Self::set_stream_volume(&self.active(), vol);
        self.active().uncork();

        self.standby().flush();
        Self::set_stream_volume(&self.standby(), 0);
        self.standby().cork();

        self.active_mut().control = self.standby_mut().control.take();
        self.active_mut().decode_handle = self.standby_mut().decode_handle.take();

        *self.start_time.lock().unwrap() = Some(Instant::now());
        *self.start_pos.lock().unwrap() = 0.0;
        self.playing.store(true, Ordering::SeqCst);
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
                let target = (self.stored_volume.min(100) as f32 / 100.0) * (1.0 - progress as f32);
                Self::set_stream_volume(&self.active(), (target * 100.0) as u8);
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

    fn set_reverb(&self, config: &ReverbConfig) {
        self.reverb_enabled.store(config.enabled, Ordering::Relaxed);
        *self.reverb_room_size.lock().unwrap() = config.room_size;
    }

    fn current_peak_level(&self) -> f32 {
        if !self.playing.load(Ordering::SeqCst) {
            return 0.0;
        }
        let vol = self.user_volume.load(Ordering::SeqCst) as f32 / 100.0;
        vol
    }
    fn current_spectrum(&self) -> Vec<f32> {
        self.spectrum.lock().unwrap().clone()
    }
}

impl PulseAudioMixer {
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

        Self::set_stream_volume(&self.stream_a, (a_vol * 100.0) as u8);
        Self::set_stream_volume(&self.stream_b, (b_vol * 100.0) as u8);

        if progress >= 1.0 {
            self.swap_active_standby();
            return true;
        }
        false
    }
}
