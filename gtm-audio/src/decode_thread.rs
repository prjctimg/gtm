// Copyright (c) 2025 - present
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

use gtm_core::state::{EQ_DEFAULT_Q, EQ_FREQUENCIES};

use crate::eq::EqGains;
use crate::ring_buffer::{DecodeControl, PREBUFFER_SAMPLES, SharedRingBuffer};
use crate::symphonia::SymphoniaSource;

// ---------------------------------------------------------------------------
// EQ helpers (moved from eq.rs EqSource — processing on decode thread)
// ---------------------------------------------------------------------------

fn build_eq_boxed(sr: f64) -> Box<dyn AudioUnit> {
    let mut chain = pipei::<U15, _, _>(|_i: u64| bell_hz(0.0_f32, 1.0, 0.0));
    chain.set_sample_rate(sr);
    Box::new(chain)
}

fn apply_band(unit: &mut dyn AudioUnit, band: usize, freq: f32, gain_db: f32) {
    unit.set(Setting::center_q_gain(freq, EQ_DEFAULT_Q as f32, db_amp(gain_db)).index(band));
}

// ---------------------------------------------------------------------------
// Reverb helper — standalone processing function (no Source wrapper needed)
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
// DecodeThread — runs on a dedicated std::thread
// ---------------------------------------------------------------------------

pub struct DecodeThread {
    path: String,
    shared: SharedRingBuffer,
    control: Arc<DecodeControl>,
    eq_gains: EqGains,
    eq_enabled: Arc<AtomicBool>,
    reverb_enabled: Arc<AtomicBool>,
    reverb_room_size: Arc<Mutex<f32>>,
}

impl DecodeThread {
    pub fn new(
        path: String,
        shared: SharedRingBuffer,
        control: Arc<DecodeControl>,
        eq_gains: EqGains,
        eq_enabled: Arc<AtomicBool>,
        reverb_enabled: Arc<AtomicBool>,
        reverb_room_size: Arc<Mutex<f32>>,
    ) -> Self {
        Self {
            path,
            shared,
            control,
            eq_gains,
            eq_enabled,
            reverb_enabled,
            reverb_room_size,
        }
    }

    pub fn spawn(self) -> JoinHandle<()> {
        std::thread::Builder::new()
            .name("gtm-decode".into())
            .spawn(move || self.run())
            .expect("failed to spawn decode thread")
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

            // Build reverb state
            let mut reverb_state = {
                let room = *self.reverb_room_size.lock().unwrap();
                Some(ReverbState::new(sr, room))
            };

            let mut sample_count: usize = 0;
            let mut prebuffered = false;

            // Decode loop — read from SymphoniaSource, process, write to ring buffer
            // Use an explicit loop over the iterator so we can check seek/running flags.
            let mut source_iter = raw;

            loop {
                // Check for stop
                if !self.control.running.load(Ordering::Acquire) {
                    self.shared.set_finished(true);
                    return;
                }

                // Check for seek request
                if let Some(target_secs) = self.control.consume_seek() {
                    log::info!("decode thread: seek to {target_secs:.2}s");
                    start_pos = target_secs;
                    self.shared.flush();
                    break; // restart decoder at new position
                }

                // Check ring buffer space — if nearly full, yield to avoid spinning
                if self.shared.free_space() < 1024 {
                    std::thread::yield_now();
                    continue;
                }

                // Read raw sample from decoder
                let sample = match source_iter.next() {
                    Some(s) => s,
                    None => {
                        // EOF — drain ring buffer then signal finished
                        // Wait a tiny bit for the consumer to read remaining samples
                        while self.shared.available() > 0 {
                            std::thread::yield_now();
                            if !self.control.running.load(Ordering::Acquire) {
                                return;
                            }
                        }
                        self.shared.set_finished(true);
                        self.control.ready.store(true, Ordering::Release);
                        return;
                    }
                };

                // Apply EQ
                let eq_sample = if self.eq_enabled.load(Ordering::Relaxed) {
                    gain_check += 1;
                    if gain_check % 128 == 0 {
                        for i in 0..15 {
                            let v = self.eq_gains.load(i);
                            if v != prev_gains[i] {
                                apply_band(&mut *eq_left, i, EQ_FREQUENCIES[i] as f32, v);
                                apply_band(&mut *eq_right, i, EQ_FREQUENCIES[i] as f32, v);
                                prev_gains[i] = v;
                            }
                        }
                    }
                    let ch = sample_count % channels as usize;
                    if ch == 0 {
                        eq_left.filter_mono(sample)
                    } else {
                        eq_right.filter_mono(sample)
                    }
                } else {
                    sample
                };

                // Apply reverb
                let final_sample = if self.reverb_enabled.load(Ordering::Relaxed) {
                    if channels == 2 {
                        let ch = sample_count % 2;
                        if ch == 0 {
                            // This is a left sample — we need the right sample too
                            match source_iter.next() {
                                Some(right_raw) => {
                                    let right_eq = if self.eq_enabled.load(Ordering::Relaxed) {
                                        eq_right.filter_mono(right_raw)
                                    } else {
                                        right_raw
                                    };
                                    sample_count += 1; // count the right sample too
                                    if let Some(ref mut rev) = reverb_state {
                                        let (out_l, out_r) = rev.process_stereo(eq_sample, right_eq);
                                        // Write left now, push right to ring buffer
                                        let _ = self.shared.push(out_l);
                                        let _ = self.shared.push(out_r);
                                        sample_count += 1;
                                        prebuffer_check(&self.shared, &self.control, &mut prebuffered);
                                        continue; // both channels written
                                    } else {
                                        let _ = self.shared.push(eq_sample);
                                        let _ = self.shared.push(right_eq);
                                        sample_count += 1;
                                        prebuffer_check(&self.shared, &self.control, &mut prebuffered);
                                        continue;
                                    }
                                }
                                None => {
                                    let _ = self.shared.push(eq_sample);
                                    self.shared.set_finished(true);
                                    self.control.ready.store(true, Ordering::Release);
                                    return;
                                }
                            }
                        } else {
                            // Odd sample — should be right, but we handle in pairs above
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

                let _ = self.shared.push(final_sample);
                sample_count += 1;
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
