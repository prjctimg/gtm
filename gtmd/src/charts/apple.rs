// iTunes / Apple RSS "Top Songs" chart provider — entirely free and
// unauthenticated. Charts are built from the classic iTunes RSS generator
// (itunes.apple.com/{cc}/rss/topsongs/...), which supports both a country and
// a genre axis, so the Top Charts picker can offer per-country and per-genre
// lists without any account. Rows resolve playable 30-second preview URLs via
// the iTunes lookup API (one batched request per chart), so a highlighted
// track can still be enqueued and streamed like any other http(s) source.

use async_trait::async_trait;
use gtm::shared::chart::{ChartError, ChartPlaylist, ChartProvider, ChartTrack};
use serde::Deserialize;

/// `{cc}` → display name for the per-country charts.
const COUNTRIES: &[(&str, &str)] = &[
    ("us", "United States"),
    ("gb", "United Kingdom"),
    ("ca", "Canada"),
    ("au", "Australia"),
    ("de", "Germany"),
    ("fr", "France"),
    ("jp", "Japan"),
    ("kr", "South Korea"),
    ("in", "India"),
    ("br", "Brazil"),
    ("mx", "Mexico"),
    ("za", "South Africa"),
];

/// `{iTunes genre id}` → display name for the per-genre charts (US store).
const GENRES: &[(u64, &str)] = &[
    (14, "Pop"),
    (21, "Rock"),
    (18, "Hip-Hop/Rap"),
    (7, "Electronic/Dance"),
    (15, "R&B/Soul"),
    (6, "Country"),
];

pub struct AppleCharts;

impl AppleCharts {
    pub fn new() -> Self {
        Self
    }

    // ═══ wire parsing ═══

    async fn get_json<T: serde::de::DeserializeOwned>(url: &str) -> Result<T, ChartError> {
        let resp = reqwest::Client::new()
            .get(url)
            .timeout(std::time::Duration::from_secs(15))
            .send()
            .await
            .map_err(|e| ChartError::Network(format!("itunes rss: {e}")))?;
        let status = resp.status();
        if !status.is_success() {
            return Err(ChartError::Network(format!("itunes rss: HTTP {status}")));
        }
        resp.json()
            .await
            .map_err(|e| ChartError::Parse(format!("itunes rss: {e}")))
    }

    /// Fetch the RSS feed for the country/genre chart and return the raw
    /// entries with their iTunes track ids (for the follow-up lookup call).
    async fn fetch_entries(cc: &str, genre: Option<u64>) -> Result<Vec<RssEntry>, ChartError> {
        let mut url = format!(
            "https://itunes.apple.com/{cc}/rss/topsongs/limit=100"
        );
        if let Some(gid) = genre {
            url.push_str(&format!("/genre={gid}"));
        }
        url.push_str("/json");
        let feed: RssFeed = Self::get_json(&url).await?;
        Ok(match feed.feed.entry {
            RssEntries::One(e) => vec![e],
            RssEntries::Many(v) => v,
        })
    }

    async fn lookup(ids: &[String]) -> Result<std::collections::HashMap<String, LookupTrack>, ChartError> {
        let mut tracks = std::collections::HashMap::new();
        // The lookup API accepts up to 200 comma-separated ids per request;
        // a 100-track chart always fits in one.
        for chunk in ids.chunks(200) {
            let url = format!(
                "https://itunes.apple.com/lookup?id={}&entity=song",
                chunk.join(",")
            );
            let resp: LookupResponse = Self::get_json(&url).await?;
            for t in resp.results {
                tracks.insert(t.track_id.to_string(), t);
            }
        }
        Ok(tracks)
    }
}

#[async_trait]
impl ChartProvider for AppleCharts {
    fn source_id(&self) -> &str {
        "apple"
    }

    fn display_name(&self) -> &str {
        "iTunes Top Songs"
    }

    fn is_configured(&self) -> bool {
        true // public, no-auth RSS feed
    }

    async fn list_charts(&self) -> Result<Vec<ChartPlaylist>, ChartError> {
        let mut charts = Vec::new();
        for (cc, name) in COUNTRIES {
            charts.push(ChartPlaylist {
                source_id: "apple".into(),
                id: format!("{cc}|songs"),
                title: format!("{name} — Top Songs"),
                description: Some(format!("iTunes top 100 songs · {name}")),
                cover_url: None,
                owner: Some("iTunes Store".into()),
                track_count: Some(100),
            });
        }
        for (gid, gname) in GENRES {
            charts.push(ChartPlaylist {
                source_id: "apple".into(),
                id: format!("us|{gid}"),
                title: format!("Top Songs — {gname}"),
                description: Some(format!("iTunes top 100 songs · {gname} (US)")),
                cover_url: None,
                owner: Some("iTunes Store".into()),
                track_count: Some(100),
            });
        }
        Ok(charts)
    }

    async fn chart_tracks(&self, chart_id: &str) -> Result<Vec<ChartTrack>, ChartError> {
        let Some((cc, rest)) = chart_id.split_once('|') else {
            return Err(ChartError::Parse(format!("invalid apple chart id: {chart_id}")));
        };
        if cc.is_empty() {
            return Err(ChartError::Parse(format!("invalid apple chart id: {chart_id}")));
        }
        let genre = if rest == "songs" {
            None
        } else {
            match rest.parse::<u64>() {
                Ok(gid) => Some(gid),
                Err(_) => {
                    return Err(ChartError::Parse(format!(
                        "invalid apple chart id: {chart_id}"
                    )))
                }
            }
        };

        let entries = Self::fetch_entries(cc, genre).await?;
        let ids: Vec<String> = entries
            .iter()
            .filter_map(|e| e.id.as_ref().and_then(|i| i.attributes.as_ref()).and_then(|a| a.im_id.clone()))
            .collect();
        let lookup = Self::lookup(&ids).await?;

        let mut tracks = Vec::new();
        for (i, entry) in entries.iter().enumerate() {
            let track_id = entry
                .id
                .as_ref()
                .and_then(|id| id.attributes.as_ref())
                .and_then(|a| a.im_id.clone())
                .unwrap_or_default();
            let detail = lookup.get(&track_id);

            let title = detail
                .and_then(|d| d.track_name.clone())
                .or_else(|| entry.name.as_ref().map(|n| n.label.clone()))
                .or_else(|| {
                    entry
                        .title
                        .as_ref()
                        .map(|t| t.label.split(" - ").next().unwrap_or("").to_string())
                })
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| "Unknown".to_string());
            let artists = detail
                .and_then(|d| d.artist_name.clone())
                .or_else(|| {
                    entry.title.as_ref().and_then(|t| {
                        t.label.split_once(" - ").map(|(_, a)| a.to_string())
                    })
                })
                .unwrap_or_default();
            let album = detail
                .and_then(|d| d.collection_name.clone())
                .or_else(|| {
                    entry
                        .collection
                        .as_ref()
                        .and_then(|c| c.name.as_ref())
                        .map(|n| n.label.clone())
                });
            let cover = detail
                .and_then(|d| d.artwork_url100.clone())
                .map(|u| upgrade_artwork(&u));
            let uri = detail.and_then(|d| d.preview_url.clone()).unwrap_or_default();

            tracks.push(ChartTrack {
                index: i,
                title,
                artists,
                album,
                duration_ms: detail.and_then(|d| d.track_time_millis),
                uri,
                cover_url: cover,
            });
        }

        if tracks.is_empty() {
            Err(ChartError::Empty)
        } else {
            Ok(tracks)
        }
    }
}

/// artowrkUrl100 is capped at 100px for the feed/lookup payload; bump the
/// thumbnail size for the cover so the now-playing art is not pixelated.
fn upgrade_artwork(url: &str) -> String {
    url.replace("/100x100bb.jpg", "/600x600bb.jpg")
}

// ─── iTunes RSS feed shape ───

#[derive(Deserialize)]
struct RssFeed {
    feed: RssFeedInner,
}

#[derive(Deserialize)]
struct RssFeedInner {
    entry: RssEntries,
}

/// The feed wraps a single entry as a bare object when limit=1; normalise to a
/// list either way.
#[derive(Deserialize)]
#[serde(untagged)]
enum RssEntries {
    Many(Vec<RssEntry>),
    One(RssEntry),
}

#[derive(Deserialize)]
struct RssEntry {
    #[serde(rename = "im:name")]
    name: Option<RssLabel>,
    title: Option<RssLabel>,
    id: Option<RssId>,
    #[serde(rename = "im:collection")]
    collection: Option<RssCollection>,
}

#[derive(Deserialize)]
struct RssLabel {
    label: String,
}

#[derive(Deserialize)]
struct RssId {
    attributes: Option<RssIdAttrs>,
}

#[derive(Deserialize)]
struct RssIdAttrs {
    #[serde(rename = "im:id")]
    im_id: Option<String>,
}

#[derive(Deserialize)]
struct RssCollection {
    #[serde(rename = "im:name")]
    name: Option<RssLabel>,
}

// ─── iTunes lookup (preview URL / duration / artwork) ───

#[derive(Deserialize)]
struct LookupResponse {
    results: Vec<LookupTrack>,
}

#[derive(Deserialize)]
struct LookupTrack {
    #[serde(rename = "trackId")]
    track_id: u64,
    #[serde(rename = "trackName")]
    track_name: Option<String>,
    #[serde(rename = "artistName")]
    artist_name: Option<String>,
    #[serde(rename = "collectionName")]
    collection_name: Option<String>,
    #[serde(rename = "previewUrl")]
    preview_url: Option<String>,
    #[serde(rename = "trackTimeMillis")]
    track_time_millis: Option<u64>,
    #[serde(rename = "artworkUrl100")]
    artwork_url100: Option<String>,
}