use super::*;

/// Tracks how long a track has actually been playing (from per-frame position
/// deltas) rather than trusting a naive `time_pos`, which drops credit when
/// the user seeks backwards or skips around. Used by the scrobble threshold.
#[derive(Default)]
pub(crate) struct ScrobbleTracker {
    pub(crate) key: String,
    pub(crate) last_pos: f64,
    pub(crate) listened: f64,
}

impl ScrobbleTracker {
    /// Re-anchor the tracker onto a freshly started track.
    pub(crate) fn start(&mut self, key: &str, pos: f64) {
        self.key = key.to_string();
        self.last_pos = pos;
        self.listened = 0.0;
    }

    /// Accumulate played time while the track is the current one.
    pub(crate) fn tick(&mut self, key: &str, pos: f64) {
        if self.key != key {
            return;
        }
        let delta = pos - self.last_pos;
        // Ignore rewind/large skips (a 30s+ single-frame delta is a seek, not
        // continuous listening) so seeking never inflates the counter.
        if delta > 0.0 && delta < 30.0 {
            self.listened += delta;
        }
        self.last_pos = pos;
    }

    /// Seconds reliably played for `key`; falls back to the raw timeline
    /// position when the tracker has not seen the track.
    pub(crate) fn listened_for(&self, key: &str, fallback: f64) -> f64 {
        if self.key == key {
            self.listened.max(fallback)
        } else {
            fallback
        }
    }
}

/// Identity key used to track which track is currently loved on Last.fm.
pub(crate) fn track_love_key(track: &TrackInfo) -> String {
    format!("{}|{}", track.artist, track.title)
}
