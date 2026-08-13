use std::path::Path;

use reqwest::Client;


use gtm_core::track::{LrcData, LrcLine, TrackInfo};

const LRCLIB_API: &str = "https://lrclib.net/api";

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
                }
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
        let url = format!(
            "{}/search?q={}",
            LRCLIB_API,
            urlencoding(&query),
        );

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
