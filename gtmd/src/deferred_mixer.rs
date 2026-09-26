// Copyright (c) 2026
// Author: prjctimg <prjctimg@outlook.com>
// Deferred (lazy) audio mixer: keeps daemon startup off the audio-device
// critical path.
//
// `Daemon::new` used to call `init_mixer` eagerly, which opens the PulseAudio
// connection, enumerates devices, and on Termux may spawn the PulseAudio
// server — all synchronously *before* the IPC socket binds. `DeferredMixer`
// implements the same `Mixer` trait but runs the factory closure on the first
// real mixer call instead, so the socket is bound and serving before any
// audio-device or network I/O happens. Initialization is retried on every
// call until it succeeds; once it succeeds the inner mixer is cached.
//
// This is free software released under the GPL-3.0 license.

use std::sync::OnceLock;

use rodio::Source;

use gtm::audio::{AudioEvent, AudioResult, Mixer};
use gtm::shared::MAX_VOLUME;
use gtm::shared::global::{EqPreset, ReverbConfig};

/// A `Mixer` that builds the real backend mixer on first use.
///
/// The factory typically performs the PulseAudio connect / rodio device open
/// plus replaying persisted device, speed, and mono settings. Cheap to
/// construct; all ~30 call sites keep working unchanged because they already
/// go through the `Mixer` trait behind the daemon's tokio mutex.
pub struct DeferredMixer {
    factory: Box<dyn Fn() -> AudioResult<Box<dyn Mixer>> + Send + Sync>,
    inner: OnceLock<Box<dyn Mixer>>,
}

impl DeferredMixer {
    pub fn new(factory: impl Fn() -> AudioResult<Box<dyn Mixer>> + Send + Sync + 'static) -> Self {
        Self {
            factory: Box::new(factory),
            inner: OnceLock::new(),
        }
    }

    fn ensure_ref(&self) -> AudioResult<&dyn Mixer> {
        if let Some(m) = self.inner.get() {
            return Ok(&**m);
        }
        match (self.factory)() {
            Ok(b) => {
                // A concurrent caller may have won the race; either way the
                // cell now holds a mixer.
                let _ = self.inner.set(b);
                self.inner.get().map(|b| &**b).ok_or_else(|| {
                    gtm::audio::AudioError::OutputError(
                        "deferred mixer initialised but missing".into(),
                    )
                })
            }
            Err(e) => Err(e),
        }
    }

    fn ensure_mut(&mut self) -> AudioResult<&mut Box<dyn Mixer>> {
        if self.inner.get().is_none() {
            let b = (self.factory)()?;
            let _ = self.inner.set(b);
        }
        self.inner.get_mut().ok_or_else(|| {
            gtm::audio::AudioError::OutputError("deferred mixer initialised but missing".into())
        })
    }
}

impl Mixer for DeferredMixer {
    fn load_active(&mut self, path: &str, start_pos: f64) -> AudioResult<()> {
        self.ensure_mut()?.load_active(path, start_pos)
    }

    fn load_active_decoded(
        &mut self,
        source: Box<dyn Source<Item = f32> + Send>,
        start_pos: f64,
    ) -> AudioResult<()> {
        self.ensure_mut()?.load_active_decoded(source, start_pos)
    }

    fn load_active_reader(
        &mut self,
        reader: Box<dyn std::io::Read + Send>,
        start_pos: f64,
    ) -> AudioResult<()> {
        self.ensure_mut()?.load_active_reader(reader, start_pos)
    }

    fn load_active_stream(
        &mut self,
        source: Box<dyn Source<Item = f32> + Send>,
        start_pos: f64,
        duration_secs: f64,
    ) -> AudioResult<()> {
        self.ensure_mut()?
            .load_active_stream(source, start_pos, duration_secs)
    }

    fn load_standby(&mut self, path: &str) -> AudioResult<()> {
        self.ensure_mut()?.load_standby(path)
    }

    fn load_standby_decoded(
        &mut self,
        source: Box<dyn Source<Item = f32> + Send>,
    ) -> AudioResult<()> {
        self.ensure_mut()?.load_standby_decoded(source)
    }

    fn standby_is_loaded(&self) -> bool {
        self.ensure_ref()
            .map(|m| m.standby_is_loaded())
            .unwrap_or(false)
    }

    fn play(&mut self) -> AudioResult<()> {
        self.ensure_mut()?.play()
    }

    fn pause(&mut self) -> AudioResult<()> {
        self.ensure_mut()?.pause()
    }

    fn stop(&mut self) -> AudioResult<()> {
        self.ensure_mut()?.stop()
    }

    fn seek(&mut self, position_secs: f64) -> AudioResult<()> {
        self.ensure_mut()?.seek(position_secs)
    }

    fn set_volume(&mut self, volume: u8) -> AudioResult<()> {
        self.ensure_mut()?.set_volume(volume)
    }

    fn volume(&self) -> u8 {
        self.ensure_ref().map(|m| m.volume()).unwrap_or(MAX_VOLUME)
    }

    fn is_playing(&self) -> bool {
        self.ensure_ref().map(|m| m.is_playing()).unwrap_or(false)
    }

    fn current_position(&self) -> f64 {
        self.ensure_ref()
            .map(|m| m.current_position())
            .unwrap_or(0.0)
    }

    fn duration(&self) -> f64 {
        self.ensure_ref().map(|m| m.duration()).unwrap_or(0.0)
    }

    fn active_remaining(&self) -> f64 {
        self.ensure_ref()
            .map(|m| m.active_remaining())
            .unwrap_or(0.0)
    }

    fn start_crossfade(&mut self, duration_secs: f64) {
        if let Ok(m) = self.ensure_mut() {
            m.start_crossfade(duration_secs);
        }
    }

    fn is_crossfading(&self) -> bool {
        self.ensure_ref()
            .map(|m| m.is_crossfading())
            .unwrap_or(false)
    }

    fn force_complete_crossfade(&mut self) {
        if let Ok(m) = self.ensure_mut() {
            m.force_complete_crossfade();
        }
    }

    fn drop_active(&mut self) {
        if let Ok(m) = self.ensure_mut() {
            m.drop_active();
        }
    }

    fn poll(&mut self) -> AudioResult<Option<AudioEvent>> {
        self.ensure_mut()?.poll()
    }

    fn current_peak_level(&self) -> f32 {
        self.ensure_ref()
            .map(|m| m.current_peak_level())
            .unwrap_or(0.0)
    }

    fn current_spectrum(&self) -> Vec<f32> {
        self.ensure_ref()
            .map(|m| m.current_spectrum())
            .unwrap_or_default()
    }

    fn current_waveform(&self) -> (Vec<f32>, bool) {
        self.ensure_ref()
            .map(|m| m.current_waveform())
            .unwrap_or_default()
    }

    fn set_eq_preset(&self, preset: &EqPreset) {
        if let Ok(m) = self.ensure_ref() {
            m.set_eq_preset(preset);
        }
    }

    fn set_eq_enabled(&self, enabled: bool) {
        if let Ok(m) = self.ensure_ref() {
            m.set_eq_enabled(enabled);
        }
    }

    fn set_reverb(&self, config: &ReverbConfig) {
        if let Ok(m) = self.ensure_ref() {
            m.set_reverb(config);
        }
    }

    fn set_speed(&self, rate: f32) {
        if let Ok(m) = self.ensure_ref() {
            m.set_speed(rate);
        }
    }

    fn speed(&self) -> f32 {
        self.ensure_ref().map(|m| m.speed()).unwrap_or(1.0)
    }

    fn list_devices(&self) -> Vec<String> {
        self.ensure_ref()
            .map(|m| m.list_devices())
            .unwrap_or_default()
    }

    fn set_device(&mut self, name: Option<String>) -> AudioResult<()> {
        self.ensure_mut()?.set_device(name)
    }

    fn set_mono(&self, enabled: bool) {
        if let Ok(m) = self.ensure_ref() {
            m.set_mono(enabled);
        }
    }

    fn mono(&self) -> bool {
        self.ensure_ref().map(|m| m.mono()).unwrap_or(false)
    }
}
