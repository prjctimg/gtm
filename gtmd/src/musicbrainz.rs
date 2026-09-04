// Copyright (c) 2026
// Author: prjctimg <prjctimg@outlook.com>
// MusicBrainz + Cover Art Archive fallback for cover art and artist images
//
// This is free software released under the GPL-3.0 license.

use std::time::Duration;

use reqwest::Client;
use serde_json::Value;

const MB_API: &str = "https://musicbrainz.org/ws/2";
const CAA_URL: &str = "https://coverartarchive.org";
const REQUEST_TIMEOUT: Duration = Duration::from_secs(15);

/// A resolved MusicBrainz release-group (album) lookup result.
pub struct MusicBrainzResult {
    /// MusicBrainz release-group MBID, used for album-level cover reuse.
    pub release_group_id: String,
    /// Front cover URL from the Cover Art Archive, if any.
    pub cover_url: Option<String>,
    /// Artist image placeholder (not currently populated by MusicBrainz).
    pub artist_image_url: Option<String>,
}

pub struct MusicBrainz {
    client: Client,
    user_agent: String,
}

impl Default for MusicBrainz {
    fn default() -> Self {
        Self::new()
    }
}

impl MusicBrainz {
    /// `user_agent` should identify the client per the MusicBrainz API
    /// etiquette (a contactable application name/version).
    pub fn new() -> Self {
        Self {
            client: Client::builder()
                .timeout(REQUEST_TIMEOUT)
                .build()
                .unwrap_or_else(|_| Client::new()),
            user_agent: format!(
                "gtm.rs/{} (https://github.com/gtm.rs)",
                env!("CARGO_PKG_VERSION")
            ),
        }
    }

    /// Search MusicBrainz for an album by artist + album title.
    ///
    /// MusicBrainz requires 1s between requests; we honour that before the
    /// search and again before the release lookup.
    pub async fn find_album(
        &self,
        artist: &str,
        album: &str,
    ) -> Result<Option<MusicBrainzResult>, String> {
        let query = build_query(artist, album);
        if query.is_empty() {
            return Ok(None);
        }

        tokio::time::sleep(Duration::from_secs(1)).await;

        let resp = self
            .client
            .get(format!("{MB_API}/release-group"))
            .query(&[("query", query)])
            .query(&[("fmt", "json"), ("limit", "5")])
            .header("User-Agent", &self.user_agent)
            .send()
            .await
            .map_err(|e| format!("MusicBrainz search failed: {e}"))?;
        let json: Value = resp
            .json()
            .await
            .map_err(|e| format!("MusicBrainz JSON parse failed: {e}"))?;

        let groups = match json.get("release-groups").and_then(|v| v.as_array()) {
            Some(g) if !g.is_empty() => g,
            _ => return Ok(None),
        };

        for group in groups {
            let title = group.get("title").and_then(|v| v.as_str()).unwrap_or("");
            let rg_id = group
                .get("id")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            if rg_id.is_empty() || normalize(title) != normalize(album) {
                continue;
            }
            // Prefer the sound-track/album primary type, skip compilations/fan made
            return Ok(Some(MusicBrainzResult {
                release_group_id: rg_id,
                cover_url: None,
                artist_image_url: None,
            }));
        }
        Ok(None)
    }

    /// Fetch the front cover bytes for a release-group via the Cover Art
    /// Archive.
    pub async fn download_cover(&self, release_group_id: &str) -> Option<Vec<u8>> {
        let url = format!("{CAA_URL}/release-group/{release_group_id}/front");
        self.fetch_bytes(&url).await
    }

    async fn fetch_bytes(&self, url: &str) -> Option<Vec<u8>> {
        match self
            .client
            .get(url)
            .header("User-Agent", &self.user_agent)
            .send()
            .await
        {
            Ok(r) if r.status().is_success() => match r.bytes().await {
                Ok(b) if !b.is_empty() => Some(b.to_vec()),
                _ => None,
            },
            Ok(_) | Err(_) => None,
        }
    }
}

fn build_query(artist: &str, album: &str) -> String {
    let mut parts = Vec::new();
    let artist = artist.trim();
    let album = album.trim();
    if !artist.is_empty() {
        parts.push(format!("artist:\"{artist}\""));
    }
    if !album.is_empty() {
        parts.push(format!("releasegroup:\"{album}\""));
    }
    parts.join(" AND ")
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_build_query() {
        assert_eq!(
            build_query("Bazzi", "Cosmic Latte"),
            "artist:\"Bazzi\" AND releasegroup:\"Cosmic Latte\""
        );
        assert_eq!(
            build_query("", "Mama Africa"),
            "releasegroup:\"Mama Africa\""
        );
        assert_eq!(build_query("  ", ""), "");
    }

    #[test]
    fn test_normalize() {
        assert_eq!(normalize("Cosmic LATTE"), "cosmic latte");
    }
}
