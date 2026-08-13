// Copyright (c) 2025 - present
// Author: prjctimg <prjctimg@outlook.com>
// LRC lyrics fetching from lrclib.net
//
// This is free software released under the GPL-3.0 license.

use std::path::Path;

use reqwest::Client;

use gtm_core::track::{LrcData, LrcLine, TrackInfo};

const LRCLIB_API: &str = "https://lrclib.net/api";

#[derive(Clone)]
pub struct LyricsManager {
    client: Client,
}

impl LyricsManager {
    pub fn new() -> Self {
        Self {
            client: Client::new(),
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
        if let Some(lrc) = self.read_sidecar(track).await {
            return Some(lrc);
        }

        if let Some(lrc) = self.fetch_lrclib_exact(track).await {
            return Some(lrc);
        }

        if let Some(lrc) = self.fetch_lrclib_search(track).await {
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
}

/// Derive `(artist, title)` from a file path when track tags are missing.
/// Parses "Artist - Title" from the file stem and strips common filler tags
/// via [`crate::metadata_cleaner::clean_youtube_title`].
pub fn meta_from_filename(path: &str) -> (String, String) {
    let stem = Path::new(path)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_string();
    let (artist, title) = crate::metadata_cleaner::clean_youtube_title(&stem);
    (artist.unwrap_or_default(), title)
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
}
