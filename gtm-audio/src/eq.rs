// Copyright (c) 2025 - present
// Author: prjctimg <prjctimg@outlook.com>
// Real-time 15-band parametric EQ and reverb using fundsp
//
// This is free software released under the GPL-3.0 license.

// `EqSource` wraps a `rodio::Source` and applies per-sample EQ via shared
// atomic gain values, allowing the mixer thread to change presets without
// restarting the stream.

use std::num::{NonZeroU16, NonZeroU32};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Arc;
use std::time::Duration;

use fundsp::audiounit::AudioUnit;
use fundsp::prelude32::*;
use rodio::Source;

use gtm_core::state::{EqPreset, EQ_DEFAULT_Q, EQ_FREQUENCIES};

// ---------------------------------------------------------------------------
// AtomicF32 (stable API wrapper)
// ---------------------------------------------------------------------------

struct AtomicF32(AtomicU32);

impl AtomicF32 {
    fn new(val: f32) -> Self {
        Self(AtomicU32::new(val.to_bits()))
    }
    fn load(&self, order: Ordering) -> f32 {
        f32::from_bits(self.0.load(order))
    }
    fn store(&self, val: f32, order: Ordering) {
        self.0.store(val.to_bits(), order);
    }
}

// ---------------------------------------------------------------------------
// EqGains — shared 15-band gain array
// ---------------------------------------------------------------------------

pub struct EqGainsInner {
    bands: [AtomicF32; 15],
    /// Global makeup-gain trim (dB, ≤ 0) applied after the bands so boosts
    /// can never push the output past full scale.
    headroom_db: AtomicF32,
}

#[derive(Clone)]
pub struct EqGains(pub Arc<EqGainsInner>);

impl EqGains {
    pub fn new_flat() -> Self {
        Self(Arc::new(EqGainsInner {
            bands: std::array::from_fn(|_| AtomicF32::new(0.0)),
            headroom_db: AtomicF32::new(0.0),
        }))
    }

    pub fn load(&self, index: usize) -> f32 {
        self.0.bands[index].load(Ordering::Relaxed)
    }

    pub fn store(&self, index: usize, value: f32) {
        self.0.bands[index].store(value, Ordering::Relaxed);
    }

    pub fn headroom(&self) -> f32 {
        self.0.headroom_db.load(Ordering::Relaxed)
    }

    pub fn apply_preset(&self, preset: &EqPreset) {
        let values = preset.to_gains();
        for (i, v) in values.iter().enumerate() {
            self.0.bands[i].store(*v, Ordering::Relaxed);
        }
        self.0
            .headroom_db
            .store(preset.headroom_db(), Ordering::Relaxed);
    }
}

// ---------------------------------------------------------------------------
// EqSource — 15-band parametric EQ applied per-sample
// ---------------------------------------------------------------------------

pub struct EqSource<I> {
    inner: I,
    eq_left: Box<dyn AudioUnit>,
    eq_right: Box<dyn AudioUnit>,
    gains: EqGains,
    prev_gains: [f32; 15],
    headroom_mult: f32,
    channels: NonZeroU16,
    sample_count: usize,
    gain_check_counter: usize,
}

impl<I> EqSource<I>
where
    I: Source<Item = f32>,
{
    pub fn new(inner: I, gains: EqGains) -> Self {
        let sr = inner.sample_rate().get() as f64;
        let channels = inner.channels();

        let mut eq_left = build_eq_boxed(sr);
        let mut eq_right = build_eq_boxed(sr);

        let mut prev_gains = [0.0_f32; 15];
        for i in 0..15 {
            let v = gains.load(i);
            prev_gains[i] = v;
            apply_band(&mut *eq_left, i, EQ_FREQUENCIES[i] as f32, v);
            apply_band(&mut *eq_right, i, EQ_FREQUENCIES[i] as f32, v);
        }

        let headroom_mult = db_amp(gains.headroom());

        Self {
            inner,
            eq_left,
            eq_right,
            gains,
            prev_gains,
            headroom_mult,
            channels,
            sample_count: 0,
            gain_check_counter: 0,
        }
    }
}

impl<I> Iterator for EqSource<I>
where
    I: Source<Item = f32>,
{
    type Item = f32;

    fn next(&mut self) -> Option<f32> {
        let raw = self.inner.next()?;

        self.gain_check_counter = self.gain_check_counter.wrapping_add(1);
        if self.gain_check_counter.is_multiple_of(128) {
            for (i, freq) in EQ_FREQUENCIES.iter().enumerate() {
                let v = self.gains.load(i);
                if v != self.prev_gains[i] {
                    apply_band(&mut *self.eq_left, i, *freq as f32, v);
                    apply_band(&mut *self.eq_right, i, *freq as f32, v);
                    self.prev_gains[i] = v;
                }
            }
            self.headroom_mult = db_amp(self.gains.headroom());
        }

        let ch = self.sample_count % self.channels.get() as usize;
        self.sample_count = self.sample_count.wrapping_add(1);

        let processed = if ch == 0 {
            self.eq_left.filter_mono(raw)
        } else {
            self.eq_right.filter_mono(raw)
        };

        Some((processed * self.headroom_mult).clamp(-1.0, 1.0))
    }
}

impl<I> Source for EqSource<I>
where
    I: Source<Item = f32>,
{
    fn current_span_len(&self) -> Option<usize> {
        None
    }

    fn channels(&self) -> NonZeroU16 {
        self.channels
    }

    fn sample_rate(&self) -> NonZeroU32 {
        self.inner.sample_rate()
    }

    fn total_duration(&self) -> Option<Duration> {
        self.inner.total_duration()
    }
}

// ---------------------------------------------------------------------------
// ReverbSource — stereo reverb, bypassable at runtime
// ---------------------------------------------------------------------------

type ReverbFn = Box<dyn FnMut(f32, f32) -> (f32, f32) + Send>;

pub struct ReverbSource<I> {
    inner: I,
    reverb: ReverbFn,
    reverb_enabled: Arc<AtomicBool>,
    channels: NonZeroU16,
    sample_count: usize,
    pending: Option<f32>,
}

impl<I> ReverbSource<I>
where
    I: Source<Item = f32>,
{
    pub fn new(inner: I, room_size: f32, reverb_enabled: Arc<AtomicBool>) -> Self {
        let sr = inner.sample_rate().get() as f64;
        let channels = inner.channels();

        let mut rev = reverb_stereo(room_size, 2.0, 0.5);
        rev.set_sample_rate(sr);

        let reverb: ReverbFn = Box::new(move |l: f32, r: f32| rev.filter_stereo(l, r));

        Self {
            inner,
            reverb,
            reverb_enabled,
            channels,
            sample_count: 0,
            pending: None,
        }
    }
}

impl<I> Iterator for ReverbSource<I>
where
    I: Source<Item = f32>,
{
    type Item = f32;

    fn next(&mut self) -> Option<f32> {
        if let Some(pending) = self.pending.take() {
            return Some(pending);
        }

        let raw = self.inner.next()?;

        if !self.reverb_enabled.load(Ordering::Relaxed) {
            return Some(raw);
        }

        if self.channels.get() == 2 {
            let right = self.inner.next()?;
            self.sample_count += 2;
            let (out_l, out_r) = (self.reverb)(raw, right);
            self.pending = Some(out_r);
            Some(out_l)
        } else {
            self.sample_count += 1;
            let (out_l, _out_r) = (self.reverb)(raw, raw);
            Some(out_l)
        }
    }
}

impl<I> Source for ReverbSource<I>
where
    I: Source<Item = f32>,
{
    fn current_span_len(&self) -> Option<usize> {
        None
    }

    fn channels(&self) -> NonZeroU16 {
        self.channels
    }

    fn sample_rate(&self) -> NonZeroU32 {
        self.inner.sample_rate()
    }

    fn total_duration(&self) -> Option<Duration> {
        self.inner.total_duration()
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn build_eq_boxed(sr: f64) -> Box<dyn AudioUnit> {
    let mut chain = pipei::<U15, _, _>(|_i: u64| bell_hz(0.0_f32, 1.0, 0.0));
    chain.set_sample_rate(sr);
    Box::new(chain)
}

fn apply_band(unit: &mut dyn AudioUnit, band: usize, freq: f32, gain_db: f32) {
    unit.set(Setting::center_q_gain(freq, EQ_DEFAULT_Q as f32, db_amp(gain_db)).index(band));
}
