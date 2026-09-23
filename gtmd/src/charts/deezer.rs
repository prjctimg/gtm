// Deezer Charts provider using the public (no-auth) api.deezer.com endpoints.
// Chart/album/track metadata is served without any key, so the Top Charts
// browser gets a free genre axis (global + per-genre charts). Tracks carry a
// `deezer://track/<id>` URI; streaming them needs the ARL token configured via
// Setup → Deezer, and any missing-token failure surfaces in the normal
// queue/playback error path.

use async_trait::async_trait;
use gtm::shared::chart::{ChartError, ChartPlaylist, ChartProvider, ChartTrack};
use serde::Deserialize;

const DEEZER_API: &str = "https://api.deezer.com";

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
            .map_err(|e| ChartError::Network(format!("deezer chart: {e}")))?;
        let status = resp.status();
        if !status.is_success() {
            return Err(ChartError::Network(format!("deezer chart: HTTP {status}")));
        }
        resp.json()
            .await
            .map_err(|e| ChartError::Parse(format!("deezer chart: {e}")))
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
        true // chart metadata is public; playback separately needs the ARL token
    }

    async fn list_charts(&self) -> Result<Vec<ChartPlaylist>, ChartError> {
        let genres: GenreList = Self::get_json(&format!("{DEEZER_API}/genre")).await?;

        let mut charts = Vec::new();
        // Global chart (genre id 0) first.
        charts.push(ChartPlaylist {
            source_id: "deezer".into(),
            id: "0".into(),
            title: "Global — Top 100".into(),
            description: Some("Deezer top 100 tracks worldwide".into()),
            cover_url: None,
            owner: Some("Deezer".into()),
            track_count: Some(100),
        });
        // One chart per top-level genre (id 0 is the "All" pseudo-genre).
        for g in genres.data {
            if g.id == 0 {
                continue;
            }
            charts.push(ChartPlaylist {
                source_id: "deezer".into(),
                id: g.id.to_string(),
                title: format!("{} — Top 100", g.name),
                description: Some(format!("Deezer top 100 tracks · {}", g.name)),
                cover_url: None,
                owner: Some("Deezer".into()),
                track_count: Some(100),
            });
        }

        if charts.len() <= 1 {
            Err(ChartError::Empty)
        } else {
            Ok(charts)
        }
    }

    async fn chart_tracks(&self, chart_id: &str) -> Result<Vec<ChartTrack>, ChartError> {
        let id: u64 = chart_id
            .parse()
            .map_err(|_| ChartError::Parse(format!("invalid deezer chart id: {chart_id}")))?;
        let tracks: TrackList = Self::get_json(&format!("{DEEZER_API}/chart/{id}/tracks?limit=100"))
            .await?;

        let out = tracks
            .data
            .into_iter()
            .enumerate()
            .map(|(i, t)| ChartTrack {
                index: i,
                title: t.title,
                artists: t.artist.name,
                album: t.album.as_ref().map(|a| a.title.clone()),
                duration_ms: t.duration.map(|d| d * 1000),
                uri: format!("deezer://track/{}", t.id),
                cover_url: t.album.as_ref().and_then(|a| a.cover_medium.clone()),
            })
            .collect::<Vec<_>>();

        if out.is_empty() {
            Err(ChartError::Empty)
        } else {
            Ok(out)
        }
    }
}

// ─── api.deezer.com wire shapes ───

#[derive(Deserialize)]
struct GenreList {
    data: Vec<Genre>,
}

#[derive(Deserialize)]
struct Genre {
    id: u64,
    name: String,
}

#[derive(Deserialize)]
struct TrackList {
    data: Vec<DeezerTrack>,
}

#[derive(Deserialize)]
struct DeezerTrack {
    id: u64,
    title: String,
    duration: Option<u64>,
    artist: DeezerArtist,
    album: Option<DeezerAlbum>,
}

#[derive(Deserialize)]
struct DeezerArtist {
    name: String,
}

#[derive(Deserialize)]
struct DeezerAlbum {
    title: String,
    #[serde(rename = "cover_medium")]
    cover_medium: Option<String>,
}