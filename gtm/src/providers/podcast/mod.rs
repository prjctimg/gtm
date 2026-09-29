// Copyright (c) 2026
// Author: prjctimg <prjctimg@outlook.com>
// Podcast subscriptions: serializable types shared over IPC
//
// This is free software released under the GPL-3.0 license.

pub mod app;
pub mod picker;

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
    /// Transcripts the feed publishes for this episode, in feed order.
    ///
    /// A feed can carry more than one and more than one kind: a hosted `.vtt`,
    /// an inline `podcast:transcript` CDATA body, a Podscribe JSON blob. The
    /// kind is kept so the fetcher picks a format it can actually parse rather
    /// than downloading an HTML page and reporting "no transcript".
    #[serde(default)]
    pub transcripts: Vec<PodcastTranscript>,
}

/// A transcript an episode's feed entry points at.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PodcastTranscript {
    /// Remote transcript URL, for the `url=` form.
    #[serde(default)]
    pub url: Option<String>,
    /// Inline transcript body, for the CDATA form where the transcript is
    /// embedded in the feed itself.
    #[serde(default)]
    pub text: Option<String>,
    /// The feed's declared `type` — `text/vtt`, `application/json`,
    /// `text/html`, `text/srt`. Advisory: the parser detects the real format,
    /// so a wrong or absent value costs nothing.
    #[serde(default)]
    pub kind: Option<String>,
    /// `rel` — `captions` for the human-readable one. When a feed offers both
    /// `captions` and `transcript`, the former is the one to show.
    #[serde(default)]
    pub rel: Option<String>,
}

impl PodcastTranscript {
    /// Whether this transcript is worth fetching: it has to carry something.
    pub fn is_present(&self) -> bool {
        self.url.as_deref().is_some_and(|u| !u.trim().is_empty())
            || self.text.as_deref().is_some_and(|t| !t.trim().is_empty())
    }

    /// `captions` first, then anything else, then inline bodies — a feed that
    /// labels its transcripts knows which one is for the listener.
    pub fn rank(&self) -> u8 {
        match (self.rel.as_deref(), self.is_present()) {
            (Some("captions"), true) => 0,
            (_, true) => 1,
            _ => 2,
        }
    }
}
