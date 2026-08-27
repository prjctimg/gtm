// Copyright (c) 2026
// Author: prjctimg <prjctimg@outlook.com>
// Dedicated decode thread that reads audio files, applies EQ/reverb,
// and writes decoded samples into a lock-free SPSC ring buffer.
//
// This is free software released under the GPL-3.0 license.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

use fundsp::audiounit::AudioUnit;
use fundsp::prelude32::*;
use rodio::Source;

use gtm_core::global::{EQ_DEFAULT_Q, EQ_FREQUENCIES};

use crate::buffer::{DecodeControl, PREBUFFER_SAMPLES, SharedRingBuffer};
use crate::eq::EqGains;
use crate::symphonia::SymphoniaSource;

// ---------------------------------------------------------------------------
// EQ helpers (moved from eq.rs EqSource: processing on decode thread)
// ---------------------------------------------------------------------------

fn build_eq_boxed(sr: f64) -> Box<dyn AudioUnit> {
    let mut chain = pipei::<U15, _, _>(|_i: u64| bell_hz(0.0_f32, 1.0, 0.0));
    chain.set_sample_rate(sr);
    Box::new(chain)
}

fn apply_band(unit: &mut dyn AudioUnit, band: usize, freq: f32, gain_db: f32) {
    unit.set(Setting::center_q_gain(freq, EQ_DEFAULT_Q as f32, db_amp(gain_db)).index(band));
}

/// Anti-clip stage: trims the filtered sample by the preset headroom so band
/// boosts can never push past full scale, then hard-clamps as a final safety
/// net. With headroom compensation the clamp rarely engages.
fn apply_headroom(sample: f32, headroom_mult: f32) -> f32 {
    (sample * headroom_mult).clamp(-1.0, 1.0)
}

// ---------------------------------------------------------------------------
// Reverb helper: standalone processing function (no Source wrapper needed)
// ---------------------------------------------------------------------------

struct ReverbState {
    reverb: Box<dyn FnMut(f32, f32) -> (f32, f32) + Send>,
}

impl ReverbState {
    fn new(sr: f64, room_size: f32) -> Self {
        let mut rev = reverb_stereo(room_size, 2.0, 0.5);
        rev.set_sample_rate(sr);
        Self {
            reverb: Box::new(move |l: f32, r: f32| rev.filter_stereo(l, r)),
        }
    }

    /// Process a stereo pair.
    fn process_stereo(&mut self, left: f32, right: f32) -> (f32, f32) {
        (self.reverb)(left, right)
    }
}

// ---------------------------------------------------------------------------
// Spectrum analysis: Hann-windowed FFT with log-spaced bands for visualizer
// ---------------------------------------------------------------------------

use rustfft::FftPlanner;
use rustfft::num_complex::Complex;

/// Number of visualizer bands published to the UI.
pub const SPECTRUM_BINS: usize = 64;
/// FFT size: 2048 samples ≈ 46 ms at 44.1 kHz — enough resolution to
/// separate low-frequency content without adding perceptible latency.
const FFT_SIZE: usize = 2048;
/// Analyzed frequency range; below ~30 Hz only rumble lives, above ~16 kHz
/// there is rarely musical energy worth a bar.
const MIN_FREQ: f32 = 30.0;
const MAX_FREQ: f32 = 16_000.0;
/// Full-scale sine reference and noise floor for dB normalization.
const DB_RANGE: f32 = 70.0;

/// Incremental FFT spectrum analyzer fed single mono samples from a decode
/// or streaming source.  Once per [`FFT_SIZE`] samples it publishes
/// `SPECTRUM_BINS` log-spaced magnitudes normalized to `[0, 1]`.
pub struct SpectrumAnalyzer {
    fft: std::sync::Arc<dyn rustfft::Fft<f32>>,
    window: Vec<f32>,
    hann: Vec<f32>,
    scratch: Vec<Complex<f32>>,
    /// Precomputed band edges as FFT bin indices (`band_edges[b]..=band_edges[b+1]`).
    band_edges: [usize; SPECTRUM_BINS + 1],
}

impl SpectrumAnalyzer {
    pub fn new(sample_rate: f32) -> Self {
        let mut planner = FftPlanner::<f32>::new();
        let fft = planner.plan_fft_forward(FFT_SIZE);
        let hann: Vec<f32> = (0..FFT_SIZE)
            .map(|i| 0.5 * (1.0 - (2.0 * std::f32::consts::PI * i as f32 / FFT_SIZE as f32).cos()))
            .collect();
        // Log-spaced band edges clamped to the valid Nyquist range.
        let nyquist = sample_rate * 0.5;
        let max_freq = MAX_FREQ.min(nyquist);
        let mut band_edges = [0usize; SPECTRUM_BINS + 1];
        for (b, edge) in band_edges.iter_mut().enumerate() {
            let t = b as f32 / SPECTRUM_BINS as f32;
            let freq = MIN_FREQ * (max_freq / MIN_FREQ).powf(t);
            let bin = ((freq / sample_rate) * FFT_SIZE as f32) as usize;
            *edge = std::cmp::Ord::min(bin, FFT_SIZE / 2);
        }
        // Ensure strictly non-decreasing edges so every band covers ≥1 bin.
        for b in 1..=SPECTRUM_BINS {
            if band_edges[b] <= band_edges[b - 1] {
                band_edges[b] = band_edges[b - 1] + 1;
            }
        }
        Self {
            fft,
            window: Vec::with_capacity(FFT_SIZE),
            hann,
            scratch: vec![Complex::new(0.0, 0.0); FFT_SIZE],
            band_edges,
        }
    }

    /// Feed one sample; when the internal window fills, writes fresh band
    /// magnitudes into `out` and returns true.
    pub fn push(&mut self, sample: f32, out: &mut [f32]) -> bool {
        self.window.push(sample);
        if self.window.len() < FFT_SIZE {
            return false;
        }
        for (i, s) in self.window.iter().enumerate() {
            self.scratch[i] = Complex::new(s * self.hann[i], 0.0);
        }
        self.fft.process(&mut self.scratch);

        // Coherent gain of the Hann window, used to scale bins back to
        // amplitude units.
        let gain_inv = 2.0 / self.hann.iter().sum::<f32>();
        for (band, slot) in out.iter_mut().enumerate() {
            let lo = self.band_edges[band];
            let hi = std::cmp::Ord::min(self.band_edges[band + 1], FFT_SIZE / 2);
            let peak = self.scratch[lo..std::cmp::Ord::max(hi, lo + 1)]
                .iter()
                .map(|c| c.norm())
                .fold(0.0f32, f32::max)
                * gain_inv;
            // dB scale with floor: quiet but nonzero beats a dead bar grid.
            let db = 20.0 * peak.max(1e-9).log10();
            *slot = ((db + DB_RANGE) / DB_RANGE).clamp(0.0, 1.0);
        }
        self.window.clear();
        true
    }
}

// ---------------------------------------------------------------------------
// DecodeThread owns a dedicated std::thread
// ---------------------------------------------------------------------------

pub struct DecodeThread {
    path: String,
    shared: SharedRingBuffer,
    control: Arc<DecodeControl>,
    eq_gains: EqGains,
    eq_enabled: Arc<AtomicBool>,
    reverb_enabled: Arc<AtomicBool>,
    reverb_room_size: Arc<Mutex<f32>>,
    spectrum: Arc<Mutex<Vec<f32>>>,
}

impl DecodeThread {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        path: String,
        shared: SharedRingBuffer,
        control: Arc<DecodeControl>,
        eq_gains: EqGains,
        eq_enabled: Arc<AtomicBool>,
        reverb_enabled: Arc<AtomicBool>,
        reverb_room_size: Arc<Mutex<f32>>,
        spectrum: Arc<Mutex<Vec<f32>>>,
    ) -> Self {
        Self {
            path,
            shared,
            control,
            eq_gains,
            eq_enabled,
            reverb_enabled,
            reverb_room_size,
            spectrum,
        }
    }

    pub fn spawn(self) -> Result<JoinHandle<()>, String> {
        std::thread::Builder::new()
            .name("gtm-decode".into())
            .spawn(move || self.run())
            .map_err(|e| format!("failed to spawn decode thread: {e}"))
    }
}

impl DecodeThread {
    fn run(self) {
        let mut start_pos = 0.0_f64;

        loop {
            if !self.control.running.load(Ordering::Acquire) {
                break;
            }

            // Create decoder for current position
            let raw = match SymphoniaSource::from_file(&self.path, start_pos) {
                Ok(s) => s,
                Err(e) => {
                    log::error!("decode thread: failed to open {}: {e}", self.path);
                    self.shared.set_finished(true);
                    self.control.ready.store(true, Ordering::Release);
                    break;
                }
            };

            let sr = raw.sample_rate().get() as f64;
            let channels = raw.channels().get();

            // Store channel/rate info for RingBufferSource
            self.control.sample_rate.store(sr as u32, Ordering::Relaxed);
            self.control.channels.store(channels, Ordering::Relaxed);

            // Build EQ processing state
            let mut eq_left = build_eq_boxed(sr);
            let mut eq_right = build_eq_boxed(sr);
            let mut prev_gains = [0.0_f32; 15];
            for i in 0..15 {
                let v = self.eq_gains.load(i);
                prev_gains[i] = v;
                apply_band(&mut *eq_left, i, EQ_FREQUENCIES[i] as f32, v);
                apply_band(&mut *eq_right, i, EQ_FREQUENCIES[i] as f32, v);
            }
            let mut gain_check: usize = 0;
            let mut headroom_mult = db_amp(self.eq_gains.headroom());

            // Build reverb state
            let mut reverb_state = {
                let room = *self.reverb_room_size.lock().unwrap();
                Some(ReverbState::new(sr, room))
            };

            let mut sample_count: usize = 0;
            let mut prebuffered = false;
            // Downsample 4:1 before the FFT: musical content above ~5.5 kHz
            // (at 44.1 kHz) is negligible for visualization and this keeps the
            // analyzer cost tiny on the decode thread.
            const SPECTRUM_DECIMATION: usize = 4;
            let decimated_sr = (sr as f32 / SPECTRUM_DECIMATION as f32).max(1.0);
            let mut spectrum_analyzer = SpectrumAnalyzer::new(decimated_sr);
            let mut spectrum_frame = vec![0.0f32; SPECTRUM_BINS];
            let mut spectrum_count: usize = 0;

            // Decode loop: read from SymphoniaSource, process, write to ring buffer
            // Use an explicit loop over the iterator so we can check seek/running flags.
            let mut source_iter = raw;

            loop {
                // Check for stop
                if !self.control.running.load(Ordering::Acquire) {
                    self.control.seeking.store(false, Ordering::Release);
                    self.shared.set_finished(true);
                    return;
                }

                // Check for seek request
                if let Some(target_secs) = self.control.consume_seek() {
                    log::info!("decode thread: seek to {target_secs:.2}s");
                    start_pos = target_secs;
                    self.shared.flush();
                    self.control.seeking.store(false, Ordering::Release);
                    break; // restart decoder at new position
                }

                // Check ring buffer space: when nearly full, sleep briefly so
                // the consumer can drain — spinning burns a core under load.
                if self.shared.free_space() < 1024 {
                    std::thread::sleep(std::time::Duration::from_millis(1));
                    continue;
                }

                // Read raw sample from decoder
                let sample = match source_iter.next() {
                    Some(s) => s,
                    None => {
                        // EOF: drain ring buffer then signal finished
                        while self.shared.available() > 0 {
                            std::thread::yield_now();
                            if !self.control.running.load(Ordering::Acquire) {
                                self.control.seeking.store(false, Ordering::Release);
                                return;
                            }
                        }
                        self.control.seeking.store(false, Ordering::Release);
                        self.shared.set_finished(true);
                        self.control.ready.store(true, Ordering::Release);
                        return;
                    }
                };

                // Apply EQ
                let eq_sample = if self.eq_enabled.load(Ordering::Relaxed) {
                    gain_check += 1;
                    if gain_check.is_multiple_of(128) {
                        for i in 0..15 {
                            let v = self.eq_gains.load(i);
                            if v != prev_gains[i] {
                                apply_band(&mut *eq_left, i, EQ_FREQUENCIES[i] as f32, v);
                                apply_band(&mut *eq_right, i, EQ_FREQUENCIES[i] as f32, v);
                                prev_gains[i] = v;
                            }
                        }
                        headroom_mult = db_amp(self.eq_gains.headroom());
                    }
                    let ch = sample_count % channels as usize;
                    let filtered = if ch == 0 {
                        eq_left.filter_mono(sample)
                    } else {
                        eq_right.filter_mono(sample)
                    };
                    apply_headroom(filtered, headroom_mult)
                } else {
                    sample
                };

                // Apply reverb
                let final_sample = if self.reverb_enabled.load(Ordering::Relaxed) {
                    if channels == 2 {
                        let ch = sample_count % 2;
                        if ch == 0 {
                            // This is a left sample: we need the right sample too
                            match source_iter.next() {
                                Some(right_raw) => {
                                    let right_eq = if self.eq_enabled.load(Ordering::Relaxed) {
                                        apply_headroom(
                                            eq_right.filter_mono(right_raw),
                                            headroom_mult,
                                        )
                                    } else {
                                        right_raw
                                    };
                                    sample_count += 1; // count the right sample too
                                    if let Some(ref mut rev) = reverb_state {
                                        let (out_l, out_r) =
                                            rev.process_stereo(eq_sample, right_eq);
                                        // Write left now, push right to ring buffer
                                        self.shared.push_blocking(out_l);
                                        self.shared.push_blocking(out_r);
                                        sample_count += 1;
                                        prebuffer_check(
                                            &self.shared,
                                            &self.control,
                                            &mut prebuffered,
                                        );
                                        continue; // both channels written
                                    } else {
                                        self.shared.push_blocking(eq_sample);
                                        self.shared.push_blocking(right_eq);
                                        sample_count += 1;
                                        prebuffer_check(
                                            &self.shared,
                                            &self.control,
                                            &mut prebuffered,
                                        );
                                        continue;
                                    }
                                }
                                None => {
                                    self.shared.push_blocking(eq_sample);
                                    self.shared.set_finished(true);
                                    self.control.ready.store(true, Ordering::Release);
                                    return;
                                }
                            }
                        } else {
                            // Odd sample: should be right, but we handle in pairs above
                            eq_sample
                        }
                    } else {
                        if let Some(ref mut rev) = reverb_state {
                            let (out_l, _out_r) = rev.process_stereo(eq_sample, eq_sample);
                            out_l
                        } else {
                            eq_sample
                        }
                    }
                } else {
                    eq_sample
                };

                self.shared.push_blocking(final_sample);
                sample_count += 1;

                // Accumulate (decimated) samples for spectrum analysis.
                if spectrum_count.is_multiple_of(SPECTRUM_DECIMATION)
                    && spectrum_analyzer.push(final_sample, &mut spectrum_frame)
                    && let Ok(mut spec) = self.spectrum.lock()
                {
                    spec.clear();
                    spec.extend_from_slice(&spectrum_frame);
                }
                spectrum_count += 1;

                prebuffer_check(&self.shared, &self.control, &mut prebuffered);
            }
        }

        self.shared.set_finished(true);
    }
}

fn prebuffer_check(shared: &SharedRingBuffer, control: &DecodeControl, prebuffered: &mut bool) {
    if !*prebuffered && shared.available() >= PREBUFFER_SAMPLES {
        *prebuffered = true;
        control.ready.store(true, Ordering::Release);
        log::info!(
            "decode thread: prebuffer ready ({} samples in buffer)",
            shared.available()
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn log_bands_are_monotonic_and_covered() {
        let a = SpectrumAnalyzer::new(44_100.0);
        for b in 0..SPECTRUM_BINS {
            assert!(a.band_edges[b + 1] > a.band_edges[b]);
        }
    }

    #[test]
    fn sine_energy_lands_in_expected_band() {
        let sr = 44_100.0;
        let freq = 440.0f32;
        let mut a = SpectrumAnalyzer::new(sr);
        let mut out = vec![0.0f32; SPECTRUM_BINS];
        for i in 0..FFT_SIZE {
            let s = (2.0 * std::f32::consts::PI * freq * i as f32 / sr).sin();
            a.push(s, &mut out);
        }
        let expected_bin = (freq / sr * FFT_SIZE as f32) as usize;
        let peak_band = (0..SPECTRUM_BINS)
            .max_by(|x, y| out[*x].total_cmp(&out[*y]))
            .unwrap();
        let lo = a.band_edges[peak_band];
        let hi = a.band_edges[peak_band + 1];
        assert!(
            lo <= expected_bin && expected_bin < hi,
            "440 Hz energy in bins {lo}..{hi}, expected {expected_bin}"
        );
        assert!(out[peak_band] > 0.5, "peak magnitude too small");
    }
}
