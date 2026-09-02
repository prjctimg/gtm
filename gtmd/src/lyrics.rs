// Copyright (c) 2026
// Author: prjctimg <prjctimg@outlook.com>
// LRC lyrics fetching from lrclib.net
//
// This is free software released under the GPL-3.0 license.

use std::path::{Path, PathBuf};

use reqwest::Client;
use urlencoding::encode;

use gtm_core::track::{LrcData, LrcLine, TrackInfo};

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

/// Check if two strings match fuzzily above threshold.
fn fuzzy_match(a: &str, b: &str) -> bool {
    jaro_winkler_similarity(a, b) >= FUZZY_THRESHOLD
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
                    let text = rest.trim().to_string();
                    for ts in timestamps {
                        lines.push(LrcLine {
                            timestamp: ts,
                            text: text.clone(),
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
            self.fetch_lrclib_search_title(&track.title).await
        } else if let Some(lrc) = self.fetch_lrclib_exact(track).await {
            Some(lrc)
        } else {
            self.fetch_lrclib_search(track).await
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
        let query = format!("{} {}", artist, title);
        let url = format!("{}/search?q={}", LRCLIB_API, encode(&query));

        for attempt in 0..2 {
            if let Ok(resp) = self.client.get(&url).send().await
                && resp.status().is_success()
                && let Ok(results) = resp.json::<Vec<serde_json::Value>>().await
            {
                // Try fuzzy match first
                for result in &results {
                    let artist_name = result.get("artistName")?.as_str()?;
                    let track_name = result.get("trackName")?.as_str()?;
                    if fuzzy_match(artist_name, artist) && fuzzy_match(track_name, title) {
                        return parse_lrclib_response(result);
                    }
                }
                // Fallback to first result
                return results.first().and_then(parse_lrclib_response);
            }
            if attempt == 0 {
                tokio::time::sleep(std::time::Duration::from_millis(200)).await;
            }
        }
        None
    }

    async fn read_sidecar(&self, track: &TrackInfo) -> Option<LrcData> {
        let path = Path::new(&track.path);
        let lrc_path = path.with_extension("lrc");
        if !lrc_path.exists() {
            return None;
        }
        let content = std::fs::read_to_string(&lrc_path).ok()?;
        Some(Self::parse_lrc(&content))
    }

    /// Resolve the cached `.lrc` file for a track. Cache entries are keyed by
    /// a sanitized "Artist - Title" (plus a short content hash) so the same
    /// song shares one entry regardless of where the file lives.
    fn cache_path(&self, track: &TrackInfo) -> Option<PathBuf> {
        let dir = self.cache_dir.as_ref()?;
        let key = if !track.artist.is_empty() || !track.title.is_empty() {
            format!("{} - {}", track.artist, track.title)
        } else {
            Path::new(&track.path)
                .file_stem()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_default()
        };
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

    fn read_cache(&self, track: &TrackInfo) -> Option<LrcData> {
        let path = self.cache_path(track)?;
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
        if let Some(path) = self.cache_path(track) {
            let _ = std::fs::write(&path, lrc_to_text(lrc));
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
                    let artist_name = result.get("artistName")?.as_str()?;
                    let track_name = result.get("trackName")?.as_str()?;
                    if fuzzy_match(artist_name, &track.artist)
                        && fuzzy_match(track_name, &track.title)
                    {
                        return parse_lrclib_response(result);
                    }
                }
                return results.first().and_then(parse_lrclib_response);
            }
            if attempt == 0 {
                tokio::time::sleep(std::time::Duration::from_millis(200)).await;
            }
        }
        None
    }

    async fn fetch_lrclib_search_title(&self, title: &str) -> Option<LrcData> {
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
            let track_name = result.get("trackName")?.as_str()?;
            if fuzzy_match(track_name, title) {
                return parse_lrclib_response(result);
            }
        }
        results.first().and_then(parse_lrclib_response)
    }
}

/// Derive `(artist, title)` from a file path when track tags are missing.
/// Parses "Artist - Title" from the file stem and strips common filler tags
/// via [`crate::cleaner::clean_filename_stem`].
pub fn meta_from_filename(path: &str) -> (String, String) {
    let stem = Path::new(path)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_string();
    let (artist, title) = crate::cleaner::clean_filename_stem(&stem);
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
        } else {
            let mins = (line.timestamp / 60.0) as u64;
            let secs = line.timestamp - (mins as f64 * 60.0);
            content.push_str(&format!("[{:02}:{:05.2}]{}\n", mins, secs, line.text));
        }
    }
    content
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

#[cfg(test)]
mod tests {
    use super::*;

    fn line(timestamp: f64, text: &str) -> LrcLine {
        LrcLine {
            timestamp,
            text: text.to_string(),
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
    fn lrclib_search_returns_lyrics() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let manager = LyricsManager::new();
        let result = rt.block_on(manager.search("The Weeknd", "Blinding Lights"));
        assert!(result.is_some(), "expected a hit for a well-known track");
    }

    #[test]
    fn parse_lrc_keeps_plain_lines() {
        let lrc = LyricsManager::parse_lrc("Line one\nLine two\n\n[00:01.00]timed line");
        assert_eq!(lrc.lines.len(), 3);
        // Timed lines sort first, plain lines go to the end.
        assert_eq!(lrc.lines[0].text, "timed line");
        assert_eq!(lrc.lines[0].timestamp, 1.0);
        assert_eq!(lrc.lines[1].text, "Line one");
        assert!(lrc.lines[1].timestamp < 0.0);
        assert_eq!(lrc.lines[2].text, "Line two");
    }

    #[test]
    fn parse_lrc_applies_offset_tag() {
        let lrc = LyricsManager::parse_lrc("[offset:+500]\n[00:10.00]shifted earlier");
        assert!((lrc.lines[0].timestamp - 9.5).abs() < 1e-6);
        let lrc = LyricsManager::parse_lrc("[offset:-1000ms]\n[00:10.00]shifted later");
        assert!((lrc.lines[0].timestamp - 11.0).abs() < 1e-6);
        // Shift never produces negative timestamps.
        let lrc = LyricsManager::parse_lrc("[offset:9999]\n[00:01.00]clamped");
        assert_eq!(lrc.lines[0].timestamp, 0.0);
    }

    #[test]
    fn parse_lrc_skips_tag_lines() {
        let lrc = LyricsManager::parse_lrc("[length:03:30]\n[00:01.00]real line");
        assert_eq!(lrc.lines.len(), 1);
        assert_eq!(lrc.lines[0].text, "real line");
    }

    #[test]
    fn meta_from_filename_parses_artist_title() {
        let (artist, title) = meta_from_filename("/tmp/music/Artist Name - Song Title.flac");
        assert_eq!(artist, "Artist Name");
        assert_eq!(title, "Song Title");
    }

    #[test]
    fn meta_from_filename_defaults_to_stem() {
        let (artist, title) = meta_from_filename("/tmp/music/Just A Title.mp3");
        assert!(artist.is_empty());
        assert_eq!(title, "Just A Title");
    }

    #[test]
    fn meta_from_filename_strips_official_tags() {
        let (artist, title) =
            meta_from_filename("/tmp/music/Drake - God's Plan (Official Audio).flac");
        assert_eq!(artist, "Drake");
        assert_eq!(title, "God's Plan");
    }

    #[test]
    fn lrc_to_text_round_trips_through_parse() {
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
    fn cache_round_trips_lyrics() {
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
    fn cache_key_stable_across_paths() {
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
        assert_eq!(manager.cache_path(&a), manager.cache_path(&b));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
