// Copyright (c) 2026
// Author: prjctimg <prjctimg@outlook.com>
// Pitch-preserving variable-speed playback via the `timestretch` real-time
// engine (phase-vocoder + WSOLA hybrid).
//
// This is free software released under the GPL-3.0 license.

use std::num::{NonZeroU16, NonZeroU32};
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

use rodio::Source;
use timestretch::engine::{Engine, EngineConfig, EngineProfile};
use timestretch::engine::{EngineController, EngineProcessor, SourceProducer};

/// Re-exported shared speed bounds (single source of truth in `gtm-core`).
pub use gtm_core::global::{DEFAULT_SPEED, MAX_SPEED, MIN_SPEED};

/// Playback rate bounds for the engine.
pub fn speed_bounds() -> (f32, f32) {
    (MIN_SPEED, MAX_SPEED)
}

/// Speed control shared with the mixer thread. The decode thread reads the
/// loaded rate every block and retargets the engine when it changes. Stored
/// as the f32 bit pattern in an atomic so status reads stay lock-free, and
/// it is clamped to the engine's supported range on store.
#[derive(Clone)]
pub struct SpeedControl(Arc<AtomicU32>);

impl Default for SpeedControl {
    fn default() -> Self {
        Self::new()
    }
}

impl SpeedControl {
    pub fn new() -> Self {
        Self(Arc::new(AtomicU32::new(DEFAULT_SPEED.to_bits())))
    }

    /// Current playback rate (0.25..=2.0).
    pub fn load(&self) -> f32 {
        f32::from_bits(self.0.load(Ordering::Relaxed))
    }

    /// Set the playback rate, clamped to [MIN_SPEED, MAX_SPEED]. Non-finite
    /// values reset to 1.0.
    pub fn store(&self, rate: f32) {
        let clamped = if rate.is_finite() {
            rate.clamp(MIN_SPEED, MAX_SPEED)
        } else {
            DEFAULT_SPEED
        };
        self.0.store(clamped.to_bits(), Ordering::Relaxed);
    }
}

/// Number of raw interleaved frames to accumulate from the inner source
/// before each engine feed. Kept modest so decode latency stays low; the
/// engine's internal source ring absorbs scheduling slack.
const FEED_FRAMES: usize = 1024;
/// Output frames requested from the engine per `process` call.
const OUT_FRAMES: usize = 1024;

/// Engine handles bundled together; `None` when the engine could not be
/// built (in which case the source falls back to a transparent passthrough).
struct EngineState {
    controller: EngineController,
    processor: EngineProcessor,
    producer: SourceProducer,
}

/// A `rodio::Source` that wraps an inner source and pitch-preserving
/// time-stretches it at the rate requested via [`SpeedControl`].
///
/// When the rate is 1.0 the source is a transparent pass-through (no added
/// latency or CPU). At any other rate, raw interleaved frames from the
/// inner source are fed into the `timestretch` engine, and the stretched
/// output frames are yielded sample-by-sample. The engine uses the
/// `WideKeylock` profile, which keylocks the full spectrum across the
/// entire 0.25–2.0 tempo range with zero added pipeline delay.
pub struct TimeStretchSource<I> {
    inner: I,
    engine: Option<EngineState>,
    speed: SpeedControl,
    channels: NonZeroU16,
    sample_rate: NonZeroU32,
    /// Interleaved output frames pending delivery to the consumer.
    out: Vec<f32>,
    out_pos: usize,
    /// Interleaved raw frames pending feed to the engine.
    raw: Vec<f32>,
    raw_pos: usize,
    _eof: bool,
    /// Rate currently applied to the engine (to avoid needless retargets).
    applied_rate: f32,
    /// Tracks whether the inner source has reported EOF.
    inner_eof: bool,
}

impl<I> TimeStretchSource<I>
where
    I: Source<Item = f32>,
{
    pub fn new(inner: I, speed: SpeedControl) -> Self {
        let sr = inner.sample_rate().get();
        let ch = inner.channels().get() as usize;
        let initial_rate = speed.load();

        let engine = Engine::build(EngineConfig {
            sample_rate: sr,
            channels: ch,
            profile: EngineProfile::WideKeylock,
            initial_tempo_rate: initial_rate as f64,
            max_block_frames: OUT_FRAMES,
            // source_capacity_frames must cover four max callbacks at max
            // tempo (2.0): OUT_FRAMES * 2 * 4, plus a margin.
            source_capacity_frames: OUT_FRAMES * 2 * 4,
            ..EngineConfig::default()
        })
        .ok()
        .map(|handles| EngineState {
            controller: handles.controller,
            processor: handles.processor,
            producer: handles.source,
        });

        if engine.is_none() {
            log::error!("timestretch: engine build failed, running at unity");
        }

        Self {
            inner,
            engine,
            speed,
            channels: NonZeroU16::new(ch as u16).unwrap_or(NonZeroU16::MIN),
            sample_rate: NonZeroU32::new(sr).unwrap_or(NonZeroU32::MIN),
            out: Vec::new(),
            out_pos: 0,
            raw: Vec::new(),
            raw_pos: 0,
            _eof: false,
            applied_rate: initial_rate,
            inner_eof: false,
        }
    }

    fn enabled(&self) -> bool {
        self.engine.is_some() && (self.speed.load() - DEFAULT_SPEED).abs() > f32::EPSILON
    }
}

impl<I> Iterator for TimeStretchSource<I>
where
    I: Source<Item = f32>,
{
    type Item = f32;

    fn next(&mut self) -> Option<f32> {
        // At unity speed (or if the engine failed to build) the inner
        // source passes straight through — no extra latency, no CPU.
        if !self.enabled() {
            return self.inner.next();
        }
        let state = self.engine.as_mut().expect("enabled() implies engine");

        // Deliver a buffered stretched sample if one is queued.
        if self.out_pos < self.out.len() {
            let s = self.out[self.out_pos];
            self.out_pos += 1;
            return Some(s);
        }

        // Retarget the engine if the requested rate changed.
        let want = self.speed.load();
        if (want - self.applied_rate).abs() > f32::EPSILON {
            state.controller.set_tempo_rate(want as f64);
            self.applied_rate = want;
        }

        let ch = self.channels.get() as usize;

        // Refill the raw frame buffer from the inner source (whole frames at
        // a time so the engine always sees interleaved frames).
        if self.raw_pos >= self.raw.len() {
            self.raw.clear();
            self.raw_pos = 0;
            let target = FEED_FRAMES * ch;
            while self.raw.len() < target {
                match self.inner.next() {
                    Some(s) => self.raw.push(s),
                    None => {
                        self.inner_eof = true;
                        break;
                    }
                }
            }
        }

        // Feed whatever raw frames we have to the engine.
        if self.raw_pos < self.raw.len() {
            let available = self.raw.len() - self.raw_pos;
            let frames = available / ch;
            if frames > 0 {
                let stop = self.raw_pos + frames * ch;
                let _ = state.producer.push(&self.raw[self.raw_pos..stop]);
                self.raw_pos = stop;
            }
        }

        // On EOF signal the engine to flush its resampler lookahead so all
        // real source audio is released to the output.
        if self.inner_eof {
            state.producer.finish();
        }

        // Pull a stretched block from the engine.
        self.out.clear();
        self.out_pos = 0;
        self.out.resize(OUT_FRAMES * ch, 0.0);
        state.processor.process(&mut self.out);

        if self.out_pos < self.out.len() {
            let s = self.out[self.out_pos];
            self.out_pos += 1;
            Some(s)
        } else {
            None
        }
    }
}

impl<I> Source for TimeStretchSource<I>
where
    I: Source<Item = f32>,
{
    fn current_span_len(&self) -> Option<usize> {
        self.inner.current_span_len()
    }

    fn channels(&self) -> NonZeroU16 {
        self.channels
    }

    fn sample_rate(&self) -> NonZeroU32 {
        self.sample_rate
    }

    fn total_duration(&self) -> Option<Duration> {
        self.inner.total_duration()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn speed_control_clamps_and_defaults() {
        let c = SpeedControl::new();
        assert_eq!(c.load(), 1.0);
        c.store(1.5);
        assert_eq!(c.load(), 1.5);
        c.store(0.1);
        assert_eq!(c.load(), MIN_SPEED);
        c.store(5.0);
        assert_eq!(c.load(), MAX_SPEED);
        c.store(f32::NAN);
        assert_eq!(c.load(), 1.0);
    }
}
