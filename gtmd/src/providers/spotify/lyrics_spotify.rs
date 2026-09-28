// Copyright (c) 2026
// Author: prjctimg <prjctimg@outlook.com>
// Spotify's own lyrics: the endpoint the official clients call
//
// This is free software released under the GPL-3.0 license.

//! Lyrics from Spotify itself, tried before lrclib.
//!
//! The documented Web API (`api.spotify.com/v1`) has no lyrics endpoint at
//! all. The only Spotify-sourced lyrics come from the partner endpoint the
//! desktop and mobile clients use, which authenticates with a *spclient*
//! token rather than a Web API bearer token:
//!
//! ```text
//! GET https://spclient.wg.spotify.com/color-lyrics/v2/track/{id}
//!     ?format=json&market=from_token
//! Authorization: Bearer <spclient token>
//! ```
//!
//! The token is the one the streaming session already obtained, so this costs
//! nothing while audio is playing — see
//! [`crate::providers::spotify::stream::StreamManager::spclient_token`]. Before
//! the first track plays there is no session and therefore no token, and the
//! caller falls back to lrclib.
//!
//! **This endpoint is undocumented.** It can be denied, throttled, or change
//! shape without notice, and Spotify gates it per account. So every failure
//! mode here returns `None` and nothing is ever surfaced as an error: lrclib is
//! a complete answer on its own, and a Spotify outage must not become a
//! missing-lyrics bug.

use std::time::Duration;

use librespot_core::spotify_uri::SpotifyUri;
use serde::Deserialize;
use tracing::debug;

use gtm::shared::track::{LrcData, LrcLine, TrackInfo};

/// How long Spotify gets before lrclib is tried. Kept short: this is the
/// preferred source, not the only one, and a slow partner endpoint should not
/// hold up lyrics that lrclib could deliver.
const SPOTIFY_TIMEOUT: Duration = Duration::from_secs(4);

const LYRICS_HOST: &str = "https://spclient.wg.spotify.com";
const LYRICS_PATH: &str = "/color-lyrics/v2/track";

/// The subset of the `color-lyrics` payload that carries anything useful.
///
/// Field names match the wire format, which mixes camelCase with the odd
/// PascalCase key. Everything not modelled here is ignored, so an added field
/// cannot break decoding — a `deny_unknown_fields` derive would make an
/// upstream addition look like a bug.
#[derive(Debug, Deserialize)]
struct ColorLyrics {
    #[serde(default)]
    lyrics: Option<LyricsBody>,
}

#[derive(Debug, Deserialize)]
struct LyricsBody {
    #[serde(default)]
    lines: Vec<LyricsLine>,
}

#[derive(Debug, Deserialize)]
struct LyricsLine {
    /// Milliseconds, carried as a *string* by this endpoint.
    #[serde(default, rename = "startTimeMs")]
    start_time_ms: Option<String>,
    #[serde(default)]
    words: Option<String>,
}

/// Lyrics for `track` from Spotify, or `None` for any reason at all.
///
/// `track.path` must be a `spotify:` URI. The id comes from it rather than
/// from a search, so this is an exact lookup with no ambiguity — the same
/// track id the stream is already playing.
pub(crate) async fn for_track(token: Option<String>, track: &TrackInfo) -> Option<LrcData> {
    let token = token?;
    let id = track_id(&track.path)?;

    let body = tokio::time::timeout(SPOTIFY_TIMEOUT, fetch(&token, &id))
        .await
        .ok()
        .flatten()?;

    let lines: Vec<LrcLine> = body
        .lyrics?
        .lines
        .into_iter()
        .filter_map(|l| {
            let text = l.words?.trim().to_string();
            if text.is_empty() {
                return None;
            }
            // A line with no start time is still worth showing, but the daemon
            // treats a negative timestamp as "unsynced", so an absent time
            // becomes 0.0 and the line renders rather than being dropped.
            let timestamp = l
                .start_time_ms
                .and_then(|ms| ms.trim().parse::<f64>().ok())
                .map(|ms| (ms / 1000.0).max(0.0))
                .unwrap_or(0.0);
            Some(LrcLine {
                timestamp,
                text,
                words: Vec::new(),
            })
        })
        .collect();

    if lines.is_empty() {
        debug!("spotify lyrics: `{}` has no lines", track.title);
        return None;
    }
    debug!(
        "spotify lyrics: `{}` returned {} lines",
        track.title,
        lines.len()
    );
    Some(LrcData {
        title: Some(track.title.clone()),
        artist: Some(track.artist.clone()),
        album: None,
        lines,
    })
}

/// The bare track id from a `spotify:` URI, or `None` if this is not a track.
fn track_id(path: &str) -> Option<String> {
    let uri = SpotifyUri::from_uri(path).ok()?;
    if uri.item_type() != "track" {
        return None;
    }
    uri.to_id().ok()
}

async fn fetch(token: &str, id: &str) -> Option<ColorLyrics> {
    let resp = reqwest::Client::new()
        .get(format!("{LYRICS_HOST}{LYRICS_PATH}/{id}"))
        .query(&[("format", "json"), ("market", "from_token")])
        .bearer_auth(token)
        .send()
        .await
        .ok()?;
    if !resp.status().is_success() {
        // Overwhelmingly 403 for an account or region without access, and 404
        // for a track that simply has no lyrics. Both are ordinary, so this is
        // a debug line rather than a warning.
        debug!("spotify lyrics: {} for {id}", resp.status());
        return None;
    }
    resp.json().await.ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn track(path: &str) -> TrackInfo {
        TrackInfo {
            path: path.to_string(),
            title: "Test".to_string(),
            artist: "Artist".to_string(),
            ..Default::default()
        }
    }

    #[test]
    fn a_track_uri_yields_its_id() {
        assert_eq!(
            track_id("spotify:track:4cOdK2wGLETKBW3PvgPWqT").as_deref(),
            Some("4cOdK2wGLETKBW3PvgPWqT")
        );
    }

    #[test]
    fn a_non_track_uri_yields_nothing() {
        assert_eq!(track_id("spotify:playlist:37i9dQZF1DXcBWIGoYBM5M"), None);
        assert_eq!(track_id("/home/me/song.flac"), None);
    }

    #[test]
    fn no_token_means_no_lookup() {
        // The common case before the first track plays: a playlist is browsed
        // with no session up, and this must not panic or reach the network.
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let got = rt.block_on(for_track(
            None,
            &track("spotify:track:4cOdK2wGLETKBW3PvgPWqT"),
        ));
        assert!(got.is_none());
    }

    #[test]
    fn start_times_are_millisecond_strings() {
        let body: ColorLyrics =
            serde_json::from_str(r#"{"lyrics":{"lines":[{"startTimeMs":"1500","words":"hello"},{"startTimeMs":"0","words":"first"}]}}"#)
                .unwrap();
        let lines = body.lyrics.unwrap().lines;
        assert_eq!(lines[0].start_time_ms.as_deref(), Some("1500"));
        assert_eq!(lines[1].start_time_ms.as_deref(), Some("0"));
    }

    #[test]
    fn unknown_fields_do_not_break_decoding() {
        // An upstream addition must not turn into a decode failure, which is
        // why the structs do not deny unknown fields.
        let ok = serde_json::from_str::<ColorLyrics>(
            r#"{"lyrics":{"lines":[],"colors":{"text":1}},"hasLyrics":true}"#,
        );
        assert!(ok.is_ok());
    }

    #[test]
    fn a_body_without_lyrics_decodes_to_nothing() {
        let body: ColorLyrics = serde_json::from_str(r#"{}"#).unwrap();
        assert!(body.lyrics.is_none());
    }
}
