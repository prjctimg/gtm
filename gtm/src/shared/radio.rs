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
