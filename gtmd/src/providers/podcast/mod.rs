// Copyright (c) 2026
// Author: prjctimg <prjctimg@outlook.com>
// Podcast subscriptions: feed storage, RSS/Atom parsing, native streaming
//
// This is free software released under the GPL-3.0 license.

use std::collections::HashMap;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::time::Duration;

use futures::StreamExt;
use quick_xml::Reader;
use quick_xml::events::Event;
use tracing::{info, warn};

use gtm::shared::podcast::{
    PodcastEpisode, PodcastFeed, PodcastResult, PodcastStatus, PodcastTranscript,
};
use gtm::shared::track::LrcData;

pub mod vtt;

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
    /// Artwork learned when the feed was subscribed, so the subscription list
    /// shows a picture without re-parsing the feed on every open.
    #[serde(default)]
    image_url: Option<String>,
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
    /// Directory results, most recent first, kept only to supply artwork to
    /// `add_feed`: the feeds themselves carry none often enough that
    /// subscribing straight from a search result would lose the picture.
    discovery: Vec<PodcastResult>,
    /// Last search per query, so re-entering the picker is free.
    discovery_cache: HashMap<String, Vec<PodcastResult>>,
}

impl PodcastManager {
    pub fn new(config_dir: PathBuf) -> Self {
        Self {
            config_dir,
            client: reqwest::Client::builder()
                .user_agent(concat!(
                    "gtm/",
                    env!("CARGO_PKG_VERSION"),
                    " (podcast-reader)"
                ))
                .timeout(FETCH_TIMEOUT)
                .redirect(reqwest::redirect::Policy::limited(10))
                .build()
                .unwrap_or_default(),
            feeds: Vec::new(),
            episodes: HashMap::new(),
            error: None,
            discovery: Vec::new(),
            discovery_cache: HashMap::new(),
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
        let raw = serde_json::to_string_pretty(&self.feeds).map_err(|e| e.to_string())?;
        std::fs::write(self.config_path(), raw).map_err(|e| format!("write: {e}"))?;
        let _ = std::fs::set_permissions(
            self.config_path(),
            std::fs::Permissions::from_mode(CONFIG_PERMS),
        );
        Ok(())
    }

    /// Fetch and subscribe to a feed URL.
    ///
    /// Artwork already known for this feed — from a directory listing, usually —
    /// is kept: the feed itself often has none, and subscribing should not throw
    /// away the only picture the user will ever see for it.
    pub async fn add_feed(&mut self, url: &str) -> Result<PodcastFeed, String> {
        let parsed = fetch_and_parse(&self.client, url).await?;
        let id = feed_id(url);
        let art = parsed.image.clone().or_else(|| self.art_for(url));
        if let Some(existing) = self.feeds.iter().position(|f| f.id == id) {
            self.feeds[existing].title = parsed.title.clone();
            self.feeds[existing].image_url = art.clone();
        } else {
            self.feeds.push(SubscribedFeed {
                id: id.clone(),
                url: url.to_string(),
                title: parsed.title.clone(),
                image_url: art.clone(),
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
            image_url: art,
        })
    }

    /// Artwork already known for a feed url, from the discovery cache.
    fn art_for(&self, url: &str) -> Option<String> {
        self.discovery
            .iter()
            .find(|r| r.url == url)
            .and_then(|r| r.image_url.clone())
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
                image_url: f.image_url.clone(),
            })
            .collect()
    }

    /// Search the public podcast directory.
    ///
    /// Backed by the iTunes Search API, which is the one podcast directory that
    /// needs no account and no API key: it answers with each show's real
    /// `feedUrl`, artwork and episode count. Results are cached by query so
    /// re-entering the picker with the same term costs nothing, and cached
    /// results are what `add_feed` falls back to for artwork.
    pub async fn discover(
        &mut self,
        term: &str,
        country: &str,
    ) -> Result<Vec<PodcastResult>, String> {
        let term = term.trim();
        if term.is_empty() {
            return Err("type something to search for".into());
        }
        let key = format!(
            "{}|{}",
            country.to_ascii_lowercase(),
            term.to_ascii_lowercase()
        );
        if let Some(hit) = self.discovery_cache.get(&key) {
            return Ok(hit.clone());
        }
        let url = format!(
            "https://itunes.apple.com/search?term={}&entity=podcast&country={}&limit=50",
            urlencoding::encode(term),
            urlencoding::encode(country)
        );
        let body: ItunesSearch = reqwest::Client::new()
            .get(&url)
            .timeout(FETCH_TIMEOUT)
            .send()
            .await
            .map_err(|e| format!("podcast search: {e}"))?
            .error_for_status()
            .map_err(|e| format!("podcast search: {e}"))?
            .json()
            .await
            .map_err(|e| format!("podcast search: {e}"))?;
        let results: Vec<PodcastResult> = body
            .results
            .into_iter()
            .filter_map(|r| {
                // A show with no feed url cannot be subscribed to or streamed, so it
                // is not a result — the directory indexes plenty of those.
                let url = r.feed_url.filter(|u| !u.trim().is_empty())?;
                Some(PodcastResult {
                    title: r.collection_name,
                    author: r.artist_name,
                    url,
                    description: String::new(),
                    image_url: r
                        .artwork_url600
                        .or(r.artwork_url100)
                        .map(|u| upgrade_art(&u)),
                    episodes: r.track_count.unwrap_or_default() as usize,
                    country: country.to_string(),
                })
            })
            .collect();
        if results.is_empty() {
            return Err(format!("no podcasts matched {term:?}"));
        }
        // Remember every result's art, not just the ones on screen, so
        // subscribing to any of them finds a picture.
        for r in &results {
            self.discovery.retain(|d| d.url != r.url);
            self.discovery.push(r.clone());
        }
        // Bounded like the rest of the caches; oldest out first.
        while self.discovery.len() > 200 {
            self.discovery.remove(0);
        }
        self.discovery_cache.insert(key, results.clone());
        Ok(results)
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
        // Keep the stored picture if the feed's own is still absent: feeds
        // commonly publish artwork only through the directory that indexed them.
        let art = parsed.image.clone().or(feed.image_url.clone());
        let slot = self.feeds.iter_mut().find(|f| f.id == feed_id).unwrap();
        slot.title = parsed.title.clone();
        slot.image_url = art.clone();
        self.episodes
            .insert(feed_id.to_string(), parsed.episodes.clone());
        self.error = None;
        self.save()?;
        Ok(PodcastFeed {
            id: feed.id,
            title: parsed.title,
            url: feed.url,
            description: parsed.description,
            episodes: parsed.episodes.len(),
            image_url: art,
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

    /// Fetch and parse an episode's transcript.
    ///
    /// Every transcript the feed offers is tried in preference order, because a
    /// feed that offers several is not obliged to keep them all reachable: the
    /// first one that yields a non-empty parse wins, and a failure to reach the
    /// first is a fallthrough rather than the answer. A feed with none is an
    /// error the caller can say out loud, not an empty pane.
    pub async fn fetch_transcript(
        &self,
        feed_id: &str,
        episode_index: usize,
    ) -> Result<LrcData, String> {
        let episode = self
            .episode_at(feed_id, episode_index)
            .ok_or_else(|| "episode missing".to_string())?;
        if episode.transcripts.is_empty() {
            return Err(format!(
                "\u{201c}{}\u{201d} has no transcript",
                if episode.title.trim().is_empty() {
                    "this episode"
                } else {
                    &episode.title
                }
            ));
        }
        let mut ranked: Vec<&PodcastTranscript> = episode.transcripts.iter().collect();
        ranked.sort_by_key(|t| t.rank());
        let mut last = String::from("no usable transcript");
        for t in ranked {
            let body = match (&t.text, &t.url) {
                // Inline first: no network, and it cannot be a stale copy.
                (Some(text), _) if !text.trim().is_empty() => text.clone(),
                (_, Some(url)) if !url.trim().is_empty() => match self.fetch_body(url).await {
                    Ok(b) => b,
                    Err(e) => {
                        last = e;
                        continue;
                    }
                },
                _ => continue,
            };
            let parsed = vtt::parse_transcript(&body);
            if !parsed.lines.is_empty() {
                return Ok(LrcData {
                    title: episode.title.clone().into(),
                    artist: Some(episode.feed_title.clone()),
                    album: None,
                    lines: parsed.lines,
                });
            }
            last = "transcript was empty".into();
        }
        Err(last)
    }

    /// GET a transcript body, size-capped like a feed.
    async fn fetch_body(&self, url: &str) -> Result<String, String> {
        let resp = self
            .client
            .get(url)
            .send()
            .await
            .map_err(|e| format!("transcript fetch: {e}"))?;
        if !resp.status().is_success() {
            return Err(format!("transcript fetch: HTTP {}", resp.status()));
        }
        let mut raw = String::with_capacity(32 * 1024);
        let mut chunks = resp.bytes_stream();
        while let Some(chunk) = chunks.next().await {
            let chunk = chunk.map_err(|e| format!("transcript stream: {e}"))?;
            raw.push_str(&String::from_utf8_lossy(&chunk));
            if raw.len() > MAX_FEED_BYTES {
                return Err("transcript too large".into());
            }
        }
        Ok(raw)
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
    /// Channel artwork, from `<itunes:image href>` or Atom `<logo>`/`<icon>`.
    /// Not read before: podcasts had no artwork anywhere, in the feed list or
    /// the episode card.
    image: Option<String>,
    episodes: Vec<PodcastEpisode>,
}

/// The iTunes Search API's podcast result, under its wire names.
#[derive(serde::Deserialize)]
struct ItunesSearch {
    #[serde(default, rename = "results")]
    results: Vec<ItunesPodcast>,
}

#[derive(serde::Deserialize)]
struct ItunesPodcast {
    #[serde(rename = "collectionName")]
    collection_name: String,
    #[serde(default, rename = "artistName")]
    artist_name: String,
    #[serde(default, rename = "feedUrl")]
    feed_url: Option<String>,
    #[serde(rename = "artworkUrl600")]
    artwork_url600: Option<String>,
    #[serde(rename = "artworkUrl100")]
    artwork_url100: Option<String>,
    #[serde(rename = "trackCount")]
    track_count: Option<u64>,
}

/// The directory publishes a 100px thumbnail; the player wants more than that,
/// and the 600px form is the same image at a different size in the same path.
fn upgrade_art(url: &str) -> String {
    url.replace("/100x100bb.jpg", "/600x600bb.jpg")
        .replace("/100x100bb.png", "/600x600bb.png")
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
    let mut image: Option<String> = None;
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
                if name == "enclosure"
                    && let Some(url) = attr_str(&e, "url")
                    && let Some(p) = ep.as_mut()
                    && p.url.is_empty()
                {
                    p.url = url;
                }
                // Channel artwork. `<itunes:image href>` is an empty element
                // carrying only its attribute, so it has to be read here rather
                // than as text; Atom's `<logo>` and `<icon>` do the same job and
                // also carry the href in an attribute. A per-episode image wins
                // over the channel's, since it is the more specific answer.
                if (name == "image" || name == "logo" || name == "icon")
                    && let Some(href) = attr_str(&e, "href").or_else(|| attr_str(&e, "url"))
                    && !href.trim().is_empty()
                {
                    match ep.as_mut() {
                        Some(p) => p.image.get_or_insert(href),
                        None => image.get_or_insert(href),
                    };
                }
                // `<media:content url=... type="audio/mpeg">` is how a growing
                // number of feeds carry the audio, in place of an RSS
                // `<enclosure>`. It was not read at all, so those episodes came
                // out with no URL and were then thrown away.
                if name == "content"
                    && let Some(url) = attr_str(&e, "url")
                    && let Some(p) = ep.as_mut()
                    && p.url.is_empty()
                    && looks_audio(&attr_str(&e, "type").unwrap_or_default())
                {
                    p.url = url;
                }
                // `<podcast:transcript>` (Atom) and `<transcript>` (RSS) both
                // open either self-closing with a `url` or with a CDATA body
                // holding the transcript itself. Only the self-closing form was
                // handled, and only for `content`.
                if name == "transcript"
                    && let Some(p) = ep.as_mut()
                {
                    p.transcripts.push(PodcastTranscript {
                        url: attr_str(&e, "url").filter(|u| !u.trim().is_empty()),
                        text: None,
                        kind: attr_str(&e, "type"),
                        rel: attr_str(&e, "rel"),
                    });
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
                if name == "link"
                    && is_atom
                    && let Some(p) = ep.as_mut()
                    && let Some(href) = attr_str(&e, "href")
                    && p.url.is_empty()
                {
                    let rel = attr_str(&e, "rel").unwrap_or_default();
                    if rel == "enclosure"
                        || rel == "audio"
                        || looks_audio(&attr_str(&e, "type").unwrap_or_default())
                    {
                        p.url = href;
                    }
                }
                // Not gated on `is_atom`, unlike the start-element arm above it.
                // `<media:content url=... type="audio/mpeg"/>` is how a growing
                // number of RSS feeds carry the audio, and self-closing is how
                // they write it: the Atom-only gate meant those feeds parsed to
                // episodes with no audio, which is no better than none at all.
                if name == "content"
                    && let Some(p) = ep.as_mut()
                    && p.url.is_empty()
                    && let Some(url) = attr_str(&e, "url")
                    && looks_audio(&attr_str(&e, "type").unwrap_or_default())
                {
                    p.url = url;
                }
                // The CDATA form: the element is open, and its text is the
                // transcript. The `url` attribute is recorded first so a feed
                // that carries both keeps the hosted copy.
                if name == "transcript"
                    && let Some(p) = ep.as_mut()
                {
                    p.transcripts.push(PodcastTranscript {
                        url: attr_str(&e, "url").filter(|u| !u.trim().is_empty()),
                        text: None,
                        kind: attr_str(&e, "type"),
                        rel: attr_str(&e, "rel"),
                    });
                }
                stack.push(name.clone());
                // Treat self-closing leaf as immediately closed.
                if let Some(p) = ep.as_mut()
                    && name == "duration"
                    && let Some(d) = attr_str(&e, "seconds")
                    && p.duration_secs.is_none()
                {
                    p.duration_secs = d.parse::<u64>().ok();
                }
                stack.pop();
            }
            Ok(Event::Text(t)) => {
                // A `transcript` element's own text is the transcript. Handled
                // before the field dispatch, which knows nothing about it.
                if ep.is_some() && stack.last().is_some_and(|f| f == "transcript") {
                    if let Some(p) = ep.as_mut()
                        && let Some(tr) = p.transcripts.last_mut()
                    {
                        let text = t.decode().unwrap_or_default().trim().to_string();
                        if !text.is_empty() {
                            tr.text = Some(text);
                        }
                    }
                    continue;
                }
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
                if ep.is_some() && stack.last().is_some_and(|f| f == "transcript") {
                    if let Some(p) = ep.as_mut()
                        && let Some(tr) = p.transcripts.last_mut()
                    {
                        let text = c.decode().unwrap_or_default().trim().to_string();
                        if !text.is_empty() {
                            tr.text = Some(text);
                        }
                    }
                    continue;
                }
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
                // The name is checked *before* `take()`. Written the other way
                // round, `take()` ran for every end tag, so the closing tag of
                // a child element — `<title>`, `<description>`, `<guid>`, which
                // is to say all of them — consumed the episode being built and
                // the `&&` then discarded it. By the time `</item>` arrived
                // there was nothing left to push, and a feed parsed to zero
                // episodes.
                if (name == "item" || name == "entry")
                    && let Some(mut p) = ep.take()
                {
                    // An entry with no audio URL used to be discarded here. That
                    // is what made the whole feed look broken when a single
                    // item used an enclosure shape the parser did not read: the
                    // episode silently vanished from the list, and with it the
                    // transcript, the title and the date that were all sitting
                    // right there. The entry is kept, and `play` reports that
                    // the audio is missing rather than the episode being a lie.
                    p.transcripts.retain(|t| t.is_present());
                    if p.title.trim().is_empty() {
                        p.title = title.clone();
                    }
                    if p.id.trim().is_empty() {
                        p.id = sensible_id(if p.url.is_empty() { &p.title } else { &p.url });
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
    let art = image.clone();
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
            // An episode that carries its own art keeps it; most do not, and
            // the channel's is a better answer than none.
            image_url: e.image.or_else(|| art.clone()),
            transcripts: e.transcripts,
        })
        .collect();
    Ok(ParsedFeed {
        title,
        description,
        image,
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
    /// Artwork this entry names for itself, from `<image href>` inside it.
    image: Option<String>,
    transcripts: Vec<PodcastTranscript>,
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
        "description" | "summary" | "subtitle" if ep.description.is_empty() => {
            ep.description = text.to_string();
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
    m.starts_with("audio/")
        || m.contains("mp3")
        || m.contains("mpeg")
        || m.contains("ogg")
        || m.is_empty()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn feed(body: &str) -> ParsedFeed {
        parse_feed(body, "https://example.com/feed.xml").expect("parse")
    }

    /// A `<podcast:transcript>` is self-closing in almost every real feed. It
    /// was not read, so the transcript was invisible and the episode looked
    /// like a feed that simply did not have one.
    #[test]
    fn a_self_closing_transcript_link_is_read() {
        let f = feed(
            r#"<rss version="2.0" xmlns:podcast="https://podcastindex.org/namespace/1.0">
<channel><title>Show</title>
<item>
  <title>Ep 1</title>
  <enclosure url="https://cdn.example.com/1.mp3" type="audio/mpeg"/>
  <podcast:transcript url="https://cdn.example.com/1.vtt" type="text/vtt" rel="captions"/>
</item>
</channel></rss>"#,
        );
        assert_eq!(f.episodes.len(), 1);
        let t = &f.episodes[0].transcripts;
        assert_eq!(t.len(), 1, "{t:?}");
        assert_eq!(t[0].url.as_deref(), Some("https://cdn.example.com/1.vtt"));
        assert_eq!(t[0].kind.as_deref(), Some("text/vtt"));
        assert_eq!(t[0].rel.as_deref(), Some("captions"));
        assert!(t[0].text.is_none());
    }

    /// Channel artwork, in both the shapes feeds publish it.
    ///
    /// `<itunes:image href>` is an empty element that carries only its
    /// attribute, and Atom's `<logo>`/`<icon>` do the same. Neither was read, so
    /// podcasts had no artwork anywhere: not in the subscription list, and not
    /// on an episode card.
    #[test]
    fn art_field_is_read_when_published() {
        let rss = feed(
            r#"<rss version="2.0" xmlns:itunes="http://www.itunes.com/dtds/podcast-1.0.dtd">
<channel><title>Show</title>
<itunes:image href="https://cdn.example.com/cover.jpg"/>
<item>
  <title>Ep 1</title>
  <enclosure url="https://cdn.example.com/1.mp3" type="audio/mpeg"/>
</item>
</channel></rss>"#,
        );
        assert_eq!(
            rss.image.as_deref(),
            Some("https://cdn.example.com/cover.jpg")
        );
        // The channel's art is the episode's default.
        assert_eq!(
            rss.episodes[0].image_url.as_deref(),
            Some("https://cdn.example.com/cover.jpg"),
            "an episode with no art of its own should inherit the channel's"
        );

        let atom = feed(
            r#"<feed xmlns="http://www.w3.org/2005/Atom">
<title>Show</title>
<logo>https://cdn.example.com/logo.png</logo>
<entry>
  <title>Ep 1</title>
  <link rel="enclosure" href="https://cdn.example.com/1.mp3" type="audio/mpeg"/>
  <image href="https://cdn.example.com/ep1.png"/>
</entry>
</feed>"#,
        );
        assert_eq!(
            atom.image.as_deref(),
            Some("https://cdn.example.com/logo.png")
        );
        assert_eq!(
            atom.episodes[0].image_url.as_deref(),
            Some("https://cdn.example.com/ep1.png"),
            "a per-episode image must win over the channel's"
        );
    }

    /// The other shape: the element is open and its CDATA *is* the transcript.
    /// Reading only the attribute would have produced a transcript with no
    /// body and no url, which `is_present` then discards.
    #[test]
    fn an_inline_transcript_body_is_read() {
        let f = feed(
            r#"<rss version="2.0" xmlns:podcast="https://podcastindex.org/namespace/1.0">
<channel><title>Show</title>
<item>
  <title>Ep 1</title>
  <enclosure url="https://cdn.example.com/1.mp3" type="audio/mpeg"/>
  <podcast:transcript type="text/vtt"><![CDATA[WEBVTT

00:00:01.000 --> 00:00:03.000
Hello]]></podcast:transcript>
</item>
</channel></rss>"#,
        );
        let t = &f.episodes[0].transcripts;
        assert_eq!(t.len(), 1, "{t:?}");
        let body = t[0].text.as_deref().expect("inline body");
        assert!(body.starts_with("WEBVTT"), "{body:?}");
        assert!(t[0].is_present());
    }

    /// An RSS feed that uses a bare `<transcript>` instead of the namespaced
    /// element. The parser matches on local name, so both work.
    #[test]
    fn an_unprefixed_transcript_element_is_read() {
        let f = feed(
            r#"<rss version="2.0"><channel><title>Show</title>
<item>
  <title>Ep 1</title>
  <enclosure url="https://cdn.example.com/1.mp3" type="audio/mpeg"/>
  <transcript url="https://cdn.example.com/1.srt" type="text/srt"/>
</item></channel></rss>"#,
        );
        assert_eq!(
            f.episodes[0].transcripts.len(),
            1,
            "{:?}",
            f.episodes[0].transcripts
        );
    }

    /// `<media:content>` is how a growing number of feeds carry the audio in
    /// place of an `<enclosure>`. It was not read, so those entries came out
    /// with no URL — and were then thrown away by the enclosure check.
    #[test]
    fn media_content_is_read_as_the_enclosure() {
        let f = feed(
            r#"<rss version="2.0" xmlns:media="http://search.yahoo.com/mrss/">
<channel><title>Show</title>
<item>
  <title>Ep 1</title>
  <media:content url="https://cdn.example.com/1.mp3" type="audio/mpeg"/>
</item></channel></rss>"#,
        );
        assert_eq!(f.episodes.len(), 1, "the entry must not be discarded");
        assert_eq!(f.episodes[0].url, "https://cdn.example.com/1.mp3");
    }

    /// An entry with genuinely no audio anywhere used to be discarded, taking
    /// its transcript, title and date with it. It is kept, and the queue route
    /// is the thing that refuses it — with the episode's name.
    #[test]
    fn an_entry_with_no_audio_is_kept_not_discarded() {
        let f = feed(
            r#"<rss version="2.0"><channel><title>Show</title>
<item>
  <title>Ep 1</title>
  <guid>ep-1</guid>
  <description>words, but no audio</description>
</item></channel></rss>"#,
        );
        assert_eq!(f.episodes.len(), 1, "the entry must not vanish");
        let ep = &f.episodes[0];
        assert!(ep.url.is_empty());
        assert_eq!(ep.title, "Ep 1");
        assert_eq!(ep.id, "ep-1");
    }

    /// A transcript with neither a url nor a body is not a transcript, and
    /// keeping it would make the picker offer a key that always fails.
    #[test]
    fn an_empty_transcript_element_is_dropped() {
        let f = feed(
            r#"<rss version="2.0"><channel><title>Show</title>
<item>
  <title>Ep 1</title>
  <enclosure url="https://cdn.example.com/1.mp3" type="audio/mpeg"/>
  <transcript/>
</item></channel></rss>"#,
        );
        assert!(
            f.episodes[0].transcripts.is_empty(),
            "{:?}",
            f.episodes[0].transcripts
        );
    }

    /// Two transcripts, and the `captions` one is the one for the listener.
    #[test]
    fn transcripts_are_ranked_captions_first() {
        let mut a = PodcastTranscript {
            url: Some("a".into()),
            text: None,
            kind: None,
            rel: None,
        };
        let mut b = PodcastTranscript {
            url: Some("b".into()),
            text: None,
            kind: None,
            rel: Some("captions".into()),
        };
        assert!(b.rank() < a.rank());
        // A hosted transcript is present even with an empty inline body: the
        // body is not the only way to get the text.
        a.text = Some(String::new());
        assert!(a.is_present());
        // An empty body and no url is nothing at all.
        let hollow = PodcastTranscript {
            url: None,
            text: Some(String::new()),
            kind: None,
            rel: Some("captions".into()),
        };
        assert!(!hollow.is_present(), "an empty body is not a transcript");
        b.text = Some("WEBVTT".into());
        assert!(b.is_present());
    }
}
