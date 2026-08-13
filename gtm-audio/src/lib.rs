// Copyright (c) 2025 - present
// Author: prjctimg <prjctimg@outlook.com>
// Audio library root: re-exports mixer backends, EQ, and null mixer
//
// This is free software released under the GPL-3.0 license.

pub mod backend;
pub mod decode_thread;
pub mod eq;
pub mod null_mixer;
pub mod ring_buffer;
pub mod rodio;
pub mod symphonia;

#[cfg(feature = "pulseaudio")]
pub mod pulse_mixer;

pub use backend::{AudioBackend, AudioError, AudioEvent, AudioResult};
pub use eq::{EqGains, EqSource, ReverbSource};
pub use mixer::{AudioMixer, Mixer};
pub use null_mixer::NullMixer;
pub use ring_buffer::{DecodeControl, RingBufferSource};

#[cfg(feature = "pulseaudio")]
pub use pulse_mixer::PulseAudioMixer;

pub mod mixer;
