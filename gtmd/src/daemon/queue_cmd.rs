use super::*;

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
                    for track in tracks {
                        queue::add_resolved(&mut state, track, *position);
                    }
                    drop(state);
                    w
                };
                if was_empty {
                    let _ = Cmd::play(inner, &first_path, 0.0, false).await;
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
}
