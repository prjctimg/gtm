// Copyright (c) 2025 - present
// Author: prjctimg <prjctimg@outlook.com>
// Daemon state machine: transitions, event application, and debug invariants
//
// This is free software released under the GPL-3.0 license.
//
// ```text
//  Playback state machine:
//
//  ┌──────────┐   play()    ┌─────────┐   pause()   ┌────────┐
//  │  Stopped  │───────────▶│ Playing │────────────▶│ Paused │
//  └──────────┘             └─────────┘             └────────┘
//       ▲                       │                       │
//       │                       │ stop()                │ stop()
//       │                       ▼                       ▼
//       └───────────────────────────────────────────────┘
//
//  All transitions increment `version` for optimistic concurrency.
//  `check_invariants()` (debug-only) asserts safety properties.
// ```

use crate::ipc::DaemonEvent;
use crate::state::{CoreError, CrossfadeConfig, DaemonState, PlaybackStatus};
use crate::track::TrackInfo;
use crate::tripwire::{self, FailPoint};
use crate::Result;

impl DaemonState {
    /// Transition to Playing with the given track.
    /// Allowed from: Stopped, Paused.
    pub fn play(&mut self, track: TrackInfo) -> Result<()> {
        tripwire::check(FailPoint::StateTransition)?;
        if self.status != PlaybackStatus::Stopped && self.status != PlaybackStatus::Paused {
            return Err(CoreError::Daemon(format!(
                "play() from invalid state: {:?}",
                self.status
            )));
        }
        self.status = PlaybackStatus::Playing;
        self.current_track = Some(track);
        self.version += 1;
        #[cfg(debug_assertions)]
        {
            self.check_invariants();
        }
        Ok(())
    }

    /// Transition to Paused.
    /// Allowed from: Playing.
    pub fn pause(&mut self) -> Result<()> {
        tripwire::check(FailPoint::StateTransition)?;
        if self.status != PlaybackStatus::Playing {
            return Err(CoreError::Daemon(format!(
                "pause() from invalid state: {:?}",
                self.status
            )));
        }
        self.status = PlaybackStatus::Paused;
        self.version += 1;
        #[cfg(debug_assertions)]
        {
            self.check_invariants();
        }
        Ok(())
    }

    /// Transition to Stopped.
    /// Allowed from: Playing, Paused.  No-op if already Stopped.
    pub fn stop(&mut self) -> Result<()> {
        tripwire::check(FailPoint::StateTransition)?;
        if self.status == PlaybackStatus::Stopped {
            return Ok(());
        }
        self.status = PlaybackStatus::Stopped;
        self.current_track = None;
        self.time_pos = 0.0;
        self.version += 1;
        #[cfg(debug_assertions)]
        {
            self.check_invariants();
        }
        Ok(())
    }

    /// Seek to absolute position in seconds. Clamped to [0, duration].
    pub fn seek(&mut self, pos: f64) -> Result<()> {
        tripwire::check(FailPoint::StateTransition)?;
        self.time_pos = pos.clamp(0.0, self.duration);
        self.version += 1;
        #[cfg(debug_assertions)]
        {
            self.check_invariants();
        }
        Ok(())
    }

    /// Set volume, clamped to [0, 100].
    pub fn set_volume(&mut self, vol: u8) -> Result<()> {
        tripwire::check(FailPoint::VolumeChange)?;
        self.volume = vol.min(100);
        self.mute = false;
        self.version += 1;
        #[cfg(debug_assertions)]
        {
            self.check_invariants();
        }
        Ok(())
    }

    pub fn toggle_shuffle(&mut self) -> Result<()> {
        tripwire::check(FailPoint::StateTransition)?;
        self.shuffle = !self.shuffle;
        self.version += 1;
        #[cfg(debug_assertions)]
        {
            self.check_invariants();
        }
        Ok(())
    }

    pub fn cycle_repeat(&mut self, mode: crate::state::RepeatMode) -> Result<()> {
        tripwire::check(FailPoint::StateTransition)?;
        self.repeat = mode;
        self.version += 1;
        #[cfg(debug_assertions)]
        {
            self.check_invariants();
        }
        Ok(())
    }

    pub fn toggle_mute(&mut self) -> Result<()> {
        tripwire::check(FailPoint::StateTransition)?;
        self.mute = !self.mute;
        self.version += 1;
        #[cfg(debug_assertions)]
        {
            self.check_invariants();
        }
        Ok(())
    }

    pub fn set_crossfade(
        &mut self,
        enabled: bool,
        duration: u8,
        easing: Option<crate::state::Easing>,
    ) -> Result<()> {
        tripwire::check(FailPoint::CrossfadeApply)?;
        self.crossfade = if enabled {
            let easing_val = easing.unwrap_or_else(|| {
                self.crossfade
                    .as_ref()
                    .map(|c| c.easing)
                    .unwrap_or_default()
            });
            Some(CrossfadeConfig {
                enabled: true,
                duration_secs: duration.min(30),
                easing: easing_val,
            })
        } else {
            None
        };
        self.version += 1;
        #[cfg(debug_assertions)]
        {
            self.check_invariants();
        }
        Ok(())
    }

    /// Set loudness mode (Off, Track, Album, Auto).
    pub fn set_loudness_mode(&mut self, mode: crate::state::LoudnessMode) -> Result<()> {
        self.loudness_mode = mode;
        self.version += 1;
        #[cfg(debug_assertions)]
        {
            self.check_invariants();
        }
        Ok(())
    }

    /// Set pre-gain in dB.
    pub fn set_pre_gain(&mut self, pre_gain_db: f32) -> Result<()> {
        self.pre_gain_db = pre_gain_db;
        self.version += 1;
        #[cfg(debug_assertions)]
        {
            self.check_invariants();
        }
        Ok(())
    }

    /// Set gapless playback.
    pub fn set_gapless(&mut self, enabled: bool) -> Result<()> {
        self.gapless = enabled;
        self.version += 1;
        #[cfg(debug_assertions)]
        {
            self.check_invariants();
        }
        Ok(())
    }

    /// Set dynamic mode configuration.
    pub fn set_dynamic_mode(
        &mut self,
        enabled: bool,
        min_queue_remaining: Option<u32>,
        max_history: Option<u32>,
    ) -> Result<()> {
        self.dynamic_mode.enabled = enabled;
        if let Some(min) = min_queue_remaining {
            self.dynamic_mode.min_queue_remaining = min;
        }
        if let Some(max) = max_history {
            self.dynamic_mode.max_history = max;
        }
        self.version += 1;
        #[cfg(debug_assertions)]
        {
            self.check_invariants();
        }
        Ok(())
    }

    /// Set scrobble configuration.
    pub fn set_scrobble(
        &mut self,
        enabled: bool,
        api_key: Option<String>,
        session_token: Option<String>,
        min_play_secs: Option<u32>,
        min_play_pct: Option<f32>,
    ) -> Result<()> {
        self.scrobble.enabled = enabled;
        self.scrobble.api_key = api_key;
        self.scrobble.session_token = session_token;
        self.scrobble.min_play_secs = min_play_secs;
        self.scrobble.min_play_pct = min_play_pct;
        self.version += 1;
        #[cfg(debug_assertions)]
        {
            self.check_invariants();
        }
        Ok(())
    }

    /// Consume the front of the one-time user queue.  The entry at the head
    /// is the currently-playing track; advancing removes it and returns the
    /// next pending user entry (now at the head), or None when the queue is
    /// exhausted.
    pub fn advance_queue(&mut self) -> Result<Option<&TrackInfo>> {
        tripwire::check(FailPoint::QueueAdvance)?;
        if self.queue.is_empty() {
            return Ok(None);
        }
        self.queue.remove(0);
        self.queue_cursor = 0;
        self.version += 1;
        #[cfg(debug_assertions)]
        {
            self.check_invariants();
        }
        Ok(self.queue.first())
    }

    /// Apply a DaemonEvent to mirror daemon state on the client side.
    pub fn apply_event(&mut self, event: &DaemonEvent) {
        match event {
            DaemonEvent::PlaybackStarted {
                track,
                time_pos,
                duration,
                ..
            } => {
                self.current_track = Some(track.clone());
                self.status = PlaybackStatus::Playing;
                self.time_pos = *time_pos;
                self.duration = *duration;
            }
            DaemonEvent::PlaybackPaused { time_pos } => {
                self.status = PlaybackStatus::Paused;
                self.time_pos = *time_pos;
            }
            DaemonEvent::PlaybackStopped => {
                self.status = PlaybackStatus::Stopped;
                self.current_track = None;
                self.time_pos = 0.0;
            }
            DaemonEvent::PositionChanged { time_pos } => {
                self.time_pos = *time_pos;
            }
            DaemonEvent::DurationChanged { duration } => {
                self.duration = *duration;
            }
            DaemonEvent::VolumeChanged { volume } => {
                self.volume = *volume;
            }
            DaemonEvent::QueueChanged { queue, cursor } => {
                self.queue = queue.clone();
                self.queue_cursor = *cursor;
            }
            DaemonEvent::QueueIndexChanged { index } => {
                self.queue_cursor = *index;
            }
            DaemonEvent::RepeatModeChanged { mode } => {
                self.repeat = *mode;
            }
            DaemonEvent::ShuffleChanged { enabled } => {
                self.shuffle = *enabled;
            }
            DaemonEvent::SleepTimerTick { remaining_secs } => {
                self.sleep_timer = Some(*remaining_secs);
            }
            DaemonEvent::TrackEnded => {
                self.status = PlaybackStatus::Stopped;
                self.current_track = None;
                self.time_pos = 0.0;
                // Note: do NOT clear the queue here — the daemon owns queue
                // consumption and mirrors every change via QueueChanged.  Wiping
                // it here previously erased pending entries on every track end.
            }
            DaemonEvent::EqEnabledChanged { enabled } => {
                self.eq_enabled = *enabled;
            }
            DaemonEvent::CrossfadeChanged {
                enabled,
                duration_secs,
                easing,
            } => {
                self.crossfade = if *enabled {
                    let easing_val = (*easing).unwrap_or_else(|| {
                        self.crossfade
                            .as_ref()
                            .map(|c| c.easing)
                            .unwrap_or_default()
                    });
                    Some(CrossfadeConfig {
                        enabled: true,
                        duration_secs: *duration_secs,
                        easing: easing_val,
                    })
                } else {
                    None
                };
            }
            DaemonEvent::ReverbChanged { enabled, room_size } => {
                self.reverb = crate::state::ReverbConfig {
                    enabled: *enabled,
                    room_size: *room_size,
                };
            }
            DaemonEvent::EqPresetChanged { preset } => {
                self.eq_preset = *preset;
            }
            DaemonEvent::LoudnessModeChanged { mode } => {
                self.loudness_mode = *mode;
            }
            DaemonEvent::PreGainChanged { pre_gain_db } => {
                self.pre_gain_db = *pre_gain_db;
            }
            DaemonEvent::GaplessChanged { enabled } => {
                self.gapless = *enabled;
            }
            DaemonEvent::DynamicModeChanged {
                enabled,
                min_queue_remaining,
                max_history,
            } => {
                self.dynamic_mode.enabled = *enabled;
                self.dynamic_mode.min_queue_remaining = *min_queue_remaining;
                self.dynamic_mode.max_history = *max_history;
            }
            DaemonEvent::ScrobbleConfigChanged { enabled } => {
                self.scrobble.enabled = *enabled;
            }
            DaemonEvent::LibraryOrganized { .. } => {
                // No state to update
            }
            DaemonEvent::LoudnessScanProgress { .. } => {}
            DaemonEvent::LoudnessScanDone { .. } => {}
            _ => {} // MetadataChanged, Custom — no state mirror field
        }
        self.version += 1;
        #[cfg(debug_assertions)]
        {
            self.check_invariants();
        }
    }

    /// Assert all internal invariants. Only compiled in debug/test builds.
    pub fn check_invariants(&self) {
        assert!(self.volume <= 100, "volume {} exceeds 100", self.volume);
        assert!(
            self.queue.is_empty() || self.queue_cursor < self.queue.len() as u64,
            "queue_cursor {} out of bounds for queue len {}",
            self.queue_cursor,
            self.queue.len()
        );
        assert!(self.time_pos >= 0.0, "negative time_pos {}", self.time_pos);
        assert!(
            self.time_pos <= self.duration || self.duration == 0.0,
            "time_pos {} exceeds duration {}",
            self.time_pos,
            self.duration
        );
        assert!(
            !(self.status == PlaybackStatus::Playing && self.current_track.is_none()),
            "status is Playing but current_track is None"
        );
        assert!(
            self.crossfade
                .as_ref()
                .is_none_or(|c| !c.enabled || c.duration_secs > 0),
            "crossfade enabled with duration_secs = 0"
        );
    }
}
