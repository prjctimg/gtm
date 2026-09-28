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
    /// Shift applied by Spotify to every line, in milliseconds. Non-zero when
    /// the catalogue's own timings sit early or late against the audio, and it
    /// is the difference between lyrics that track and lyrics that are
    /// uniformly wrong. Ignored, the whole track is out by a fixed amount that
    /// looks like bad timing rather than a dropped field.
    #[serde(default, rename = "syncOffset")]
    sync_offset: Option<Millis>,
    #[serde(default)]
    lines: Vec<LyricsLine>,
}

#[derive(Debug, Deserialize)]
struct LyricsLine {
    /// Milliseconds since the track starts. This endpoint has been observed
    /// sending this as both a JSON string and a bare number, so it is decoded
    /// permissively: a `String` here would fail the *entire* body on a numeric
    /// value, silently discarding perfectly good lyrics with no way to tell
    /// that happened.
    #[serde(default, rename = "startTimeMs", deserialize_with = "de_millis")]
    start_time_ms: Option<Millis>,
    #[serde(default)]
    words: Option<String>,
}

/// Milliseconds, tolerant of the string-or-number ambiguity in the payload.
type Millis = f64;

/// Accept `"1500"`, `1500`, `1500.0` and `null`, and reject nothing else.
fn de_millis<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Option<Millis>, D::Error> {
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Raw {
        Num(f64),
        Str(String),
        Null,
    }
    Ok(match Option::<Raw>::deserialize(d)? {
        None | Some(Raw::Null) => None,
        Some(Raw::Num(n)) => Some(n),
        // A non-numeric string decodes to `None` rather than failing the body,
        // for the same reason: one odd line must not discard the rest.
        Some(Raw::Str(s)) => s.trim().parse::<f64>().ok(),
    })
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

    let body = body.lyrics?;
    let offset = body.sync_offset.unwrap_or(0.0) / 1000.0;
    let lines: Vec<LrcLine> = body
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
            //
            // `syncOffset` is added in seconds here rather than folded into
            // the millisecond maths above, so the two are kept visibly
            // separate: the offset shifts *every* line, whereas a missing time
            // is a per-line fallback to the start of the track.
            let timestamp = l
                .start_time_ms
                .map(|ms| ms / 1000.0 + offset)
                .unwrap_or(0.0)
                .max(0.0);
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
    // Read as text first so a decode failure can be reported with the shape
    // that caused it. This endpoint is undocumented, so a silent `None` here is
    // indistinguishable from "no lyrics for this track" — which is how the
    // original string-typed `startTimeMs` cost a whole feature: it decoded to
    // nothing, fell through to lrclib, and looked like the source was empty.
    let raw = resp.text().await.ok()?;
    match serde_json::from_str::<ColorLyrics>(&raw) {
        Ok(parsed) => Some(parsed),
        Err(e) => {
            debug!(
                "spotify lyrics: could not decode {id} ({e}); body starts {:?}",
                raw.chars().take(SHAPE_SAMPLE).collect::<String>()
            );
            None
        }
    }
}

/// How much of an undecodable body to log. Enough to show the field names and
/// value types, short enough not to dump a whole track's lyrics into the log.
const SHAPE_SAMPLE: usize = 400;

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
    fn start_times_decode_from_either_a_string_or_a_number() {
        // This is the ambiguity that silently killed the feature: a
        // `String`-typed field makes a *numeric* value fail the entire body, so
        // every line was discarded and the lookup fell through to lrclib with
        // nothing to show for it.
        for raw in [
            r#"{"startTimeMs":"1500","words":"a"}"#,
            r#"{"startTimeMs":1500,"words":"a"}"#,
        ] {
            let line: LyricsLine = serde_json::from_str(raw).unwrap();
            assert_eq!(line.start_time_ms, Some(1500.0), "failed for {raw}");
            assert_eq!(line.words.as_deref(), Some("a"));
        }
    }

    #[test]
    fn a_fractional_or_absent_start_time_is_tolerated() {
        let line: LyricsLine =
            serde_json::from_str(r#"{"startTimeMs":1500.5,"words":"a"}"#).unwrap();
        assert_eq!(line.start_time_ms, Some(1500.5));
        let missing: LyricsLine = serde_json::from_str(r#"{"words":"a"}"#).unwrap();
        assert_eq!(missing.start_time_ms, None);
        let null: LyricsLine = serde_json::from_str(r#"{"startTimeMs":null,"words":"a"}"#).unwrap();
        assert_eq!(null.start_time_ms, None);
    }

    #[test]
    fn one_unparsable_time_does_not_discard_the_others() {
        // Same reasoning one level down: a single odd value must not cost the
        // whole track's lyrics.
        let body: ColorLyrics = serde_json::from_str(
            r#"{"lyrics":{"lines":[{"startTimeMs":"soon","words":"a"},{"startTimeMs":"2000","words":"b"}]}}"#,
        )
        .unwrap();
        let lines = body.lyrics.unwrap().lines;
        assert_eq!(lines[0].start_time_ms, None);
        assert_eq!(lines[1].start_time_ms, Some(2000.0));
    }

    #[test]
    fn sync_offset_decodes_and_defaults_to_zero() {
        let shifted: ColorLyrics =
            serde_json::from_str(r#"{"lyrics":{"syncOffset":-250,"lines":[]}}"#).unwrap();
        assert_eq!(shifted.lyrics.unwrap().sync_offset, Some(-250.0));
        let plain: ColorLyrics = serde_json::from_str(r#"{"lyrics":{"lines":[]}}"#).unwrap();
        assert_eq!(plain.lyrics.unwrap().sync_offset, None);
    }

    #[test]
    fn sync_offset_shifts_every_line() {
        // A non-zero offset is the difference between lyrics that track the
        // audio and lyrics that are uniformly wrong, in a way that looks like
        // bad timing rather than a field that was never read.
        let mut body: ColorLyrics = serde_json::from_str(
            r#"{"lyrics":{"syncOffset":500,"lines":[{"startTimeMs":"1000","words":"a"}]}}"#,
        )
        .unwrap();
        let body = body.lyrics.take().unwrap();
        let offset = body.sync_offset.unwrap_or(0.0) / 1000.0;
        let line = body.lines.into_iter().next().unwrap();
        let at = line.start_time_ms.unwrap() / 1000.0 + offset;
        assert!((at - 1.5).abs() < f64::EPSILON, "expected 1.5s, got {at}");
    }

    #[test]
    fn a_line_with_no_text_is_skipped_but_its_neighbour_is_kept() {
        // Instrumental breaks come through as null or empty `words`; the
        // surrounding lines must survive them.
        let body: ColorLyrics = serde_json::from_str(
            r#"{"lyrics":{"lines":[{"startTimeMs":"1000","words":"a"},{"startTimeMs":"2000","words":null},{"startTimeMs":"3000","words":"  "},{"startTimeMs":"4000","words":"d"}]}}"#,
        )
        .unwrap();
        let lines = body.lyrics.unwrap().lines;
        assert_eq!(lines.len(), 4);
        assert_eq!(lines[1].words, None);
        assert_eq!(lines[3].words.as_deref(), Some("d"));
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
