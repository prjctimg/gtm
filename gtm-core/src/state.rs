// Copyright (c) 2026 - present
// Author: prjctimg <prjctimg@outlook.com>
// Core error type, primitive enums, daemon state, and UI types
//
// This is free software released under the GPL-3.0 license.

use serde::{Deserialize, Serialize};

use crate::spotify::SoloistStatus;
use crate::track::TrackInfo;

#[derive(Debug, thiserror::Error)]
pub enum CoreError {
    #[error("serialization error: {0}")]
    Serialization(#[from] serde_json::Error),
    #[error("i/o error: {0}")]
    Io(#[from] std::io::Error),
    #[error("daemon error: {0}")]
    Daemon(String),
    #[error("not connected")]
    NotConnected,
    #[error("timeout")]
    Timeout,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum Easing {
    #[default]
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum LoudnessMode {
    #[default]
    Off,
    Track,
    Album,
    Auto,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[derive(Default)]
pub enum DynamicMode {
    #[default]
    Off,
    On,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DynamicModeConfig {
    pub enabled: bool,
    pub min_queue_remaining: u32,
    pub max_history: u32,
    #[serde(default = "default_cooldown_weight")]
    pub cooldown_weight: f32,
}

fn default_cooldown_weight() -> f32 {
    0.1
}

fn default_master_volume() -> u8 {
    100
}

impl Default for DynamicModeConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            min_queue_remaining: 3,
            max_history: 50,
            cooldown_weight: 0.1,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ScrobbleConfig {
    pub enabled: bool,
    pub api_key: Option<String>,
    pub session_token: Option<String>,
    pub min_play_secs: Option<u32>,
    pub min_play_pct: Option<f32>,
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
        Self {
            enabled: false,
            room_size: 0.5,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DaemonState {
    pub version: u64,
    pub status: PlaybackStatus,
    /// User-added, one-time queue entries. The currently-playing entry is at
    /// index 0 and is removed once it finishes or Next is pressed.
    pub queue: Vec<TrackInfo>,
    pub queue_cursor: u64,
    /// Default playback list (whole library sorted by title, optionally
    /// shuffled). Loaded lazily when the user queue exhausts; entries persist
    /// in the view while `default_cursor` advances through them.
    pub default_list: Vec<TrackInfo>,
    pub default_cursor: usize,
    /// Set by Clear so the default list is not auto-built after the current
    /// track ends. Re-enabled by any explicit play or queue add.
    pub fallback_disabled: bool,
    pub volume: u8,
    pub master_volume: u8,
    pub repeat: RepeatMode,
    pub shuffle: bool,
    pub mute: bool,
    pub crossfade: Option<CrossfadeConfig>,
    pub current_track: Option<TrackInfo>,
    pub time_pos: f64,
    pub duration: f64,
    pub playback_speed: f64,
    pub sleep_timer: Option<u32>,
    pub eq_preset: EqPreset,
    pub eq_enabled: bool,
    pub reverb: ReverbConfig,
    pub loudness_mode: LoudnessMode,
    pub pre_gain_db: f32,
    pub gapless: bool,
    pub dynamic_mode: DynamicModeConfig,
    pub scrobble: ScrobbleConfig,
    /// Soloist playback bridge status (Spotify via the local `soloist` daemon).
    #[serde(default)]
    pub soloist: SoloistStatus,
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
    25.0, 40.0, 63.0, 100.0, 160.0, 250.0, 400.0, 630.0, 1000.0, 1600.0, 2500.0, 4000.0, 6300.0,
    10000.0, 16000.0,
];
/// All available EQ preset names (excluding Custom).
pub const EQ_PRESETS: &[&str] = &[
    "flat",
    "normal",
    "pop",
    "rock",
    "jazz",
    "classical",
    "bass",
    "vocal",
    "electronic",
    "hiphop",
    "latin",
    "acoustic",
    "podcast",
    "dance",
    "headphones",
    "speaker",
];

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EqPreset {
    Flat,
    Normal,
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
            Self::Normal => "Normal",
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
    ///
    /// Bands follow ISO 1/3-octave centers: 25, 40, 63, 100, 160, 250, 400,
    /// 630, 1000, 1600, 2500, 4000, 6300, 10000, 16000 Hz.
    ///
    /// Tuning rationale (research-based, all curves capped at ±4 dB):
    /// - Human hearing is most sensitive from ~2–5 kHz (ISO 226 equal-loudness
    ///   contours, ear-canal resonance ~3–4 kHz), so presence bands get modest
    ///   boosts and harshness-prone 4 kHz is treated with care.
    /// - 200–500 Hz is the "mud" zone where bass, kick, guitars, piano, and
    ///   vocals all accumulate; gentle cuts here clear a mix without thinning it.
    /// - Sub-bass (20–60 Hz) is felt more than heard and only exists on capable
    ///   speakers, so playback-agnostic presets cut it instead of boosting it.
    /// - Boosts are kept wide and adjacent transitions smooth to avoid the
    ///   resonances and comb-filter artifacts that adjacent ±5 dB steps cause.
    pub fn to_gains(&self) -> [f32; 15] {
        match self {
            Self::Flat => [
                0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0,
            ],
            // Normal: the signal exactly as the artist/mastering intended.
            Self::Normal => [
                0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0,
            ],
            // Pop: punchy kick/bass (60–120 Hz), neutral 250–630 so vocals sit
            // forward, gentle 1.6–2.5 kHz presence lift, air above 10 kHz.
            Self::Pop => [
                1.0, 2.0, 3.0, 3.0, 2.0, 1.0, -1.0, 0.0, 1.0, 2.0, 2.0, 1.0, 1.0, 2.0, 3.0,
            ],
            // Rock: moderate lows for kick/bass guitar, guitar body in the mids,
            // a wide 400 Hz mud cut and a gentle 4 kHz dip to tame cymbal
            // harshness, light top for snare crack.
            Self::Rock => [
                0.0, 0.0, 2.0, 3.0, 2.0, 1.0, -1.0, 0.0, 1.0, 2.0, 2.0, -1.0, 2.0, 2.0, 1.0,
            ],
            // Jazz: dynamics preserved: nearly flat, clears low-mid congestion,
            // lifts presence and top so brushes and double bass articulate.
            Self::Jazz => [
                0.0, 0.0, 1.0, 2.0, 1.0, 0.0, -1.0, 0.0, 0.0, 1.0, 1.0, 1.0, 1.0, 2.0, 1.0,
            ],
            // Classical: minimal touch: slight warmth, mostly a gentle
            // brightness tilt so strings and hall air shine through.
            Self::Classical => [
                0.0, 0.0, 1.0, 1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 2.0, 2.0, 2.0,
            ],
            // Bass Boost: sub/bass shelf with mids backed off to avoid mud and a
            // hint of treble for balance.
            Self::Bass => [
                4.0, 4.0, 3.0, 2.0, 1.0, 0.0, -1.0, -1.0, -1.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0,
            ],
            // Vocal: aggressive low cut, chest warmth at 250 Hz, a 400 Hz mud
            // dip, presence lift 1.6–4 kHz (voice fundamentals/harmonics) and a
            // de-ess tilt above 6.3 kHz.
            Self::Vocal => [
                -4.0, -3.0, -2.0, -1.0, 0.0, 1.0, -1.0, 0.0, 1.0, 2.0, 3.0, 2.0, 1.0, -1.0, -2.0,
            ],
            // Electronic: sub weight for synth bass/kicks, 200–400 Hz mud cut so
            // kicks don't smear into the mids, crisp hats via 6.3–16 kHz lift.
            Self::Electronic => [
                4.0, 4.0, 3.0, 1.0, 0.0, -2.0, -1.0, 0.0, 0.0, 0.0, 1.0, 1.0, 2.0, 3.0, 3.0,
            ],
            // Hip-Hop: target the 808 fundamental at 40–63 Hz, keep the 100–250
            // region clean, cut 400 Hz mud, lift 2.5 kHz so vocals stay above
            // the low end.
            Self::HipHop => [
                4.0, 4.0, 3.0, 1.0, 0.0, -2.0, -1.0, 0.0, 1.0, 1.0, 2.0, 1.0, 1.0, 1.0, 0.0,
            ],
            // Latin: percussive mids and horns: full 250–1000 Hz body for
            // congas/bongos, presence and a touch of air for clave and brass.
            Self::Latin => [
                0.0, 0.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 2.0, 2.0, 2.0, 1.0, 1.0,
            ],
            // Acoustic: small moves only: acoustic recordings punish heavy EQ;
            // a slight 250 Hz trim, gentle 1.6–4 kHz presence and top air.
            Self::Acoustic => [
                0.0, 0.0, 1.0, 1.0, 0.0, -1.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 1.0, 2.0, 2.0,
            ],
            // Podcast: speech-optimized: hard low cut for rumble/plosives,
            // 2.5 kHz intelligibility lift, narrow sibilance dip at 6.3 kHz.
            Self::Podcast => [
                -4.0, -4.0, -2.0, -1.0, 0.0, 1.0, 0.0, 0.0, 1.0, 2.0, 3.0, 2.0, -1.0, -1.0, -2.0,
            ],
            // Dance: club-oriented: strong bass, mids scooped to leave room for
            // bass and treble, sparkle above 10 kHz.
            Self::Dance => [
                3.0, 4.0, 4.0, 3.0, 1.0, -1.0, -1.0, -1.0, 0.0, 0.0, 1.0, 1.0, 2.0, 2.0, 2.0,
            ],
            // Headphones: mild smiley for consumer cans: a touch of low-end
            // warmth, mids kept present (not scooped), gentle treble lift.
            Self::Headphones => [
                2.0, 3.0, 2.0, 1.0, 0.0, -1.0, -1.0, 0.0, 0.0, 0.0, 1.0, 1.0, 2.0, 2.0, 2.0,
            ],
            // Speaker: small desktop speakers can't reproduce sub-bass: cut
            // 25–63 Hz (saves excursion, prevents distortion), keep the
            // presence band 2.5–4 kHz forward for clarity, minimal top.
            Self::Speaker => [
                -4.0, -4.0, -2.0, 0.0, 1.0, 1.0, 0.0, 0.0, 1.0, 1.0, 2.0, 2.0, 1.0, 1.0, 0.0,
            ],
            Self::Custom(gains) => *gains,
        }
    }

    /// Headroom trim in dB applied after the EQ so boosts can never push the
    /// output past full scale. Equals the negative of the largest positive
    /// band gain (0 dB for presets with no boosts). This is the standard
    /// "preamp" compensation used in professional EQ presets; it guarantees
    /// no clipping/artifacts regardless of the source's peak level.
    pub fn headroom_db(&self) -> f32 {
        let max_boost = self.to_gains().iter().copied().fold(0.0f32, f32::max);
        -max_boost
    }

    /// Convert to 15 `EqBand` structs with ISO frequencies.
    pub fn to_bands(&self) -> Vec<EqBand> {
        let gains = self.to_gains();
        EQ_FREQUENCIES
            .iter()
            .zip(gains.iter())
            .map(|(freq, gain)| EqBand {
                frequency: *freq,
                gain_db: *gain,
                q: EQ_DEFAULT_Q,
            })
            .collect()
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
/// Only contains user preferences and queue data: ephemeral session
/// state (status, current_track, time_pos, duration, sleep_timer) is
/// not persisted.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SavedState {
    pub queue: Vec<TrackInfo>,
    pub queue_cursor: u64,
    pub volume: u8,
    #[serde(default = "default_master_volume")]
    pub master_volume: u8,
    pub repeat: RepeatMode,
    pub shuffle: bool,
    pub mute: bool,
    pub crossfade: Option<CrossfadeConfig>,
    pub eq_preset: EqPreset,
    pub eq_enabled: bool,
    pub reverb: ReverbConfig,
    pub loudness_mode: LoudnessMode,
    pub pre_gain_db: f32,
    pub gapless: bool,
    pub dynamic_mode: DynamicModeConfig,
    pub scrobble: ScrobbleConfig,
    #[serde(default = "default_playback_speed")]
    pub playback_speed: f64,
}

fn default_playback_speed() -> f64 {
    1.0
}

impl SavedState {
    /// Capture persistent state from a `DaemonState`.
    pub fn from_state(state: &DaemonState) -> Self {
        Self {
            queue: state.queue.clone(),
            queue_cursor: state.queue_cursor,
            volume: state.volume,
            master_volume: state.master_volume,
            repeat: state.repeat,
            shuffle: state.shuffle,
            mute: state.mute,
            crossfade: state.crossfade.clone(),
            eq_preset: state.eq_preset,
            eq_enabled: state.eq_enabled,
            reverb: state.reverb.clone(),
            loudness_mode: state.loudness_mode,
            pre_gain_db: state.pre_gain_db,
            gapless: state.gapless,
            dynamic_mode: state.dynamic_mode.clone(),
            scrobble: state.scrobble.clone(),
            playback_speed: state.playback_speed,
        }
    }

    /// Apply this saved state to a `DaemonState`, restoring persisted fields.
    pub fn apply_to(&self, state: &mut DaemonState) {
        state.queue = self.queue.clone();
        state.queue_cursor = self.queue_cursor.min(state.queue.len() as u64);
        state.volume = self.volume;
        state.master_volume = self.master_volume;
        state.repeat = self.repeat;
        state.shuffle = self.shuffle;
        state.mute = self.mute;
        state.crossfade = self.crossfade.clone();
        state.eq_preset = self.eq_preset;
        state.eq_enabled = self.eq_enabled;
        state.reverb = self.reverb.clone();
        state.loudness_mode = self.loudness_mode;
        state.pre_gain_db = self.pre_gain_db;
        state.gapless = self.gapless;
        state.dynamic_mode = self.dynamic_mode.clone();
        state.scrobble = self.scrobble.clone();
        state.playback_speed = self.playback_speed;
    }

    /// Save to a JSON file. Creates parent directories if needed.
    /// Writes atomically via a temp file + rename.
    pub fn save(&self, path: &std::path::Path) -> std::io::Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let tmp = path.with_extension("json.tmp");
        let json = serde_json::to_string_pretty(self).map_err(std::io::Error::other)?;
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
