// Copyright (c) 2026
// Author: prjctimg <prjctimg@outlook.com>
// Podcast subscriptions: feed storage, RSS/Atom parsing, native streaming
//
// This is free software released under the GPL-3.0 license.

use std::collections::HashMap;
use std::path::PathBuf;
use std::time::Duration;

use quick_xml::Reader;
use quick_xml::events::Event;
use tracing::{info, warn};

use gtm_core::podcast::{PodcastEpisode, PodcastFeed, PodcastStatus};

const CONFIG_FILE: &str = "podcast.json";
const CONFIG_PERMS: u32 = 0o600;
const FETCH_TIMEOUT: Duration = Duration::from_secs(30);
const MAX_FEED_BYTES: usize = 8 * 1024 * 1024;

/// A subscribed feed as persisted.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct SubscribedFeed {
    id: String,
    url: String,
    #[serde(default)]
    title: String,
}

/// Owns podcast subscriptions and the parsed episode cache. Feed URLs are
/// stored in a 0600 JSON file under the daemon config directory; episodes are
/// fetched on demand and cached in memory. Playback streams each episode's
/// audio URL over HTTP through the native streaming decoder.
pub struct PodcastManager {
    config_dir: PathBuf,
    client: reqwest::Client,
    feeds: Vec<SubscribedFeed>,
    episodes: HashMap<String, Vec<PodcastEpisode>>,
    error: Option<String>,
}

impl PodcastManager {
    pub fn new(config_dir: PathBuf) -> Self {
        Self {
            config_dir,
            client: reqwest::Client::builder()
                .user_agent(concat!("gtm/", env!("CARGO_PKG_VERSION"), " (podcast-reader)"))
                .timeout(FETCH_TIMEOUT)
                .redirect(reqwest::redirect::Policy::limited(10))
                .build()
                .unwrap_or_default(),
            feeds: Vec::new(),
            episodes: HashMap::new(),
            error: None,
        }
    }

    fn config_path(&self) -> PathBuf {
        self.config_dir.join(CONFIG_FILE)
    }

    /// Load subscriptions at daemon startup.
    pub fn load(&mut self) {
        let Ok(raw) = std::fs::read_to_string(self.config_path()) else {
            return;
        };
        match serde_json::from_str::<Vec<SubscribedFeed>>(&raw) {
            Ok(feeds) => {
                self.feeds = feeds;
                info!("loaded {} podcast feeds", self.feeds.len());
            }
            Err(e) => warn!("ignoring corrupt podcast config: {e}"),
        }
    }

    fn save(&self) -> Result<(), String> {
        use std::os::unix::fs::PermissionsExt;
        let raw = serde_json::to_string_pretty(&self.feeds).map_err(|e| e.to_string())?;
        std::fs::write(self.config_path(), raw).map_err(|e| format!("write: {e}"))?;
        let _ = std::fs::set_permissions(
            self.config_path(),
            std::fs::Permissions::from_mode(CONFIG_PERMS),
        );
        Ok(())
    }

    /// Fetch and subscribe to a feed URL.
    pub async fn add_feed(&mut self, url: &str) -> Result<PodcastFeed, String> {
        let parsed = fetch_and_parse(&self.client, url).await?;
        let id = feed_id(url);
        if let Some(existing) = self.feeds.iter().position(|f| f.id == id) {
            self.feeds[existing].title = parsed.title.clone();
        } else {
            self.feeds.push(SubscribedFeed {
                id: id.clone(),
                url: url.to_string(),
                title: parsed.title.clone(),
            });
        }
        self.episodes.insert(id.clone(), parsed.episodes.clone());
        self.error = None;
        self.save()?;
        Ok(PodcastFeed {
            id,
            title: parsed.title,
            url: url.to_string(),
            description: parsed.description,
            episodes: parsed.episodes.len(),
        })
    }

    pub fn remove_feed(&mut self, id: &str) -> Result<(), String> {
        let pos = self
            .feeds
            .iter()
            .position(|f| f.id == id)
            .ok_or_else(|| "unknown podcast feed".to_string())?;
        self.feeds.remove(pos);
        self.episodes.remove(id);
        self.save()
    }

    /// The subscribed feed list, with episode counts from the last fetch.
    pub fn feeds(&self) -> Vec<PodcastFeed> {
        self.feeds
            .iter()
            .map(|f| PodcastFeed {
                id: f.id.clone(),
                title: f.title.clone(),
                url: f.url.clone(),
                description: String::new(),
                episodes: self.episodes.get(&f.id).map(|e| e.len()).unwrap_or(0),
            })
            .collect()
    }

    /// Episodes of a feed. Returns the cached list; `refresh_feed` re-fetches.
    pub fn episodes(&self, feed_id: &str) -> Result<(String, Vec<PodcastEpisode>), String> {
        let feed = self
            .feeds
            .iter()
            .find(|f| f.id == feed_id)
            .ok_or_else(|| "unknown podcast feed".to_string())?;
        let eps = self.episodes.get(feed_id).cloned().unwrap_or_default();
        Ok((feed.title.clone(), eps))
    }

    /// Cached episode at `index` of a feed, or `None` when the feed is unknown
    /// or has not been fetched yet.
    pub fn episode_at(&self, feed_id: &str, index: usize) -> Option<PodcastEpisode> {
        self.episodes.get(feed_id)?.get(index).cloned()
    }

    /// Re-fetch a single feed.
    pub async fn refresh_feed(&mut self, feed_id: &str) -> Result<PodcastFeed, String> {
        let feed = self
            .feeds
            .iter()
            .find(|f| f.id == feed_id)
            .cloned()
            .ok_or_else(|| "unknown podcast feed".to_string())?;
        let parsed = fetch_and_parse(&self.client, &feed.url).await?;
        self.feeds
            .iter_mut()
            .find(|f| f.id == feed_id)
            .unwrap()
            .title = parsed.title.clone();
        self.episodes.insert(feed_id.to_string(), parsed.episodes.clone());
        self.error = None;
        self.save()?;
        Ok(PodcastFeed {
            id: feed.id,
            title: parsed.title,
            url: feed.url,
            description: parsed.description,
            episodes: parsed.episodes.len(),
        })
    }

    /// Re-fetch every subscribed feed.
    pub async fn refresh_all(&mut self) -> Result<usize, String> {
        let urls: Vec<(String, String)> = self
            .feeds
            .iter()
            .map(|f| (f.id.clone(), f.url.clone()))
            .collect();
        let mut ok = 0usize;
        for (id, url) in urls {
            match fetch_and_parse(&self.client, &url).await {
                Ok(parsed) => {
                    if let Some(feed) = self.feeds.iter_mut().find(|f| f.id == id) {
                        feed.title = parsed.title.clone();
                    }
                    self.episodes.insert(id, parsed.episodes);
                    ok += 1;
                }
                Err(e) => {
                    self.error = Some(format!("refresh {url}: {e}"));
                    warn!("{}", self.error.as_deref().unwrap_or(""));
                }
            }
        }
        self.save()?;
        Ok(ok)
    }

    pub fn status(&self) -> PodcastStatus {
        PodcastStatus {
            feeds: self.feeds.len(),
            episodes: self.episodes.values().map(|e| e.len()).sum(),
            error: self.error.clone(),
        }
    }
}

/// Stable feed id derived from the feed URL.
pub fn feed_id(url: &str) -> String {
    format!("{:x}", md5::compute(url.as_bytes()))
}

/// Strips query strings from an audio URL for stable episode ids.
fn sensible_id(url: &str) -> String {
    url.split(['?', '#']).next().unwrap_or(url).to_string()
}

struct ParsedFeed {
    title: String,
    description: String,
    episodes: Vec<PodcastEpisode>,
}

async fn fetch_and_parse(client: &reqwest::Client, url: &str) -> Result<ParsedFeed, String> {
    let resp = client
        .get(url)
        .send()
        .await
        .map_err(|e| format!("fetch: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("fetch: HTTP {}", resp.status()));
    }
    let mut raw = String::with_capacity(64 * 1024);
    let mut chunks = resp.bytes_stream();
    use futures::StreamExt;
    while let Some(chunk) = chunks.next().await {
        let chunk = chunk.map_err(|e| format!("stream: {e}"))?;
        raw.push_str(&String::from_utf8_lossy(&chunk));
        if raw.len() > MAX_FEED_BYTES {
            return Err("feed too large".into());
        }
    }
    parse_feed(&raw, url)
}

/// Parse an RSS 2.0 or Atom podcast feed.
fn parse_feed(raw: &str, feed_url: &str) -> Result<ParsedFeed, String> {
    let mut reader = Reader::from_str(raw);
    reader.config_mut().trim_text(true);
    let mut buf = Vec::new();

    let mut title = String::new();
    let mut description = String::new();
    let mut is_atom = false;
    let mut in_channel = false;

    // Active episode being accumulated.
    let mut ep: Option<ParsedEpisode> = None;
    let mut stack: Vec<String> = Vec::new();
    let mut episodes: Vec<ParsedEpisode> = Vec::new();

    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) => {
                let name = local_name(&e);
                if name == "rss" || name == "feed" {
                    is_atom = name == "feed";
                }
                if !is_atom && name == "channel" {
                    in_channel = true;
                }
                if is_atom && name == "entry" {
                    ep = Some(ParsedEpisode::default());
                }
                if !is_atom && name == "item" {
                    ep = Some(ParsedEpisode::default());
                }
                if ep.is_some()
                    && name == "enclosure"
                {
                    if let Some(url) = attr_str(&e, "url") {
                        if let Some(p) = ep.as_mut() {
                            if p.url.is_empty() {
                                p.url = url;
                            }
                        }
                    }
                }
                if ep.is_some() && name == "link" && is_atom {
                    // Atom enclosure/alternate links carry the URL in href.
                    if let Some(href) = attr_str(&e, "href") {
                        let rel = attr_str(&e, "rel").unwrap_or_default();
                        if rel == "enclosure" || rel == "audio" {
                            if let Some(p) = ep.as_mut()
                                && p.url.is_empty()
                            {
                                p.url = href;
                            }
                        } else if let Some(p) = ep.as_mut()
                            && p.url.is_empty()
                            // No explicit enclosure rel: fall back to a direct
                            // media link only when it points at audio media.
                            && looks_audio(&attr_str(&e, "type").unwrap_or_default())
                        {
                            p.url = href;
                        }
                    }
                }
                stack.push(name);
            }
            Ok(Event::Empty(e)) => {
                let name = local_name(&e);
                if !is_atom && name == "channel" {
                    in_channel = true;
                }
                if name == "enclosure"
                    && let Some(p) = ep.as_mut()
                    && let Some(url) = attr_str(&e, "url")
                    && p.url.is_empty()
                {
                    p.url = url;
                }
                if name == "link" && is_atom
                    && let Some(p) = ep.as_mut()
                    && let Some(href) = attr_str(&e, "href")
                    && p.url.is_empty()
                {
                    let rel = attr_str(&e, "rel").unwrap_or_default();
                    if rel == "enclosure" || rel == "audio" || looks_audio(&attr_str(&e, "type").unwrap_or_default()) {
                        p.url = href;
                    }
                }
                if name == "content" && is_atom
                    && let Some(p) = ep.as_mut()
                    && p.url.is_empty()
                    && let Some(url) = attr_str(&e, "url")
                    && looks_audio(&attr_str(&e, "type").unwrap_or_default())
                {
                    p.url = url;
                }
                stack.push(name.clone());
                // Treat self-closing leaf as immediately closed.
                if let Some(p) = ep.as_mut()
                    && name == "duration"
                {
                    if let Some(d) = attr_str(&e, "seconds") {
                        if p.duration_secs.is_none() {
                            p.duration_secs = d.parse::<u64>().ok();
                        }
                    }
                }
                stack.pop();
            }
            Ok(Event::Text(t)) => {
                // Skip whitespace-only text without an active context.
                if ep.is_none() && stack.is_empty() {
                    continue;
                }
                if let Some(p) = ep.as_mut()
                    && let Some(field) = stack.last()
                {
                    let text = t.decode().unwrap_or_default().trim().to_string();
                    if text.is_empty() {
                        continue;
                    }
                    apply_field(p, field, &text);
                } else if let Some(field) = stack.last()
                    && title.is_empty()
                    && field == "title"
                    && (in_channel || is_atom)
                {
                    title = t.decode().unwrap_or_default().trim().to_string();
                } else if let Some(field) = stack.last()
                    && field == "description"
                    && (in_channel || is_atom)
                    && (description.is_empty() || !is_atom)
                {
                    description = t.decode().unwrap_or_default().trim().to_string();
                }
            }
            Ok(Event::CData(c)) => {
                if let Some(p) = ep.as_mut()
                    && let Some(field) = stack.last()
                {
                    let text = c.decode().unwrap_or_default().trim().to_string();
                    if !text.is_empty() {
                        apply_field(p, field, &text);
                    }
                }
            }
            Ok(Event::End(e)) => {
                let name = end_name(&e);
                if name == "channel" {
                    in_channel = false;
                }
                if let Some(mut p) = ep.take()
                    && (name == "item" || name == "entry")
                {
                    // Skip entries with no playable enclosure.
                    if p.url.is_empty() {
                        stack.pop();
                        continue;
                    }
                    if p.title.trim().is_empty() {
                        p.title = title.clone();
                    }
                    if p.id.trim().is_empty() {
                        p.id = sensible_id(&p.url);
                    }
                    let id = p.id.clone();
                    if episodes.iter().any(|e| e.id == id) {
                        p.id = format!("{id}-{}", episodes.len());
                    }
                    episodes.push(p);
                }
                stack.pop();
            }
            Ok(Event::Eof) => break,
            Err(e) => return Err(format!("parse error: {e}")),
            _ => {}
        }
        buf.clear();
    }

    let title = if title.trim().is_empty() {
        "Untitled Podcast".to_string()
    } else {
        title
    };
    let feed_id = feed_id(feed_url);
    let episodes: Vec<PodcastEpisode> = episodes
        .into_iter()
        .map(|e| PodcastEpisode {
            feed_id: feed_id.clone(),
            feed_title: title.clone(),
            id: e.id,
            title: e.title,
            url: e.url,
            duration_secs: e.duration_secs,
            published: e.published,
            description: if e.description.is_empty() {
                None
            } else {
                Some(e.description)
            },
        })
        .collect();
    Ok(ParsedFeed {
        title,
        description,
        episodes,
    })
}

#[derive(Default)]
struct ParsedEpisode {
    id: String,
    title: String,
    url: String,
    duration_secs: Option<u64>,
    published: Option<String>,
    description: String,
}

fn apply_field(ep: &mut ParsedEpisode, field: &str, text: &str) {
    match field {
        "title" => {
            if ep.title.is_empty() {
                ep.title = text.to_string();
            }
        }
        "guid" | "id" => {
            if ep.id.is_empty() {
                ep.id = text.to_string();
            }
        }
        "duration" => {
            if ep.duration_secs.is_none() {
                ep.duration_secs = parse_duration(text);
            }
        }
        "pubdate" | "published" | "updated" => {
            if ep.published.is_none() {
                ep.published = Some(normalize_date(text));
            }
        }
        "description" | "summary" | "subtitle" => {
            if ep.description.is_empty() {
                ep.description = text.to_string();
            }
        }
        _ => {}
    }
}

/// Parse `HH:MM:SS`, `MM:SS`, or plain seconds durations.
fn parse_duration(s: &str) -> Option<u64> {
    let s = s.trim();
    if let Ok(secs) = s.parse::<u64>() {
        return Some(secs);
    }
    let parts: Vec<&str> = s.split(':').collect();
    match parts.len() {
        3 => {
            let h: u64 = parts[0].parse().ok()?;
            let m: u64 = parts[1].parse().ok()?;
            let sec: u64 = parts[2].parse().ok()?;
            Some(h * 3600 + m * 60 + sec)
        }
        2 => {
            let m: u64 = parts[0].parse().ok()?;
            let sec: u64 = parts[1].parse().ok()?;
            Some(m * 60 + sec)
        }
        _ => None,
    }
}

/// Best-effort RFC 3339 published timestamp. Feeds commonly use RFC 2822.
fn normalize_date(s: &str) -> String {
    // Try RFC 3339 already.
    if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(s) {
        return dt.to_rfc3339();
    }
    // Try RFC 2822 (pubDate).
    if let Ok(dt) = chrono::DateTime::parse_from_rfc2822(s) {
        return dt.to_rfc3339();
    }
    s.to_string()
}

fn local_name(e: &quick_xml::events::BytesStart<'_>) -> String {
    String::from_utf8_lossy(e.local_name().as_ref()).into_owned()
}

fn end_name(e: &quick_xml::events::BytesEnd<'_>) -> String {
    String::from_utf8_lossy(e.name().as_ref()).into_owned()
}

fn attr_str(e: &quick_xml::events::BytesStart<'_>, key: &str) -> Option<String> {
    for attr in e.attributes().flatten() {
        if attr.key.local_name().as_ref() == key.as_bytes() {
            return attr.unescape_value().ok().map(|c| c.into_owned());
        }
    }
    None
}

fn looks_audio(mime: &str) -> bool {
    let m = mime.to_ascii_lowercase();
    m.starts_with("audio/") || m.contains("mp3") || m.contains("mpeg") || m.contains("ogg") || m.is_empty()
}