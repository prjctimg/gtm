// Copyright (c) 2026 - present
// Author: prjctimg <prjctimg@outlook.com>
// No-op mixer for headless/daemon mode without audio output
//
// This is free software released under the GPL-3.0 license.

use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU8, Ordering};
use std::sync::Mutex;

use rodio::Source;

use crate::backend::{AudioEvent, AudioResult};
use crate::mixer::Mixer;
use crate::stretch::speed_to_fixed;
use gtm_core::state::{Easing, EqPreset, ReverbConfig};

/// A silent no-op mixer for environments without audio hardware (CI, testing).
pub struct NullMixer {
    playing: AtomicBool,
    volume: AtomicU8,
    master_volume: AtomicU8,
    position: Mutex<f64>,
    duration: Mutex<f64>,
    crossfading: Mutex<bool>,
    standby_loaded: Mutex<bool>,
    playback_speed: AtomicU32,
}

impl Default for NullMixer {
    fn default() -> Self {
        Self::new()
    }
}

impl NullMixer {
    pub fn new() -> Self {
        Self {
            playing: AtomicBool::new(false),
            volume: AtomicU8::new(100),
            master_volume: AtomicU8::new(100),
            position: Mutex::new(0.0),
            duration: Mutex::new(0.0),
            crossfading: Mutex::new(false),
            standby_loaded: Mutex::new(false),
            playback_speed: AtomicU32::new(1000),
        }
    }
}

impl Mixer for NullMixer {
    fn load_active(&mut self, _path: &str, start_pos: f64) -> AudioResult<()> {
        *self.position.lock().unwrap() = start_pos;
        self.playing.store(false, Ordering::SeqCst);
        Ok(())
    }

    fn load_active_decoded(
        &mut self,
        _source: Box<dyn Source<Item = f32> + Send>,
        start_pos: f64,
    ) -> AudioResult<()> {
        *self.position.lock().unwrap() = start_pos;
        self.playing.store(false, Ordering::SeqCst);
        Ok(())
    }

    fn load_standby(&mut self, _path: &str) -> AudioResult<()> {
        *self.standby_loaded.lock().unwrap() = true;
        Ok(())
    }

    fn load_standby_decoded(
        &mut self,
        _source: Box<dyn Source<Item = f32> + Send>,
    ) -> AudioResult<()> {
        *self.standby_loaded.lock().unwrap() = true;
        Ok(())
    }

    fn standby_is_loaded(&self) -> bool {
        *self.standby_loaded.lock().unwrap()
    }

    fn play(&mut self) -> AudioResult<()> {
        self.playing.store(true, Ordering::SeqCst);
        Ok(())
    }

    fn pause(&mut self) -> AudioResult<()> {
        self.playing.store(false, Ordering::SeqCst);
        Ok(())
    }

    fn stop(&mut self) -> AudioResult<()> {
        self.playing.store(false, Ordering::SeqCst);
        *self.position.lock().unwrap() = 0.0;
        *self.crossfading.lock().unwrap() = false;
        *self.standby_loaded.lock().unwrap() = false;
        Ok(())
    }

    fn seek(&mut self, position_secs: f64) -> AudioResult<()> {
        *self.position.lock().unwrap() = position_secs;
        Ok(())
    }

    fn set_volume(&mut self, volume: u8) -> AudioResult<()> {
        self.volume.store(volume.min(100), Ordering::SeqCst);
        Ok(())
    }

    fn volume(&self) -> u8 {
        self.volume.load(Ordering::SeqCst)
    }

    fn set_master_volume(&mut self, volume: u8) -> AudioResult<()> {
        self.master_volume.store(volume.min(100), Ordering::SeqCst);
        Ok(())
    }

    fn master_volume(&self) -> u8 {
        self.master_volume.load(Ordering::SeqCst)
    }

    fn is_playing(&self) -> bool {
        self.playing.load(Ordering::SeqCst)
    }

    fn current_position(&self) -> f64 {
        *self.position.lock().unwrap()
    }

    fn duration(&self) -> f64 {
        *self.duration.lock().unwrap()
    }

    fn active_remaining(&self) -> f64 {
        0.0
    }

    fn start_crossfade(&mut self, _duration_secs: f64) {
        *self.crossfading.lock().unwrap() = true;
    }

    fn set_crossfade_easing(&mut self, _easing: Easing) {}

    fn is_crossfading(&self) -> bool {
        *self.crossfading.lock().unwrap()
    }

    fn force_complete_crossfade(&mut self) {
        *self.crossfading.lock().unwrap() = false;
    }

    fn drop_active(&mut self) {
        *self.crossfading.lock().unwrap() = false;
        *self.standby_loaded.lock().unwrap() = false;
    }

    fn poll(&mut self) -> AudioResult<Option<AudioEvent>> {
        Ok(None)
    }

    fn set_eq_preset(&self, _preset: &EqPreset) {}
    fn set_eq_enabled(&self, _enabled: bool) {}
    fn set_reverb(&self, _config: &ReverbConfig) {}

    fn set_playback_speed(&self, speed: f64) {
        self.playback_speed
            .store(speed_to_fixed(speed), Ordering::Relaxed);
    }
}
