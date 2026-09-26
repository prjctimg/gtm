use crate::app::*;

/// Map a picker overlay to the extension that owns it, if any. Used to gate
/// keybindings/palette entries whose surface was moved out of core.
pub(crate) fn overlay_extension(id: PickerId) -> Option<ExtensionId> {
    Some(match id {
        PickerId::VisualizerPreset => ExtensionId::Visualizer,
        PickerId::Notifications => ExtensionId::FloatingNotifications,
        PickerId::NotificationSettings => ExtensionId::NotificationOverlay,
        _ => return None,
    })
}

pub(crate) fn build_keybindings(
    overrides: &std::collections::HashMap<String, String>,
) -> Keybindings {
    let mut defaults = default_keybindings();

    if overrides.is_empty() {
        return defaults;
    }

    // Parse user overrides into (KeyEvent, action_name, contexts) triples.
    let mut user_bindings: Vec<(crossterm::event::KeyEvent, String, Vec<KeyContext>)> = Vec::new();
    for (key_str, action_str) in overrides {
        let key = match parse_key_event(key_str) {
            Some(k) => k,
            None => {
                eprintln!("gtm: unknown key \"{}\" in config keybindings", key_str);
                continue;
            }
        };
        let action = match KeyboardAction::from_name(action_str) {
            Some(a) => a,
            None => {
                eprintln!(
                    "gtm: unknown action \"{}\" in config keybindings",
                    action_str
                );
                continue;
            }
        };
        // Bind in Normal + List context by default for most actions.
        let contexts = vec![KeyContext::Normal, KeyContext::List];
        user_bindings.push((key, action_str.clone(), contexts));
        defaults.bindings.push((
            key,
            BoundCommand {
                action,
                contexts: vec![KeyContext::Normal, KeyContext::List],
            },
        ));
    }

    let warnings = detect_clashes(&user_bindings);
    for w in &warnings {
        eprintln!("gtm: keybinding clash: {}", w);
    }

    defaults
}

impl App {
    pub(crate) async fn handle_key(&mut self, key: event::KeyEvent) -> bool {
        let tx = self.cmd_tx();
        // Ctrl+Z: suspend to background (pass-through SIGTSTP)
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('z') {
            self.pending_suspend = true;
            return false;
        }
        // Reset pending_motion if the key is not 'g'
        if key.code != KeyCode::Char('g') {
            self.pending_motion = None;
        }
        // If an picker is open, Esc closes it; keys pass through to picker
        if self.pickers.is_open() {
            return match key.code {
                KeyCode::Esc => {
                    if self
                        .pickers
                        .top()
                        .is_some_and(|o| o.id == PickerId::SpotifyLink)
                        && self.spotify.oauth_pending
                    {
                        // Cancel the pending OAuth browser flow.
                        self.spotify.oauth_pending = false;
                        self.spotify.oauth_url = None;
                        self.spotify.oauth_error = None;
                        let c = self.client.clone();
                        let _ = tx.send(TuiCommand::fire(move || async move {
                            let _ = c.spotify().oauth_cancel().await;
                        }));
                        self.close_picker();
                    } else if self
                        .pickers
                        .top()
                        .is_some_and(|o| o.id == PickerId::SpotifySearch)
                        && self.spotify.oauth_pending
                        && self.spotify.status.as_ref().is_none_or(|s| !s.linked)
                    {
                        // The Alt+s picker was mid-OAuth: cancel the browser
                        // flow and close the picker.
                        self.spotify.oauth_pending = false;
                        self.spotify.oauth_url = None;
                        self.spotify.oauth_error = None;
                        let c = self.client.clone();
                        let _ = tx.send(TuiCommand::fire(move || async move {
                            let _ = c.spotify().oauth_cancel().await;
                        }));
                        self.close_picker();
                    } else if self
                        .pickers
                        .top()
                        .is_some_and(|o| o.id == PickerId::PlaylistSelect)
                        && self.playlist_creating
                    {
                        self.playlist_creating = false;
                        if let Some(top) = self.pickers.top_mut() {
                            top.query.clear();
                            top.selected = 0;
                            top.viewport_offset = 0;
                        }
                    } else if self.pickers.top().is_some_and(|o| o.id == PickerId::Radio)
                        && self.radio.section != RadioSection::Root
                    {
                        // Esc pops a radio drill-down (Stations / Results)
                        // back to the merged root view; a second Esc closes.
                        self.radio.section = RadioSection::Root;
                        if let Some(top) = self.pickers.top_mut() {
                            top.query.clear();
                            top.selected = 0;
                            top.viewport_offset = 0;
                        }
                    } else {
                        self.close_picker();
                    }
                    true
                }
                _ => {
                    // Pass key to picker handler
                    self.handle_picker_key(key).await;
                    true
                }
            };
        }

        match self.input_mode {
            InputMode::Searching => match key.code {
                KeyCode::Esc => {
                    self.input_mode = InputMode::Normal;
                    self.search_query.clear();
                }
                KeyCode::Enter => {
                    // Commit the filter without clearing it and play
                    // the highlighted row in the filtered list.
                    self.play_filtered_highlighted();
                }
                KeyCode::Char(c) => {
                    self.search_query.push(c);
                }
                KeyCode::Backspace => {
                    self.search_query.pop();
                }
                _ => {}
            },
            InputMode::Normal => {
                // If a prompt is pending, intercept keys
                if let Some(prompt) = self.pending_prompt.take() {
                    let is_confirm = prompt.confirm_keys.contains(&key.code);
                    let is_cancel = prompt.cancel_keys.contains(&key.code);
                    if is_confirm {
                        match prompt.prompt_type {
                            PromptType::DeleteTrack(track_id) => {
                                let tx = self.cmd_tx();
                                let _ = tx.send(TuiCommand::RemoveTrack(track_id)).await;
                            }
                            PromptType::DeletePlaylist(playlist_id) => {
                                let client = self.client.clone();
                                let ipc_tx = self.ipc_tx.clone();
                                let _ = tx.send(TuiCommand::fire(move || async move {
                                    match client.library().delete_playlist(playlist_id).await {
                                        Ok(()) => {
                                            if let Ok(DaemonRes::Playlists { playlists, .. }) =
                                                client.library().get_playlists().await
                                            {
                                                let _ =
                                                    ipc_tx.send(IpcResult::Playlists(playlists));
                                            }
                                            let _ = ipc_tx.send(IpcResult::Notification(
                                                "Playlist".to_string(),
                                                "Deleted playlist".to_string(),
                                                NotificationKind::Success,
                                                NotifType::NowPlaying,
                                            ));
                                        }
                                        Err(e) => {
                                            let _ = ipc_tx.send(IpcResult::Error(format!(
                                                "Failed to delete playlist: {e}"
                                            )));
                                        }
                                    }
                                }));
                            }
                            PromptType::MultiselectAddToQueue => {
                                // Resolve the selection against the current
                                // pane so a shifted list can't queue the
                                // wrong tracks (or none).
                                let targets = self.selected_play_targets();
                                let mut added = 0;
                                for target in targets {
                                    let c = self.client.clone();
                                    let _ = tx.send(TuiCommand::fire(move || async move {
                                        let _ = c.queue().add(&target, None).await;
                                    }));
                                    added += 1;
                                }
                                self.clear_selection();
                                self.multiselect_mode = false;
                                self.fetch_queue().await;
                                self.footer_notification = Some((
                                    format!("Added {added} track(s) to queue"),
                                    std::time::Instant::now() + std::time::Duration::from_secs(2),
                                ));
                            }
                            PromptType::MultiselectAddToPlaylist => {
                                let indices = self.selected_library_ids();
                                self.clear_selection();
                                self.multiselect_mode = false;
                                if !indices.is_empty() {
                                    self.pending_track_ids = indices;
                                    self.playlist_creating = false;
                                    self.pickers.open(PickerId::PlaylistSelect);
                                }
                            }
                            PromptType::MultiselectDelete(ids) => {
                                let client = self.client.clone();
                                let ipc_tx = self.ipc_tx.clone();
                                let mut deleted = 0;
                                for id in ids {
                                    if client.library().remove_track(id).await.is_ok() {
                                        deleted += 1;
                                    }
                                }
                                if deleted > 0
                                    && let Ok(DaemonRes::Tracks { tracks, .. }) =
                                        client.library().get_tracks(None, None).await
                                {
                                    let _ = ipc_tx.send(IpcResult::LibraryTracks(*tracks));
                                }
                                self.clear_selection();
                                self.multiselect_mode = false;
                                let msg = if deleted == 1 {
                                    "Deleted 1 track".to_string()
                                } else {
                                    format!("Deleted {deleted} track(s)")
                                };
                                self.footer_notification = Some((
                                    msg,
                                    std::time::Instant::now() + std::time::Duration::from_secs(2),
                                ));
                            }
                            PromptType::RemoveCustomRadio(name) => {
                                match crate::shared::custom::remove_custom_station(&name) {
                                    Ok(removed) => {
                                        self.refresh_custom_stations();
                                        self.notify_typed(
                                            "System",
                                            format!(
                                                "Removed \"{}\" from custom stations",
                                                removed.name
                                            ),
                                            NotificationKind::Success,
                                            false,
                                            NotifType::NowPlaying,
                                        );
                                    }
                                    Err(e) => {
                                        let _ = self.ipc_tx.send(IpcResult::Error(format!(
                                            "Failed to remove custom station: {e}"
                                        )));
                                    }
                                }
                            }
                            PromptType::None => {}
                        }
                    } else if is_cancel {
                        // Just drop the prompt
                    } else {
                        // Put the prompt back if key wasn't recognized
                        self.pending_prompt = Some(prompt);
                    }
                    return true;
                }
                // Health panel: Esc closes it
                if self.show_health_panel && key.code == KeyCode::Esc {
                    self.show_health_panel = false;
                    return true;
                }
                // Zen mode: only a small set of keys acts — Tab/Shift-Tab
                // cycle the surface, l toggles lyrics, Space toggles
                // playback, and z/Esc/q leave. Everything else is swallowed
                // so the fullscreen surfaces are never disturbed.
                if self.zen {
                    self.zen_key(key);
                    return true;
                }
                // Handle gg (vim-style double-press) for jump to start
                if key.code == KeyCode::Char('g') && !self.library_pane_focus {
                    if self.pending_motion == Some('g') {
                        // Second 'g': execute jump to start
                        self.pending_motion = None;
                        self.set_list_pos(0);

                        return true;
                    } else {
                        // First 'g': wait for second press
                        self.pending_motion = Some('g');
                        return true;
                    }
                }
                // In multiselect mode, Tab toggles selection and advances
                if key.code == KeyCode::Tab && self.multiselect_mode && !self.library_pane_focus {
                    let pos = self.list_pos();
                    self.toggle_row(pos);
                    // Advance by one row in the *current* pane list (track
                    // lists, playlist tracks, chart tracks alike).
                    let max = self.library_list_len().saturating_sub(1);
                    self.set_list_pos((pos + 1).min(max));
                    let count = self.selected_count();
                    self.notify_typed(
                        "System",
                        format!("{count} selected"),
                        NotificationKind::Info,
                        true,
                        NotifType::Prefs,
                    );
                    return true;
                }
                match self.keybindings.dispatch(key, KeyContext::Normal) {
                    Some(KeyboardAction::Quit) => {
                        if self.browse_detail.is_some() {
                            self.browse_detail = None;
                            self.set_list_pos(0);
                            if self.library_category == 5 {
                                self.spotify.playlist_tracks_cache.clear();
                            }
                        } else {
                            return false;
                        }
                    }
                    Some(KeyboardAction::QuitDaemon) => {
                        let c = self.client.clone();
                        // Await the daemon's reply so the quit request is
                        // actually delivered before the TUI exits. The daemon
                        // replies Ok then shuts down ~200ms later.
                        let _ = tokio::time::timeout(Duration::from_millis(1500), c.quit()).await;
                        return false;
                    }
                    Some(KeyboardAction::NextPane) => {
                        self.cycle_pane_focus(true);
                        self.dismiss_track_popup();
                    }
                    Some(KeyboardAction::PrevPane) => {
                        self.cycle_pane_focus(false);
                        self.dismiss_track_popup();
                    }
                    Some(KeyboardAction::OpenOverlay(id)) => {
                        if let Some(ext) = overlay_extension(id)
                            && self.extensions.is_disabled(ext)
                        {
                            self.notify(
                                format!("{} is an optional extension (disabled)", ext.label()),
                                NotificationKind::Info,
                            );
                            return true;
                        }
                        self.pickers.open(id);
                        self.dismiss_track_popup();
                        self.on_picker_opened(id);
                    }
                    Some(KeyboardAction::Search) => {
                        let id = self.search_picker();
                        self.pickers.open(id);
                        self.dismiss_track_popup();
                        self.on_picker_opened(id);
                    }
                    Some(KeyboardAction::ToggleHelp) => {
                        if self.pickers.top().is_some_and(|o| o.id == PickerId::Help) {
                            self.pickers.close_top();
                        } else {
                            self.pickers.open(PickerId::Help);
                            self.dismiss_track_popup();
                        }
                    }
                    Some(KeyboardAction::HideHelpBar) => {
                        self.hide_help_bar = !self.hide_help_bar;
                    }
                    Some(KeyboardAction::PlayPause) => {
                        self.set_last_action("Play/Pause");
                        match self.state.status {
                            PlaybackStatus::Playing => {
                                self.send_high(TuiCommand::Pause);
                            }
                            PlaybackStatus::Paused => {
                                self.send_high(TuiCommand::PlayPause);
                            }
                            PlaybackStatus::Stopped => {
                                if !self.queue.cache.is_empty() {
                                    let idx = self.queue.cursor.min(self.queue.cache.len() - 1);
                                    let path = self.queue.cache[idx].path.clone();
                                    self.send_high(TuiCommand::Play(path));
                                }
                            }
                        }
                    }
                    Some(KeyboardAction::Next) => {
                        self.set_last_action("Next");
                        if self.multiselect_mode && self.selected_count() > 0 {
                            let count = self.selected_count();
                            self.pending_prompt = Some(PendingPrompt {
                                message: format!("Queue {count} selected track(s)? [y/N]"),
                                confirm_keys: vec![
                                    KeyCode::Char('y'),
                                    KeyCode::Char('Y'),
                                    KeyCode::Enter,
                                ],
                                cancel_keys: vec![
                                    KeyCode::Char('n'),
                                    KeyCode::Char('N'),
                                    KeyCode::Esc,
                                    KeyCode::Char('q'),
                                ],
                                prompt_type: PromptType::MultiselectAddToQueue,
                            });
                        } else {
                            self.send_high(TuiCommand::Next);
                        }
                    }
                    Some(KeyboardAction::Prev) => {
                        self.set_last_action("Previous");
                        if self.multiselect_mode && self.selected_count() > 0 {
                            let count = self.selected_count();
                            self.pending_prompt = Some(PendingPrompt {
                                message: format!("Queue {count} selected track(s)? [y/N]"),
                                confirm_keys: vec![
                                    KeyCode::Char('y'),
                                    KeyCode::Char('Y'),
                                    KeyCode::Enter,
                                ],
                                cancel_keys: vec![
                                    KeyCode::Char('n'),
                                    KeyCode::Char('N'),
                                    KeyCode::Esc,
                                    KeyCode::Char('q'),
                                ],
                                prompt_type: PromptType::MultiselectAddToQueue,
                            });
                        } else {
                            self.send_high(TuiCommand::Prev);
                        }
                    }
                    Some(KeyboardAction::Stop) => {
                        self.set_last_action("Stop");
                        self.send_high(TuiCommand::Stop);
                    }
                    Some(KeyboardAction::VolumeUp) => {
                        let new_vol = (self.state.volume + 5).min(MAX_VOLUME);
                        self.send_high(TuiCommand::SetVolume(new_vol));
                        self.notify_volume(new_vol);
                    }
                    Some(KeyboardAction::VolumeDown) => {
                        self.set_last_action("Volume Down");
                        let new_vol = self.state.volume.saturating_sub(5);
                        self.send_high(TuiCommand::SetVolume(new_vol));
                        self.notify_volume(new_vol);
                    }
                    Some(KeyboardAction::SpeedUp) => {
                        // Round to nearest 0.25 so the step stays predictable.
                        let new_speed = ((self.state.audio.speed + 0.25) * 4.0).ceil() / 4.0;
                        let new_speed = new_speed.min(MAX_SPEED);
                        self.set_last_action(&format!("Speed {:.2}x", new_speed));
                        self.send_high(TuiCommand::SetSpeed(new_speed));
                    }
                    Some(KeyboardAction::SpeedDown) => {
                        self.set_last_action("Speed Down");
                        let new_speed = ((self.state.audio.speed - 0.25) * 4.0).ceil() / 4.0;
                        let new_speed = new_speed.max(MIN_SPEED);
                        self.send_high(TuiCommand::SetSpeed(new_speed));
                    }
                    Some(KeyboardAction::ToggleLowPower) => {
                        self.set_last_action("Toggle Low-Power");
                        self.send_high(TuiCommand::SetLowPower(!self.state.low_power));
                        let msg = if self.state.low_power {
                            "Leaving low-power mode"
                        } else {
                            "Low-power mode (playback paused)"
                        };
                        self.footer_notification = Some((
                            msg.to_string(),
                            std::time::Instant::now() + std::time::Duration::from_secs(2),
                        ));
                    }
                    Some(KeyboardAction::ToggleZen) => {
                        self.set_last_action(if self.zen {
                            "Leave Zen Mode"
                        } else {
                            "Zen Mode"
                        });
                        self.zen = !self.zen;
                        if self.zen {
                            // One surface at a time: start on the lyrics
                            // surface when lyrics were already open,
                            // otherwise the enlarged cover + progress.
                            self.zen_surface = if self.lyrics.show {
                                ZenSurface::Lyrics
                            } else {
                                ZenSurface::Cover
                            };
                            self.dismiss_track_popup();
                        }
                    }
                    Some(KeyboardAction::SeekForward) => {
                        self.set_last_action("Seek Forward");
                        self.accumulate_seek(5.0);
                    }
                    Some(KeyboardAction::SeekBackward) => {
                        self.set_last_action("SeekBackward");
                        self.accumulate_seek(-5.0);
                    }
                    Some(KeyboardAction::ToggleMute) => {
                        self.set_last_action("Toggle Mute");
                        self.send_high(TuiCommand::ToggleMute);
                        let msg = if self.state.mute { "Unmuted" } else { "Muted" };
                        self.footer_notification = Some((
                            msg.to_string(),
                            std::time::Instant::now() + std::time::Duration::from_secs(2),
                        ));
                    }
                    Some(KeyboardAction::ToggleMono) => {
                        self.set_last_action("Toggle Mono");
                        self.send_high(TuiCommand::ToggleMono);
                        let msg = if self.state.mono {
                            "Mono off"
                        } else {
                            "Mono on"
                        };
                        self.footer_notification = Some((
                            msg.to_string(),
                            std::time::Instant::now() + std::time::Duration::from_secs(2),
                        ));
                    }
                    Some(KeyboardAction::ToggleLove) => {
                        self.set_last_action(
                            if self.setup.lastfm_status.as_ref().is_some_and(|s| s.loved) {
                                "Un-love on Last.fm"
                            } else {
                                "Love on Last.fm"
                            },
                        );
                        return self.manage_lastfm_love();
                    }
                    Some(KeyboardAction::ToggleScrobble) => {
                        self.set_last_action("Toggle Last.fm scrobbling");
                        self.toggle_scrobble_session();
                    }
                    Some(KeyboardAction::CycleRepeat) => {
                        self.set_last_action("Cycle Repeat");
                        let new_mode = match self.state.repeat {
                            RepeatMode::Off => RepeatMode::One,
                            RepeatMode::One => RepeatMode::All,
                            RepeatMode::All => RepeatMode::Off,
                        };
                        self.send_high(TuiCommand::CycleRepeat(new_mode));
                        self.footer_notification = Some((
                            format!("Repeat: {:?}", new_mode),
                            std::time::Instant::now() + std::time::Duration::from_secs(2),
                        ));
                    }
                    Some(KeyboardAction::ToggleShuffle) => {
                        // In a Spotify playlist drill-down, `S` (shift-s) means
                        // "shuffle play this playlist" instead of toggling the
                        // global queue shuffle.
                        if self.in_spotify_playlist() && !self.library_pane_focus {
                            let playlist_id = self.browse_detail.clone().unwrap_or_default();
                            let c = self.client.clone();
                            let ipc_tx2 = self.ipc_tx.clone();
                            self.footer_notification = Some((
                                "Shuffling playlist…".to_string(),
                                std::time::Instant::now() + std::time::Duration::from_secs(2),
                            ));
                            let _ = tx.send(TuiCommand::fire(move || async move {
                                match c.spotify().play_all(&playlist_id, true).await {
                                    Ok(()) => {}
                                    Err(e) => {
                                        let _ = ipc_tx2.send(IpcResult::Error(format!(
                                            "Spotify shuffle failed: {e}"
                                        )));
                                    }
                                }
                            }));
                            return true;
                        }
                        self.set_last_action("Toggle Shuffle");
                        self.send_high(TuiCommand::ToggleShuffle);
                        let msg = if self.state.shuffle {
                            "Shuffle OFF"
                        } else {
                            "Shuffle ON"
                        };
                        self.footer_notification = Some((
                            msg.to_string(),
                            std::time::Instant::now() + std::time::Duration::from_secs(2),
                        ));
                    }
                    Some(KeyboardAction::ToggleFavourite) => {
                        self.set_last_action("Toggle Favourite");
                        if self.library_pane_focus {
                            return true;
                        }
                        let (ids, label) = match self.library_category {
                            2 => {
                                // Album row: all tracks in the album
                                if let Some((name, _)) = self.unique_albums().get(self.list_pos()) {
                                    let ids: Vec<i64> = self
                                        .tracks_cache
                                        .iter()
                                        .filter(|t| {
                                            let album: &str = if t.album.is_empty() {
                                                "Unknown Album"
                                            } else {
                                                &t.album
                                            };
                                            album == name
                                        })
                                        .map(|t| t.id)
                                        .collect();
                                    (ids, name.clone())
                                } else {
                                    (Vec::new(), String::new())
                                }
                            }
                            3 => {
                                // Artist row: all tracks by the artist
                                if let Some((name, _)) = self.unique_artists().get(self.list_pos())
                                {
                                    let ids: Vec<i64> = self
                                        .tracks_cache
                                        .iter()
                                        .filter(|t| {
                                            let artist: &str = if t.artist.is_empty() {
                                                "Unknown Artist"
                                            } else {
                                                &t.artist
                                            };
                                            artist == name
                                        })
                                        .map(|t| t.id)
                                        .collect();
                                    (ids, name.clone())
                                } else {
                                    (Vec::new(), String::new())
                                }
                            }
                            4 => {
                                // Playlist row (drill-down open): all tracks in the playlist
                                if self.browse_detail.is_some() {
                                    let ids: Vec<i64> =
                                        self.filtered_tracks().iter().map(|t| t.id).collect();
                                    (ids, "Playlist".to_string())
                                } else {
                                    (Vec::new(), String::new())
                                }
                            }
                            _ => {
                                if self.library_category == 12 {
                                    // Chart tracks are streamed — they have no
                                    // library id to favourite.
                                    self.notify_typed(
                                        "System",
                                        "Chart tracks are streamed \u{2014} not in your library",
                                        NotificationKind::Info,
                                        false,
                                        NotifType::NowPlaying,
                                    );
                                    return true;
                                }
                                // Track row (flat list / detail / Liked): toggle the
                                // highlighted track, or the whole selection in
                                // multiselect mode (excluding the highlighted row).
                                let filtered = self.filtered_tracks();
                                if self.multiselect_mode && self.selected_count() > 0 {
                                    let pos = self.list_pos();
                                    let cursor_id =
                                        self.selectable_rows().get(pos).and_then(|r| r.2);
                                    let ids: Vec<i64> = self
                                        .selected_library_ids()
                                        .into_iter()
                                        .filter(|&id| Some(id) != cursor_id)
                                        .collect();
                                    if ids.is_empty() {
                                        (Vec::new(), String::new())
                                    } else {
                                        let count = ids.len();
                                        (ids, format!("{count} tracks"))
                                    }
                                } else if let Some(t) = filtered.get(self.list_pos()) {
                                    (vec![t.id], t.title.clone())
                                } else {
                                    (Vec::new(), String::new())
                                }
                            }
                        };
                        if ids.is_empty() {
                            return true;
                        }
                        let all_fav = self
                            .tracks_cache
                            .iter()
                            .filter(|t| ids.contains(&t.id))
                            .all(|t| t.favourite);
                        let new_fav = !all_fav;
                        for t in &mut self.tracks_cache {
                            if ids.contains(&t.id) {
                                t.favourite = new_fav;
                            }
                        }
                        let tx = self.cmd_tx();
                        for id in ids {
                            let _ = tx
                                .send(if new_fav {
                                    TuiCommand::AddFavourite(id)
                                } else {
                                    TuiCommand::RemoveFavourite(id)
                                })
                                .await;
                        }
                        if !label.is_empty() {
                            let verb = if new_fav { "added to" } else { "removed from" };
                            self.notify_silent(
                                "System",
                                format!("{label}: {verb} favourites"),
                                NotificationKind::Info,
                            );
                        }
                    }
                    Some(KeyboardAction::ClearQueue) => {
                        self.set_last_action("Clear Queue");
                        let tx = self.cmd_tx();
                        let _ = tx.send(TuiCommand::QueueClear).await;
                        self.footer_notification = Some((
                            "Queue cleared".to_string(),
                            std::time::Instant::now() + std::time::Duration::from_secs(2),
                        ));
                    }
                    Some(KeyboardAction::ToggleVisualizer) => {
                        if self.extensions.is_disabled(ExtensionId::Visualizer) {
                            self.footer_notification = Some((
                                format!(
                                    "{} disabled — enable in [extensions]",
                                    ExtensionId::Visualizer.label()
                                ),
                                std::time::Instant::now() + std::time::Duration::from_secs(2),
                            ));
                        } else {
                            self.visualizer.toggle();
                            let state = if self.visualizer.is_enabled() {
                                "ON"
                            } else {
                                "OFF"
                            };
                            self.footer_notification = Some((
                                format!("Visualizer: {}", state),
                                std::time::Instant::now() + std::time::Duration::from_secs(2),
                            ));
                        }
                    }
                    Some(KeyboardAction::ToggleTheme) => {
                        self.toggle_theme();
                    }
                    Some(KeyboardAction::CycleSort) => {
                        self.cycle_track_sort();
                    }
                    Some(KeyboardAction::CheckHealth) => {
                        self.send_high(TuiCommand::CheckHealth);
                    }
                    Some(KeyboardAction::FocusLeft) => {
                        if self.lyrics.show && self.lyrics.pane_focus {
                            // `[` trims lyrics highlights earlier while the
                            // lyrics pane holds focus (exit focus with Back/Tab).
                            self.nudge_lyrics_offset(-0.1);
                        } else if self.lyrics.show {
                            // `[` with lyrics open but not focused returns focus
                            // to the library pane.
                            self.library_pane_focus = true;
                        } else {
                            self.library_pane_focus = true;
                        }
                    }
                    Some(KeyboardAction::FocusRight) => {
                        if self.lyrics.show && self.lyrics.pane_focus {
                            // `]` delays lyrics highlights later.
                            self.nudge_lyrics_offset(0.1);
                        } else if self.lyrics.show {
                            if self.library_pane_focus {
                                // left → right (track) pane
                                self.library_pane_focus = false;
                                self.update_track_popup();
                            } else {
                                // right pane → lyrics pane
                                self.lyrics.pane_focus = true;
                            }
                        } else {
                            self.library_pane_focus = false;
                            self.update_track_popup();
                        }
                    }
                    Some(KeyboardAction::Back) => {
                        if self.lyrics.pane_focus {
                            // Exit lyrics focus back to the track pane.
                            self.lyrics.pane_focus = false;
                            self.lyrics.manual_scroll = false;
                        } else {
                            let is_narrow = self.terminal_cols < 60;
                            if is_narrow && self.lyrics.show {
                                self.lyrics.show = false;
                            } else if self.browse_detail.is_some() {
                                self.browse_detail = None;
                                self.set_list_pos(0);
                                if self.library_category == 5 {
                                    self.spotify.playlist_tracks_cache.clear();
                                }
                            } else if !self.library_pane_focus {
                                self.library_pane_focus = true;
                            }
                        }
                    }
                    Some(KeyboardAction::FetchLyrics) => {
                        self.lyrics.show = !self.lyrics.show;
                        if !self.lyrics.show {
                            self.lyrics.pane_focus = false;
                            self.lyrics.manual_scroll = false;
                        }
                        if self.lyrics.show
                            && self.lyrics.current.is_none()
                            && !self.lyrics.fetching
                        {
                            self.lyrics.fetching = true;
                            self.send_high(TuiCommand::FetchLyrics);
                        }
                        self.dismiss_track_popup();
                    }
                    Some(KeyboardAction::EnterFilter) => {
                        self.input_mode = InputMode::Searching;
                        self.dismiss_track_popup();
                    }
                    Some(KeyboardAction::MoveUp) => {
                        if self.lyrics.pane_focus && self.lyrics.show {
                            self.lyrics.manual_scroll = true;
                            self.lyrics.scroll = self.lyrics.scroll.saturating_sub(1);
                        } else if self.library_pane_focus {
                            let visible = self.visible_library_indices();
                            let pos = visible
                                .iter()
                                .position(|&i| i == self.library_category)
                                .unwrap_or(0);
                            let new_cat = visible[pos.saturating_sub(1).min(visible.len() - 1)];
                            if new_cat != self.library_category {
                                self.reset_library_view(new_cat, None);
                            }
                        } else {
                            self.set_list_pos(self.list_pos().saturating_sub(1));
                            self.update_track_popup();
                        }
                    }
                    Some(KeyboardAction::MoveDown) => {
                        if self.lyrics.pane_focus && self.lyrics.show {
                            self.lyrics.manual_scroll = true;
                            let max = self
                                .lyrics
                                .current
                                .as_ref()
                                .map(|l| l.lines.len().saturating_sub(1))
                                .unwrap_or(0);
                            self.lyrics.scroll = (self.lyrics.scroll + 1).min(max);
                        } else if self.library_pane_focus {
                            let visible = self.visible_library_indices();
                            let pos = visible
                                .iter()
                                .position(|&i| i == self.library_category)
                                .unwrap_or(0);
                            let new_cat = visible[(pos + 1).min(visible.len() - 1)];
                            if new_cat != self.library_category {
                                self.reset_library_view(new_cat, None);
                            }
                        } else {
                            let max_list = self.library_list_len().saturating_sub(1);
                            self.set_list_pos((self.list_pos() + 1).min(max_list));
                            self.update_track_popup();
                            self.preload_upcoming_covers();
                        }
                    }
                    Some(KeyboardAction::PageUp) => {
                        if self.lyrics.pane_focus && self.lyrics.show {
                            self.lyrics.manual_scroll = true;
                            let page = self.viewport_items.max(1);
                            self.lyrics.scroll = self.lyrics.scroll.saturating_sub(page);
                        } else if !self.library_pane_focus {
                            let page = self.viewport_items.max(1);
                            self.set_list_pos(self.list_pos().saturating_sub(page));
                            self.update_track_popup();
                        }
                    }
                    Some(KeyboardAction::PageDown) => {
                        if self.lyrics.pane_focus && self.lyrics.show {
                            self.lyrics.manual_scroll = true;
                            let page = self.viewport_items.max(1);
                            let max = self
                                .lyrics
                                .current
                                .as_ref()
                                .map(|l| l.lines.len().saturating_sub(1))
                                .unwrap_or(0);
                            self.lyrics.scroll = (self.lyrics.scroll + page).min(max);
                        } else if !self.library_pane_focus {
                            let page = self.viewport_items.max(1);
                            let max_list = self.library_list_len().saturating_sub(1);
                            self.set_list_pos((self.list_pos() + page).min(max_list));
                            self.update_track_popup();
                            self.preload_upcoming_covers();
                        }
                    }
                    Some(KeyboardAction::MultiselectUp) => {
                        if self.multiselect_mode && !self.library_pane_focus {
                            let pos = self.list_pos().saturating_sub(1);
                            self.set_list_pos(pos);
                            self.update_track_popup();
                            self.add_row_selection(pos);
                        }
                    }
                    Some(KeyboardAction::MultiselectDown) => {
                        if self.multiselect_mode && !self.library_pane_focus {
                            let max_list = self.library_list_len().saturating_sub(1);
                            let pos = (self.list_pos() + 1).min(max_list);
                            self.set_list_pos(pos);
                            self.update_track_popup();
                            self.preload_upcoming_covers();
                            self.add_row_selection(pos);
                        }
                    }
                    Some(KeyboardAction::Top) => {
                        if self.lyrics.pane_focus && self.lyrics.show {
                            self.lyrics.manual_scroll = true;
                            self.lyrics.scroll = 0;
                        } else if !self.library_pane_focus {
                            self.set_list_pos(0);
                        }
                    }
                    Some(KeyboardAction::Bottom) => {
                        if self.lyrics.pane_focus && self.lyrics.show {
                            self.lyrics.manual_scroll = true;
                            let max = self
                                .lyrics
                                .current
                                .as_ref()
                                .map(|l| l.lines.len().saturating_sub(1))
                                .unwrap_or(0);
                            self.lyrics.scroll = max;
                        } else if !self.library_pane_focus {
                            let max_list = self.library_list_len().saturating_sub(1);
                            self.set_list_pos(max_list);
                        }
                    }
                    Some(KeyboardAction::Select) => {
                        {
                            if self.library_pane_focus {
                                self.library_pane_focus = false;
                            } else if self.browse_detail.is_some() {
                                // In detail view: play the selected track of
                                // the rendered right-pane list.
                                if self.library_category == 5 {
                                    // Spotify playlist drill-down: rows 0/1 are
                                    // virtual actions (Play All / Shuffle), rows
                                    // 2+ resolve their track to a playable stream.
                                    let pos = self.list_pos();
                                    if pos < Self::SPOTIFY_PLAYLIST_ROWS {
                                        let shuffle = pos == 1;
                                        let playlist_id =
                                            self.browse_detail.clone().unwrap_or_default();
                                        let c = self.client.clone();
                                        let ipc_tx2 = self.ipc_tx.clone();
                                        let _ = tx.send(TuiCommand::fire(move || async move {
                                            match c.spotify().play_all(&playlist_id, shuffle).await
                                            {
                                                Ok(()) => {}
                                                Err(e) => {
                                                    let _ = ipc_tx2.send(IpcResult::Error(
                                                        format!("Spotify play-all failed: {e}"),
                                                    ));
                                                }
                                            }
                                        }));
                                        return true;
                                    }
                                    if let Some(track) = self.selected_spotify_track().cloned() {
                                        // Resolve the track to a playable local
                                        // stream, enqueue it, and start playback
                                        // (Enter = play intent; the daemon stops
                                        // the current source before switching).
                                        let playlist_id =
                                            self.browse_detail.clone().unwrap_or_default();
                                        let track_index = track.index;
                                        let c = self.client.clone();
                                        let ipc_tx2 = self.ipc_tx.clone();
                                        let _ = tx.send(TuiCommand::fire(move || async move {
                                            match c
                                                .spotify()
                                                .resolve(&playlist_id, track_index, true)
                                                .await
                                            {
                                                Ok(()) => {}
                                                Err(e) => {
                                                    let _ = ipc_tx2.send(IpcResult::Error(
                                                        format!("Spotify resolve failed: {e}"),
                                                    ));
                                                }
                                            }
                                        }));
                                    }
                                } else {
                                    // Local categories (All/Liked/Album/Artist/
                                    // Playlist): the rendered rows are exactly
                                    // filtered_tracks(), so play that row.
                                    self.play_filtered_highlighted();
                                }
                            } else if self.library_category == 2 {
                                // Albums: select album → show its tracks
                                let albums = self.unique_albums();
                                let pos = self.list_pos();
                                if pos < albums.len() {
                                    self.browse_detail = Some(albums[pos].0.clone());
                                    self.set_list_pos(0);
                                }
                            } else if self.library_category == 3 {
                                // Artists: select artist → show its tracks
                                let artists = self.unique_artists();
                                let pos = self.list_pos();
                                if pos < artists.len() {
                                    self.browse_detail = Some(artists[pos].0.clone());
                                    self.set_list_pos(0);
                                }
                            } else if self.library_category == 4 {
                                // Playlists: select playlist → show its tracks
                                if self.list_pos() < self.playlist_cache.len() {
                                    let playlist = self.playlist_cache[self.list_pos()].clone();
                                    self.browse_detail = Some(playlist.name.clone());
                                    self.set_list_pos(0);
                                    self.playlist_tracks_cache.clear();
                                    let c = self.client.clone();
                                    let ipc_tx2 = self.ipc_tx.clone();
                                    let pid = playlist.id;
                                    let _ = tx.send(TuiCommand::fire(move || async move {
                                        if let Ok(DaemonRes::Tracks { tracks }) =
                                            c.library().get_playlist_tracks(pid).await
                                        {
                                            let _ =
                                                ipc_tx2.send(IpcResult::PlaylistTracks(*tracks));
                                        }
                                    }));
                                }
                            } else if self.library_category == 5 {
                                // Spotify: select playlist → show its cached tracks
                                if self.list_pos() < self.spotify.playlists.len() {
                                    let playlist = self.spotify.playlists[self.list_pos()].clone();
                                    self.browse_detail = Some(playlist.id.clone());
                                    self.set_list_pos(0);
                                    self.spotify.playlist_tracks_cache.clear();
                                    let c = self.client.clone();
                                    let ipc_tx2 = self.ipc_tx.clone();
                                    let pid = playlist.id;
                                    let _ = tx.send(TuiCommand::fire(move || async move {
                                        match c.spotify().playlist_tracks(&pid).await {
                                            Ok(tracks) => {
                                                let _ =
                                                    ipc_tx2.send(IpcResult::SpotifyTracks(tracks));
                                            }
                                            Err(e) => {
                                                let _ = ipc_tx2.send(IpcResult::Error(format!(
                                                    "Spotify playlist load failed: {e}"
                                                )));
                                            }
                                        }
                                    }));
                                }
                            } else if self.library_category == 6 {
                                // Radio: select custom station → play it
                                let pos = self.list_pos();
                                if let Some(station) = self.radio.custom.get(pos).cloned() {
                                    let id = match station.uuid.as_deref() {
                                        Some(uuid) => uuid.to_string(),
                                        None => format!("custom:{}", pos + 1),
                                    };
                                    let c = self.client.clone();
                                    let _ = tx.send(TuiCommand::fire(move || async move {
                                        let _ = c.radio().play(&id, &station.name).await;
                                    }));
                                }
                            } else if self.library_category == 10 {
                                // Genres: select genre → show its tracks
                                let genres = self.unique_genres();
                                let pos = self.list_pos();
                                if pos < genres.len() {
                                    self.browse_detail = Some(genres[pos].0.clone());
                                    self.set_list_pos(0);
                                }
                            } else if self.library_category == 11 {
                                // Folders: select folder → show its tracks
                                let folders = self.unique_folders();
                                let pos = self.list_pos();
                                if pos < folders.len() {
                                    self.browse_detail = Some(folders[pos].0.clone());
                                    self.set_list_pos(0);
                                }
                            } else if self.library_category == 12 {
                                // Top Charts: three-level navigation
                                if self.charts.selected_source.is_none() {
                                    // Level 0: Select source → fetch charts
                                    let pos = self.list_pos();
                                    if pos < self.charts.sources.len() {
                                        self.charts.selected_source = Some(pos);
                                        self.charts.selected_chart = None;
                                        self.charts.charts.clear();
                                        self.charts.chart_tracks.clear();
                                        self.set_list_pos(0);
                                        let source_id = self.charts.sources[pos].id.clone();
                                        let c = self.client.clone();
                                        let ipc_tx2 = self.ipc_tx.clone();
                                        let _ = tx.send(TuiCommand::fire(move || async move {
                                            if let Ok(charts) =
                                                c.charts().list(Some(source_id)).await
                                            {
                                                let _ =
                                                    ipc_tx2.send(IpcResult::ChartsLoaded(charts));
                                            }
                                        }));
                                    }
                                } else if self.charts.selected_chart.is_none() {
                                    // Level 1: Select chart → fetch tracks
                                    let pos = self.list_pos();
                                    if pos < self.charts.charts.len() {
                                        self.charts.selected_chart = Some(pos);
                                        self.charts.chart_tracks.clear();
                                        self.set_list_pos(0);
                                        let source_id = self
                                            .charts
                                            .sources
                                            .get(self.charts.selected_source.unwrap_or(0))
                                            .map(|s| s.id.clone())
                                            .unwrap_or_default();
                                        let chart_id = self.charts.charts[pos].id.clone();
                                        let c = self.client.clone();
                                        let ipc_tx2 = self.ipc_tx.clone();
                                        let _ = tx.send(TuiCommand::fire(move || async move {
                                            if let Ok(tracks) =
                                                c.charts().tracks(source_id, chart_id).await
                                            {
                                                let _ = ipc_tx2
                                                    .send(IpcResult::ChartTracksLoaded(tracks));
                                            }
                                        }));
                                    }
                                } else {
                                    // Level 2: Play selected track
                                    let pos = self.list_pos();
                                    if let Some(track) = self.charts.chart_tracks.get(pos).cloned()
                                    {
                                        let c = self.client.clone();
                                        let uri = track.uri.clone();
                                        let _ = tx.send(TuiCommand::fire(move || async move {
                                            let _ = c.queue().add(&uri, None).await;
                                        }));
                                    }
                                }
                            } else if self.library_category <= 1 || self.library_category >= 7 {
                                // Default: play track from a flat list
                                // (All Tracks / Liked / Most Played /
                                // Recently Played / Recently Added).
                                self.play_filtered_highlighted();
                            }
                        }
                    }
                    Some(KeyboardAction::Delete) => {
                        if !self.library_pane_focus {
                            // Playlist overview rows let the user delete the whole
                            // playlist; rows inside a playlist delete the track.
                            if self.library_category == 4 && self.browse_detail.is_none() {
                                if let Some(pl) = self.playlist_cache.get(self.list_pos()).cloned()
                                {
                                    self.pending_prompt = Some(PendingPrompt {
                                        message: format!("Delete playlist \"{}\"? [y/N]", pl.name),
                                        confirm_keys: vec![
                                            KeyCode::Char('y'),
                                            KeyCode::Char('Y'),
                                            KeyCode::Enter,
                                        ],
                                        cancel_keys: vec![
                                            KeyCode::Char('n'),
                                            KeyCode::Char('N'),
                                            KeyCode::Esc,
                                            KeyCode::Char('q'),
                                        ],
                                        prompt_type: PromptType::DeletePlaylist(pl.id),
                                    });
                                } else {
                                    self.notify_typed(
                                        "System",
                                        "No playlist selected",
                                        NotificationKind::Info,
                                        false,
                                        NotifType::NowPlaying,
                                    );
                                }
                            } else if let Some(ids) = self.motion_row_ids() {
                                // Album / artist view: batch-delete every track
                                // that belongs to the selected row.
                                let count = ids.len();
                                if count > 0 {
                                    let label = if self.library_category == 2 {
                                        "album"
                                    } else {
                                        "artist"
                                    };
                                    self.pending_prompt = Some(PendingPrompt {
                                        message: format!(
                                            "Delete {count} track(s) in this {label}? \
                                             [y/N]"
                                        ),
                                        confirm_keys: vec![
                                            KeyCode::Char('y'),
                                            KeyCode::Char('Y'),
                                            KeyCode::Enter,
                                        ],
                                        cancel_keys: vec![
                                            KeyCode::Char('n'),
                                            KeyCode::Char('N'),
                                            KeyCode::Esc,
                                            KeyCode::Char('q'),
                                        ],
                                        prompt_type: PromptType::MultiselectDelete(ids),
                                    });
                                }
                            } else if self.library_category == 12 {
                                // Chart tracks are streamed — nothing in the
                                // library to delete.
                                self.notify_typed(
                                    "System",
                                    "Charts are streamed \u{2014} not in your library",
                                    NotificationKind::Info,
                                    false,
                                    NotifType::NowPlaying,
                                );
                            } else {
                                let tracks = self.filtered_tracks();
                                let pos = self.list_pos();
                                if self.multiselect_mode && self.selected_count() > 0 {
                                    // Batch delete of the selection (minus the
                                    // highlighted row, as before), re-resolved
                                    // by stable key so a shifted list can't
                                    // delete the wrong tracks.
                                    let cursor_id =
                                        self.selectable_rows().get(pos).and_then(|r| r.2);
                                    let ids: Vec<i64> = self
                                        .selected_library_ids()
                                        .into_iter()
                                        .filter(|&id| Some(id) != cursor_id)
                                        .collect();
                                    if !ids.is_empty() {
                                        self.pending_prompt = Some(PendingPrompt {
                                            message: format!(
                                                "Delete {} track(s)? [y/N]",
                                                ids.len()
                                            ),
                                            confirm_keys: vec![
                                                KeyCode::Char('y'),
                                                KeyCode::Char('Y'),
                                                KeyCode::Enter,
                                            ],
                                            cancel_keys: vec![
                                                KeyCode::Char('n'),
                                                KeyCode::Char('N'),
                                                KeyCode::Esc,
                                                KeyCode::Char('q'),
                                            ],
                                            prompt_type: PromptType::MultiselectDelete(ids),
                                        });
                                    }
                                } else {
                                    let track_data =
                                        tracks.get(pos).map(|t| (t.id, t.title.clone()));
                                    if let Some((track_id, track_name)) = track_data {
                                        self.pending_prompt = Some(PendingPrompt {
                                            message: format!("Delete \"{track_name}\"? [y/N]"),
                                            confirm_keys: vec![
                                                KeyCode::Char('y'),
                                                KeyCode::Char('Y'),
                                                KeyCode::Enter,
                                            ],
                                            cancel_keys: vec![
                                                KeyCode::Char('n'),
                                                KeyCode::Char('N'),
                                                KeyCode::Esc,
                                                KeyCode::Char('q'),
                                            ],
                                            prompt_type: PromptType::DeleteTrack(track_id),
                                        });
                                    }
                                }
                            }
                        }
                    }
                    Some(KeyboardAction::ToggleMultiselect) => {
                        if !self.library_pane_focus {
                            // Charts Level 0/1 list sources/charts, not tracks —
                            // there is nothing to select until a chart opens.
                            if self.library_category == 12 && self.charts.selected_chart.is_none() {
                                self.notify_typed(
                                    "System",
                                    "Select a chart first to multiselect its tracks",
                                    NotificationKind::Info,
                                    false,
                                    NotifType::NowPlaying,
                                );
                                return true;
                            }
                            self.multiselect_mode = !self.multiselect_mode;
                            if !self.multiselect_mode {
                                self.clear_selection();
                            }
                            let msg = if self.multiselect_mode {
                                "Multiselect ON"
                            } else {
                                "Multiselect OFF"
                            };
                            self.footer_notification = Some((
                                msg.to_string(),
                                std::time::Instant::now() + std::time::Duration::from_secs(2),
                            ));
                        }
                    }
                    Some(KeyboardAction::AddToQueue) => {
                        if !self.library_pane_focus {
                            // Playlist overview rows have no tracks of their
                            // own: open the playlist first.
                            if self.library_category == 4 && self.browse_detail.is_none() {
                                self.notify_typed(
                                    "System",
                                    Self::NEED_PLAYLIST_FOR_ADD,
                                    NotificationKind::Info,
                                    false,
                                    NotifType::NowPlaying,
                                );
                                return true;
                            }
                            if let Some(ids) = self.motion_row_ids() {
                                // Album/artist row: queue every cached track in
                                // the album/artist.
                                let mut added = 0;
                                for t in &self.tracks_cache {
                                    if ids.contains(&t.id) {
                                        let c = self.client.clone();
                                        let path = t.path.clone();
                                        let _ = tx.send(TuiCommand::fire(move || async move {
                                            let _ = c.queue().add(&path, None).await;
                                        }));
                                        added += 1;
                                    }
                                }
                                self.fetch_queue().await;
                                self.footer_notification = Some((
                                    format!("Added {added} track(s) to queue"),
                                    std::time::Instant::now() + std::time::Duration::from_secs(2),
                                ));
                            } else {
                                let count = self.selected_count();
                                if self.multiselect_mode && count > 0 {
                                    self.pending_prompt = Some(PendingPrompt {
                                        message: format!("Add {count} tracks to queue? [y/N]"),
                                        confirm_keys: vec![
                                            KeyCode::Char('y'),
                                            KeyCode::Char('Y'),
                                            KeyCode::Enter,
                                        ],
                                        cancel_keys: vec![
                                            KeyCode::Char('n'),
                                            KeyCode::Char('N'),
                                            KeyCode::Esc,
                                            KeyCode::Char('q'),
                                        ],
                                        prompt_type: PromptType::MultiselectAddToQueue,
                                    });
                                } else {
                                    // Single row: its play target — the library
                                    // path or the streamed chart URI.
                                    let mut added = 0;
                                    if let Some(target) = self.play_target_at(self.list_pos()) {
                                        let c = self.client.clone();
                                        let _ = tx.send(TuiCommand::fire(move || async move {
                                            let _ = c.queue().add(&target, None).await;
                                        }));
                                        added += 1;
                                    }
                                    self.fetch_queue().await;
                                    self.footer_notification = Some((
                                        format!("Added {added} track(s) to queue"),
                                        std::time::Instant::now()
                                            + std::time::Duration::from_secs(2),
                                    ));
                                }
                            }
                        }
                    }
                    Some(KeyboardAction::AddToPlaylist) => {
                        if !self.library_pane_focus {
                            // Playlist overview rows have no tracks of their
                            // own: open the playlist first.
                            if self.library_category == 4 && self.browse_detail.is_none() {
                                self.notify_typed(
                                    "System",
                                    Self::NEED_PLAYLIST_FOR_ADD,
                                    NotificationKind::Info,
                                    false,
                                    NotifType::NowPlaying,
                                );
                                return true;
                            }
                            let row_expanded =
                                self.library_category == 2 || self.library_category == 3;
                            let indices: Vec<i64> = if let Some(ids) = self.motion_row_ids() {
                                // Album/artist row: add every cached track in
                                // the album/artist to the playlist.
                                ids
                            } else if self.multiselect_mode && self.selected_count() > 0 {
                                // Re-resolve the selection by stable key; only
                                // library tracks (which have an id) can be put
                                // into a playlist.
                                self.selected_library_ids()
                            } else {
                                let tracks = self.filtered_tracks();
                                tracks
                                    .get(self.list_pos())
                                    .map(|t| vec![t.id])
                                    .unwrap_or_default()
                            };
                            if row_expanded {
                                // Album/artist rows skip the confirmation prompt:
                                // expanding the row is an explicit act.
                                if !indices.is_empty() {
                                    self.pending_track_ids = indices;
                                    self.playlist_creating = false;
                                    self.pickers.open(PickerId::PlaylistSelect);
                                }
                            } else if self.multiselect_mode && self.selected_count() > 0 {
                                if indices.is_empty() {
                                    self.notify_typed(
                                        "System",
                                        "Selected tracks are streamed \u{2014} only library tracks can go into playlists",
                                        NotificationKind::Info,
                                        false,
                                        NotifType::NowPlaying,
                                    );
                                    return true;
                                }
                                let count = indices.len();
                                self.pending_prompt = Some(PendingPrompt {
                                    message: format!("Add {count} tracks to playlist? [y/N]"),
                                    confirm_keys: vec![
                                        KeyCode::Char('y'),
                                        KeyCode::Char('Y'),
                                        KeyCode::Enter,
                                    ],
                                    cancel_keys: vec![
                                        KeyCode::Char('n'),
                                        KeyCode::Char('N'),
                                        KeyCode::Esc,
                                        KeyCode::Char('q'),
                                    ],
                                    prompt_type: PromptType::MultiselectAddToPlaylist,
                                });
                            } else if !indices.is_empty() {
                                self.pending_track_ids = indices;
                                self.playlist_creating = false;
                                self.pickers.open(PickerId::PlaylistSelect);
                            } else if self.library_category == 12 {
                                self.notify_typed(
                                    "System",
                                    "Chart tracks are streamed \u{2014} not in your library",
                                    NotificationKind::Info,
                                    false,
                                    NotifType::NowPlaying,
                                );
                            }
                        }
                    }
                    Some(KeyboardAction::DeleteFromList) => {
                        if !self.library_pane_focus {
                            if self.library_category == 4 && self.browse_detail.is_some() {
                                // In playlist view: remove the highlighted track (or the
                                // multiselect batch, excluding the highlighted row) from
                                // the playlist in one round trip, notifying once.
                                let filtered = self.filtered_tracks();
                                if let Some(pl) = self
                                    .playlist_cache
                                    .iter()
                                    .find(|p| self.browse_detail.as_deref() == Some(&p.name))
                                {
                                    let playlist_id = pl.id;
                                    let client = self.client.clone();
                                    let ipc_tx = self.ipc_tx.clone();
                                    let pos = self.list_pos();
                                    // Batch remove re-resolved by stable key so a
                                    // shifted playlist list can't remove the
                                    // wrong rows.
                                    let ids: Vec<i64> = if self.multiselect_mode
                                        && self.selected_count() > 0
                                    {
                                        let cursor_id =
                                            self.selectable_rows().get(pos).and_then(|r| r.2);
                                        self.selected_library_ids()
                                            .into_iter()
                                            .filter(|&id| Some(id) != cursor_id)
                                            .collect()
                                    } else {
                                        filtered.get(pos).map(|t| vec![t.id]).unwrap_or_default()
                                    };
                                    if ids.is_empty() {
                                        return true;
                                    }
                                    let mut removed = 0;
                                    for id in ids {
                                        if client
                                            .library()
                                            .remove_from_playlist(playlist_id, id)
                                            .await
                                            .is_ok()
                                        {
                                            removed += 1;
                                        }
                                    }
                                    if removed > 0 {
                                        if let Ok(DaemonRes::Playlists { playlists, .. }) =
                                            client.library().get_playlists().await
                                        {
                                            let _ = ipc_tx.send(IpcResult::Playlists(playlists));
                                        }
                                        if let Ok(DaemonRes::Tracks { tracks, .. }) =
                                            client.library().get_playlist_tracks(playlist_id).await
                                        {
                                            let _ = ipc_tx.send(IpcResult::PlaylistTracks(*tracks));
                                        }
                                        self.clear_selection();
                                        self.multiselect_mode = false;
                                        let msg = if removed == 1 {
                                            "Removed from playlist".to_string()
                                        } else {
                                            format!("Removed {removed} from playlist")
                                        };
                                        self.notify_typed(
                                            "System",
                                            msg,
                                            NotificationKind::Info,
                                            false,
                                            NotifType::NowPlaying,
                                        );
                                    }
                                }
                            } else if self.library_category == 6 {
                                // Radio category: removing a custom station
                                // edits radios.toml, so require confirmation.
                                if let Some(station) =
                                    self.radio.custom.get(self.list_pos()).cloned()
                                {
                                    self.pending_prompt = Some(PendingPrompt {
                                        message: format!(
                                            "Remove \"{}\" from custom stations? [y/N]",
                                            station.name
                                        ),
                                        confirm_keys: vec![
                                            KeyCode::Char('y'),
                                            KeyCode::Char('Y'),
                                            KeyCode::Enter,
                                        ],
                                        cancel_keys: vec![
                                            KeyCode::Char('n'),
                                            KeyCode::Char('N'),
                                            KeyCode::Esc,
                                            KeyCode::Char('q'),
                                        ],
                                        prompt_type: PromptType::RemoveCustomRadio(station.name),
                                    });
                                }
                            } else if self.library_category == 12 {
                                // Chart tracks are streamed — nothing in the
                                // library to remove.
                                self.notify_typed(
                                    "System",
                                    "Charts are streamed \u{2014} not in your library",
                                    NotificationKind::Info,
                                    false,
                                    NotifType::NowPlaying,
                                );
                            } else {
                                self.notify_typed(
                                    "System",
                                    Self::PLAYLIST_VIEW_ONLY_REMOVE,
                                    NotificationKind::Info,
                                    false,
                                    NotifType::NowPlaying,
                                );
                            }
                        }
                    }
                    Some(KeyboardAction::JumpToEnd) => {
                        if !self.library_pane_focus {
                            let max = self.library_list_len().saturating_sub(1);
                            self.set_list_pos(max);
                        }
                    }
                    Some(KeyboardAction::EditMetadata) => {
                        if !self.library_pane_focus {
                            // Playlist overview rows have no metadata of their
                            // own: `e` renames the playlist (typing a new name
                            // in the PlaylistSelect input, Enter to commit).
                            if self.library_category == 4 && self.browse_detail.is_none() {
                                if let Some(pl) = self.playlist_cache.get(self.list_pos()).cloned()
                                {
                                    self.renaming_playlist = Some(pl.id);
                                    self.playlist_creating = true;
                                    self.pickers.open(PickerId::PlaylistSelect);
                                    if let Some(top) = self.pickers.top_mut() {
                                        top.query = pl.name;
                                    }
                                    return true;
                                }
                                return true;
                            }
                            if self.library_category == 12 {
                                // Chart tracks are streamed — they have no
                                // library metadata to edit.
                                self.notify_typed(
                                    "System",
                                    "Chart tracks are streamed \u{2014} not in your library",
                                    NotificationKind::Info,
                                    false,
                                    NotifType::NowPlaying,
                                );
                                return true;
                            }
                            let ids = if self.library_category == 2 || self.library_category == 3 {
                                // Album/artist row: edit every cached track in
                                // the album/artist in one batch.
                                self.motion_row_ids().unwrap_or_default()
                            } else {
                                let tracks = self.filtered_tracks();
                                match tracks.get(self.list_pos()) {
                                    Some(t) => vec![t.id],
                                    None => return true,
                                }
                            };
                            if ids.is_empty() {
                                return true;
                            }
                            // Seed the field template from the row itself: the
                            // album/artist name for grouped rows, the
                            // highlighted track otherwise.
                            let (title, artist, album, genre, year, track_num) =
                                match self.library_category {
                                    2 => (
                                        String::new(),
                                        String::new(),
                                        self.unique_albums()
                                            .get(self.list_pos())
                                            .map_or_else(String::new, |(n, _)| n.clone()),
                                        String::new(),
                                        None,
                                        None,
                                    ),
                                    3 => (
                                        String::new(),
                                        self.unique_artists()
                                            .get(self.list_pos())
                                            .map_or_else(String::new, |(n, _)| n.clone()),
                                        String::new(),
                                        String::new(),
                                        None,
                                        None,
                                    ),
                                    _ => {
                                        let tracks = self.filtered_tracks();
                                        match tracks.get(self.list_pos()) {
                                            Some(t) => (
                                                t.title.clone(),
                                                t.artist.clone(),
                                                t.album.clone(),
                                                t.genre.clone(),
                                                t.year,
                                                t.track_number,
                                            ),
                                            None => return true,
                                        }
                                    }
                                };
                            self.metadata.edit_track_ids = ids;
                            self.metadata.fields = [
                                title,
                                artist,
                                album,
                                String::new(),
                                genre,
                                year.map_or(String::new(), |y| y.to_string()),
                                track_num.map_or(String::new(), |n| n.to_string()),
                            ];
                            self.metadata.field_idx = 0;
                            self.pickers.open(PickerId::EditMetadata);
                            self.fetch_metadata_cover();
                        }
                    }
                    // Queue move actions: only handled in picker mode
                    Some(KeyboardAction::QueueMoveUp)
                    | Some(KeyboardAction::QueueMoveDown)
                    | Some(KeyboardAction::QueueMoveConfirm)
                    | Some(KeyboardAction::QueueMoveCancel) => {}
                    None => {
                        match key.code {
                            KeyCode::Char('q') => {
                                if self.browse_detail.is_some() {
                                    self.browse_detail = None;
                                    self.set_list_pos(0);
                                } else {
                                    return false;
                                }
                            }
                            KeyCode::Esc => {
                                if self.browse_detail.is_some() {
                                    self.browse_detail = None;
                                    self.set_list_pos(0);
                                } else if self.library_category == 12 {
                                    // Top Charts: three-level back navigation
                                    if self.charts.selected_chart.is_some() {
                                        // Level 2 -> Level 1
                                        self.charts.selected_chart = None;
                                        self.charts.chart_tracks.clear();
                                        self.set_list_pos(0);
                                    } else if self.charts.selected_source.is_some() {
                                        // Level 1 -> Level 0: refetch so a
                                        // freshly linked Spotify shows up.
                                        self.charts.selected_source = None;
                                        self.charts.charts.clear();
                                        self.charts.chart_tracks.clear();
                                        self.set_list_pos(0);
                                        self.fetch_chart_sources();
                                    }
                                }
                            }
                            KeyCode::Char('S') => {
                                // Sync covers for tracks missing cover art
                                self.notify_titled(
                                    "Library",
                                    "Syncing covers...",
                                    NotificationKind::Info,
                                    true,
                                    NotifType::Library,
                                );
                                sync_and_wait(
                                    self.client.clone(),
                                    SyncKind::Covers,
                                    "Covers",
                                    self.ipc_tx.clone(),
                                );
                            }
                            _ => {}
                        }
                    }
                }
            }
        }
        true
    }

    pub(crate) async fn handle_picker_key(&mut self, key: event::KeyEvent) {
        // While the OAuth browser flow is pending, the SpotifyLink /
        // SpotifySearch pickers are in a waiting state; ignore all key input
        // except Esc (handled in handle_key) so the user can't mutate the
        // now-irrelevant input.
        if self.spotify.oauth_pending
            && self
                .pickers
                .top()
                .is_some_and(|o| o.id == PickerId::SpotifyLink || o.id == PickerId::SpotifySearch)
        {
            return;
        }

        let tx = self.cmd_tx();

        // Notifications picker: 'y' copies the highlighted notification's text
        // (title + message) to the clipboard so error messages are easy to grab.
        if key.code == KeyCode::Char('y')
            && !key.modifiers.contains(KeyModifiers::CONTROL)
            && self
                .pickers
                .top()
                .is_some_and(|o| o.id == PickerId::Notifications)
        {
            let sel = self.pickers.top().map_or(0, |o| o.selected);
            if let Some(rec) = self.notification_history.get(sel) {
                let text = format!("{}: {}", rec.title, rec.message);
                match copy_to_clipboard(&text) {
                    Ok(()) => self.notify_typed(
                        "System",
                        "Notification copied to clipboard",
                        NotificationKind::Success,
                        true,
                        NotifType::System,
                    ),
                    Err(e) => self.notify_typed(
                        "System",
                        format!("Copy failed: {e}"),
                        NotificationKind::Error,
                        true,
                        NotifType::System,
                    ),
                }
            }
            return;
        }

        // Ctrl+D in SpotifySearch picker: download the selected track via YouTube
        if key.modifiers.contains(KeyModifiers::CONTROL)
            && key.code == KeyCode::Char('d')
            && self
                .pickers
                .top()
                .is_some_and(|o| o.id == PickerId::SpotifySearch)
        {
            // `selected` indexes the filtered rows, so it must be mapped back
            // through the pick filter before touching the result list.
            let sel = self.pickers.top().map_or(0, |o| o.selected);
            if let Some(&idx) = self.spot_picks().get(sel) {
                let (playlist_id, _, track) = self.spotify.search_results[idx].clone();
                if track.kind.is_some() {
                    self.notify_typed(
                        "System",
                        "Only single tracks can be downloaded",
                        NotificationKind::Info,
                        true,
                        NotifType::Downloads,
                    );
                    return;
                }
                let track_index = track.index;
                let can_stream = track.uri.is_some();
                let c = self.client.clone();
                let ipc_tx = self.ipc_tx.clone();
                self.pickers.close_top();
                let _ = tx.send(TuiCommand::fire(move || async move {
                    // Web hits are not in any synced playlist: resolve by
                    // metadata + known URI (streams natively on Premium), not
                    // by playlist index.
                    let res = if playlist_id == "web" {
                        c.spotify()
                            .resolve_track(
                                &track.name,
                                &track.artists,
                                track.album.as_deref().unwrap_or(""),
                                track.uri.clone(),
                                false,
                            )
                            .await
                    } else {
                        c.spotify().resolve(&playlist_id, track_index, false).await
                    };
                    match res {
                        Ok(()) => {
                            let _ = ipc_tx.send(IpcResult::Notification(
                                "Spotify".to_string(),
                                format!(
                                    "Queued: {} - {}{}",
                                    track.artists,
                                    track.name,
                                    if can_stream { "" } else { " (YouTube)" }
                                ),
                                NotificationKind::Success,
                                NotifType::Spotify,
                            ));
                        }
                        Err(e) => {
                            let _ = ipc_tx
                                .send(IpcResult::Error(format!("Spotify resolve failed: {e}")));
                        }
                    }
                }));
            } else {
                self.notify_typed(
                    "System",
                    "No track selected to download",
                    NotificationKind::Info,
                    true,
                    NotifType::Downloads,
                );
            }
            return;
        }

        // PlaylistTrackSelect picker (post-create multi-select):
        //   Space / Tab    toggle the highlighted track
        //   Ctrl+Enter     commit highlighted tracks to the playlist
        //   Esc            cancel (handled by the common Esc path)
        if self
            .pickers
            .top()
            .is_some_and(|o| o.id == PickerId::PlaylistTrackSelect)
        {
            let is_ctrl_enter = key.modifiers.contains(KeyModifiers::CONTROL)
                && (key.code == KeyCode::Enter || key.code == KeyCode::Char('m'));
            let is_toggle = key.code == KeyCode::Char(' ') || key.code == KeyCode::Tab;
            if is_ctrl_enter {
                self.commit_playlist_selection();
                return;
            }
            if is_toggle {
                if let Some(top) = self.pickers.top()
                    && let Some(track) = self.tracks_cache.get(top.selected)
                {
                    let id = track.id;
                    if !self.selected_track_ids.remove(&id) {
                        self.selected_track_ids.insert(id);
                    }
                }
                return;
            }
        }

        if matches!(self.pickers.top().map(|o| o.id), Some(PickerId::Queue)) {
            // Queue move mode: Ctrl+j/k to move, Enter to confirm, Esc to cancel
            if self.queue.move_index.is_some() {
                match key.code {
                    KeyCode::Esc => {
                        // Cancel move mode
                        self.queue.move_index = None;
                        self.queue.move_target = 0;
                        if let Some(top) = self.pickers.top_mut() {
                            top.selected = self.queue.move_target;
                        }
                        return;
                    }
                    KeyCode::Enter => {
                        // Confirm move
                        if let Some(from_idx) = self.queue.move_index {
                            let to_idx = self.queue.move_target;
                            if from_idx != to_idx && to_idx < self.queue.cache.len() {
                                self.send_high(TuiCommand::QueueMove(
                                    from_idx as u64,
                                    to_idx as u64,
                                ));
                            }
                        }
                        self.queue.move_index = None;
                        self.queue.move_target = 0;
                        return;
                    }
                    KeyCode::Char('j') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                        // Move down
                        if let Some(top) = self.pickers.top_mut() {
                            top.selected =
                                (top.selected + 1).min(self.queue.cache.len().saturating_sub(1));
                            self.queue.move_target = top.selected;
                        }
                        return;
                    }
                    KeyCode::Char('k') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                        // Move up
                        if let Some(top) = self.pickers.top_mut() {
                            top.selected = top.selected.saturating_sub(1);
                            self.queue.move_target = top.selected;
                        }
                        return;
                    }
                    _ => {}
                }
            } else {
                // Enter move mode when Ctrl+j or Ctrl+k is pressed
                match key.code {
                    KeyCode::Char('j') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                        if !self.queue.cache.is_empty() {
                            self.queue.move_index = Some(
                                self.pickers
                                    .top()
                                    .map(|o| o.selected)
                                    .unwrap_or(0)
                                    .min(self.queue.cache.len().saturating_sub(1)),
                            );
                            self.queue.move_target = self.queue.move_index.unwrap();
                            if let Some(top) = self.pickers.top_mut() {
                                top.selected = (top.selected + 1)
                                    .min(self.queue.cache.len().saturating_sub(1));
                                self.queue.move_target = top.selected;
                            }
                        }
                        return;
                    }
                    KeyCode::Char('k') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                        if !self.queue.cache.is_empty() {
                            self.queue.move_index = Some(
                                self.pickers
                                    .top()
                                    .map(|o| o.selected)
                                    .unwrap_or(0)
                                    .min(self.queue.cache.len().saturating_sub(1)),
                            );
                            self.queue.move_target = self.queue.move_index.unwrap();
                            if let Some(top) = self.pickers.top_mut() {
                                top.selected = top.selected.saturating_sub(1);
                                self.queue.move_target = top.selected;
                            }
                        }
                        return;
                    }
                    _ => {}
                }
            }
        }

        if matches!(self.pickers.top().map(|o| o.id), Some(PickerId::SleepTimer)) {
            if self.sleep_timer.input_mode {
                match key.code {
                    KeyCode::Esc => {
                        self.sleep_timer.input_mode = false;
                        self.sleep_timer.input_buf.clear();
                    }
                    KeyCode::Enter => {
                        if let Ok(m) = self.sleep_timer.input_buf.parse::<u32>() {
                            self.sleep_timer.minutes = m.min(180);
                        }
                        self.sleep_timer.input_mode = false;
                        self.sleep_timer.input_buf.clear();
                    }
                    KeyCode::Backspace => {
                        self.sleep_timer.input_buf.pop();
                    }
                    KeyCode::Up | KeyCode::Char('j') => {
                        self.sleep_timer.minutes = (self.sleep_timer.minutes + 1).min(180);
                        self.sleep_timer.input_buf.clear();
                    }
                    KeyCode::Down | KeyCode::Char('k') => {
                        self.sleep_timer.minutes = self.sleep_timer.minutes.saturating_sub(1);
                        self.sleep_timer.input_buf.clear();
                    }
                    KeyCode::Char(c) if c.is_ascii_digit() => {
                        self.sleep_timer.input_buf.push(c);
                    }
                    _ => {}
                }
                return;
            }
            match key.code {
                KeyCode::Esc => {
                    self.sleep_timer.remaining = None;
                    self.sleep_timer.minutes = 30;
                    self.sleep_timer.input_mode = false;
                    self.sleep_timer.input_buf.clear();
                    self.sleep_timer.focus = 0;
                    self.pickers.close_top();
                    return;
                }
                // Up/Down (and vim j/k) navigate the option rows: the time
                // slider, the seven quick presets, then the immediate-stop
                // checkbox. They never adjust the time.
                KeyCode::Up | KeyCode::Char('j') => {
                    let n = 9;
                    self.sleep_timer.focus = (self.sleep_timer.focus + n - 1) % n;
                    return;
                }
                KeyCode::Down | KeyCode::Char('k') => {
                    let n = 9;
                    self.sleep_timer.focus = (self.sleep_timer.focus + 1) % n;
                    return;
                }
                // Side arrows adjust the time like h/l do.
                KeyCode::Left | KeyCode::Char('h') => {
                    self.sleep_timer.minutes = self.sleep_timer.minutes.saturating_sub(5);
                    return;
                }
                KeyCode::Right | KeyCode::Char('l') => {
                    self.sleep_timer.minutes = (self.sleep_timer.minutes + 5).min(180);
                    return;
                }
                KeyCode::Char('-') => {
                    self.sleep_timer.minutes = self.sleep_timer.minutes.saturating_sub(1);
                    return;
                }
                KeyCode::Char('+') | KeyCode::Char('=') => {
                    self.sleep_timer.minutes = (self.sleep_timer.minutes + 1).min(180);
                    return;
                }
                KeyCode::Enter => {
                    // Row 8 is the immediate-stop checkbox: Enter toggles it
                    // and keeps the picker open.
                    if self.sleep_timer.focus == 8 {
                        self.sleep_timer.stop_immediately = !self.sleep_timer.stop_immediately;
                        return;
                    }
                    // Rows 1..=7 are the quick presets: selecting one sets the
                    // minutes. Row 0 (slider) uses the current minutes.
                    if (1..=7).contains(&self.sleep_timer.focus) {
                        let presets = [5u32, 10, 15, 30, 60, 90, 120];
                        if let Some(&m) = presets.get(self.sleep_timer.focus - 1) {
                            self.sleep_timer.minutes = m;
                        }
                    }
                    let mins = self.sleep_timer.minutes;
                    let stop_now = self.sleep_timer.stop_immediately;
                    self.sleep_timer.remaining = Some(mins as u64);
                    self.send_high(TuiCommand::SetSleepTimer(mins, stop_now));
                    self.footer_notification = Some((
                        format!("Sleep timer set: {} min", mins),
                        std::time::Instant::now() + std::time::Duration::from_secs(2),
                    ));
                    self.pickers.close_top();
                    return;
                }
                KeyCode::Char(' ') => {
                    if self.sleep_timer.focus == 8 {
                        self.sleep_timer.stop_immediately = !self.sleep_timer.stop_immediately;
                    }
                    return;
                }
                KeyCode::Char('i') => {
                    self.sleep_timer.input_mode = true;
                    self.sleep_timer.input_buf.clear();
                    return;
                }
                KeyCode::Char('c') => {
                    self.sleep_timer.remaining = None;
                    self.send_high(TuiCommand::CancelSleepTimer);
                    self.footer_notification = Some((
                        "Sleep timer cancelled".to_string(),
                        std::time::Instant::now() + std::time::Duration::from_secs(2),
                    ));
                    return;
                }
                _ => {}
            }
            return;
        }

        if matches!(self.pickers.top().map(|o| o.id), Some(PickerId::Settings)) {
            let settings_focus = self.settings_pane_focus;
            match key.code {
                KeyCode::Esc => {
                    self.pickers.close_top();
                    return;
                }
                KeyCode::Tab => {
                    self.settings_pane_focus = !self.settings_pane_focus;
                    return;
                }
                KeyCode::Left | KeyCode::Char('h') => {
                    if !settings_focus {
                        match self.settings_category {
                            1 => match self.settings_option {
                                0 => {
                                    let next = match self.state.repeat {
                                        RepeatMode::Off => RepeatMode::One,
                                        RepeatMode::One => RepeatMode::All,
                                        RepeatMode::All => RepeatMode::Off,
                                    };
                                    let c = self.client.clone();
                                    let _ = tx.send(TuiCommand::fire(move || async move {
                                        let _ = c.cycle_repeat(next).await;
                                    }));
                                    self.state.repeat = next;
                                }
                                1 => {
                                    let c = self.client.clone();
                                    let _ = tx.send(TuiCommand::fire(move || async move {
                                        let _ = c.toggle_shuffle().await;
                                    }));
                                    self.state.shuffle = !self.state.shuffle;
                                }
                                3 => {
                                    let new_enabled = !self.state.audio.eq_enabled;
                                    self.state.audio.eq_enabled = new_enabled;
                                    let c = self.client.clone();
                                    let _ = tx.send(TuiCommand::fire(move || async move {
                                        let _ = c.set_eq_enabled(new_enabled).await;
                                    }));
                                }
                                4 => {
                                    let new_enabled = !self.state.audio.reverb.enabled;
                                    let room_size = self.state.audio.reverb.room_size;
                                    self.state.audio.reverb.enabled = new_enabled;
                                    let c = self.client.clone();
                                    let _ = tx.send(TuiCommand::fire(move || async move {
                                        let _ = c.set_reverb(new_enabled, room_size).await;
                                    }));
                                }
                                5 => {
                                    self.cycle_cover_provider();
                                }
                                _ => {}
                            },
                            2 => match self.settings_option {
                                1 => {
                                    self.transparent_bg = !self.transparent_bg;
                                    save_prefs(&self.current_prefs());
                                }
                                2 => {
                                    self.transparent_pickers = !self.transparent_pickers;
                                    save_prefs(&self.current_prefs());
                                }
                                8 => {
                                    self.reactive_theme = !self.reactive_theme;
                                    if self.reactive_theme && self.reactive_palette.is_none() {
                                        if let Some(c) = self.np_cover.image.clone() {
                                            let tx = self.ipc_tx.clone();
                                            self.request_reactive_palette(&c, tx);
                                        } else if let Some(tid) =
                                            self.state.current_track.as_ref().map(|t| t.id)
                                        {
                                            let fetch_gen = self.next_cover_gen();
                                            self.np_cover.pending_gen = Some(fetch_gen);
                                            let client = self.client.clone();
                                            let ipc_tx = self.ipc_tx.clone();
                                            let _ = tx.send(TuiCommand::fire(move || async move {
                                                if let Ok(Some(b64)) = client.art().cover(tid).await
                                                    && let Ok(bytes) =
                                                        base64::engine::general_purpose::STANDARD
                                                            .decode(&b64)
                                                {
                                                    let _ = ipc_tx.send(IpcResult::CoverArt(
                                                        Some(bytes),
                                                        Some(tid),
                                                        fetch_gen,
                                                    ));
                                                }
                                            }));
                                        }
                                    }
                                    self.apply_reactive();
                                    save_prefs(&self.current_prefs());
                                }
                                9 => {
                                    self.cycle_reactive_intensity();
                                }
                                _ => {}
                            },
                            3 => {}
                            _ => {}
                        }
                    }
                    return;
                }
                KeyCode::Right | KeyCode::Char('l') => {
                    if !settings_focus {
                        match self.settings_category {
                            1 => match self.settings_option {
                                0 => {
                                    let next = match self.state.repeat {
                                        RepeatMode::Off => RepeatMode::One,
                                        RepeatMode::One => RepeatMode::All,
                                        RepeatMode::All => RepeatMode::Off,
                                    };
                                    let c = self.client.clone();
                                    let _ = tx.send(TuiCommand::fire(move || async move {
                                        let _ = c.cycle_repeat(next).await;
                                    }));
                                    self.state.repeat = next;
                                }
                                1 => {
                                    let c = self.client.clone();
                                    let _ = tx.send(TuiCommand::fire(move || async move {
                                        let _ = c.toggle_shuffle().await;
                                    }));
                                    self.state.shuffle = !self.state.shuffle;
                                }
                                3 => {
                                    let new_enabled = !self.state.audio.eq_enabled;
                                    self.state.audio.eq_enabled = new_enabled;
                                    let c = self.client.clone();
                                    let _ = tx.send(TuiCommand::fire(move || async move {
                                        let _ = c.set_eq_enabled(new_enabled).await;
                                    }));
                                }
                                4 => {
                                    let new_enabled = !self.state.audio.reverb.enabled;
                                    let room_size = self.state.audio.reverb.room_size;
                                    self.state.audio.reverb.enabled = new_enabled;
                                    let c = self.client.clone();
                                    let _ = tx.send(TuiCommand::fire(move || async move {
                                        let _ = c.set_reverb(new_enabled, room_size).await;
                                    }));
                                }
                                5 => {
                                    self.cycle_cover_provider();
                                }
                                _ => {}
                            },
                            2 => match self.settings_option {
                                1 => {
                                    self.transparent_bg = !self.transparent_bg;
                                    save_prefs(&self.current_prefs());
                                }
                                2 => {
                                    self.transparent_pickers = !self.transparent_pickers;
                                    save_prefs(&self.current_prefs());
                                }
                                8 => {
                                    self.reactive_theme = !self.reactive_theme;
                                    if self.reactive_theme && self.reactive_palette.is_none() {
                                        if let Some(c) = self.np_cover.image.clone() {
                                            let tx = self.ipc_tx.clone();
                                            self.request_reactive_palette(&c, tx);
                                        } else if let Some(tid) =
                                            self.state.current_track.as_ref().map(|t| t.id)
                                        {
                                            let fetch_gen = self.next_cover_gen();
                                            self.np_cover.pending_gen = Some(fetch_gen);
                                            let client = self.client.clone();
                                            let ipc_tx = self.ipc_tx.clone();
                                            let _ = tx.send(TuiCommand::fire(move || async move {
                                                if let Ok(Some(b64)) = client.art().cover(tid).await
                                                    && let Ok(bytes) =
                                                        base64::engine::general_purpose::STANDARD
                                                            .decode(&b64)
                                                {
                                                    let _ = ipc_tx.send(IpcResult::CoverArt(
                                                        Some(bytes),
                                                        Some(tid),
                                                        fetch_gen,
                                                    ));
                                                }
                                            }));
                                        }
                                    }
                                    self.apply_reactive();
                                    save_prefs(&self.current_prefs());
                                }
                                9 => {
                                    self.cycle_reactive_intensity();
                                }
                                _ => {}
                            },
                            3 => {}
                            _ => {}
                        }
                    }
                    return;
                }
                KeyCode::Up | KeyCode::Char('k') => {
                    if settings_focus {
                        self.settings_category = self.settings_category.saturating_sub(1);
                        self.settings_option = 0;
                    } else {
                        self.settings_option = self.settings_option.saturating_sub(1);
                    }
                    return;
                }
                KeyCode::Down | KeyCode::Char('j') => {
                    if settings_focus {
                        self.settings_category =
                            (self.settings_category + 1).min(NUM_SETTINGS_CATEGORIES - 1);
                        self.settings_option = 0;
                    } else {
                        let max = self.category_options().saturating_sub(1);
                        self.settings_option = (self.settings_option + 1).min(max);
                    }
                    return;
                }
                KeyCode::Enter => {
                    if !settings_focus {
                        let opt = self.settings_option;
                        match self.settings_category {
                            0 => {
                                if opt == 1 {
                                    let current = self.cookie_file.clone();
                                    let new_path = if current.is_some() {
                                        None
                                    } else {
                                        let home = std::env::var("HOME").unwrap_or_default();
                                        Some(format!("{home}/.cookies/youtube.txt"))
                                    };
                                    let display =
                                        new_path.clone().unwrap_or_else(|| "(none)".to_string());
                                    self.cookie_file = new_path.clone();
                                    let c = self.client.clone();
                                    let cf = new_path;
                                    let _ = tx.send(TuiCommand::fire(move || async move {
                                        let _ = c.yt().set_config(None, cf, None, None, None).await;
                                    }));
                                    self.notify_typed(
                                        "System",
                                        format!("Cookie file: {display}"),
                                        NotificationKind::Info,
                                        true,
                                        NotifType::Prefs,
                                    );
                                }
                            }
                            1 => match opt {
                                0 => {
                                    let next = match self.state.repeat {
                                        RepeatMode::Off => RepeatMode::One,
                                        RepeatMode::One => RepeatMode::All,
                                        RepeatMode::All => RepeatMode::Off,
                                    };
                                    let c = self.client.clone();
                                    let _ = tx.send(TuiCommand::fire(move || async move {
                                        let _ = c.cycle_repeat(next).await;
                                    }));
                                    self.state.repeat = next;
                                }
                                1 => {
                                    let c = self.client.clone();
                                    let _ = tx.send(TuiCommand::fire(move || async move {
                                        let _ = c.toggle_shuffle().await;
                                    }));
                                    self.state.shuffle = !self.state.shuffle;
                                }
                                2 => {
                                    self.pickers.open(PickerId::Crossfade);
                                }
                                3 => {
                                    let new_enabled = !self.state.audio.eq_enabled;
                                    self.state.audio.eq_enabled = new_enabled;
                                    let c = self.client.clone();
                                    let _ = tx.send(TuiCommand::fire(move || async move {
                                        let _ = c.set_eq_enabled(new_enabled).await;
                                    }));
                                }
                                4 => {
                                    let new_enabled = !self.state.audio.reverb.enabled;
                                    let room_size = self.state.audio.reverb.room_size;
                                    self.state.audio.reverb.enabled = new_enabled;
                                    let c = self.client.clone();
                                    let _ = tx.send(TuiCommand::fire(move || async move {
                                        let _ = c.set_reverb(new_enabled, room_size).await;
                                    }));
                                }
                                5 => {
                                    self.cycle_cover_provider();
                                }
                                _ => {}
                            },
                            2 => match opt {
                                0 => {
                                    self.pickers.open(PickerId::ThemePicker);
                                }
                                1 => {
                                    self.transparent_bg = !self.transparent_bg;
                                    save_prefs(&self.current_prefs());
                                }
                                2 => {
                                    self.transparent_pickers = !self.transparent_pickers;
                                    save_prefs(&self.current_prefs());
                                }
                                3 => {
                                    sync_and_wait(
                                        self.client.clone(),
                                        SyncKind::Covers,
                                        "Covers",
                                        self.ipc_tx.clone(),
                                    );
                                }
                                4 => {
                                    sync_and_wait(
                                        self.client.clone(),
                                        SyncKind::Lyrics,
                                        "Lyrics",
                                        self.ipc_tx.clone(),
                                    );
                                }
                                5 => {
                                    sync_and_wait(
                                        self.client.clone(),
                                        SyncKind::Metadata,
                                        "Metadata",
                                        self.ipc_tx.clone(),
                                    );
                                }
                                6 => {
                                    self.pickers.open(PickerId::FooterPreset);
                                }
                                7 => {
                                    self.pickers.open(PickerId::VisualizerPreset);
                                }
                                8 => {
                                    self.reactive_theme = !self.reactive_theme;
                                    if self.reactive_theme && self.reactive_palette.is_none() {
                                        if let Some(c) = self.np_cover.image.clone() {
                                            let tx = self.ipc_tx.clone();
                                            self.request_reactive_palette(&c, tx);
                                        } else if let Some(tid) =
                                            self.state.current_track.as_ref().map(|t| t.id)
                                        {
                                            let fetch_gen = self.next_cover_gen();
                                            self.np_cover.pending_gen = Some(fetch_gen);
                                            let client = self.client.clone();
                                            let ipc_tx = self.ipc_tx.clone();
                                            let _ = tx.send(TuiCommand::fire(move || async move {
                                                if let Ok(Some(b64)) = client.art().cover(tid).await
                                                    && let Ok(bytes) =
                                                        base64::engine::general_purpose::STANDARD
                                                            .decode(&b64)
                                                {
                                                    let _ = ipc_tx.send(IpcResult::CoverArt(
                                                        Some(bytes),
                                                        Some(tid),
                                                        fetch_gen,
                                                    ));
                                                }
                                            }));
                                        }
                                    }
                                    self.apply_reactive();
                                    save_prefs(&self.current_prefs());
                                }
                                9 => {
                                    self.cycle_reactive_intensity();
                                }
                                10 | 11 => {
                                    let what = if opt == 10 {
                                        CacheKind::Lyrics
                                    } else {
                                        CacheKind::Covers
                                    };
                                    let label = if opt == 10 { "lyrics" } else { "cover art" };
                                    let c = self.client.clone();
                                    let ipc_tx = self.ipc_tx.clone();
                                    let _ = tx.send(TuiCommand::fire(move || async move {
                                        match c.clear_cache(what).await {
                                            Ok(()) => {
                                                let _ = ipc_tx.send(IpcResult::Notification(
                                                    "Cache".to_string(),
                                                    format!("Cleared {label} cache"),
                                                    NotificationKind::Success,
                                                    NotifType::Prefs,
                                                ));
                                            }
                                            Err(e) => {
                                                let _ = ipc_tx.send(IpcResult::Error(format!(
                                                    "Clear {label} cache: {e}"
                                                )));
                                            }
                                        }
                                    }));
                                }
                                12 => {
                                    self.cycle_cover_cache();
                                }
                                13 => {
                                    self.open_settings_overlay();
                                }
                                14 => {
                                    self.cycle_theme_mode();
                                }
                                _ => {}
                            },
                            3 => match opt {
                                0 => {
                                    let c = self.client.clone();
                                    let ipc_tx = self.ipc_tx.clone();
                                    let _ = tx.send(TuiCommand::fire(move || async move {
                                        match c.spotify().play_pause().await {
                                            Ok(status) => {
                                                let _ =
                                                    ipc_tx.send(IpcResult::SpotifyStatus(status));
                                            }
                                            Err(e) => {
                                                let _ = ipc_tx.send(IpcResult::Error(format!(
                                                    "Spotify play/pause: {e}"
                                                )));
                                            }
                                        }
                                    }));
                                }
                                3 => {
                                    self.spotify.link_input.clear();
                                    self.spotify.oauth_port = "8990".to_string();
                                    self.spotify.link_field = 0;
                                    if let Some(cid) = get_secret(SPOTIFY_CLIENT_ID) {
                                        self.spotify.link_input = cid;
                                    }
                                    self.pickers.open(PickerId::SpotifyLink);
                                }
                                4 => {
                                    self.trigger_spotify_sync(true);
                                }
                                5 => {
                                    let c = self.client.clone();
                                    let ipc_tx = self.ipc_tx.clone();
                                    let _ = tx.send(TuiCommand::fire(move || async move {
                                        match c.spotify().clear().await {
                                            Ok(status) => {
                                                let _ =
                                                    ipc_tx.send(IpcResult::SpotifyStatus(status));
                                                let _ = ipc_tx.send(IpcResult::Notification(
                                                    "Spotify".to_string(),
                                                    "Account unlinked".to_string(),
                                                    NotificationKind::Info,
                                                    NotifType::Spotify,
                                                ));
                                            }
                                            Err(e) => {
                                                let _ = ipc_tx.send(IpcResult::Error(format!(
                                                    "Spotify unlink: {e}"
                                                )));
                                            }
                                        }
                                    }));
                                }
                                7 => {
                                    self.spotify.link_input.clear();
                                    self.spotify.oauth_port = "8990".to_string();
                                    self.spotify.link_field = 0;
                                    if let Some(cid) = get_secret(SPOTIFY_CLIENT_ID) {
                                        self.spotify.link_input = cid;
                                    }
                                    self.pickers.open(PickerId::SpotifyLink);
                                }
                                8..=11 => self.spot_ctrl(opt),
                                _ => {}
                            },
                            9 => {
                                self.hide_footer = !self.hide_footer;
                                save_prefs(&self.current_prefs());
                            }
                            _ => {}
                        }
                    }
                    return;
                }
                _ => {}
            }
            return;
        }

        // ─── gtm setup service chooser ───
        if matches!(self.pickers.top().map(|o| o.id), Some(PickerId::Setup)) {
            match key.code {
                KeyCode::Up | KeyCode::Down => {
                    let n = 3;
                    self.setup.selection = (self.setup.selection as i32
                        + if key.code == KeyCode::Down { 1 } else { -1 })
                    .rem_euclid(n) as usize;
                }
                KeyCode::Enter => {
                    let (sel, service) = setup_selection(self);
                    let _ = sel;
                    self.pickers.close_top();
                    self.open_setup_picker(Some(service));
                }
                _ => {}
            }
            return;
        }

        // ─── Last.fm setup form ───
        if matches!(self.pickers.top().map(|o| o.id), Some(PickerId::LastfmAuth)) {
            match key.code {
                KeyCode::Char(c) => {
                    if !c.is_control() {
                        match self.setup.lastfm_focus {
                            0 => self.setup.lastfm_api_key.push(c),
                            _ => self.setup.lastfm_api_secret.push(c),
                        }
                    }
                }
                KeyCode::Backspace => match self.setup.lastfm_focus {
                    0 => {
                        self.setup.lastfm_api_key.pop();
                    }
                    _ => {
                        self.setup.lastfm_api_secret.pop();
                    }
                },
                KeyCode::Tab => {
                    self.setup.lastfm_focus = (self.setup.lastfm_focus + 1) % 2;
                }
                KeyCode::Enter => {
                    let api_key = self.setup.lastfm_api_key.clone();
                    let api_secret = self.setup.lastfm_api_secret.clone();
                    if api_key.trim().is_empty() || api_secret.trim().is_empty() {
                        self.setup.lastfm_error = Some("API key and secret are required".into());
                        return;
                    }
                    let port = lastfm_callback_port();
                    let c = self.client.clone();
                    let ipc_tx = self.ipc_tx.clone();
                    self.setup.lastfm_pending = true;
                    self.setup.lastfm_error = None;
                    let _ = tx.send(TuiCommand::fire(move || async move {
                        match c
                            .lastfm()
                            .set_config(true, Some(api_key), Some(api_secret), None, None, None)
                            .await
                        {
                            Ok(()) => match c.lastfm().oauth_start(port).await {
                                Ok(url) => {
                                    // The daemon bound the callback port before
                                    // returning this URL, so the browser's
                                    // redirect after authorization always lands
                                    // on a live listener. Completion (or
                                    // failure) arrives back as a status event
                                    // pushed by the daemon — no client-side
                                    // callback socket or polling.
                                    let ipc_url = ipc_tx.clone();
                                    let open_url = url.clone();
                                    tokio::spawn(async move {
                                        if !open_browser(&open_url).await {
                                            let _ = ipc_url.send(IpcResult::AuthError(
                                                "Last.fm",
                                                "Could not open a browser automatically — copy the URL from the last.fm setup screen".into(),
                                            ));
                                        }
                                    });
                                    let _ = ipc_tx.send(IpcResult::AuthUrl("Last.fm", url));
                                }
                                Err(e) => {
                                    let _ = ipc_tx.send(IpcResult::AuthError(
                                        "Last.fm",
                                        format!("Last.fm link failed: {e}"),
                                    ));
                                }
                            },
                            Err(e) => {
                                let _ = ipc_tx.send(IpcResult::AuthError(
                                    "Last.fm",
                                    format!("saving Last.fm config failed: {e}"),
                                ));
                            }
                        }
                    }));
                }
                _ => {}
            }
            return;
        }

        // ─── YouTube cookie-file form ───
        if matches!(
            self.pickers.top().map(|o| o.id),
            Some(PickerId::YoutubeSetup)
        ) {
            match key.code {
                KeyCode::Char(c) => {
                    if !c.is_control() {
                        self.setup.youtube_cookie_input.push(c);
                    }
                }
                KeyCode::Backspace => {
                    self.setup.youtube_cookie_input.pop();
                }
                KeyCode::Esc => {
                    self.pickers.close_top();
                }
                KeyCode::Enter => {
                    let path = self.setup.youtube_cookie_input.clone();
                    let trimmed = path.trim();
                    let new_path = if trimmed.is_empty() {
                        None
                    } else {
                        Some(trimmed.to_string())
                    };
                    self.cookie_file = new_path.clone();
                    let c = self.client.clone();
                    let cf = new_path.clone();
                    let display = new_path.clone().unwrap_or_else(|| "(none)".to_string());
                    self.pickers.close_top();
                    let _ = tx.send(TuiCommand::fire(move || async move {
                        let _ = c.yt().set_config(None, cf, None, None, None).await;
                    }));
                    let (msg, kind) = if trimmed.is_empty() {
                        ("Cookie file cleared".to_string(), NotificationKind::Info)
                    } else {
                        (format!("Cookie file: {display}"), NotificationKind::Info)
                    };
                    self.notify_typed("System", msg, kind, true, NotifType::Prefs);
                }
                _ => {}
            }
            return;
        }

        // ─── Podcast feeds ───
        if matches!(
            self.pickers.top().map(|o| o.id),
            Some(PickerId::PodcastFeeds)
        ) {
            match key.code {
                KeyCode::Enter => {
                    let sel = self.pickers.top().map_or(0, |o| o.selected);
                    if let Some(feed) = self.podcast.feeds.get(sel).cloned() {
                        self.podcast.episodes.clear();
                        self.podcast.episodes_feed_id = Some(feed.id.clone());
                        self.pickers.open(PickerId::PodcastEpisodes);
                        self.fetch_podcast_episodes(feed.id);
                    }
                }
                KeyCode::Char('a') => {
                    self.podcast.subscribe_url.clear();
                    self.pickers.open(PickerId::PodcastSubscribe);
                }
                KeyCode::Char('r') => {
                    let c = self.client.clone();
                    let _ = tx.send(TuiCommand::fire(move || async move {
                        let _ = c.podcast().refresh(None).await;
                    }));
                    self.podcast.feeds.clear();
                    self.podcast.feeds_pending = true;
                    let c = self.client.clone();
                    let ipc_tx = self.ipc_tx.clone();
                    let _ = tx.send(TuiCommand::fire(move || async move {
                        match c.podcast().feeds().await {
                            Ok(f) => {
                                let _ = ipc_tx.send(IpcResult::PodcastFeeds(f));
                            }
                            Err(e) => {
                                self_err(&ipc_tx, format!("podcast feeds failed: {e}"));
                            }
                        }
                    }));
                }
                KeyCode::Up | KeyCode::Down => {
                    self.move_picker_selection(key.code == KeyCode::Down);
                }
                _ => {}
            }
            return;
        }

        // ─── Podcast episodes ───
        if matches!(
            self.pickers.top().map(|o| o.id),
            Some(PickerId::PodcastEpisodes)
        ) {
            match key.code {
                KeyCode::Enter => {
                    let idx = self.pickers.top().map_or(0, |o| o.selected);
                    let feed_id = self.podcast.episodes_feed_id.clone();
                    if let (Some(feed_id), Some(_ep)) = (feed_id, self.podcast.episodes.get(idx)) {
                        let c = self.client.clone();
                        self.pickers.close_top();
                        let _ = tx.send(TuiCommand::fire(move || async move {
                            let _ = c.podcast().play(&feed_id, idx).await;
                        }));
                    }
                }
                KeyCode::Backspace => {
                    self.podcast.episodes.clear();
                    self.podcast.episodes_feed_id = None;
                    self.pickers.close_top();
                }
                KeyCode::Up | KeyCode::Down => {
                    self.move_picker_selection(key.code == KeyCode::Down);
                }
                _ => {}
            }
            return;
        }

        // ─── Podcast subscribe form ───
        if matches!(
            self.pickers.top().map(|o| o.id),
            Some(PickerId::PodcastSubscribe)
        ) {
            match key.code {
                KeyCode::Char(c) => {
                    if !c.is_control() {
                        self.podcast.subscribe_url.push(c);
                    }
                }
                KeyCode::Backspace => {
                    self.podcast.subscribe_url.pop();
                }
                KeyCode::Enter => {
                    let url = self.podcast.subscribe_url.clone();
                    if url.is_empty() {
                        return;
                    }
                    let c = self.client.clone();
                    let ipc_tx = self.ipc_tx.clone();
                    self.pickers.close_top();
                    let _ = tx.send(TuiCommand::fire(move || async move {
                        match c.podcast().add_feed(&url).await {
                            Ok(_) => {
                                let _ = ipc_tx.send(IpcResult::Notification(
                                    "Podcast".into(),
                                    format!("Subscribed to {url}"),
                                    NotificationKind::Success,
                                    NotifType::Podcast,
                                ));
                            }
                            Err(e) => {
                                self_err(&ipc_tx, format!("subscribe failed: {e}"));
                            }
                        }
                    }));
                }
                _ => {}
            }
            return;
        }

        // ─── Load stream picker (Alt+O) ───
        if matches!(self.pickers.top().map(|o| o.id), Some(PickerId::LoadStream)) {
            match key.code {
                KeyCode::Char(c) => {
                    if let Some(top) = self.pickers.top_mut()
                        && !c.is_control()
                        && top.query.len() < 2048
                    {
                        top.query.push(c);
                    }
                }
                KeyCode::Backspace => {
                    if let Some(top) = self.pickers.top_mut() {
                        top.query.pop();
                    }
                }
                KeyCode::Enter => {
                    let url = self
                        .pickers
                        .top()
                        .map_or(String::new(), |o| o.query.clone());
                    if url.trim().is_empty() {
                        return;
                    }
                    let c = self.client.clone();
                    let ipc_tx = self.ipc_tx.clone();
                    self.pickers.close_top();
                    let _ = tx.send(TuiCommand::fire(move || async move {
                        if let Err(e) = c.play_stream(url.trim()).await {
                            self_err(&ipc_tx, format!("stream failed: {e}"));
                        }
                    }));
                }
                _ => {}
            }
            return;
        }

        // ─── Unified Radio picker (Alt+R) ───
        // One picker over the merged root list (Saved / Top / Tags /
        // Countries) with drill-down into tag/country stations and a
        // filterable search box. Tab cycles the filter field, 's' saves a
        // directory station, 'x' removes a saved station, 'r' refreshes the
        // current section, Esc pops drill-downs back to root (handled in
        // handle_key).
        if matches!(self.pickers.top().map(|o| o.id), Some(PickerId::Radio)) {
            match key.code {
                KeyCode::Tab => {
                    self.radio.filter = self.radio.filter.next();
                }
                KeyCode::Enter => {
                    self.radio_enter();
                }
                KeyCode::Char('s') => {
                    let picks = self.radio_picks();
                    let sel = self.pickers.top().map_or(0, |o| o.selected);
                    let station = match picks.get(sel) {
                        Some(RadioPick::Station(i)) => match self.radio.section {
                            RadioSection::Stations => self.radio.browse_stations.get(*i).cloned(),
                            RadioSection::Results => self.radio.search.get(*i).cloned(),
                            RadioSection::Root => self.radio.top.get(*i).cloned(),
                        },
                        _ => None,
                    };
                    if let Some(station) = station {
                        self.save_custom_station(&station);
                    }
                }
                KeyCode::Char('x') => {
                    let picks = self.radio_picks();
                    let sel = self.pickers.top().map_or(0, |o| o.selected);
                    if let Some(RadioPick::Custom(i)) = picks.get(sel)
                        && let Some(station) = self.radio.custom.get(*i)
                    {
                        // Prompt keys are intercepted in handle_key's normal
                        // mode, so leave the picker before arming it.
                        let name = station.name.clone();
                        self.pickers.close_top();
                        self.pending_prompt = Some(PendingPrompt {
                            message: format!("Remove \"{name}\" from custom stations? [y/N]"),
                            confirm_keys: vec![
                                KeyCode::Char('y'),
                                KeyCode::Char('Y'),
                                KeyCode::Enter,
                            ],
                            cancel_keys: vec![
                                KeyCode::Char('n'),
                                KeyCode::Char('N'),
                                KeyCode::Esc,
                                KeyCode::Char('q'),
                            ],
                            prompt_type: PromptType::RemoveCustomRadio(name),
                        });
                    }
                }
                KeyCode::Char('r') => {
                    self.refresh_radio_section();
                }
                KeyCode::Char(c) => {
                    if !c.is_control()
                        && let Some(top) = self.pickers.top_mut()
                    {
                        top.query.push(c);
                    }
                }
                KeyCode::Backspace => {
                    if let Some(top) = self.pickers.top_mut() {
                        top.query.pop();
                    }
                }
                KeyCode::Up | KeyCode::Down => {
                    self.move_radio_selection(key.code == KeyCode::Down);
                }
                _ => {}
            }
            return;
        }

        let top_id = self.pickers.top().map(|o| o.id);
        let is_help = top_id == Some(PickerId::Help);
        let ctrl_or_alt = key
            .modifiers
            .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT);

        match key.code {
            KeyCode::Esc => {
                self.close_picker();
            }
            // Uniform picker navigation: Left/Right leave
            // single-section pickers like Esc, or cycle the mode in the
            // notification settings picker.
            KeyCode::Left | KeyCode::Right => {
                let top_id = self.pickers.top().map(|o| o.id);
                if top_id == Some(PickerId::NotificationSettings) {
                    self.cycle_notification_mode(if key.code == KeyCode::Right { 1 } else { -1 });
                } else if matches!(
                    top_id,
                    Some(PickerId::Equalizer)
                        | Some(PickerId::ThemePicker)
                        | Some(PickerId::PlaylistSelect)
                ) {
                    self.close_picker();
                }
            }
            // Help picker vim motions
            KeyCode::Char('g') if key.modifiers == KeyModifiers::CONTROL && is_help => {
                if let Some(top) = self.pickers.top_mut() {
                    top.selected = 0;
                }
            }
            KeyCode::Char('G') if is_help && !ctrl_or_alt => {
                let total = self.help_picker_total();
                if let Some(top) = self.pickers.top_mut() {
                    top.selected = total.saturating_sub(1);
                }
            }
            KeyCode::Char('0') if is_help && !ctrl_or_alt => {
                if let Some(top) = self.pickers.top_mut() {
                    top.selected = 0;
                }
            }
            KeyCode::Char('$') if is_help && !ctrl_or_alt => {
                let total = self.help_picker_total();
                if let Some(top) = self.pickers.top_mut() {
                    top.selected = total.saturating_sub(1);
                }
            }
            KeyCode::Char('n') if is_help && !ctrl_or_alt => {
                let total = self.help_picker_total();
                if let Some(top) = self.pickers.top_mut()
                    && total > 0
                {
                    top.selected = (top.selected + 1).min(total - 1);
                }
            }
            KeyCode::Char('N') if is_help && !ctrl_or_alt => {
                if let Some(top) = self.pickers.top_mut() {
                    top.selected = top.selected.saturating_sub(1);
                }
            }
            // Queue move up/down (Ctrl+K/J) must come before plain k/j
            KeyCode::Char('k') if key.modifiers == KeyModifiers::CONTROL => {
                if let Some(top) = self.pickers.top()
                    && top.id == PickerId::Queue
                    && !self.queue.cache.is_empty()
                {
                    let idx = top.selected.min(self.queue.cache.len() - 1);
                    if idx > 0 {
                        let _ = tx
                            .send(TuiCommand::QueueMove(
                                idx as u64,
                                idx.saturating_sub(1) as u64,
                            ))
                            .await;
                        self.fetch_queue().await;
                    }
                }
            }
            KeyCode::Char('j') if key.modifiers == KeyModifiers::CONTROL => {
                if let Some(top) = self.pickers.top()
                    && top.id == PickerId::Queue
                    && !self.queue.cache.is_empty()
                {
                    let idx = top.selected.min(self.queue.cache.len() - 1);
                    if idx < self.queue.cache.len() - 1 {
                        let _ = tx
                            .send(TuiCommand::QueueMove(idx as u64, (idx + 1) as u64))
                            .await;
                        self.fetch_queue().await;
                    }
                }
            }
            KeyCode::Up | KeyCode::Char('k') => {
                let has_input = matches!(
                    self.pickers.top().map(|o| o.id),
                    Some(PickerId::YTSearch)
                        | Some(PickerId::SearchLibrary)
                        | Some(PickerId::CommandPalette)
                        | Some(PickerId::ThemePicker)
                        | Some(PickerId::SpotifySearch)
                        | Some(PickerId::SpotifyLink)
                );
                let is_metadata = matches!(
                    self.pickers.top().map(|o| o.id),
                    Some(PickerId::EditMetadata)
                );
                if is_metadata {
                    if self.metadata.field_idx > 0 {
                        self.metadata.field_idx -= 1;
                    }
                    return;
                }
                if has_input && key.code != KeyCode::Up {
                    // Add 'k' to the query instead of navigating
                    if let Some(top) = self.pickers.top_mut() {
                        top.query.push('k');
                    }
                    return;
                }
                let count = self.picker_item_count();
                if let Some(top) = self.pickers.top_mut() {
                    if count > 0 && top.selected == 0 {
                        top.selected = count - 1;
                    } else {
                        top.selected = top.selected.saturating_sub(1);
                    }
                    let is_theme = top.id == PickerId::ThemePicker;
                    let selected = top.selected;
                    if is_theme {
                        self.apply_theme_index(selected);
                    }
                }
                self.clamp_picker_selection();
                self.apply_eq_nav().await;
                self.apply_preset_preview();
                // Refresh picker preview cover for SearchLibrary when selection changes
                if self
                    .pickers
                    .top()
                    .is_some_and(|t| t.id == PickerId::SearchLibrary)
                {
                    self.update_picker_preview();
                }
                // Spotify search results can carry album art too: refresh the
                // preview when the highlighted row changes.
                if self
                    .pickers
                    .top()
                    .is_some_and(|t| t.id == PickerId::SpotifySearch)
                {
                    self.update_spot_preview();
                }
            }
            KeyCode::Down | KeyCode::Char('j') => {
                let has_input = matches!(
                    self.pickers.top().map(|o| o.id),
                    Some(PickerId::YTSearch)
                        | Some(PickerId::SearchLibrary)
                        | Some(PickerId::CommandPalette)
                        | Some(PickerId::ThemePicker)
                        | Some(PickerId::SpotifySearch)
                        | Some(PickerId::SpotifyLink)
                );
                let is_metadata = matches!(
                    self.pickers.top().map(|o| o.id),
                    Some(PickerId::EditMetadata)
                );
                if is_metadata {
                    if self.metadata.field_idx < 6 {
                        self.metadata.field_idx += 1;
                    }
                    return;
                }
                if has_input && key.code != KeyCode::Down {
                    // Add 'j' to the query instead of navigating
                    if let Some(top) = self.pickers.top_mut() {
                        top.query.push('j');
                    }
                    return;
                }
                let count = self.picker_item_count();
                if let Some(top) = self.pickers.top_mut() {
                    let max = count.saturating_sub(1);
                    if count > 0 && top.selected >= max {
                        top.selected = 0;
                    } else {
                        top.selected += 1;
                    }
                    let is_theme = top.id == PickerId::ThemePicker;
                    let selected = top.selected;
                    if is_theme {
                        self.apply_theme_index(selected);
                    }
                }
                self.clamp_picker_selection();
                self.apply_eq_nav().await;
                self.apply_preset_preview();
                // Refresh picker preview cover for SearchLibrary when selection changes
                if self
                    .pickers
                    .top()
                    .is_some_and(|t| t.id == PickerId::SearchLibrary)
                {
                    self.update_picker_preview();
                }
                if self
                    .pickers
                    .top()
                    .is_some_and(|t| t.id == PickerId::SpotifySearch)
                {
                    self.update_spot_preview();
                }
            }
            KeyCode::Enter if key.modifiers.contains(KeyModifiers::CONTROL) => {
                if matches!(
                    self.pickers.top().map(|o| o.id),
                    Some(PickerId::EditMetadata)
                ) {
                    if !self.metadata.edit_track_ids.is_empty() {
                        let title = self.metadata.fields[0].clone();
                        let artist = self.metadata.fields[1].clone();
                        let album = self.metadata.fields[2].clone();
                        let genre = self.metadata.fields[4].clone();
                        let year = self.metadata.fields[5].parse::<i32>().ok();
                        let track_number = self.metadata.fields[6].parse::<i32>().ok();
                        let ids = self.metadata.edit_track_ids.clone();
                        let client = self.client.clone();
                        let ipc_tx = self.ipc_tx.clone();
                        let _ = tx.send(TuiCommand::fire(move || async move {
                            let patch = MetadataPatch {
                                title: Some(title),
                                artist: Some(artist),
                                album: Some(album),
                                genre: Some(genre),
                                year,
                                track_number,
                                album_id: None,
                            };
                            for track_id in ids {
                                let _ = client
                                    .library()
                                    .update_metadata(track_id, patch.clone())
                                    .await;
                            }
                            let _ = ipc_tx.send(IpcResult::Notification(
                                "Library".to_string(),
                                "Metadata saved".to_string(),
                                NotificationKind::Success,
                                NotifType::Library,
                            ));
                        }));
                        self.metadata.cover = None;
                        self.metadata.cover_stateful = None;
                        self.metadata.edit_track_ids.clear();
                    }
                    self.pickers.close_top();
                }
            }
            KeyCode::Enter => {
                // Dispatch based on picker type
                if let Some(top) = self.pickers.top() {
                    match top.id {
                        PickerId::SpotifySearch => {
                            let not_linked = self.spotify.status.as_ref().is_none_or(|s| !s.linked);
                            if not_linked {
                                // No manual token-paste path: hand off to the
                                // client-id form (Enter there starts the
                                // browser OAuth flow, so the access token
                                // carries a refresh_token and playlists
                                // auto-sync).
                                self.close_picker();
                                self.open_spotify_link_form();
                            } else if self.spotify.search_results.is_empty() {
                                self.notify_typed(
                                    "System",
                                    "Type to search your synced Spotify playlists",
                                    NotificationKind::Info,
                                    true,
                                    NotifType::Spotify,
                                );
                            } else {
                                let picks = self.spot_picks();
                                if picks.is_empty() {
                                    return;
                                }
                                let idx = picks[top.selected.min(picks.len() - 1)];
                                let (playlist_id, _, track) =
                                    self.spotify.search_results[idx].clone();
                                let track_index = track.index;
                                let c = self.client.clone();
                                let ipc_tx = self.ipc_tx.clone();
                                self.pickers.close_top();
                                if playlist_id == "web" {
                                    match track.kind {
                                        Some(SpotifySearchKind::Album)
                                        | Some(SpotifySearchKind::Artist)
                                        | Some(SpotifySearchKind::Playlist) => {
                                            let c2 = c.clone();
                                            let ipc_tx2 = ipc_tx.clone();
                                            let uri = track.uri.clone().unwrap_or_default();
                                            let label = track.name.clone();
                                            let kind = track.kind;
                                            let _ = tx.send(TuiCommand::fire(move || async move {
                                                let result = match kind {
                                                    Some(SpotifySearchKind::Album) => {
                                                        c2.spotify().album_tracks(&uri).await
                                                    }
                                                    Some(SpotifySearchKind::Artist) => {
                                                        c2.spotify().artist_top_tracks(&uri).await
                                                    }
                                                    Some(SpotifySearchKind::Playlist) => {
                                                        c2.spotify().web_playlist_tracks(&uri).await
                                                    }
                                                    _ => Err(CoreError::Daemon(
                                                        "unsupported spotify result kind".into(),
                                                    )),
                                                };
                                                match result {
                                                    Ok(tracks) if tracks.is_empty() => {
                                                        let _ = ipc_tx2.send(IpcResult::Error(
                                                            format!(
                                                                "Spotify: no tracks for \
                                                                 '{label}'"
                                                            ),
                                                        ));
                                                    }
                                                    Ok(tracks) => {
                                                        for (n, t) in tracks.iter().enumerate() {
                                                            // Enter = play: the
                                                            // first track starts
                                                            // playback immediately
                                                            // (switching source);
                                                            // the rest queue behind.
                                                            let _ = c2
                                                                .spotify()
                                                                .resolve_track(
                                                                    &t.name,
                                                                    &t.artists,
                                                                    t.album
                                                                        .as_deref()
                                                                        .unwrap_or(""),
                                                                    t.uri.clone(),
                                                                    n == 0,
                                                                )
                                                                .await;
                                                        }
                                                    }
                                                    Err(e) => {
                                                        let _ = ipc_tx2.send(IpcResult::Error(
                                                            format!("Spotify resolve failed: {e}"),
                                                        ));
                                                    }
                                                }
                                            }));
                                        }
                                        _ => {
                                            let c2 = c.clone();
                                            let ipc_tx2 = ipc_tx.clone();
                                            let track_clone = track.clone();
                                            let _ = tx.send(TuiCommand::fire(move || async move {
                                                match c2
                                                    .spotify()
                                                    .resolve_track(
                                                        &track_clone.name,
                                                        &track_clone.artists,
                                                        track_clone.album.as_deref().unwrap_or(""),
                                                        track_clone.uri.clone(),
                                                        true,
                                                    )
                                                    .await
                                                {
                                                    Ok(()) => {}
                                                    Err(e) => {
                                                        let _ = ipc_tx2.send(IpcResult::Error(
                                                            format!("Spotify resolve failed: {e}"),
                                                        ));
                                                    }
                                                }
                                            }));
                                        }
                                    }
                                } else {
                                    let _ = tx.send(TuiCommand::fire(move || async move {
                                        match c
                                            .spotify()
                                            .resolve(&playlist_id, track_index, true)
                                            .await
                                        {
                                            Ok(()) => {}
                                            Err(e) => {
                                                let _ = ipc_tx.send(IpcResult::Error(format!(
                                                    "Spotify resolve failed: {e}"
                                                )));
                                            }
                                        }
                                    }));
                                }
                            }
                        }
                        PickerId::SpotifyLink => {
                            // An empty entry falls back to librespot's public
                            // desktop client id so no dashboard app is needed.
                            let client_id = self.spotify.link_input.trim().to_string();
                            let client_id = if client_id.is_empty() {
                                LIBRESPOT_CLIENT_ID.to_string()
                            } else {
                                client_id
                            };
                            let port = self
                                .spotify
                                .oauth_port
                                .trim()
                                .parse::<u16>()
                                .unwrap_or(8990);
                            // Keep the picker open and show a waiting state until
                            // the daemon reports the link completed.
                            self.start_spotify_oauth(client_id, port);
                        }
                        PickerId::Queue => {
                            if !self.queue.cache.is_empty() {
                                let idx = top.selected.min(self.queue.cache.len() - 1);
                                let path = self.queue.cache[idx].path.clone();
                                // Immediately update queue_cursor so the up-next preview
                                // reflects the newly selected track's next track.
                                self.queue.cursor = idx;
                                self.send_high(TuiCommand::Play(path));
                            }
                        }
                        PickerId::YTSearch => {
                            if top.query.is_empty() {
                                // Start search
                            } else if !self.yt_results_cache.is_empty() {
                                let idx = top.selected.min(self.yt_results_cache.len() - 1);
                                if self.yt_results_cache[idx].is_playlist {
                                    // Playlist drill-down: search using the playlist URL
                                    let url = self.yt_results_cache[idx].url.clone();
                                    if let Some(top) = self.pickers.top_mut() {
                                        top.query = url;
                                    }
                                    let query = self.yt_results_cache[idx].url.clone();
                                    let _ = tx.send(TuiCommand::YtSearch(query)).await;
                                } else {
                                    let url = self.yt_results_cache[idx].url.clone();
                                    let _ = tx.send(TuiCommand::YtResolve(url)).await;
                                }
                            } else {
                                // Initiate a new search
                                let query = top.query.clone();
                                let _ = tx.send(TuiCommand::YtSearch(query)).await;
                                let _ = tx.send(TuiCommand::RefreshYt).await;
                            }
                        }
                        PickerId::SleepTimer => {
                            // Handled by early return above
                        }
                        PickerId::Crossfade => {
                            let sel = top.selected;
                            // Rows: [0] "Duration" header, [1..=5] durations.
                            if (1..=5).contains(&sel) {
                                let dur = CROSSFADE_DURATIONS[sel - 1];
                                let enabled = self
                                    .state
                                    .crossfade
                                    .as_ref()
                                    .map(|c| c.enabled)
                                    .unwrap_or(true);
                                let tx = self.cmd_tx();
                                let _ = tx.send(TuiCommand::Crossfade(enabled, dur)).await;
                                if let Some(ref mut cf) = self.state.crossfade {
                                    cf.duration_secs = dur;
                                }
                                self.pickers.close_top();
                            }
                        }
                        PickerId::VisualizerPreset => {
                            let presets = VisualizerPreset::all();
                            if let Some(top) = self.pickers.top() {
                                let idx = top.selected.min(presets.len() - 1);
                                self.visualizer.preset = presets[idx];
                                save_prefs(&self.current_prefs());
                                self.footer_notification = Some((
                                    format!("Visualizer: {}", self.visualizer.preset.name()),
                                    std::time::Instant::now() + std::time::Duration::from_secs(2),
                                ));
                            }
                            self.pickers.close_top();
                        }
                        PickerId::ProgressStyle => {
                            let styles = ProgressStyle::all();
                            if let Some(top) = self.pickers.top() {
                                let idx = top.selected.min(styles.len() - 1);
                                self.progress_style = styles[idx];
                                save_prefs(&self.current_prefs());
                                self.notify_typed(
                                    "System",
                                    format!("Progress: {}", self.progress_style.name()),
                                    NotificationKind::Info,
                                    true,
                                    NotifType::Playback,
                                );
                            }
                            self.pickers.close_top();
                        }
                        PickerId::FooterPreset => {
                            if let Some(top) = self.pickers.top() {
                                let idx = top
                                    .selected
                                    .min(self.footer_presets.len().saturating_sub(1));
                                self.apply_preset_index(idx);
                                let name = self
                                    .footer_presets
                                    .get(self.footer_preset)
                                    .map(|p| p.name.to_string())
                                    .unwrap_or_else(|| "Default".into());
                                self.notify_typed(
                                    "System",
                                    format!("Footer preset: {name}"),
                                    NotificationKind::Info,
                                    true,
                                    NotifType::Playback,
                                );
                            }
                            self.pickers.close_top();
                        }
                        PickerId::NotificationSettings => {
                            self.cycle_notification_mode(1);
                        }
                        PickerId::CommandPalette => {
                            let commands = CommandPalette::commands(&self.icon_style);
                            let query = top.query.to_lowercase();
                            let filtered: Vec<&Command> = if query.is_empty() {
                                commands.iter().collect()
                            } else {
                                commands
                                    .iter()
                                    .filter(|c| !(c.keys.is_empty() && c.hint.is_empty()))
                                    .filter(|c| {
                                        let lower = c.icon.to_lowercase();
                                        let mut qi = 0usize;
                                        for ch in lower.chars() {
                                            if qi < query.len()
                                                && ch == query.as_bytes()[qi] as char
                                            {
                                                qi += 1;
                                            }
                                        }
                                        qi == query.len()
                                    })
                                    .collect()
                            };
                            let idx = top.selected.min(filtered.len().saturating_sub(1));
                            if let Some(cmd) = filtered.get(idx) {
                                // Dispatch on the stable action id, never on the
                                // display label, so the highlighted row's command
                                // is what actually runs.
                                let action = cmd.hint;
                                if action == "play/pause" {
                                    self.send_high(TuiCommand::PlayPause);
                                } else if action == "next track" {
                                    self.send_high(TuiCommand::Next);
                                } else if action == "prev track" {
                                    self.send_high(TuiCommand::Prev);
                                } else if action == "volume up" {
                                    let new_vol = (self.state.volume + 5).min(MAX_VOLUME);
                                    self.send_high(TuiCommand::SetVolume(new_vol));
                                } else if action == "volume down" {
                                    let new_vol = self.state.volume.saturating_sub(5);
                                    self.send_high(TuiCommand::SetVolume(new_vol));
                                } else if action == "mute" {
                                    self.send_high(TuiCommand::ToggleMute);
                                } else if action == "toggle mono" {
                                    self.send_high(TuiCommand::ToggleMono);
                                } else if action == "repeat" {
                                    let new_mode = match self.state.repeat {
                                        RepeatMode::Off => RepeatMode::One,
                                        RepeatMode::One => RepeatMode::All,
                                        RepeatMode::All => RepeatMode::Off,
                                    };
                                    self.send_high(TuiCommand::CycleRepeat(new_mode));
                                } else if action == "shuffle" {
                                    self.send_high(TuiCommand::ToggleShuffle);
                                } else if action == "quit daemon" {
                                    let c = self.client.clone();
                                    let _ =
                                        tokio::time::timeout(Duration::from_millis(1500), c.quit())
                                            .await;
                                    self.pending_quit = true;
                                } else if action == "quit" {
                                    self.pending_quit = true;
                                } else if action == "tab cycle" {
                                    self.cycle_pane_focus(true);
                                    self.pickers.close_top();
                                } else if action == "settings" {
                                    self.pickers.open(PickerId::Settings);
                                } else if action == "queue" {
                                    self.pickers.open(PickerId::Queue);
                                } else if action == "youtube" {
                                    self.pickers.open(PickerId::YTSearch);
                                } else if action == "search lib" {
                                    self.pickers.open(PickerId::SearchLibrary);
                                } else if action == "eq" {
                                    self.pickers.open(PickerId::Equalizer);
                                } else if action == "sleeptimer" {
                                    self.sleep_timer.focus = 0;
                                    self.pickers.open(PickerId::SleepTimer);
                                } else if action == "themepicker" {
                                    self.pickers.open_with_selection(
                                        PickerId::ThemePicker,
                                        self.theme_index,
                                    );
                                } else if action == "about" {
                                    self.pickers.open(PickerId::About);
                                } else if action == "notifications" {
                                    if let Some(ext) = overlay_extension(PickerId::Notifications)
                                        && self.extensions.is_disabled(ext)
                                    {
                                        self.notify(
                                            format!(
                                                "{} is an optional extension (disabled)",
                                                ext.label()
                                            ),
                                            NotificationKind::Info,
                                        );
                                    } else {
                                        self.pickers.open(PickerId::Notifications);
                                    }
                                } else if action == "search" {
                                    self.pickers.open(PickerId::SearchLibrary);
                                } else if action == "spotify" {
                                    self.pickers.open(PickerId::SpotifySearch);
                                } else if action == "fetch lyrics" {
                                    self.lyrics.show = true;
                                    self.send_high(TuiCommand::FetchLyrics);
                                } else if action == "progress style" {
                                    self.pickers.open(PickerId::ProgressStyle);
                                } else if action == "visualizer preset" {
                                    self.open_visualizer_picker();
                                } else if action == "visualizer" {
                                    if self.extensions.is_disabled(ExtensionId::Visualizer) {
                                        self.notify(
                                            format!(
                                                "{} is an optional extension (disabled)",
                                                ExtensionId::Visualizer.label()
                                            ),
                                            NotificationKind::Info,
                                        );
                                    } else {
                                        self.visualizer.toggle();
                                        let state = if self.visualizer.is_enabled() {
                                            "ON"
                                        } else {
                                            "OFF"
                                        };
                                        self.notify_typed(
                                            "System",
                                            format!("Visualizer: {}", state),
                                            NotificationKind::Info,
                                            true,
                                            NotifType::Playback,
                                        );
                                    }
                                } else if action == "stop" {
                                    self.send_high(TuiCommand::Stop);
                                } else if action == "seek forward" {
                                    let pos =
                                        (self.display_position + 5.0).min(self.state.duration);
                                    self.send_high(TuiCommand::Seek(pos));
                                } else if action == "seek backward" {
                                    let pos = (self.display_position - 5.0).max(0.0);
                                    self.send_high(TuiCommand::Seek(pos));
                                } else if action == "toggle favourite" {
                                    if let Some(ref track) = self.state.current_track {
                                        let track_id = track.id;
                                        let new_fav = !track.favourite;
                                        if let Some(ref mut ct) = self.state.current_track {
                                            ct.favourite = new_fav;
                                        }
                                        for t in &mut self.tracks_cache {
                                            if t.id == track_id {
                                                t.favourite = new_fav;
                                                break;
                                            }
                                        }
                                        let tx = self.cmd_tx();
                                        let _ = tx.send(TuiCommand::AddFavourite(track_id)).await;
                                        self.notify_titled(
                                            "Library",
                                            "Favourite toggled",
                                            NotificationKind::Info,
                                            true,
                                            NotifType::Playback,
                                        );
                                    }
                                } else if action == "clear queue" {
                                    let tx = self.cmd_tx();
                                    let _ = tx.send(TuiCommand::QueueClear).await;
                                    self.footer_notification = Some((
                                        "Queue cleared".to_string(),
                                        std::time::Instant::now()
                                            + std::time::Duration::from_secs(2),
                                    ));
                                } else if action == "love last.fm" {
                                    self.manage_lastfm_love();
                                } else if action == "toggle scrobbling" {
                                    self.toggle_scrobble_session();
                                } else if action == "prev tab" {
                                    self.cycle_pane_focus(false);
                                    self.pickers.close_top();
                                } else if action == "multiselect" {
                                    if !self.library_pane_focus {
                                        self.multiselect_mode = !self.multiselect_mode;
                                        if !self.multiselect_mode {
                                            self.clear_selection();
                                        }
                                        let msg = if self.multiselect_mode {
                                            "Multiselect ON: use v/a/x to queue"
                                        } else {
                                            "Multiselect OFF"
                                        };
                                        self.footer_notification = Some((
                                            msg.to_string(),
                                            std::time::Instant::now()
                                                + std::time::Duration::from_secs(2),
                                        ));
                                    }
                                } else if action == "add to queue" {
                                    if !self.library_pane_focus {
                                        // Single row: its play target. Batch:
                                        // the whole key-based selection.
                                        let targets =
                                            if self.multiselect_mode && self.selected_count() > 0 {
                                                self.selected_play_targets()
                                            } else {
                                                self.play_target_at(self.list_pos())
                                                    .into_iter()
                                                    .collect()
                                            };
                                        let mut added = 0;
                                        for target in targets {
                                            let c = self.client.clone();
                                            let _ = tx.send(TuiCommand::fire(move || async move {
                                                let _ = c.queue().add(&target, None).await;
                                            }));
                                            added += 1;
                                        }
                                        self.fetch_queue().await;
                                        self.notify_typed(
                                            "System",
                                            format!("Added {added} track(s) to queue"),
                                            NotificationKind::Info,
                                            false,
                                            NotifType::NowPlaying,
                                        );
                                    }
                                } else if action == "add to playlist" {
                                    if !self.library_pane_focus {
                                        let indices: Vec<i64> =
                                            if self.multiselect_mode && self.selected_count() > 0 {
                                                self.selected_library_ids()
                                            } else {
                                                self.filtered_tracks()
                                                    .get(self.list_pos())
                                                    .map(|t| vec![t.id])
                                                    .unwrap_or_default()
                                            };
                                        if !indices.is_empty() {
                                            self.pending_track_ids = indices;
                                            self.playlist_creating = false;
                                            self.pickers.open(PickerId::PlaylistSelect);
                                        } else if self.multiselect_mode {
                                            self.notify_typed(
                                                "System",
                                                "Selected tracks are streamed \u{2014} only library tracks can go into playlists",
                                                NotificationKind::Info,
                                                false,
                                                NotifType::NowPlaying,
                                            );
                                        } else if self.library_category == 12 {
                                            self.notify_typed(
                                                "System",
                                                "Chart tracks are streamed \u{2014} not in your library",
                                                NotificationKind::Info,
                                                false,
                                                NotifType::NowPlaying,
                                            );
                                        }
                                    }
                                } else if action == "delete from list" {
                                    if !self.library_pane_focus {
                                        if self.library_category == 12 {
                                            self.notify_typed(
                                                "System",
                                                "Charts are streamed \u{2014} not in your library",
                                                NotificationKind::Info,
                                                false,
                                                NotifType::NowPlaying,
                                            );
                                        } else if self.library_category == 4
                                            && self.browse_detail.is_some()
                                        {
                                            let filtered = self.filtered_tracks();
                                            if let Some(track) = filtered.get(self.list_pos()) {
                                                let track_id = track.id;
                                                if let Some(pl) =
                                                    self.playlist_cache.iter().find(|p| {
                                                        self.browse_detail.as_deref()
                                                            == Some(&p.name)
                                                    })
                                                {
                                                    let playlist_id = pl.id;
                                                    let tx = self.cmd_tx();
                                                    let _ = tx
                                                        .send(TuiCommand::RemoveFromPlaylist(
                                                            playlist_id,
                                                            track_id,
                                                        ))
                                                        .await;
                                                    self.notify_typed(
                                                        "System",
                                                        "Removed from playlist",
                                                        NotificationKind::Info,
                                                        false,
                                                        NotifType::NowPlaying,
                                                    );
                                                }
                                            }
                                        } else {
                                            self.notify_typed(
                                                "System",
                                                Self::PLAYLIST_VIEW_ONLY_REMOVE,
                                                NotificationKind::Info,
                                                false,
                                                NotifType::NowPlaying,
                                            );
                                        }
                                    }
                                } else if action == "jump to end" {
                                    if !self.library_pane_focus {
                                        let max = self.library_list_len().saturating_sub(1);
                                        self.set_list_pos(max);
                                    }
                                } else if action == "edit metadata" {
                                    if !self.library_pane_focus {
                                        if self.library_category == 12 {
                                            // Chart tracks are streamed — they
                                            // have no library metadata to edit.
                                            self.notify_typed(
                                                "System",
                                                "Chart tracks are streamed \u{2014} not in your library",
                                                NotificationKind::Info,
                                                false,
                                                NotifType::NowPlaying,
                                            );
                                        } else {
                                            let track_data = {
                                                let tracks = self.filtered_tracks();
                                                tracks.get(self.list_pos()).map(|t| {
                                                    (
                                                        t.id,
                                                        t.title.clone(),
                                                        t.artist.clone(),
                                                        t.album.clone(),
                                                        t.genre.clone(),
                                                        t.year,
                                                        t.track_number,
                                                    )
                                                })
                                            };
                                            if let Some((
                                                id,
                                                title,
                                                artist,
                                                album,
                                                genre,
                                                year,
                                                track_num,
                                            )) = track_data
                                            {
                                                self.metadata.edit_track_ids = vec![id];
                                                self.metadata.fields = [
                                                    title,
                                                    artist,
                                                    album,
                                                    String::new(),
                                                    genre,
                                                    year.map_or(String::new(), |y| y.to_string()),
                                                    track_num
                                                        .map_or(String::new(), |n| n.to_string()),
                                                ];
                                                self.metadata.field_idx = 0;
                                                self.pickers.open(PickerId::EditMetadata);
                                                self.fetch_metadata_cover();
                                            }
                                        }
                                    }
                                } else if action == "toggle help" {
                                    if self.pickers.top().is_some_and(|o| o.id == PickerId::Help) {
                                        self.pickers.close_top();
                                    } else {
                                        self.pickers.open(PickerId::Help);
                                    }
                                } else if action == "hide help bar" {
                                    self.hide_help_bar = !self.hide_help_bar;
                                } else if action == "health check" {
                                    self.send_high(TuiCommand::CheckHealth);
                                } else if action == "setup" {
                                    self.open_setup_picker(None);
                                } else if action == "radio browse" {
                                    self.pickers.open(PickerId::Radio);
                                    self.on_picker_opened(PickerId::Radio);
                                } else if action == "play stream url" {
                                    self.pickers.open(PickerId::LoadStream);
                                }
                            }
                            // If the action opened a sub-picker it was stacked on
                            // top of the palette; leave it open.  Otherwise the
                            // palette closes.
                            if self
                                .pickers
                                .top()
                                .is_some_and(|o| o.id == PickerId::CommandPalette)
                            {
                                self.pickers.close_top();
                            }
                        }
                        PickerId::PlaylistSelect => {
                            if self.playlist_creating {
                                let name = top.query.trim().to_string();
                                if name.is_empty() {
                                    // Keep the name input open until a name is given.
                                } else if let Some(rename_id) = self.renaming_playlist.take() {
                                    let client = self.client.clone();
                                    let ipc_tx = self.ipc_tx.clone();
                                    let _ = tx.send(TuiCommand::fire(move || async move {
                                        match client
                                            .library()
                                            .rename_playlist(rename_id, &name)
                                            .await
                                        {
                                            Ok(()) => {
                                                if let Ok(DaemonRes::Playlists {
                                                    playlists, ..
                                                }) = client.library().get_playlists().await
                                                {
                                                    let _ = ipc_tx
                                                        .send(IpcResult::Playlists(playlists));
                                                }
                                                let _ = ipc_tx.send(IpcResult::Notification(
                                                    "Playlist".to_string(),
                                                    format!("Renamed playlist — {name}"),
                                                    NotificationKind::Success,
                                                    NotifType::NowPlaying,
                                                ));
                                            }
                                            Err(e) => {
                                                let _ = ipc_tx.send(IpcResult::Error(format!(
                                                    "Failed to rename playlist: {e}"
                                                )));
                                            }
                                        }
                                    }));
                                    self.playlist_creating = false;
                                    self.close_picker();
                                } else {
                                    let client = self.client.clone();
                                    let ipc_tx = self.ipc_tx.clone();
                                    let _ = tx.send(TuiCommand::fire(move || async move {
                                        match client.library().create_playlist(&name).await {
                                            Ok(playlists) => {
                                                if let Some(new_p) = playlists.first().cloned() {
                                                    let playlists =
                                                        client.library().get_playlists().await;
                                                    if let Ok(DaemonRes::Playlists {
                                                        playlists,
                                                        ..
                                                    }) = playlists
                                                    {
                                                        let _ = ipc_tx
                                                            .send(IpcResult::Playlists(playlists));
                                                    }
                                                    let _ =
                                                        ipc_tx.send(IpcResult::PlaylistCreated(
                                                            new_p.id,
                                                            name.clone(),
                                                        ));
                                                    let _ = ipc_tx.send(IpcResult::Notification(
                                                        "Playlist".to_string(),
                                                        format!("Created {name} — pick tracks"),
                                                        NotificationKind::Success,
                                                        NotifType::NowPlaying,
                                                    ));
                                                }
                                            }
                                            Err(e) => {
                                                let _ = ipc_tx.send(IpcResult::Error(format!(
                                                    "Failed to create playlist: {e}"
                                                )));
                                            }
                                        }
                                    }));
                                    self.playlist_creating = false;
                                    self.close_picker();
                                }
                            } else if top.selected == 0 {
                                self.playlist_creating = true;
                            } else {
                                let idx = top.selected.saturating_sub(1);
                                let track_ids = self.pending_track_ids.clone();
                                if let Some(pl) = self.playlist_cache.get(idx) {
                                    let playlist_id = pl.id;
                                    if !track_ids.is_empty() {
                                        let client = self.client.clone();
                                        let _ = tx.send(TuiCommand::fire(move || async move {
                                            let _ = client
                                                .library()
                                                .add_to_playlist(playlist_id, track_ids)
                                                .await;
                                        }));
                                        self.notify_titled(
                                            "Playlist",
                                            "Added to playlist",
                                            NotificationKind::Success,
                                            true,
                                            NotifType::NowPlaying,
                                        );
                                    }
                                    self.close_picker();
                                }
                            }
                        }
                        PickerId::Equalizer => {
                            // Apply selected EQ preset
                            let idx = top.selected.min(EQ_PRESETS.len() - 1);
                            let c = self.client.clone();
                            let preset = EQ_PRESETS[idx];
                            let _ = tx.send(TuiCommand::fire(move || async move {
                                let _ = c.set_eq_preset(preset).await;
                            }));
                            self.footer_notification = Some((
                                format!("Equalizer preset: {}", preset.label()),
                                std::time::Instant::now() + std::time::Duration::from_secs(2),
                            ));
                            self.pickers.close_top();
                        }
                        PickerId::ThemePicker => {
                            let idx = top.selected;
                            self.apply_theme_index(idx);
                            let name = &self.themes[idx].name;
                            let light = if self.themes[idx].light {
                                " (light)"
                            } else {
                                ""
                            };
                            self.notify_titled(
                                "Theme",
                                format!("Theme: {}{}", name, light),
                                NotificationKind::Info,
                                true,
                                NotifType::Prefs,
                            );
                            self.pickers.close_top();
                        }
                        PickerId::SearchLibrary => {
                            let picks = self.search_library_picks();
                            if !picks.is_empty() {
                                let idx = top.selected.min(picks.len() - 1);
                                match &picks[idx] {
                                    LibraryPick::Track(i) => {
                                        let path = self.tracks_cache[*i].path.clone();
                                        self.send_high(TuiCommand::Play(path));
                                    }
                                    LibraryPick::Artist(name) => {
                                        self.reset_library_view(3, Some(name.clone()));
                                        self.library_pane_focus = false;
                                    }
                                    LibraryPick::Album(album) => {
                                        self.reset_library_view(2, Some(album.clone()));
                                        self.library_pane_focus = false;
                                    }
                                    LibraryPick::Playlist(i) => {
                                        let playlist = self.playlist_cache[*i].clone();
                                        self.reset_library_view(4, Some(playlist.name.clone()));
                                        self.library_pane_focus = false;
                                        let c = self.client.clone();
                                        let ipc_tx2 = self.ipc_tx.clone();
                                        let pid = playlist.id;
                                        let _ = tx.send(TuiCommand::fire(move || async move {
                                            if let Ok(DaemonRes::Tracks { tracks }) =
                                                c.library().get_playlist_tracks(pid).await
                                            {
                                                let _ = ipc_tx2
                                                    .send(IpcResult::PlaylistTracks(*tracks));
                                            }
                                        }));
                                    }
                                    LibraryPick::Radio(i) => {
                                        if let Some(station) = self.radio.custom.get(*i).cloned() {
                                            let id = match station.uuid.as_deref() {
                                                Some(uuid) => uuid.to_string(),
                                                None => format!("custom:{}", i + 1),
                                            };
                                            let c = self.client.clone();
                                            self.pickers.close_top();
                                            let _ = tx.send(TuiCommand::fire(move || async move {
                                                let _ = c.radio().play(&id, &station.name).await;
                                            }));
                                        }
                                    }
                                }
                            }
                            self.pickers.close_top();
                        }
                        PickerId::EditMetadata => {
                            if self.metadata.field_idx < 6 {
                                self.metadata.field_idx += 1;
                            } else {
                                if !self.metadata.edit_track_ids.is_empty() {
                                    let title = self.metadata.fields[0].clone();
                                    let artist = self.metadata.fields[1].clone();
                                    let album = self.metadata.fields[2].clone();
                                    let genre = self.metadata.fields[4].clone();
                                    let year = self.metadata.fields[5].parse::<i32>().ok();
                                    let track_number = self.metadata.fields[6].parse::<i32>().ok();
                                    let ids = self.metadata.edit_track_ids.clone();
                                    let client = self.client.clone();
                                    let ipc_tx = self.ipc_tx.clone();
                                    let _ = tx.send(TuiCommand::fire(move || async move {
                                        let patch = MetadataPatch {
                                            title: Some(title),
                                            artist: Some(artist),
                                            album: Some(album),
                                            genre: Some(genre),
                                            year,
                                            track_number,
                                            album_id: None,
                                        };
                                        for track_id in ids {
                                            let _ = client
                                                .library()
                                                .update_metadata(track_id, patch.clone())
                                                .await;
                                        }
                                        let _ = ipc_tx.send(IpcResult::Notification(
                                            "Library".to_string(),
                                            "Metadata saved".to_string(),
                                            NotificationKind::Success,
                                            NotifType::Library,
                                        ));
                                    }));
                                    self.metadata.cover = None;
                                    self.metadata.cover_stateful = None;
                                    self.metadata.edit_track_ids.clear();
                                }
                                self.pickers.close_top();
                            }
                        }
                        _ => {}
                    }
                }
            }
            KeyCode::Char('d') if key.modifiers == KeyModifiers::CONTROL => {
                // YT search: download selected result
                let idx = self.pickers.top().map_or(0, |o| {
                    o.selected
                        .min(self.yt_results_cache.len().saturating_sub(1))
                });
                if !self.yt_results_cache.is_empty() {
                    let result = &self.yt_results_cache[idx];
                    let url = result.url.clone();
                    let title = Some(result.title.clone());
                    let artist = result
                        .artist
                        .clone()
                        .or_else(|| Some(result.channel.clone()));
                    let _ = tx.send(TuiCommand::YtDownload { url, title, artist }).await;
                }
            }
            KeyCode::Char('a') if key.modifiers == KeyModifiers::CONTROL => {
                // YT search: add selected result to queue
                let idx = self.pickers.top().map_or(0, |o| {
                    o.selected
                        .min(self.yt_results_cache.len().saturating_sub(1))
                });
                if !self.yt_results_cache.is_empty() {
                    let url = self.yt_results_cache[idx].url.clone();
                    if self.yt_results_cache[idx].is_playlist {
                        let _ = tx.send(TuiCommand::YtResolve(url)).await;
                    } else {
                        let _ = tx.send(TuiCommand::QueueAdd(url)).await;
                    }
                }
            }
            KeyCode::Char('s') if key.modifiers == KeyModifiers::CONTROL => {
                // Edit Metadata: sync cover using the currently-entered metadata. Only
                // meaningful for single-track edits (batch rows preview the
                // first track's cover but sync it individually).
                if top_id == Some(PickerId::EditMetadata)
                    && let Some(&track_id) = self.metadata.edit_track_ids.first()
                {
                    let title = self.metadata.fields[0].clone();
                    let artist = self.metadata.fields[1].clone();
                    let album = self.metadata.fields[2].clone();
                    let genre = self.metadata.fields[4].clone();
                    let year = self.metadata.fields[5].parse::<i32>().ok();
                    let track_number = self.metadata.fields[6].parse::<i32>().ok();
                    let fetch_gen = self.next_cover_gen();
                    self.metadata.cover_fetch_gen = Some(fetch_gen);
                    let client = self.client.clone();
                    let ipc_tx = self.ipc_tx.clone();
                    let _ = tx.send(TuiCommand::fire(move || async move {
                        let patch = MetadataPatch {
                            title: Some(title),
                            artist: Some(artist),
                            album: Some(album),
                            genre: Some(genre),
                            year,
                            track_number,
                            album_id: None,
                        };
                        // Persist edited metadata first so the cover lookup
                        // uses the updated artist/album/title.
                        let _ = client.library().update_metadata(track_id, patch).await;
                        if let Ok(Some(b64)) = client.art().cover(track_id).await
                            && let Ok(bytes) =
                                base64::engine::general_purpose::STANDARD.decode(&b64)
                        {
                            let _ = ipc_tx.send(IpcResult::MetadataCoverArt(
                                Some(bytes),
                                track_id,
                                fetch_gen,
                            ));
                        }
                    }));
                    self.metadata.cover_dirty = true;
                    self.notify_titled(
                        "Library",
                        "Syncing cover…",
                        NotificationKind::Info,
                        true,
                        NotifType::Library,
                    );
                }
            }
            KeyCode::Char(c) if !ctrl_or_alt => {
                if let Some(top) = self.pickers.top_mut() {
                    match top.id {
                        PickerId::YTSearch
                        | PickerId::SearchLibrary
                        | PickerId::CommandPalette
                        | PickerId::ThemePicker => {
                            top.query.push(c);
                            if top.id == PickerId::YTSearch {
                                // Invalidate stale results immediately so the
                                // picker never shows results from an older query.
                                self.yt_results_cache.clear();
                                self.yt_search_loading = false;
                                self.yt_search_debounce =
                                    Some(std::time::Instant::now() + Duration::from_millis(500));
                            }
                        }
                        PickerId::EditMetadata => {
                            self.metadata.fields[self.metadata.field_idx].push(c);
                        }
                        PickerId::SpotifySearch => {
                            if self.spotify.status.as_ref().is_none_or(|s| !s.linked) {
                                // Unlinked: this picker only runs the browser
                                // OAuth flow, so typed input is ignored (Enter
                                // (re)starts the flow, Esc cancels).
                            } else {
                                top.query.push(c);
                                // Invalidate stale results immediately and
                                // re-search after the debounce elapses.
                                self.spotify.search_results.clear();
                                self.spotify.web_seq = self.spotify.web_seq.wrapping_add(1);
                                self.spotify.search_debounce = Some(
                                    std::time::Instant::now()
                                        + Duration::from_millis(SEARCH_DEBOUNCE_MS),
                                );
                            }
                        }
                        PickerId::SpotifyLink => {
                            if self.spotify.link_field == 0 {
                                self.spotify.link_input.push(c);
                            } else {
                                self.spotify.oauth_port.push(c);
                            }
                        }
                        PickerId::PlaylistSelect if self.playlist_creating => {
                            top.query.push(c);
                        }
                        PickerId::PlaylistSelect if c == 'n' || c == 'N' => {
                            top.query.clear();
                            self.playlist_creating = true;
                        }
                        _ => {}
                    }
                }
            }
            KeyCode::Tab => {
                if let Some(top) = self.pickers.top_mut() {
                    if matches!(top.id, PickerId::SearchLibrary | PickerId::SpotifySearch) {
                        // Both search pickers share the same filter model, so
                        // Tab narrows results the same way in either.
                        top.source = top.source.next();
                        top.selected = 0;
                        top.viewport_offset = 0;
                        self.picker_preview_cover = None;
                        self.picker_preview_stateful = None;
                        self.picker_slot.id = None;
                        self.artist_cover = None;
                        self.artist_cover_stateful = None;
                        self.artist_slot.id = None;
                        self.spotify.preview_fetch.id = None;
                        self.spotify.preview_fetch.version = None;
                    } else if top.id == PickerId::EditMetadata {
                        self.metadata.field_idx = (self.metadata.field_idx + 1) % 7;
                    } else if top.id == PickerId::SpotifyLink {
                        self.spotify.link_field = (self.spotify.link_field + 1) % 2;
                    }
                }
            }
            KeyCode::Backspace => {
                if let Some(top) = self.pickers.top_mut() {
                    match top.id {
                        PickerId::EditMetadata => {
                            self.metadata.fields[self.metadata.field_idx].pop();
                        }
                        PickerId::SpotifySearch => {
                            if self.spotify.status.as_ref().is_none_or(|s| !s.linked) {
                                // Unlinked: no manual token input anymore.
                            } else {
                                top.query.pop();
                                self.spotify.search_results.clear();
                                self.spotify.web_seq = self.spotify.web_seq.wrapping_add(1);
                                self.spotify.search_debounce = Some(
                                    std::time::Instant::now()
                                        + Duration::from_millis(SEARCH_DEBOUNCE_MS),
                                );
                            }
                        }
                        PickerId::SpotifyLink => {
                            if self.spotify.link_field == 0 {
                                self.spotify.link_input.pop();
                            } else {
                                self.spotify.oauth_port.pop();
                            }
                        }
                        PickerId::PlaylistSelect if self.playlist_creating => {
                            top.query.pop();
                        }
                        _ => {
                            top.query.pop();
                            if top.id == PickerId::YTSearch {
                                self.yt_results_cache.clear();
                                self.yt_search_loading = false;
                                self.yt_search_debounce =
                                    Some(std::time::Instant::now() + Duration::from_millis(500));
                            }
                        }
                    }
                }
            }
            _ => {}
        }
    }

    pub(crate) async fn apply_eq_nav(&mut self) {
        if let Some(top) = self.pickers.top()
            && top.id == PickerId::Equalizer
        {
            let idx = top.selected.min(EQ_PRESETS.len() - 1);
            self.send_high(TuiCommand::SetEqPreset(EQ_PRESETS[idx]));
            self.state.audio.eq_preset = EQ_PRESETS[idx];
        }
    }

    pub(crate) fn apply_preset_preview(&mut self) {
        if let Some(top) = self.pickers.top() {
            match top.id {
                PickerId::VisualizerPreset => {
                    let presets = VisualizerPreset::all();
                    let idx = top.selected.min(presets.len() - 1);
                    self.visualizer.preset = presets[idx];
                }
                PickerId::ProgressStyle => {
                    let styles = ProgressStyle::all();
                    let idx = top.selected.min(styles.len() - 1);
                    self.progress_style = styles[idx];
                }
                PickerId::FooterPreset => {
                    let idx = top
                        .selected
                        .min(self.footer_presets.len().saturating_sub(1));
                    self.footer_preset = idx;
                    self.footer_cache.suppress_refresh = false;
                }
                _ => {}
            }
        }
    }
}
