use super::*;

pub(crate) struct LibraryHandler;

impl LibraryHandler {
    pub async fn handle(
        inner: &DaemonInner,
        action: &LibraryAction,
    ) -> Result<DaemonRes, CoreError> {
        let res = match action {
            LibraryAction::Scan { path } => {
                let audio_dir = path.clone();
                let data_dir = inner.config.data_dir.clone();
                let cache_dir = inner.config.cache_dir.to_string_lossy().to_string();
                let result = tokio::task::spawn_blocking(move || {
                    let lib = Library::new(data_dir.to_str().unwrap_or(""))?;
                    lib.scan_directory(&audio_dir, true, Some(&cache_dir))
                })
                .await
                .map_err(|e| CoreError::Daemon(e.to_string()))?;
                match result {
                    Ok(tracks) => DaemonRes::Tracks {
                        tracks: Box::new(tracks),
                    },
                    Err(e) => DaemonRes::Error { message: e },
                }
            }
            LibraryAction::GetTracks { filter: _, sort: _ } => {
                let data_dir = inner.config.data_dir.clone();
                let result = tokio::task::spawn_blocking(move || {
                    let lib = Library::new(data_dir.to_str().unwrap_or(""))?;
                    lib.list_tracks()
                })
                .await
                .map_err(|e| CoreError::Daemon(e.to_string()))?;
                match result {
                    Ok(tracks) => DaemonRes::Tracks {
                        tracks: Box::new(tracks),
                    },
                    Err(e) => DaemonRes::Error { message: e },
                }
            }
            LibraryAction::GetMostPlayed { limit } => {
                let limit = *limit;
                let data_dir = inner.config.data_dir.clone();
                let result = tokio::task::spawn_blocking(move || {
                    let lib = Library::new(data_dir.to_str().unwrap_or(""))?;
                    lib.list_most_played(limit)
                })
                .await
                .map_err(|e| CoreError::Daemon(e.to_string()))?;
                match result {
                    Ok(tracks) => DaemonRes::Tracks {
                        tracks: Box::new(tracks),
                    },
                    Err(e) => DaemonRes::Error { message: e },
                }
            }
            LibraryAction::GetRecentlyPlayed { limit } => {
                let limit = *limit;
                let data_dir = inner.config.data_dir.clone();
                let result = tokio::task::spawn_blocking(move || {
                    let lib = Library::new(data_dir.to_str().unwrap_or(""))?;
                    lib.list_recently_played(limit)
                })
                .await
                .map_err(|e| CoreError::Daemon(e.to_string()))?;
                match result {
                    Ok(tracks) => DaemonRes::Tracks {
                        tracks: Box::new(tracks),
                    },
                    Err(e) => DaemonRes::Error { message: e },
                }
            }
            LibraryAction::GetRecentlyAdded { limit } => {
                let limit = *limit;
                let data_dir = inner.config.data_dir.clone();
                let result = tokio::task::spawn_blocking(move || {
                    let lib = Library::new(data_dir.to_str().unwrap_or(""))?;
                    lib.list_recently_added(limit)
                })
                .await
                .map_err(|e| CoreError::Daemon(e.to_string()))?;
                match result {
                    Ok(tracks) => DaemonRes::Tracks {
                        tracks: Box::new(tracks),
                    },
                    Err(e) => DaemonRes::Error { message: e },
                }
            }
            LibraryAction::GetPlaylists => {
                let data_dir = inner.config.data_dir.clone();
                let result = tokio::task::spawn_blocking(move || {
                    let lib = Library::new(data_dir.to_str().unwrap_or(""))?;
                    lib.get_playlists()
                })
                .await
                .map_err(|e| CoreError::Daemon(e.to_string()))?;
                match result {
                    Ok(playlists) => DaemonRes::Playlists { playlists },
                    Err(e) => DaemonRes::Error { message: e },
                }
            }
            LibraryAction::GetPlaylistTracks { id } => {
                let data_dir = inner.config.data_dir.clone();
                let pid = *id;
                let result = tokio::task::spawn_blocking(move || {
                    let lib = Library::new(data_dir.to_str().unwrap_or(""))?;
                    lib.get_playlist_tracks(pid)
                })
                .await
                .map_err(|e| CoreError::Daemon(e.to_string()))?;
                match result {
                    Ok(tracks) => DaemonRes::Tracks {
                        tracks: Box::new(tracks),
                    },
                    Err(e) => DaemonRes::Error { message: e },
                }
            }
            LibraryAction::CreatePlaylist { name } => {
                let name = name.clone();
                let data_dir = inner.config.data_dir.clone();
                let result = tokio::task::spawn_blocking(move || {
                    let lib = Library::new(data_dir.to_str().unwrap_or(""))?;
                    lib.create_playlist(&name)
                })
                .await
                .map_err(|e| CoreError::Daemon(e.to_string()))?;
                match result {
                    Ok(playlist) => {
                        let playlists = vec![playlist];
                        DaemonRes::Playlists { playlists }
                    }
                    Err(e) => DaemonRes::Error { message: e },
                }
            }
            LibraryAction::RenamePlaylist { id, name } => {
                let id = *id;
                let name = name.trim().to_string();
                let data_dir = inner.config.data_dir.clone();
                let result = tokio::task::spawn_blocking(move || {
                    let lib = Library::new(data_dir.to_str().unwrap_or(""))?;
                    lib.rename_playlist(id, &name).map(|_| ())
                })
                .await
                .map_err(|e| CoreError::Daemon(e.to_string()))?;
                match result {
                    Ok(_) => DaemonRes::Ok,
                    Err(e) => DaemonRes::Error { message: e },
                }
            }
            LibraryAction::DeletePlaylist { id } => {
                let id = *id;
                let data_dir = inner.config.data_dir.clone();
                let result = tokio::task::spawn_blocking(move || {
                    let lib = Library::new(data_dir.to_str().unwrap_or(""))?;
                    lib.delete_playlist(id)
                })
                .await
                .map_err(|e| CoreError::Daemon(e.to_string()))?;
                match result {
                    Ok(_) => DaemonRes::Ok,
                    Err(e) => DaemonRes::Error { message: e },
                }
            }
            LibraryAction::AddToPlaylist {
                playlist_id,
                track_ids,
            } => {
                let playlist_id = *playlist_id;
                let track_ids = track_ids.clone();
                let data_dir = inner.config.data_dir.clone();
                let result = tokio::task::spawn_blocking(move || {
                    let lib = Library::new(data_dir.to_str().unwrap_or(""))?;
                    for tid in &track_ids {
                        lib.add_to_playlist(playlist_id, *tid)?;
                    }
                    Ok::<_, String>(())
                })
                .await
                .map_err(|e| CoreError::Daemon(e.to_string()))?;
                match result {
                    Ok(_) => DaemonRes::Ok,
                    Err(e) => DaemonRes::Error { message: e },
                }
            }
            LibraryAction::ImportPlaylist { path, format } => {
                let path = path.clone();
                let format = *format;
                let data_dir = inner.config.data_dir.clone();
                let result = tokio::task::spawn_blocking(move || {
                    let lib = Library::new(data_dir.to_str().unwrap_or(""))?;
                    lib.import_playlist(&path, format)
                })
                .await
                .map_err(|e| CoreError::Daemon(e.to_string()))?;
                match result {
                    Ok(playlist) => {
                        let playlists = vec![playlist];
                        DaemonRes::Playlists { playlists }
                    }
                    Err(e) => DaemonRes::Error { message: e },
                }
            }
            LibraryAction::GetRecent { count } => {
                let count = *count;
                let data_dir = inner.config.data_dir.clone();
                let result = tokio::task::spawn_blocking(move || {
                    let lib = Library::new(data_dir.to_str().unwrap_or(""))?;
                    lib.get_recent(count)
                })
                .await
                .map_err(|e| CoreError::Daemon(e.to_string()))?;
                match result {
                    Ok(tracks) => DaemonRes::Tracks {
                        tracks: Box::new(tracks),
                    },
                    Err(e) => DaemonRes::Error { message: e },
                }
            }
            LibraryAction::SyncCovers => {
                LibraryHandler::sync_start(inner, SyncKind::Covers, None).await?
            }
            LibraryAction::SyncLyrics => {
                LibraryHandler::sync_start(inner, SyncKind::Lyrics, None).await?
            }
            LibraryAction::SyncMetadata { path } => {
                LibraryHandler::sync_start(inner, SyncKind::Metadata, path.clone()).await?
            }
            LibraryAction::SyncStatus => LibraryHandler::sync_status(inner).await?,
            LibraryAction::ExportPlaylist {
                playlist_id,
                path,
                format,
            } => {
                let playlist_id = *playlist_id;
                let export_path = path.clone();
                let format = *format;
                let data_dir = inner.config.data_dir.clone();
                let result = tokio::task::spawn_blocking(move || {
                    let lib = Library::new(data_dir.to_str().unwrap_or(""))?;
                    lib.export_playlist(playlist_id, &export_path, format)
                })
                .await
                .map_err(|e| CoreError::Daemon(e.to_string()))?;
                match result {
                    Ok(_) => DaemonRes::Ok,
                    Err(e) => DaemonRes::Error { message: e },
                }
            }
            LibraryAction::RemoveFromPlaylist {
                playlist_id,
                track_id,
            } => {
                let playlist_id = *playlist_id;
                let track_id = *track_id;
                let data_dir = inner.config.data_dir.clone();
                let result = tokio::task::spawn_blocking(move || {
                    let lib = Library::new(data_dir.to_str().unwrap_or(""))?;
                    lib.remove_from_playlist(playlist_id, track_id)
                })
                .await
                .map_err(|e| CoreError::Daemon(e.to_string()))?;
                match result {
                    Ok(()) => DaemonRes::Ok,
                    Err(e) => DaemonRes::Error { message: e },
                }
            }
            LibraryAction::PlaylistDedup { playlist_id } => {
                let playlist_id = *playlist_id;
                let data_dir = inner.config.data_dir.clone();
                let result = tokio::task::spawn_blocking(move || {
                    let lib = Library::new(data_dir.to_str().unwrap_or(""))?;
                    lib.playlist_dedup(playlist_id)
                })
                .await
                .map_err(|e| CoreError::Daemon(e.to_string()))?;
                match result {
                    Ok(removed) => DaemonRes::Value {
                        value: serde_json::json!({ "removed": removed }),
                    },
                    Err(e) => DaemonRes::Error { message: e },
                }
            }
            LibraryAction::PlaylistDoctor { playlist_id } => {
                let playlist_id = *playlist_id;
                let data_dir = inner.config.data_dir.clone();
                let result = tokio::task::spawn_blocking(move || {
                    let lib = Library::new(data_dir.to_str().unwrap_or(""))?;
                    lib.playlist_doctor(playlist_id)
                })
                .await
                .map_err(|e| CoreError::Daemon(e.to_string()))?;
                match result {
                    Ok(removed) => DaemonRes::Value {
                        value: serde_json::json!({ "removed": removed }),
                    },
                    Err(e) => DaemonRes::Error { message: e },
                }
            }
            LibraryAction::PlaylistSort { playlist_id, field } => {
                let playlist_id = *playlist_id;
                let field = field.clone();
                let data_dir = inner.config.data_dir.clone();
                let result = tokio::task::spawn_blocking(move || {
                    let lib = Library::new(data_dir.to_str().unwrap_or(""))?;
                    lib.playlist_sort(playlist_id, &field)
                })
                .await
                .map_err(|e| CoreError::Daemon(e.to_string()))?;
                match result {
                    Ok(()) => DaemonRes::Ok,
                    Err(e) => DaemonRes::Error { message: e },
                }
            }
            LibraryAction::RemoveTrack { id } => {
                let id = *id;
                let data_dir = inner.config.data_dir.clone();
                let library_dirs = inner.config.library_paths.clone();
                let allow_delete_files = inner.config.allow_delete_files;
                let result = tokio::task::spawn_blocking(move || {
                    let lib = Library::new(data_dir.to_str().unwrap_or(""))?;
                    lib.remove_track_full(id, &library_dirs, allow_delete_files)
                })
                .await
                .map_err(|e| CoreError::Daemon(e.to_string()))?;
                match result {
                    Ok(Some(removed_path)) => {
                        let mut state = inner.state.write().await;
                        state.queue.retain(|t| t.path != removed_path);
                        let was_current = state
                            .current_track
                            .as_ref()
                            .is_some_and(|t| t.path == removed_path);
                        drop(state);
                        if was_current {
                            Daemon::stop_playback(inner).await;
                        } else {
                            Daemon::push_queue_state(inner).await;
                        }
                        Daemon::push_event(
                            inner,
                            DaemonEvent::Custom {
                                name: "library_changed".into(),
                                data: Default::default(),
                            },
                        );
                        DaemonRes::Ok
                    }
                    Ok(None) => DaemonRes::Error {
                        message: "track not found".to_string(),
                    },
                    Err(e) => DaemonRes::Error { message: e },
                }
            }
            LibraryAction::UpdateMetadata { track_id, patch } => {
                let track_id = *track_id;
                let patch = patch.clone();
                let data_dir = inner.config.data_dir.clone();
                let result = tokio::task::spawn_blocking(move || {
                    let lib = Library::new(data_dir.to_str().unwrap_or(""))?;
                    lib.update_metadata(track_id, &patch)?;
                    // Re-read the row so any queue/current tracker holding the
                    // edited path can be refreshed (Cmd::play reuses queue
                    // metadata instead of re-resolving it from disk).
                    let path = lib.track_path(track_id)?;
                    Ok::<_, String>((path.clone(), lib.track_by_path(&path)?))
                })
                .await
                .map_err(|e| CoreError::Daemon(e.to_string()))?;
                match result {
                    Ok((path, Some(refreshed))) => {
                        let mut state = inner.state.write().await;
                        for t in state.queue.iter_mut() {
                            if t.path == path {
                                *t = refreshed.clone();
                            }
                        }
                        if let Some(cur) = state.current_track.as_mut()
                            && cur.path == path
                        {
                            *cur = refreshed.clone();
                        }
                        drop(state);
                        DaemonRes::Ok
                    }
                    Ok((_, None)) => DaemonRes::Ok,
                    Err(e) => DaemonRes::Error { message: e },
                }
            }
        };
        Ok(res)
    }

    pub async fn sync_start(
        inner: &DaemonInner,
        kind: SyncKind,
        only_path: Option<String>,
    ) -> Result<DaemonRes, CoreError> {
        if inner.sync_progress.running.load(Ordering::Acquire) {
            return Ok(DaemonRes::Error {
                message: "a library sync is already running".into(),
            });
        }
        *inner.sync_progress.kind.lock().unwrap() = kind;
        inner.sync_progress.synced.store(0, Ordering::Relaxed);
        inner.sync_progress.total.store(0, Ordering::Relaxed);
        inner.sync_progress.running.store(true, Ordering::Release);

        let data_dir = inner.config.data_dir.clone();
        let cache_dir = inner.config.cache_dir.clone();
        let covers_provider = inner.effective_cover_provider().await;
        let lyrics_manager = inner.lyrics_manager().await;
        let progress = inner.sync_progress.clone();
        let event_tx = inner.event_tx.clone();
        tokio::spawn(async move {
            let progress_inner = progress.clone();
            let result = tokio::task::spawn_blocking(move || match kind {
                SyncKind::Covers => {
                    run_covers_sync(data_dir, cache_dir, covers_provider, &progress_inner)
                }
                SyncKind::Lyrics => run_lyrics_sync(data_dir, lyrics_manager, &progress_inner),
                SyncKind::Metadata => {
                    run_metadata_sync(data_dir, cache_dir, only_path, &progress_inner)
                }
            })
            .await;
            let (synced, total, error) = match result {
                Ok(Ok((s, t))) => (s, t, None),
                Ok(Err(e)) => (0, 0, Some(e)),
                Err(e) => (0, 0, Some(e.to_string())),
            };
            progress.synced.store(synced, Ordering::Relaxed);
            progress.total.store(total, Ordering::Relaxed);
            progress.running.store(false, Ordering::Release);
            let mut data = std::collections::HashMap::new();
            data.insert("kind".to_string(), format!("{kind:?}").to_lowercase());
            data.insert("synced".to_string(), synced.to_string());
            data.insert("total".to_string(), total.to_string());
            if let Some(e) = error {
                data.insert("error".to_string(), e);
            }
            let _ = event_tx.send(DaemonEvent::Custom {
                name: "sync_done".into(),
                data,
            });
        });
        Ok(DaemonRes::Ok)
    }

    pub async fn sync_status(inner: &DaemonInner) -> Result<DaemonRes, CoreError> {
        let progress = &inner.sync_progress;
        Ok(DaemonRes::SyncStatus {
            running: progress.running.load(Ordering::Acquire),
            kind: *progress.kind.lock().unwrap(),
            synced: progress.synced.load(Ordering::Relaxed),
            total: progress.total.load(Ordering::Relaxed),
        })
    }
}
