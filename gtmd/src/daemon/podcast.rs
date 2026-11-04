use super::*;

use gtm::shared::podcast::PodcastFeed;

/// Fetch and apply one feed without holding the podcast lock.
///
/// The lock is taken, cloned out what the fetch needs, released, and only
/// re-taken to persist the result. A feed fetch waits on the network; the
/// read-only commands (feeds, status, episodes) must not wait behind it or
/// the client's 30 s IPC timeout fires — every feed fetch used to run under
/// this lock, and refreshing a dozen feeds starved the UI for minutes.
pub(crate) async fn refresh_unlocked(
    inner: &DaemonInner,
    feed_id: &str,
) -> Result<PodcastFeed, CoreError> {
    let (url, client) = {
        let podcast = inner.podcast.lock().await;
        (
            podcast.feed_target(feed_id).map_err(CoreError::Daemon)?,
            podcast.client(),
        )
    };
    let parsed = crate::podcast::fetch_feed(&client, &url)
        .await
        .map_err(CoreError::Daemon)?;
    inner
        .podcast
        .lock()
        .await
        .apply_refresh(feed_id, parsed)
        .map_err(CoreError::Daemon)
}

/// Podcast feed subscriptions and episodes.
pub(crate) struct Podcast;

impl Podcast {
    pub async fn add_feed(inner: &DaemonInner, url: &str) -> Result<DaemonRes, CoreError> {
        let client = inner.podcast.lock().await.client();
        let parsed = crate::podcast::fetch_feed(&client, url)
            .await
            .map_err(CoreError::Daemon)?;
        let feed = inner
            .podcast
            .lock()
            .await
            .apply_add(url, parsed)
            .map_err(CoreError::Daemon)?;
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

    /// Search the public directory. Results carry the feed url rather than an
    /// id, so subscribing is a second call with the url the row holds.
    pub async fn discover(
        inner: &DaemonInner,
        term: &str,
        country: &str,
    ) -> Result<DaemonRes, CoreError> {
        let key = PodcastManager::search_key(term, country);
        if let Some(hit) = inner.podcast.lock().await.cached_result(&key) {
            return Ok(DaemonRes::PodcastSearchRes { results: hit });
        }
        let results = crate::podcast::discover_net(term, country)
            .await
            .map_err(CoreError::Daemon)?;
        inner.podcast.lock().await.memoize(key, &results);
        Ok(DaemonRes::PodcastSearchRes { results })
    }

    /// Apple's ranked charts, resolved to subscribable feeds.
    pub async fn charts(
        inner: &DaemonInner,
        country: &str,
        genre: &str,
    ) -> Result<DaemonRes, CoreError> {
        let key = PodcastManager::chart_key(country, genre);
        if let Some(hit) = inner.podcast.lock().await.cached_result(&key) {
            return Ok(DaemonRes::PodcastSearchRes { results: hit });
        }
        let results = crate::podcast::charts_net(country, genre)
            .await
            .map_err(CoreError::Daemon)?;
        inner.podcast.lock().await.memoize(key, &results);
        Ok(DaemonRes::PodcastSearchRes { results })
    }

    pub async fn episodes(inner: &DaemonInner, feed_id: &str) -> Result<DaemonRes, CoreError> {
        // Ensure the feed is loaded and its episodes are available.
        // If the episodes cache is empty for this feed, refresh it. The fetch
        // runs outside the lock: one slow feed used to starve the whole UI.
        if !inner.podcast.lock().await.has_episodes(feed_id)
            && let Err(e) = refresh_unlocked(inner, feed_id).await
        {
            warn!("refresh episodes for {feed_id}: {e}");
        }
        match inner.podcast.lock().await.episodes(feed_id) {
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
        match feed_id {
            Some(id) => {
                let feed = refresh_unlocked(inner, id).await?;
                Ok(DaemonRes::PodcastFeedsRes { feeds: vec![feed] })
            }
            None => {
                let ids = inner.podcast.lock().await.feed_ids();
                let mut ok = 0usize;
                for id in ids {
                    match refresh_unlocked(inner, &id).await {
                        Ok(_) => ok += 1,
                        Err(e) => warn!("refresh podcast feed {id}: {e}"),
                    }
                }
                Ok(DaemonRes::Value {
                    value: serde_json::json!({ "refreshed": ok }),
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
        if inner.podcast.lock().await.episode_at(feed_id, episode_index).is_none() {
            // Ensure the feed is loaded first; outside the lock like every
            // other refresh.
            let _ = refresh_unlocked(inner, feed_id).await;
        }
        let (episode, client) = {
            let podcast = inner.podcast.lock().await;
            let episode = podcast
                .episode_at(feed_id, episode_index)
                .ok_or_else(|| CoreError::Daemon("episode missing".into()))?;
            (episode, podcast.client())
        };
        let lyrics = crate::podcast::fetch_transcripts(&client, &episode)
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
            match inner.podcast.lock().await.episode_at(feed_id, episode_index) {
                Some(ep) => (ep.feed_title.clone(), ep),
                None => {
                    refresh_unlocked(inner, feed_id).await?;
                    let ep = inner
                        .podcast
                        .lock()
                        .await
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
        Ok(base)
    }
}
