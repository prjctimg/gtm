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

    pub async fn cancel(inner: &DaemonInner) -> Result<DaemonRes, CoreError> {
        inner.youtube.lock().await.cancel().await;
        Ok(DaemonRes::Ok)
    }

    pub async fn resolve_stream(inner: &DaemonInner, url: &str) -> Result<DaemonRes, CoreError> {
        let (auth, sem, gate) = {
            let yt = inner.youtube.lock().await;
            yt.yt_extras()
        };
        match crate::youtube::resolve_stream_ytdlp(&sem, &gate, &auth, url).await {
            Ok(direct) => Ok(DaemonRes::StreamInfo {
                info: Box::new(StreamInfo {
                    url: direct,
                    title: url.to_string(),
                    ext: "m4a".to_string(),
                    duration: 0.0,
                }),
            }),
            Err(e) => Ok(DaemonRes::Error { message: e }),
        }
    }
}
