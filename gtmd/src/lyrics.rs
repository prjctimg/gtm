// Copyright (c) 2026 - present
// Author: prjctimg <prjctimg@outlook.com>
// LRC lyrics fetching from lrclib.net
//
// This is free software released under the GPL-3.0 license.

use std::path::{Path, PathBuf};

use reqwest::Client;

use gtm_core::track::{LrcData, LrcLine, TrackInfo};

const LRCLIB_API: &str = "https://lrclib.net/api";

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
        Self {
            client: Client::new(),
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
        let mut lines = Vec::new();

        for line in content.lines() {
            let line = line.trim();
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

            if let Some((ts_str, text)) = parse_timestamp_line(line) {
                if let Some(ts) = parse_lrc_timestamp(ts_str) {
                    lines.push(LrcLine {
                        timestamp: ts,
                        text: text.to_string(),
                    });
                    continue;
                }
            }

            // Untimed line (plain lyrics): keep it with a sentinel timestamp
            // so non-synced lyrics still display instead of being dropped.
            // Tag lines like "[length:...]" are skipped.
            if !line.starts_with('[') {
                lines.push(LrcLine {
                    timestamp: -1.0,
                    text: line.to_string(),
                });
            }
        }

        LrcData {
            title,
            artist,
            album,
            lines,
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
        let url = format!("{}/search?q={}", LRCLIB_API, urlencoding(&query),);

        let resp = self.client.get(&url).send().await.ok()?;
        if !resp.status().is_success() {
            return None;
        }

        let results: Vec<serde_json::Value> = resp.json().await.ok()?;
        for result in &results {
            let artist_name = result.get("artistName")?.as_str()?.to_lowercase();
            let track_name = result.get("trackName")?.as_str()?.to_lowercase();
            if artist_name == artist.to_lowercase() && track_name == title.to_lowercase() {
                return parse_lrclib_response(result);
            }
        }
        results.first().and_then(parse_lrclib_response)
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
        let url = format!(
            "{}/get?artist={}&title={}&album={}",
            LRCLIB_API,
            urlencoding(&track.artist),
            urlencoding(&track.title),
            urlencoding(&track.album),
        );

        let resp = self.client.get(&url).send().await.ok()?;
        if !resp.status().is_success() {
            return None;
        }

        let json: serde_json::Value = resp.json().await.ok()?;
        parse_lrclib_response(&json)
    }

    async fn fetch_lrclib_search(&self, track: &TrackInfo) -> Option<LrcData> {
        let query = format!("{} {}", track.artist, track.title);
        let url = format!("{}/search?q={}", LRCLIB_API, urlencoding(&query),);

        let resp = self.client.get(&url).send().await.ok()?;
        if !resp.status().is_success() {
            return None;
        }

        let results: Vec<serde_json::Value> = resp.json().await.ok()?;
        for result in &results {
            let artist_name = result.get("artistName")?.as_str()?.to_lowercase();
            let track_name = result.get("trackName")?.as_str()?.to_lowercase();
            if artist_name == track.artist.to_lowercase()
                && track_name == track.title.to_lowercase()
            {
                return parse_lrclib_response(result);
            }
        }
        results.first().and_then(parse_lrclib_response)
    }

    /// Search lrclib by title alone, preferring an exact track-name match and
    /// otherwise accepting the top hit.  Used when the artist is unknown.
    async fn fetch_lrclib_search_title(&self, title: &str) -> Option<LrcData> {
        if title.is_empty() {
            return None;
        }
        let url = format!("{}/search?q={}", LRCLIB_API, urlencoding(title));

        let resp = self.client.get(&url).send().await.ok()?;
        if !resp.status().is_success() {
            return None;
        }

        let results: Vec<serde_json::Value> = resp.json().await.ok()?;
        let q = title.to_lowercase();
        for result in &results {
            let track_name = result.get("trackName")?.as_str()?.to_lowercase();
            if track_name == q {
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

    if let Some(s) = synced {
        if !s.is_empty() {
            return Some(LyricsManager::parse_lrc(s));
        }
    }

    if let Some(p) = plain {
        if !p.is_empty() {
            return Some(LyricsManager::parse_lrc(p));
        }
    }

    None
}

fn parse_timestamp_line(line: &str) -> Option<(&str, &str)> {
    let line = line.trim_start_matches('[');
    let (ts, rest) = line.split_once(']')?;
    Some((ts, rest.trim()))
}

fn parse_lrc_timestamp(ts: &str) -> Option<f64> {
    let parts: Vec<&str> = ts.split(':').collect();
    match parts.len() {
        2 => {
            let mm: f64 = parts[0].parse().ok()?;
            let ss: f64 = parts[1].parse().ok()?;
            Some(mm * 60.0 + ss)
        }
        3 => {
            let hh: f64 = parts[0].parse().ok()?;
            let mm: f64 = parts[1].parse().ok()?;
            let ss: f64 = parts[2].parse().ok()?;
            Some(hh * 3600.0 + mm * 60.0 + ss)
        }
        _ => None,
    }
}

fn urlencoding(s: &str) -> String {
    s.chars()
        .map(|c| match c {
            'A'..='Z' | 'a'..='z' | '0'..='9' | '-' | '_' | '.' | '~' => c.to_string(),
            ' ' => "%20".to_string(),
            other => format!("%{:02X}", other as u8),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

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
        assert_eq!(lrc.lines[0].text, "Line one");
        assert!(lrc.lines[0].timestamp < 0.0);
        assert_eq!(lrc.lines[1].text, "Line two");
        assert_eq!(lrc.lines[2].text, "timed line");
        assert_eq!(lrc.lines[2].timestamp, 1.0);
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
        let lrc = LrcData {
            title: Some("Some Song".to_string()),
            artist: Some("Some Artist".to_string()),
            album: Some("Some Album".to_string()),
            lines: vec![
                LrcLine {
                    timestamp: 65.0,
                    text: "timed line".to_string(),
                },
                LrcLine {
                    timestamp: -1.0,
                    text: "plain line".to_string(),
                },
            ],
        };

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
        let dir = std::env::temp_dir().join(format!("gtm-lyrics-cache-{}", std::process::id()));
        let manager = LyricsManager::with_cache_dir(dir.clone());
        let track = TrackInfo {
            id: 1,
            path: "/tmp/music/Some Song.flac".to_string(),
            title: "Some Song".to_string(),
            artist: "Some Artist".to_string(),
            ..Default::default()
        };
        let lrc = LrcData {
            title: Some("Some Song".to_string()),
            artist: Some("Some Artist".to_string()),
            album: Some("Some Album".to_string()),
            lines: vec![
                LrcLine {
                    timestamp: 0.0,
                    text: "first".to_string(),
                },
                LrcLine {
                    timestamp: -1.0,
                    text: "plain line".to_string(),
                },
            ],
        };

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
        let dir = std::env::temp_dir().join(format!("gtm-lyrics-key-{}", std::process::id()));
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
