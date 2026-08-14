// Copyright (c) 2026 - present
// Author: prjctimg <prjctimg@outlook.com>
// WSOLA time-stretch source for pitch-preserving playback speed
//
// This is free software released under the GPL-3.0 license.
//
// Implements Waveform Similarity Overlap-Add: the decoded input stream is cut
// into overlapping windows that are re-spliced at a different rate so the
// duration changes while pitch stays constant. Speed is read from a shared
// fixed-point atomic so it can be adjusted live mid-track.

use std::num::{NonZeroU16, NonZeroU32};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use std::time::Duration;

use rodio::Source;

/// Analysis window length in frames (46 ms at 44.1 kHz).
const ANALYSIS_N: usize = 2048;
/// Synthesis hop in frames (50% overlap).
const SYNTH_HOP: usize = ANALYSIS_N / 2;
/// Cross-correlation search range in frames.
const SEARCH_RANGE: usize = 128;

/// Convert a playback rate (0.5–2.0) to the fixed-point value shared between
/// the UI, daemon and decode thread.
pub fn speed_to_fixed(rate: f64) -> u32 {
    (rate.clamp(0.5, 2.0) * 1000.0).round() as u32
}

/// Read the fixed-point speed as a float, clamped to the supported range.
pub fn fixed_to_speed(fixed: u32) -> f64 {
    (fixed as f64 / 1000.0).clamp(0.5, 2.0)
}

/// A `rodio::Source` that time-stretches its input while preserving pitch.
///
/// The input stream (interleaved f32 samples) is buffered into frames and
/// re-synthesized with a streaming WSOLA engine. Speed changes are picked up
/// from `speed` at the start of each synthesis step.
pub struct TimeStretchSource<S> {
    inner: S,
    channels: usize,
    speed: Arc<AtomicU32>,
    /// Buffered input frames (interleaved). `input[0]` is the frame at
    /// absolute position `input_head`.
    input: Vec<f32>,
    input_head: usize,
    /// Nominal absolute input position of the next window start (advances by
    /// `hop` each step).
    nominal_pos: f64,
    /// OLA accumulation buffer, `ANALYSIS_N * channels` frames wide.
    out_buf: Vec<f32>,
    /// Produced output waiting to be served.
    output: Vec<f32>,
    output_pos: usize,
    /// Last `SYNTH_HOP` frames of the final output, used as the correlation
    /// reference for the next window.
    prev_tail: Vec<f32>,
    window: Vec<f32>,
    inner_finished: bool,
    flushed: bool,
}

impl<S> TimeStretchSource<S>
where
    S: Source<Item = f32>,
{
    pub fn new(inner: S, speed: Arc<AtomicU32>) -> Self {
        let channels = inner.channels().get() as usize;
        let window = hann_window(ANALYSIS_N);
        Self {
            inner,
            channels,
            speed,
            input: Vec::with_capacity((ANALYSIS_N + SEARCH_RANGE * 2 + SYNTH_HOP) * channels),
            input_head: 0,
            nominal_pos: 0.0,
            out_buf: vec![0.0; ANALYSIS_N * channels],
            output: Vec::new(),
            output_pos: 0,
            prev_tail: vec![0.0; SYNTH_HOP * channels],
            window,
            inner_finished: false,
            flushed: false,
        }
    }

    /// Consume the current output block, then try to synthesize the next one.
    fn serve(&mut self) -> Option<f32> {
        if self.output_pos < self.output.len() {
            let s = self.output[self.output_pos];
            self.output_pos += 1;
            return Some(s);
        }
        if self.flushed {
            return None;
        }
        self.output.clear();
        self.output_pos = 0;

        let hop = self.current_hop();
        let mut produced = false;
        while !produced && !self.flushed {
            // Pull input until we can run the next synthesis step or EOF.
            self.fill_input();
            if let Some(step) = self.try_synthesize(hop) {
                produced = step;
            }
        }
        if self.output_pos < self.output.len() {
            let s = self.output[self.output_pos];
            self.output_pos += 1;
            return Some(s);
        }
        None
    }

    /// Analysis hop in input frames for the current speed.
    fn current_hop(&self) -> f64 {
        let fixed = self.speed.load(Ordering::Relaxed).max(1);
        SYNTH_HOP as f64 * fixed_to_speed(fixed)
    }

    /// Pull frames from the inner source until the input buffer covers the
    /// next window (nominal ± search range + window) or the source is spent.
    fn fill_input(&mut self) {
        let ch = self.channels;
        let required_end = (self.nominal_pos + SEARCH_RANGE as f64 + ANALYSIS_N as f64) as usize;
        let have_end = self.input_head + self.input.len() / ch;
        if self.inner_finished || have_end >= required_end {
            return;
        }
        let mut broke = false;
        for sample in self.inner.by_ref() {
            self.input.push(sample);
            let have = self.input.len() / ch;
            if self.input_head + have >= required_end {
                broke = true;
                break;
            }
        }
        // The iterator is exhausted only when the loop ran to completion.
        self.inner_finished = !broke;
    }

    /// One WSOLA step: search for the best window alignment, overlap-add it,
    /// and hand `SYNTH_HOP` frames to the output. Returns `true` when a step
    /// ran, `false` when there is not enough input (final flush pending).
    fn try_synthesize(&mut self, hop: f64) -> Option<bool> {
        let ch = self.channels;
        // The window may start anywhere in [nominal - RANGE, nominal + RANGE]
        // and extend ANALYSIS_N frames, so relative to input_head we need:
        let required = (self.nominal_pos + SEARCH_RANGE as f64 + ANALYSIS_N as f64) as isize
            - self.input_head as isize;
        let avail = self.input.len() / ch;
        if (avail as isize) >= required && !self.flushed {
            let delta = self.search_alignment();
            let wpos = (self.nominal_pos as isize + delta).max(0) as usize;
            let rel = wpos - self.input_head;
            self.overlap_add(rel);
            self.emit_block();

            // Advance and trim input.
            self.nominal_pos += hop;
            let trim_to = (self.nominal_pos as i64 - SEARCH_RANGE as i64).max(0) as usize;
            let drop = (trim_to.saturating_sub(self.input_head)) * ch;
            if drop > 0 {
                self.input.drain(..drop);
                self.input_head = trim_to;
            }
            Some(true)
        } else {
            self.flush();
            Some(false)
        }
    }

    /// Find the offset (in frames) of the best-aligned window start within
    /// the search range, by correlating the candidate window lead against the
    /// tail of the previously synthesized output.
    fn search_alignment(&mut self) -> isize {
        let ch = self.channels;
        let nominal = self.nominal_pos as isize;
        let mut best_delta: isize = 0;
        let mut best_score = f64::NEG_INFINITY;
        let prev = &self.prev_tail;
        let window = &self.window;
        // Only search starts that lie inside the buffered input.
        let first = (nominal - SEARCH_RANGE as isize).max(self.input_head as isize);
        for delta in (first - nominal)..=(SEARCH_RANGE as isize) {
            let rel = (nominal + delta - self.input_head as isize) as usize;
            let mut num = 0.0f64;
            let mut den_a = 0.0f64;
            let mut den_b = 0.0f64;
            // Correlate the first synthesis hop of the candidate window with
            // the reference tail.
            for f in 0..SYNTH_HOP {
                let win = window[f];
                for c in 0..ch {
                    let a = self.input[(rel + f) * ch + c] * win;
                    let b = prev[f * ch + c];
                    num += a as f64 * b as f64;
                    den_a += a as f64 * a as f64;
                    den_b += b as f64 * b as f64;
                }
            }
            let score = if den_a > 1e-12 && den_b > 1e-12 {
                num / (den_a * den_b).sqrt()
            } else {
                0.0
            };
            if score > best_score {
                best_score = score;
                best_delta = delta;
            }
        }
        best_delta
    }

    /// Overlap-add the window starting at input-relative frame `rel`.
    fn overlap_add(&mut self, rel: usize) {
        let ch = self.channels;
        let window = &self.window;
        let input = &self.input;
        for (f, &w) in window.iter().enumerate().take(ANALYSIS_N) {
            let src = (rel + f) * ch;
            let dst = f * ch;
            for c in 0..ch {
                self.out_buf[dst + c] += w * input[src + c];
            }
        }
    }

    /// Move the first synthesis hop to the output, shift the accumulation
    /// buffer, and refresh the correlation reference.
    fn emit_block(&mut self) {
        let ch = self.channels;
        let block = SYNTH_HOP * ch;
        self.output.extend_from_slice(&self.out_buf[..block]);
        self.prev_tail.copy_from_slice(&self.output[..block]);
        self.out_buf.copy_within(block.., 0);
        self.out_buf[ANALYSIS_N * ch - block..].fill(0.0);
    }

    /// No more input: emit whatever remains accumulated, then stop.
    fn flush(&mut self) {
        if self.flushed {
            return;
        }
        self.flushed = true;
        self.output.extend_from_slice(&self.out_buf);
        self.out_buf.fill(0.0);
    }
}

/// Symmetric Hann window of length `n`.
fn hann_window(n: usize) -> Vec<f32> {
    (0..n)
        .map(|i| {
            let t = (i as f64 + 0.5) * std::f64::consts::TAU / n as f64;
            (0.5 - 0.5 * t.cos()) as f32
        })
        .collect()
}

impl<S> Iterator for TimeStretchSource<S>
where
    S: Source<Item = f32>,
{
    type Item = f32;
    fn next(&mut self) -> Option<f32> {
        self.serve()
    }
}

impl<S> Source for TimeStretchSource<S>
where
    S: Source<Item = f32>,
{
    fn current_span_len(&self) -> Option<usize> {
        None
    }

    fn channels(&self) -> NonZeroU16 {
        NonZeroU16::new(self.channels as u16).unwrap_or(NonZeroU16::MIN)
    }

    fn sample_rate(&self) -> NonZeroU32 {
        self.inner.sample_rate()
    }

    fn total_duration(&self) -> Option<Duration> {
        self.inner
            .total_duration()
            .map(|d| d.div_f64(fixed_to_speed(self.speed.load(Ordering::Relaxed).max(1))))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::num::{NonZeroU16, NonZeroU32};
    use std::sync::atomic::AtomicU32;

    struct Sine {
        phase: f64,
        total: usize,
        emitted: usize,
    }

    impl Iterator for Sine {
        type Item = f32;
        fn next(&mut self) -> Option<f32> {
            if self.emitted >= self.total {
                return None;
            }
            self.emitted += 1;
            let s = (self.phase * std::f64::consts::TAU).sin() as f32;
            self.phase += 440.0 / 44100.0;
            Some(s)
        }
    }

    impl Source for Sine {
        fn channels(&self) -> NonZeroU16 {
            NonZeroU16::new(1).unwrap()
        }
        fn sample_rate(&self) -> NonZeroU32 {
            NonZeroU32::new(44100).unwrap()
        }
        fn current_span_len(&self) -> Option<usize> {
            None
        }
        fn total_duration(&self) -> Option<Duration> {
            None
        }
    }

    fn run(speed: u32, input_secs: f64) -> (usize, usize) {
        let total = (input_secs * 44100.0) as usize;
        let inner = Sine {
            phase: 0.0,
            total,
            emitted: 0,
        };
        let speed_arc = Arc::new(AtomicU32::new(speed));
        let mut src = TimeStretchSource::new(inner, speed_arc);
        let mut out_len = 0usize;
        while src.next().is_some() {
            out_len += 1;
        }
        (total, out_len)
    }

    #[test]
    fn stretches_duration_inversely_to_speed() {
        let (in_len, out_len) = run(2000, 2.0); // 2.0x speed
        let ratio = out_len as f64 / in_len as f64;
        // ~0.5x duration; allow slack for the window latency + flush.
        assert!(
            (0.40..0.55).contains(&ratio),
            "2.0x speed produced ratio {ratio}"
        );
    }

    #[test]
    fn identity_speed_preserves_length() {
        let (in_len, out_len) = run(1000, 2.0); // 1.0x
        let ratio = out_len as f64 / in_len as f64;
        assert!(
            (0.9..1.1).contains(&ratio),
            "1.0x speed produced ratio {ratio}"
        );
    }

    #[test]
    fn slow_speed_lengthens() {
        let (in_len, out_len) = run(500, 2.0); // 0.5x
        let ratio = out_len as f64 / in_len as f64;
        assert!(
            (1.8..2.2).contains(&ratio),
            "0.5x speed produced ratio {ratio}"
        );
    }
}
