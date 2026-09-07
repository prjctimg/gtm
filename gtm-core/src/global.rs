// Copyright (c) 2026
// Author: prjctimg <prjctimg@outlook.com>
// Centralized shared state namespace.
//
// All cross-module references to daemon/UI state should be routed through this
// module rather than reaching into the implementation modules directly. This
// keeps the single source of truth for state types in one place.
//
// This is free software released under the GPL-3.0 license.

pub use crate::state::{
    AudioSettings, CoreError, CrossfadeConfig, DaemonState, DynamicMode, DynamicModeConfig,
    EQ_DEFAULT_Q, EQ_FREQUENCIES, EQ_PRESETS, EqBand, EqPreset, Image, LoudnessMode, MAX_VOLUME,
    PlaybackStatus, RepeatMode, ReverbConfig, SavedState, ScrobbleConfig, ThemeMode, UIMode,
    YTFilter,
};
pub use crate::state::{volume_from_ratio, volume_ratio};
