// Copyright (c) 2025 - present
// Author: prjctimg <prjctimg@outlook.com>
// Fail-point injection for testing error paths (debug-fail feature)
//
// This is free software released under the GPL-3.0 license.

use crate::Result;

#[cfg(feature = "debug-fail")]
use crate::CoreError;

/// Named points in the codebase where errors can be injected during tests.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FailPoint {
    SerializeEvent,
    DeserializeFrame,
    StateTransition,
    QueueAdvance,
    VolumeChange,
    CrossfadeApply,
}

#[cfg(feature = "debug-fail")]
thread_local! {
    static FAIL_POINTS: std::cell::RefCell<rustc_hash::FxHashMap<FailPoint, u32>> =
        const { std::cell::RefCell::new(rustc_hash::FxHashMap::default()) };
}

/// Check whether a fail point is tripped.
/// Returns `Err(CoreError::Daemon(…))` if armed, `Ok(())` otherwise.
/// In release builds (without `debug-fail` feature), this is a no-op.
#[inline(always)]
pub fn check(_fp: FailPoint) -> Result<()> {
    #[cfg(feature = "debug-fail")]
    {
        FAIL_POINTS.with(|fps| {
            let mut fps = fps.borrow_mut();
            if let Some(count) = fps.get_mut(&fp) {
                if *count == 0 {
                    return Err(CoreError::Daemon(format!("fail point tripped: {fp:?}")));
                }
                *count -= 1;
            }
            Ok(())
        })
    }
    #[cfg(not(feature = "debug-fail"))]
    {
        Ok(())
    }
}

/// Arm a fail point to fail the next `n` times it is checked.
#[cfg(feature = "debug-fail")]
pub fn arm(fp: FailPoint, n: u32) {
    FAIL_POINTS.with(|fps| {
        fps.borrow_mut().insert(fp, n);
    });
}

/// Disarm a fail point entirely.
#[cfg(feature = "debug-fail")]
pub fn disarm(fp: FailPoint) {
    FAIL_POINTS.with(|fps| {
        fps.borrow_mut().remove(&fp);
    });
}

/// Clear all armed fail points.
#[cfg(feature = "debug-fail")]
pub fn clear() {
    FAIL_POINTS.with(|fps| {
        fps.borrow_mut().clear();
    });
}
