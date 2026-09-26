use super::*;

/// Radio Browser directory integration.
pub(crate) struct Radio;

impl Radio {
    pub async fn search(
        inner: &DaemonInner,
        query: &str,
        limit: u16,
    ) -> Result<DaemonRes, CoreError> {
        let radio = inner.radio.lock().await;
        let stations = radio
            .search(query, limit)
            .await
            .map_err(CoreError::Daemon)?;
        Ok(DaemonRes::RadioStationsRes { stations })
    }

    pub async fn top(inner: &DaemonInner, limit: u16) -> Result<DaemonRes, CoreError> {
        let radio = inner.radio.lock().await;
        let stations = radio.top(limit).await.map_err(CoreError::Daemon)?;
        Ok(DaemonRes::RadioStationsRes { stations })
    }

    pub async fn tags(inner: &DaemonInner, limit: u16) -> Result<DaemonRes, CoreError> {
        let radio = inner.radio.lock().await;
        let tags = radio.tags(limit).await.map_err(CoreError::Daemon)?;
        Ok(DaemonRes::RadioTagsRes { tags })
    }

    pub async fn by_tag(
        inner: &DaemonInner,
        tag: &str,
        limit: u16,
    ) -> Result<DaemonRes, CoreError> {
        let radio = inner.radio.lock().await;
        let stations = radio
            .stations_by_tag(tag, limit)
            .await
            .map_err(CoreError::Daemon)?;
        Ok(DaemonRes::RadioStationsRes { stations })
    }

    pub async fn countries(inner: &DaemonInner, limit: u16) -> Result<DaemonRes, CoreError> {
        let radio = inner.radio.lock().await;
        let countries = radio.countries(limit).await.map_err(CoreError::Daemon)?;
        Ok(DaemonRes::RadioCountriesRes { countries })
    }

    pub async fn by_country(
        inner: &DaemonInner,
        country: &str,
        limit: u16,
    ) -> Result<DaemonRes, CoreError> {
        let radio = inner.radio.lock().await;
        let stations = radio
            .stations_by_country(country, limit)
            .await
            .map_err(CoreError::Daemon)?;
        Ok(DaemonRes::RadioStationsRes { stations })
    }

    pub async fn play(
        inner: &DaemonInner,
        station_id: &str,
        _station_name: &str,
    ) -> Result<DaemonRes, CoreError> {
        // Don't push radio into the queue - live streams don't belong there.
        // play_remote() builds the TrackInfo from the Radio kind's station_id,
        // so the playlist-name fallback is no longer needed.
        let path = format!("radio://{station_id}");
        Cmd::play(inner, &path, 0.0, false).await
    }
}
