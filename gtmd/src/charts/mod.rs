// Chart providers registry — dynamically lists configured sources.

mod spotify;

use crate::spotify::SpotifyManager;
use gtm::shared::chart::{ChartError, ChartPlaylist, ChartProvider, ChartSource, ChartTrack};

pub struct ChartsRegistry {
    providers: Vec<Box<dyn ChartProvider>>,
}

impl ChartsRegistry {
    pub fn empty() -> Self {
        Self {
            providers: Vec::new(),
        }
    }

    pub fn add_spotify(&mut self, spotify: std::sync::Arc<tokio::sync::Mutex<SpotifyManager>>) {
        self.providers.push(Box::new(SpotifyCharts::new(spotify)));
    }

    /// Register the Spotify charts provider exactly once. The registry is
    /// built in `Daemon::new` before any token is loaded, so the startup and
    /// OAuth-link paths re-register here once the client is actually ready.
    pub fn ensure_spotify(&mut self, spotify: std::sync::Arc<tokio::sync::Mutex<SpotifyManager>>) {
        if self.get("spotify").is_none() {
            self.add_spotify(spotify);
        }
    }

    pub fn sources(&self) -> Vec<ChartSource> {
        self.providers
            .iter()
            .map(|p| ChartSource {
                id: p.source_id().to_string(),
                display: p.display_name().to_string(),
                configured: p.is_configured(),
            })
            .collect()
    }

    pub fn get(&self, source_id: &str) -> Option<&dyn ChartProvider> {
        self.providers
            .iter()
            .find(|p| p.source_id() == source_id)
            .map(|p| p.as_ref())
    }

    /// List charts from all configured providers, annotated with source.
    pub async fn list_all_charts(&self) -> Result<Vec<ChartPlaylist>, ChartError> {
        let mut all = Vec::new();
        for p in &self.providers {
            if !p.is_configured() {
                continue;
            }
            match p.list_charts().await {
                Ok(mut charts) => {
                    for c in &mut charts {
                        c.source_id = p.source_id().to_string();
                    }
                    all.append(&mut charts);
                }
                Err(ChartError::Unconfigured(_)) => {}
                Err(e) => return Err(e),
            }
        }
        if all.is_empty() {
            Err(ChartError::Empty)
        } else {
            Ok(all)
        }
    }

    /// Fetch tracks for a chart from a specific source.
    pub async fn chart_tracks(
        &self,
        source_id: &str,
        chart_id: &str,
    ) -> Result<Vec<ChartTrack>, ChartError> {
        let provider = self.get(source_id).ok_or_else(|| {
            ChartError::Unconfigured(format!("unknown chart source: {source_id}"))
        })?;
        provider.chart_tracks(chart_id).await
    }
}

pub use self::spotify::SpotifyCharts;
