use crate::app::*;
use crate::oauth::mask_credential;

/// Validate the link form's redirect port.
///
/// A port that does not parse is a mistake, and it must be reported rather
/// than discarded: the client id and the port sit one Tab apart, so a client id
/// pasted into the port box used to fall back to the default and link the
/// account against the shared app while looking like the id had been honoured.
/// Returning the error keeps the form on screen to correct.
///
/// Blank is not a mistake — the default is a legitimate answer to "I did not
/// change this".
pub(crate) fn parse_oauth_port(raw: &str) -> Result<u16, String> {
    match raw.trim() {
        "" => Ok(8990),
        text => text
            .parse::<u16>()
            .map_err(|_| "Port must be a number. Tab switches to the client id.".to_string()),
    }
}

impl App {
    pub fn open_setup_picker(&mut self, service: Option<&str>) {
        match service.map(|s| s.trim().to_ascii_lowercase()).as_deref() {
            Some("spotify") => {
                self.setup.selection = 0;
                self.open_spot_link();
            }
            Some("lastfm" | "last.fm") => {
                self.setup.selection = 1;
                self.pickers.open(PickerId::LastfmAuth);
                self.on_picker_opened(PickerId::LastfmAuth);
            }
            Some("youtube") | Some("yt") => {
                self.setup.selection = 2;
                self.pickers.open(PickerId::YoutubeSetup);
                self.on_picker_opened(PickerId::YoutubeSetup);
            }
            Some("discord") => {
                self.setup.selection = 3;
                self.pickers.open(PickerId::DiscordSetup);
                self.on_picker_opened(PickerId::DiscordSetup);
            }
            _ => {
                self.pickers.open(PickerId::Setup);
                self.on_picker_opened(PickerId::Setup);
            }
        }
    }

    /// Open the SpotifyLink picker. This does NOT start the OAuth flow: the
    /// screen renders the client id and redirect port and waits for Enter, which
    /// is what starts it.
    ///
    /// The client id field is left as-is rather than reset, so a user who
    /// already supplied one does not retype it on every re-link — the id
    /// persists on disk anyway, and this only avoids a pointless retype when
    /// the form is reopened in the same session.
    pub fn open_spot_link(&mut self) {
        self.spotify.oauth_port = "8990".to_string();
        self.spotify.oauth_field = 0;
        self.spotify.oauth_form_error = None;
        // No pending flow: the picker shows the input form, not a waiting view.
        self.spotify.oauth_pending = false;
        self.spotify.oauth_url = None;
        self.spotify.oauth_error = None;
        self.pickers.open(PickerId::SpotifyLink);
    }

    /// Check if config.toml was modified since last load; if so, re-parse
    /// and apply hot-reloadable settings (theme, transparent_bg,
    /// progress_style, visualizer_preset, keybindings).
    pub(crate) fn check_config_reload(&mut self) {
        let path = prefs_path();
        let mtime = match std::fs::metadata(&path).and_then(|m| m.modified()) {
            Ok(t) => t,
            Err(_) => return,
        };
        if self.last_config_mtime == Some(mtime) {
            return;
        }
        self.last_config_mtime = Some(mtime);
        let prefs = load_prefs();

        // Theme
        let theme_index = resolve_theme_index(&self.themes, &prefs.theme_name, &prefs.theme_mode);
        self.theme_index = theme_index;
        self.theme_mode = if prefs.theme_mode.is_empty() {
            default_theme_mode()
        } else {
            prefs.theme_mode.clone()
        };
        self.apply_reactive();

        // Transparent bg
        self.transparent_bg = prefs.transparent_bg;
        self.transparent_pickers = prefs.transparent_pickers;
        self.reactive_theme = prefs.reactive_theme;
        self.reactive_theme_intensity = prefs.reactive_theme_intensity;
        // Reactive palette may be retained from before a reload; re-derive
        // the theme so a changed intensity/wash applies immediately.
        self.apply_reactive();

        self.footer_cache.suppress_refresh = true;

        // Footer preset
        let footer_preset = self
            .footer_presets
            .iter()
            .position(|p| p.name == prefs.footer_preset_name)
            .unwrap_or(0)
            .min(self.footer_presets.len().saturating_sub(1));
        self.footer_preset = footer_preset;

        // Progress style
        self.progress_style = prefs.progress_style;

        // Visualizer preset
        self.visualizer.preset = prefs.visualizer_preset;
        self.daydream_secs = prefs.daydream_secs;

        // Footer time format
        self.footer_time_format = if prefs.time_format.is_empty() {
            default_time_format()
        } else {
            prefs.time_format.clone()
        };

        // Keybindings
        self.prefs_keybindings = prefs.keybindings.clone();
        self.keybindings = build_keybindings(&prefs.keybindings);

        // Per-type notification modes (defaults used for types not present in
        // an older config file).
        self.notification_modes.clear();
        for (k, v) in &prefs.notification_modes {
            let t = NotifType::from_str_lossy(k);
            let m = NotifMode::from_str_lossy(v);
            self.notification_modes.insert(t, m);
        }

        // Hide footer
        self.hide_footer = prefs.hide_footer;

        // Left-pane lists + preview toggle (sanitized: unknown names dropped,
        // empty falls back to the full set; active category clamped in).
        self.left_pane_lists = clean_left_pane(&prefs.left_pane_lists);
        self.show_preview = prefs.show_preview;
        if !self
            .visible_library_indices()
            .contains(&self.library_category)
            && let Some(&first) = self.visible_library_indices().first()
        {
            self.library_category = first;
        }
    }

    /// Open the visualizer preset picker, refusing when the visualizer
    /// extension is disabled. Shared by the palette entry and keybindings.
    pub(crate) fn open_visualizer_picker(&mut self) {
        if self.extensions.is_disabled(ExtensionId::Visualizer) {
            self.notify(
                format!(
                    "{} is an optional extension (disabled)",
                    ExtensionId::Visualizer.label()
                ),
                NotificationKind::Info,
            );
            return;
        }
        self.pickers.open(PickerId::VisualizerPreset);
        self.dismiss_track_popup();
        self.on_picker_opened(PickerId::VisualizerPreset);
    }

    /// Open the notification settings overlay, refusing when the overlay
    /// extension is disabled.
    pub(crate) fn open_settings_overlay(&mut self) {
        if self
            .extensions
            .is_disabled(ExtensionId::NotificationOverlay)
        {
            self.notify(
                format!(
                    "{} is an optional extension (disabled)",
                    ExtensionId::NotificationOverlay.label()
                ),
                NotificationKind::Info,
            );
            return;
        }
        self.pickers.open(PickerId::NotificationSettings);
        self.dismiss_track_popup();
        self.on_picker_opened(PickerId::NotificationSettings);
        self.refresh_cover_stat();
    }

    /// Accumulate a seek delta from a repeated (held) key press. Updates the
    /// local position estimate immediately for smooth feedback and only
    /// defers the authoritative daemon seek to `ensure_seek_flush`, so a
    /// long-press never floods the daemon with full re-decodes.
    pub(crate) fn accumulate_seek(&mut self, delta: f64) {
        if self.state.current_track.is_none() {
            return;
        }
        let raw = self.seek_cmd_accum.unwrap_or(self.raw_position) + delta;
        self.seek_cmd_accum = Some(raw);
        self.last_seek_press = Some(std::time::Instant::now());
        let clamped = raw.clamp(0.0, self.state.duration.max(0.0));
        // Immediate local feedback; the daemon catches up on flush.
        self.raw_position = clamped;
        self.display_position = clamped;
        self.seek_pending = Some(std::time::Instant::now());
    }

    /// Send the coalesced seek to the daemon once the user stops hammering the
    /// seek key (or a full command seek was requested). Called every frame.
    pub(crate) fn ensure_seek_flush(&mut self) {
        let Some(accum) = self.seek_cmd_accum else {
            return;
        };
        // Flush once no repeat press has arrived for a short window.
        let idle = self
            .last_seek_press
            .map(|t| t.elapsed())
            .unwrap_or(std::time::Duration::ZERO)
            >= std::time::Duration::from_millis(160);
        if idle {
            let pos = accum.clamp(0.0, self.state.duration.max(0.0));
            self.send_high(TuiCommand::Seek(pos));
            self.seek_cmd_accum = None;
            self.last_seek_press = None;
        }
    }

    pub async fn run(
        mut self,
        terminal: &mut Terminal<ratatui::backend::CrosstermBackend<std::io::Stdout>>,
    ) -> Result<(), Box<dyn std::error::Error>> {
        // All initial IPC is done in background tasks so the TUI renders
        // immediately with an empty state. Results arrive via ipc_rx and
        // are applied on the next loop iteration.
        {
            let c = self.client.clone();
            let ipc_tx = self.ipc_tx.clone();
            tokio::spawn(async move {
                if let Ok(state) = c.get_status().await {
                    let _ = ipc_tx.send(IpcResult::RefreshDone(Box::new(state), None, None));
                }
            });
        }
        {
            let c = self.client.clone();
            let ipc_tx = self.ipc_tx.clone();
            tokio::spawn(async move {
                if let Ok(DaemonRes::QueueState {
                    queue: tracks,
                    cursor,
                    ..
                }) = c.queue().list().await
                {
                    let _ = ipc_tx.send(IpcResult::Queue(*tracks, cursor as usize));
                }
            });
        }
        {
            let c = self.client.clone();
            let ipc_tx = self.ipc_tx.clone();
            tokio::spawn(async move {
                if let Ok(DaemonRes::Tracks { tracks, .. }) =
                    c.library().get_tracks(None, None).await
                    && !tracks.is_empty()
                {
                    let _ = ipc_tx.send(IpcResult::LibraryTracks(*tracks));
                }
            });
        }
        {
            let c = self.client.clone();
            let ipc_tx = self.ipc_tx.clone();
            tokio::spawn(async move {
                let (status, playlists) =
                    tokio::join!(async { c.spotify().status().await.ok() }, async {
                        c.spotify().playlists().await.ok()
                    });
                if let Some(status) = status {
                    let _ = ipc_tx.send(IpcResult::SpotifyStatus(status));
                }
                if let Some(playlists) = playlists {
                    let _ = ipc_tx.send(IpcResult::SpotifyPlaylists(playlists));
                }
            });
        }
        {
            let c = self.client.clone();
            let ipc_tx = self.ipc_tx.clone();
            tokio::spawn(async move {
                if let Ok(report) = c.check_health().await {
                    let _ = ipc_tx.send(IpcResult::HealthReport(report));
                }
            });
        }
        {
            let c = self.client.clone();
            let ipc_tx = self.ipc_tx.clone();
            tokio::spawn(async move {
                if let Ok(DaemonRes::Playlists { playlists, .. }) =
                    c.library().get_playlists().await
                {
                    let _ = ipc_tx.send(IpcResult::Playlists(playlists));
                }
            });
        }

        // Initialize cover image picker in background (blocking terminal query).
        {
            let ipc_tx = self.ipc_tx.clone();
            tokio::spawn(async move {
                let picker = tokio::task::spawn_blocking(|| Picker::from_query_stdio().ok())
                    .await
                    .unwrap_or(None);
                let _ = ipc_tx.send(IpcResult::CoverPicker(picker));
            });
        }

        self.is_ready = true;

        // Seed the track-info popup so the first row's details are visible
        // immediately when the middle pane starts focused.
        self.update_track_popup();

        // Animate the initial frame so the library list and Now Playing pane
        // evolve into view on startup.
        self.track_anim_trigger = true;

        // Render the initial frame immediately, before the main loop, so
        // the user never sees a blank alternate screen on startup.
        let _ = terminal.draw(|f| ui::render(f, &mut self));

        let cmd_tx = self.cmd_tx();
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(Duration::from_millis(1000)).await;
                let _ = cmd_tx.try_send(TuiCommand::Refresh);
            }
        });

        'outer: loop {
            // Input-first: process any already-buffered terminal events before
            // state work and rendering so key actions are applied immediately.
            while event::poll(Duration::ZERO).unwrap_or(false) {
                match event::read() {
                    Ok(ev) => {
                        if !self.handle_terminal_event(ev).await {
                            break 'outer;
                        }
                    }
                    Err(_) => break,
                }
            }
            let mut events_received = false;
            let mut had_track_change = false;
            let mut had_sleep_expired = false;
            let mut had_sync_done = false;
            let mut had_spotify_change = false;
            let mut had_lastfm_change = false;
            for ev in self.client.drain().await {
                if let DaemonEvent::PlaybackStarted { .. } = &ev {
                    // Was this change an automatic advance (not a manual
                    // Next/Prev)? Capture that before the flag is reset so the
                    // dust animation is only shown on genuine auto-advances.
                    self.auto_track_advance = !self.manual_track_advance;
                    // Any PlaybackStarted consumes the manual-advance flag.
                    self.manual_track_advance = false;
                }
                if matches!(ev, DaemonEvent::PlaybackStarted { .. }) {
                    had_track_change = true;
                }
                if matches!(ev, DaemonEvent::SleepTimerExpired) {
                    had_sleep_expired = true;
                }
                // The visualizer is a configurable extension: when disabled
                // the spectrum + waveform streams are zeroed on the client so
                // the IPC pipe is effectively idle from the app's perspective.
                if self.extensions.is_disabled(ExtensionId::Visualizer)
                    && matches!(
                        ev,
                        DaemonEvent::SpectrumChanged { .. } | DaemonEvent::WaveformChanged { .. }
                    )
                {
                    self.state.audio_levels.clear();
                    self.state.wave_samples.clear();
                } else {
                    self.state.apply_event(&ev);
                }
                // The daemon finished an OAuth link flow: pull the fresh
                // status + playlists so they appear without a restart.
                if matches!(ev, DaemonEvent::SpotifyStatusChanged) {
                    had_spotify_change = true;
                }
                // The daemon finished the (daemon-hosted) Last.fm link flow —
                // same completion contract as Spotify: picking up the status
                // here dismisses the prompt and proceeds to a ready state.
                if matches!(ev, DaemonEvent::LastfmStatusChanged) {
                    had_lastfm_change = true;
                }
                // After a background metadata sync finishes, re-pull the
                // library so scrubbed tags / fetched covers show up live.
                if let DaemonEvent::Custom { name, data } = &ev
                    && name == "sleep_timer_deferred"
                {
                    // The timer expired with "stop immediately" unchecked: the
                    // daemon keeps playing until the current track ends. Clear
                    // the countdown and surface the deferral in the footer.
                    self.sleep_timer.remaining = None;
                    let note = data
                        .get("note")
                        .cloned()
                        .unwrap_or_else(|| "Stopping at end of current track".into());
                    self.footer_notification = Some((
                        note,
                        std::time::Instant::now() + std::time::Duration::from_secs(3),
                    ));
                }
                if let DaemonEvent::ProviderError { provider, message } = &ev {
                    // A provider that refuses every request used to be
                    // indistinguishable from a muted player: librespot logged
                    // the rejection to its own logger, the resulting
                    // `Unavailable` ended the source, and the queue moved on.
                    self.notify_typed(
                        &format!("{provider} error"),
                        message.clone(),
                        NotificationKind::Error,
                        false,
                        NotifType::NowPlaying,
                    );
                }
                if let DaemonEvent::Custom { name, data } = &ev
                    && name == "sync_done"
                    && data.get("kind").is_some_and(|k| k == "metadata")
                {
                    had_sync_done = true;
                }
                events_received = true;
            }
            // Mark the next frame dirty so any mid-transition state change
            // (a `PlaybackStarted` right after a play command) is drawn even
            // though position/animation triggers may not fire yet.
            if events_received {
                self.data_dirty = true;
            }
            // A reattached client knows nothing about what the daemon has been
            // doing. The event stream is a forward-only broadcast, so anything
            // emitted during the outage — the `PlaybackStarted` that named the
            // current track — is gone, and the mirror's now-playing is null
            // until something replaces it. Pull a fresh snapshot and re-arm the
            // cover fetch on the rising edge, rather than waiting for a poll
            // that only reaches the UI if some other request happens to land.
            let linked = self.client.is_connected();
            if linked && !self.link_up {
                self.link_up = true;
                self.path_display = None;
                let client = self.client.clone();
                let ipc_tx = self.ipc_tx.clone();
                tokio::spawn(async move {
                    if let Ok(state) = client.get_status().await {
                        let _ = ipc_tx.send(IpcResult::RefreshDone(Box::new(state), None, None));
                    }
                });
            } else if !linked {
                self.link_up = false;
            }
            // A new track must take the theme back to the base palette until
            // its own art arrives, or it keeps the outgoing track's colours
            // for the whole time the new one plays.
            //
            // Keyed on the palette being held, not on the cover: this runs
            // before the track-change block below clears `np_cover.image`, so
            // a test on the cover reads the *outgoing* track's bytes and fires
            // only when the previous track had no art — exactly inverting the
            // intent. The new art, if any, arrives later and re-tints.
            if had_track_change && self.reactive_theme && self.reactive_palette.is_some() {
                self.reactive_palette = None;
                self.apply_reactive();
            }
            if events_received {
                self.last_event_time = std::time::Instant::now();
            }
            if had_sleep_expired {
                self.sleep_timer.remaining = None;
                self.notify_titled(
                    "Sleep Timer",
                    "Sleep timer expired: shutting down",
                    NotificationKind::Info,
                    false,
                    NotifType::Playback,
                );
                // Draw a final frame so the user sees the shutdown message
                // before the terminal restores.
                let _ = terminal.draw(|f| ui::render(f, &mut self));
                tokio::time::sleep(Duration::from_millis(1200)).await;
                // Await the daemon's reply so the quit request is actually
                // delivered before the TUI exits. The daemon replies Ok then
                // shuts down ~200ms later.
                let c = self.client.clone();
                let _ = tokio::time::timeout(Duration::from_millis(1500), c.quit()).await;
                break;
            }
            // Sync sleep_timer_remaining from daemon state
            if let Some(secs) = self.state.sleep_timer {
                self.sleep_timer.remaining = Some(secs as u64);
            } else if self.sleep_timer.remaining.is_some() && self.state.sleep_timer.is_none() {
                self.sleep_timer.remaining = None;
            }
            // Re-seed clock from state after track change events so the
            // local position estimate stays in sync with the daemon.
            if had_track_change {
                self.client.seed_clock(&self.state).await;
                // The now-playing section is only re-seeded when the *path*
                // changes, so a `PlaybackStarted` for a track already
                // displayed — a crossfade landing back on it, or the same song
                // restarting — left the previous elapsed position and cover on
                // screen. The event means a track (re)started whatever its
                // path, so the position follows it either way.
                if self.path_display.as_deref()
                    == self.state.current_track.as_ref().map(|t| t.path.as_str())
                {
                    let raw = self.client.estimated_position().await;
                    self.display_position = raw;
                    self.last_display_position = raw;
                    self.raw_position = raw;
                    let d = if self.state.duration > 0.0 {
                        self.state.duration
                    } else {
                        self.state
                            .current_track
                            .as_ref()
                            .map(|t| t.duration)
                            .unwrap_or(0.0)
                    };
                    self.progress_smoother
                        .reset(if d > 0.0 { raw / d } else { 0.0 });
                }
            }
            if had_sync_done
                && let Ok(DaemonRes::Tracks { tracks, .. }) =
                    self.client.library().get_tracks(None, None).await
            {
                self.tracks_cache = *tracks;
                self.tracks_cache_gen = self.tracks_cache_gen.wrapping_add(1);
            }

            if had_spotify_change {
                if let Ok(status) = self.client.spotify().status().await {
                    let was_linked = self.spotify.status.as_ref().is_some_and(|s| s.linked);
                    if status.linked {
                        // Clearing the waiting state is idempotent on purpose.
                        // It used to ride the `!was_linked` edge, which meant a
                        // single failed status call left the picker up for good:
                        // the daemon's post-sync event could no longer fire the
                        // transition, because by then the account was already
                        // linked. Every event gets a chance to dismiss instead,
                        // so a missed one costs a moment, not a stuck screen.
                        self.spotify.oauth_pending = false;
                        self.spotify.oauth_url = None;
                        self.spotify.oauth_error = None;
                        if self.pickers.top().map(|o| o.id) == Some(PickerId::SpotifyLink) {
                            self.close_picker();
                        }
                    }
                    if status.linked && !was_linked {
                        let user = status.user.clone().unwrap_or_else(|| "account".into());
                        // If the flow started from the Setup walkthrough
                        // navigate to Spotify; if it started from the Alt+s
                        // search picker, keep that picker open so it now
                        // behaves as the search box. Deliberately *not*
                        // idempotent: re-running this on the post-sync event
                        // would reset the library pane the user has already
                        // navigated.
                        match self.pickers.top().map(|o| o.id) {
                            Some(PickerId::SpotifySearch) => {}
                            _ => {
                                self.reset_library_view(5, None);
                                self.library_pane_focus = true;
                            }
                        }
                        // Playlists are still being paginated in the daemon
                        // background, so say that rather than claiming the
                        // sync already finished. The app is named because it
                        // decides which rate limit the sync is spending, and
                        // nothing on screen said so before.
                        let app_desc = match self.spotify.oauth_sent_id.as_deref() {
                            Some(id) if !id.trim().is_empty() => {
                                format!("via your own app ({})", mask_credential(id.trim()))
                            }
                            _ => "via the shared app".to_string(),
                        };
                        self.notify_titled(
                            "Spotify",
                            format!("Linked as {user} {app_desc} — syncing playlists"),
                            NotificationKind::Success,
                            false,
                            NotifType::Spotify,
                        );
                        // The id is persisted daemon-side; the form's copy is a
                        // credential that has no further use, so do not keep it
                        // in pane state for the rest of the session.
                        self.spotify.oauth_sent_id = None;
                        self.spotify.oauth_client_id.clear();
                    } else if self.spotify.oauth_pending && !status.linked {
                        // The OAuth browser flow failed (e.g. no network): stop
                        // waiting, dismiss the picker and report the failure.
                        let msg = status
                            .error
                            .clone()
                            .filter(|m| !m.is_empty())
                            .unwrap_or_else(|| "Spotify link failed".to_string());
                        self.spotify.oauth_pending = false;
                        // Keep the picker open and render the error inline: a
                        // floating toast is suppressed while a picker is open,
                        // so dismissing here would drop the only feedback.
                        self.spotify.oauth_error = Some(msg);
                        // The authorize URL (set when the flow started) stays
                        // visible so the user can retry from the browser side.
                        self.notify_titled(
                            "Spotify",
                            "Spotify link failed",
                            NotificationKind::Error,
                            false,
                            NotifType::Spotify,
                        );
                    }
                    self.spotify.status = Some(status);
                }
                if let Ok(playlists) = self.client.spotify().playlists().await {
                    // Only announce completion when the daemon actually
                    // delivered playlists; the first status event fires as
                    // soon as the client is ready, well before pagination
                    // finishes.
                    if !playlists.is_empty() && !self.spotify.sync_announced {
                        self.spotify.sync_announced = true;
                        let count = playlists.len();
                        self.notify_titled(
                            "Spotify",
                            format!("Synced {count} playlists"),
                            NotificationKind::Success,
                            false,
                            NotifType::Spotify,
                        );
                    }
                    self.spotify.playlists = playlists;
                }
            }

            // The daemon finished the (daemon-hosted) Last.fm link flow; pull
            // the status the same way the Spotify hook does. The LastfmStatus
            // IpcResult handler below dismisses the prompt on `ready` (or shows
            // the failure reason), so no client-side callback polling exists.
            if had_lastfm_change {
                self.refresh_lastfm_status();
            }

            // Force a state refresh if no events received for 8s to prevent
            // stale state from broadcast lag. Works in all playback states,
            // not just Playing, to catch lag when paused/stopped too.
            if self.last_event_time.elapsed() > Duration::from_secs(8) && self.client.is_connected()
            {
                let c = self.client.clone();
                let ipc_tx = self.ipc_tx.clone();
                tokio::spawn(async move {
                    if let Ok(state) = c.get_status_lite().await {
                        let _ = ipc_tx.send(IpcResult::RefreshDone(Box::new(state), None, None));
                    }
                });
                self.last_event_time = std::time::Instant::now();
            }

            self.last_queue_cursor = self.state.queue_cursor;

            // Track-change detection keyed on path.  Queued/foreign tracks
            // share `id == 0`, so id-based detection misses auto-advance
            // between them (stale elapsed, cover art and lyrics).
            let current_tid = self.state.current_track.as_ref().map(|t| t.id);
            let current_path = self.state.current_track.as_ref().map(|t| t.path.clone());
            let track_changed = current_path.as_deref() != self.path_display.as_deref();
            if track_changed {
                self.path_display = current_path;
                let raw = self.client.estimated_position().await;
                self.display_position = raw;
                self.last_display_position = raw;
                self.raw_position = raw;
                let d = if self.state.duration > 0.0 {
                    self.state.duration
                } else {
                    self.state
                        .current_track
                        .as_ref()
                        .map(|t| t.duration)
                        .unwrap_or(0.0)
                };
                self.progress_smoother
                    .reset(if d > 0.0 { raw / d } else { 0.0 });
                self.track_anim_trigger = true;
                // Re-enable auto-sync for the new track; a manual lyric scroll
                // on a previous track must not stick across track changes.
                self.lyrics.manual_scroll = false;
            }

            // A live stream is a second case: its path never changes, so
            // `track_changed` alone would fetch artwork once for the whole
            // session. An advance of the track on air counts as a track change
            // so the cover and reactive palette follow the rotation. Evaluated
            // unconditionally, since it also refreshes the remembered title.
            let live_advanced = self.live_advanced();

            // Clear stale cover immediately so we don't show old art on the
            // new track, then trigger a cover fetch + lyrics auto-fetch.
            if track_changed || had_track_change || live_advanced {
                self.np_cover.image = None;
                self.np_cover.stateful = None;
                // Invalidate pending fetch so stale responses cannot overwrite
                // the new track. Path is used alongside id to disambiguate
                // `id == 0` locally-inserted tracks.
                let cur_path = self.state.current_track.as_ref().map(|t| t.path.clone());
                self.np_cover.track_id = current_tid;
                self.np_cover.track_path = cur_path.clone();
                self.np_cover.pending_gen = None;
                // Fetch cover art when needed: for display (no_image_protocol
                // check) OR for reactive-theming palette extraction.
                let needs_cover = self.reactive_theme || !no_image_protocol();
                if needs_cover && let Some(tid) = current_tid {
                    let fetch_gen = self.next_cover_gen();
                    self.np_cover.pending_gen = Some(fetch_gen);
                    // Pass the track path as well as the id. A provider track
                    // has no library row, so `id == 0` is ambiguous on its own
                    // and the daemon resolves it by exact path instead.
                    let art_path = cur_path.clone();
                    let client = self.client.clone();
                    let ipc_tx = self.ipc_tx.clone();
                    tokio::spawn(async move {
                        let art = match client.art().cover_for(tid, art_path).await {
                            Ok(b64) => b64,
                            Err(_) => client.art().cover(tid).await.ok().flatten(),
                        };
                        // Always answer, including "no art". A miss used to
                        // send nothing at all, which left `pending_gen` claimed
                        // for the rest of the session: the reply handler
                        // treats that as "still in flight", so the pane stayed
                        // blank and no later attempt could ever claim the slot
                        // again. That also starved reactive theming, which
                        // fetches a cover solely to extract a palette from it.
                        let msg = art
                            .and_then(|b64| {
                                base64::engine::general_purpose::STANDARD.decode(&b64).ok()
                            })
                            .map_or(IpcResult::CoverArt(None, Some(tid), fetch_gen), |bytes| {
                                IpcResult::CoverArt(Some(bytes), Some(tid), fetch_gen)
                            });
                        let _ = ipc_tx.send(msg);
                    });
                }
                // Auto-fetch lyrics on track change if enabled and (pane
                // visible or auto-fetch enabled).
                //
                // Keyed on a *different* track, not on every `PlaybackStarted`.
                // A `PlaybackStarted` fires for a crossfade and for the same
                // track restarting, where the lyrics already in the pane are
                // the right ones and re-fetching them resets the user's manual
                // sync offset for nothing.
                if track_changed
                    && (self.auto_fetch_lyrics || self.lyrics.show)
                    && let Some(track) = self.state.current_track.clone()
                {
                    self.fetch_lyrics(&track);
                }
            }

            while let Ok(result) = self.ipc_rx.try_recv() {
                match result {
                    IpcResult::RefreshDone(state, cover, cover_tid) => {
                        // Only apply if the new state is at least as recent as ours.
                        // A stale get_status() response can arrive after a PlaybackStarted
                        // event and overwrite the newer state if we don't guard this.
                        //
                        // Exception: a large backward version jump means the daemon
                        // restarted (its counter resets to 0).  The fresh snapshot is
                        // authoritative even though its version is lower than the local
                        // mirror, so it must not be dropped forever.
                        let restarted = state.version < self.state.version
                            && self.state.version.saturating_sub(state.version) > 1000;
                        // A snapshot naming a different track is the one thing
                        // this guard must never throw away: it is how a
                        // reattaching client learns what is playing, and a
                        // lower version is the daemon being *behind* an event
                        // we already applied, not the snapshot being stale. The
                        // version heuristic was standing in for this, and it
                        // could not tell the two apart — the mirror counts
                        // events this counter never sees.
                        let track_differs = state.current_track.as_ref().map(|t| &t.path)
                            != self.state.current_track.as_ref().map(|t| &t.path);
                        if state.version >= self.state.version || restarted || track_differs {
                            self.state = *state;
                            self.client.seed_clock(&self.state).await;
                            // Re-render so the initial state (or any daemon-side
                            // refresh) paints immediately instead of waiting for
                            // the next keypress or animation trigger.
                            self.data_dirty = true;
                            // Cover art is fetched on track-change events only.
                            // Do not clear track_id when a periodic RefreshDone
                            // carries no cover, or the track-change guard would
                            // re-download art (and re-fetch lyrics) every second.
                            // Robustness: verify cover_tid still matches current
                            // track (id+path) to avoid stale periodic cover
                            // overwriting a newer track's art.
                            if let Some(c) = cover {
                                let cur_tid = self.state.current_track.as_ref().map(|t| t.id);
                                let cur_path =
                                    self.state.current_track.as_ref().map(|t| t.path.clone());
                                let tid_matches = cover_tid == cur_tid;
                                let path_matches = match (&cover_tid, &cur_path) {
                                    (Some(_), Some(cp)) => {
                                        // If daemon supplied a tid, ensure path
                                        // consistency for `id == 0` disambiguation
                                        self.np_cover.track_path.as_deref() == Some(cp.as_str())
                                            || self.np_cover.track_path.is_none()
                                    }
                                    (None, None) => true,
                                    _ => tid_matches,
                                };
                                if tid_matches && path_matches {
                                    if self.reactive_theme {
                                        let tx = self.ipc_tx.clone();
                                        self.reactive_gen = Some(self.next_cover_gen());
                                        self.request_reactive_palette(
                                            &c,
                                            self.reactive_gen.expect("just set"),
                                            tx,
                                        );
                                    }
                                    self.np_cover.image = Some(c);
                                    self.np_cover.track_id = cover_tid;
                                    if !no_image_protocol() {
                                        self.cover_sync();
                                    } else {
                                        self.cover_art_dirty = true;
                                    }
                                }
                            }
                        }
                    }
                    IpcResult::CoverArt(cover, cover_tid, fetch_gen) => {
                        // Generation + id+path guard: stale Covers for fast
                        // skips (A->B->C) are dropped if fetch_gen mismatches or tid
                        // no longer equals current track. Covers `id == 0`
                        // reuse (queued Spotify/YouTube) via generation.
                        let Some(pending) = self.np_cover.pending_gen else {
                            // No pending fetch — likely track changed and cleared,
                            // drop stale.
                            continue;
                        };
                        if fetch_gen != pending {
                            continue;
                        }
                        let cur_tid = self.state.current_track.as_ref().map(|t| t.id);
                        if cover_tid != cur_tid {
                            continue;
                        }
                        // Path check for `id == 0` disambiguation
                        if let (Some(_), Some(cur_path)) = (
                            &cover_tid,
                            self.state.current_track.as_ref().map(|t| &t.path),
                        ) && let Some(pending_path) = self.np_cover.track_path.as_ref()
                            && pending_path != cur_path
                        {
                            continue;
                        }
                        if let Some(c) = cover.as_ref()
                            && self.reactive_theme
                        {
                            let tx = self.ipc_tx.clone();
                            self.reactive_gen = Some(self.next_cover_gen());
                            let pal_gen = self.reactive_gen.expect("just set");
                            self.request_reactive_palette(c, pal_gen, tx);
                        }
                        self.np_cover.pending_gen = None;
                        self.np_cover.image = cover;
                        self.np_cover.track_id = cover_tid;
                        if !no_image_protocol() {
                            self.cover_sync();
                        }
                        self.cover_art_dirty = true;
                    }
                    IpcResult::ReactivePalette(pal, pal_gen) => {
                        // Only the newest extraction may paint. A reply for a
                        // cover the cursor has already left is a late arrival,
                        // not a late update. Guarded on its own generation
                        // rather than the cover's, which is released as soon as
                        // the bytes land — the palette always arrives after.
                        if self.reactive_theme && self.reactive_gen == Some(pal_gen) {
                            self.reactive_palette = pal;
                            self.apply_reactive();
                        }
                    }
                    IpcResult::PodcastStatus(st) => self.podcast.status = st,
                    IpcResult::PodcastFeeds(feeds) => {
                        self.podcast.feeds = feeds;
                        self.podcast.feeds_pending = false;
                    }
                    IpcResult::PodcastEpisodes(eps) => {
                        self.podcast.episodes = eps;
                    }
                    IpcResult::RadioSearch(stations) => {
                        self.radio.search = stations;
                        self.radio.search_pending = false;
                    }
                    IpcResult::RadioTop(stations) => {
                        self.radio.top = stations;
                        self.radio.top_pending = false;
                    }
                    IpcResult::RadioTags(tags) => {
                        self.radio.browse_tags = tags;
                        self.radio.browse_pending = false;
                    }
                    IpcResult::RadioCountries(countries) => {
                        self.radio.browse_countries = countries;
                        self.radio.browse_pending = false;
                    }
                    IpcResult::RadioBrowseStations(stations) => {
                        self.radio.browse_stations = stations;
                        self.radio.browse_stations_pending = false;
                    }
                    IpcResult::SpotifyMatch(uri) => {
                        self.live_uri = Some(uri);
                    }
                    IpcResult::RadioTracklist(list) => {
                        // The daemon's refresh tick owns this field once it
                        // lands, so only fill it while still empty.
                        if self.state.radio_tracks.tracks.is_empty() {
                            self.state.radio_tracks = list;
                        }
                    }
                    IpcResult::ChartsLoaded(charts) => {
                        // A stale selected source index (e.g. Spotify got
                        // unlinked between fetches) must never crash the
                        // picker: if the new list has no such row, reset the
                        // drill-down to Level 0.
                        if let Some(src) = self.charts.selected_source
                            && src >= charts.len()
                        {
                            self.charts.selected_source = None;
                            self.charts.selected_chart = None;
                        }
                        self.charts.charts = charts;
                        // The card describes the highlighted chart, so it can
                        // only be built once the list is here.
                        self.update_track_popup();
                    }
                    IpcResult::ChartTracksLoaded(tracks) => {
                        self.charts.chart_tracks = tracks;
                        // Warm the left card for the row that is now
                        // highlighted. Without this the card waits for the next
                        // cursor move, and at the row the user actually landed
                        // on after opening the chart it showed nothing at all.
                        self.update_track_popup();
                    }
                    IpcResult::ChartsSources(sources) => {
                        if let Some(src) = self.charts.selected_source
                            && src >= sources.len()
                        {
                            self.charts.selected_source = None;
                            self.charts.selected_chart = None;
                            self.charts.charts.clear();
                            self.charts.chart_tracks.clear();
                        }
                        self.charts.sources = sources;
                        // Likewise for the source row itself.
                        self.update_track_popup();
                    }
                    IpcResult::LastfmStatus(st) => {
                        let was_ready = self.setup.lastfm_status.as_ref().is_some_and(|s| s.ready);
                        // A daemon-pushed failure (callback timeout, token
                        // exchange error) while the prompt is waiting must
                        // surface immediately: stop waiting, keep the picker
                        // open and render the reason inline (a toast is
                        // suppressed while a picker is open).
                        if self.setup.lastfm_pending
                            && let Some(err) = st.as_ref().and_then(|s| s.error.clone())
                        {
                            self.setup.lastfm_pending = false;
                            self.setup.lastfm_auth_url = None;
                            self.setup.lastfm_error = Some(err.clone());
                            self.notify_titled(
                                "Last.fm",
                                err,
                                NotificationKind::Error,
                                false,
                                NotifType::Lastfm,
                            );
                        }
                        self.setup.lastfm_status = st;
                        let now_ready = self.setup.lastfm_status.as_ref().is_some_and(|s| s.ready);
                        if now_ready
                            && !was_ready
                            && self
                                .pickers
                                .top()
                                .is_some_and(|o| o.id == PickerId::LastfmAuth)
                        {
                            self.setup.lastfm_pending = false;
                            self.setup.lastfm_auth_url = None;
                            self.setup.lastfm_error = None;
                            self.notify_titled(
                                "Last.fm",
                                "Last.fm linked — scrobbling is now active",
                                NotificationKind::Success,
                                false,
                                NotifType::Lastfm,
                            );
                            self.close_picker();
                        }
                    }
                    IpcResult::AuthUrl("Last.fm", url) => {
                        // The daemon bound the callback port and started the
                        // flow before answering, so the URL can be opened
                        // safely; completion/failure arrives as a pushed status
                        // event. Here we only surface the URL for display.
                        self.setup.lastfm_auth_url = Some(url);
                        self.setup.lastfm_pending = true;
                    }
                    IpcResult::AuthError(provider, e) => match provider {
                        "Last.fm" => {
                            self.setup.lastfm_pending = false;
                            self.setup.lastfm_error = Some(e.clone());
                            self.notify_titled(
                                "Last.fm",
                                e,
                                NotificationKind::Error,
                                false,
                                NotifType::Lastfm,
                            );
                        }
                        _ => {
                            self.spotify.oauth_pending = false;
                            self.spotify.oauth_error = Some(e.clone());
                            self.notify_titled(
                                "Spotify",
                                e,
                                NotificationKind::Error,
                                false,
                                NotifType::Spotify,
                            );
                        }
                    },
                    IpcResult::LibraryTracks(tracks) => {
                        self.tracks_cache = tracks;
                        self.tracks_cache_gen = self.tracks_cache_gen.wrapping_add(1);
                    }
                    IpcResult::MostPlayed(tracks) => self.most_played_cache = tracks,
                    IpcResult::RecentlyPlayed(tracks) => self.recently_played_cache = tracks,
                    IpcResult::RecentlyAdded(tracks) => self.recently_added_cache = tracks,
                    IpcResult::Playlists(playlists) => self.playlist_cache = playlists,
                    IpcResult::PlaylistCreated(id, _name) => {
                        self.pending_playlist_id = Some(id);
                        self.selected_track_ids.clear();
                        self.pickers.open(PickerId::PlaylistTrackSelect);
                        self.pending_track_ids = vec![id];
                    }
                    IpcResult::PlaylistTracks(tracks) => self.playlist_tracks_cache = tracks,
                    IpcResult::Queue(tracks, cursor) => {
                        let cursor_changed = self.queue.cursor != cursor;
                        self.queue.cache = tracks.clone();
                        self.queue.cursor = cursor;
                        if cursor_changed {
                            self.queue.preview_cover = None;
                            self.queue.preview_cover_stateful = None;
                        }
                        // Fire once per spell of "idle", not on every poll.
                        // The queue is re-polled at 1 Hz and an empty answer is
                        // assigned straight back, so an ungated check never
                        // stops being true — which reset the library cursor to
                        // the top row, dropped any open drill-down and cleared
                        // the cover, once a second, for as long as the player
                        // sat idle. It read as a sync bug because a Spotify
                        // link is exactly when the list is growing and nothing
                        // is playing, but it fired in every category.
                        let idle =
                            self.queue.cache.is_empty() && self.state.current_track.is_none();
                        if idle && !self.idle_reset {
                            self.idle_reset = true;
                            self.reset_library_view(self.library_category, None);
                        } else if !idle {
                            self.idle_reset = false;
                        }
                    }
                    IpcResult::YtResults(query, results) => {
                        // Apply results only when they belong to the query the
                        // picker currently shows; stale results from a
                        // superseded search are dropped.
                        let current = self
                            .pickers
                            .top()
                            .filter(|t| t.id == PickerId::YTSearch)
                            .map(|t| t.query.clone());
                        if current.as_deref() == Some(query.as_str()) {
                            // Interleave: 1 playlist for every 3 tracks
                            self.yt_results_cache = Self::interleave_yt_results(results);
                            self.yt_search_loading = false;
                        }
                    }
                    IpcResult::Notification(title, msg, kind, ntype) => {
                        // Petty flow acknowledgements (playlist add/remove,
                        // cache clears, etc.) must not interrupt: the playlist
                        // notices surface in the footer as before, while cache
                        // clears are fully silent and only recorded in history.
                        if title == "Cache" {
                            self.notify_silent(&title, msg, kind);
                        } else {
                            let trivial = title == "Playlist"
                                && (msg.starts_with("Tracks added to playlist")
                                    || msg.starts_with("Removed from playlist")
                                    || msg.starts_with("Created "));
                            self.notify_typed(&title, msg, kind, trivial, ntype);
                        }
                    }
                    IpcResult::Error(e) => {
                        // Generic failures go to the log file and the
                        // notifications picker instead of a floating card.
                        self.notify_typed(
                            "Error",
                            e,
                            NotificationKind::Error,
                            true,
                            NotifType::System,
                        );
                    }
                    IpcResult::YtDownloadProgress {
                        id,
                        url,
                        title,
                        progress,
                        status,
                        file_path,
                        downloaded_bytes,
                        total_bytes,
                        rate_bps,
                        eta_secs,
                    } => {
                        let terminal =
                            matches!(status.as_str(), "completed" | "failed" | "cancelled");
                        if terminal {
                            self.downloads.remove(&id);
                            self.downloading_urls.remove(&url);
                        } else {
                            let last = self.downloads.get(&id).cloned();
                            let smooth = if let Some(last) = last
                                && last.percent > 0.0
                                && progress > last.percent
                            {
                                // EMA with ~0.6 inertia per update (~250ms) so
                                // the footer bar glides instead of jittering.
                                last.percent + (progress - last.percent) * 0.4
                            } else {
                                progress
                            };
                            self.downloads.insert(
                                id,
                                DownloadProgressView {
                                    url,
                                    title,
                                    status,
                                    file_path,
                                    percent: smooth,
                                    downloaded_bytes,
                                    total_bytes,
                                    rate_bps,
                                    eta_secs,
                                    updated_at: std::time::Instant::now(),
                                },
                            );
                        }
                    }
                    IpcResult::PopupCoverArt(cover, track_id, fetch_gen) => {
                        if !no_image_protocol()
                            && self.popup_track_id == Some(track_id)
                            && self.popup_slot.matches(fetch_gen)
                        {
                            self.track_popup_cover = cover;
                            self.popup_cover_sync();
                        }
                    }
                    IpcResult::SpotifyPopupCover(cover, url, fetch_gen) => {
                        if !no_image_protocol()
                            && self.spotify_popup_slot.id.as_deref() == Some(&url)
                            && self.spotify_popup_slot.matches(fetch_gen)
                        {
                            self.track_popup_cover = cover;
                            self.popup_cover_sync();
                        }
                    }
                    IpcResult::ClipboardPaste(field, text) => {
                        self.apply_paste(field, &text);
                    }
                    IpcResult::QueuePreviewCover(cover, key, fetch_gen) => {
                        if !no_image_protocol()
                            && self.queue.preview_slot.id.as_deref() == Some(&key)
                            && self.queue.preview_slot.matches(fetch_gen)
                        {
                            self.queue.preview_cover = cover;
                            self.sync_preview_cover();
                            // The cover arrived on the IPC event loop; force a
                            // redraw this frame so the picker shows it without
                            // waiting for a coincidental render trigger.
                            self.cover_art_dirty = true;
                            if self.queue.preview_cover.is_some() {
                                self.queue.preview_fail_until = None;
                            } else {
                                // Failed or empty: release the guard so a later
                                // preview retries, and throttle the refetch.
                                self.queue.preview_slot.version = None;
                                self.queue.preview_fail_until = Some((
                                    key,
                                    std::time::Instant::now() + Duration::from_secs(30),
                                ));
                            }
                        }
                    }
                    IpcResult::PickerPreviewCover(cover, track_id, fetch_gen) => {
                        if !no_image_protocol()
                            && self.picker_slot.id == Some(track_id)
                            && self.picker_slot.matches(fetch_gen)
                        {
                            self.picker_preview_cover = cover;
                            self.picker_preview_sync();
                            self.cover_art_dirty = true;
                            // Release the guard on a miss so the same row can be
                            // retried later instead of staying blank forever.
                            if self.picker_preview_cover.is_none() {
                                self.picker_slot.clear();
                            }
                        }
                    }
                    IpcResult::MetadataCoverArt(cover, track_id, fetch_gen) => {
                        if !no_image_protocol()
                            && self.metadata.edit_track_ids.first() == Some(&track_id)
                            && self.metadata.cover_fetch.matches(fetch_gen)
                        {
                            self.metadata.cover = cover;
                            self.metadata_cover_sync();
                            self.metadata.cover_dirty = true;
                        }
                    }
                    IpcResult::ArtistCoverArt(cover, artist, fetch_gen) => {
                        if !no_image_protocol()
                            && self.artist_slot.id.as_deref() == Some(&artist)
                            && self.artist_slot.matches(fetch_gen)
                        {
                            self.artist_cover = cover;
                            self.artist_cover_sync();
                        }
                    }
                    IpcResult::SpotifyListCover(cover, url, fetch_gen) => {
                        if !no_image_protocol()
                            && self.spotify.list_fetch.id.as_deref() == Some(&url)
                            && self.spotify.list_fetch.matches(fetch_gen)
                        {
                            // Only latch `list_shown` on a hit. Recording the
                            // URL on a miss made one transient failure look
                            // like "already displayed" for good, so the playlist
                            // card never came back for that playlist — the
                            // fetch was skipped forever after.
                            let hit = cover.is_some();
                            self.spotify.list_cover = cover;
                            if hit {
                                self.spotify.list_shown = Some(url.clone());
                            }
                            self.spotify.list_fetch.clear();
                            self.list_cover_sync();
                            self.cover_art_dirty = true;
                        }
                    }
                    IpcResult::SpotifyRowCover(cover, url, fetch_gen) => {
                        // Same guards as the search preview: the reply has to
                        // match the URL still being awaited and the generation
                        // the cursor is on, or a slow reply paints the wrong row.
                        if !no_image_protocol()
                            && self.spotify.row_fetch.id.as_deref() == Some(&url)
                            && self.spotify.row_fetch.matches(fetch_gen)
                        {
                            let hit = cover.is_some();
                            self.spotify.row_cover = cover;
                            if hit {
                                self.spotify.row_shown = Some(url.clone());
                            }
                            // Released unconditionally: the row latch above
                            // is a hit-only claim, and a claimed slot reads as
                            // "in flight" to the next fetch attempt.
                            self.spotify.row_fetch.clear();
                            self.row_cover_sync();
                            self.cover_art_dirty = true;
                        }
                    }
                    IpcResult::SpotifyPreviewCover(cover, url, fetch_gen) => {
                        if !no_image_protocol()
                            && self.spotify.preview_fetch.id.as_deref() == Some(&url)
                            && self.spotify.preview_fetch.matches(fetch_gen)
                        {
                            let prev_shown = self.spotify.preview_shown.clone();
                            self.spotify.preview_cover = cover.clone();
                            if cover.is_some() {
                                // Hit-only, like the row and list latches: on a
                                // miss nothing is cached, so claiming the URL
                                // as "shown" left the next frame free to
                                // re-request it. `update_spot_preview` runs per
                                // rendered frame, which turned one failed
                                // lookup into a Spotify CDN request carrying a
                                // bearer token, at frame rate.
                                self.spotify.preview_shown = Some(url.clone());
                            }
                            if let Some(bytes) = cover {
                                // Bounded: each album is ~150 KB of decoded
                                // JPEG, so an unbounded map would grow for the
                                // whole session. Drop the entry we just left
                                // once full, which keeps the recent page of
                                // results warm.
                                if self.spotify.preview_cache.len() >= PREVIEW_CACHE_MAX
                                    && let Some(prev) = prev_shown
                                {
                                    self.spotify.preview_cache.remove(&prev);
                                }
                                self.spotify.preview_cache.insert(url.clone(), bytes);
                            }
                            self.spotify_preview_sync();
                            // Release the guard either way. A hit left it
                            // claimed, so returning to the same row looked
                            // "still in flight" forever and the slot could
                            // never be reused for a different album.
                            self.spotify.preview_fetch.clear();
                            if self.spotify.preview_cover.is_none() {
                                self.spotify
                                    .preview_fail_until
                                    .insert(url, std::time::Instant::now() + PREVIEW_RETRY);
                            }
                            self.cover_art_dirty = true;
                        }
                    }
                    IpcResult::CoverPicker(picker) => {
                        self.np_cover.picker = picker;
                        // Rebuild all active StatefulProtocols with the new
                        // picker geometry. Previously they retained the old
                        // picker's resize state, causing cropped / stale sizes
                        // after terminal resize or image-protocol renegotiation
                        //.
                        self.cover_sync();
                        self.popup_cover_sync();
                        self.sync_preview_cover();
                        self.picker_preview_sync();
                        self.artist_cover_sync();
                        self.spotify_preview_sync();
                        self.row_cover_sync();
                        self.list_cover_sync();
                        self.metadata_cover_sync();
                    }
                    IpcResult::PodcastTranscript(lyrics, title) => {
                        self.lyrics.pending_gen = None;
                        self.lyrics.current = lyrics.or(Some(LrcData {
                            title: Some(title.clone()),
                            artist: None,
                            album: None,
                            lines: vec![LrcLine {
                                timestamp: 0.0,
                                text: "Transcript was empty".to_string(),
                                words: Vec::new(),
                            }],
                        }));
                        self.lyrics.kind = LyricsKind::Transcript;
                        self.lyrics.fetching = false;
                        self.lyrics.scroll = self.current_lyric_index();
                        self.lyrics.show = true;
                    }
                    IpcResult::Lyrics(lyrics, lyrics_gen, for_path) => {
                        let now_playing =
                            self.state.current_track.as_ref().map(|t| t.path.as_str());
                        let wrong_track = match (for_path.as_deref(), now_playing) {
                            (Some(want), Some(have)) => !want.is_empty() && want != have,
                            (Some(want), None) => !want.is_empty(),
                            // A reply with no path came from a fetch that
                            // predates this check; the generation is all there
                            // is to go on.
                            _ => false,
                        };
                        if Some(lyrics_gen) != self.lyrics.pending_gen || wrong_track {
                            // Stale: the track changed while this fetch was in
                            // flight, so the lines belong to the previous song.
                            // Drop rather than flash the wrong lyrics.
                        } else {
                            self.lyrics.pending_gen = None;
                            self.lyrics.fetching = false;
                            // Snap to the lyric line matching the current
                            // playback position so opening lyrics mid-track
                            // doesn't start with the first line highlighted.
                            self.lyrics.scroll = self.current_lyric_index();
                            // "No lyrics found" is written into `lyrics.current`
                            // so the pane has something to render. It is
                            // therefore a real value, and assigning over it
                            // unconditionally was wrong in both directions: a
                            // *new* track's reply overwrote the lines the user
                            // was reading, and an *old* track's late failure
                            // replaced them with "No lyrics found".
                            if let Some(lyrics) = lyrics {
                                self.lyrics.current = Some(lyrics);
                                self.lyrics.kind = LyricsKind::Track;
                            } else if self.lyrics.kind == LyricsKind::None {
                                self.lyrics.current = Some(LrcData {
                                    title: None,
                                    artist: None,
                                    album: None,
                                    lines: vec![LrcLine {
                                        timestamp: 0.0,
                                        text: "No lyrics found".to_string(),
                                        words: Vec::new(),
                                    }],
                                });
                                self.lyrics.kind = LyricsKind::Missing;
                            }
                        }
                    }
                    IpcResult::HealthReport(report) => {
                        self.health_report = Some(report);
                        self.show_health_panel = std::mem::take(&mut self.report_health);
                    }
                    IpcResult::SpotifyStatus(s) => self.spotify.status = Some(s),
                    IpcResult::AuthUrl("Spotify", url) => {
                        self.spotify.oauth_url = Some(url);
                        self.spotify.oauth_error = None;
                    }
                    IpcResult::AuthUrl(_, _) => {}
                    IpcResult::AuthFallback(e) => {
                        // Browser auto-open failed but the authorize URL is
                        // already inline in the picker; record quietly.
                        self.spotify.oauth_error = Some(e.clone());
                        self.spotify.oauth_pending = false;
                        self.notify_titled(
                            "Spotify",
                            e,
                            NotificationKind::Info,
                            true,
                            NotifType::Spotify,
                        );
                    }
                    IpcResult::CoverCacheStat(bytes) => self.cover_cache_bytes = bytes,
                    IpcResult::AudioDevices(devices) => self.audio_devices = devices,
                    IpcResult::SpotifyPlaylists(p) => self.spotify.playlists = p,
                    IpcResult::SpotifySyncFinished(ok) => {
                        if ok {
                            self.spotify.synced_once = true;
                        }
                        self.spotify.sync_pending = false;
                    }
                    IpcResult::SpotifyTracks(t) => {
                        // The drill-down may have been closed while this was in
                        // flight; adopting the rows then would leave a stale
                        // track list attached to whatever opens next.
                        if self.in_spotify_playlist() {
                            self.spotify.playlist_tracks_cache = t;
                            // Opening a playlist should land with artwork
                            // already requested, not only after the first
                            // cursor move.
                            self.fetch_row_cover();
                        }
                    }
                    IpcResult::SpotifySearchWebDone(seq, res) => {
                        if seq != self.spotify.web_seq {
                            // Stale: a newer query superseded this in-flight
                            // response, so its rows would be for the wrong
                            // search. Drop rather than flash wrong results —
                            // the newer search owns the spinner.
                            continue;
                        }
                        match res {
                            Ok(tracks) => {
                                self.spotify.search_results.extend(
                                    tracks
                                        .into_iter()
                                        .map(|track| ("web".into(), "Spotify".into(), track)),
                                );
                            }
                            Err(e) => {
                                self.notify_titled(
                                    "Spotify",
                                    format!("Spotify Web search failed: {e}"),
                                    NotificationKind::Error,
                                    true,
                                    NotifType::Spotify,
                                );
                            }
                        }
                        self.spotify.search_loading = false;
                    }
                }
            }

            while let Ok(cmd) = self.pri_cmd_rx.try_recv() {
                self.handle_command(cmd);
            }
            while let Ok(cmd) = self.cmd_rx.try_recv() {
                self.handle_command(cmd);
            }

            // YT search debounce: auto-search 500ms after last keystroke
            let now = std::time::Instant::now();
            if let Some(deadline) = self.yt_search_debounce
                && now >= deadline
            {
                self.yt_search_debounce = None;
                if let Some(top) = self.pickers.top()
                    && top.id == PickerId::YTSearch
                    && !top.query.is_empty()
                {
                    let q = top.query.clone();
                    let tx = self.cmd_tx();
                    let _ = tx.send(TuiCommand::YtSearch(q)).await;
                }
            }

            // Spotify web-search debounce: auto-search 500ms after the last
            // keystroke, mirroring the YT search behaviour above.
            let now = std::time::Instant::now();
            if self.pickers.top().is_some_and(|t| t.id == PickerId::About) {
                self.tick_about_viz(now);
            }
            if let Some(deadline) = self.spotify.search_debounce
                && now >= deadline
            {
                self.spotify.search_debounce = None;
                if let Some(top) = self.pickers.top()
                    && top.id == PickerId::SpotifySearch
                    && !top.query.is_empty()
                {
                    self.search_spotify();
                }
            }

            // YT search polling: while a search is in flight, poll for results
            // so the picker populates without requiring an extra Enter.  The
            // daemon runs the search on a background task; polls return the
            // results once it completes.
            if self.yt_search_loading
                && self
                    .pickers
                    .top()
                    .is_some_and(|t| t.id == PickerId::YTSearch)
            {
                if self.search_deadline.is_none() {
                    self.search_deadline = Some(now + Duration::from_millis(500));
                }
                if now >= self.search_deadline.unwrap_or(now) {
                    self.search_deadline = Some(now + Duration::from_millis(700));
                    let tx = self.cmd_tx();
                    let _ = tx.send(TuiCommand::RefreshYt).await;
                }
            } else {
                self.search_deadline = None;
            }

            // Detect state changes
            let current_tid = self.state.current_track.as_ref().map(|t| t.id);
            if current_tid != self.prev_track_id {
                self.prev_track_id = current_tid;
            }
            if self.state.status != self.prev_status {
                self.prev_status = self.state.status;
            }
            // Volume changes: update the previous volume tracker without triggering an animation.
            if self.state.volume != self.prev_volume {
                self.prev_volume = self.state.volume;
            }
            if self.np_cover.track_id != self.prev_cover_id {
                self.prev_cover_id = self.np_cover.track_id;
            }

            let mut raw_pos = self.client.estimated_position().await;
            // Monotonic guard: prevent large backward jumps from clock skew.
            // Allow at most 0.5s of regression to avoid visible stutter.
            // Skipped right after a seek, otherwise a backward seek gets clamped
            // and the lyric highlight never re-syncs to the new position.
            let seeking = self
                .seek_pending
                .is_some_and(|t| t.elapsed() < std::time::Duration::from_millis(1200));
            if seeking {
                // A real (in-track) seek target has landed: drop the window so
                // the guard resumes once playback continues past it.
                if self.state.duration > 0.0 && self.raw_position <= self.state.duration {
                    self.seek_pending = None;
                }
            } else {
                raw_pos = raw_pos.max(self.display_position - 0.5);
            }
            // Lyric matching uses the raw (guard-only) position so the active
            // verse switches at the right timestamp; the smoothing below only
            // glides the progress bar. While a seek is being accumulated the
            // daemon hasn't seeked yet, so prefer the optimistic local estimate
            // rather than the still-stale daemon position.
            if self.seek_cmd_accum.is_none() {
                self.raw_position = raw_pos;
            }
            let now = std::time::Instant::now();
            let dt = now.duration_since(self.last_frame).as_secs_f64().min(0.25);
            self.last_frame = now;
            if self.seek_cmd_accum.is_some() {
                // Holding a seek key: hold the optimistic position so the bar
                // and highlight track the accumulated target, not the stale
                // daemon position.
                self.display_position = self.raw_position;
            } else {
                self.display_position +=
                    (raw_pos - self.display_position) * (1.0 - (-dt / 0.08).exp());
            }
            // Debounced daemon seek dispatch for long-press seeking.
            self.ensure_seek_flush();
            let bar_dur = if self.state.duration > 0.0 {
                self.state.duration
            } else {
                self.state
                    .current_track
                    .as_ref()
                    .map(|t| t.duration)
                    .unwrap_or(0.0)
            };
            let bar_target = if bar_dur > 0.0 {
                self.display_position / bar_dur
            } else {
                0.0
            };
            self.progress_smoother.smooth(bar_target, dt);

            // Auto-resume lyric auto-follow: after a manual scroll, once
            // playback reaches the line the user scrolled to, re-enable
            // auto-follow so the highlight can't stay frozen for the rest of
            // the track.  Reading ahead still holds until the audio catches up.
            if self.lyrics.manual_scroll
                && self.lyrics.show
                && self.lyrics.current.is_some()
                && self.current_lyric_index() >= self.lyrics.scroll
            {
                self.lyrics.manual_scroll = false;
            }

            // Auto-scroll lyrics to current playback position.  Not gated on
            // `status == Playing` so a mirrored-status desync (e.g. after a
            // daemon restart) can't freeze the highlight at the first line.
            if !self.lyrics.manual_scroll && self.lyrics.show && self.lyrics.current.is_some() {
                self.lyrics.scroll = self.current_lyric_index();
            }

            // Dirty-render: skip redraw if position hasn't changed meaningfully
            // to reduce CPU usage.  Always render every 10th frame as a safety net.
            let frame_count = self.frame_count.wrapping_add(1);
            self.frame_count = frame_count;

            // Advance progress whip scanner (Knight Rider style)
            const SCANNER_WIDTH: i32 = 12;
            const SCANNER_HOLD: i32 = 3;
            if self.scanner_hold > 0 {
                self.scanner_hold -= 1;
            } else {
                self.scanner_pos += self.scanner_dir;
                if self.scanner_pos >= SCANNER_WIDTH - 1 {
                    self.scanner_dir = -1;
                    self.scanner_hold = SCANNER_HOLD;
                } else if self.scanner_pos <= 0 {
                    self.scanner_dir = 1;
                    self.scanner_hold = SCANNER_HOLD;
                }
            }

            // Blink cursor every 8 frames
            if frame_count.is_multiple_of(8) {
                self.cursor_blink = !self.cursor_blink;
            }

            // Hot-reload config every 120 frames (~2s at 60fps)
            if frame_count.is_multiple_of(120) {
                self.check_config_reload();
            }

            let pos_changed = (self.display_position - self.last_display_position).abs() >= 0.05;
            // Advance title scroll animations (every 3rd frame)
            if frame_count.is_multiple_of(3) {
                self.footer_title_scroll = self.footer_title_scroll.wrapping_add(1);
                self.np_title_scroll = self.np_title_scroll.wrapping_add(1);
            }

            let playing = self.state.status == PlaybackStatus::Playing;
            // The visualizer animates continuously, idle wave included, so the
            // surfaces that draw it are worth a frame every tick even while the
            // position is unchanged. It is not always on: Zen only when its
            // surface is selected, and daydreaming only once the TUI has gone
            // quiet. The `[extensions]` switch zeroes the streams outright.
            let animating = !self.extensions.is_disabled(ExtensionId::Visualizer)
                && (self.daydreaming || (self.zen && self.zen_surface == ZenSurface::Visualizer));
            let mut force_render = pos_changed
                || (playing && frame_count.is_multiple_of(2))
                || animating
                || !self.notifications.is_empty()
                || frame_count.is_multiple_of(10)
                || self.cover_art_dirty
                || self.metadata.cover_dirty
                || self.track_anim_trigger
                || self.data_dirty
                || self.anim_fx.is_running();
            self.cover_art_dirty = false;
            self.metadata.cover_dirty = false;
            self.last_display_position = self.display_position;
            self.data_dirty = false;

            // Daydreaming: after this long with no key, paste or click, the
            // visualizer takes over the library view. Suppressed while a picker
            // is open or Zen is up — those are deliberate views of the user's
            // own choosing, and something they are actively looking at, and
            // daydreaming must not overwrite either. Cleared the instant
            // anything is touched again, so it never lingers behind a keypress.
            let idle = self.last_input.elapsed();
            self.daydreaming = idle >= Duration::from_secs(self.daydream_secs)
                && !self.zen
                && !self.pickers.is_open()
                && self.extensions.is_enabled(ExtensionId::Visualizer);

            // A resize forces a frame even when nothing else is dirty: the
            // layout is all size-derived, so a pane that did not redraw would
            // keep the old geometry until some unrelated change woke it.
            if std::mem::take(&mut self.resized) {
                force_render = true;
                // Drop the buffer ratatui diffs against. It is still the old
                // size, and diffing a new-size frame against it is what panics.
                let _ = terminal.clear();
                self.footer_cache.suppress_refresh = true;
            }

            if force_render {
                let (cols, rows) = crossterm::terminal::size().unwrap_or((80, 24));
                self.terminal_cols = cols;
                self.terminal_rows = rows;
                if cols < 20 || rows < 6 {
                    let _ = terminal.draw(|f| {
                        let msg = Paragraph::new("Terminal too small (min 20x6)")
                            .alignment(Alignment::Center);
                        f.render_widget(msg, f.area());
                    });
                } else if terminal.draw(|f| ui::render(f, &mut self)).is_ok() {
                    self.footer_cache.suppress_refresh = false;
                }
            }

            if event::poll(Duration::from_millis(16)).unwrap_or(false)
                && let Ok(ev) = event::read()
                && !self.handle_terminal_event(ev).await
            {
                break 'outer;
            }

            // Handle Ctrl+Z suspend: leave terminal, SIGTSTP, re-init on resume
            if self.pending_suspend {
                self.pending_suspend = false;
                let _ = crossterm::terminal::disable_raw_mode();
                let mut stdout = std::io::stdout();
                let _ = crossterm::execute!(
                    stdout,
                    crossterm::terminal::LeaveAlternateScreen,
                    crossterm::event::DisableBracketedPaste,
                    crossterm::event::DisableMouseCapture
                );
                // Suspend: this blocks until SIGCONT
                unsafe {
                    libc::raise(libc::SIGTSTP);
                }
                // Resumed: re-init terminal
                crossterm::terminal::enable_raw_mode()?;
                let mut stdout = std::io::stdout();
                crossterm::execute!(
                    stdout,
                    crossterm::terminal::EnterAlternateScreen,
                    crossterm::event::EnableBracketedPaste,
                    crossterm::event::EnableMouseCapture
                )?;
                terminal.clear()?;
            }
        }
        Ok(())
    }

    /// Cycle pane focus with Tab/Shift-Tab.  With lyrics open this walks
    /// left pane → right pane → lyrics pane (or the reverse for Shift-Tab);
    /// otherwise it toggles the left/right panes.  Leaving the lyrics pane
    /// re-enables lyric auto-follow.
    pub(crate) fn cycle_pane_focus(&mut self, forward: bool) {
        if self.lyrics.show {
            let (lib, lyr) =
                cycle_library_focus(self.library_pane_focus, self.lyrics.pane_focus, forward);
            self.library_pane_focus = lib;
            self.lyrics.pane_focus = lyr;
            if !lyr {
                self.lyrics.manual_scroll = false;
            }
        } else {
            self.library_pane_focus = !self.library_pane_focus;
        }
    }
}
