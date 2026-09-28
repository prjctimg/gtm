// Copyright (c) 2026
// Author: prjctimg <prjctimg@outlook.com>
// Spotify lyrics: lookups keyed off the playlist's own track metadata
//
// This is free software released under the GPL-3.0 license.

//! A synced playlist track has no library row and no file to read a sidecar
//! from, so its lyrics can only be found by searching. The daemon's lyrics
//! manager already caches by artist and title on disk, so fetching a track's
//! lyrics once means every later visit — and every later session — is a cache
//! hit. This is the "fetch on demand" half: the playlist itself is never
//! scanned, only whichever track the user is actually looking at.
//!
//! lrclib is the only source. Spotify's own colour-lyrics were tried first, via
//! the undocumented `spclient.wg.spotify.com` partner endpoint, and removed: it
//! needs a live streaming session for its token, so lyrics were unreachable
//! until a track had played, it is gated per account with no way to tell "denied"
//! from "no lyrics for this track", and its timings never agreed with the audio
//! closely enough to be worth 350 lines and a session dependency. What was
//! actually wrong with lyrics was in the lrclib parser the whole time — see
//! `lrclib::parse_lrclib_response`.

use std::time::Duration;

use gtm::shared::ipc::DaemonRes;
use gtm::shared::track::TrackInfo;
use tracing::debug;

use crate::daemon::DaemonInner;
/// How long one track's lookup may take before the UI gives up on it.
const LOOKUP_TIMEOUT: Duration = Duration::from_secs(10);

/// Lyrics for one synced playlist track, resolved from the metadata the sync
/// already cached rather than from a library row.
///
/// The disk cache is consulted first, so a track's lyrics are fetched once
/// whatever the source. Then the lrclib route runs, and it is the same path a
/// local file takes: cache, then lrclib's exact `/api/get` keyed on artist,
/// track, album *and* duration, then progressively looser fallbacks. The
/// previous [`LyricsManager::search`] sent only `/api/search?q=<artist>
/// <title>` — no album, no duration, no cache — so a synced playlist row was
/// the one track kind that could not get a precise match even when the sync
/// held every field needed to make one.
///
/// `None` when the track carries no artist or title to search on, or when lrclib
/// had nothing. Never errors: a playlist row without lyrics is a normal
/// outcome, not a failure worth surfacing.
pub(crate) async fn for_track(inner: &DaemonInner, track: &TrackInfo) -> DaemonRes {
    if query_of(track).is_none() {
        return DaemonRes::Lyrics { lyrics: None };
    }
    let Some(manager) = inner.lyrics_manager().await else {
        return DaemonRes::Lyrics { lyrics: None };
    };

    // The cache is consulted here rather than only inside `get_lyrics`, so a
    // track that already has synced lyrics on disk is answered without a
    // network round trip on every visit.
    if let Some(lrc) = manager.cached_synced(track) {
        debug!("spotify lyrics for `{}`: cache hit", track.title);
        return DaemonRes::Lyrics { lyrics: Some(lrc) };
    }

    let lyrics = tokio::time::timeout(LOOKUP_TIMEOUT, manager.get_lyrics(track))
        .await
        .ok()
        .flatten();
    debug!("spotify lyrics for `{}`: {}", track.title, lyrics.is_some());
    DaemonRes::Lyrics { lyrics }
}

/// The `(artist, title)` a lookup can be made with, or `None` when either half
/// is missing — a search with a blank field matches nothing but the wrong thing.
fn query_of(track: &TrackInfo) -> Option<(String, String)> {
    match (track.artist.trim(), track.title.trim()) {
        ("", _) | (_, "") => None,
        (artist, title) => Some((artist.to_string(), title.to_string())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn track(artist: &str, title: &str) -> TrackInfo {
        TrackInfo {
            path: "spotify:track:4cOdK2wGLETKBW3PvgPWqT".to_string(),
            artist: artist.to_string(),
            title: title.to_string(),
            ..Default::default()
        }
    }

    #[test]
    fn a_queued_track_searches_its_own_metadata() {
        assert_eq!(
            query_of(&track("Daft Punk", "Get Lucky")),
            Some(("Daft Punk".to_string(), "Get Lucky".to_string()))
        );
    }

    /// A blank half cannot be searched on: LRCLIB would answer with an unrelated
    /// track rather than nothing, which is worse than a clean miss.
    #[test]
    fn a_blank_half_is_not_searchable() {
        assert_eq!(query_of(&track("", "Get Lucky")), None);
        assert_eq!(query_of(&track("Daft Punk", "  ")), None);
    }
}
