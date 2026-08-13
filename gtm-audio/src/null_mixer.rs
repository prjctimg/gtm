use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::Mutex;

use rodio::Source;

use crate::backend::{AudioEvent, AudioResult};
use crate::mixer::Mixer;
use gtm_core::state::Easing;

/// A silent no-op mixer for environments without audio hardware (CI, testing).
pub struct NullMixer {
    playing: AtomicBool,
    volume: AtomicU8,
    position: Mutex<f64>,
    duration: Mutex<f64>,
    crossfading: Mutex<bool>,
    standby_loaded: Mutex<bool>,
}

impl NullMixer {
    pub fn new() -> Self {
        Self {
            playing: AtomicBool::new(false),
            volume: AtomicU8::new(100),
            position: Mutex::new(0.0),
            duration: Mutex::new(0.0),
            crossfading: Mutex::new(false),
            standby_loaded: Mutex::new(false),
        }
    }
}

impl Mixer for NullMixer {
    fn load_active(&mut self, _path: &str, start_pos: f64) -> AudioResult<()> {
        *self.position.lock().unwrap() = start_pos;
        self.playing.store(false, Ordering::SeqCst);
        Ok(())
    }

    fn load_active_decoded(&mut self, _source: Box<dyn Source<Item = f32> + Send>, start_pos: f64) -> AudioResult<()> {
        *self.position.lock().unwrap() = start_pos;
        self.playing.store(false, Ordering::SeqCst);
        Ok(())
    }

    fn load_standby(&mut self, _path: &str) -> AudioResult<()> {
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

    fn poll(&mut self) -> AudioResult<Option<AudioEvent>> {
        Ok(None)
    }
}
