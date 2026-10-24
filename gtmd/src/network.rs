// Copyright (c) 2026
// Author: prjctimg <prjctimg@outlook.com>
// Generic internet-connectivity probes for the footer `Network` module.
//
// Design notes:
// - Generic hosts only (no provider endpoints): a Spotify-side outage must
//   never read as "offline". Probes run from first boot, independent of any
//   link state.
// - Every probe is bounded (no unbounded network I/O on a background task):
//   each attempt carries its own timeout and the whole pass is capped.
// - Through the same `reqwest` client every other request in the daemon uses,
//   so the answer describes the network gtm actually has. A raw TCP connect
//   does not honour `HTTP(S)_PROXY`, which made the footer read "offline" on a
//   machine behind a corporate proxy while every Spotify and Last.fm request
//   was succeeding.
// - A real response, not a completed handshake: a captive portal completes the
//   TCP and TLS handshakes and then serves an interception page, and a connect
//   test would have called that online.
// - Callers poll on an interval and broadcast only when the verdict moves;
//   probing is skipped entirely while no client is attached.
//
// This is free software released under the GPL-3.0 license.

use std::sync::OnceLock;
use std::time::Duration;

pub use gtm::shared::network::{
    Connectivity, ConnectivityReading, FRESHNESS, OFFLINE_STREAK, ONLINE_STREAK, Verdict,
};

/// Per-target request timeout. Keeps the picker/event loop responsive even when
/// the network is black-holed rather than refusing fast.
const CONNECT_TIMEOUT: Duration = Duration::from_millis(1000);

/// Generic connectivity targets, in probe order.
///
/// All are CDN-fronted static paths that answer a HEAD cheaply and belong to
/// nobody whose outage should read as the user's link being down.
const TARGETS: &[&str] = &[
    "https://connectivitycheck.gstatic.com/generate_204",
    "https://www.msftconnecttest.com/connecttest.txt",
];

/// The probe client, built once.
///
/// Built per probe it would open a fresh TLS session to two hosts every thirty
/// seconds for the life of the daemon, and at this size the handshake is the
/// whole cost of the request.
fn client() -> &'static reqwest::Client {
    static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
    CLIENT.get_or_init(|| {
        reqwest::Client::builder()
            .user_agent(concat!("gtm/", env!("CARGO_PKG_VERSION")))
            .redirect(reqwest::redirect::Policy::limited(2))
            .build()
            .unwrap_or_default()
    })
}

async fn request_ok(url: &str) -> bool {
    matches!(
        tokio::time::timeout(CONNECT_TIMEOUT, client().head(url).send()).await,
        Ok(Ok(r)) if r.status().is_success() || r.status().is_redirection()
    )
}

/// Return true when any generic target answers a real request.
///
/// Bounded: at most `TARGETS.len() * CONNECT_TIMEOUT`.
pub async fn probe_online() -> bool {
    for target in TARGETS {
        if request_ok(target).await {
            return true;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::TARGETS;

    /// Guard the "generic connectivity only" decision: no provider or
    /// vendor-specific API host may sneak into the probe list.
    #[test]
    fn generic_targets_only() {
        for t in TARGETS {
            let lower = t.to_ascii_lowercase();
            assert!(
                !lower.contains("spotify"),
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

    /// Every target has to be asked the way gtm asks everything else, or the
    /// verdict describes a network the daemon does not use. A `host:port` here
    /// is the regression that made the footer disagree with every request the
    /// daemon actually made.
    #[test]
    fn targets_are_requests_not_sockets() {
        for t in TARGETS {
            assert!(t.starts_with("https://"), "target is not a request: {t}");
        }
    }
}
