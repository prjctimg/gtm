// Copyright (c) 2026
// Author: prjctimg <prjctimg@outlook.com>
// Audio mixer: crossfade, volume, EQ, and backend abstraction
//
// This is free software released under the GPL-3.0 license.

use std::fs::File;
use std::io::BufReader;
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use rodio::{Decoder, DeviceSinkBuilder, Player, Source};

use crate::backend::{AudioError, AudioEvent, AudioResult};
use crate::buffer::{BUFFER_CAPACITY_SAMPLES, DecodeControl, RingBufferInner, RingBufferSource};
use crate::decoder::DecodeThread;
use crate::eq::{EqGains, EqSource, ReverbSource};
use crate::symphonia::SymphoniaSource;
use gtm_core::global::{Easing, EqPreset, ReverbConfig};

pub trait Mixer: Send + Sync {
    fn load_active(&mut self, path: &str, start_pos: f64) -> AudioResult<()>;
    fn load_active_decoded(
        &mut self,
        source: Box<dyn Source<Item = f32> + Send>,
        start_pos: f64,
    ) -> AudioResult<()>;
    fn load_standby(&mut self, path: &str) -> AudioResult<()>;
    fn load_standby_decoded(
        &mut self,
        source: Box<dyn Source<Item = f32> + Send>,
    ) -> AudioResult<()>;
    fn standby_is_loaded(&self) -> bool;
    fn play(&mut self) -> AudioResult<()>;
    fn pause(&mut self) -> AudioResult<()>;
    fn stop(&mut self) -> AudioResult<()>;
    fn seek(&mut self, position_secs: f64) -> AudioResult<()>;
    fn set_volume(&mut self, volume: u8) -> AudioResult<()>;
    fn volume(&self) -> u8;
    fn is_playing(&self) -> bool;
    fn current_position(&self) -> f64;
    fn duration(&self) -> f64;
    fn active_remaining(&self) -> f64;
    fn start_crossfade(&mut self, duration_secs: f64);
    fn set_crossfade_easing(&mut self, easing: Easing);
    fn is_crossfading(&self) -> bool;
    fn force_complete_crossfade(&mut self);
    fn drop_active(&mut self);
    fn poll(&mut self) -> AudioResult<Option<AudioEvent>>;
    fn current_peak_level(&self) -> f32;
    fn current_spectrum(&self) -> Vec<f32>;
    /// Publish externally produced spectrum levels (e.g. from streamed
    /// sources that bypass the decode thread). No-op by default.
    fn publish_spectrum(&self, _levels: Vec<f32>) {}

    // ─── EQ / Reverb ───
    fn set_eq_preset(&self, preset: &EqPreset);
    fn set_eq_enabled(&self, enabled: bool);
    fn set_reverb(&self, config: &ReverbConfig);
}

pub struct AudioMixer {
    #[allow(dead_code)]
    sink: Arc<MixerDeviceSink>,
    player_a: Player,
    player_b: Player,
    is_a_active: bool,
    position: Arc<Mutex<f64>>,
    duration: Arc<Mutex<f64>>,
    playing: Arc<AtomicBool>,
    volume: Arc<AtomicU8>,
    start_time: Arc<Mutex<Option<Instant>>>,
    start_pos: Arc<Mutex<f64>>,
    crossfade_start: Option<Instant>,
    crossfade_duration: f64,
    standby_duration: f64,
    pending_pause: bool,
    pause_fade_start: Option<Instant>,
    stored_volume: u8,
    last_reported_pos: f64,
    crossfade_easing: Easing,
    // ─── EQ / Reverb ───
    pub eq_gains: EqGains,
    eq_enabled: Arc<AtomicBool>,
    reverb_enabled: Arc<AtomicBool>,
    reverb_room_size: Arc<Mutex<f32>>,
    // ─── Decode thread / Ring buffer ───
    active_control: Option<Arc<DecodeControl>>,
    active_decode_handle: Option<std::thread::JoinHandle<()>>,
    standby_control: Option<Arc<DecodeControl>>,
    standby_decode_handle: Option<std::thread::JoinHandle<()>>,
    underrun_since: Option<Instant>,
    // ─── Spectrum ───
    spectrum: Arc<Mutex<Vec<f32>>>,
}

const UNDERFLOW_GRACE: Duration = Duration::from_millis(100);

struct MixerDeviceSink(rodio::MixerDeviceSink);

impl Mixer for AudioMixer {
    fn load_active(&mut self, path: &str, start_pos: f64) -> AudioResult<()> {
        self.load_active(path, start_pos)
    }
    fn load_active_decoded(
        &mut self,
        source: Box<dyn Source<Item = f32> + Send>,
        start_pos: f64,
    ) -> AudioResult<()> {
        self.load_active_decoded(source, start_pos)
    }
    fn load_standby(&mut self, path: &str) -> AudioResult<()> {
        self.load_standby(path)
    }
    fn load_standby_decoded(
        &mut self,
        source: Box<dyn Source<Item = f32> + Send>,
    ) -> AudioResult<()> {
        self.load_standby_decoded(source)
    }
    fn standby_is_loaded(&self) -> bool {
        self.standby_is_loaded()
    }
    fn play(&mut self) -> AudioResult<()> {
        self.play()
    }
    fn pause(&mut self) -> AudioResult<()> {
        self.pause()
    }
    fn stop(&mut self) -> AudioResult<()> {
        self.stop()
    }
    fn seek(&mut self, position_secs: f64) -> AudioResult<()> {
        self.seek(position_secs)
    }
    fn set_volume(&mut self, volume: u8) -> AudioResult<()> {
        self.set_volume(volume)
    }
    fn volume(&self) -> u8 {
        self.volume()
    }
    fn is_playing(&self) -> bool {
        self.is_playing()
    }
    fn current_position(&self) -> f64 {
        self.current_position()
    }
    fn duration(&self) -> f64 {
        self.duration()
    }
    fn active_remaining(&self) -> f64 {
        self.active_remaining()
    }
    fn start_crossfade(&mut self, duration_secs: f64) {
        self.start_crossfade(duration_secs)
    }
    fn set_crossfade_easing(&mut self, easing: Easing) {
        self.set_crossfade_easing(easing)
    }
    fn is_crossfading(&self) -> bool {
        self.is_crossfading()
    }
    fn force_complete_crossfade(&mut self) {
        self.force_complete_crossfade()
    }
    fn drop_active(&mut self) {
        self.drop_active()
    }
    fn poll(&mut self) -> AudioResult<Option<AudioEvent>> {
        self.poll()
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
        let vol = self.volume.load(Ordering::SeqCst) as f32 / 100.0;
        vol
    }
    fn current_spectrum(&self) -> Vec<f32> {
        self.spectrum.lock().unwrap().clone()
    }
}

impl AudioMixer {
    /// Publish externally produced spectrum levels (e.g. from streamed
    /// sources that bypass the decode thread).
    pub fn publish_spectrum(&self, levels: Vec<f32>) {
        *self.spectrum.lock().unwrap() = levels;
    }

    pub fn new() -> AudioResult<Self> {
        let mut sink = DeviceSinkBuilder::open_default_sink()
            .map_err(|e| AudioError::OutputError(e.to_string()))?;
        sink.log_on_drop(false);
        let sink = Arc::new(MixerDeviceSink(sink));
        let mixer = sink.0.mixer();
        let a = Player::connect_new(mixer);
        let b = Player::connect_new(mixer);

        b.set_volume(0.0);
        b.pause();

        Ok(Self {
            sink,
            player_a: a,
            player_b: b,
            is_a_active: true,
            position: Arc::new(Mutex::new(0.0)),
            duration: Arc::new(Mutex::new(0.0)),
            playing: Arc::new(AtomicBool::new(false)),
            volume: Arc::new(AtomicU8::new(100)),
            start_time: Arc::new(Mutex::new(None)),
            start_pos: Arc::new(Mutex::new(0.0)),
            crossfade_start: None,
            crossfade_duration: 0.0,
            standby_duration: 0.0,
            crossfade_easing: Easing::default(),
            pending_pause: false,
            pause_fade_start: None,
            stored_volume: 100,
            last_reported_pos: f64::NEG_INFINITY,
            eq_gains: EqGains::new_flat(),
            eq_enabled: Arc::new(AtomicBool::new(true)),
            reverb_enabled: Arc::new(AtomicBool::new(false)),
            reverb_room_size: Arc::new(Mutex::new(0.3)),
            active_control: None,
            active_decode_handle: None,
            standby_control: None,
            standby_decode_handle: None,
            underrun_since: None,
            spectrum: Arc::new(Mutex::new(Vec::new())),
        })
    }

    fn active(&self) -> &Player {
        if self.is_a_active {
            &self.player_a
        } else {
            &self.player_b
        }
    }

    fn standby(&self) -> &Player {
        if self.is_a_active {
            &self.player_b
        } else {
            &self.player_a
        }
    }

    pub fn decode_file(path: &str) -> AudioResult<Box<dyn Source<Item = f32> + Send>> {
        Self::decode(path)
    }

    pub fn decode_file_at(
        path: &str,
        start_pos: f64,
    ) -> AudioResult<Box<dyn Source<Item = f32> + Send>> {
        SymphoniaSource::from_file(path, start_pos)
    }

    fn decode(path: &str) -> AudioResult<Box<dyn Source<Item = f32> + Send>> {
        let file = File::open(path).map_err(|e| AudioError::OpenFailed(e.to_string()))?;
        let reader = BufReader::new(file);
        if let Ok(source) = Decoder::new(reader) {
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

    fn stop_decode_thread(
        control: &Option<Arc<DecodeControl>>,
        handle: &mut Option<std::thread::JoinHandle<()>>,
    ) {
        if let Some(ctrl) = control {
            ctrl.signal_stop();
        }
        if let Some(h) = handle.take() {
            let _ = h.join();
        }
    }

    fn wrap_source(
        &self,
        source: Box<dyn Source<Item = f32> + Send>,
    ) -> Box<dyn Source<Item = f32> + Send> {
        let boxed: Box<dyn Source<Item = f32> + Send> = if self.eq_enabled.load(Ordering::Relaxed) {
            Box::new(EqSource::new(source, self.eq_gains.clone()))
        } else {
            source
        };
        if self.reverb_enabled.load(Ordering::Relaxed) {
            let room_size = *self.reverb_room_size.lock().unwrap();
            Box::new(ReverbSource::new(
                boxed,
                room_size,
                self.reverb_enabled.clone(),
            ))
        } else {
            boxed
        }
    }

    fn start_decode_thread(
        path: &str,
        eq_gains: &EqGains,
        eq_enabled: &Arc<AtomicBool>,
        reverb_enabled: &Arc<AtomicBool>,
        reverb_room_size: &Arc<Mutex<f32>>,
        spectrum: &Arc<Mutex<Vec<f32>>>,
    ) -> AudioResult<(
        Arc<DecodeControl>,
        RingBufferSource,
        std::thread::JoinHandle<()>,
    )> {
        let control = Arc::new(DecodeControl::new());
        let shared = Arc::new(RingBufferInner::new(BUFFER_CAPACITY_SAMPLES));

        let thread = DecodeThread::new(
            path.to_string(),
            shared.clone(),
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

        let source = RingBufferSource::new(shared, control.clone());
        Ok((control, source, handle))
    }

    pub fn load_active(&mut self, path: &str, start_pos: f64) -> AudioResult<()> {
        Self::stop_decode_thread(&self.active_control, &mut self.active_decode_handle);

        let vol = self.volume.load(Ordering::SeqCst) as f32 / 100.0;
        self.active().stop();
        self.active().set_volume(vol);

        let dur = Self::probe_duration(path)?;
        if dur > 0.0 {
            *self.duration.lock().unwrap() = dur;
        }

        let (control, source, handle) = Self::start_decode_thread(
            path,
            &self.eq_gains,
            &self.eq_enabled,
            &self.reverb_enabled,
            &self.reverb_room_size,
            &self.spectrum,
        )?;

        self.active().append(source);

        self.active_control = Some(control);
        self.active_decode_handle = Some(handle);

        *self.position.lock().unwrap() = start_pos;
        *self.start_time.lock().unwrap() = None;
        *self.start_pos.lock().unwrap() = start_pos;
        self.playing.store(false, Ordering::SeqCst);
        self.crossfade_start = None;

        Ok(())
    }

    pub fn load_active_decoded(
        &mut self,
        source: Box<dyn Source<Item = f32> + Send>,
        start_pos: f64,
    ) -> AudioResult<()> {
        Self::stop_decode_thread(&self.active_control, &mut self.active_decode_handle);

        let vol = self.volume.load(Ordering::SeqCst) as f32 / 100.0;
        self.active().stop();
        self.active().set_volume(vol);

        if let Some(ref dur) = source.total_duration() {
            *self.duration.lock().unwrap() = dur.as_secs_f64();
        }
        let source = self.wrap_source(source);
        self.active().append(source);

        self.active_control = None;

        *self.position.lock().unwrap() = start_pos;
        *self.start_time.lock().unwrap() = None;
        *self.start_pos.lock().unwrap() = start_pos;
        self.playing.store(false, Ordering::SeqCst);
        self.crossfade_start = None;

        Ok(())
    }

    pub fn load_standby(&mut self, path: &str) -> AudioResult<()> {
        Self::stop_decode_thread(&self.standby_control, &mut self.standby_decode_handle);

        self.standby().stop();
        self.standby().set_volume(0.0);

        let (control, source, handle) = Self::start_decode_thread(
            path,
            &self.eq_gains,
            &self.eq_enabled,
            &self.reverb_enabled,
            &self.reverb_room_size,
            &self.spectrum,
        )?;

        self.standby().append(source);
        self.standby_control = Some(control);
        self.standby_decode_handle = Some(handle);
        Ok(())
    }

    pub fn load_standby_decoded(
        &mut self,
        source: Box<dyn Source<Item = f32> + Send>,
    ) -> AudioResult<()> {
        Self::stop_decode_thread(&self.standby_control, &mut self.standby_decode_handle);

        self.standby().stop();
        self.standby().set_volume(0.0);
        self.standby_duration = source
            .total_duration()
            .map(|d| d.as_secs_f64())
            .unwrap_or(0.0);
        let source = self.wrap_source(source);
        self.standby().append(source);
        self.standby_control = None;
        Ok(())
    }

    pub fn standby_is_loaded(&self) -> bool {
        !self.standby().empty()
    }

    pub fn play(&mut self) -> AudioResult<()> {
        if self.active().is_paused() {
            self.active().play();
            let vol = self.volume.load(Ordering::SeqCst) as f32 / 100.0;
            self.active().set_volume(vol);
        }
        if self.pending_pause {
            self.pending_pause = false;
            self.pause_fade_start = None;
            let vol = self.stored_volume.min(100) as f32 / 100.0;
            self.active().set_volume(vol);
        }
        *self.start_time.lock().unwrap() = Some(Instant::now());
        self.playing.store(true, Ordering::SeqCst);
        Ok(())
    }

    pub fn pause(&mut self) -> AudioResult<()> {
        if self.active().is_paused() {
            return Ok(());
        }
        self.stored_volume = self.volume.load(Ordering::SeqCst);
        self.pending_pause = true;
        self.pause_fade_start = Some(Instant::now());
        Ok(())
    }

    pub fn stop(&mut self) -> AudioResult<()> {
        self.player_a.stop();
        self.player_b.stop();
        *self.position.lock().unwrap() = 0.0;
        self.playing.store(false, Ordering::SeqCst);
        *self.start_time.lock().unwrap() = None;
        *self.start_pos.lock().unwrap() = 0.0;
        self.crossfade_start = None;
        Ok(())
    }

    pub fn seek(&mut self, position_secs: f64) -> AudioResult<()> {
        let Some(ref ctrl) = self.active_control else {
            return Ok(());
        };
        ctrl.signal_seek(position_secs);
        *self.position.lock().unwrap() = position_secs;
        *self.start_time.lock().unwrap() = (position_secs > 0.0).then(Instant::now);
        *self.start_pos.lock().unwrap() = position_secs;
        Ok(())
    }

    fn effective_vol_ratio(&self, volume: u8) -> f32 {
        volume.min(100) as f32 / 100.0
    }

    pub fn set_volume(&mut self, volume: u8) -> AudioResult<()> {
        self.volume.store(volume.min(100), Ordering::SeqCst);
        self.active().set_volume(self.effective_vol_ratio(volume));
        Ok(())
    }

    pub fn volume(&self) -> u8 {
        self.volume.load(Ordering::SeqCst)
    }

    pub fn is_playing(&self) -> bool {
        self.playing.load(Ordering::SeqCst) && !self.pending_pause
    }

    pub fn track_position(&self) -> f64 {
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

    pub fn current_position(&self) -> f64 {
        self.track_position()
    }

    pub fn duration(&self) -> f64 {
        *self.duration.lock().unwrap()
    }

    pub fn active_remaining(&self) -> f64 {
        let total = *self.duration.lock().unwrap();
        if total <= 0.0 {
            return 0.0;
        }
        (total - self.current_position()).max(0.0)
    }

    pub fn set_crossfade_easing(&mut self, easing: Easing) {
        self.crossfade_easing = easing;
    }

    pub fn start_crossfade(&mut self, duration_secs: f64) {
        if self.standby().empty() {
            return;
        }
        self.standby().set_volume(0.0);
        self.standby().play();
        self.crossfade_start = Some(Instant::now());
        self.crossfade_duration = duration_secs.max(1.0);
    }

    pub fn is_crossfading(&self) -> bool {
        self.crossfade_start.is_some()
    }

    pub fn drop_active(&mut self) {
        let vol = self.volume.load(Ordering::SeqCst) as f32 / 100.0;
        let outgoing = if self.is_a_active {
            &self.player_a
        } else {
            &self.player_b
        };
        let incoming = if self.is_a_active {
            &self.player_b
        } else {
            &self.player_a
        };
        outgoing.stop();
        outgoing.set_volume(0.0);
        incoming.set_volume(vol);

        self.is_a_active = !self.is_a_active;
        if self.standby_duration > 0.0 {
            *self.duration.lock().unwrap() = self.standby_duration;
        }
        *self.start_time.lock().unwrap() = Some(Instant::now());
        *self.start_pos.lock().unwrap() = 0.0;
        self.crossfade_start = None;
        self.playing.store(true, Ordering::SeqCst);
        self.underrun_since = None;

        std::mem::swap(&mut self.active_control, &mut self.standby_control);
        std::mem::swap(
            &mut self.active_decode_handle,
            &mut self.standby_decode_handle,
        );
        let new_standby = if self.is_a_active {
            &self.player_b
        } else {
            &self.player_a
        };
        new_standby.set_volume(0.0);
        new_standby.pause();
        Self::stop_decode_thread(&self.standby_control, &mut self.standby_decode_handle);
    }

    pub fn force_complete_crossfade(&mut self) {
        if self.crossfade_start.is_none() {
            return;
        }
        self.crossfade_start = None;
        let vol = self.volume.load(Ordering::SeqCst) as f32 / 100.0;
        if self.is_a_active {
            self.player_a.set_volume(0.0);
            self.player_a.stop();
            self.player_b.set_volume(vol);
        } else {
            self.player_b.set_volume(0.0);
            self.player_b.stop();
            self.player_a.set_volume(vol);
        }
        self.is_a_active = !self.is_a_active;
        if self.standby_duration > 0.0 {
            *self.duration.lock().unwrap() = self.standby_duration;
        }
        *self.start_time.lock().unwrap() = Some(Instant::now());
        *self.start_pos.lock().unwrap() = 0.0;
        self.playing.store(true, Ordering::SeqCst);

        let new_standby = if self.is_a_active {
            &self.player_b
        } else {
            &self.player_a
        };
        new_standby.stop();
        new_standby.set_volume(0.0);
        new_standby.pause();
    }

    fn ease_in(t: f64, easing: Easing) -> f64 {
        match easing {
            Easing::Linear => t,
            Easing::SlowFadeInFastFadeOut => t * t,
            Easing::FastFadeInSlowFadeOut => t.sqrt(),
            Easing::Logarithmic => 2.0f64.powf(t) - 1.0,
            Easing::Smoothstep => t * t * (3.0 - 2.0 * t),
            Easing::EqualPower => (t * std::f64::consts::FRAC_PI_2).sin(),
            Easing::Exponential => t.powf(3.0),
        }
    }

    fn ease_out(t: f64, easing: Easing) -> f64 {
        match easing {
            Easing::Linear => 1.0 - t,
            Easing::SlowFadeInFastFadeOut => 1.0 - t * t,
            Easing::FastFadeInSlowFadeOut => (1.0 - t) * (1.0 - t),
            Easing::Logarithmic => 2.0f64.powf(1.0 - t) - 1.0,
            Easing::Smoothstep => 1.0 - (t * t * (3.0 - 2.0 * t)),
            Easing::EqualPower => (t * std::f64::consts::FRAC_PI_2).cos(),
            Easing::Exponential => (1.0 - t).powf(3.0),
        }
    }

    fn step_crossfade(&mut self) -> bool {
        let start = match self.crossfade_start {
            Some(s) => s,
            None => return false,
        };
        let elapsed = start.elapsed().as_secs_f64();
        let progress = (elapsed / self.crossfade_duration).min(1.0);
        let eased_out = Self::ease_out(progress, self.crossfade_easing);
        let eased_in = Self::ease_in(progress, self.crossfade_easing);
        let vol = self.volume.load(Ordering::SeqCst) as f64 / 100.0;
        let base = vol.min(1.0);

        self.player_a.set_volume(if self.is_a_active {
            eased_out * base
        } else {
            eased_in * base
        } as f32);
        self.player_b.set_volume(if self.is_a_active {
            eased_in * base
        } else {
            eased_out * base
        } as f32);

        if progress >= 1.0 {
            let cut_old = self.active_remaining() <= 0.05;

            self.is_a_active = !self.is_a_active;
            self.crossfade_start = None;
            if self.standby_duration > 0.0 {
                *self.duration.lock().unwrap() = self.standby_duration;
            }
            *self.start_time.lock().unwrap() = Some(Instant::now());
            *self.start_pos.lock().unwrap() = 0.0;
            self.playing.store(true, Ordering::SeqCst);

            let new_standby = if self.is_a_active {
                &self.player_b
            } else {
                &self.player_a
            };
            if cut_old {
                new_standby.stop();
                new_standby.set_volume(0.0);
                new_standby.pause();
            } else {
                new_standby.set_volume(0.0);
            }
            return true;
        }
        false
    }

    pub fn poll(&mut self) -> AudioResult<Option<AudioEvent>> {
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
                self.active().pause();
                let paused_pos = self.track_position();
                *self.position.lock().unwrap() = paused_pos;
                *self.start_pos.lock().unwrap() = paused_pos;
                *self.start_time.lock().unwrap() = None;
                self.playing.store(false, Ordering::SeqCst);
            } else {
                let progress = elapsed / FADE_MS;
                let start = self.stored_volume.min(100) as f32 / 100.0;
                let target = start * (1.0 - progress as f32);
                self.active().set_volume(target);
            }
        }

        if self.active().empty() {
            if self.crossfade_start.is_some() {
                self.force_complete_crossfade();
                if !self.active().empty() {
                    self.underrun_since = None;
                    return Ok(None);
                }
            }
            if self.playing.load(Ordering::SeqCst) {
                let since = self.underrun_since.get_or_insert_with(Instant::now);
                if since.elapsed() < UNDERFLOW_GRACE {
                    return Ok(None);
                }
                self.underrun_since = None;
                self.playing.store(false, Ordering::SeqCst);
                return Ok(Some(AudioEvent::Finished));
            }
            self.underrun_since = None;
            return Ok(None);
        }
        self.underrun_since = None;

        if !self.active().is_paused() {
            let pos = self.track_position();
            *self.position.lock().unwrap() = pos;
            if (pos - self.last_reported_pos).abs() >= 0.05 {
                self.last_reported_pos = pos;
                return Ok(Some(AudioEvent::Position(pos)));
            }
            return Ok(None);
        }

        Ok(None)
    }
}
