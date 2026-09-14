// Copyright (c) 2026
// Author: prjctimg <prjctimg@outlook.com>
// LRC lyrics fetching from lrclib.net
//
// This is free software released under the GPL-3.0 license.

use std::path::{Path, PathBuf};

use reqwest::Client;
use urlencoding::encode;

use gtm_core::track::{LrcData, LrcLine, LrcWord, TrackInfo};

use crate::cleaner::clean_filename_stem;

const LRCLIB_API: &str = "https://lrclib.net/api";

/// Similarity threshold for fuzzy matching artist/title against search results.
const FUZZY_THRESHOLD: f64 = 0.75;

/// Calculate Jaro-Winkler similarity between two strings (0.0 to 1.0).
/// Used for fuzzy matching track/artist names against search results.
fn jaro_winkler_similarity(a: &str, b: &str) -> f64 {
    let a = a.to_lowercase();
    let b = b.to_lowercase();

    if a == b {
        return 1.0;
    }
    if a.is_empty() || b.is_empty() {
        return 0.0;
    }

    let a_chars: Vec<char> = a.chars().collect();
    let b_chars: Vec<char> = b.chars().collect();
    let a_len = a_chars.len();
    let b_len = b_chars.len();

    let match_distance = (a_len.max(b_len) / 2).max(1) - 1;
    let mut a_matches = vec![false; a_len];
    let mut b_matches = vec![false; b_len];
    let mut matches = 0;
    let mut transpositions = 0;

    for i in 0..a_len {
        let start = i.saturating_sub(match_distance);
        let end = (i + match_distance + 1).min(b_len);
        for j in start..end {
            if b_matches[j] {
                continue;
            }
            if a_chars[i] != b_chars[j] {
                continue;
            }
            a_matches[i] = true;
            b_matches[j] = true;
            matches += 1;
            break;
        }
    }

    if matches == 0 {
        return 0.0;
    }

    let mut k = 0;
    for i in 0..a_len {
        if !a_matches[i] {
            continue;
        }
        while !b_matches[k] {
            k += 1;
        }
        if a_chars[i] != b_chars[k] {
            transpositions += 1;
        }
        k += 1;
    }

    let jaro = (matches as f64 / a_len as f64
        + matches as f64 / b_len as f64
        + (matches as f64 - transpositions as f64 / 2.0) / matches as f64)
        / 3.0;

    let prefix_len = a_chars
        .iter()
        .zip(b_chars.iter())
        .take_while(|(ca, cb)| ca == cb)
        .count()
        .min(4);

    jaro + (0.1 * prefix_len as f64 * (1.0 - jaro))
}

/// Strip clutter that should not participate in similarity matching: case,
/// punctuation and non-alphanumerics. This lets variants like "Song (feat. X)"
/// or "Sömeone – Rêmix" fuzzy-match a plain "Song"/"Remix" result instead of
/// being rejected at the threshold.
fn normalize_for_match(s: &str) -> String {
    s.to_lowercase()
        .chars()
        .filter(|c| c.is_alphanumeric())
        .collect()
}

/// Check if two strings match fuzzily above the configured threshold,
/// comparing normalized forms.
fn fuzzy_match(a: &str, b: &str) -> bool {
    jaro_winkler_similarity(&normalize_for_match(a), &normalize_for_match(b)) >= FUZZY_THRESHOLD
}

#[derive(Clone)]
pub struct LyricsManager {
    client: Client,
    /// Directory where fetched lyrics are persisted (one `.lrc` file per
    /// track) so they can be reused without a network connection.
    cache_dir: Option<PathBuf>,
}

impl Default for LyricsManager {
    fn default() -> Self {
        Self::new()
    }
}

impl LyricsManager {
    pub fn new() -> Self {
        let client = Client::builder()
            .user_agent("gtm/0.2 (+https://github.com/prjctimg/gtm.rs)")
            .build()
            .unwrap_or_else(|_| Client::new());
        Self {
            client,
            cache_dir: None,
        }
    }

    /// Create a manager that persists fetched lyrics under `dir`, keyed by a
    /// sanitized "Artist - Title", so they are available offline.
    pub fn with_cache_dir(dir: PathBuf) -> Self {
        let _ = std::fs::create_dir_all(&dir);
        Self {
            client: Client::new(),
            cache_dir: Some(dir),
        }
    }

    pub fn parse_lrc(content: &str) -> LrcData {
        let mut title = None;
        let mut artist = None;
        let mut album = None;
        // Global timing shift in milliseconds ([offset:+ms] shifts lines
        // earlier, negative later).
        let mut offset_ms: f64 = 0.0;
        let mut lines: Vec<LrcLine> = Vec::new();

        for raw in content.lines() {
            let line = raw.trim();
            if line.is_empty() {
                continue;
            }

            if let Some(meta) = line.strip_prefix("[ti:") {
                title = Some(meta.trim_end_matches(']').trim().to_string());
                continue;
            }
            if let Some(meta) = line.strip_prefix("[ar:") {
                artist = Some(meta.trim_end_matches(']').trim().to_string());
                continue;
            }
            if let Some(meta) = line.strip_prefix("[al:") {
                album = Some(meta.trim_end_matches(']').trim().to_string());
                continue;
            }
            if let Some(meta) = line.strip_prefix("[offset:") {
                let cleaned = meta
                    .trim_end_matches(']')
                    .trim()
                    .trim_end_matches("ms")
                    .trim();
                if let Ok(v) = cleaned.parse::<f64>()
                    && v.is_finite()
                    && v.abs() < 3_600_000.0
                {
                    offset_ms = v;
                }
                continue;
            }

            // Try to extract one or more timestamps like [00:01.00][00:05.00]text
            if line.starts_with('[') {
                let mut rest = line;
                let mut timestamps: Vec<f64> = Vec::new();
                loop {
                    if !rest.starts_with('[') {
                        break;
                    }
                    let Some(close) = rest.find(']') else {
                        break;
                    };
                    let ts_str = &rest[1..close];
                    if let Some(ts) = parse_lrc_timestamp(ts_str) {
                        timestamps.push(ts);
                        rest = rest[close + 1..].trim_start();
                        // Continue if next char is '[' (multi-timestamp)
                        if rest.starts_with('[') {
                            continue;
                        }
                        break;
                    } else {
                        // Malformed timestamp: stop trying to parse more
                        break;
                    }
                }
                if !timestamps.is_empty() {
                    let (text, words) = strip_word_timings(rest.trim());
                    for ts in timestamps {
                        lines.push(LrcLine {
                            timestamp: ts,
                            text: text.clone(),
                            words: words.clone(),
                        });
                    }
                    continue;
                }
                // If it looked like a timestamp block but parsing failed,
                // treat tag lines like [length:...] as skippable.
                if line.starts_with("[length:") || line.starts_with("[by:") {
                    continue;
                }
            }

            // Untimed line (plain lyrics): keep it with a sentinel timestamp
            // so non-synced lyrics still display instead of being dropped.
            if !line.starts_with('[') {
                lines.push(LrcLine {
                    timestamp: -1.0,
                    text: line.to_string(),
                    words: Vec::new(),
                });
            }
        }

        // Sort timed lines by timestamp so multi-timestamp expansion
        // and out-of-order input both render chronologically. Plain lines
        // (timestamp < 0) stay at the end in original order.
        let mut timed: Vec<LrcLine> = lines
            .iter()
            .filter(|l| l.timestamp >= 0.0)
            .cloned()
            .collect();
        let plain: Vec<LrcLine> = lines
            .iter()
            .filter(|l| l.timestamp < 0.0)
            .cloned()
            .collect();
        timed.sort_by(|a, b| {
            a.timestamp
                .partial_cmp(&b.timestamp)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        if offset_ms != 0.0 {
            let shift = offset_ms / 1000.0;
            for l in &mut timed {
                l.timestamp = (l.timestamp - shift).max(0.0);
            }
        }
        timed.extend(plain);

        LrcData {
            title,
            artist,
            album,
            lines: timed,
        }
    }

    /// Render a `.srt` subtitle file into timed lyric lines (one `LrcLine` per
    /// cue, using the cue start time). Returns `None` when the content has no
    /// parseable cues.
    pub fn parse_srt(content: &str) -> Option<LrcData> {
        parse_srt(content)
    }

    /// Parse a timed JSON lyrics file: either a full `LrcData`-shaped object
    /// (`{"title": ..., "lines": [{"time": 12.5, "text": "..."}]}`) or a bare
    /// line list under `lines` / `lyrics`. `time` (or `start`) entries are in
    /// seconds.
    pub fn parse_json_timed(content: &str) -> Option<LrcData> {
        parse_json_timed(content)
    }

    pub async fn get_lyrics(&self, track: &TrackInfo) -> Option<LrcData> {
        // 1. Check the .lrc sidecar next to the audio file first.
        if let Some(lrc) = self.read_sidecar(track).await {
            return Some(lrc);
        }

        // 2. Check the offline cache before hitting the network.
        if let Some(lrc) = self.read_cache(track) {
            return Some(lrc);
        }

        // 3. lrclib's exact lookup requires an artist; tracks with missing
        //    tags (e.g. queued/foreign files) fall back to a title-only
        //    search.
        let fetched = if track.artist.is_empty() {
            self.fetch_lrclib_title(&track.title).await
        } else if let Some(lrc) = self.fetch_lrclib_exact(track).await {
            Some(lrc)
        } else if let Some(lrc) = self.fetch_lrclib_search(track).await {
            Some(lrc)
        } else {
            // Strict artist+title matching failed (e.g. the artist tag is
            // spelled differently or carries a featuring/remix credit). Drop
            // to a title-only search so a correct song is still recovered;
            // the tight title threshold keeps an unrelated song's lyrics out.
            self.fetch_lrclib_title(&track.title).await
        };

        if let Some(lrc) = fetched {
            if !lrc.lines.is_empty() {
                self.write_cache(track, &lrc);
            }
            return Some(lrc);
        }

        None
    }

    /// Search lrclib for a free-form "Artist - Title" pair, returning the best
    /// match without touching sidecar files. Used by the `gtm lyrics` CLI.
    pub async fn search(&self, artist: &str, title: &str) -> Option<LrcData> {
        let key = format!("{} - {}", artist, title);
        // Serve from the disk cache first: this path is used by the CLI and by
        // `status --stream`, which would otherwise hit the network on every run.
        if let Some(lrc) = self.read_cache_key(&key) {
            return Some(lrc);
        }

        let query = format!("{} {}", artist, title);
        let url = format!("{}/search?q={}", LRCLIB_API, encode(&query));

        for attempt in 0..2 {
            if let Ok(resp) = self.client.get(&url).send().await
                && resp.status().is_success()
                && let Ok(results) = resp.json::<Vec<serde_json::Value>>().await
            {
                // Try fuzzy match first; malformed entries are skipped so a
                // single bad hit can't abort the whole list.
                for result in &results {
                    let Some(artist_name) = result.get("artistName").and_then(|v| v.as_str())
                    else {
                        continue;
                    };
                    let Some(track_name) = result.get("trackName").and_then(|v| v.as_str()) else {
                        continue;
                    };
                    if fuzzy_match(artist_name, artist)
                        && fuzzy_match(track_name, title)
                        && let Some(lrc) = parse_lrclib_response(result)
                    {
                        self.write_cache_key(&key, &lrc);
                        return Some(lrc);
                    }
                }
                // No fuzzy match on artist+title: prefer no lyrics over a
                // wrong song's, so a mismatched first hit is never served.
                return None;
            }
            if attempt == 0 {
                tokio::time::sleep(std::time::Duration::from_millis(200)).await;
            }
        }
        None
    }

    async fn read_sidecar(&self, track: &TrackInfo) -> Option<LrcData> {
        let path = Path::new(&track.path);
        // Try an `.lrc` sidecar first, then `.srt` / timed `.json` sources.
        for ext in ["lrc", "srt", "json"] {
            let sidecar = path.with_extension(ext);
            let Ok(content) = tokio::fs::read_to_string(&sidecar).await else {
                continue;
            };
            let lrc = match ext {
                "lrc" => Self::parse_lrc(&content),
                "srt" => match parse_srt(&content) {
                    Some(lrc) => lrc,
                    None => continue,
                },
                _ => match parse_json_timed(&content) {
                    Some(lrc) => lrc,
                    None => continue,
                },
            };
            // An empty or malformed sidecar must not shadow the offline cache
            // or the network fetch, otherwise a stray/empty sidecar file would
            // make a track permanently lyric-less.
            if !lrc.lines.is_empty() {
                return Some(lrc);
            }
        }
        None
    }

    /// Resolve the cached `.lrc` file for a key. Cache entries are keyed by
    /// a sanitized "Artist - Title" (plus a short content hash) so the same
    /// song shares one entry regardless of where the file lives.
    fn cache_path(&self, key: &str) -> Option<PathBuf> {
        let dir = self.cache_dir.as_ref()?;
        let sanitized: String = key
            .chars()
            .filter(|c| c.is_ascii_alphanumeric() || *c == '-')
            .take(80)
            .collect::<String>()
            .to_lowercase();
        let mut hash = 0u64;
        for b in key.bytes() {
            hash = hash.wrapping_mul(31).wrapping_add(u64::from(b));
        }
        let name = if sanitized.is_empty() {
            format!("{:05x}.lrc", hash % 1_000_000)
        } else {
            format!("{}-{:05x}.lrc", sanitized, hash % 1_000_000)
        };
        Some(dir.join(name))
    }

    /// Build the cache key for a track: "Artist - Title", falling back to the
    /// file stem when no tags are present.
    fn track_cache_key(&self, track: &TrackInfo) -> Option<String> {
        if !track.artist.is_empty() || !track.title.is_empty() {
            Some(format!("{} - {}", track.artist, track.title))
        } else {
            Path::new(&track.path)
                .file_stem()
                .map(|s| s.to_string_lossy().to_string())
                .filter(|k| !k.is_empty())
        }
    }

    fn read_cache(&self, track: &TrackInfo) -> Option<LrcData> {
        let key = self.track_cache_key(track)?;
        self.read_cache_key(&key)
    }

    fn read_cache_key(&self, key: &str) -> Option<LrcData> {
        let path = self.cache_path(key)?;
        if !path.exists() {
            return None;
        }
        let content = std::fs::read_to_string(&path).ok()?;
        let lrc = Self::parse_lrc(&content);
        if lrc.lines.is_empty() {
            None
        } else {
            Some(lrc)
        }
    }

    fn write_cache(&self, track: &TrackInfo, lrc: &LrcData) {
        if let Some(key) = self.track_cache_key(track)
            && let Some(path) = self.cache_path(&key)
        {
            let _ = std::fs::write(path, lrc_to_text(lrc));
        }
    }

    /// Persist a fetched result under an explicit "Artist - Title" key (used by
    /// the CLI `search` path, which has no `TrackInfo`).
    fn write_cache_key(&self, key: &str, lrc: &LrcData) {
        if let Some(path) = self.cache_path(key) {
            let _ = std::fs::write(path, lrc_to_text(lrc));
        }
    }

    async fn fetch_lrclib_exact(&self, track: &TrackInfo) -> Option<LrcData> {
        let mut url = format!(
            "{}/get?artist_name={}&track_name={}&album_name={}",
            LRCLIB_API,
            encode(&track.artist),
            encode(&track.title),
            encode(&track.album),
        );
        if track.duration >= 1.0 {
            url.push_str(&format!("&duration={}", track.duration as u64));
        }

        let resp = self.client.get(&url).send().await.ok()?;
        if !resp.status().is_success() {
            return None;
        }

        let json: serde_json::Value = resp.json().await.ok()?;
        parse_lrclib_response(&json)
    }

    async fn fetch_lrclib_search(&self, track: &TrackInfo) -> Option<LrcData> {
        let query = format!("{} {}", track.artist, track.title);
        let url = format!("{}/search?q={}", LRCLIB_API, encode(&query));

        for attempt in 0..2 {
            if let Ok(resp) = self.client.get(&url).send().await
                && resp.status().is_success()
                && let Ok(results) = resp.json::<Vec<serde_json::Value>>().await
            {
                // Try fuzzy match first
                for result in &results {
                    let Some(artist_name) = result.get("artistName").and_then(|v| v.as_str())
                    else {
                        continue;
                    };
                    let Some(track_name) = result.get("trackName").and_then(|v| v.as_str()) else {
                        continue;
                    };
                    if fuzzy_match(artist_name, &track.artist)
                        && fuzzy_match(track_name, &track.title)
                    {
                        return parse_lrclib_response(result);
                    }
                }
                // No fuzzy match: skip the first-hit fallback so a wrong
                // song's lyrics are never shown for this track.
                return None;
            }
            if attempt == 0 {
                tokio::time::sleep(std::time::Duration::from_millis(200)).await;
            }
        }
        None
    }

    async fn fetch_lrclib_title(&self, title: &str) -> Option<LrcData> {
        if title.is_empty() {
            return None;
        }
        let url = format!("{}/search?q={}", LRCLIB_API, encode(title));

        let resp = self.client.get(&url).send().await.ok()?;
        if !resp.status().is_success() {
            return None;
        }

        let results: Vec<serde_json::Value> = resp.json().await.ok()?;
        // Try fuzzy match against title
        for result in &results {
            let Some(track_name) = result.get("trackName").and_then(|v| v.as_str()) else {
                continue;
            };
            if fuzzy_match(track_name, title) {
                return parse_lrclib_response(result);
            }
        }
        // No fuzzy title match: don't serve an unrelated song's lyrics.
        None
    }
}

/// Derive `(artist, title)` from a file path when track tags are missing.
/// Parses "Artist - Title" from the file stem and strips common filler tags
/// via [`clean_filename_stem`].
pub fn meta_from_filename(path: &str) -> (String, String) {
    let stem = Path::new(path)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_string();
    let (artist, title) = clean_filename_stem(&stem);
    (artist.unwrap_or_default(), title)
}

/// Serialize parsed lyrics back to LRC text. Timed lines keep their
/// timestamps; untimed lines are written as plain text so they round-trip
/// through [`LyricsManager::parse_lrc`].
pub fn lrc_to_text(lrc: &LrcData) -> String {
    let mut content = String::new();
    if let Some(ref ar) = lrc.artist {
        content.push_str(&format!("[ar:{}]\n", ar));
    }
    if let Some(ref al) = lrc.album {
        content.push_str(&format!("[al:{}]\n", al));
    }
    if let Some(ref ti) = lrc.title {
        content.push_str(&format!("[ti:{}]\n", ti));
    }
    for line in &lrc.lines {
        if line.timestamp < 0.0 {
            content.push_str(&line.text);
            content.push('\n');
        } else if !line.words.is_empty() {
            // Enhanced-LRC line: keep per-word timings so cached copies
            // round-trip through [`LyricsManager::parse_lrc`] with karaoke
            // timing intact.
            let (mins, secs) = lrc_minutes_seconds(line.timestamp);
            content.push_str(&format!("[{:02}:{:05.2}]", mins, secs));
            for w in &line.words {
                let (wm, ws) = lrc_minutes_seconds(w.time);
                content.push_str(&format!("<{:02}:{:05.2}>{}", wm, ws, w.text));
            }
            content.push('\n');
        } else {
            let (mins, secs) = lrc_minutes_seconds(line.timestamp);
            content.push_str(&format!("[{:02}:{:05.2}]{}\n", mins, secs, line.text));
        }
    }
    content
}

/// Split a floating seconds value into `(minutes, remaining_seconds)` for LRC
/// timestamp formatting.
fn lrc_minutes_seconds(seconds: f64) -> (u64, f64) {
    let mins = (seconds / 60.0) as u64;
    let secs = seconds - (mins as f64 * 60.0);
    (mins, secs)
}

fn parse_lrclib_response(json: &serde_json::Value) -> Option<LrcData> {
    let synced = json.get("syncLyrics").and_then(|v| v.as_str());
    let plain = json.get("plainLyrics").and_then(|v| v.as_str());

    if let Some(s) = synced
        && !s.is_empty()
    {
        return Some(LyricsManager::parse_lrc(s));
    }

    if let Some(p) = plain
        && !p.is_empty()
    {
        return Some(LyricsManager::parse_lrc(p));
    }

    None
}

fn parse_lrc_timestamp(ts: &str) -> Option<f64> {
    // Hardened: reject non-ASCII/malformed fractions that previously caused
    // panics or incorrect large values.
    if ts.is_empty() || ts.len() > 16 {
        return None;
    }
    // Fractions must be ascii digits with optional '.'; reject others early.
    let has_fraction = ts.contains('.');
    if has_fraction {
        let dot = ts.find('.')?;
        let frac = &ts[dot + 1..];
        if frac.is_empty() || frac.len() > 3 || !frac.chars().all(|c| c.is_ascii_digit()) {
            return None;
        }
    }
    let parts: Vec<&str> = ts.split(':').collect();
    let parse_part = |s: &str| -> Option<f64> {
        // Allow decimal in final part only (ss or ss.frac)
        if s.contains('.') && s.matches('.').count() > 1 {
            return None;
        }
        s.parse::<f64>()
            .ok()
            .filter(|v| v.is_finite() && *v >= 0.0 && *v < 10000.0)
    };
    match parts.len() {
        2 => {
            let mm = parse_part(parts[0])?;
            let ss = parse_part(parts[1])?;
            if mm >= 1000.0 || ss >= 100.0 {
                return None;
            }
            let total = mm * 60.0 + ss;
            if total.is_finite() && total < 100000.0 {
                Some(total)
            } else {
                None
            }
        }
        3 => {
            let hh = parse_part(parts[0])?;
            let mm = parse_part(parts[1])?;
            let ss = parse_part(parts[2])?;
            if mm >= 60.0 || ss >= 100.0 {
                return None;
            }
            let total = hh * 3600.0 + mm * 60.0 + ss;
            if total.is_finite() && total < 200000.0 {
                Some(total)
            } else {
                None
            }
        }
        _ => None,
    }
}

/// Extract inline word timings from an enhanced-LRC lyric body
/// (`<00:12.50>Hel<00:12.90>lo ...`) into `(plain_text, words)`. The final
/// terminating token (e.g. `<00:13.00>` with nothing after it) is dropped.
/// If the body contains no `<mm:ss.xx>` word tokens the whole body is kept
/// as plain line text with no words.
fn strip_word_timings(body: &str) -> (String, Vec<LrcWord>) {
    let mut text = String::new();
    let mut words: Vec<LrcWord> = Vec::new();

    if !body.contains('<') {
        return (body.to_string(), words);
    }

    let mut rest = body;
    while let Some(open) = rest.find('<') {
        text.push_str(&rest[..open]);
        let after = &rest[open + 1..];
        let Some(close) = after.find('>') else {
            text.push_str(&rest[open..]);
            break;
        };
        let ts_str = &after[..close];
        rest = &after[close + 1..];
        let Some(ts) = parse_lrc_timestamp(ts_str) else {
            // Not a word-timing token: keep as literal text.
            text.push('<');
            text.push_str(ts_str);
            text.push('>');
            continue;
        };
        let end = rest.find('<').unwrap_or(rest.len());
        let word = rest[..end].trim().to_string();
        if !word.is_empty() {
            words.push(LrcWord {
                time: ts,
                text: word.clone(),
            });
            // Word separators live in the whitespace between tokens, which is
            // dropped when caching (lrc_to_text writes <ts>word back-to-back),
            // so rejoin with a single space unless one already trails the
            // accumulated text.
            if !text.is_empty() && !text.ends_with(char::is_whitespace) {
                text.push(' ');
            }
            text.push_str(&word);
        }
        if end < rest.len() {
            rest = &rest[end..];
        } else {
            break;
        }
    }

    if words.is_empty() {
        return (body.to_string(), Vec::new());
    }
    (text, words)
}

/// Parse a SubRip (`.srt`) subtitle stream into timed lyric lines. Each cue
/// becomes one `LrcLine` stamped with its start time; multi-line cue text is
/// joined with newlines. Returns `None` when no cues parse.
fn parse_srt(content: &str) -> Option<LrcData> {
    let mut lines: Vec<LrcLine> = Vec::new();
    let mut pending_start: Option<f64> = None;
    let mut pending_text: Vec<String> = Vec::new();

    let flush = |lines: &mut Vec<LrcLine>, start: &mut Option<f64>, text: &mut Vec<String>| {
        if let Some(ts) = start.take() {
            let body = text.join("\n");
            if !body.is_empty() {
                lines.push(LrcLine {
                    timestamp: ts,
                    text: body,
                    words: Vec::new(),
                });
            }
            text.clear();
        }
    };

    for raw in content.lines() {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            flush(&mut lines, &mut pending_start, &mut pending_text);
            continue;
        }
        if pending_start.is_none() {
            // Timing line (the cue index line is skipped).
            if let Some(arrow) = trimmed.find("-->")
                && let Some(ts) = parse_srt_timestamp(trimmed[..arrow].trim())
            {
                pending_start = Some(ts);
            }
            continue;
        }
        pending_text.push(raw.trim().to_string());
    }
    flush(&mut lines, &mut pending_start, &mut pending_text);

    if lines.is_empty() {
        return None;
    }
    lines.sort_by(|a, b| {
        a.timestamp
            .partial_cmp(&b.timestamp)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    Some(LrcData {
        title: None,
        artist: None,
        album: None,
        lines,
    })
}

fn parse_srt_timestamp(ts: &str) -> Option<f64> {
    let parts: Vec<&str> = ts.split(':').collect();
    if parts.len() != 3 {
        return None;
    }
    let h: f64 = parts[0].trim().parse().ok()?;
    let m: f64 = parts[1].trim().parse().ok()?;
    let s: f64 = parts[2].trim().replace(',', ".").parse().ok()?;
    let total = h * 3600.0 + m * 60.0 + s;
    ((0.0..200000.0).contains(&total)).then_some(total)
}

/// Parse a timed JSON lyrics document. Accepts a full `LrcData`-shaped object
/// or a bare line list under `lines` / `lyrics`; each entry needs a numeric
/// `time` (or `start`) in seconds and a `text` string. Returns `None` when the
/// document is malformed or has no timed lines.
fn parse_json_timed(content: &str) -> Option<LrcData> {
    let value: serde_json::Value = serde_json::from_str(content).ok()?;
    let title = value
        .get("title")
        .and_then(|v| v.as_str())
        .map(String::from);
    let artist = value
        .get("artist")
        .and_then(|v| v.as_str())
        .map(String::from);
    let album = value
        .get("album")
        .and_then(|v| v.as_str())
        .map(String::from);
    let entries = value
        .get("lines")
        .or_else(|| value.get("lyrics"))
        .and_then(|v| v.as_array())?;

    let mut lines: Vec<LrcLine> = Vec::new();
    for entry in entries {
        let time = entry
            .get("time")
            .or_else(|| entry.get("start"))
            .and_then(|v| v.as_f64())?;
        let text = entry.get("text").and_then(|v| v.as_str())?.to_string();
        if time.is_finite() && time >= 0.0 {
            lines.push(LrcLine {
                timestamp: time,
                text,
                words: Vec::new(),
            });
        }
    }
    sort_lyric_lines(&mut lines);
    if lines.is_empty() {
        return None;
    }
    Some(LrcData {
        title,
        artist,
        album,
        lines,
    })
}

fn sort_lyric_lines(lines: &mut [LrcLine]) {
    lines.sort_by(|a, b| {
        a.timestamp
            .partial_cmp(&b.timestamp)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(timestamp: f64, text: &str) -> LrcLine {
        LrcLine {
            timestamp,
            text: text.to_string(),
            words: Vec::new(),
        }
    }

    fn sample_lrc(lines: Vec<LrcLine>) -> LrcData {
        LrcData {
            title: Some("Some Song".to_string()),
            artist: Some("Some Artist".to_string()),
            album: Some("Some Album".to_string()),
            lines,
        }
    }

    fn temp_cache_dir(suffix: &str) -> PathBuf {
        std::env::temp_dir().join(format!("gtm-lyrics-{suffix}-{}", std::process::id()))
    }

    #[test]
    #[ignore = "requires network access to lrclib.net"]
    fn lrclib_returns_lyrics() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let manager = LyricsManager::new();
        let result = rt.block_on(manager.search("The Weeknd", "Blinding Lights"));
        assert!(result.is_some(), "expected a hit for a well-known track");
    }

    #[test]
    fn lrc_plain_lines() {
        let lrc = LyricsManager::parse_lrc("Line one\nLine two\n\n[00:01.00]timed line");
        assert_eq!(lrc.lines.len(), 3);
        // Timed lines sort first, plain lines go to the end.
        assert_eq!(lrc.lines[0].text, "timed line");
        assert_eq!(lrc.lines[0].timestamp, 1.0);
        assert_eq!(lrc.lines[1].text, "Line one");
        assert!(lrc.lines[1].timestamp < 0.0);
        assert_eq!(lrc.lines[2].text, "Line two");
        assert!(lrc.lines[0].words.is_empty());
    }

    #[test]
    fn lrc_enhanced_words() {
        let lrc = LyricsManager::parse_lrc(
            "[00:12.00]<00:12.00>Hello <00:12.60>world <00:13.20>tonight<00:13.80>",
        );
        assert_eq!(lrc.lines.len(), 1);
        let line = &lrc.lines[0];
        assert_eq!(line.timestamp, 12.0);
        assert_eq!(line.text, "Hello world tonight");
        assert_eq!(line.words.len(), 3);
        assert_eq!(line.words[0].time, 12.0);
        assert_eq!(line.words[0].text, "Hello");
        assert_eq!(line.words[1].time, 12.6);
        assert_eq!(line.words[2].time, 13.2);
        // The trailing `<00:13.80>` terminator must be dropped.
        assert_eq!(line.words[2].text, "tonight");

        // Enhanced lines round-trip through the cache text writer.
        let text = lrc_to_text(&lrc);
        let parsed = LyricsManager::parse_lrc(&text);
        assert_eq!(parsed.lines[0].text, line.text);
        assert_eq!(parsed.lines[0].words.len(), 3);
        assert_eq!(parsed.lines[0].words[1].time, 12.6);
    }

    #[test]
    fn srt_parses_cues() {
        let srt = "1\n00:00:00,500 --> 00:00:02,000\nHello there\n\n\
                   2\n00:00:03,000 --> 00:00:05,000\nSecond line\n  Third line\n";
        let lrc = LyricsManager::parse_srt(srt).expect("srt should parse");
        assert_eq!(lrc.lines.len(), 2);
        assert_eq!(lrc.lines[0].timestamp, 0.5);
        assert_eq!(lrc.lines[0].text, "Hello there");
        assert_eq!(lrc.lines[1].timestamp, 3.0);
        assert_eq!(lrc.lines[1].text, "Second line\nThird line");

        assert!(LyricsManager::parse_srt("not a subtitle").is_none());
    }

    #[test]
    fn json_timed_parses() {
        let json =
            r#"{"title":"Song","lines":[{"time":1.5,"text":"first"},{"start":3,"text":"second"}]}"#;
        let lrc = LyricsManager::parse_json_timed(json).expect("json should parse");
        assert_eq!(lrc.title.as_deref(), Some("Song"));
        assert_eq!(lrc.lines[0].timestamp, 1.5);
        assert_eq!(lrc.lines[0].text, "first");
        assert_eq!(lrc.lines[1].timestamp, 3.0);
        assert_eq!(lrc.lines[1].text, "second");

        let lyrics_key = r#"{"lyrics":[{"time":7.0,"text":"x"}]}"#;
        let lrc = LyricsManager::parse_json_timed(lyrics_key).expect("lyrics key should work");
        assert_eq!(lrc.lines[0].timestamp, 7.0);

        assert!(LyricsManager::parse_json_timed("not json").is_none());
    }

    #[test]
    fn lrc_offset_tag() {
        let lrc = LyricsManager::parse_lrc("[offset:+500]\n[00:10.00]shifted earlier");
        assert!((lrc.lines[0].timestamp - 9.5).abs() < 1e-6);
        let lrc = LyricsManager::parse_lrc("[offset:-1000ms]\n[00:10.00]shifted later");
        assert!((lrc.lines[0].timestamp - 11.0).abs() < 1e-6);
        // Shift never produces negative timestamps.
        let lrc = LyricsManager::parse_lrc("[offset:9999]\n[00:01.00]clamped");
        assert_eq!(lrc.lines[0].timestamp, 0.0);
    }

    #[test]
    fn lrc_skips_tags() {
        let lrc = LyricsManager::parse_lrc("[length:03:30]\n[00:01.00]real line");
        assert_eq!(lrc.lines.len(), 1);
        assert_eq!(lrc.lines[0].text, "real line");
    }

    #[test]
    fn meta_parses_artist() {
        let (artist, title) = meta_from_filename("/tmp/music/Artist Name - Song Title.flac");
        assert_eq!(artist, "Artist Name");
        assert_eq!(title, "Song Title");
    }

    #[test]
    fn meta_defaults_stem() {
        let (artist, title) = meta_from_filename("/tmp/music/Just A Title.mp3");
        assert!(artist.is_empty());
        assert_eq!(title, "Just A Title");
    }

    #[test]
    fn meta_strips_tags() {
        let (artist, title) =
            meta_from_filename("/tmp/music/Drake - God's Plan (Official Audio).flac");
        assert_eq!(artist, "Drake");
        assert_eq!(title, "God's Plan");
    }

    #[test]
    fn lrc_roundtrip() {
        let lrc = sample_lrc(vec![line(65.0, "timed line"), line(-1.0, "plain line")]);

        let text = lrc_to_text(&lrc);
        let parsed = LyricsManager::parse_lrc(&text);

        assert_eq!(parsed.lines.len(), 2);
        assert_eq!(parsed.lines[0].text, "timed line");
        assert_eq!(parsed.lines[0].timestamp, 65.0);
        assert_eq!(parsed.lines[1].text, "plain line");
        assert!(parsed.lines[1].timestamp < 0.0);
    }

    #[test]
    fn cache_lyrics_roundtrip() {
        let dir = temp_cache_dir("cache");
        let manager = LyricsManager::with_cache_dir(dir.clone());
        let track = TrackInfo {
            id: 1,
            path: "/tmp/music/Some Song.flac".to_string(),
            title: "Some Song".to_string(),
            artist: "Some Artist".to_string(),
            ..Default::default()
        };
        let lrc = sample_lrc(vec![line(0.0, "first"), line(-1.0, "plain line")]);

        assert!(manager.read_cache(&track).is_none(), "cold cache misses");
        manager.write_cache(&track, &lrc);

        let cached = manager.read_cache(&track).expect("cache should hit");
        assert_eq!(cached.lines.len(), 2);
        assert_eq!(cached.lines[0].text, "first");
        assert_eq!(cached.lines[0].timestamp, 0.0);
        assert_eq!(cached.lines[1].text, "plain line");
        assert!(cached.lines[1].timestamp < 0.0);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn cache_key_stable() {
        let dir = temp_cache_dir("key");
        let manager = LyricsManager::with_cache_dir(dir.clone());
        let a = TrackInfo {
            path: "/one/Artist - Song.mp3".to_string(),
            title: "Song".to_string(),
            artist: "Artist".to_string(),
            ..Default::default()
        };
        let b = TrackInfo {
            path: "/two/Artist - Song.flac".to_string(),
            title: "Song".to_string(),
            artist: "Artist".to_string(),
            ..Default::default()
        };
        assert_eq!(
            manager.cache_path(manager.track_cache_key(&a).as_deref().unwrap()),
            manager.cache_path(manager.track_cache_key(&b).as_deref().unwrap())
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
