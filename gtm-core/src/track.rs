// Copyright (c) 2026
// Author: prjctimg <prjctimg@outlook.com>
// Track, playlist, lyrics, and search result types
//
// This is free software released under the GPL-3.0 license.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TrackInfo {
    pub id: i64,
    pub path: String,
    pub title: String,
    pub artist: String,
    pub album: String,
    pub duration: f64,
    #[serde(default)]
    pub actual_duration: Option<f64>,
    pub track_number: Option<i32>,
    pub genre: String,
    pub year: Option<i32>,
    pub bitrate: Option<i32>,
    pub samplerate: Option<i32>,
    pub hash: String,
    pub cover_path: Option<String>,
    pub favourite: bool,
    #[serde(default)]
    pub loudness_lufs: Option<f32>,
    #[serde(default)]
    pub loudness_peak_db: Option<f32>,
    #[serde(default)]
    pub loudness_range: Option<f32>,
    #[serde(default)]
    pub artist_image: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Playlist {
    pub id: i64,
    pub name: String,
    pub created_at: String, // ISO 8601
    pub track_count: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LrcLine {
    pub timestamp: f64,
    pub text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LrcData {
    pub title: Option<String>,
    pub artist: Option<String>,
    pub album: Option<String>,
    pub lines: Vec<LrcLine>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct YTSearchResult {
    pub id: String,
    pub title: String,
    pub url: String,
    pub channel: String,
    pub duration: f64,
    pub views: u64,
    pub thumbnail: Option<String>,
    pub is_playlist: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StreamInfo {
    pub url: String,
    pub title: String,
    pub ext: String,
    pub duration: f64,
}
