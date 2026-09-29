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

    /// Fetch and parse an episode's transcript, in the shape the lyrics pane
    /// already renders.
    pub async fn transcript(
        inner: &DaemonInner,
        feed_id: &str,
        episode_index: usize,
    ) -> Result<DaemonRes, CoreError> {
        // The episode has to be in cache to know what transcripts it offers, so
        // an uncached feed is refreshed rather than reported as having none.
        let mut podcast = inner.podcast.lock().await;
        if podcast.episode_at(feed_id, episode_index).is_none() {
            let _ = podcast.refresh_feed(feed_id).await;
        }
        let lyrics = podcast
            .fetch_transcript(feed_id, episode_index)
            .await
            .map_err(CoreError::Daemon)?;
        Ok(DaemonRes::PodcastTranscriptRes {
            feed_id: feed_id.to_string(),
            episode_index,
            lyrics: Box::new(lyrics),
        })
    }

    /// Play the selected episode by enqueueing its synthetic path and calling
    /// the native streaming player.
    pub async fn play(
        inner: &DaemonInner,
        feed_id: &str,
        episode_index: usize,
    ) -> Result<DaemonRes, CoreError> {
        let track = Self::episode_track(inner, feed_id, episode_index).await?;
        let path = track.path.clone();
        {
            let mut state = inner.state.write().await;
            state.queue.push(track);
        }
        Daemon::push_queue_state(inner).await;
        Cmd::play(inner, &path, 0.0, false).await
    }

    /// Queue an episode without starting it, as `podcast://feed/index`.
    ///
    /// The queue route needs this because a `podcast://` uri is not a path:
    /// `resolve_track` has no way to turn one into a title, so the row came out
    /// blank. The same fix the `spotify:` branch of the queue route needs.
    pub async fn queue_episode(
        inner: &DaemonInner,
        feed_id: &str,
        episode_index: usize,
        position: Option<u64>,
    ) -> Result<DaemonRes, CoreError> {
        let track = Self::episode_track(inner, feed_id, episode_index).await?;
        {
            let mut state = inner.state.write().await;
            state.fallback_disabled = false;
            queue::add_resolved(&mut state, track, position);
        }
        Daemon::push_queue_state(inner).await;
        Daemon::save_state(inner);
        Ok(DaemonRes::Ok)
    }

    /// The queue row for one episode, resolved from the feed (refreshing it if
    /// the feed is not cached yet).
    ///
    /// The source is the feed's own `<enclosure>`, streamed directly by
    /// `play_remote`. No Spotify or YouTube lookup stands in front of it: an
    /// episode is not a track, and searching for one by title finds a different
    /// recording, a different cut, or nothing — and it would download the whole
    /// episode to play a URL that streams. What the earlier plan wanted from
    /// that chain is already delivered a different way, by reading
    /// `<media:content>` and keeping entries whose enclosure the parser did not
    /// recognise; this refuses the episode with a reason rather than sending a
    /// blank row to the queue.
    async fn episode_track(
        inner: &DaemonInner,
        feed_id: &str,
        episode_index: usize,
    ) -> Result<TrackInfo, CoreError> {
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
        let base = TrackInfo {
            id: 0,
            path: format!("podcast://{feed_id}/{episode_index}"),
            title: title.clone(),
            artist: feed_title.clone(),
            album: format!("Podcast \u{b7} {feed_title}"),
            duration: episode.duration_secs.unwrap_or(0) as f64,
            cover_path: None,
            favourite: false,
            ..Default::default()
        };

        if episode.url.trim().is_empty() {
            return Err(CoreError::Daemon(format!(
                "\u{201c}{title}\u{201d} has no audio in this feed",
            )));
        }
        // A direct stream is already playable; nothing to resolve.
        return Ok(base);
    }
}
