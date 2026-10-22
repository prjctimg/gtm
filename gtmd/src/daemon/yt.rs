use super::*;

#[cfg(feature = "youtube")]
pub(crate) struct Yt;

#[cfg(feature = "youtube")]
impl Yt {
    pub async fn search(
        inner: &DaemonInner,
        query: &str,
        filter: Option<YTFilter>,
    ) -> Result<DaemonRes, CoreError> {
        inner.health.yt.count.fetch_add(1, Ordering::Relaxed);
        let _ = inner.youtube.lock().await.start_search(query, filter).await;
        Ok(DaemonRes::Ok)
    }

    pub async fn poll(inner: &DaemonInner) -> Result<DaemonRes, CoreError> {
        match inner.youtube.lock().await.poll_results().await {
            Ok(Some((query, results))) => Ok(DaemonRes::YtSearchResults { query, results }),
            Ok(None) => Ok(DaemonRes::Ok),
            Err(e) => {
                inner.health.yt.errors.fetch_add(1, Ordering::Relaxed);
                Ok(DaemonRes::Error { message: e })
            }
        }
    }
}
