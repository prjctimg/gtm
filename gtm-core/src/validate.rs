// Copyright (c) 2026
// Author: prjctimg <prjctimg@outlook.com>
// Validated constructors and helper methods for core types
//
// This is free software released under the GPL-3.0 license.

use crate::global::{
    CrossfadeConfig, DaemonState, DynamicModeConfig, LoudnessMode, PlaybackStatus, RepeatMode,
    ReverbConfig, ScrobbleConfig,
};
use crate::track::TrackInfo;

impl CrossfadeConfig {
    /// Create a validated CrossfadeConfig.
    /// duration_secs is clamped to [0, 30].
    pub fn new(enabled: bool, duration_secs: u8) -> Self {
        Self {
            enabled,
            duration_secs: duration_secs.min(30),
            easing: crate::global::Easing::default(),
        }
    }
}

impl DaemonState {
    pub fn new() -> Self {
        Self {
            version: 0,
            status: PlaybackStatus::Stopped,
            queue: Vec::new(),
            queue_cursor: 0,
            default_list: Vec::new(),
            default_cursor: 0,
            fallback_disabled: false,
            volume: 100,
            repeat: RepeatMode::Off,
            shuffle: false,
            mute: false,
            crossfade: Some(crate::global::CrossfadeConfig {
                enabled: true,
                duration_secs: 6,
                easing: crate::global::Easing::Linear,
            }),
            current_track: None,
            time_pos: 0.0,
            duration: 0.0,
            sleep_timer: None,
            eq_preset: crate::global::EqPreset::Flat,
            eq_enabled: true,
            reverb: ReverbConfig::default(),
            loudness_mode: LoudnessMode::Off,
            pre_gain_db: 0.0,
            gapless: false,
            dynamic_mode: DynamicModeConfig::default(),
            scrobble: ScrobbleConfig::default(),
            lyrics_provider: "lrclib".to_string(),
            audio_levels: Vec::new(),
        }
    }
}

impl Default for DaemonState {
    fn default() -> Self {
        Self::new()
    }
}

impl TrackInfo {
    /// Returns true if the track has valid required fields.
    pub fn is_valid(&self) -> bool {
        !self.path.is_empty() && !self.hash.is_empty() && self.duration >= 0.0
    }

    /// Create a minimal TrackInfo from a file path and duration.
    /// Used when playing a file not in the library.
    pub fn from_path(path: &str, duration: f64) -> Self {
        Self {
            path: path.to_string(),
            title: std::path::Path::new(path)
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("Unknown")
                .to_string(),
            artist: "Unknown Artist".into(),
            album: "Unknown Album".into(),
            duration,
            ..Default::default()
        }
    }

    /// Format duration as "M:SS" or "H:MM:SS".
    pub fn duration_formatted(&self) -> String {
        let total = self.duration as u64;
        let hours = total / 3600;
        let mins = (total % 3600) / 60;
        let secs = total % 60;
        if hours > 0 {
            format!("{hours}:{mins:02}:{secs:02}")
        } else {
            format!("{mins}:{secs:02}")
        }
    }
}
