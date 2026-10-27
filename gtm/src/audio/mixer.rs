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

use rodio::cpal::traits::{DeviceTrait, HostTrait};
use rodio::{Decoder, DeviceSinkBuilder, Player, Source};

use crate::audio::backend::{AudioError, AudioEvent, AudioResult};
use crate::audio::buffer::{
    BUFFER_CAPACITY_SAMPLES, DecodeControl, PREBUFFER_SAMPLES, PREBUFFER_SAMPLES_REDUCED,
    RingBufferInner, RingBufferSource,
};
use crate::audio::decoder::DecodeThread;
use crate::audio::eq::{EqGains, EqSource, PreGain, ReverbSource};
use crate::audio::mono::MonoSource;
use crate::audio::symphonia::{StreamingReopen, SymphoniaSource};
use crate::audio::wave::{WAVEFORM_FRESHNESS, WaveformShared};
use crate::shared::global::{EqPreset, ReverbConfig};
use crate::shared::{MAX_VOLUME, volume_ratio};

/// How long a provider-decoded source is given to prime its ring before the
/// play proceeds anyway. Generous, because the cost of waiting is a late start
/// while the cost of failing is a track that will not play at all. Shared with
/// the PulseAudio backend so both drain paths behave identically.
pub(crate) const STREAM_PREBUFFER_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(20);

pub trait Mixer: Send + Sync {
    /// Open, decode and prebuffer `path`, then make it active. The outgoing
    /// source must keep playing until the replacement has real audio ready.
    fn load_active(&mut self, path: &str, start_pos: f64) -> AudioResult<()>;
    /// Load an already-opened decoded source, prebuffering it before the
    /// outgoing source is stopped.
    fn load_active_decoded(
        &mut self,
        source: Box<dyn Source<Item = f32> + Send>,
        start_pos: f64,
    ) -> AudioResult<()>;
    /// Load a provider-decoded sample source (Spotify via librespot) as the
    /// active source. A decode thread owns draining it into the ring buffer,
    /// so a source that blocks waiting on the network can never stall the
    /// output callback. This waits for the ring to prime before returning and
    /// keeps the real track duration so seeking and position reporting work.
    fn load_active_stream(
        &mut self,
        source: Box<dyn Source<Item = f32> + Send>,
        start_pos: f64,
        duration_secs: f64,
    ) -> AudioResult<()>;
    /// Load a remote byte transport (radio, podcast, HTTP stream) as the active
    /// source.
    ///
    /// The decode thread owns reading, EQ, reverb and ring-buffer feeding, so
    /// network jitter can never stall the audio callback, and the call returns
    /// only once the ring holds enough audio that playback starts on a full
    /// buffer rather than draining one. `reopen` is how a seek gets back to the
    /// start of a transport that cannot seek on its own.
    fn load_active_reader(
        &mut self,
        reader: Box<dyn std::io::Read + Send>,
        reopen: Option<Box<dyn StreamingReopen>>,
        start_pos: f64,
    ) -> AudioResult<()>;
    /// Samples the decode thread had to discard because the consumer stalled.
    fn dropped_samples(&self) -> u64;
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
    fn is_crossfading(&self) -> bool;
    fn force_complete_crossfade(&mut self);
    /// Abandon an in-flight crossfade without changing tracks.
    ///
    /// The outgoing source stays active and returns to the user's volume; the
    /// incoming standby is silenced and discarded. Used when a new selected
    /// track arrives while an automatic transition is already under way.
    fn cancel_crossfade(&mut self);
    fn drop_active(&mut self);
    fn poll(&mut self) -> AudioResult<Option<AudioEvent>>;
    fn current_peak_level(&self) -> f32;
    fn current_spectrum(&self) -> Vec<f32>;

    /// Latest time-domain waveform ring plus whether the source is stereo.
    /// Returns `(samples, stereo)`; empty when nothing fresh has been
    /// captured (paused/stopped) so visualizers decay to rest.
    fn current_waveform(&self) -> (Vec<f32>, bool) {
        (Vec::new(), false)
    }

    // ─── EQ / Reverb ───
    fn set_eq_preset(&self, preset: &EqPreset);
    fn set_eq_enabled(&self, enabled: bool);
    /// Gain in dB applied to every sample before the EQ and reverb.
    fn set_pre_gain(&self, db: f32);
    fn set_reverb(&self, config: &ReverbConfig);

    // ─── Audio device switching ───
    /// List available output device names. Empty when the backend can't
    /// enumerate devices (e.g. the PulseAudio network backend).
    fn list_devices(&self) -> Vec<String> {
        Vec::new()
    }
    /// Switch the active output device; `None` reselects the system default.
    /// Restarts the output, dropping any buffered playback. Backends that
    /// can't switch report an error.
    fn set_device(&mut self, _name: Option<String>) -> AudioResult<()> {
        Err(AudioError::OutputError(
            "device switching not supported on this backend".into(),
        ))
    }

    // ─── Mono downmix ───
    /// Force mono playback by summing every channel into one shared mix.
    /// Applies live to sources already attached to the output.
    fn set_mono(&self, _enabled: bool) {}
    /// Whether mono downmix is currently active.
    fn mono(&self) -> bool {
        false
    }
}

pub struct AudioMixer {
    // Kept alive for the mixer's lifetime: the underlying device must stay
    // open or ALSA/pulse would tear the output sinks down.
    _keepalive_sink: Arc<MixerDeviceSink>,
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
    // ─── EQ / Reverb ───
    pub eq_gains: EqGains,
    eq_enabled: Arc<AtomicBool>,
    pre_gain: PreGain,
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
    // ─── Waveform (Wave/Stereo visualizer modes) ───
    wave: WaveformShared,
    // ─── Mono downmix ───
    mono: Arc<AtomicBool>,
    // Track if this is the first track (for prebuffer optimization)
    first_track: bool,
}

const UNDERFLOW_GRACE: Duration = Duration::from_millis(30);

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
    fn load_active_reader(
        &mut self,
        reader: Box<dyn std::io::Read + Send>,
        reopen: Option<Box<dyn StreamingReopen>>,
        start_pos: f64,
    ) -> AudioResult<()> {
        self.load_active_reader(reader, reopen, start_pos)
    }
    fn dropped_samples(&self) -> u64 {
        self.dropped_samples()
    }
    fn load_active_stream(
        &mut self,
        source: Box<dyn Source<Item = f32> + Send>,
        start_pos: f64,
        duration_secs: f64,
    ) -> AudioResult<()> {
        self.load_active_stream(source, start_pos, duration_secs)
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
    fn is_crossfading(&self) -> bool {
        self.is_crossfading()
    }
    fn force_complete_crossfade(&mut self) {
        self.force_complete_crossfade()
    }
    fn cancel_crossfade(&mut self) {
        self.cancel_crossfade()
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

    fn set_mono(&self, enabled: bool) {
        self.mono.store(enabled, Ordering::Relaxed);
    }

    fn mono(&self) -> bool {
        self.mono.load(Ordering::Relaxed)
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

    fn list_devices(&self) -> Vec<String> {
        rodio::cpal::default_host()
            .output_devices()
            .map(|devs| {
                devs.filter_map(|d| d.id().map(|id| id.to_string()).ok())
                    .collect()
            })
            .unwrap_or_default()
    }

    fn set_device(&mut self, name: Option<String>) -> AudioResult<()> {
        let volume = self.volume.load(Ordering::SeqCst);
        let eq_enabled = self.eq_enabled.load(Ordering::Relaxed);
        let eq_gains = self.eq_gains.clone();
        let pre_gain = self.pre_gain.clone();
        let reverb_enabled = self.reverb_enabled.load(Ordering::Relaxed);
        let reverb_room = *self.reverb_room_size.lock().unwrap();
        let mono = self.mono.load(Ordering::Relaxed);

        Self::stop_decode_thread(&self.active_control, &mut self.active_decode_handle);
        Self::stop_decode_thread(&self.standby_control, &mut self.standby_decode_handle);
        self.player_a.stop();
        self.player_b.stop();

        let mut fresh = Self::open_for_device(name)?;
        let _ = fresh.set_volume(volume);
        fresh.eq_enabled.store(eq_enabled, Ordering::Relaxed);
        fresh.eq_gains = eq_gains;
        fresh.pre_gain = pre_gain;
        fresh
            .reverb_enabled
            .store(reverb_enabled, Ordering::Relaxed);
        *fresh.reverb_room_size.lock().unwrap() = reverb_room;
        fresh.mono.store(mono, Ordering::Relaxed);
        *self = fresh;
        Ok(())
    }

    fn current_peak_level(&self) -> f32 {
        if !self.playing.load(Ordering::SeqCst) {
            return 0.0;
        }

        self.volume.load(Ordering::SeqCst) as f32 / MAX_VOLUME as f32
    }
    fn current_spectrum(&self) -> Vec<f32> {
        self.spectrum.lock().unwrap().clone()
    }
    fn current_waveform(&self) -> (Vec<f32>, bool) {
        self.wave.snapshot(WAVEFORM_FRESHNESS)
    }
}

impl AudioMixer {
    pub fn new() -> AudioResult<Self> {
        Self::open_for_device(None)
    }

    /// Open the output on `name` (`None` = system default device) and wire up
    /// both crossfade players.
    fn open_for_device(name: Option<String>) -> AudioResult<Self> {
        let mut sink = if let Some(dev_name) = name {
            let devices = rodio::cpal::default_host()
                .output_devices()
                .map_err(|e| AudioError::OutputError(e.to_string()))?;
            let device = devices
                .filter_map(|d| d.id().map(|id| (id.to_string(), d)).ok())
                .find(|(name, _)| *name == dev_name)
                .map(|(_, d)| d)
                .ok_or_else(|| {
                    AudioError::OutputError(format!("no output device named '{dev_name}'"))
                })?;
            DeviceSinkBuilder::from_device(device)
                .map_err(|e| AudioError::OutputError(e.to_string()))?
                .open_stream()
                .map_err(|e| AudioError::OutputError(e.to_string()))?
        } else {
            DeviceSinkBuilder::open_default_sink()
                .map_err(|e| AudioError::OutputError(e.to_string()))?
        };
        sink.log_on_drop(false);
        let sink = Arc::new(MixerDeviceSink(sink));
        let mixer = sink.0.mixer();
        let a = Player::connect_new(mixer);
        let b = Player::connect_new(mixer);

        b.set_volume(0.0);
        b.pause();

        Ok(Self {
            _keepalive_sink: sink,
            player_a: a,
            player_b: b,
            is_a_active: true,
            position: Arc::new(Mutex::new(0.0)),
            duration: Arc::new(Mutex::new(0.0)),
            playing: Arc::new(AtomicBool::new(false)),
            volume: Arc::new(AtomicU8::new(MAX_VOLUME)),
            start_time: Arc::new(Mutex::new(None)),
            start_pos: Arc::new(Mutex::new(0.0)),
            crossfade_start: None,
            crossfade_duration: 0.0,
            standby_duration: 0.0,
            pending_pause: false,
            pause_fade_start: None,
            stored_volume: MAX_VOLUME,
            last_reported_pos: f64::NEG_INFINITY,
            eq_gains: EqGains::new_flat(),
            eq_enabled: Arc::new(AtomicBool::new(true)),
            pre_gain: PreGain::default(),
            reverb_enabled: Arc::new(AtomicBool::new(false)),
            reverb_room_size: Arc::new(Mutex::new(0.3)),
            active_control: None,
            active_decode_handle: None,
            standby_control: None,
            standby_decode_handle: None,
            underrun_since: None,
            spectrum: Arc::new(Mutex::new(Vec::new())),
            wave: WaveformShared::default(),
            mono: Arc::new(AtomicBool::new(false)),
            first_track: true,
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

    /// Build a symphonia source over an arbitrary byte stream (e.g. a remote
    /// HTTP stream). `start_pos` skips leading samples; `reopen` enables seek
    /// by reconnecting, `None` disables seeking (live transports).
    pub fn decode_reader(
        reader: Box<dyn std::io::Read + Send + 'static>,
        reopen: Option<Box<dyn StreamingReopen>>,
        start_pos: f64,
    ) -> AudioResult<Box<dyn Source<Item = f32> + Send>> {
        SymphoniaSource::from_reader(reader, reopen, start_pos)
            .map(|s| Box::new(s) as Box<dyn Source<Item = f32> + Send>)
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

    /// Apply live mono downmix as the outermost stage (closest to the output)
    /// so it runs after EQ/reverb regardless of the source path. The shared
    /// flag means toggling mono re-evaluates on the very next sample.
    fn apply_mono(
        &self,
        source: Box<dyn Source<Item = f32> + Send>,
    ) -> Box<dyn Source<Item = f32> + Send> {
        Box::new(MonoSource::new(source, self.mono.clone()))
    }

    #[allow(clippy::too_many_arguments)]
    fn start_decode_thread(
        path: &str,
        eq_gains: &EqGains,
        eq_enabled: &Arc<AtomicBool>,
        pre_gain: &PreGain,
        reverb_enabled: &Arc<AtomicBool>,
        reverb_room_size: &Arc<Mutex<f32>>,
        spectrum: &Arc<Mutex<Vec<f32>>>,
        wave: &WaveformShared,
        prebuffer_samples: usize,
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
            pre_gain.clone(),
            reverb_enabled.clone(),
            reverb_room_size.clone(),
            spectrum.clone(),
            wave.clone(),
            prebuffer_samples,
        );
        let handle = thread.spawn().map_err(AudioError::DecodeError)?;

        let start = Instant::now();
        let timeout = STREAM_PREBUFFER_TIMEOUT;
        while !control.ready.load(Ordering::Acquire) && start.elapsed() < timeout {
            if !control.running.load(Ordering::Acquire) {
                return Err(AudioError::DecodeError(
                    "decode thread exited before prebuffer".into(),
                ));
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        if !control.ready.load(Ordering::Acquire) {
            return Err(AudioError::DecodeError(format!(
                "decode thread for {} did not become ready in time",
                path,
            )));
        }

        let source = RingBufferSource::new(shared, control.clone());
        Ok((control, source, handle))
    }

    pub fn load_active(&mut self, path: &str, start_pos: f64) -> AudioResult<()> {
        let prebuffer = if self.first_track {
            PREBUFFER_SAMPLES
        } else {
            PREBUFFER_SAMPLES_REDUCED
        };
        self.first_track = false;

        // Open, probe and prime the incoming source before touching the
        // outgoing one. The old track keeps sounding through this whole step,
        // so a slow disk or header probe can no longer punch a silent gap into
        // a user-selected track change.
        let dur = Self::probe_duration(path)?;
        let (control, source, handle) = Self::start_decode_thread(
            path,
            &self.eq_gains,
            &self.eq_enabled,
            &self.pre_gain,
            &self.reverb_enabled,
            &self.reverb_room_size,
            &self.spectrum,
            &self.wave,
            prebuffer,
        )?;

        self.cancel_crossfade();
        Self::stop_decode_thread(&self.active_control, &mut self.active_decode_handle);

        let vol = volume_ratio(self.volume.load(Ordering::SeqCst));
        self.active().stop();
        self.active().set_volume(vol);

        if dur > 0.0 {
            *self.duration.lock().unwrap() = dur;
        }

        self.active().append(self.apply_mono(Box::new(source)));

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
        // A decoded source is still only decoded lazily. Drain it into a ring
        // and wait for real samples before stopping the outgoing player, so a
        // slow disk or an expensive EQ chain cannot create a silent gap.
        let duration = source
            .total_duration()
            .map(|d| d.as_secs_f64())
            .unwrap_or(0.0);
        self.load_active_stream(source, start_pos, duration)
    }

    /// Launch a decode thread that reads samples from a live byte transport
    /// (radio/HTTP) into the ring buffer. The ring is backed by
    /// [`BUFFER_CAPACITY_SAMPLES`] (6 s) so network stalls of up to that
    /// duration do not interrupt playback. The 0.5 s pre-buffer keeps the
    /// audio callback from signalling a false underflow on start.
    #[allow(clippy::too_many_arguments)]
    fn start_decode_reader(
        reader: Box<dyn std::io::Read + Send>,
        reopen: Option<Box<dyn StreamingReopen>>,
        eq_gains: &EqGains,
        eq_enabled: &Arc<AtomicBool>,
        pre_gain: &PreGain,
        reverb_enabled: &Arc<AtomicBool>,
        reverb_room_size: &Arc<Mutex<f32>>,
        spectrum: &Arc<Mutex<Vec<f32>>>,
        wave: &WaveformShared,
    ) -> AudioResult<(
        Arc<DecodeControl>,
        RingBufferSource,
        std::thread::JoinHandle<()>,
    )> {
        let control = Arc::new(DecodeControl::new());
        let shared = Arc::new(RingBufferInner::new(BUFFER_CAPACITY_SAMPLES));

        let thread = DecodeThread::new_reader(
            reader,
            reopen,
            shared.clone(),
            control.clone(),
            eq_gains.clone(),
            eq_enabled.clone(),
            pre_gain.clone(),
            reverb_enabled.clone(),
            reverb_room_size.clone(),
            spectrum.clone(),
            wave.clone(),
            PREBUFFER_SAMPLES,
        );
        let handle = thread.spawn().map_err(AudioError::DecodeError)?;

        // For live readers the decode thread sets `ready` on probe success
        // (ring has initial data). A pre-existing reader error (e.g., HTTP
        // probe) skips `ready` entirely and signals `finished`, so the wait
        // loop below exits immediately via the `finished` fast-path instead
        // of waiting for a timeout.
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

        let source = RingBufferSource::new(shared, control.clone());
        Ok((control, source, handle))
    }

    /// Load a live byte transport (radio/HTTP stream) as the active source.
    /// Duration is set to zero so the `Finished` stall-guard (`total > 0.0 &&
    /// pos < total - 0.5`) is always bypassed — a genuine EOF drains the ring
    /// and fires `Finished` through the normal `Player::empty()` path instead.
    pub fn load_active_reader(
        &mut self,
        reader: Box<dyn std::io::Read + Send>,
        reopen: Option<Box<dyn StreamingReopen>>,
        start_pos: f64,
    ) -> AudioResult<()> {
        // The reader is opened and primed into its own ring while the outgoing
        // source is still playing. Only once the new transport has real audio
        // buffered do we stop the old player and hand the ring over.
        let (control, source, handle) = Self::start_decode_reader(
            reader,
            reopen,
            &self.eq_gains,
            &self.eq_enabled,
            &self.pre_gain,
            &self.reverb_enabled,
            &self.reverb_room_size,
            &self.spectrum,
            &self.wave,
        )?;

        self.cancel_crossfade();
        Self::stop_decode_thread(&self.active_control, &mut self.active_decode_handle);

        let vol = volume_ratio(self.volume.load(Ordering::SeqCst));
        self.active().stop();
        self.active().set_volume(vol);

        // Live streams have no meaningful total duration; zero both so the
        // Finished stall-guard is permanently disabled for this source.
        *self.duration.lock().unwrap() = 0.0;

        self.active().append(self.apply_mono(Box::new(source)));

        self.active_control = Some(control);
        self.active_decode_handle = Some(handle);

        *self.position.lock().unwrap() = 0.0;
        *self.start_time.lock().unwrap() = None;
        *self.start_pos.lock().unwrap() = start_pos;
        self.playing.store(false, Ordering::SeqCst);
        self.crossfade_start = None;

        Ok(())
    }

    /// Samples the decode thread had to discard because the consumer stalled.
    ///
    /// A nonzero value on a healthy machine means the output device stopped
    /// draining, which is otherwise invisible: playback just sounds wrong.
    pub fn dropped_samples(&self) -> u64 {
        self.active_control
            .as_ref()
            .map(|c| c.dropped_samples())
            .unwrap_or(0)
    }

    /// Load a provider-decoded sample source (Spotify via librespot) as the
    /// active source, and wait for it to prime before returning.
    ///
    /// The source is drained by a decode thread into the ring buffer, exactly
    /// as a local file or a live transport is. That is the whole point: a
    /// provider source blocks waiting for the network, and the output callback
    /// must never be the thread that waits. Handing such a source straight to
    /// rodio stalls the callback until the device underruns, and rodio evicts a
    /// source the moment it yields `None`, so a slow start becomes permanent
    /// silence rather than a delayed start.
    pub fn load_active_stream(
        &mut self,
        source: Box<dyn Source<Item = f32> + Send>,
        start_pos: f64,
        duration_secs: f64,
    ) -> AudioResult<()> {
        // Provider sources are drained into their own ring first. The outgoing
        // track stays active until that ring is primed, so connecting and
        // waiting for the provider cannot silence it.
        let (control, ring, handle) = Self::start_decode_stream(
            source,
            &self.eq_gains,
            &self.eq_enabled,
            &self.pre_gain,
            &self.reverb_enabled,
            &self.reverb_room_size,
            &self.spectrum,
            &self.wave,
        )?;

        self.cancel_crossfade();
        Self::stop_decode_thread(&self.active_control, &mut self.active_decode_handle);

        let vol = volume_ratio(self.volume.load(Ordering::SeqCst));
        self.active().stop();
        self.active().set_volume(vol);

        if duration_secs > 0.0 {
            *self.duration.lock().unwrap() = duration_secs;
        } else {
            *self.duration.lock().unwrap() = 0.0;
        }

        // No `wrap_source` here: EQ and reverb both run inside the decode
        // thread now, so wrapping again would apply them twice.
        self.active().append(self.apply_mono(Box::new(ring)));

        self.active_control = Some(control);
        self.active_decode_handle = Some(handle);

        *self.position.lock().unwrap() = start_pos;
        *self.start_time.lock().unwrap() = None;
        *self.start_pos.lock().unwrap() = start_pos;
        self.playing.store(false, Ordering::SeqCst);
        self.crossfade_start = None;

        Ok(())
    }

    /// Spawn the decode thread for a provider-decoded source and wait for it to
    /// prime the ring. Mirrors [`Self::start_decode_reader`].
    ///
    /// The wait is bounded but not fatal: a provider track can take a while to
    /// deliver its first audio, and failing the play outright would turn a slow
    /// start into a dead button. On timeout the thread keeps running, the ring
    /// primes whenever the samples land, and the source's own watchdog reports
    /// the silence — which is the outcome the caller asked for: wait, and say
    /// so, rather than cut the track.
    #[allow(clippy::too_many_arguments)]
    fn start_decode_stream(
        source: Box<dyn Source<Item = f32> + Send>,
        eq_gains: &EqGains,
        eq_enabled: &Arc<AtomicBool>,
        pre_gain: &PreGain,
        reverb_enabled: &Arc<AtomicBool>,
        reverb_room_size: &Arc<Mutex<f32>>,
        spectrum: &Arc<Mutex<Vec<f32>>>,
        wave: &WaveformShared,
    ) -> AudioResult<(
        Arc<DecodeControl>,
        RingBufferSource,
        std::thread::JoinHandle<()>,
    )> {
        let control = Arc::new(DecodeControl::new());
        let shared = Arc::new(RingBufferInner::new(BUFFER_CAPACITY_SAMPLES));

        let thread = DecodeThread::new_stream(
            source,
            shared.clone(),
            control.clone(),
            eq_gains.clone(),
            eq_enabled.clone(),
            pre_gain.clone(),
            reverb_enabled.clone(),
            reverb_room_size.clone(),
            spectrum.clone(),
            wave.clone(),
            PREBUFFER_SAMPLES_REDUCED,
        );
        let handle = thread.spawn().map_err(AudioError::DecodeError)?;

        // Only a source that has already failed is fatal here. Silence is not:
        // the decode thread is still allowed to deliver.
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

        let ring = RingBufferSource::new(shared, control.clone());
        Ok((control, ring, handle))
    }

    pub fn load_standby(&mut self, path: &str) -> AudioResult<()> {
        Self::stop_decode_thread(&self.standby_control, &mut self.standby_decode_handle);

        self.standby().stop();
        self.standby().set_volume(0.0);

        let (control, source, handle) = Self::start_decode_thread(
            path,
            &self.eq_gains,
            &self.eq_enabled,
            &self.pre_gain,
            &self.reverb_enabled,
            &self.reverb_room_size,
            &self.spectrum,
            &self.wave,
            PREBUFFER_SAMPLES,
        )?;

        self.standby().append(self.apply_mono(Box::new(source)));
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
        self.standby().append(self.apply_mono(source));
        self.standby_control = None;
        Ok(())
    }

    pub fn standby_is_loaded(&self) -> bool {
        !self.standby().empty()
    }

    pub fn play(&mut self) -> AudioResult<()> {
        if self.active().is_paused() {
            self.active().play();
            let vol = volume_ratio(self.volume.load(Ordering::SeqCst));
            self.active().set_volume(vol);
        }
        if self.pending_pause {
            self.pending_pause = false;
            self.pause_fade_start = None;
            let vol = volume_ratio(self.stored_volume.min(MAX_VOLUME));
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
        volume_ratio(volume.min(MAX_VOLUME))
    }

    pub fn set_volume(&mut self, volume: u8) -> AudioResult<()> {
        self.volume.store(volume.min(MAX_VOLUME), Ordering::SeqCst);
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
        let vol = volume_ratio(self.volume.load(Ordering::SeqCst));
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
        // Unconditionally, as the normal path in `start_track` does. The guard
        // that used to be here skipped the write whenever the incoming source
        // had no duration -- which is every live stream -- and left the
        // *outgoing* file's duration in place while the two lines below reset
        // the position to zero. So a stream promoted by a crossfade reported
        // position 0 against the previous song's length: a full progress bar,
        // re-filling from the start, on a track with no end to reach. A
        // stream's honest duration is 0, which is what this now writes.
        *self.duration.lock().unwrap() = self.standby_duration;
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

    pub fn cancel_crossfade(&mut self) {
        let crossfading = self.crossfade_start.take().is_some();
        if !crossfading && self.standby_control.is_none() && self.standby().empty() {
            return;
        }
        if crossfading {
            // The ramp may already have lowered the outgoing player; restore
            // the user's volume before keeping it as the active track.
            let vol = volume_ratio(self.volume.load(Ordering::SeqCst));
            self.active().set_volume(vol);
        }
        self.standby().set_volume(0.0);
        self.standby().stop();
        Self::stop_decode_thread(&self.standby_control, &mut self.standby_decode_handle);
        self.standby_duration = 0.0;
    }

    pub fn force_complete_crossfade(&mut self) {
        if self.crossfade_start.is_none() {
            return;
        }
        self.crossfade_start = None;
        let vol = volume_ratio(self.volume.load(Ordering::SeqCst));
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
        // Unconditionally, as the normal path in `start_track` does. The guard
        // that used to be here skipped the write whenever the incoming source
        // had no duration -- which is every live stream -- and left the
        // *outgoing* file's duration in place while the two lines below reset
        // the position to zero. So a stream promoted by a crossfade reported
        // position 0 against the previous song's length: a full progress bar,
        // re-filling from the start, on a track with no end to reach. A
        // stream's honest duration is 0, which is what this now writes.
        *self.duration.lock().unwrap() = self.standby_duration;
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

    fn step_crossfade(&mut self) -> bool {
        let start = match self.crossfade_start {
            Some(s) => s,
            None => return false,
        };
        let elapsed = start.elapsed().as_secs_f64();
        let progress = (elapsed / self.crossfade_duration).min(1.0);
        // Equal power, not linear. A linear ramp holds the *sum of amplitudes*
        // constant, so two uncorrelated tracks sum to 1 in the middle while
        // their power sums to 0.5 — the audible 3dB dip at the midpoint that
        // makes a crossfade sound like a hole rather than a transition.
        //
        // cos/sin instead holds the sum of *squares* at 1.0, which is what
        // "power" means here: the two tracks add in the RMS domain the ear
        // integrates, so the level through the middle stays where it was.
        //
        // Endpoints are untouched — cos(0)=1, sin(0)=0, cos(π/2)=0, sin(π/2)=1 —
        // so the fade still begins exactly from the outgoing track's level and
        // lands exactly on the incoming one's, and `force_complete_crossfade`
        // and the standby-at-zero pre-roll need no companion change.
        //
        // The cost is a ~1.5dB rise through the middle, which is what equal
        // power trades for the dip. Deliberate.
        let (eased_out, eased_in) = crossfade_gains(progress);
        let vol = volume_ratio(self.volume.load(Ordering::SeqCst)) as f64;
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
            // Unconditionally, as the normal path in `start_track` does. The guard
            // that used to be here skipped the write whenever the incoming source
            // had no duration -- which is every live stream -- and left the
            // *outgoing* file's duration in place while the two lines below reset
            // the position to zero. So a stream promoted by a crossfade reported
            // position 0 against the previous song's length: a full progress bar,
            // re-filling from the start, on a track with no end to reach. A
            // stream's honest duration is 0, which is what this now writes.
            *self.duration.lock().unwrap() = self.standby_duration;
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
                let start = volume_ratio(self.stored_volume.min(MAX_VOLUME));
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
                // A silent ring alone isn't proof the track ended: a long
                // decode stall (slow disk, format hiccup) drains the buffer
                // without hitting EOF. Only report `Finished` when the known
                // duration has actually been reached, mirroring the PulseAudio
                // backend's guard.
                let total = *self.duration.lock().unwrap();
                let pos = self.track_position();
                if total > 0.0 && pos < total - 0.5 {
                    return Ok(None);
                }
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

/// The outgoing and incoming gains at `progress` through a crossfade.
///
/// Equal power: `out² + in² == 1` for every `progress`, so the summed power of
/// two uncorrelated tracks is constant across the whole fade.
///
/// Linear gains — `1 - t` and `t` — sum to 1 in amplitude but only 0.5 in
/// power at the midpoint, which is the 3dB dip every listener hears in the
/// middle of a naive crossfade and reads as a dropout rather than a transition.
///
/// Endpoints are exact: `(1, 0)` at the start and `(0, 1)` at the end, so the
/// fade joins the steady state before it and after it without a step.
fn crossfade_gains(progress: f64) -> (f64, f64) {
    let t = progress.clamp(0.0, 1.0);
    let phase = t * std::f64::consts::FRAC_PI_2;
    (phase.cos(), phase.sin())
}

#[cfg(test)]
mod crossfade_curve_tests {
    use super::crossfade_gains;

    /// The whole point: power is held, not amplitude.
    ///
    /// Swept rather than sampled at the midpoint alone, because the property
    /// has to hold at every point in the fade and the midpoint is the one place
    /// a linear ramp also happens to look reasonable on a casual check.
    #[test]
    fn power_is_constant_across_the_fade() {
        for i in 0..=200 {
            let t = i as f64 / 200.0;
            let (out, inn) = crossfade_gains(t);
            assert!(
                (out * out + inn * inn - 1.0).abs() < 1e-9,
                "power {out:.4}^2 + {inn:.4}^2 != 1 at t={t}"
            );
        }
    }

    /// Exact endpoints, because the fade joins the steady state on both sides.
    ///
    /// Cos(0) and sin(π/2) are 1.0 exactly; cos(π/2) is 6.1e-17 rather than 0,
    /// which is inaudible but worth pinning so a future curve change that makes
    /// it worse is visible.
    #[test]
    fn the_ends_are_untouched() {
        let (out0, in0) = crossfade_gains(0.0);
        assert_eq!((out0, in0), (1.0, 0.0));

        let (out1, in1) = crossfade_gains(1.0);
        assert!(
            (out1 - 0.0).abs() < 1e-12,
            "outgoing did not reach zero: {out1}"
        );
        assert!(
            (in1 - 1.0).abs() < 1e-12,
            "incoming did not reach full: {in1}"
        );
    }

    /// Outgoing falls and incoming rises, monotonically, with one crossover —
    /// at the midpoint and nowhere else.
    ///
    /// Equal-power cos/sin must cross once: both gains are √½ when the phase
    /// hits π/4. The failure mode this rules out is a curve that holds power
    /// but inverts somewhere — tanh variants do that, and it sounds like a
    /// reversed fade rather than a louder middle.
    #[test]
    fn the_two_never_cross() {
        let mut prev_out = 1.0;
        let mut prev_in = 0.0;
        for i in 0..=200 {
            let t = i as f64 / 200.0;
            let (out, inn) = crossfade_gains(t);
            assert!(out <= prev_out + 1e-12, "outgoing rose at t={t}");
            assert!(inn >= prev_in - 1e-12, "incoming fell at t={t}");
            if t <= 0.5 {
                assert!(out >= inn, "crossover early at t={t}: {out} < {inn}");
            } else {
                assert!(inn >= out, "crossover late at t={t}: {out} > {inn}");
            }
            prev_out = out;
            prev_in = inn;
        }
    }

    /// Out of range is clamped rather than extrapolated.
    ///
    /// `progress` is already `.min(1.0)` at the call site, but the clamp is
    /// what makes the function safe on its own, and cos/sin of a negative phase
    /// would hand back a negative gain and invert a channel.
    #[test]
    fn out_of_range_clamps() {
        assert_eq!(crossfade_gains(-1.0), crossfade_gains(0.0));
        assert_eq!(crossfade_gains(2.0), crossfade_gains(1.0));
    }

    /// The old curve really was 3dB down in the middle — the bug being fixed.
    ///
    /// Kept as a linear-vs-equal-power comparison at the midpoint rather than a
    /// bare number, so the reason the curve exists is readable here and not
    /// only in the commit that introduced it.
    #[test]
    fn the_linear_curve_dipped_where_this_one_does_not() {
        let linear_power: f64 = 0.5 * 0.5 + 0.5 * 0.5;
        let (out, inn) = crossfade_gains(0.5);
        let equal_power = out * out + inn * inn;

        assert!(
            (linear_power - 0.5).abs() < 1e-12,
            "linear should sum to 0.5 power, got {linear_power}"
        );
        assert!(
            (equal_power - 1.0).abs() < 1e-9,
            "equal power should sum to 1.0, got {equal_power}"
        );
        // 10*log10(0.5) is the -3dB, stated so the number has a meaning.
        let dip_db = 10.0 * linear_power.log10();
        assert!((dip_db + 3.0103).abs() < 0.001, "dip was {dip_db} dB");
    }
}
