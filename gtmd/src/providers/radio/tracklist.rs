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

/// How much history a source publishes.
///
/// `Full` sources carry a list, and the code anchors and walks it. `Now`
/// sources publish one entry and nothing else -- that is the shape, not a
/// degraded list, and handing back a one-row `RadioTracklist` would make the
/// queue view show a history of one that never existed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Depth {
    /// A list of recent entries, newest first.
    Full,
    /// One entry: what is on air right now.
    Now,
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

/// Where a platform keeps the station's name in its stream URL. They differ:
/// laut.fm names the station in the subdomain, SomaFM in the path
/// (`/groovesalad-128-mp3`), and neither is derivable from the other.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Slug {
    /// The leftmost hostname label: `dance-wave-radio.stream.laut.fm`.
    Host,
    /// The first path segment up to the bitrate suffix: `/groovesalad-128-mp3`
    /// yields `groovesalad`.
    Path,
}

/// One known tracklist endpoint. Adding a station family is an entry here, not
/// a new fetch-and-parse path.
#[derive(Debug, Clone, Copy)]
struct Source {
    /// Hostname suffix this source claims, matched against the resolved stream
    /// host so a station is recognised by where it streams from.
    host: &'static str,
    /// Authority to actually request. Often not the match host: stations
    /// stream from one domain and publish their tracklist on an API subdomain
    /// (laut.fm streams on `laut.fm` but serves on `api.laut.fm`).
    api: &'static str,
    /// Path template. `{id}` is the station slug, `{stream}` the station's
    /// stream mount.
    path: &'static str,
    slug: Slug,
    keys: Keys,
    /// Headers this endpoint refuses to answer without.
    ///
    /// Some panels gate their CGI on looking like the site's own AJAX call and
    /// answer everything else with a placeholder row. That is a header triple,
    /// not one header: verified by dropping each in turn and watching the
    /// response collapse, so all three are sent together or the request is
    /// pointless.
    headers: &'static [(&'static str, &'static str)],
    /// Titles this endpoint serves *instead of* a tracklist when the request is
    /// refused. Matched against the now-playing entry, because a placeholder is
    /// a single row and would otherwise be reported as the track on air.
    placeholder: &'static [&'static str],
    /// How much this endpoint publishes. See [`Depth`].
    depth: Depth,
}

/// Headers that make a request look like the panel's own AJAX call. The user
/// agent is the panel's site rather than the default `gtm/…`, because the gate
/// checks for a browser.
const BROWSER_HEADERS: &[(&str, &str)] = &[
    (
        "User-Agent",
        "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) \
         Chrome/126.0.0.0 Safari/537.36",
    ),
    ("Referer", "https://dancewave.online/tracklist/"),
];

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

/// Radio Paradise. A single flat object rather than a list, so `rows` is the
/// body itself.
const RADIOPARADISE_KEYS: Keys = Keys {
    rows: "",
    artist: "artist",
    title: "title",
    start: "",
    art: "cover",
    stamp: Stamp::Clock,
};

/// Recognised tracklist endpoints, tried in order against a station's stream
/// host. laut.fm and SomaFM lead because both publish absolute stamps and need
/// no configuration; the StreamSB panel is last and only carries wall clocks.
const SOURCES: &[Source] = &[
    Source {
        host: "stream.laut.fm",
        api: "api.laut.fm",
        path: "/station/{id}/last_songs",
        slug: Slug::Host,
        keys: LAUT_KEYS,
        headers: &[],
        placeholder: &[],
        depth: Depth::Full,
    },
    Source {
        host: "laut.fm",
        api: "api.laut.fm",
        path: "/station/{id}/last_songs",
        slug: Slug::Host,
        keys: LAUT_KEYS,
        headers: &[],
        placeholder: &[],
        depth: Depth::Full,
    },
    Source {
        host: "somafm.com",
        api: "somafm.com",
        path: "/songs/{id}.json",
        slug: Slug::Path,
        keys: SOMAFM_KEYS,
        headers: &[],
        placeholder: &[],
        depth: Depth::Full,
    },
    // A documented, key-less JSON API that answers on any host, so the channel
    // lives in the `chan` parameter rather than in the slug. It publishes the
    // track on air with its own cover, which no other source here does -- and
    // only that one entry: `/api/history` and `/api/archive` both exist and both
    // answer "Nothing to see here", so there is no list to take.
    Source {
        host: "radioparadise.com",
        api: "api.radioparadise.com",
        path: "/api/now_playing?chan=0",
        slug: Slug::Host,
        keys: RADIOPARADISE_KEYS,
        headers: &[],
        placeholder: &[],
        depth: Depth::Now,
    },
    // Neither the station id nor the mount is derivable from the stream host
    // here: `Slug::Host` would yield "dancewave", but the panel keys its user
    // on `dw8080` and its mount on `/dw.ogg`. Both are literal in the template,
    // which is also why `slug` is unused for this entry — a station reachable
    // only through a panel-specific key cannot be described generically.
    Source {
        host: "dancewave.online",
        api: "dancewave.online",
        path: "/api/playlist.cgi?user=dw8080&streamid=1&mount=%2Fdw.ogg\
               &num=40&excludestring=Dance%20Wave&out=json",
        slug: Slug::Host,
        keys: STREAMSB_KEYS,
        headers: BROWSER_HEADERS,
        placeholder: &["dancewave.online"],
        depth: Depth::Full,
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
        let body = get(client, url, &[]).await?;
        let list = override_list(&body, now);
        if list.tracks.is_empty() {
            return Err("tracklist override matched no known shape".to_string());
        }
        return Ok(list);
    }
    let parsed =
        reqwest::Url::parse(stream).map_err(|_| format!("unparseable stream url: {stream}"))?;
    let host = parsed
        .host_str()
        .ok_or_else(|| format!("stream url has no host: {stream}"))?
        .to_string();
    for src in matching(&host) {
        let url = expand(src, &host, parsed.path(), id);
        match get(client, &url, src.headers).await {
            Ok(body) => {
                let list = match src.depth {
                    Depth::Full => build(&body, &src.keys, now),
                    Depth::Now => build_now(&body, &src.keys, now),
                };
                // A refused request still parses: the placeholder is a
                // well-formed single-row list, so without this the panel's
                // "listen on our website" notice would be accepted as the
                // tracklist and reported as the track on air.
                if let Some(title) = list.now().map(|t| t.title.as_str())
                    && src.placeholder.contains(&title)
                {
                    debug!(
                        "tracklist source {} returned its placeholder ({title}), \
                         treating as no hit",
                        src.host
                    );
                    continue;
                }
                if !list.tracks.is_empty() {
                    debug!(
                        "tracklist source {} -> {} tracks",
                        src.host,
                        list.tracks.len()
                    );
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
fn expand(src: &Source, host: &str, stream_path: &str, id: &str) -> String {
    let slug = station_slug(src, host, stream_path).unwrap_or_else(|| id.to_string());
    let path = src
        .path
        .replace("{id}", &urlencoding::encode(&slug))
        .replace("{stream}", &urlencoding::encode(&format!("/{}", id)));
    format!("https://{}{path}", src.api)
}

/// The station's own name, taken from wherever the platform puts it. `None`
/// when the URL carries no usable segment, so the caller can fall back rather
/// than request a nonsense channel.
fn station_slug(src: &Source, host: &str, path: &str) -> Option<String> {
    let seg = path.split('/').find(|s| !s.is_empty())?;
    let name = match src.slug {
        Slug::Host => host.split('.').next().unwrap_or_default(),
        // SomaFM names channels with underscores and appends `-<bitrate>-<codec>`
        // to the path, so the name is whatever precedes the first dash.
        Slug::Path => seg.split('-').next().unwrap_or_default(),
    };
    (!name.is_empty()).then(|| name.to_string())
}

/// One `GET` with a bounded body size. A tracklist endpoint is untrusted input,
/// so the read is capped rather than streamed to completion.
async fn get(
    client: &reqwest::Client,
    url: &str,
    headers: &[(&str, &str)],
) -> Result<String, String> {
    let mut req = client.get(url);
    for (name, value) in headers {
        req = req.header(*name, *value);
    }
    let resp = req.send().await.map_err(|e| format!("get: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("HTTP {}", resp.status()));
    }
    let body = resp.bytes().await.map_err(|e| format!("body: {e}"))?;
    if body.len() > MAX_BODY {
        return Err("tracklist body too large".to_string());
    }
    // The body is bytes, not text: a panel that mislabels its own encoding
    // still sends well-formed UTF-8 here, and rejecting it would lose the list.
    // What is mislabelled gets repaired per-field by [`unmojibake`] instead.
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
/// One client for the tracklist fetcher.
///
/// Rebuilt on every call before, and the tracklist refreshes every 30s while a
/// station plays: a fresh TLS stack and connection pool each time, for a
/// request to the same handful of hosts.
pub fn client() -> &'static reqwest::Client {
    static CLIENT: std::sync::OnceLock<reqwest::Client> = std::sync::OnceLock::new();
    CLIENT.get_or_init(|| {
        reqwest::Client::builder()
            .user_agent(format!("gtm/{} ({})", env!("CARGO_PKG_VERSION"), "gtm"))
            .timeout(Duration::from_secs(8))
            .build()
            .unwrap_or_default()
    })
}

/// Parse a tracklist body into normalised absolute stamps.
///
/// Bare wall clocks are anchored on the entry playing now — which such sources
/// lead with, as their own tracklist pages do — and walked backwards, wrapping
/// at midnight. Ordering and the current-track index are then exact without
/// knowing the station's timezone, which is the whole point: a wall clock alone
/// cannot be converted correctly, but it never needs to be.
fn build(body: &str, keys: &Keys, now: i64) -> RadioTracklist {
    build_at(body, keys, now, Depth::Full)
}

/// The same parse, for a source that publishes only what is on air.
///
/// A single flat object rather than a list, so it is read as one row and
/// anchored at `now` like the head of any list. Kept a separate entry point
/// rather than a flag on `Keys` because `Keys` describes where fields are and
/// this describes how much there is -- two different questions, and the walk in
/// [`build`] must not be handed a row it will try to walk backwards from.
fn build_now(body: &str, keys: &Keys, now: i64) -> RadioTracklist {
    build_at(body, keys, now, Depth::Now)
}

fn build_at(body: &str, keys: &Keys, now: i64, depth: Depth) -> RadioTracklist {
    let Ok(json) = serde_json::from_str::<Value>(&scrub_surrogates(body)) else {
        return RadioTracklist {
            at_time: now,
            ..Default::default()
        };
    };
    // A `Now` source's body is the entry itself. Wrapping it is what lets one
    // parse handle both shapes without a second reader per source.
    let wrapped;
    let body = if depth == Depth::Now {
        wrapped = serde_json::json!([json]);
        &wrapped
    } else {
        &json
    };
    let Some(items) = dig(body, keys.rows).and_then(Value::as_array) else {
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
                t.start = text(dig(it, keys.start)).and_then(|s| stamp(&s, keys.stamp, now));
            }
        }
        Stamp::Clock => {
            first.start = Some(now);
            let mut prev = now;
            // Seeded from the leading entry's own stamp, not from `now`. A
            // station publishes its local wall clock and the fetch clock is
            // UTC, so deriving the first reading from `now` places every gap
            // after it out by exactly the station's offset -- three hours for a
            // GMT+3 panel, which then reads as a 19-hour gap between
            // consecutive tracks.
            let mut prev_clock = text(dig(&items[0], keys.start)).and_then(|s| clock(&s));
            for (t, it) in tracks.iter_mut().zip(items.iter()).skip(1) {
                let Some(cur) = text(dig(it, keys.start)).and_then(|s| clock(&s)) else {
                    continue;
                };
                let Some(at) = prev_clock else {
                    // The leading entry carried no stamp, so this one becomes
                    // the reference and the pair before it stays unstamped.
                    prev_clock = Some(cur);
                    continue;
                };
                // Walk backwards through the day. A stamp later on the clock
                // than the entry before it means the broadcast crossed
                // midnight in between, so that gap spans a day boundary.
                prev -= if cur > at {
                    at + 86_400 - cur
                } else {
                    at - cur
                };
                t.start = Some(prev);
                prev_clock = Some(cur);
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
    path.split('.').try_fold(value, |acc, seg| acc.get(seg))
}

/// A field as text, accepting the string, number or nested-object forms the
/// sources use for the same value, with any transport encoding undone.
///
/// The repair runs on the decoded value rather than on the raw body: JSON
/// escapes are resolved by the parser first, so an escaped `é` and a literal
/// one arrive as the same character and need the same treatment.
fn text(value: Option<&Value>) -> Option<String> {
    match value? {
        Value::String(s) => Some(unmojibake(s.trim())),
        Value::Number(n) => Some(n.to_string()),
        Value::Object(map) => map
            .get("name")
            .or_else(|| map.get("title"))
            .and_then(|v| text(Some(v))),
        _ => None,
    }
    .map(|s| s.trim().to_string())
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

/// Seconds since midnight for a bare `HH:MM` or `HH:MM:SS` wall clock.
/// Unparseable input yields `None` so the caller leaves that entry unstamped
/// rather than inventing a time.
///
/// Seconds are read when present and dropped when not, rather than rejected:
/// the field is a wall clock at minute resolution in some panels and second
/// resolution in others, and rejecting `"15:22:45"` outright lost the entire
/// walk-backwards — every entry after the first came back unstamped, so the
/// list rendered with one real timestamp and a column of nothing.
fn clock(raw: &str) -> Option<i64> {
    let mut parts = raw.trim().split(':');
    let mut take = || parts.next()?.trim().parse::<i64>().ok().filter(|n| *n >= 0);
    let h = take()?;
    let m = take()?;
    let s = take().unwrap_or(0);
    // Reject out-of-range components rather than carrying them into the
    // midnight arithmetic in [`build`], where a bogus 90-minute offset would
    // silently shift every stamp before it.
    (h < 24 && m < 60 && s < 60).then_some(h * 3600 + m * 60 + s)
}

/// Windows-1252's printables for bytes `0x80..=0x9F`, the only range where
/// cp1252 and ISO-8859-1 disagree.
///
/// One table, read in both directions, so the forward and reverse lookups
/// cannot disagree. The five bytes cp1252 leaves undefined are absent, which is
/// what marks a string carrying one as unfit for the repair.
const CP1252_HIGH: [(u8, char); 27] = [
    (0x80, '\u{20ac}'),
    (0x82, '\u{201a}'),
    (0x83, '\u{0192}'),
    (0x84, '\u{201e}'),
    (0x85, '\u{2026}'),
    (0x86, '\u{2020}'),
    (0x87, '\u{2021}'),
    (0x88, '\u{02c6}'),
    (0x89, '\u{2030}'),
    (0x8a, '\u{0160}'),
    (0x8b, '\u{2039}'),
    (0x8c, '\u{0152}'),
    (0x8e, '\u{017d}'),
    (0x91, '\u{2018}'),
    (0x92, '\u{2019}'),
    (0x93, '\u{201c}'),
    (0x94, '\u{201d}'),
    (0x95, '\u{2022}'),
    (0x96, '\u{2013}'),
    (0x97, '\u{2014}'),
    (0x98, '\u{02dc}'),
    (0x99, '\u{2122}'),
    (0x9a, '\u{0161}'),
    (0x9b, '\u{203a}'),
    (0x9c, '\u{0153}'),
    (0x9e, '\u{017e}'),
    (0x9f, '\u{0178}'),
];

/// The byte a character stands for in Windows-1252, or `None` when it has none.
///
/// `None` is the load-bearing part. A character outside this table cannot have
/// come out of a cp1252 misread, so a string containing one was already correct
/// and is left alone. That is what protects CJK, emoji and anything else the
/// panel could not have produced.
fn cp1252_byte(c: char) -> Option<u8> {
    match c {
        '\u{00}'..='\u{7f}' | '\u{a0}'..='\u{ff}' => Some(c as u8),
        // The C1 controls are how a misread of bytes 0x80..=0x9F looks when
        // the misread was latin-1 rather than cp1252. Kept so a latin-1 panel
        // repairs too, and unconditionally: whether a byte is a defined
        // *character* in cp1252 says nothing about whether it is a valid UTF-8
        // *continuation* byte, and refusing the five undefined ones threw away
        // `\u{9d}` -- the last byte of every mangled right curly quote. The
        // UTF-8 check in [`unmojibake`] is what actually declines, because a
        // control standing alone is not valid UTF-8 and one inside a sequence
        // is exactly what belongs there.
        '\u{80}'..='\u{9f}' => Some(c as u8),
        _ => CP1252_HIGH.iter().find(|(_, ch)| *ch == c).map(|(b, _)| *b),
    }
}

/// Undo text that a source re-encoded through Windows-1252.
///
/// Some panels hand back UTF-8 that was decoded as cp1252 and re-encoded as
/// UTF-8 again, so `é` arrives as `Ã©`. The damage is byte-for-byte reversible:
/// map each character back to the cp1252 byte it stands for, then read those
/// bytes as UTF-8.
///
/// The UTF-8 validity check is what makes this safe rather than clever. Correct
/// text almost never survives it — `Café` re-encodes to a lone `E9`, `“quotes”`
/// to a lone `93`, `€100` to a lone `80`, each invalid on its own — so the
/// repair declines and the original is returned. Only text that really is
/// misread produces valid UTF-8, and that is exactly what should be rewritten.
fn unmojibake(s: &str) -> String {
    fn once(s: &str) -> Option<String> {
        let bytes: Vec<u8> = s.chars().map(cp1252_byte).collect::<Option<Vec<u8>>>()?;
        String::from_utf8(bytes).ok()
    }
    if s.is_ascii() {
        return s.to_string();
    }
    let mut out = s.to_string();
    // Bounded, and a pass that stops improving ends it: a stack of misreading
    // proxies applies the transform more than once, and an accidental no-op
    // loop would be worse than one pass. Two layers covers every panel seen.
    for _ in 0..2 {
        let Some(next) = once(&out) else { return out };
        if next == out {
            return out;
        }
        out = next;
    }
    out
}

/// Replace lone surrogate escapes with U+FFFD so a body that carries one still
/// parses.
///
/// `serde_json` refuses an unpaired `\uD800`-style escape outright, which loses
/// the whole tracklist over one bad character in one title. Replacing the
/// escape keeps every other field on that row and every other row on the list.
/// Real surrogate *pairs* are left alone — those are valid and must decode to
/// the astral character they encode.
fn scrub_surrogates(body: &str) -> std::borrow::Cow<'_, str> {
    if !body.contains("\\u") {
        return std::borrow::Cow::Borrowed(body);
    }
    let bytes = body.as_bytes();
    let mut out = String::with_capacity(body.len());
    let mut i = 0;
    let mut changed = false;
    while i < bytes.len() {
        // Cheap pre-check: a backslash-u is the only place a surrogate can be,
        // so copying everything else verbatim needs no per-char work.
        if body[i..].starts_with("\\u") && i + 6 <= body.len() {
            let hex = &body[i + 2..i + 6];
            if let Some(unit) = u16::from_str_radix(hex, 16).ok()
                && (0xd800..0xdc00).contains(&unit)
            {
                // A high surrogate is only real when a low surrogate follows.
                let paired = body[i + 6..]
                    .strip_prefix("\\u")
                    .and_then(|rest| {
                        (rest.len() >= 4)
                            .then(|| u16::from_str_radix(&rest[..4], 16).ok())
                            .flatten()
                    })
                    .is_some_and(|low| (0xdc00..0xe000).contains(&low));
                if !paired {
                    out.push('\u{fffd}');
                    i += 6;
                    changed = true;
                    continue;
                }
            }
        }
        // `body` is `&str`, so this slice is on a char boundary.
        let ch = body[i..].chars().next().unwrap_or('\u{fffd}');
        out.push(ch);
        i += ch.len_utf8();
    }
    if changed {
        std::borrow::Cow::Owned(out)
    } else {
        std::borrow::Cow::Borrowed(body)
    }
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
        assert_eq!(
            list.tracks[1].query(),
            "Josh Gabriel pres. Winter Kills - My Friend (Josh Gabriel Remix)"
        );
        // The nested artist object must not leak in as a title.
        assert!(
            list.tracks
                .iter()
                .all(|t| t.title.contains(char::is_alphanumeric))
        );
        let starts: Vec<_> = list.tracks.iter().filter_map(|t| t.start).collect();
        assert!(starts.windows(2).all(|w| w[0] > w[1]), "{starts:?}");
    }

    /// Trimmed from a live `dancewave.online` response, mangled text and all.
    ///
    /// These four rows are the reason [`unmojibake`] exists: the panel sends
    /// UTF-8 that was decoded as cp1252 and re-encoded, so `É` is stored as
    /// `Ã` followed by `‰` and `é` as `Ã` followed by `©`.
    const DANCEWAVE: &str = concat!(
        r#"{"mscp":{"playlist":["#,
        r#"{"time":"14:08:38","artist":"DÃ‰ SAINT.","title":"Take Over Me"},"#,
        r#"{"time":"13:47:15","artist":"Demi RiquÃ­simo","title":"No Given Time"},"#,
        r#"{"time":"13:33:23","artist":"Kara Okay","title":"LibertÃ© (Short Mix)"},"#,
        r#"{"time":"13:03:06","artist":"Beyonce","title":"Single Ladies (Put A Ring On It) (Dave AudÃ© Radio Edit)"}"#,
        r#"]}}"#
    );

    #[test]
    fn live_dancewave_body_parses_and_unmangles() {
        let list = build(DANCEWAVE, &STREAMSB_KEYS, 1_790_500_000);
        assert_eq!(list.tracks.len(), 4);
        assert_eq!(list.tracks[0].artist, "DÉ SAINT.");
        assert_eq!(list.tracks[1].artist, "Demi Riquísimo");
        assert_eq!(list.tracks[2].title, "Liberté (Short Mix)");
        assert_eq!(
            list.tracks[3].title,
            "Single Ladies (Put A Ring On It) (Dave Audé Radio Edit)"
        );
        // Nothing anywhere in the list may still carry the mangling heads.
        for t in &list.tracks {
            assert!(
                !format!("{} {}", t.artist, t.title).contains('\u{c3}')
                    && !format!("{} {}", t.artist, t.title).contains('\u{c2}'),
                "left mangled: {t:?}"
            );
        }
    }

    #[test]
    fn dancewave_seconds_stamp_every_entry() {
        // The bug this guards: `clock` split on the first colon and failed to
        // parse `"22:45"` as minutes, so `HH:MM:SS` yielded `None` for every
        // entry after the first and the walk-backwards never ran.
        let list = build(DANCEWAVE, &STREAMSB_KEYS, 1_790_500_000);
        assert!(list.tracks.iter().all(|t| t.start.is_some()), "{list:?}");
        // 14:08:38 -> 13:47:15 is 21m23s.
        let gap = list.tracks[0].start.unwrap() - list.tracks[1].start.unwrap();
        assert_eq!(gap, 21 * 60 + 23, "{gap}");
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
        let url = expand(
            hits[0],
            "dance-wave-radio.stream.laut.fm",
            "/dance-wave-radio",
            "x",
        );
        assert!(!url.contains('{'), "{url}");
        assert!(url.contains("dance-wave-radio"), "{url}");
    }

    #[test]
    fn slug_comes_from_where_each_platform_puts_it() {
        // SomaFM names the channel in the path, not the subdomain: the shared
        // `iceN` host says nothing about which channel is playing.
        let hit = matching("ice2.somafm.com");
        assert!(!hit.is_empty());
        assert_eq!(
            expand(hit[0], "ice2.somafm.com", "/groovesalad-128-mp3", "x"),
            "https://somafm.com/songs/groovesalad.json"
        );
        // laut.fm names it in the subdomain.
        let hit = matching("dance-wave-radio.stream.laut.fm");
        assert_eq!(
            expand(
                hit[0],
                "dance-wave-radio.stream.laut.fm",
                "/dance-wave-radio",
                "x"
            ),
            "https://api.laut.fm/station/dance-wave-radio/last_songs"
        );
    }

    #[test]
    fn offset_stamps_are_absolute() {
        // 2026-09-26 16:50:31 +0200 is 14:50:31 UTC on the 26th.
        let body =
            r#"[{"title":"A","artist":{"name":"X"},"started_at":"2026-09-26 16:50:31 +0200"}]"#;
        let list = build(body, &LAUT_KEYS, 1_790_440_000);
        assert_eq!(list.tracks[0].start, Some(1_790_434_231));
    }

    #[test]
    fn epoch_stamps_parse_from_strings() {
        let body = r#"{"songs":[{"title":"A","artist":"X","date":"1790434286"}]}"#;
        let list = build(body, &SOMAFM_KEYS, 1_790_440_000);
        assert_eq!(list.tracks[0].start, Some(1_790_434_286));
    }

    /// Trimmed from a live `api.radioparadise.com` response.
    const PARADISE: &str = r#"{"time":171,"artist":"Ezra Collective",
     "title":"Well Organised (feat. Lila Iké)","album":"Well Organised",
     "year":"2026","cover":"https://img.radioparadise.com/covers/l/30662.jpg",
     "cover_med":"https://img.radioparadise.com/covers/m/30662.jpg"}"#;

    /// A source that publishes one entry parses as one entry, with its own cover
    /// and anchored at the fetch clock.
    ///
    /// The body is a flat object, not a list, and there is no date or time of
    /// any kind in it -- so it cannot go through the list walk, which would
    /// either find no rows or try to walk backwards from a timestamp that is
    /// not there. Wrapping it as a single row is what makes one parser serve
    /// both shapes.
    #[test]
    fn a_now_source_parses_as_one_row() {
        let now = 1_790_440_000;
        let list = build_now(PARADISE, &RADIOPARADISE_KEYS, now);
        assert_eq!(list.tracks.len(), 1, "{list:?}");
        assert_eq!(list.tracks[0].artist, "Ezra Collective");
        assert_eq!(list.tracks[0].title, "Well Organised (feat. Lila Iké)");
        // The one thing this source has that no other entry does.
        assert_eq!(
            list.tracks[0].art.as_deref(),
            Some("https://img.radioparadise.com/covers/l/30662.jpg")
        );
        assert_eq!(list.tracks[0].start, Some(now), "not anchored at now");
        assert_eq!(list.at, 0);
        assert_eq!(list.at_time, now);
    }

    #[test]
    fn a_now_source_survives_a_bad_body() {
        for body in ["", "null", "<html/>", "{}"] {
            let list = build_now(body, &RADIOPARADISE_KEYS, 1);
            assert!(list.tracks.is_empty(), "invented a track from {body:?}");
        }
    }

    /// The registry must not claim a history it cannot deliver.
    #[test]
    fn only_publishing_sources_are_marked_full() {
        for src in SOURCES {
            let list = match src.depth {
                Depth::Full => build("[]", &src.keys, 1),
                Depth::Now => build_now("{}", &src.keys, 1),
            };
            assert!(
                list.tracks.is_empty(),
                "the empty fixture parsed for {src:?}"
            );
        }
        assert!(
            SOURCES.iter().any(|s| s.host == "radioparadise.com"),
            "radio paradise fell out of the registry"
        );
    }

    #[test]
    fn clock_stamps_anchor_on_now_and_wrap_midnight() {
        // Now is 00:04 and the previous track was 23:58, so that gap crosses
        // midnight. The list is newest first, so the earlier row must land on
        // the previous day rather than 23h56m in the past.
        let now = 86_400 * 2 + 4 * 60;
        let body =
            r#"{"mscp":{"playlist":[{"time":"00:04","title":"A"},{"time":"23:58","title":"B"}]}}"#;
        let list = build(body, &STREAMSB_KEYS, now);
        assert_eq!(list.tracks[0].start, Some(now));
        assert_eq!(list.tracks[1].start, Some(now - 6 * 60));
    }

    #[test]
    fn clock_stamps_do_not_wrap_without_crossing_midnight() {
        // Both entries after midnight: 00:20 then 00:14, a six-minute gap with
        // no day boundary to account for.
        let now = 86_400 * 2 + 20 * 60;
        let body =
            r#"{"mscp":{"playlist":[{"time":"00:20","title":"A"},{"time":"00:14","title":"B"}]}}"#;
        let list = build(body, &STREAMSB_KEYS, now);
        assert_eq!(list.tracks[1].start, Some(now - 6 * 60));
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

    /// The repair is only safe because mangled text cannot contain a character
    /// above U+00FF. Each case here is a real character that must survive
    /// untouched, and the failure mode is silently wrong data rather than a
    /// crash, so they are pinned individually.
    #[test]
    fn unmangling_leaves_correct_text_alone() {
        for good in [
            "Plain ASCII",
            "Café", // Latin-1, all bytes >= 0x80
            "Zoë Kravitz",
            "Möö",
            "日",   // 3-byte UTF-8
            "北京", // CJK
            "🎵 emoji",
            "“curly quotes”", // real U+201C/U+201D
            "dash – and em—dash",
            "Ångström × ½",
            "€100 — 20% off™",
            "",
            " ",
        ] {
            assert_eq!(unmojibake(good), good, "mangled a correct string: {good:?}");
        }
    }

    #[test]
    fn unmangling_undoes_cp1252_misreads() {
        // Every pair here is (correct, as the panel sent it). The `É` and `Ö`
        // rows are the ones a latin-1 implementation gets wrong: their second
        // byte is in 0x80..=0x9F, where cp1252 and ISO-8859-1 disagree.
        for (wrong, right) in [
            ("Ã©", "é"), // 0xA9 -> ©   (both cp1252 and latin-1 agree here)
            // 0x89 is the case a latin-1 implementation gets wrong: cp1252 reads
            // it as per-mille, latin-1 leaves a control character, so `É` comes
            // back as `Ã` + `‰` and only the cp1252 table recovers it.
            ("Ã\u{89}", "É"),
            // Real strings, captured from a live panel and checked against a
            // reference decoder.
            ("DÃ‰ SAINT.", "DÉ SAINT."),
            // The soft hyphen is part of the damage: `í` is `C3 AD`, and
            // `AD` is the soft hyphen in cp1252. Escaped, because an
            // invisible character written literally is both a lint
            // failure and impossible to see in a diff.
            ("Demi RiquÃ\u{ad}simo", "Demi Riquísimo"),
            ("LibertÃ© (Short Mix)", "Liberté (Short Mix)"),
            ("Dave AudÃ© Radio Edit", "Dave Audé Radio Edit"),
            ("MÃ–Ã–NE", "MÖÖNE"),
        ] {
            assert_eq!(unmojibake(wrong), right, "failed on {wrong:?}");
        }
    }

    /// The cp1252 character a byte in `0x80..=0x9F` decodes to.
    ///
    /// Read straight out of the production table rather than derived, so the
    /// test forward direction cannot silently drift away from the repair's
    /// reverse one -- which is exactly what a derived lookup did: it matched the
    /// C1-control identity entry first and never reached the printable
    /// character, so the mangler emitted \u{80} where the panel emits \u{20ac}.
    fn cp1252_char(byte: u8) -> Option<char> {
        CP1252_HIGH
            .iter()
            .find(|(b, _)| *b == byte)
            .map(|(_, c)| *c)
    }

    /// Apply the panel's damage to correct text, so the repair can be checked
    /// against a derived corpus instead of only hand-written pairs.
    ///
    /// Forward is byte -> cp1252 character; the repair is its inverse. Deriving
    /// the broken inputs this way removes the class of bug where the
    /// *expectation* is miscomputed -- which hand-writing them does constantly,
    /// because a multi-byte character hides behind several mangled characters
    /// and the byte that actually changes is rarely the first one.
    fn mangle(s: &str) -> String {
        let mut out = String::with_capacity(s.len());
        for c in s.chars() {
            if (c as u32) < 0x80 {
                // ASCII survives a misread untouched.
                out.push(c);
                continue;
            }
            for byte in c.to_string().as_bytes() {
                out.push(cp1252_char(*byte).unwrap_or(char::from(*byte)));
            }
        }
        out
    }

    #[test]
    fn unmangling_inverts_the_damage_derived_from_real_text() {
        for good in [
            "DÉ SAINT.",
            "Liberté (Short Mix)",
            "Dave Audé Radio Edit",
            "MÖÖNE",
            "Demi Riquísimo",
            "日本語",
            "Ø",
            "Ñandú",
            "Ça va",
            "A$ £ ¥ €",
            "Sean “Groove” Paul",
            "x—y",
            "naïve coöperate",
        ] {
            let broken = mangle(good);
            assert_ne!(broken, good, "test text has nothing to repair: {good:?}");
            assert_eq!(unmojibake(&broken), good, "failed on {broken:?}");
        }
    }

    /// Exhaustive rather than illustrative: every character cp1252 can carry,
    /// mangled and repaired, must come back unchanged.
    ///
    /// The hand-written pairs above are the ones worth reading; this is the one
    /// that catches the character nobody thought of. It walks the whole byte
    /// range and every two-byte and three-byte UTF-8 sequence that cp1252 is
    /// able to misread, which is the entire space of damage this function can
    /// be asked to undo.
    #[test]
    fn every_mangleable_character_round_trips() {
        let mut checked = 0usize;
        // Single cp1252 characters.
        for byte in 0u8..=255 {
            let Some(c) = mangle_byte(byte) else { continue };
            assert_eq!(cp1252_byte(c), Some(byte), "table disagrees at {byte:#04x}");
            if (0x80..=0x9f).contains(&byte) {
                let broken = c.to_string();
                assert_eq!(unmojibake(&broken), c.to_string(), "byte {byte:#04x}");
                checked += 1;
            }
        }
        // Two- and three-byte UTF-8 sequences: the lead byte plus its
        // continuations, mangled as a unit the way a panel sends them.
        for lead in 0xc2u8..=0xf4 {
            for cont in 0x80u8..=0xbf {
                let seq = [lead, cont];
                if let Ok(text) = std::str::from_utf8(&seq) {
                    let broken = mangle(text);
                    assert_eq!(unmojibake(&broken), text, "2-byte {text:?}");
                    checked += 1;
                }
                let seq3 = [lead, cont, cont];
                if let Ok(text) = std::str::from_utf8(&seq3) {
                    let broken = mangle(text);
                    assert_eq!(unmojibake(&broken), text, "3-byte {text:?}");
                    checked += 1;
                }
            }
        }
        assert!(checked > 2_500, "only {checked} cases exercised");
    }

    /// The character a single byte decodes to under a cp1252 misread.
    fn mangle_byte(byte: u8) -> Option<char> {
        match cp1252_char(byte) {
            Some(c) => Some(c),
            // Below 0x80 a misread is the identity; above it, latin-1.
            None => char::from_u32(u32::from(byte))
                .filter(|c| byte < 0x80 || *c as u32 == u32::from(byte)),
        }
    }

    /// A long list walked end to end, the way the queue view renders it.
    ///
    /// Ordering is the property the read-only queue depends on: the list is
    /// drawn newest first, and every stamp after the first is derived by
    /// subtraction, so a single bad pair puts the whole column out of order.
    #[test]
    fn a_long_list_stays_ordered_across_a_midnight_crossing() {
        // 40 entries is what the panel is asked for, spanning six hours, which
        // is enough to cross midnight once and to exercise the wrap twice.
        let now = 86_400 * 4 + 3600 + 12 * 60;
        let mut playlist = Vec::new();
        for i in 0..40 {
            // Start at 01:12 and step backwards three minutes and twelve
            // seconds, so the list crosses midnight partway down.
            let back = i * (3 * 60 + 12);
            let secs: i64 = 3600 + 12 * 60 + 60 - back;
            // Hour of day only. The panel publishes no date, so encoding the
            // day here would test the generator rather than the walk that has
            // to infer it.
            let sod = secs.rem_euclid(86_400);
            playlist.push(format!(
                r#"{{"time":"{:02}:{:02}:{:02}","artist":"A{i}","title":"T{i}"}}"#,
                sod / 3600,
                (sod / 60) % 60,
                sod % 60
            ));
        }
        let body = format!(r#"{{"mscp":{{"playlist":[{}]}}}}"#, playlist.join(","));
        let list = build(&body, &STREAMSB_KEYS, now);
        assert_eq!(list.tracks.len(), 40);
        assert!(
            list.tracks.iter().all(|t| t.start.is_some()),
            "a gap in the column"
        );
        let starts: Vec<i64> = list.tracks.iter().map(|t| t.start.unwrap()).collect();
        assert!(
            starts.windows(2).all(|w| w[0] > w[1]),
            "not strictly decreasing: {starts:?}"
        );
        // Every gap is exactly the step, with no day-sized jump in the middle.
        for w in starts.windows(2) {
            assert_eq!(w[0] - w[1], 3 * 60 + 12, "gap drifted at {w:?}");
        }
    }

    /// A body that is not the shape this source describes, and a few that are
    /// merely malformed, must all degrade to an empty list rather than panic.
    #[test]
    fn hostile_bodies_degrade_without_panicking() {
        for body in [
            "",
            "null",
            "{}",
            "[]",
            r#"{"mscp":"not an object"}"#,
            r#"{"mscp":{"playlist":"not an array"}}"#,
            r#"{"mscp":{"playlist":[null,1,true,"x"]}}"#,
            r#"{"mscp":{"playlist":[{"time":"","title":""}]}}"#,
            r#"{"mscp":{"playlist":[{"time":null,"title":"A"}]}}"#,
            r#"{"mscp":{"playlist":[{"title":{}},{"title":[]}]}}"#,
            "<html><body>404</body></html>",
            "\u{0}\u{1}\u{2}",
            "\u{feff}{\"mscp\":{\"playlist\":[]}}",
            &"x".repeat(2_000),
        ] {
            // The contract is that nothing panics and nothing is invented:
            // a row may survive, but only if it really carried a title, and it
            // never gets a stamp the source did not publish.
            let list = build(body, &STREAMSB_KEYS, 1);
            assert_eq!(list.at_time, 1);
            assert!(
                list.tracks.iter().all(|t| !t.title.is_empty()),
                "invented a title from {body:.40?}"
            );
        }
    }

    /// Live check, opt-in: `cargo test -p gtmd --lib -- --ignored`.
    ///
    /// Not part of the suite because it needs the network and a station that is
    /// actually broadcasting, but it is the only assertion that proves the
    /// registered URL and the header gate are still right against the panel
    /// rather than against my reading of it.
    #[test]
    #[ignore = "hits the network"]
    fn live_dancewave_endpoint_still_answers() {
        let host = "dancewave.online";
        let src = matching(host)[0];
        let url = expand(src, host, "/", "");
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("runtime");
        let body = rt
            .block_on(get(client(), &url, src.headers))
            .expect("request failed");
        let list = build(&body, &src.keys, chrono::Utc::now().timestamp());
        assert!(!list.tracks.is_empty(), "no tracks from {url}");
        let title = list.now().unwrap().title.clone();
        assert!(
            !src.placeholder.contains(&title.as_str()),
            "got the refusal: {title}"
        );
        // Nothing may arrive mangled, and nothing may be left unstamped.
        for t in &list.tracks {
            assert!(t.start.is_some(), "unstamped: {t:?}");
            assert!(!t.title.contains('\u{c3}'), "mangled: {t:?}");
        }
        let starts: Vec<i64> = list.tracks.iter().map(|t| t.start.unwrap()).collect();
        assert!(starts.windows(2).all(|w| w[0] > w[1]), "{starts:?}");
        // Printed because a human reading this is usually checking the panel,
        // not the parser: the shapes below are what the queue view renders.
        println!("{} tracks, fetched from {url}", list.tracks.len());
        for t in list.tracks.iter().take(5) {
            let at = t.start.unwrap();
            println!(
                "  {}  {}",
                chrono::DateTime::from_timestamp(at, 0)
                    .map_or_else(|| at.to_string(), |d| d.to_rfc3339()),
                t.query()
            );
        }
    }

    #[test]
    fn unmangling_undoes_a_second_layer() {
        // A stack of misreading proxies applies the transform twice. One pass
        // gets `ÃƒÂ©`, not `é`, so the loop has to run to a fixpoint.
        assert_eq!(unmojibake("ÃƒÂ©"), "é");
        // `DÉ` is `D C3 89`; misread once that is `D` + `Ã` + `‰`, and
        // misread again each of those splits further. Hand-writing the
        // intermediate is where this goes wrong, so it is spelled out.
        assert_eq!(unmojibake("D\u{c3}\u{192}\u{e2}\u{20ac}\u{b0}"), "DÉ");
        assert_eq!(
            unmojibake("D\u{c3}\u{192}\u{e2}\u{20ac}\u{b0} SAINT."),
            "DÉ SAINT."
        );
        // And it must stop rather than oscillate.
        assert_eq!(unmojibake("é"), "é");
    }

    #[test]
    fn unmangling_refuses_text_it_cannot_prove_is_wrong() {
        // cp1252 leaves five bytes in 0x80..=0x9F undefined; a string carrying
        // one cannot have come from a clean round-trip, so it is left as sent
        // rather than guessed at.
        assert_eq!(unmojibake("weird \u{81} char"), "weird \u{81} char");
        // An embedded NUL is not a cp1252 byte and not a real title character.
        assert_eq!(unmojibake("a\0b"), "a\0b");
    }

    #[test]
    fn clock_reads_minutes_and_seconds() {
        assert_eq!(clock("14:08:38"), Some(14 * 3600 + 8 * 60 + 38));
        assert_eq!(clock("00:00:00"), Some(0));
        // The minute-resolution shape still parses, and the seconds are zero.
        assert_eq!(clock("14:08"), Some(14 * 3600 + 8 * 60));
        // A lone hour, with nothing after it, is not a time.
        assert_eq!(clock("14"), None);
    }

    #[test]
    fn clock_rejects_out_of_range_components() {
        // These would silently shift every stamp before them in the midnight
        // walk, so they are refused rather than clamped.
        assert_eq!(clock("24:00:00"), None);
        assert_eq!(clock("12:60:00"), None);
        assert_eq!(clock("12:00:60"), None);
        assert_eq!(clock("-1:00:00"), None);
        assert_eq!(clock("aa:bb:cc"), None);
        assert_eq!(clock(""), None);
    }

    #[test]
    fn clock_walk_crosses_midnight_with_seconds() {
        // Now is 00:04:30 and the previous track started 23:58:00, so the gap
        // crosses midnight and the earlier row must land on the previous day.
        let now = 86_400 * 2 + 4 * 60 + 30;
        let body = concat!(
            r#"{"mscp":{"playlist":["#,
            r#"{"time":"00:04:30","title":"A"},"#,
            r#"{"time":"23:58:00","title":"B"}"#,
            r#"]}}"#
        );
        let list = build(body, &STREAMSB_KEYS, now);
        assert_eq!(list.tracks[0].start, Some(now));
        assert_eq!(list.tracks[1].start, Some(now - 6 * 60 - 30));
    }

    #[test]
    fn clock_walk_handles_the_first_second_of_a_day() {
        // The worst case for the wrap: the entry playing now is stamped
        // `00:00:00`, which is also what the panel's refusal placeholder uses.
        let now = 86_400 * 3;
        let body = concat!(
            r#"{"mscp":{"playlist":["#,
            r#"{"time":"00:00:00","title":"A"},"#,
            r#"{"time":"23:56:12","title":"B"}"#,
            r#"]}}"#
        );
        let list = build(body, &STREAMSB_KEYS, now);
        assert_eq!(list.tracks[0].start, Some(now));
        assert_eq!(list.tracks[1].start, Some(now - (3 * 60 + 48)));
    }

    /// A body that parses but describes nothing is the state a refused request
    /// leaves behind, and it is reported as a hit unless the registry says the
    /// title is a known placeholder.
    #[test]
    fn the_refusal_placeholder_is_not_a_tracklist() {
        let refused = concat!(
            r#"{"mscp":{"playlist":[{"time":"00:00:00","#,
            r#""artist":"You can view the playlist on our website.","#,
            r#""title":"dancewave.online"}]}}"#
        );
        // It does parse — the guard is about the title, not the shape.
        assert_eq!(build(refused, &STREAMSB_KEYS, 1).tracks.len(), 1);
        let src = matching("dancewave.online")[0];
        let list = build(refused, &src.keys, 1);
        let title = list.now().map(|t| t.title.as_str()).unwrap_or_default();
        assert!(src.placeholder.contains(&title), "{title:?}");
        // And the real tracklist must not be mistaken for the placeholder.
        let real = build(DANCEWAVE, &src.keys, 1);
        let real_title = real.now().map(|t| t.title.as_str()).unwrap_or_default();
        assert!(!src.placeholder.contains(&real_title), "{real_title:?}");
    }

    /// The dancewave entry is the only one that cannot be built from the stream
    /// host, so its user and mount are literal. Getting either wrong returns a
    /// well-formed placeholder rather than an error, which is why they are
    /// pinned here instead of being left to the first person to watch it fail.
    #[test]
    fn dancewave_requests_the_real_station() {
        let src = matching("dancewave.online")[0];
        let url = expand(src, "dancewave.online", "/", "ignored");
        assert!(!url.contains('{'), "{url}");
        assert!(url.contains("user=dw8080"), "{url}");
        assert!(url.contains("mount=%2Fdw.ogg"), "{url}");
        assert!(url.contains("streamid=1"), "{url}");
        assert!(url.contains("out=json"), "{url}");
        // The whole gate travels with the entry.
        assert!(src.headers.iter().any(|(n, _)| *n == "Referer"));
        assert!(src.headers.iter().any(|(n, _)| *n == "User-Agent"));
    }

    #[test]
    fn a_body_with_a_lone_surrogate_still_parses() {
        // `serde_json` rejects an unpaired `\uD800` outright, which loses the
        // entire list over one bad character in one title.
        let body = concat!(
            r#"{"songs":[{"title":"A","artist":"X","date":"1790434286"},"#,
            r#"{"title":"bad \ud800 title","artist":"Y","date":"1790434287"}]}"#
        );
        assert!(serde_json::from_str::<serde_json::Value>(body).is_err());
        let list = build(body, &SOMAFM_KEYS, 1_790_440_000);
        assert_eq!(list.tracks.len(), 2, "one bad row cost the whole list");
        assert_eq!(list.tracks[0].title, "A");
        assert!(list.tracks[1].title.contains('\u{fffd}'));
    }

    #[test]
    fn surrogate_pairs_are_left_intact() {
        // A valid pair is not a lone surrogate and must decode to the astral
        // character, not be replaced.
        let body = r#"{"songs":[{"title":"\ud83c\udfb5 now","artist":"X","date":"1790434286"}]}"#;
        assert!(serde_json::from_str::<serde_json::Value>(body).is_ok());
        let list = build(body, &SOMAFM_KEYS, 1_790_440_000);
        assert_eq!(list.tracks[0].title, "🎵 now");
    }

    #[test]
    fn scrubbing_only_copies_when_it_changes_something() {
        // The common case is a body with no escapes at all; it should be
        // borrowed rather than rebuilt.
        let plain = r#"{"songs":[]}"#;
        assert!(matches!(
            scrub_surrogates(plain),
            std::borrow::Cow::Borrowed(_)
        ));
        // A `\u` that is not a surrogate is copied through unchanged.
        let escaped = r#"{"a":"café"}"#;
        assert_eq!(&*scrub_surrogates(escaped), escaped);
        // Escaped ASCII is common and must survive untouched.
        let ascii_escapes = r#"{"a":"\u00e9\ud83c\udfb5"}"#;
        assert_eq!(&*scrub_surrogates(ascii_escapes), ascii_escapes);
        // A truncated escape at the end of the body must not panic.
        for tail in [r#"\u"#, r#"\uD"#, r#"\uD8"#, r#"\uD83"#, r#"\ud83c"#] {
            let body = format!(r#"{{"a":"{tail}"}}"#);
            let _ = scrub_surrogates(&body);
        }
    }

    #[test]
    fn list_match_title_falls_back_to_leading_entry() {
        let list = RadioTracklist {
            tracks: vec![row("Silent Tears", None), row("Virtual Self", None)],
            at: 0,
            at_time: 1,
        };
        assert_eq!(list.match_title("Virtual Self"), 1);
        assert_eq!(list.match_title("Something Else Entirely"), 0);
        // A single word carries too little signal to move off the leading
        // entry, which is the on-air track anyway.
        assert_eq!(list.match_title("Tears"), 0);
    }
}
