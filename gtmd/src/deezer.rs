// Copyright (c) 2026 - present
// Author: prjctimg <prjctimg@outlook.com>
// Deezer search for enriching unreliable track metadata
//
// This is free software released under the GPL-3.0 license.

use std::time::Duration;

use reqwest::Client;
use serde_json::Value;
use tracing::warn;

const DEEZER_API: &str = "https://api.deezer.com/search";
const RATE_LIMIT_MS: u64 = 200;
const REQUEST_TIMEOUT: Duration = Duration::from_secs(15);

/// Resolved track metadata from a Deezer search hit.
#[derive(Debug, Clone)]
pub struct DeezerTrack {
    pub title: String,
    pub artist: String,
    pub album: String,
    pub genre: Option<String>,
    pub year: Option<i32>,
    pub track_number: Option<i32>,
    pub duration: f64,
    pub cover_url: Option<String>,
}

pub struct DeezerSearch {
    client: Client,
}

impl Default for DeezerSearch {
    fn default() -> Self {
        Self::new()
    }
}

impl DeezerSearch {
    pub fn new() -> Self {
        Self {
            client: Client::builder()
                .timeout(REQUEST_TIMEOUT)
                .build()
                .unwrap_or_else(|_| Client::new()),
        }
    }

    /// Search Deezer for a track and return the best matching result.
    ///
    /// Results are scored against the expected artist/title/duration; a hit is
    /// only accepted when the title matches and the overall score clears a
    /// minimum bar, so bogus lookalikes are not written back.
    pub async fn search(
        &self,
        artist: &str,
        title: &str,
        duration: f64,
    ) -> Result<Option<DeezerTrack>, String> {
        let query = build_query(artist, title);
        if query.is_empty() {
            return Ok(None);
        }

        tokio::time::sleep(Duration::from_millis(RATE_LIMIT_MS)).await;

        let resp = self
            .client
            .get(DEEZER_API)
            .query(&[("q", query)])
            .send()
            .await
            .map_err(|e| format!("Deezer request failed: {e}"))?;
        let json: Value = resp
            .json()
            .await
            .map_err(|e| format!("Deezer JSON parse failed: {e}"))?;
        let data = match json.get("data").and_then(|d| d.as_array()) {
            Some(d) if !d.is_empty() => d,
            _ => return Ok(None),
        };

        Ok(best_match(data, artist, title, duration))
    }

    /// Download cover art bytes for an album/cover URL.
    pub async fn download_cover(&self, url: &str) -> Option<Vec<u8>> {
        match self.client.get(url).send().await {
            Ok(r) => match r.bytes().await {
                Ok(b) if !b.is_empty() => Some(b.to_vec()),
                Ok(_) => None,
                Err(e) => {
                    warn!("Failed to read cover bytes from {url}: {e}");
                    None
                }
            },
            Err(e) => {
                warn!("Failed to download cover from {url}: {e}");
                None
            }
        }
    }
}

/// Build a Deezer `q` parameter, quoted per-field for a tighter match.
///
/// The values are intentionally NOT percent-encoded here: the HTTP client
/// encodes the whole `q` value when building the URL, and pre-encoding would
/// double-encode it (breaking multi-byte titles in particular).
fn build_query(artist: &str, title: &str) -> String {
    let mut parts = Vec::new();
    let artist = artist.trim();
    let title = title.trim();
    if !artist.is_empty() {
        parts.push(format!("artist:\"{artist}\""));
    }
    if !title.is_empty() {
        parts.push(format!("track:\"{title}\""));
    }
    parts.join(" ")
}

/// Pick the highest-scoring result that plausibly matches the expected track.
fn best_match(results: &[Value], artist: &str, title: &str, duration: f64) -> Option<DeezerTrack> {
    let na = normalize(artist);
    let nt = normalize(title);

    let mut best: Option<(DeezerTrack, u32, bool)> = None;
    for r in results {
        let r_artist = r
            .pointer("/artist/name")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let r_title = r.get("title").and_then(|v| v.as_str()).unwrap_or("");
        let r_dur = r.get("duration").and_then(|v| v.as_f64()).unwrap_or(0.0);

        let ra = normalize(r_artist);
        let rt = normalize(r_title);
        let mut score = 0u32;

        if !na.is_empty() && !ra.is_empty() {
            if na == ra {
                score += 4;
            } else if contains_any(&na, &ra) {
                score += 2;
            }
        }
        let title_matched = !nt.is_empty() && !rt.is_empty() && {
            if nt == rt {
                score += 4;
                true
            } else if contains_any(&nt, &rt) {
                score += 2;
                true
            } else {
                false
            }
        };
        if duration > 0.0 && r_dur > 0.0 {
            let diff = (duration - r_dur).abs();
            if diff <= 3.0 {
                score += 3;
            } else if diff <= 10.0 {
                score += 2;
            } else if diff <= 20.0 {
                score += 1;
            }
        }

        if !title_matched {
            continue;
        }
        if best
            .as_ref()
            .is_none_or(|(_, best_score, _)| score > *best_score)
        {
            best = Some((to_deezer_track(r), score, title_matched));
        }
    }

    best.and_then(|(t, score, _)| (score >= 4).then_some(t))
}

fn to_deezer_track(r: &Value) -> DeezerTrack {
    let cover_url = r
        .get("album")
        .and_then(|a| a.get("cover_big"))
        .or_else(|| r.get("album").and_then(|a| a.get("cover_medium")))
        .or_else(|| r.get("cover_big"))
        .or_else(|| r.get("cover_medium"))
        .and_then(|c| c.as_str())
        .map(str::to_string);

    let genre = r
        .get("genres")
        .and_then(|g| g.get("data"))
        .and_then(|d| d.as_array())
        .and_then(|arr| arr.first())
        .and_then(|g| g.get("name"))
        .and_then(|n| n.as_str())
        .map(str::to_string);

    let year = r
        .get("release_date")
        .or_else(|| r.get("album").and_then(|a| a.get("release_date")))
        .and_then(|v| v.as_str())
        .and_then(|s| s.get(..4))
        .and_then(|y| y.parse::<i32>().ok());

    let track_number = r
        .get("track_position")
        .and_then(|v| v.as_i64())
        .map(|n| n as i32);

    DeezerTrack {
        title: r
            .get("title")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string(),
        artist: r
            .pointer("/artist/name")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string(),
        album: r
            .get("album")
            .and_then(|a| a.get("title"))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string(),
        genre,
        year,
        track_number,
        duration: r.get("duration").and_then(|v| v.as_f64()).unwrap_or(0.0),
        cover_url,
    }
}

/// Lowercase alphanumerics/whitespace only, for lenient matching.
fn normalize(s: &str) -> String {
    s.chars()
        .filter(|c| c.is_alphanumeric() || c.is_whitespace())
        .flat_map(char::to_lowercase)
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// True when one normalized string contains the other and the contained
/// (shorter) one is long enough to be meaningful (avoids "Be" matching
/// "Beatles").
fn contains_any(a: &str, b: &str) -> bool {
    let (shorter, longer) = if a.len() <= b.len() { (a, b) } else { (b, a) };
    shorter.len() >= 3 && longer.contains(shorter)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_normalize() {
        assert_eq!(
            normalize("Beautiful (feat. Camila)"),
            "beautiful feat camila"
        );
        assert_eq!(normalize("God's Plan"), "gods plan");
    }

    #[test]
    fn test_contains_any() {
        assert!(contains_any("beautiful feat camila", "beautiful"));
        assert!(contains_any("beautiful", "beautiful feat camila"));
        assert!(!contains_any("be", "beatles"));
    }

    #[test]
    fn test_best_match_accepts_plausible_hit() {
        let results = vec![json!({
            "id": 1,
            "title": "Beautiful (feat. Camila Cabello)",
            "artist": {"name": "Bazzi"},
            "album": {"title": "Cosmic Latte", "cover_big": "http://x/c.jpg"},
            "duration": 208,
            "track_position": 4,
            "release_date": "2018-07-13"
        })];
        let hit = best_match(&results, "Bazzi", "Beautiful feat. Camila", 207.0).unwrap();
        assert_eq!(hit.title, "Beautiful (feat. Camila Cabello)");
        assert_eq!(hit.artist, "Bazzi");
        assert_eq!(hit.year, Some(2018));
        assert_eq!(hit.track_number, Some(4));
        assert_eq!(hit.duration, 208.0);
        assert_eq!(hit.cover_url.as_deref(), Some("http://x/c.jpg"));
    }

    #[test]
    fn test_best_match_rejects_unrelated() {
        let results = vec![json!({
            "id": 2,
            "title": "Something Completely Different",
            "artist": {"name": "Random Band"},
            "album": {"title": "Other"},
            "duration": 240
        })];
        assert!(best_match(&results, "Bazzi", "Beautiful feat. Camila", 207.0).is_none());
    }

    #[test]
    fn test_best_match_empty_results() {
        assert!(best_match(&[], "Bazzi", "Beautiful", 0.0).is_none());
    }

    #[test]
    fn test_build_query_does_not_pre_encode() {
        assert_eq!(
            build_query("Bazzi", "Beautiful feat. Camila"),
            "artist:\"Bazzi\" track:\"Beautiful feat. Camila\""
        );
        assert_eq!(
            build_query("Kygo, Avicii", "Forever Yours ⧸ Remix"),
            "artist:\"Kygo, Avicii\" track:\"Forever Yours ⧸ Remix\""
        );
        assert_eq!(build_query("", "Mama Africa"), "track:\"Mama Africa\"");
        assert_eq!(build_query("  ", ""), "");
    }
}
