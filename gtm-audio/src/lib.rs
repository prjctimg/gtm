// Copyright (c) 2026
// Author: prjctimg <prjctimg@outlook.com>
// Audio library root: re-exports mixer backends, EQ, and null mixer
//
// This is free software released under the GPL-3.0 license.

pub mod backend;
pub mod buffer;
pub mod decoder;
pub mod eq;
pub mod mixer;
pub mod silent;
pub mod symphonia;

#[cfg(feature = "pulseaudio")]
pub mod pulse;

pub use backend::{AudioError, AudioEvent, AudioResult};
pub use buffer::{DecodeControl, RingBufferSource};
pub use decoder::{SPECTRUM_BINS, SpectrumAnalyzer};
pub use eq::{EqGains, EqSource, ReverbSource};
pub use mixer::{AudioMixer, Mixer};
pub use silent::NullMixer;

#[cfg(feature = "pulseaudio")]
pub use pulse::PulseAudioMixer;
