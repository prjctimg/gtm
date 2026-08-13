// Copyright (c) 2025 - present
// Author: prjctimg <prjctimg@outlook.com>
// SPSC lock-free ring buffer and rodio Source adapter
//
// This is free software released under the GPL-3.0 license.

use std::num::{NonZeroU16, NonZeroU32};
use std::sync::atomic::{AtomicBool, AtomicU16, AtomicU32, AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use rodio::Source;

// ---------------------------------------------------------------------------
// Ring buffer constants
// ---------------------------------------------------------------------------

/// Sentinel value for "no seek request pending".
pub const NO_SEEK: u64 = u64::MAX;

/// Prebuffer threshold in samples (0.75 seconds at 44100 stereo).
pub const PREBUFFER_SAMPLES: usize = 44100 * 2 * 3 / 4; // 66150

// ---------------------------------------------------------------------------
// RingBufferInner — shared SPSC state
// ---------------------------------------------------------------------------

// SAFETY: RingBufferInner uses UnsafeCell for SPSC lock-free access.
// - Only one producer thread calls push()
// - Only one consumer thread calls pop()
// - No concurrent mutable access to the buffer data
unsafe impl Send for RingBufferInner {}
unsafe impl Sync for RingBufferInner {}

pub struct RingBufferInner {
    buf_ptr: *mut f32,
    buf_len: usize,
    capacity: usize,
    mask: usize,
    write_pos: AtomicUsize,
    read_pos: AtomicUsize,
    finished: AtomicBool,
}

impl RingBufferInner {
    pub fn new(capacity: usize) -> Self {
        let cap = capacity.max(1024);
        let mut buf = vec![0.0f32; cap];
        let ptr = buf.as_mut_ptr();
        std::mem::forget(buf);
        Self {
            buf_ptr: ptr,
            buf_len: cap,
            capacity: cap,
            mask: cap - 1,
            write_pos: AtomicUsize::new(0),
            read_pos: AtomicUsize::new(0),
            finished: AtomicBool::new(false),
        }
    }

    /// Number of samples available to read.
    pub fn available(&self) -> usize {
        let w = self.write_pos.load(Ordering::Acquire);
        let r = self.read_pos.load(Ordering::Relaxed);
        w.saturating_sub(r)
    }

    /// Space available to write.
    pub fn free_space(&self) -> usize {
        self.capacity - self.available()
    }

    /// Write a single sample. Returns false if buffer is full (sample dropped).
    /// SAFETY: Only the producer thread calls push(), so no data race with pop().
    pub fn push(&self, sample: f32) -> bool {
        let w = self.write_pos.load(Ordering::Relaxed);
        let r = self.read_pos.load(Ordering::Acquire);
        if w - r >= self.capacity {
            return false;
        }
        unsafe {
            self.buf_ptr.add(w & self.mask).write(sample);
        }
        self.write_pos.store(w + 1, Ordering::Release);
        true
    }

    /// Read a single sample. Returns None if empty.
    /// SAFETY: Only the consumer thread calls pop(), so no data race with push().
    pub fn pop(&self) -> Option<f32> {
        let r = self.read_pos.load(Ordering::Relaxed);
        let w = self.write_pos.load(Ordering::Acquire);
        if r >= w {
            return None;
        }
        let sample = unsafe { self.buf_ptr.add(r & self.mask).read() };
        self.read_pos.store(r + 1, Ordering::Release);
        Some(sample)
    }

    /// Reset buffer to empty state (used on seek).
    pub fn flush(&self) {
        self.read_pos.store(0, Ordering::Release);
        self.write_pos.store(0, Ordering::Release);
    }

    pub fn is_finished(&self) -> bool {
        self.finished.load(Ordering::Acquire)
    }

    pub fn set_finished(&self, val: bool) {
        self.finished.store(val, Ordering::Release);
    }
}

impl Drop for RingBufferInner {
    fn drop(&mut self) {
        unsafe {
            let _ = Vec::from_raw_parts(self.buf_ptr, self.buf_len, self.buf_len);
        }
    }
}

pub type SharedRingBuffer = Arc<RingBufferInner>;

// ---------------------------------------------------------------------------
// DecodeControl — flags shared between decode thread and mixer
// ---------------------------------------------------------------------------

pub struct DecodeControl {
    pub running: Arc<AtomicBool>,
    pub seek_request: Arc<AtomicU64>,
    pub seeking: Arc<AtomicBool>,
    pub ready: Arc<AtomicBool>,
    pub finished: Arc<AtomicBool>,
    pub sample_rate: AtomicU32,
    pub channels: AtomicU16,
}

impl Default for DecodeControl {
    fn default() -> Self {
        Self::new()
    }
}

impl DecodeControl {
    pub fn new() -> Self {
        Self {
            running: Arc::new(AtomicBool::new(true)),
            seek_request: Arc::new(AtomicU64::new(NO_SEEK)),
            seeking: Arc::new(AtomicBool::new(false)),
            ready: Arc::new(AtomicBool::new(false)),
            finished: Arc::new(AtomicBool::new(false)),
            sample_rate: AtomicU32::new(44100),
            channels: AtomicU16::new(2),
        }
    }

    pub fn signal_stop(&self) {
        self.running.store(false, Ordering::Release);
    }

    pub fn signal_seek(&self, position_secs: f64) {
        self.seeking.store(true, Ordering::Release);
        let bits = position_secs.to_bits();
        self.seek_request.store(bits, Ordering::Release);
    }

    pub fn consume_seek(&self) -> Option<f64> {
        let val = self.seek_request.swap(NO_SEEK, Ordering::Acquire);
        if val == NO_SEEK {
            None
        } else {
            Some(f64::from_bits(val))
        }
    }
}

// ---------------------------------------------------------------------------
// RingBufferSource — implements rodio::Source, reads from ring buffer
// ---------------------------------------------------------------------------

pub struct RingBufferSource {
    shared: SharedRingBuffer,
    control: Arc<DecodeControl>,
    channels: NonZeroU16,
    sample_rate: NonZeroU32,
    total_duration: Option<Duration>,
}

impl RingBufferSource {
    pub fn new(shared: SharedRingBuffer, control: Arc<DecodeControl>) -> Self {
        let sr = control.sample_rate.load(Ordering::Relaxed);
        let ch = control.channels.load(Ordering::Relaxed);
        Self {
            shared,
            control,
            channels: NonZeroU16::new(ch).unwrap_or(NonZeroU16::MIN),
            sample_rate: NonZeroU32::new(sr).unwrap_or(NonZeroU32::MIN),
            total_duration: None,
        }
    }

    pub fn with_duration(mut self, dur: Option<Duration>) -> Self {
        self.total_duration = dur;
        self
    }
}

impl Iterator for RingBufferSource {
    type Item = f32;

    fn next(&mut self) -> Option<f32> {
        loop {
            // If a seek is in progress, spin until it completes
            if self.control.seeking.load(Ordering::Acquire) {
                std::thread::yield_now();
                continue;
            }

            if let Some(sample) = self.shared.pop() {
                return Some(sample);
            }

            // Buffer empty — check if decode thread is done
            if self.shared.is_finished() {
                return None;
            }

            // Decode still running but buffer empty — brief yield to avoid hot spin
            std::thread::yield_now();
        }
    }
}

impl Source for RingBufferSource {
    fn current_span_len(&self) -> Option<usize> {
        None
    }

    fn channels(&self) -> NonZeroU16 {
        self.channels
    }

    fn sample_rate(&self) -> NonZeroU32 {
        self.sample_rate
    }

    fn total_duration(&self) -> Option<Duration> {
        self.total_duration
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ring_buffer_basic() {
        let rb = RingBufferInner::new(1024);
        assert_eq!(rb.available(), 0);
        assert!(rb.push(1.0));
        assert!(rb.push(2.0));
        assert_eq!(rb.available(), 2);
        assert_eq!(rb.pop(), Some(1.0));
        assert_eq!(rb.pop(), Some(2.0));
        assert_eq!(rb.pop(), None);
    }

    #[test]
    fn test_ring_buffer_full() {
        let rb = RingBufferInner::new(1024);
        for i in 0..1024 {
            assert!(rb.push(i as f32));
        }
        assert!(!rb.push(1024.0)); // full, should drop
        assert_eq!(rb.available(), 1024);
    }

    #[test]
    fn test_ring_buffer_flush() {
        let rb = RingBufferInner::new(1024);
        rb.push(1.0);
        rb.push(2.0);
        rb.flush();
        assert_eq!(rb.available(), 0);
        assert_eq!(rb.pop(), None);
    }

    #[test]
    fn test_ring_buffer_wraparound() {
        let rb = RingBufferInner::new(1024);
        for _ in 0..2 {
            for i in 0..1024 {
                rb.push(i as f32);
            }
            for _ in 0..1024 {
                rb.pop();
            }
        }
        assert_eq!(rb.available(), 0);
        assert!(rb.push(99.0));
        assert_eq!(rb.pop(), Some(99.0));
    }
}
