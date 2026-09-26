use super::*;

/// Podcast feed subscriptions and episodes.
pub(crate) struct Podcast;

impl Podcast {
    pub async fn add_feed(inner: &DaemonInner, url: &str) -> Result<DaemonRes, CoreError> {
        let mut podcast = inner.podcast.lock().await;
        let feed = podcast.add_feed(url).await.map_err(CoreError::Daemon)?;
        Ok(DaemonRes::PodcastFeedsRes { feeds: vec![feed] })
    }

    pub async fn remove_feed(inner: &DaemonInner, feed_id: &str) -> Result<DaemonRes, CoreError> {
        inner
            .podcast
            .lock()
            .await
            .remove_feed(feed_id)
            .map_err(CoreError::Daemon)?;
        Ok(DaemonRes::Ok)
    }

    pub async fn feeds(inner: &DaemonInner) -> Result<DaemonRes, CoreError> {
        let feeds = inner.podcast.lock().await.feeds();
        Ok(DaemonRes::PodcastFeedsRes { feeds })
    }

    pub async fn episodes(inner: &DaemonInner, feed_id: &str) -> Result<DaemonRes, CoreError> {
        let podcast = inner.podcast.lock().await;
        match podcast.episodes(feed_id) {
            Ok((feed_title, episodes)) => Ok(DaemonRes::PodcastEpisodesRes {
                feed_id: feed_id.to_string(),
                feed_title,
                episodes,
            }),
            Err(e) => Err(CoreError::Daemon(e)),
        }
    }

    pub async fn refresh(
        inner: &DaemonInner,
        feed_id: Option<&str>,
    ) -> Result<DaemonRes, CoreError> {
        let mut podcast = inner.podcast.lock().await;
        match feed_id {
            Some(id) => {
                let feed = podcast.refresh_feed(id).await.map_err(CoreError::Daemon)?;
                Ok(DaemonRes::PodcastFeedsRes { feeds: vec![feed] })
            }
            None => {
                let n = podcast.refresh_all().await.map_err(CoreError::Daemon)?;
                Ok(DaemonRes::Value {
                    value: serde_json::json!({ "refreshed": n }),
                })
            }
        }
    }

    pub async fn status(inner: &DaemonInner) -> Result<DaemonRes, CoreError> {
        let status = inner.podcast.lock().await.status();
        Ok(DaemonRes::PodcastStatusRes { status })
    }

    /// Play the selected episode by enqueueing its synthetic path and calling
    /// the native streaming player.
    pub async fn play(
        inner: &DaemonInner,
        feed_id: &str,
        episode_index: usize,
    ) -> Result<DaemonRes, CoreError> {
        let (feed_title, episode) = {
            let mut podcast = inner.podcast.lock().await;
            match podcast.episode_at(feed_id, episode_index) {
                Some(ep) => (ep.feed_title.clone(), ep),
                None => {
                    podcast
                        .refresh_feed(feed_id)
                        .await
                        .map_err(CoreError::Daemon)?;
                    let ep = podcast
                        .episode_at(feed_id, episode_index)
                        .ok_or_else(|| CoreError::Daemon("episode missing".into()))?;
                    (ep.feed_title.clone(), ep)
                }
            }
        };
        let title = if episode.title.trim().is_empty() {
            feed_title.clone()
        } else {
            episode.title.clone()
        };
        let path = format!("podcast://{feed_id}/{episode_index}");
        let track = TrackInfo {
            id: 0,
            path: path.clone(),
            title,
            artist: feed_title.clone(),
            album: format!("Podcast · {feed_title}"),
            duration: episode.duration_secs.unwrap_or(0) as f64,
            ..Default::default()
        };
        {
            let mut state = inner.state.write().await;
            state.queue.push(track);
        }
        Daemon::push_queue_state(inner).await;
        Cmd::play(inner, &path, 0.0, false).await
    }
}
