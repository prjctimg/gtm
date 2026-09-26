use super::*;

/// Chart providers (Spotify first; further sources plug in via the same trait).
pub(crate) struct Charts;

impl Charts {
    pub async fn sources(inner: &DaemonInner) -> Result<DaemonRes, CoreError> {
        let registry = inner.charts.lock().await;
        Ok(DaemonRes::ChartsSourcesRes {
            sources: registry.sources(),
        })
    }

    pub async fn list(
        inner: &DaemonInner,
        source_id: Option<String>,
    ) -> Result<DaemonRes, CoreError> {
        let registry = inner.charts.lock().await;
        match source_id {
            Some(id) => {
                if let Some(provider) = registry.get(&id) {
                    match provider.list_charts().await {
                        Ok(charts) => Ok(DaemonRes::ChartsListRes { charts }),
                        Err(e) => Ok(DaemonRes::Error {
                            message: format!("charts list failed: {e}"),
                        }),
                    }
                } else {
                    Ok(DaemonRes::Error {
                        message: format!("unknown chart source: {id}"),
                    })
                }
            }
            None => match registry.list_all_charts().await {
                Ok(charts) => Ok(DaemonRes::ChartsListRes { charts }),
                Err(e) => Ok(DaemonRes::Error {
                    message: format!("charts list failed: {e}"),
                }),
            },
        }
    }

    pub async fn tracks(
        inner: &DaemonInner,
        source_id: String,
        chart_id: String,
    ) -> Result<DaemonRes, CoreError> {
        let registry = inner.charts.lock().await;
        match registry.chart_tracks(&source_id, &chart_id).await {
            Ok(tracks) => Ok(DaemonRes::ChartsTracksRes { tracks }),
            Err(e) => Ok(DaemonRes::Error {
                message: format!("charts tracks failed: {e}"),
            }),
        }
    }
}
