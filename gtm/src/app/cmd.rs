use crate::app::*;

impl App {
    pub(crate) fn handle_command(&mut self, cmd: TuiCommand) {
        // All IPC calls are spawned as background tasks to avoid blocking the UI loop.
        let client = self.client.clone();
        let ipc_tx = self.ipc_tx.clone();
        let err_tx = ipc_tx.clone();
        let error_handler = move |e: CoreError| {
            let _ = err_tx.send(IpcResult::Error(e.to_string()));
        };
        let err_tx2 = ipc_tx.clone();
        let error_handler2 = move |e: CoreError| {
            let _ = err_tx2.send(IpcResult::Error(e.to_string()));
        };

        match cmd {
            TuiCommand::Play(path) => {
                tokio::spawn(async move {
                    if let Err(e) = client.play(&path, 0.0).await {
                        error_handler(e);
                    }
                });
            }
            TuiCommand::PlayPause => {
                tokio::spawn(async move {
                    if let Err(e) = client.play_pause().await {
                        error_handler(e);
                    }
                });
            }
            TuiCommand::Pause => {
                tokio::spawn(async move {
                    if let Err(e) = client.pause().await {
                        error_handler(e);
                    }
                });
            }
            TuiCommand::Stop => {
                tokio::spawn(async move {
                    if let Err(e) = client.stop().await {
                        error_handler(e);
                    }
                });
            }
            TuiCommand::Next => {
                self.manual_track_advance = true;
                tokio::spawn(async move {
                    if let Err(e) = client.next().await {
                        error_handler(e);
                    }
                });
            }
            TuiCommand::Prev => {
                self.manual_track_advance = true;
                tokio::spawn(async move {
                    if let Err(e) = client.prev().await {
                        error_handler(e);
                    }
                });
            }
            TuiCommand::Seek(pos) => {
                // Mirror the seek into the local position estimate immediately so
                // the lyric highlight follows the new position right away (not a
                // full daemon round-trip later), and clear the monotonic guard so a
                // backward seek isn't clamped. `estimated_position` also honours
                // this seek target while it is fresh.
                self.seek_pending = Some(std::time::Instant::now());
                if self.state.current_track.is_some() {
                    self.raw_position = pos;
                    self.display_position = pos;
                }
                tokio::spawn(async move {
                    if let Err(e) = client.seek(pos).await {
                        error_handler(e);
                    }
                });
            }
            TuiCommand::SetVolume(v) => {
                tokio::spawn(async move {
                    if let Err(e) = client.set_volume(v).await {
                        error_handler(e);
                    }
                });
            }
            TuiCommand::SetSpeed(r) => {
                tokio::spawn(async move {
                    if let Err(e) = client.set_speed(r).await {
                        error_handler(e);
                    }
                });
            }
            TuiCommand::SetLowPower(enabled) => {
                tokio::spawn(async move {
                    if let Err(e) = client.set_low_power(enabled).await {
                        error_handler(e);
                    }
                });
            }
            TuiCommand::ToggleShuffle => {
                tokio::spawn(async move {
                    if let Err(e) = client.toggle_shuffle().await {
                        error_handler(e);
                    }
                });
            }
            TuiCommand::CycleRepeat(m) => {
                tokio::spawn(async move {
                    if let Err(e) = client.cycle_repeat(m).await {
                        error_handler(e);
                    }
                });
            }
            TuiCommand::ToggleMute => {
                tokio::spawn(async move {
                    if let Err(e) = client.toggle_mute().await {
                        error_handler(e);
                    }
                });
            }
            TuiCommand::ToggleMono => {
                tokio::spawn(async move {
                    if let Err(e) = client.toggle_mono().await {
                        error_handler(e);
                    }
                });
            }
            TuiCommand::Crossfade(en, dur) => {
                tokio::spawn(async move {
                    if let Err(e) = client.crossfade(en, dur).await {
                        error_handler(e);
                    }
                });
            }
            TuiCommand::QueueAdd(p) => {
                tokio::spawn(async move {
                    if let Err(e) = client.queue().add(&p, None).await {
                        error_handler2(e);
                    }
                });
            }
            TuiCommand::QueueMove(from, to) => {
                tokio::spawn(async move {
                    if let Err(e) = client.queue().reorder(from, to).await {
                        error_handler(e);
                    }
                });
            }
            TuiCommand::QueueClear => {
                tokio::spawn(async move {
                    if let Err(e) = client.queue().clear().await {
                        error_handler(e);
                    }
                });
            }
            TuiCommand::YtSearch(q) => {
                self.yt_search_loading = true;
                self.yt_results_cache.clear();
                let c = client.clone();
                tokio::spawn(async move {
                    if let Err(e) = c.yt().search(&q, None).await {
                        error_handler2(e);
                    }
                });
            }
            TuiCommand::YtDownload { url, title, artist } => {
                // "Download started" renders inline on the YT list row; the
                // finished event is the only download toast.
                self.downloading_urls.insert(url.clone());
                let ipc = ipc_tx.clone();
                let client2 = self.client.clone();
                tokio::spawn(async move {
                    let msg = async {
                        // Kick off the daemon-side yt-dlp download (fresh PO
                        // token + signature extraction on every run).
                        let _ = match client2
                            .yt()
                            .download(url.clone(), title.clone(), artist.clone())
                            .await
                        {
                            Ok(id) => id,
                            Err(e) => return format!("Download error: {e}"),
                        };
                        let deadline = std::time::Instant::now() + Duration::from_secs(600);
                        let file_path = loop {
                            if std::time::Instant::now() > deadline {
                                let _ = client2.yt().cancel_download(url.clone()).await;
                                return "Download timed out".to_string();
                            }
                            let res = match client2.yt().download_poll().await {
                                Ok(res) => res,
                                Err(e) => return format!("Download error: {e}"),
                            };
                            match &res {
                                DaemonRes::YtDownloadProgress {
                                    id,
                                    url,
                                    title,
                                    progress,
                                    status,
                                    error,
                                    file_path: fp,
                                    downloaded_bytes,
                                    total_bytes,
                                    rate_bps,
                                    eta_secs,
                                } => {
                                    // Mirror live progress to the TUI footer so
                                    // the user sees the download moving.
                                    let _ = ipc.send(IpcResult::YtDownloadProgress {
                                        id: *id,
                                        url: url.clone(),
                                        title: title.clone(),
                                        progress: *progress,
                                        status: status.clone(),
                                        file_path: fp.clone(),
                                        downloaded_bytes: *downloaded_bytes,
                                        total_bytes: *total_bytes,
                                        rate_bps: *rate_bps,
                                        eta_secs: *eta_secs,
                                    });
                                    if let Some(fp) = fp.clone().filter(|_| status == "completed") {
                                        break fp.clone();
                                    }
                                    if status == "failed" || status == "cancelled" {
                                        return error
                                            .clone()
                                            .unwrap_or_else(|| format!("Download {status}"));
                                    }
                                }
                                DaemonRes::Error { message } => {
                                    return format!("Download failed: {message}");
                                }
                                DaemonRes::Value { .. } => {}
                                _ => {
                                    return "Download error: unexpected daemon response"
                                        .to_string();
                                }
                            }
                            tokio::time::sleep(Duration::from_millis(250)).await;
                        };
                        let audio_dir = std::path::PathBuf::from(&file_path)
                            .parent()
                            .map(|p| p.to_path_buf())
                            .unwrap_or_else(|| std::path::PathBuf::from("."));

                        let _ = client2
                            .library()
                            .scan(audio_dir.to_string_lossy().as_ref())
                            .await;
                        // Also refresh the track cache
                        if let Ok(DaemonRes::Tracks { tracks, .. }) =
                            client2.library().get_tracks(None, None).await
                        {
                            let _ = ipc.send(IpcResult::LibraryTracks(*tracks));
                        }
                        // Try to fetch lyrics for the newly downloaded track
                        if let Ok(DaemonRes::Tracks { tracks, .. }) =
                            client2.library().get_tracks(None, None).await
                            && let Some(track) = tracks
                                .iter()
                                .find(|t| t.path.contains(&file_path))
                                .or_else(|| tracks.last())
                        {
                            if let Ok(Some(lyrics_data)) =
                                client2.lyrics().get(track.id, Some(&track.path)).await
                                && !lyrics_data.lines.is_empty()
                            {
                                // Write .lrc sidecar next to the audio file
                                let lrc_path = {
                                    let p = std::path::PathBuf::from(&track.path);
                                    p.with_extension("lrc")
                                };
                                let mut lrc_content = String::new();
                                if let Some(ref ar) = lyrics_data.artist {
                                    lrc_content.push_str(&format!("[ar:{}]\n", ar));
                                }
                                if let Some(ref al) = lyrics_data.album {
                                    lrc_content.push_str(&format!("[al:{}]\n", al));
                                }
                                if let Some(ref ti) = lyrics_data.title {
                                    lrc_content.push_str(&format!("[ti:{}]\n", ti));
                                }
                                for line in &lyrics_data.lines {
                                    if line.timestamp < 0.0 {
                                        lrc_content.push_str(&line.text);
                                        lrc_content.push('\n');
                                        continue;
                                    }
                                    let mins = (line.timestamp / 60.0) as u64;
                                    let secs = line.timestamp - (mins as f64 * 60.0);
                                    lrc_content.push_str(&format!(
                                        "[{:02}:{:05.2}]{}\n",
                                        mins, secs, line.text
                                    ));
                                }
                                let _ = std::fs::write(&lrc_path, lrc_content);
                            }
                            // Scrub the downloaded file's tags and fetch
                            // its cover art (runs per-path in the
                            // background).
                            let _ = client2
                                .library()
                                .sync_metadata(Some(track.path.clone()))
                                .await;
                        }
                        match (&title, &artist) {
                            (Some(t), Some(a)) => format!("Downloaded: {} - {}", a, t),
                            (Some(t), _) => format!("Downloaded: {}", t),
                            _ => {
                                let name = std::path::Path::new(&file_path)
                                    .file_name()
                                    .map(|n| n.to_string_lossy().into_owned())
                                    .unwrap_or_else(|| file_path.clone());
                                format!("Downloaded: {}", name)
                            }
                        }
                    }
                    .await;
                    let kind = if msg.starts_with("Downloaded") {
                        NotificationKind::Success
                    } else {
                        NotificationKind::Error
                    };
                    let _ = ipc.send(IpcResult::Notification(
                        "YouTube".to_string(),
                        msg,
                        kind,
                        NotifType::Downloads,
                    ));
                });
            }
            TuiCommand::YtResolve(u) => {
                tokio::spawn(async move {
                    if let Err(e) = client.yt().resolve_stream(&u).await {
                        error_handler2(e);
                    }
                });
            }
            TuiCommand::SetEqPreset(preset) => {
                tokio::spawn(async move {
                    if let Err(e) = client.set_eq_preset(preset).await {
                        error_handler(e);
                    }
                });
            }
            TuiCommand::Search(q) => {
                tokio::spawn(async move {
                    if let Err(e) = client.search(&q).await {
                        error_handler2(e);
                    }
                });
            }
            TuiCommand::AddFavourite(id) => {
                tokio::spawn(async move {
                    if let Err(e) = client.favourites().add(id).await {
                        error_handler(e);
                    }
                });
            }
            TuiCommand::RemoveFavourite(id) => {
                tokio::spawn(async move {
                    if let Err(e) = client.favourites().remove(id).await {
                        error_handler(e);
                    }
                });
            }
            TuiCommand::Refresh => {
                // Skip the redundant queue list (below) when the local cache
                // already has entries: it was only there to recover from an
                // early background spawn failure.
                let needs_queue = self.queue.cache.is_empty();
                let client2 = self.client.clone();
                let ipc_tx2 = self.ipc_tx.clone();
                tokio::spawn(async move {
                    if let Ok(state) = client2.get_status_lite().await {
                        // Cover art is fetched on track-change events, not
                        // here, to avoid an extra IPC call every second.
                        let _ = ipc_tx2.send(IpcResult::RefreshDone(Box::new(state), None, None));
                    }
                    if needs_queue
                        && let Ok(DaemonRes::QueueState {
                            queue: tracks,
                            cursor,
                            ..
                        }) = client2.queue().list().await
                    {
                        let _ = ipc_tx2.send(IpcResult::Queue(*tracks, cursor as usize));
                    }
                });
            }
            TuiCommand::RefreshLibrary => {
                tokio::spawn(async move {
                    if let Ok(DaemonRes::Tracks { tracks, .. }) =
                        client.library().get_tracks(None, None).await
                    {
                        let _ = ipc_tx.send(IpcResult::LibraryTracks(*tracks));
                    }
                });
            }
            TuiCommand::RefreshYt => {
                tokio::spawn(async move {
                    if let Ok(DaemonRes::YtSearchResults { query, results }) =
                        client.yt().poll().await
                    {
                        let _ = ipc_tx.send(IpcResult::YtResults(query, results));
                    }
                });
            }
            TuiCommand::RemoveTrack(track_id) => {
                tokio::spawn(async move {
                    if let Err(e) = client.library().remove_track(track_id).await {
                        error_handler(e);
                    } else {
                        let _ = ipc_tx.send(IpcResult::Notification(
                            "Library".to_string(),
                            "Track deleted".to_string(),
                            NotificationKind::Success,
                            NotifType::Library,
                        ));
                        if let Ok(DaemonRes::Tracks { tracks, .. }) =
                            client.library().get_tracks(None, None).await
                        {
                            let _ = ipc_tx.send(IpcResult::LibraryTracks(*tracks));
                        }
                    }
                });
            }
            TuiCommand::RemoveFromPlaylist(playlist_id, track_id) => {
                tokio::spawn(async move {
                    if let Err(e) = client
                        .library()
                        .remove_from_playlist(playlist_id, track_id)
                        .await
                    {
                        error_handler(e);
                    } else {
                        let _ = ipc_tx.send(IpcResult::Notification(
                            "Playlist".to_string(),
                            "Removed from playlist".to_string(),
                            NotificationKind::Success,
                            NotifType::NowPlaying,
                        ));
                        if let Ok(DaemonRes::Playlists { playlists, .. }) =
                            client.library().get_playlists().await
                        {
                            let _ = ipc_tx.send(IpcResult::Playlists(playlists));
                        }
                    }
                });
            }
            TuiCommand::FetchLyrics => {
                let track_path = self.state.current_track.as_ref().map(|t| t.path.clone());
                let track_id = self.state.current_track.as_ref().map(|t| t.id).unwrap_or(0);
                let fetch_gen = self.next_lyrics_gen();
                self.lyrics.pending_gen = Some(fetch_gen);
                let client2 = self.client.clone();
                let ipc_tx2 = self.ipc_tx.clone();
                tokio::spawn(async move {
                    let result = tokio::time::timeout(
                        Duration::from_secs(12),
                        client2.lyrics().get(track_id, track_path.as_deref()),
                    )
                    .await;
                    match result {
                        Ok(Ok(lyrics)) => {
                            let _ = ipc_tx2.send(IpcResult::Lyrics(lyrics, fetch_gen));
                        }
                        Ok(Err(e)) => {
                            let _ = ipc_tx2.send(IpcResult::Lyrics(None, fetch_gen));
                            let _ = ipc_tx2.send(IpcResult::Error(format!("Lyrics: {e}")));
                        }
                        Err(_) => {
                            let _ = ipc_tx2.send(IpcResult::Lyrics(None, fetch_gen));
                            let _ = ipc_tx2
                                .send(IpcResult::Error("Lyrics fetch timed out".to_string()));
                        }
                    }
                });
            }
            TuiCommand::SetSleepTimer(minutes, stop_immediately) => {
                let client = self.client.clone();
                let ipc_tx = self.ipc_tx.clone();
                tokio::spawn(async move {
                    if let Err(e) = client.set_sleep_timer(minutes, stop_immediately).await {
                        let _ = ipc_tx.send(IpcResult::Error(e.to_string()));
                    }
                });
            }
            TuiCommand::CancelSleepTimer => {
                let client = self.client.clone();
                let ipc_tx = self.ipc_tx.clone();
                tokio::spawn(async move {
                    if let Err(e) = client.cancel_sleep_timer().await {
                        let _ = ipc_tx.send(IpcResult::Error(e.to_string()));
                    }
                });
            }
            TuiCommand::CheckHealth => {
                self.report_health = true;
                let client = self.client.clone();
                let ipc_tx = self.ipc_tx.clone();
                tokio::spawn(async move {
                    match client.check_health().await {
                        Ok(report) => {
                            let _ = ipc_tx.send(IpcResult::HealthReport(report));
                        }
                        Err(e) => {
                            let _ = ipc_tx.send(IpcResult::Error(e.to_string()));
                        }
                    }
                });
            }
            TuiCommand::Fire(f) => {
                tokio::spawn(f());
            }
        };
    }
}
