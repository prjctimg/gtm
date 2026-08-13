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
use crate::state::{CoreError, CrossfadeConfig, DaemonState, PlaybackStatus, RepeatMode};
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

    pub fn set_crossfade(&mut self, enabled: bool, duration: u8) -> Result<()> {
        tripwire::check(FailPoint::CrossfadeApply)?;
        self.crossfade = if enabled {
            let easing = self
                .crossfade
                .as_ref()
                .map(|c| c.easing)
                .unwrap_or_default();
            Some(CrossfadeConfig {
                enabled: true,
                duration_secs: duration.min(30),
                easing,
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

    /// Advance the queue cursor by `dir` (±1). Returns the next track if valid.
    pub fn advance_queue(&mut self, dir: i32) -> Result<Option<&TrackInfo>> {
        tripwire::check(FailPoint::QueueAdvance)?;
        if self.queue.is_empty() {
            return Ok(None);
        }
        let len = self.queue.len() as i128;
        let new = self.queue_cursor as i128 + dir as i128;
        if new >= len {
            if matches!(self.repeat, RepeatMode::Off) {
                return Ok(None);
            }
            self.queue_cursor = 0;
        } else if new < 0 {
            if matches!(self.repeat, RepeatMode::Off) {
                return Ok(None);
            }
            self.queue_cursor = (len - 1) as u128;
        } else {
            self.queue_cursor = new as u128;
        }
        self.version += 1;
        #[cfg(debug_assertions)]
        {
            self.check_invariants();
        }
        Ok(self.queue.get(self.queue_cursor as usize))
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
            }
            DaemonEvent::EqEnabledChanged { enabled } => {
                self.eq_enabled = *enabled;
            }
            DaemonEvent::EqPresetChanged { preset } => {
                self.eq_preset = preset.clone();
            }
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
            self.queue.is_empty() || self.queue_cursor < self.queue.len() as u128,
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
                .map_or(true, |c| !c.enabled || c.duration_secs > 0),
            "crossfade enabled with duration_secs = 0"
        );
    }
}
