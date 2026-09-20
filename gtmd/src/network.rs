// Copyright (c) 2026
// Author: prjctimg <prjctimg@outlook.com>
// Generic internet-connectivity probes for the footer `Network` module.
//
// Design notes:
// - Generic hosts only (no provider endpoints): a Spotify-side outage must
//   never read as "offline". Probes run from first boot, independent of any
//   link state.
// - Every probe is bounded (no unbounded network I/O on a background task):
//   each TCP connect carries its own timeout and the whole pass is capped.
// - Callers poll on an interval and broadcast only on change; low-power mode
//   pauses probing entirely.
//
// This is free software released under the GPL-3.0 license.

use std::time::Duration;

/// Per-target TCP connect timeout. Keeps the picker/event loop responsive
/// even when the network is black-holed rather than refusing fast.
const CONNECT_TIMEOUT: Duration = Duration::from_millis(1500);

/// Generic connectivity targets, in probe order.
const TARGETS: &[&str] = &["1.1.1.1:443", "gstatic.com:443"];

async fn tcp_ok(target: &str) -> bool {
    match tokio::time::timeout(CONNECT_TIMEOUT, tokio::net::TcpStream::connect(target)).await {
        Ok(Ok(_)) => true,
        _ => false,
    }
}

/// Return true when any generic target is reachable. Bounded: at most
/// `TARGETS.len() * CONNECT_TIMEOUT` plus DNS resolution for the hostname
/// target (which itself rides on the same timeout via the connect future).
pub async fn probe_online() -> bool {
    for target in TARGETS {
        if tcp_ok(target).await {
            return true;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::TARGETS;

    #[test]
    fn generic_targets_only() {
        // Guard the "generic connectivity only" decision: no provider or
        // vendor-specific API hosts may sneak into the probe list.
        for t in TARGETS {
            let lower = t.to_ascii_lowercase();
            assert!(
                !lower.contains("spotify"),
                "probe target must stay generic: {t}"
            );
            assert!(
                !lower.contains("subsonic"),
                "probe target must stay generic: {t}"
            );
            assert!(
                !lower.contains("youtube"),
                "probe target must stay generic: {t}"
            );
            assert!(
                !lower.contains("googlevideo"),
                "probe target must stay generic: {t}"
            );
        }
        assert!(!TARGETS.is_empty());
    }
}
