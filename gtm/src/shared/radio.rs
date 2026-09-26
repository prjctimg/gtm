// Copyright (c) 2026
// Author: prjctimg <prjctimg@outlook.com>
// Radio Browser directory: serializable types shared over IPC
//
// This is free software released under the GPL-3.0 license.

use serde::{Deserialize, Serialize};

/// A radio station from the Radio Browser directory.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RadioStation {
    /// Radio Browser station uuid.
    pub id: String,
    pub name: String,
    /// Homepage URL of the station, if published.
    #[serde(default)]
    pub homepage: String,
    /// Direct stream URL as published.
    #[serde(default)]
    pub url: String,
    /// Resolved (redirect-followed) stream URL, preferred for playback.
    #[serde(default)]
    pub url_resolved: String,
    #[serde(default)]
    pub country: String,
    #[serde(default)]
    pub language: String,
    #[serde(default)]
    pub tags: String,
    #[serde(default)]
    pub codec: String,
    #[serde(default)]
    pub bitrate_kbps: Option<u64>,
    #[serde(default)]
    pub votes: u64,
    /// Favicon URL, when known.
    #[serde(default)]
    pub favicon: String,
}

impl RadioStation {
    /// The URL best suited for playback: the redirect-resolved stream when the
    /// directory resolved one, falling back to the published stream URL.
    pub fn playable_url(&self) -> &str {
        if !self.url_resolved.is_empty() {
            &self.url_resolved
        } else {
            &self.url
        }
    }
}

/// One entry of a station's published tracklist.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct RadioTrack {
    pub title: String,
    pub artist: String,
    /// Unix seconds the track started, when the source timestamps it. Sources
    /// that publish only a bare wall clock are normalised to absolute seconds
    /// against the fetch time, so this field means one thing everywhere.
    #[serde(default)]
    pub start: Option<i64>,
    /// Cover art URL advertised by the source, when it carries one.
    #[serde(default)]
    pub art: Option<String>,
}

impl RadioTrack {
    /// The `"{artist} - {title}"` form every metadata provider accepts as a
    /// search query. Stations that publish no separate artist field still get
    /// the title alone, so the query stays well-formed.
    pub fn query(&self) -> String {
        if self.artist.is_empty() {
            self.title.clone()
        } else {
            format!("{} - {}", self.artist, self.title)
        }
    }

    /// Whether this entry and `other` name the same track. A station and its
    /// own stream spell the same track differently — a `feat.` clause appears
    /// in one and not the other, remix suffixes get trimmed, a title arrives
    /// truncated — so neither is a substring of the other. Compare the
    /// alphanumeric words instead: the shorter side's words must all appear in
    /// the longer side, which survives insertions and trims while still
    /// rejecting a genuinely different track.
    pub fn same_as(&self, title: &str) -> bool {
        let words = |s: &str| -> Vec<String> {
            let mut out: Vec<String> = Vec::new();
            let mut cur = String::new();
            for c in s.chars() {
                if c.is_alphanumeric() {
                    cur.extend(c.to_lowercase());
                } else if !cur.is_empty() {
                    out.push(std::mem::take(&mut cur));
                }
            }
            if !cur.is_empty() {
                out.push(cur);
            }
            out
        };
        let (a, b) = (words(&self.query()), words(title));
        let (long, short) = if a.len() >= b.len() {
            (&a, &b)
        } else {
            (&b, &a)
        };
        // A one-word title is too weak to match on; it would pair any track
        // called "Fading" with any other.
        short.len() >= 2 && short.iter().all(|w| long.contains(w))
    }
}

/// A station's tracklist, newest first, led by the entry playing now. The
/// read-only queue renders straight from this.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct RadioTracklist {
    pub tracks: Vec<RadioTrack>,
    /// Index into `tracks` of the entry playing now. Sources that publish a
    /// dedicated current-track field can override the leading position.
    #[serde(default)]
    pub at: usize,
    /// Unix seconds the list was fetched, the anchor normalising relative
    /// stamps.
    #[serde(default)]
    pub at_time: i64,
}

impl RadioTracklist {
    /// The entry playing now, if the list is non-empty.
    pub fn now(&self) -> Option<&RadioTrack> {
        self.tracks.get(self.at)
    }

    /// Index of the entry playing now for a title observed off the stream's
    /// ICY metadata, which is authoritative and needs no source lookup. Falls
    /// back to the leading entry when the source and the stream disagree on
    /// spelling.
    pub fn match_title(&self, title: &str) -> usize {
        self.tracks
            .iter()
            .position(|t| t.same_as(title))
            .unwrap_or(self.at)
    }
}

/// A Radio Browser directory tag (`/json/tags`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RadioTag {
    pub name: String,
    #[serde(default, rename = "stationcount")]
    pub station_count: u64,
}

/// A Radio Browser directory country (`/json/countries`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RadioCountry {
    pub name: String,
    #[serde(default, rename = "stationcount")]
    pub station_count: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn track(artist: &str, title: &str) -> RadioTrack {
        RadioTrack {
            title: title.into(),
            artist: artist.into(),
            ..Default::default()
        }
    }

    /// A station's tracklist and its own ICY stream routinely spell the same
    /// track differently, and the differences are insertions rather than
    /// edits: a `feat.` clause in one and not the other, a trimmed remix
    /// suffix, a truncated title. None of those is a substring of the other.
    #[test]
    fn matching_survives_feat_clauses_and_trimming() {
        let t = track(
            "Mark Sherry feat. Sharone",
            "Silent Tears (Orjan Nilsen Remix)",
        );
        assert!(t.same_as("Mark Sherry feat. Sharone - Silent Tears (Orjan Nilsen Remix)"));
        assert!(t.same_as("Mark Sherry - Silent Tears"));
        assert!(t.same_as("Mark Sherry feat. Sharone - Silent Tears"));
        assert!(t.same_as("Silent Tears (Orjan Nilsen Remix)"));
    }

    /// A different track must not match, or the now-playing title would pair
    /// with the wrong tracklist row.
    #[test]
    fn a_different_track_does_not_match() {
        let t = track(
            "Mark Sherry feat. Sharone",
            "Silent Tears (Orjan Nilsen Remix)",
        );
        assert!(!t.same_as("Completely Different Song"));
        assert!(!t.same_as("Mark Sherry - Hello"));
        assert!(!t.same_as(""));
    }

    /// A single shared word is not enough evidence. Every tracklist has a row
    /// called something short, and pairing on one word would be arbitrary.
    #[test]
    fn a_single_shared_word_is_too_weak() {
        let t = track("Some Artist", "Fading Lights");
        assert!(!t.same_as("Lights"));
        assert!(!t.same_as("Fading"));
    }

    #[test]
    fn query_is_well_formed_without_an_artist() {
        assert_eq!(track("", "Alone").query(), "Alone");
        assert_eq!(track("X", "Alone").query(), "X - Alone");
    }
}
