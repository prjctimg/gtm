// Spotify Charts provider using rspotify. Fetches editorial "Top 50" / "Viral 50"
// category playlists as the chart list, then playlist tracks for playback.

use super::super::spotify::{SpotifyManager, track_from_playable};
use async_trait::async_trait;
use gtm::shared::chart::{ChartError, ChartPlaylist, ChartProvider, ChartTrack};
use rspotify::clients::BaseClient;
use tracing::warn;

pub struct SpotifyCharts {
    spotify: std::sync::Arc<tokio::sync::Mutex<SpotifyManager>>,
}

impl SpotifyCharts {
    pub fn new(spotify: std::sync::Arc<tokio::sync::Mutex<SpotifyManager>>) -> Self {
        Self { spotify }
    }
}

#[async_trait]
impl ChartProvider for SpotifyCharts {
    fn source_id(&self) -> &str {
        "spotify"
    }

    fn display_name(&self) -> &str {
        "Spotify Charts"
    }

    fn is_configured(&self) -> bool {
        true // added to registry only when client exists
    }

    async fn list_charts(&self) -> Result<Vec<ChartPlaylist>, ChartError> {
        let client = {
            let s = self.spotify.lock().await;
            s.sync_client()
        };
        let Some(client) = client else {
            return Err(ChartError::Unconfigured("spotify not linked".into()));
        };

        // Well-known global chart playlist IDs (these are stable Spotify editorial playlists)
        let chart_playlist_ids = [
            "37i9dQZEVXbMDoHDwVN2tF", // Top 50 - Global
            "37i9dQZEVXbLiRSasKsNU9", // Top 50 - USA
            "37i9dQZEVXbNFJfN1Vw8d9", // Top 50 - UK
            "37i9dQZEVXbKXQ4mDTEBXq", // Viral 50 - Global
            "37i9dQZEVXbKCF6dqVpKkS", // Viral 50 - USA
        ];

        let mut charts = Vec::new();
        for id in chart_playlist_ids {
            let playlist_id = match rspotify::model::PlaylistId::from_id(id) {
                Ok(pid) => pid,
                Err(_) => continue,
            };

            match client.playlist(playlist_id, None, None).await {
                Ok(pl) => {
                    let pid = pl.id.to_string();
                    let track_count = pl.items.items.len();
                    charts.push(ChartPlaylist {
                        source_id: "spotify".into(),
                        id: pid,
                        title: pl.name,
                        description: pl.description,
                        cover_url: crate::spotify::pick_largest_image(&pl.images),
                        owner: pl.owner.display_name,
                        track_count: Some(track_count),
                    });
                }
                Err(e) => {
                    warn!("failed to fetch chart playlist {id}: {e}");
                    continue;
                }
            }
        }

        if charts.is_empty() {
            Err(ChartError::Empty)
        } else {
            Ok(charts)
        }
    }

    async fn chart_tracks(&self, chart_id: &str) -> Result<Vec<ChartTrack>, ChartError> {
        let client = {
            let s = self.spotify.lock().await;
            s.sync_client()
        };
        let Some(client) = client else {
            return Err(ChartError::Unconfigured("spotify not linked".into()));
        };

        let playlist_id = match rspotify::model::PlaylistId::from_id(chart_id) {
            Ok(pid) => pid,
            Err(_) => return Err(ChartError::Parse(format!("invalid chart id: {chart_id}"))),
        };

        let mut items = client.playlist_items(playlist_id, None, None);
        let mut tracks = Vec::new();
        let mut index = 0;

        use futures::StreamExt;
        while let Some(item) = items.next().await {
            match item {
                Ok(item) => {
                    if let Some(playable) = item.item.as_ref()
                        && let Some(st) = track_from_playable(playable)
                    {
                        let uri = st.uri.unwrap_or_default();
                        if !uri.is_empty() {
                            tracks.push(ChartTrack {
                                index,
                                title: st.name,
                                artists: st.artists,
                                album: st.album,
                                duration_ms: st.duration_ms,
                                uri,
                                cover_url: st.image_url,
                            });
                            index += 1;
                        }
                    }
                }
                Err(e) => warn!("spotify chart track: {e}"),
            }
        }

        if tracks.is_empty() {
            Err(ChartError::Empty)
        } else {
            Ok(tracks)
        }
    }
}
