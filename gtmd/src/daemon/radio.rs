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

    pub async fn cliamp(inner: &DaemonInner, refresh: bool) -> Result<DaemonRes, CoreError> {
        let radio = inner.radio.lock().await;
        let stations = radio.cliamp(refresh).await.map_err(CoreError::Daemon)?;
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
        station_name: &str,
    ) -> Result<DaemonRes, CoreError> {
        // Don't push radio into the queue - live streams don't belong there.
        // The name rides in the path because `parse_remote_path` reads it back
        // out to label the track: a station published as `radio://<id>` alone
        // has no name to resolve, and the now-playing line came up empty
        // whenever the stream published no ICY title to stand in for it.
        let path = match station_name.trim() {
            "" => format!("radio://{station_id}"),
            name => format!("radio://{station_id}/{name}"),
        };
        Cmd::play(inner, &path, 0.0, false).await
    }
}
