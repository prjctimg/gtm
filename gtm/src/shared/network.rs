// Copyright (c) 2026
// Author: prjctimg <prjctimg@outlook.com>
// Connectivity state shared between the daemon that probes and the client that
// draws it.
//
// Design notes:
// - Generic hosts only (no provider endpoints): a Spotify-side outage must
//   never read as "offline".
// - Four states, not a bool. "Never probed", "probing stopped" and "the last
//   answer is too old to stand behind" are three different things, and a bool
//   collapses all of them into two -- so the footer either lies or hides.
// - A verdict carries the time it was taken. Without that, a reading from an
//   hour ago is indistinguishable from a fresh one.
// - Staleness is computed at read time from `checked_at`, not stored as its own
//   state, so it still holds if nothing ever probes again.
//
// This is free software released under the GPL-3.0 license.

/// Whether the network is reachable, and how sure of it we are.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize, Default)]
pub enum Connectivity {
    /// Never probed.
    #[default]
    Unknown,
    /// A probe succeeded within the freshness window.
    Online,
    /// Consecutive probes have failed.
    Offline,
    /// The last probe answered, but not recently enough to still claim it.
    Stale,
}

/// How long a reading is worth repeating before it is called stale.
///
/// Three probe intervals: long enough that one slow answer is not a dead
/// network, short enough that a reader is not looking at an hour-old verdict
/// while the link is down.
pub const FRESHNESS: std::time::Duration = std::time::Duration::from_secs(90);

/// Successes required before the reading flips to online, and failures before it
/// flips to offline.
///
/// One each way makes a marginal link produce a
/// badge that changes every thirty seconds forever. Two stops the flapping
/// without making a real outage take twice as long to admit.
pub const ONLINE_STREAK: u32 = 2;
pub const OFFLINE_STREAK: u32 = 2;

/// A verdict and the evidence for it.
#[derive(Debug, Clone, Copy, Default, serde::Serialize, serde::Deserialize)]
pub struct ConnectivityReading {
    pub state: Connectivity,
    /// Unix seconds the probe ran. Zero means never.
    pub checked_at: i64,
}

impl ConnectivityReading {
    /// The state as of `now_secs`, demoting a reading that has aged out.
    ///
    /// Demotion lives here rather than in the daemon so it holds even when
    /// probing has stopped: a daemon that pauses probes because nobody is
    /// attached must not go on asserting a verdict once someone is.
    pub fn now(&self, now_secs: i64) -> Connectivity {
        if self.checked_at == 0 {
            return Connectivity::Unknown;
        }
        if now_secs - self.checked_at > FRESHNESS.as_secs() as i64 {
            return Connectivity::Stale;
        }
        self.state
    }
}

/// A verdict plus the run lengths behind it.
///
/// The streaks live here, not in the daemon's probe task, so the hysteresis is
/// one small testable type rather than counters threaded through a loop.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Verdict {
    pub state: Connectivity,
    pub on_streak: u32,
    pub off_streak: u32,
}

impl Verdict {
    /// A verdict in a settled state, with no streak behind it.
    pub fn settled(state: Connectivity) -> Self {
        Self {
            state,
            on_streak: 0,
            off_streak: 0,
        }
    }

    /// Fold one probe result in, returning the new state if the verdict moved.
    ///
    /// `None` means the drawing has not changed. The reading still moved --
    /// `checked_at` advanced -- but there is nothing new to look at, so the
    /// footer keeps its glyph and the event channel stays quiet.
    pub fn observe(&mut self, reachable: bool) -> Option<Connectivity> {
        if reachable {
            self.on_streak += 1;
            self.off_streak = 0;
        } else {
            self.off_streak += 1;
            self.on_streak = 0;
        }
        let next = if self.on_streak >= ONLINE_STREAK {
            Connectivity::Online
        } else if self.off_streak >= OFFLINE_STREAK {
            Connectivity::Offline
        } else {
            return None;
        };
        if next == self.state {
            return None;
        }
        self.state = next;
        Some(next)
    }
}

#[cfg(test)]
mod tests {
    use super::{
        Connectivity, ConnectivityReading, FRESHNESS, OFFLINE_STREAK, ONLINE_STREAK, Verdict,
    };

    #[test]
    fn a_single_success_does_not_call_it_online() {
        let mut v = Verdict::settled(Connectivity::Unknown);
        assert_eq!(v.observe(true), None);
        assert_eq!(v.observe(true), Some(Connectivity::Online));
    }

    #[test]
    fn a_single_failure_does_not_call_it_offline() {
        let mut v = Verdict::settled(Connectivity::Online);
        assert_eq!(v.observe(false), None);
        assert_eq!(v.observe(false), Some(Connectivity::Offline));
    }

    #[test]
    fn one_good_result_resets_the_bad_run() {
        let mut v = Verdict::settled(Connectivity::Online);
        assert_eq!(v.observe(false), None); // one failure: no verdict yet
        assert_eq!(v.observe(true), None); // one good result wipes the run
        // The two failures that had built up have to start again: the first
        // fresh failure is still one short, only the second flips.
        assert_eq!(v.observe(false), None);
        assert_eq!(v.observe(false), Some(Connectivity::Offline));
    }

    #[test]
    fn a_settled_verdict_repeats_without_further_events() {
        let mut v = Verdict::settled(Connectivity::Online);
        for _ in 0..(ONLINE_STREAK + 4) {
            assert_eq!(v.observe(true), None);
        }
        // A settled verdict is not a veto on change: two failures still flip it,
        // and from then on the failures that agree with the offline reading stay
        // quiet.
        assert_eq!(v.observe(false), None);
        assert_eq!(v.observe(false), Some(Connectivity::Offline));
        for _ in 0..(OFFLINE_STREAK + 4) {
            assert_eq!(v.observe(false), None);
        }
    }

    /// A client that attaches between probes must not be handed a verdict the
    /// daemon has stopped believing.
    #[test]
    fn an_aged_reading_goes_stale() {
        let now = 1_000_000i64;
        let fresh = ConnectivityReading {
            state: Connectivity::Online,
            checked_at: now,
        };
        assert_eq!(fresh.now(now), Connectivity::Online);
        assert_eq!(
            fresh.now(now + FRESHNESS.as_secs() as i64 + 1),
            Connectivity::Stale
        );
    }

    #[test]
    fn a_reading_never_taken_is_unknown() {
        assert_eq!(
            ConnectivityReading::default().now(1_000_000),
            Connectivity::Unknown
        );
    }
}
