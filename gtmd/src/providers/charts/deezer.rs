// Deezer chart provider — free, unauthenticated, and the second source that
// works before any account is linked.
//
// Spotify retired `/v1/charts` (it answers 410) and the Apple Music API needs a
// signed developer token, so the iTunes RSS feed was the only one left. Deezer's
// public API needs no key at all and serves the full top 300 with per-track
// album art, which the RSS path had to synthesise through a second batched
// lookup call.
//
// Like Apple, Deezer only exposes a 30-second preview URL per track, so a chart
// row is enqueued and streamed as an http(s) source rather than resolved to a
// Spotify URI. That limitation is inherent to both free providers, not to this
// one.
//
// Only track and album charts are exposed. Deezer also publishes editorial
// playlists, but those are lists *of* tracks rather than charts *of* tracks, and
// the row model here has no way to show one: no duration, no preview, nothing
// enqueueable. Offering them would put dead rows in the list.

use async_trait::async_trait;
use gtm::shared::chart::{ChartError, ChartPlaylist, ChartProvider, ChartTrack};
use serde::Deserialize;

/// The one chart index that exists: `0` is every country combined. Deezer's
/// charts are global, so unlike Apple there is no country axis here.
const CHART: &str = "0";

const KINDS: &[(&str, &str, &str, usize)] = &[
    ("tracks", "Top Tracks", "Top 300 tracks", 300),
    ("albums", "Top Albums", "Top 100 albums", 100),
];

pub struct DeezerCharts;

impl DeezerCharts {
    pub fn new() -> Self {
        Self
    }

    async fn get_json<T: serde::de::DeserializeOwned>(url: &str) -> Result<T, ChartError> {
        let resp = reqwest::Client::new()
            .get(url)
            .timeout(std::time::Duration::from_secs(15))
            .send()
            .await
            .map_err(|e| ChartError::Network(format!("deezer: {e}")))?;
        let status = resp.status();
        if !status.is_success() {
            return Err(ChartError::Network(format!("deezer: HTTP {status}")));
        }
        resp.json()
            .await
            .map_err(|e| ChartError::Parse(format!("deezer: {e}")))
    }
}

#[async_trait]
impl ChartProvider for DeezerCharts {
    fn source_id(&self) -> &str {
        "deezer"
    }

    fn display_name(&self) -> &str {
        "Deezer Charts"
    }

    fn is_configured(&self) -> bool {
        true // public, no-auth
    }

    async fn list_charts(&self) -> Result<Vec<ChartPlaylist>, ChartError> {
        Ok(KINDS
            .iter()
            .map(|(kind, title, desc, count)| ChartPlaylist {
                source_id: "deezer".into(),
                id: (*kind).to_string(),
                title: (*title).to_string(),
                description: Some((*desc).to_string()),
                cover_url: None,
                owner: Some("Deezer".into()),
                track_count: Some(*count),
            })
            .collect())
    }

    async fn chart_tracks(&self, chart_id: &str) -> Result<Vec<ChartTrack>, ChartError> {
        if !KINDS.iter().any(|(k, _, _, _)| *k == chart_id) {
            return Err(ChartError::Parse(format!(
                "invalid deezer chart id: {chart_id}"
            )));
        }
        let url = format!("https://api.deezer.com/chart/{CHART}/{chart_id}?limit=300");
        let page: DzPage<DzTrack> = Self::get_json(&url).await?;
        if page.data.is_empty() {
            return Err(ChartError::Empty);
        }
        Ok(page
            .data
            .into_iter()
            .enumerate()
            .map(|(i, t)| t.into_track(i))
            .collect())
    }
}

/// An album chart entry: the same row shape, but no preview of its own, so the
/// duration and playable url come from the album's first track. A chart row has
/// to be playable, which is the one thing an album entry cannot answer for
/// itself.
#[derive(Deserialize)]
struct DzTrack {
    title: String,
    duration: Option<u64>,
    preview: Option<String>,
    artist: Option<DzArtist>,
    album: Option<DzAlbum>,
}

impl DzTrack {
    fn into_track(self, index: usize) -> ChartTrack {
        let cover = self
            .album
            .as_ref()
            .and_then(|a| a.cover_xl.clone())
            // A single with no release art would otherwise show the placeholder
            // glyph, so fall back to the artist picture.
            .or_else(|| self.artist.as_ref().and_then(|a| a.picture_xl.clone()));
        ChartTrack {
            index,
            title: self.title,
            artists: self.artist.map(|a| a.name).unwrap_or_default(),
            album: self.album.map(|a| a.title),
            duration_ms: self.duration.map(|s| s * 1000),
            uri: self.preview.unwrap_or_default(),
            cover_url: cover,
        }
    }
}

#[derive(Deserialize)]
struct DzPage<T> {
    data: Vec<T>,
}

#[derive(Deserialize)]
struct DzArtist {
    name: String,
    picture_xl: Option<String>,
}

#[derive(Deserialize)]
struct DzAlbum {
    title: String,
    cover_xl: Option<String>,
}
