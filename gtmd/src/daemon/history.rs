use super::*;

pub(crate) enum HistoryEntry {
    User(TrackInfo),
    Default { index: usize, track: TrackInfo },
}

/// Upper bound on the in-memory play history. Back-navigation only walks a
/// handful of recent tracks, so a long auto-advancing session must not retain
/// one full `TrackInfo` per track played (a second library copy over time).
pub(crate) const MAX_HISTORY: usize = 256;
/// Upper bound on the radio station rotation ring.
pub(crate) const MAX_RADIO_HISTORY: usize = 200;
