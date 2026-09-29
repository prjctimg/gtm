use super::*;

/// A path that names a provider resource rather than a file on disk.
///
/// These arrive from the library and chart pickers, where a row's identity *is*
/// its uri, and from `gtm queue add`. Treating one as a path is what produced
/// an untitled "Spotify Track" row: `expand_paths` passes anything that is
/// neither a directory nor an existing file straight through, and
/// `resolve_track` then has a uri to work with and no way to label it.
pub(crate) fn is_provider_path(path: &str) -> bool {
    path.starts_with("spotify:")
        || path.starts_with("podcast://")
        || path.starts_with("radio://")
        || path.starts_with("youtube:")
}

pub(crate) struct Queue;

impl Queue {
    pub async fn handle(inner: &DaemonInner, action: &QueueAction) -> Result<DaemonRes, CoreError> {
        match action {
            QueueAction::List => {
                let state = inner.state.read().await;
                let (queue, cursor) = queue::visible(&state);
                drop(state);
                Ok(DaemonRes::QueueState {
                    queue: Box::new(queue),
                    cursor,
                })
            }
            QueueAction::Clear => {
                Daemon::clear_history(inner).await;
                {
                    let mut state = inner.state.write().await;
                    queue::clear(&mut state);
                }
                Daemon::push_queue_state(inner).await;
                Daemon::save_state(inner);
                Ok(DaemonRes::Ok)
            }
            QueueAction::Remove { index } => {
                {
                    let mut state = inner.state.write().await;
                    queue::remove(&mut state, *index);
                }
                Daemon::push_queue_state(inner).await;
                Daemon::save_state(inner);
                Ok(DaemonRes::Ok)
            }
            QueueAction::Move { from, to } => {
                {
                    let mut state = inner.state.write().await;
                    queue::move_track(&mut state, *from, *to);
                }
                Daemon::push_queue_state(inner).await;
                Daemon::save_state(inner);
                Ok(DaemonRes::Ok)
            }
            QueueAction::Add { paths, position } => {
                // A provider uri is not a filesystem path. `resolve_track` has
                // no way to turn `spotify:track:<id>` into a title, so the row
                // landed as the literal "Spotify Track" placeholder and nothing
                // ever repaired it — the repairing code only runs inside the
                // provider's own resolver. Route those to the resolver, which
                // is the same path Enter on the row already took.
                let remote: Vec<String> = paths
                    .iter()
                    .filter(|p| is_provider_path(p))
                    .cloned()
                    .collect();
                if !remote.is_empty() {
                    return Self::add_remote(inner, &remote, *position).await;
                }
                // Directory walk + per-file tag reads all happen on a
                // blocking thread so adding a huge folder never stalls the
                // command loop; the state write below only inserts entries.
                let base = paths.clone();
                let prepared = tokio::task::spawn_blocking(move || {
                    let expanded = queue::expand_paths(&base)?;
                    if expanded.is_empty() {
                        return Err::<Vec<TrackInfo>, String>("no audio files found".into());
                    }
                    Ok(expanded
                        .iter()
                        .map(|p| queue::resolve_track(p))
                        .collect::<Vec<_>>())
                })
                .await
                .map_err(|e| CoreError::Daemon(e.to_string()))?;
                let tracks = match prepared {
                    Ok(t) => t,
                    Err(e) => return Ok(DaemonRes::Error { message: e }),
                };
                let first_path = tracks[0].path.clone();
                let was_empty = {
                    let mut state = inner.state.write().await;
                    state.fallback_disabled = false;
                    let w = state.queue.is_empty() && state.status == PlaybackStatus::Stopped;
                    // One call, so the batch keeps its order. Adding one at a
                    // time recomputed `insert_base` per track, and that returns
                    // the "play next" slot every time — so `add a b c` produced
                    // `head c b a`.
                    queue::add_resolved_many(&mut state, tracks, *position);
                    drop(state);
                    w
                };
                if was_empty
                    && let DaemonRes::Error { message } =
                        Cmd::play(inner, &first_path, 0.0, false).await?
                {
                    // The play result used to be discarded, so a first track
                    // that could not play — Premium required, stream refused —
                    // left a queued row, silence, and no error anywhere.
                    return Ok(DaemonRes::Error { message });
                }
                Daemon::push_queue_state(inner).await;
                Daemon::save_state(inner);
                Ok(DaemonRes::Ok)
            }
            QueueAction::Set {
                paths,
                start_idx: _,
            } => {
                Daemon::clear_history(inner).await;
                let base = paths.clone();
                let tracks = tokio::task::spawn_blocking(move || {
                    base.iter()
                        .map(|p| queue::resolve_track(p))
                        .collect::<Vec<_>>()
                })
                .await
                .map_err(|e| CoreError::Daemon(e.to_string()))?;
                {
                    let mut state = inner.state.write().await;
                    queue::set_resolved(&mut state, tracks);
                }
                Daemon::push_queue_state(inner).await;
                Daemon::save_state(inner);
                Ok(DaemonRes::Ok)
            }
        }
    }

    /// Enqueue provider uris as real, titled, playable rows.
    ///
    /// A `spotify:` uri goes through the Spotify resolver, which either queues
    /// it for native streaming or falls back to a YouTube download, and writes
    /// the entry with its title, artist, album, duration and cover already
    /// filled in. A `podcast://feed/index` goes through the podcast resolver,
    /// which is the only place that knows what that feed's episode is called.
    /// Anything the resolver cannot handle is queued unresolved: a row the user
    /// can see, reorder and remove beats a row that silently refuses to play.
    async fn add_remote(
        inner: &DaemonInner,
        paths: &[String],
        position: Option<u64>,
    ) -> Result<DaemonRes, CoreError> {
        let mut queued = 0usize;
        for path in paths {
            if path.starts_with("spotify:") && Self::add_spotify(inner, path, position).await {
                queued += 1;
                continue;
            }
            if path.starts_with("podcast://") {
                match Self::split_podcast_uri(path) {
                    Some((feed, index)) => {
                        match Podcast::queue_episode(inner, &feed, index, position).await {
                            Ok(DaemonRes::Ok) => queued += 1,
                            Ok(DaemonRes::Error { message }) => {
                                return Ok(DaemonRes::Error { message });
                            }
                            Ok(_) => {}
                            Err(e) => return Err(e),
                        }
                        continue;
                    }
                    None => {
                        return Ok(DaemonRes::Error {
                            message: format!("not a podcast uri: {path}"),
                        });
                    }
                }
            }
            let mut state = inner.state.write().await;
            state.fallback_disabled = false;
            queue::add_resolved_many(&mut state, vec![queue::resolve_track(path)], position);
            queued += 1;
        }
        if queued == 0 {
            return Ok(DaemonRes::Error {
                message: format!("nothing to queue from {}", paths.join(", ")),
            });
        }
        Daemon::push_queue_state(inner).await;
        Daemon::save_state(inner);
        Ok(DaemonRes::Ok)
    }

    /// Split `podcast://<feed-id>/<episode-index>`.
    ///
    /// The feed id is an md5 hex digest, so the separator cannot collide with
    /// anything inside it, and the index is the only part that can be malformed.
    fn split_podcast_uri(path: &str) -> Option<(String, usize)> {
        let rest = path.strip_prefix("podcast://")?;
        let (feed, index) = rest.rsplit_once('/')?;
        if feed.is_empty() {
            return None;
        }
        Some((feed.to_string(), index.parse().ok()?))
    }

    /// Resolve one `spotify:` uri and enqueue the resolved row.
    ///
    /// The uri is all the queue carries, so the metadata has to come from the
    /// Web API here. Without it the row is the bare placeholder, because
    /// `resolve_track` is given a uri and no way to label it.
    async fn add_spotify(inner: &DaemonInner, uri: &str, position: Option<u64>) -> bool {
        let Ok(client) = linked(inner).await else {
            return false;
        };
        let id = uri.rsplit(':').next().unwrap_or_default();
        let Some(track) = crate::spotify::api::track(&client, id).await else {
            return false;
        };
        let meta = StreamMeta {
            title: &track.name,
            artist: &track.artists,
            album: track.album.as_deref().unwrap_or(""),
            image_url: track.image_url.as_deref(),
            duration: track.duration_ms.map(|ms| ms as f64 / 1000.0),
        };
        matches!(
            Spotify::queue_stream(inner, uri, meta, false, position).await,
            Ok(DaemonRes::Ok)
        )
    }
}
