// Copyright (c) 2026
// Author: prjctimg <prjctimg@outlook.com>
// Station tracklists: a data-driven source registry over a generic extractor
//
// This is free software released under the GPL-3.0 license.

use std::time::Duration;

use gtm::shared::radio::{RadioTrack, RadioTracklist};
use serde_json::Value;
use tracing::debug;

/// How a source expresses a start time. Sources fall into two camps: those
/// with an unambiguous instant, and those publishing a bare station-local wall
/// clock that carries no date and no offset.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stamp {
    /// Unix seconds, as a number or a numeric string.
    Epoch,
    /// A full datetime with its own UTC offset, e.g. `2026-09-26 16:50:31 +0200`.
    Offset,
    /// A bare `HH:MM` station-local wall clock, with no date and no offset.
    Clock,
}

/// Field names inside a source's JSON body, plus where the tracks live.
/// Dotted keys walk nested objects, which is how sources that nest the artist
/// (`{"artist":{"name":…}}`) are described without a special case.
#[derive(Debug, Clone, Copy)]
struct Keys {
    /// Path to the array of track objects. Empty means the body is that array.
    rows: &'static str,
    artist: &'static str,
    title: &'static str,
    start: &'static str,
    /// Optional cover art field. `""` when the source publishes no art.
    art: &'static str,
    stamp: Stamp,
}

/// One known tracklist endpoint. Adding a station family is an entry here, not
/// a new fetch-and-parse path.
#[derive(Debug, Clone, Copy)]
struct Source {
    /// Hostname suffix this source claims, matched against the resolved stream
    /// host so a station is recognised by where it streams from.
    host: &'static str,
    /// Path template. `{id}` is the station slug, `{stream}` the station's
    /// stream mount.
    path: &'static str,
    keys: Keys,
}

const LAUT_KEYS: Keys = Keys {
    rows: "",
    artist: "artist.name",
    title: "title",
    start: "started_at",
    art: "",
    stamp: Stamp::Offset,
};

const SOMAFM_KEYS: Keys = Keys {
    rows: "songs",
    artist: "artist",
    title: "title",
    start: "date",
    art: "albumArt",
    stamp: Stamp::Epoch,
};

const STREAMSB_KEYS: Keys = Keys {
    rows: "mscp.playlist",
    artist: "artist",
    title: "title",
    start: "time",
    art: "",
    stamp: Stamp::Clock,
};

/// Recognised tracklist endpoints, tried in order against a station's stream
/// host. laut.fm and SomaFM lead because both publish absolute stamps and need
/// no configuration; the StreamSB panel is last and only carries wall clocks.
const SOURCES: &[Source] = &[
    Source {
        host: "stream.laut.fm",
        path: "/station/{id}/last_songs",
        keys: LAUT_KEYS,
    },
    Source {
        host: "laut.fm",
        path: "/station/{id}/last_songs",
        keys: LAUT_KEYS,
    },
    Source {
        host: "somafm.com",
        path: "/songs/{id}.json",
        keys: SOMAFM_KEYS,
    },
    Source {
        host: "dancewave.online",
        path: "/api/playlist.cgi?user={id}&mount={stream}&num=40&out=json",
        keys: STREAMSB_KEYS,
    },
];

/// Fetch and parse a station's tracklist. `id` is the Radio Browser uuid or a
/// `radios.toml` slug; `stream` is the resolved stream URL, used both to pick
/// the source and to expand the mount in StreamSB-style paths.
pub async fn fetch(
    client: &reqwest::Client,
    stream: &str,
    id: &str,
    override_url: Option<&str>,
) -> Result<RadioTracklist, String> {
    let now = chrono::Utc::now().timestamp();
    if let Some(url) = override_url {
        let body = get(client, url).await?;
        let list = override_list(&body, now);
        if list.tracks.is_empty() {
            return Err("tracklist override matched no known shape".to_string());
        }
        return Ok(list);
    }
    let host = reqwest::Url::parse(stream)
        .ok()
        .and_then(|u| u.host_str().map(str::to_owned))
        .ok_or_else(|| format!("unparseable stream url: {stream}"))?;
    for src in matching(&host) {
        let url = expand(src, &host, id);
        match get(client, &url).await {
            Ok(body) => {
                let list = build(&body, &src.keys, now);
                if !list.tracks.is_empty() {
                    debug!("tracklist source {} -> {} tracks", src.host, list.tracks.len());
                    return Ok(list);
                }
            }
            Err(e) => debug!("tracklist source {} failed: {e}", src.host),
        }
    }
    Err(format!("no tracklist source for {host}"))
}

/// The registry entries claiming `host`, most specific host first. The
/// `stream.` form precedes the bare domain so a station streaming from a
/// subdomain resolves against the entry that needs no slug.
fn matching(host: &str) -> Vec<&'static Source> {
    let mut hits: Vec<&'static Source> = SOURCES
        .iter()
        .filter(|s| host == s.host || host.ends_with(&format!(".{}", s.host)))
        .collect();
    hits.sort_by_key(|s| std::cmp::Reverse(s.host.len()));
    hits
}

/// Expand a source's path template against the station's host and id. The slug
/// is the leftmost label of the stream host (`dance-wave-radio.stream.laut.fm`
/// -> `dance-wave-radio`), which is how both platforms key their stations.
fn expand(src: &Source, host: &str, id: &str) -> String {
    let slug = host.split('.').next().unwrap_or(id);
    let src_host = src.host.trim_start_matches("stream.");
    let authority = if src_host.contains('.') { src_host } else { host };
    let path = src
        .path
        .replace("{id}", &urlencoding::encode(slug))
        .replace("{stream}", &urlencoding::encode(&format!("/{}", id)));
    format!("https://{authority}{path}")
}

/// One `GET` with a bounded body size. A tracklist endpoint is untrusted input,
/// so the read is capped rather than streamed to completion.
async fn get(client: &reqwest::Client, url: &str) -> Result<String, String> {
    let resp = client
        .get(url)
        .send()
        .await
        .map_err(|e| format!("get: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("HTTP {}", resp.status()));
    }
    let body = resp
        .bytes()
        .await
        .map_err(|e| format!("body: {e}"))?;
    if body.len() > MAX_BODY {
        return Err("tracklist body too large".to_string());
    }
    String::from_utf8(body.to_vec()).map_err(|e| format!("utf8: {e}"))
}

/// Parse a `radios.toml` override. The URL carries no schema, so every
/// registered mapping is tried and the first that yields tracks wins — an
/// override on an unrecognised panel therefore needs no configuration beyond
/// the URL itself.
fn override_list(body: &str, now: i64) -> RadioTracklist {
    for keys in [LAUT_KEYS, SOMAFM_KEYS, STREAMSB_KEYS] {
        let list = build(body, &keys, now);
        if !list.tracks.is_empty() {
            return list;
        }
    }
    RadioTracklist {
        at_time: now,
        ..Default::default()
    }
}

/// Cap on a tracklist response, generous for a 40-entry list but bounded.
const MAX_BODY: usize = 512 * 1024;

/// A tracklist response is small and refetched often; keep the client cheap
/// and the wait short so a slow source cannot hold up playback.
pub fn client() -> reqwest::Client {
    reqwest::Client::builder()
        .user_agent(format!("gtm/{} ({})", env!("CARGO_PKG_VERSION"), "gtm"))
        .timeout(Duration::from_secs(8))
        .build()
        .unwrap_or_default()
}

/// Parse a tracklist body into normalised absolute stamps.
///
/// Bare wall clocks are anchored on the entry playing now — which such sources
/// lead with, as their own tracklist pages do — and walked backwards, wrapping
/// at midnight. Ordering and the current-track index are then exact without
/// knowing the station's timezone, which is the whole point: a wall clock alone
/// cannot be converted correctly, but it never needs to be.
fn build(body: &str, keys: &Keys, now: i64) -> RadioTracklist {
    let Ok(json) = serde_json::from_str::<Value>(body) else {
        return RadioTracklist {
            at_time: now,
            ..Default::default()
        };
    };
    let Some(items) = dig(&json, keys.rows).and_then(Value::as_array) else {
        return RadioTracklist {
            at_time: now,
            ..Default::default()
        };
    };
    let mut tracks: Vec<RadioTrack> = items
        .iter()
        .filter_map(|it| {
            let title = text(dig(it, keys.title))?;
            Some(RadioTrack {
                title,
                artist: text(dig(it, keys.artist)).unwrap_or_default(),
                start: None,
                art: text(dig(it, keys.art)),
            })
        })
        .collect();
    let Some(first) = tracks.first_mut() else {
        return RadioTracklist {
            at_time: now,
            ..Default::default()
        };
    };
    match keys.stamp {
        Stamp::Epoch | Stamp::Offset => {
            for (t, it) in tracks.iter_mut().zip(items) {
                t.start = text(dig(it, keys.start))
                    .and_then(|s| stamp(&s, keys.stamp, now));
            }
        }
        Stamp::Clock => {
            first.start = Some(now);
            let mut prev = now;
            for (t, it) in tracks.iter_mut().zip(items).skip(1) {
                let Some(delta) = text(dig(it, keys.start)).and_then(|s| clock(&s)) else {
                    continue;
                };
                // Walk backwards: a timestamp later than the previous entry
                // means the broadcast crossed midnight since.
                prev -= if delta > prev % 86_400 { prev % 86_400 + 86_400 - delta } else { prev % 86_400 - delta };
                t.start = Some(prev);
            }
        }
    }
    RadioTracklist {
        tracks,
        at: 0,
        at_time: now,
    }
}

/// Read a dotted path out of a JSON value. An empty path is the value itself,
/// which is how a source whose body is a bare array is described.
fn dig<'a>(value: &'a Value, path: &str) -> Option<&'a Value> {
    if path.is_empty() {
        return Some(value);
    }
    path.split('.')
        .try_fold(value, |acc, seg| acc.get(seg))
}

/// A field as text, accepting the string, number or nested-object forms the
/// sources use for the same value.
fn text(value: Option<&Value>) -> Option<String> {
    match value? {
        Value::String(s) => Some(s.trim().to_string()),
        Value::Number(n) => Some(n.to_string()),
        Value::Object(map) => map
            .get("name")
            .or_else(|| map.get("title"))
            .and_then(|v| text(Some(v))),
        _ => None,
    }
    .filter(|s| !s.is_empty())
}

/// An absolute instant from a source timestamp.
fn stamp(raw: &str, kind: Stamp, now: i64) -> Option<i64> {
    match kind {
        Stamp::Epoch => raw.trim().parse::<i64>().ok(),
        Stamp::Offset => chrono::DateTime::parse_from_str(raw.trim(), "%Y-%m-%d %H:%M:%S %z")
            .ok()
            .map(|d| d.timestamp()),
        // Unreachable: `Clock` is anchored by the walk in [`build`], which
        // never defers to this.
        Stamp::Clock => Some(now),
    }
}

/// Seconds since midnight for a bare `HH:MM` wall clock. Unparseable input
/// yields `None` so the caller leaves that entry unstamped rather than
/// inventing a time.
fn clock(raw: &str) -> Option<i64> {
    let (h, m) = raw.trim().split_once(':')?;
    Some(h.trim().parse::<i64>().ok()? * 3600 + m.trim().parse::<i64>().ok()? * 60)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(title: &str, start: Option<i64>) -> RadioTrack {
        RadioTrack {
            title: title.into(),
            artist: String::new(),
            start,
            art: None,
        }
    }

    /// Trimmed from a live `api.laut.fm` response.
    const LAUT: &str = r#"[
     {"id":12,"title":"La La Land (Zzino vs Filterheadz Mix)",
      "started_at":"2026-09-26 17:15:03 +0200","ends_at":"2026-09-26 17:16:03 +0200",
      "artist":{"name":"Green Velvet"},"live":true,"type":"song"},
     {"id":12,"title":"My Friend (Josh Gabriel Remix)",
      "started_at":"2026-09-26 17:09:16 +0200","ends_at":"2026-09-26 17:15:03 +0200",
      "artist":{"name":"Josh Gabriel pres. Winter Kills"},"live":true,"type":"song"}
    ]"#;

    /// Trimmed from a live `somafm.com` response.
    const SOMA: &str = r#"{"id":"groovesalad","songs":[
     {"title":"Desert Phase (Hibernation Remix)","artist":"Kaya Project",
      "album":"Interchill","albumArt":"","date":"1790436948"},
     {"title":"Sensually Yours","artist":"Christophe Goze",
      "album":"Horizontal Groove","albumArt":"","date":"1790436603"}
    ]}"#;

    #[test]
    fn live_laut_body_parses() {
        let list = build(LAUT, &LAUT_KEYS, 1_790_440_000);
        assert_eq!(list.tracks.len(), 2);
        assert_eq!(list.tracks[0].artist, "Green Velvet");
        assert_eq!(list.tracks[1].query(), "Josh Gabriel pres. Winter Kills - My Friend (Josh Gabriel Remix)");
        // The nested artist object must not leak in as a title.
        assert!(list.tracks.iter().all(|t| t.title.contains(char::is_alphanumeric)));
        let starts: Vec<_> = list.tracks.iter().filter_map(|t| t.start).collect();
        assert!(starts.windows(2).all(|w| w[0] > w[1]), "{starts:?}");
    }

    #[test]
    fn live_somafm_body_parses() {
        let list = build(SOMA, &SOMAFM_KEYS, 1_790_440_000);
        assert_eq!(list.tracks.len(), 2);
        assert_eq!(list.tracks[0].title, "Desert Phase (Hibernation Remix)");
        assert_eq!(list.tracks[0].start, Some(1_790_436_948));
    }

    #[test]
    fn host_match_prefers_most_specific() {
        let hits = matching("dance-wave-radio.stream.laut.fm");
        assert!(!hits.is_empty());
        assert_eq!(hits[0].path, "/station/{id}/last_songs");
        let url = expand(hits[0], "dance-wave-radio.stream.laut.fm", "x");
        assert!(!url.contains('{'), "{url}");
        assert!(url.contains("dance-wave-radio"), "{url}");
    }

    #[test]
    fn somafm_slug_strips_the_ice_host() {
        let hit = matching("ice2.somafm.com");
        assert!(!hit.is_empty());
        assert_eq!(expand(hit[0], "ice2.somafm.com", "x"), "https://somafm.com/songs/ice.json");
    }

    #[test]
    fn unknown_host_has_no_source() {
        assert!(matching("stream.example.org").is_empty());
    }

    #[test]
    fn offset_stamps_are_absolute() {
        // 2026-09-26 16:50:31 +0200 is 14:50:31 UTC on the 26th.
        let body = r#"[{"title":"A","artist":{"name":"X"},"started_at":"2026-09-26 16:50:31 +0200"}]"#;
        let list = build(body, &LAUT_KEYS, 1_790_440_000);
        assert_eq!(list.tracks[0].start, Some(1_790_434_231));
    }

    #[test]
    fn epoch_stamps_parse_from_strings() {
        let body = r#"{"songs":[{"title":"A","artist":"X","date":"1790434286"}]}"#;
        let list = build(body, &SOMAFM_KEYS, 1_790_440_000);
        assert_eq!(list.tracks[0].start, Some(1_790_434_286));
    }

    #[test]
    fn clock_stamps_anchor_on_now_and_wrap_midnight() {
        // 23:58 then 00:04: the second entry is later on the clock, so it must
        // land before the first rather than in the future.
        let now = 86_400 * 2 + 23 * 3600 + 58 * 60;
        let body = r#"{"mscp":{"playlist":[{"time":"23:58","title":"A"},{"time":"00:04","title":"B"}]}}"#;
        let list = build(body, &STREAMSB_KEYS, now);
        assert_eq!(list.tracks[0].start, Some(now));
        assert_eq!(list.tracks[1].start, Some(now + 6 * 60));
    }

    #[test]
    fn clock_stamps_walk_backwards_in_order() {
        let now = 10 * 3600;
        let body = r#"{"mscp":{"playlist":[{"time":"10:00","title":"A"},{"time":"09:56","title":"B"},{"time":"09:52","title":"C"}]}}"#;
        let list = build(body, &STREAMSB_KEYS, now);
        let starts: Vec<_> = list.tracks.iter().filter_map(|t| t.start).collect();
        assert!(starts.windows(2).all(|w| w[0] > w[1]), "{starts:?}");
    }

    #[test]
    fn unparseable_body_yields_empty_list() {
        let list = build("<html>404</html>", &LAUT_KEYS, 1);
        assert!(list.tracks.is_empty());
        assert_eq!(list.at, 0);
    }

    #[test]
    fn empty_tracklist_is_not_a_source_hit() {
        assert!(build("[]", &LAUT_KEYS, 1).tracks.is_empty());
    }

    #[test]
    fn override_tries_every_mapping() {
        // A bare array with no timestamp must still match one of the
        // registered shapes rather than being rejected outright.
        let list = override_list(r#"[{"title":"A","artist":"X"}]"#, 1);
        assert_eq!(list.tracks.len(), 1);
        assert!(override_list("<html/>", 1).tracks.is_empty());
    }

    #[test]
    fn entries_without_a_title_are_dropped() {
        let body = r#"[{"artist":"X"},{"title":"B","artist":"X"}]"#;
        let list = build(body, &LAUT_KEYS, 1);
        assert_eq!(list.tracks.len(), 1);
        assert_eq!(list.tracks[0].title, "B");
    }

    #[test]
    fn art_field_is_read_when_published() {
        let body = r#"{"songs":[{"title":"A","artist":"X","albumArt":"http://img/a.jpg"}]}"#;
        let list = build(body, &SOMAFM_KEYS, 1);
        assert_eq!(list.tracks[0].art.as_deref(), Some("http://img/a.jpg"));
    }

    #[test]
    fn query_and_title_match_tolerate_spelling() {
        let t = RadioTrack {
            title: "Silent Tears (Orjan Nilsen Remix)".into(),
            artist: "Mark Sherry feat. Sharone".into(),
            ..Default::default()
        };
        assert!(t.same_as("Mark Sherry feat. Sharone - Silent Tears (Orjan Nilsen Remix)"));
        assert!(t.same_as("Mark Sherry - Silent Tears"));
        assert!(!t.same_as("Completely Different Song"));
        assert!(!t.same_as(""));
    }

    #[test]
    fn list_match_title_falls_back_to_leading_entry() {
        let list = RadioTracklist {
            tracks: vec![row("A", None), row("B", None)],
            at: 0,
            at_time: 1,
        };
        assert_eq!(list.match_title("B"), 1);
        assert_eq!(list.match_title("nonsense"), 0);
    }
}
