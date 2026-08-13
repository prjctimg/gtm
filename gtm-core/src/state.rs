// Copyright (c) 2025 - present
// Author: prjctimg <prjctimg@outlook.com>
// Core error type, primitive enums, daemon state, and UI types
//
// This is free software released under the GPL-3.0 license.

use serde::{Deserialize, Serialize};

use crate::track::TrackInfo;

#[derive(Debug, thiserror::Error)]
pub enum CoreError {
    #[error("serialization error: {0}")]
    Serialization(#[from] serde_json::Error),
    #[error("bincode error: {0}")]
    Bincode(#[from] bincode::Error),
    #[error("i/o error: {0}")]
    Io(#[from] std::io::Error),
    #[error("daemon error: {0}")]
    Daemon(String),
    #[error("not connected")]
    NotConnected,
    #[error("timeout")]
    Timeout,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Easing {
    Linear,
    SlowFadeInFastFadeOut,
    FastFadeInSlowFadeOut,
    Logarithmic,
    Smoothstep,
    EqualPower,
    Exponential,
}

impl Easing {
    pub fn name(&self) -> &'static str {
        match self {
            Self::Linear => "Linear",
            Self::SlowFadeInFastFadeOut => "Slow In, Fast Out",
            Self::FastFadeInSlowFadeOut => "Fast In, Slow Out",
            Self::Logarithmic => "Logarithmic",
            Self::Smoothstep => "Smoothstep",
            Self::EqualPower => "Equal Power",
            Self::Exponential => "Exponential",
        }
    }
}

impl Default for Easing {
    fn default() -> Self {
        Self::Linear
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CrossfadeConfig {
    pub enabled: bool,
    pub duration_secs: u8,
    pub easing: Easing,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReverbConfig {
    pub enabled: bool,
    pub room_size: f32,
}

impl Default for ReverbConfig {
    fn default() -> Self {
        Self { enabled: false, room_size: 0.5 }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DaemonState {
    pub version: u128,
    pub status: PlaybackStatus,
    pub queue: Vec<TrackInfo>,
    pub queue_cursor: u128,
    pub volume: u8,
    pub repeat: RepeatMode,
    pub shuffle: bool,
    pub mute: bool,
    pub crossfade: Option<CrossfadeConfig>,
    pub current_track: Option<TrackInfo>,
    pub time_pos: f64,
    pub duration: f64,
    pub sleep_timer: Option<u32>,
    pub eq_preset: EqPreset,
    pub eq_enabled: bool,
    pub reverb: ReverbConfig,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PlaybackStatus {
    Stopped,
    Playing,
    Paused,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RepeatMode {
    Off,
    One,
    All,
}

impl std::str::FromStr for RepeatMode {
    type Err = String;
    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "off" => Ok(Self::Off),
            "one" => Ok(Self::One),
            "all" => Ok(Self::All),
            _ => Err(format!("invalid repeat mode: {s}, expected off|one|all")),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
pub enum YTFilter {
    Song,
    Video,
    Playlist,
    Channel,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum UIMode {
    Normal,
    Filter,
    Command,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ThemeMode {
    Dark,
    Light,
}

/// 15-band graphic EQ band with ISO 1/3-octave center frequency.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct EqBand {
    pub frequency: f64,
    pub gain_db: f32,
    pub q: f64,
}

/// ISO 1/3-octave center frequencies for 15-band graphic EQ.
pub const EQ_FREQUENCIES: [f64; 15] = [
    25.0, 40.0, 63.0, 100.0, 160.0, 250.0, 400.0, 630.0,
    1000.0, 1600.0, 2500.0, 4000.0, 6300.0, 10000.0, 16000.0,
];
/// All available EQ preset names (excluding Custom).
pub const EQ_PRESETS: &[&str] = &[
    "flat", "pop", "rock", "jazz", "classical", "bass", "vocal", "electronic",
    "hiphop", "latin", "acoustic", "podcast", "dance", "headphones", "speaker",
];

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EqPreset {
    Flat,
    Pop,
    Rock,
    Jazz,
    Classical,
    Bass,
    Vocal,
    Electronic,
    HipHop,
    Latin,
    Acoustic,
    Podcast,
    Dance,
    Headphones,
    Speaker,
    Custom([f32; 15]),
}

/// Default Q value for bell filters (1/3-octave bandwidth).
pub const EQ_DEFAULT_Q: f64 = 1.414;

impl EqPreset {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Flat => "Flat",
            Self::Pop => "Pop",
            Self::Rock => "Rock",
            Self::Jazz => "Jazz",
            Self::Classical => "Classical",
            Self::Bass => "Bass Boost",
            Self::Vocal => "Vocal",
            Self::Electronic => "Electronic",
            Self::HipHop => "Hip-Hop",
            Self::Latin => "Latin",
            Self::Acoustic => "Acoustic",
            Self::Podcast => "Podcast",
            Self::Dance => "Dance",
            Self::Headphones => "Headphones",
            Self::Speaker => "Speaker",
            Self::Custom(_) => "Custom",
        }
    }

    /// Convert this preset to 15 per-band gain values in dB.
    /// All presets are capped at ±5 dB for musicality.
    pub fn to_gains(&self) -> [f32; 15] {
        match self {
            Self::Flat    => [0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0],
            Self::Pop     => [-1.0, 0.0, 1.0, 3.0, 4.0, 5.0, 4.0, 2.0, 0.0, -1.0, -1.0, 0.0, 2.0, 3.0, 2.0],
            Self::Rock    => [4.0, 4.0, 3.0, 1.0, -1.0, -2.0, -3.0, -1.0, 0.0, 1.0, 3.0, 4.0, 4.0, 4.0, 3.0],
            Self::Jazz    => [3.0, 2.0, 2.0, 1.0, 0.0, 1.0, 0.0, -1.0, -1.0, 0.0, 1.0, 2.0, 2.0, 2.0, 2.0],
            Self::Classical => [3.0, 3.0, 2.0, 2.0, 1.0, 0.0, 0.0, 0.0, -1.0, 0.0, 1.0, 2.0, 2.0, 2.0, 2.0],
            Self::Bass    => [5.0, 5.0, 4.0, 4.0, 3.0, 2.0, 1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0],
            Self::Vocal   => [-2.0, -2.0, -1.0, 0.0, 1.0, 2.0, 3.0, 4.0, 4.0, 3.0, 2.0, 1.0, 0.0, -1.0, -2.0],
            Self::Electronic => [4.0, 4.0, 3.0, 2.0, 0.0, -1.0, -1.0, 0.0, 0.0, 1.0, 2.0, 3.0, 4.0, 4.0, 3.0],
            Self::HipHop  => [4.0, 4.0, 3.0, 2.0, 1.0, 1.0, 0.0, -1.0, -1.0, 0.0, 1.0, 1.0, 2.0, 2.0, 2.0],
            Self::Latin   => [3.0, 3.0, 2.0, 1.0, 0.0, 0.0, -1.0, -1.0, -1.0, 0.0, 1.0, 2.0, 2.0, 3.0, 3.0],
            Self::Acoustic => [3.0, 3.0, 2.0, 2.0, 1.0, 0.0, 0.0, 1.0, 1.0, 1.0, 2.0, 2.0, 2.0, 2.0, 1.0],
            // Podcast: speech-optimized — low-cut rumble, narrow presence boost, de-ess, air
            Self::Podcast => [-4.0, -3.0, -2.0, -1.0, 0.0, 1.0, 2.0, 3.0, 4.0, 3.0, 1.0, 2.0, 3.0, 3.0, 2.0],
            Self::Dance   => [4.0, 4.0, 4.0, 3.0, 2.0, 0.0, 0.0, -1.0, -2.0, -1.0, 0.0, 1.0, 2.0, 3.0, 4.0],
            // Headphones: compensate closed-back — sub-bass warmth, presence dip, air shelf
            Self::Headphones => [3.0, 3.0, 2.0, 1.0, 0.0, -1.0, -2.0, -1.0, 0.0, 1.0, 2.0, 3.0, 4.0, 4.0, 3.0],
            // Speaker: desktop speakers — cut unplayable sub-bass, boost presence, add clarity
            Self::Speaker => [-3.0, -2.0, -1.0, 0.0, 1.0, 2.0, 3.0, 3.0, 3.0, 2.0, 1.0, 0.0, 1.0, 2.0, 1.0],
            Self::Custom(gains) => *gains,
        }
    }

    /// Convert to 15 `EqBand` structs with ISO frequencies.
    pub fn to_bands(&self) -> Vec<EqBand> {
        let gains = self.to_gains();
        EQ_FREQUENCIES.iter().zip(gains.iter()).map(|(freq, gain)| {
            EqBand { frequency: *freq, gain_db: *gain, q: EQ_DEFAULT_Q }
        }).collect()
    }
}

/// Image data parsed from metadata or API call
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Image {
    pub data: Vec<u8>,
    pub mime: String, // e.g 'image/jpeg' though we prefer png
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Tab {
    Library,
    Settings,
}

/// Persistent daemon state saved to disk across restarts.
///
/// Only contains user preferences and queue data — ephemeral session
/// state (status, current_track, time_pos, duration, sleep_timer) is
/// not persisted.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SavedState {
    pub queue: Vec<TrackInfo>,
    pub queue_cursor: u128,
    pub volume: u8,
    pub repeat: RepeatMode,
    pub shuffle: bool,
    pub mute: bool,
    pub crossfade: Option<CrossfadeConfig>,
    pub eq_preset: EqPreset,
    pub eq_enabled: bool,
    pub reverb: ReverbConfig,
}

impl SavedState {
    /// Capture persistent state from a `DaemonState`.
    pub fn from_state(state: &DaemonState) -> Self {
        Self {
            queue: state.queue.clone(),
            queue_cursor: state.queue_cursor,
            volume: state.volume,
            repeat: state.repeat,
            shuffle: state.shuffle,
            mute: state.mute,
            crossfade: state.crossfade.clone(),
            eq_preset: state.eq_preset,
            eq_enabled: state.eq_enabled,
            reverb: state.reverb.clone(),
        }
    }

    /// Apply this saved state to a `DaemonState`, restoring persisted fields.
    pub fn apply_to(&self, state: &mut DaemonState) {
        state.queue = self.queue.clone();
        state.queue_cursor = self.queue_cursor;
        state.volume = self.volume;
        state.repeat = self.repeat;
        state.shuffle = self.shuffle;
        state.mute = self.mute;
        state.crossfade = self.crossfade.clone();
        state.eq_preset = self.eq_preset;
        state.eq_enabled = self.eq_enabled;
        state.reverb = self.reverb.clone();
    }

    /// Save to a JSON file. Creates parent directories if needed.
    /// Writes atomically via a temp file + rename.
    pub fn save(&self, path: &std::path::Path) -> std::io::Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let tmp = path.with_extension("json.tmp");
        let json = serde_json::to_string_pretty(self)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;
        std::fs::write(&tmp, json)?;
        std::fs::rename(&tmp, path)?;
        Ok(())
    }

    /// Load from a JSON file. Returns `None` if the file doesn't exist
    /// or is corrupted.
    pub fn load(path: &std::path::Path) -> Option<Self> {
        let data = std::fs::read_to_string(path).ok()?;
        serde_json::from_str(&data).ok()
    }
}
