use super::*;

/// Capacity of the daemon→client event broadcast channel.
pub const EVENT_CHANNEL_CAPACITY: usize = 4096;

use std::time::Instant;

pub(crate) struct Counter {
    pub(crate) count: AtomicUsize,
    pub(crate) errors: AtomicUsize,
}

impl Counter {
    pub(crate) fn new() -> Self {
        Self {
            count: AtomicUsize::new(0),
            errors: AtomicUsize::new(0),
        }
    }
}

pub(crate) struct HealthTracker {
    pub(crate) start_time: Instant,
    pub(crate) audio_backend: String,
    pub(crate) scan: Counter,
    #[cfg(feature = "youtube")]
    pub(crate) yt: Counter,
    pub(crate) cover: Counter,
    pub(crate) lyrics: Counter,
}

impl HealthTracker {
    pub(crate) fn new(audio_backend: &str) -> Self {
        Self {
            start_time: Instant::now(),
            audio_backend: audio_backend.to_string(),
            scan: Counter::new(),
            #[cfg(feature = "youtube")]
            yt: Counter::new(),
            cover: Counter::new(),
            lyrics: Counter::new(),
        }
    }

    pub(crate) fn uptime_secs(&self) -> f64 {
        self.start_time.elapsed().as_secs_f64()
    }
}

pub(crate) struct SyncProgress {
    pub(crate) running: AtomicBool,
    pub(crate) kind: std::sync::Mutex<SyncKind>,
    pub(crate) synced: AtomicUsize,
    pub(crate) total: AtomicUsize,
}

impl Default for SyncProgress {
    fn default() -> Self {
        Self {
            running: AtomicBool::new(false),
            kind: std::sync::Mutex::new(SyncKind::Covers),
            synced: AtomicUsize::new(0),
            total: AtomicUsize::new(0),
        }
    }
}
