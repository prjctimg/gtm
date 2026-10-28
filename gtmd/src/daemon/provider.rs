use super::*;

use gtm::providers::external::ProviderTrack;

/// Catalogue provider requests.
pub(crate) struct Provider;

impl Provider {
    pub async fn sources(inner: &DaemonInner) -> Result<DaemonRes, CoreError> {
        Ok(DaemonRes::ProviderSourcesRes {
            sources: inner.providers.sources(),
        })
    }

    pub async fn search(
        inner: &DaemonInner,
        provider: &str,
        query: &str,
        limit: u16,
    ) -> Result<DaemonRes, CoreError> {
        let query = query.trim();
        if query.is_empty() {
            return Ok(DaemonRes::ProviderTracksRes { tracks: Vec::new() });
        }
        let tracks = inner
            .providers
            .search(provider, query, usize::from(limit))
            .await
            .map_err(CoreError::Daemon)?;
        Ok(DaemonRes::ProviderTracksRes { tracks })
    }

    pub async fn playlists(inner: &DaemonInner, provider: &str) -> Result<DaemonRes, CoreError> {
        let playlists = inner
            .providers
            .playlists(provider)
            .await
            .map_err(CoreError::Daemon)?;
        Ok(DaemonRes::ProviderPlaylistsRes { playlists })
    }

    pub async fn playlist_tracks(
        inner: &DaemonInner,
        provider: &str,
        id: &str,
    ) -> Result<DaemonRes, CoreError> {
        let tracks = inner
            .providers
            .playlist_tracks(provider, id)
            .await
            .map_err(CoreError::Daemon)?;
        Ok(DaemonRes::ProviderTracksRes { tracks })
    }

    /// Queue one provider track with its metadata already attached, then play
    /// it.
    ///
    /// The row goes into the queue first because playback resolves the path a
    /// second time (and every auto-advance resolves it again). Without the row,
    /// the now-playing pane would have only the opaque id to show while the
    /// provider's network call was in flight.
    pub async fn play(
        inner: &DaemonInner,
        provider: &str,
        id: &str,
        title: &str,
        artist: &str,
        album: &str,
        duration_secs: f64,
    ) -> Result<DaemonRes, CoreError> {
        inner
            .providers
            .available(provider)
            .map_err(CoreError::Daemon)?;
        let spec = gtm::providers::external::provider_spec(provider)
            .ok_or_else(|| CoreError::Daemon(format!("unknown provider {provider:?}")))?;
        if !spec.caps.stream {
            return Err(CoreError::Daemon(format!(
                "{} does not support streaming",
                spec.name
            )));
        }
        let track = ProviderTrack {
            provider: provider.to_string(),
            id: id.to_string(),
            title: if title.trim().is_empty() {
                format!("{} track", spec.name)
            } else {
                title.to_string()
            },
            artist: artist.to_string(),
            album: album.to_string(),
            duration_secs,
            ..Default::default()
        };
        let path = track.path();
        let row = TrackInfo {
            id: 0,
            path: path.clone(),
            title: track.title.clone(),
            artist: track.artist.clone(),
            album: track.album.clone(),
            duration: track.duration_secs,
            favourite: false,
            ..Default::default()
        };
        {
            let mut state = inner.state.write().await;
            state.queue.push(row);
        }
        Daemon::push_queue_state(inner).await;
        Cmd::play(inner, &path, 0.0, false).await
    }

    pub async fn like(
        inner: &DaemonInner,
        provider: &str,
        id: &str,
        title: &str,
    ) -> Result<DaemonRes, CoreError> {
        inner
            .providers
            .like(provider, id, title)
            .await
            .map_err(CoreError::Daemon)?;
        Ok(DaemonRes::Ok)
    }
}
