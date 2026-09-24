// Chart data model shared by daemon ↔ client. Provider-agnostic: new chart
// sources implement the `ChartProvider` trait (gtmd/src/charts/) and appear in
// the UI without any shared-crate changes.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ChartSource {
    pub id: String,       // "spotify" | "apple" | <community provider id> ...
    pub display: String,  // human label e.g. "Spotify Charts"
    pub configured: bool, // has valid auth / is available
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ChartPlaylist {
    pub source_id: String, // matches ChartSource.id
    pub id: String,        // provider-specific playlist/chart id
    pub title: String,
    pub description: Option<String>,
    pub cover_url: Option<String>,
    pub owner: Option<String>,
    pub track_count: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ChartTrack {
    pub index: usize,
    pub title: String,
    pub artists: String,
    pub album: Option<String>,
    pub duration_ms: Option<u64>,
    pub uri: String, // provider-specific playable URI (e.g. spotify:track:...)
    pub cover_url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ChartError {
    Unconfigured(String),
    Network(String),
    Parse(String),
    Empty,
}

impl std::fmt::Display for ChartError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ChartError::Unconfigured(s) => write!(f, "charts not configured: {s}"),
            ChartError::Network(s) => write!(f, "network error: {s}"),
            ChartError::Parse(s) => write!(f, "parse error: {s}"),
            ChartError::Empty => write!(f, "no charts available"),
        }
    }
}

impl std::error::Error for ChartError {}

// Daemon-side trait. Implementations live in gtmd/src/charts/.
// async_trait makes it dyn-compatible for `Box<dyn ChartProvider>`.
#[async_trait]
pub trait ChartProvider: Send + Sync {
    fn source_id(&self) -> &str;
    fn display_name(&self) -> &str;
    fn is_configured(&self) -> bool;

    /// List available charts/editorial playlists for this provider.
    async fn list_charts(&self) -> Result<Vec<ChartPlaylist>, ChartError>;

    /// Fetch tracks for a specific chart playlist.
    async fn chart_tracks(&self, chart_id: &str) -> Result<Vec<ChartTrack>, ChartError>;
}
