// Copyright (c) 2026
// Author: prjctimg <prjctimg@outlook.com>
// Mono downmix source wrapper
//
// This is free software released under the GPL-3.0 license.

use std::num::{NonZeroU16, NonZeroU32};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use rodio::Source;

/// Wraps any `Source<Item = f32>` and, while `mono` is set, downmixes each
/// interleaved frame into a single summed channel replicated to every output
/// channel. Keeping the channel count stable (rather than emitting a true
/// mono stream) means the flag can be toggled **live** mid-track without the
/// downstream mixer ever seeing a channel-count change.
pub struct MonoSource<B> {
    inner: B,
    channels: NonZeroU16,
    mono: Arc<AtomicBool>,
    /// The replicated mono frame pending delivery, if mid-emit.
    emit: Vec<f32>,
    emit_pos: usize,
}

impl<B> MonoSource<B>
where
    B: Source<Item = f32>,
{
    pub fn new(inner: B, mono: Arc<AtomicBool>) -> Self {
        let channels = inner.channels();
        Self {
            inner,
            channels,
            mono,
            emit: Vec::new(),
            emit_pos: 0,
        }
    }
}

impl<B> Iterator for MonoSource<B>
where
    B: Source<Item = f32>,
{
    type Item = f32;

    fn next(&mut self) -> Option<f32> {
        if self.emit_pos < self.emit.len() {
            let s = self.emit[self.emit_pos];
            self.emit_pos += 1;
            return Some(s);
        }
        if !self.mono.load(Ordering::Relaxed) {
            return self.inner.next();
        }

        let ch = self.channels.get() as usize;
        let mut sum = 0.0f32;
        for _ in 0..ch {
            match self.inner.next() {
                Some(s) => sum += s,
                None => return None,
            }
        }
        let mixed = sum / ch as f32;
        self.emit = vec![mixed; ch];
        self.emit_pos = 1;
        Some(mixed)
    }
}

impl<B> Source for MonoSource<B>
where
    B: Source<Item = f32>,
{
    fn current_span_len(&self) -> Option<usize> {
        self.inner.current_span_len()
    }

    fn channels(&self) -> NonZeroU16 {
        self.channels
    }

    fn sample_rate(&self) -> NonZeroU32 {
        self.inner.sample_rate()
    }

    fn total_duration(&self) -> Option<Duration> {
        self.inner.total_duration()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicBool;

    struct FlatSource {
        samples: Vec<f32>,
        pos: usize,
    }

    impl Iterator for FlatSource {
        type Item = f32;

        fn next(&mut self) -> Option<f32> {
            let s = self.samples.get(self.pos)?;
            self.pos += 1;
            Some(*s)
        }
    }

    impl Source for FlatSource {
        fn current_span_len(&self) -> Option<usize> {
            None
        }

        fn channels(&self) -> NonZeroU16 {
            NonZeroU16::new(2).unwrap()
        }

        fn sample_rate(&self) -> NonZeroU32 {
            NonZeroU32::new(44100).unwrap()
        }

        fn total_duration(&self) -> Option<Duration> {
            None
        }
    }

    #[test]
    fn mono_off_passes_through() {
        let inner = FlatSource {
            samples: vec![1.0, 2.0, 3.0, 4.0],
            pos: 0,
        };
        let mono = Arc::new(AtomicBool::new(false));
        let mut src = MonoSource::new(inner, mono);
        assert_eq!(src.next(), Some(1.0));
        assert_eq!(src.next(), Some(2.0));
        assert_eq!(src.next(), Some(3.0));
    }

    #[test]
    fn mono_on_averages_and_replicates() {
        let inner = FlatSource {
            samples: vec![1.0, 3.0, 0.0, 4.0],
            pos: 0,
        };
        let mono = Arc::new(AtomicBool::new(true));
        let mut src = MonoSource::new(inner, mono);
        assert_eq!(src.next(), Some(2.0));
        assert_eq!(src.next(), Some(2.0));
        assert_eq!(src.next(), Some(2.0));
        assert_eq!(src.next(), Some(2.0));
        assert_eq!(src.next(), None);
    }

    #[test]
    fn mono_toggle_mid_stream() {
        let inner = FlatSource {
            samples: vec![1.0, 3.0, 9.0, 1.0, 5.0, 5.0],
            pos: 0,
        };
        let flag = Arc::new(AtomicBool::new(true));
        let mut src = MonoSource::new(inner, flag.clone());
        assert_eq!(src.next(), Some(2.0)); // 1.0, 3.0 -> 2.0
        assert_eq!(src.next(), Some(2.0));
        flag.store(false, Ordering::Relaxed);
        assert_eq!(src.next(), Some(9.0));
        assert_eq!(src.next(), Some(1.0));
        flag.store(true, Ordering::Relaxed);
        assert_eq!(src.next(), Some(5.0));
        assert_eq!(src.next(), Some(5.0));
        assert_eq!(src.next(), None);
    }
}
