use super::*;

/// Shared YouTube-fallback path for Spotify tracks: search `query`, pick the
/// top hit, and download its audio into the cache under `cache_key` via
/// yt-dlp. Returns the local file path. The youtube lock is held only during
/// the (fast) search, dropped before the (slow) download.
#[cfg(feature = "youtube")]
pub(crate) async fn spotify_yt_fallback(
    inner: &DaemonInner,
    cache_key: &str,
    query: &str,
) -> Result<String, String> {
    {
        let mut yt = inner.youtube.lock().await;
        yt.search(query, None).await?;
        let top = match yt.poll_results().await {
            Ok(Some((_, mut results))) if !results.is_empty() => results.remove(0),
            _ => return Err("no youtube results for track".to_string()),
        };
        let (auth, sem, gate) = yt.yt_extras();
        drop(yt);
        Daemon::download_to_cache(
            &inner.config.cache_dir,
            cache_key,
            &top.url,
            auth,
            sem,
            gate,
        )
        .await
    }
}

#[cfg(not(feature = "youtube"))]
pub(crate) async fn spotify_yt_fallback(
    _inner: &DaemonInner,
    _cache_key: &str,
    _query: &str,
) -> Result<String, String> {
    Err("youtube support is disabled in this build".to_string())
}
