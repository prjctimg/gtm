// Copyright (c) 2026
// Author: prjctimg <prjctimg@outlook.com>
// Podcast subscriptions: serializable types shared over IPC
//
// This is free software released under the GPL-3.0 license.

use serde::{Deserialize, Serialize};

/// Aggregate state of the podcast integration.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct PodcastStatus {
    /// Number of subscribed feeds.
    pub feeds: usize,
    /// Total number of episodes across all feeds.
    pub episodes: usize,
    /// Most recent error message, if a feed fetch failed.
    pub error: Option<String>,
}

/// A subscribed podcast feed.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PodcastFeed {
    /// Stable feed id (URL-derived), used to address the feed in commands.
    pub id: String,
    pub title: String,
    /// Canonical feed URL.
    pub url: String,
    #[serde(default)]
    pub description: String,
    /// Number of episodes in the last successful fetch.
    #[serde(default)]
    pub episodes: usize,
}

/// A single podcast episode.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PodcastEpisode {
    pub feed_id: String,
    pub feed_title: String,
    /// Stable episode id (URL-derived).
    pub id: String,
    pub title: String,
    /// Direct audio URL to stream.
    pub url: String,
    #[serde(default)]
    pub duration_secs: Option<u64>,
    /// RFC 3339 publication timestamp, when the feed provides one.
    #[serde(default)]
    pub published: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
}