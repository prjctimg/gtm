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

    /// Whether this entry and `other` name the same track. Sources spell the
    /// same track differently from the ICY `StreamTitle` (remix suffixes, feat
    /// clauses, leading dashes), so comparison is on the longest shared
    /// normalised token run rather than whole-string equality.
    pub fn same_as(&self, title: &str) -> bool {
        let norm = |s: &str| {
            s.chars()
                .filter(|c| c.is_alphanumeric())
                .flat_map(char::to_lowercase)
                .collect::<String>()
        };
        let (a, b) = (norm(&self.query()), norm(title));
        !a.is_empty() && !b.is_empty() && (a.contains(&b) || b.contains(&a))
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
