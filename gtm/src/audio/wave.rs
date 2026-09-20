// Copyright (c) 2026
// Author: prjctimg <prjctimg@outlook.com>
// Time-domain waveform ring for oscilloscope / stereo-meter visualizer modes
//
// This is free software released under the GPL-3.0 license.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Maximum interleaved samples retained in the waveform ring. 1024 samples →
/// 512 stereo frames, ~93 ms at the ~11 kHz decimated capture rate — a
/// comfortable oscilloscope window for a terminal-width renderer, mirroring
/// cliamp's 2048-sample full-rate window in spirit.
pub const WAVEFORM_CAP: usize = 1024;

/// Frame decimation applied by the decode/stream threads: keep every Nth
/// stereo frame (8 at 44.1 kHz → ~5.5 kHz ring update) so the hot audio path
/// only pays one mutex lock per 8th frame.
pub const WAVEFORM_DECIM: usize = 8;

/// Freshness window after which a waveform snapshot is treated as silence.
/// Mirrors the spectrum freshness used for streamed sources.
pub const WAVEFORM_FRESHNESS: Duration = Duration::from_millis(300);

#[derive(Clone, Default)]
pub struct WaveformShared(Arc<Mutex<WaveformState>>);

#[derive(Default)]
struct WaveformState {
    /// Interleaved L/R pairs (±1.0). Mono sources duplicate each sample so
    /// the ring layout is uniform regardless of channel count.
    samples: Vec<f32>,
    /// True when the current source carries two distinct channels.
    stereo: bool,
    /// When the ring was last written; `None` after [`WaveformShared::clear`].
    touched: Option<Instant>,
}

impl WaveformShared {
    /// Append one interleaved (left, right) frame, evicting the oldest
    /// samples past the cap so the ring stays bounded and fresh.
    pub fn push_frame(&self, left: f32, right: f32) {
        let mut s = self.0.lock().unwrap();
        if s.samples.len() + 2 > WAVEFORM_CAP {
            // Drop the oldest aligned pairs to make room for one more.
            let over = s.samples.len() + 2 - WAVEFORM_CAP;
            let drop = over + (over % 2);
            s.samples.drain(0..drop);
        }
        s.samples.push(left);
        s.samples.push(right);
        s.touched = Some(Instant::now());
    }

    /// Replace the ring wholesale with externally produced interleaved
    /// samples (streamed sources), mirroring `publish_spectrum`.
    pub fn publish(&self, samples: Vec<f32>, stereo: bool) {
        let mut s = self.0.lock().unwrap();
        let mut samples = samples;
        samples.truncate(WAVEFORM_CAP);
        if samples.len() % 2 != 0 {
            samples.pop();
        }
        s.samples = samples;
        s.stereo = stereo;
        s.touched = Some(Instant::now());
    }

    /// Stamp the channel layout of the current source (written on open).
    pub fn set_stereo(&self, stereo: bool) {
        self.0.lock().unwrap().stereo = stereo;
    }

    /// Drop all samples so the UI decays to rest (used on stop).
    pub fn clear(&self) {
        let mut s = self.0.lock().unwrap();
        s.samples.clear();
        s.touched = None;
    }

    /// Latest waveform ring if written within `max_age`, plus its stereo
    /// flag. Returns an empty buffer otherwise so callers render silence.
    pub fn snapshot(&self, max_age: Duration) -> (Vec<f32>, bool) {
        let s = self.0.lock().unwrap();
        match s.touched {
            Some(t) if t.elapsed() <= max_age && !s.samples.is_empty() => {
                (s.samples.clone(), s.stereo)
            }
            _ => (Vec::new(), false),
        }
    }
}