use crate::app::*;

impl App {
    /// Open the `gtm setup` walkthrough, either the service chooser or the
    /// matching setup picker directly. Called from `gtm setup SERVICE`.
    /// Advance the About window's decorative visualization.
    pub fn tick_about_viz(&mut self, now: std::time::Instant) {
        let width = 64usize;
        if self.about_viz.bars.len() != width {
            self.about_viz.bars = vec![0.0; width];
        }
        self.about_viz.tick(now);
        // Cosmetic only, but the frame loop still needs to repaint.
        self.data_dirty = true;
    }

    /// Snapshot of the user-facing preferences resolved from current indices.
    /// Stored by name (not index) so adding/removing built-in themes or
    /// footer presets doesn't shift what was saved.
    pub(crate) fn current_prefs(&self) -> Prefs {
        Prefs {
            theme_name: self
                .themes
                .get(self.theme_index)
                .map(|t| t.name.to_string())
                .unwrap_or_else(default_theme_name),
            transparent_bg: self.transparent_bg,
            transparent_pickers: self.transparent_pickers,
            reactive_theme: self.reactive_theme,
            reactive_theme_intensity: self.reactive_theme_intensity,
            footer_preset_name: self
                .footer_presets
                .get(self.footer_preset)
                .map(|p| p.name.to_string())
                .unwrap_or_else(default_preset_name),
            progress_style: self.progress_style,
            visualizer_preset: self.visualizer.preset,
            extensions: self.extensions.clone(),
            time_format: self.footer_time_format.clone(),
            theme_mode: self.theme_mode.clone(),
            track_sort: self.track_sort,
            keybindings: self.prefs_keybindings.clone(),
            notification_modes: self
                .notification_modes
                .iter()
                .map(|(t, m)| (t.as_str().to_string(), m.as_str().to_string()))
                .collect(),
            // Config-file only, so it is carried through verbatim: rebuilding
            // the prefs must not silently reset the user's choice.
            footer_key_action: self.footer_key_action,
            cover_provider: self.cover_provider.clone(),
            cover_cache_mb: self.cover_cache_mb,
            auto_fetch_lyrics: self.auto_fetch_lyrics,
            icon_style: self.icon_style.clone(),
            hide_footer: self.hide_footer,
            left_pane_lists: self.left_pane_lists.clone(),
            show_preview: self.show_preview,
        }
    }

    /// Apply a theme picker selection by index, persist by name, and refresh
    /// the live `theme` field.
    pub(crate) fn apply_theme_index(&mut self, idx: usize) {
        let idx = idx.min(self.themes.len().saturating_sub(1));
        self.theme_index = idx;
        self.apply_reactive();
        // A manual selection opts out of OS-theme auto-detection so the OS
        // preference can't fight the choice on restart.
        self.theme_mode = "manual".to_string();
        save_prefs(&self.current_prefs());
    }

    /// Cycle the theme-following mode (auto → dark → light → manual → auto),
    /// immediately re-resolve the matching theme and apply it, then persist so
    /// the choice survives restarts. `manual` keeps the current pick; `auto`
    /// re-enables OS light/dark detection.
    /// Toggle footer visibility. Shared by the System and Spotify rows, which
    /// both expose a Hide Footer toggle.
    pub(crate) fn toggle_hide_footer(&mut self) {
        self.hide_footer = !self.hide_footer;
        save_prefs(&self.current_prefs());
    }

    pub(crate) fn cycle_theme_mode(&mut self) {
        let next_mode = match self.theme_mode.trim().to_ascii_lowercase().as_str() {
            "auto" => "dark",
            "dark" => "light",
            "light" => "manual",
            "manual" => "auto",
            _ => "auto",
        }
        .to_string();
        self.theme_mode = next_mode;
        let saved_name = self
            .themes
            .get(self.theme_index)
            .map(|t| t.name.to_string())
            .unwrap_or_else(default_theme_name);
        let idx = resolve_theme_index(&self.themes, &saved_name, &self.theme_mode);
        self.theme_index = idx;
        self.apply_reactive();
        save_prefs(&self.current_prefs());
        self.notify_silent(
            "Theme",
            format!("Theme mode: {}", theme_mode_label(&self.theme_mode)),
            NotificationKind::Info,
        );
    }

    /// Cycle the cover-art provider (auto → musicbrainz → deezer → spotify →
    /// auto), push the live value to the daemon and persist it for restarts.
    pub(crate) fn cycle_cover_provider(&mut self) {
        let next = match self.cover_provider.trim().to_ascii_lowercase().as_str() {
            "auto" => "musicbrainz",
            "musicbrainz" | "mb" => "deezer",
            "deezer" => "spotify",
            _ => "auto",
        }
        .to_string();
        self.cover_provider = next.clone();
        let label = cover_provider_label(&next);
        let c = self.client.clone();
        tokio::spawn(async move {
            let _ = c.set_cover_provider(&next).await;
        });
        save_prefs(&self.current_prefs());
        self.notify_typed(
            "System",
            format!("Cover source: {}", label),
            NotificationKind::Info,
            true,
            NotifType::Prefs,
        );
    }

    /// Apply a footer-preset picker selection by index and persist by name.
    pub(crate) fn apply_preset_index(&mut self, idx: usize) {
        let idx = idx.min(self.footer_presets.len().saturating_sub(1));
        self.footer_preset = idx;
        save_prefs(&self.current_prefs());
    }

    /// Recompute the live theme: the base preset, re-tinted by the reactive
    /// cover palette when reactive theming is enabled.
    pub(crate) fn apply_reactive(&mut self) {
        let Some(entry) = self.themes.get(self.theme_index) else {
            return;
        };
        let light = entry.light;
        let base = entry.theme;
        self.theme = match (self.reactive_theme, self.reactive_palette) {
            (true, Some(pal)) => derive_theme(&base, &pal, light, self.reactive_theme_intensity),
            _ => base,
        };
    }

    /// Cycle the reactive background wash strength between presets, persist,
    /// and re-apply the reactive theme so the change is visible immediately.
    pub(crate) fn cycle_reactive_intensity(&mut self) {
        const STEPS: [f32; 5] = [0.15, 0.25, 0.34, 0.45, 0.6];
        let cur = STEPS
            .iter()
            .position(|v| (v - self.reactive_theme_intensity).abs() < 1e-3);
        let next = match cur {
            Some(i) => (i + 1) % STEPS.len(),
            // Exact-match on a custom value: advance to the next preset, or
            // wrap back to the strongest one.
            None => STEPS
                .iter()
                .position(|v| *v > self.reactive_theme_intensity)
                .unwrap_or(0),
        };
        self.reactive_theme_intensity = STEPS[next];
        self.apply_reactive();
        save_prefs(&self.current_prefs());
        self.notify_silent(
            "Reactive Theme",
            format!("Intensity: {:.0}%", self.reactive_theme_intensity * 100.0),
            NotificationKind::Info,
        );
    }

    /// Kick off palette extraction for freshly received cover art.  Runs on
    /// a blocking thread; the result comes back through
    /// [`IpcResult::ReactivePalette`].
    pub(crate) fn request_reactive_palette(
        &self,
        cover_bytes: &[u8],
        ipc_tx: mpsc::UnboundedSender<IpcResult>,
    ) {
        let bytes = cover_bytes.to_vec();
        tokio::task::spawn_blocking(move || {
            let pal = extract_palette(&bytes);
            let _ = ipc_tx.send(IpcResult::ReactivePalette(pal));
        });
    }

    /// Cycle to the next theme in the list, persist, and refresh.
    pub(crate) fn toggle_theme(&mut self) {
        let next = (self.theme_index + 1) % self.themes.len();
        self.apply_theme_index(next);
        // A manual toggle opts out of OS-theme auto-detection until the user
        // returns to "auto" mode, so the OS preference can't fight the choice.
        self.theme_mode = "manual".to_string();
        let name = &self.themes[next].name;
        let light = if self.themes[next].light {
            " (light)"
        } else {
            ""
        };
        self.notify_silent(
            "Theme",
            format!("Theme: {}{}", name, light),
            NotificationKind::Info,
        );
    }

    /// Cycle the library track list sort order and persist the selection.
    pub(crate) fn cycle_track_sort(&mut self) {
        self.track_sort = self.track_sort.next();
        self.notify_silent(
            "Sort",
            format!("Sorting by: {}", self.track_sort.label()),
            NotificationKind::Info,
        );
        save_prefs(&self.current_prefs());
    }

    pub(crate) fn category_options(&self) -> usize {
        match self.settings_category {
            0 => 4,  // YouTube: Cookie Source, Cookie File, JS Runtime, Auto Download
            1 => 6,  // Playback: Repeat, Shuffle, Crossfade, EQ Enabled, Reverb, Cover Source
            2 => 17, // System: Theme, Transparent BG, Transparent Pickers, Sync Covers, Sync Lyrics, Sync Metadata, Footer Preset, Visualizer, Reactive Theme, Reactive Intensity, Hide Footer, Clear Lyrics Cache, Clear Cover Cache, Cover Cache Size, Notification Settings, Theme Mode, Audio Output
            3 => 11, // Spotify: Status, Account, Playlists, Link, Sync, Unlink, Device, Next, Previous, Shuffle, Repeat
            _ => 0,
        }
    }

    /// Cycle the visibility mode of the notification category selected in the
    /// `NotificationSettings` picker. +1 advances Floating → Footer → Off, -1
    /// reverses. Changes persist immediately through the normal prefs path.
    pub(crate) fn cycle_notification_mode(&mut self, dir: isize) {
        let Some(top) = self.pickers.top() else {
            return;
        };
        let idx = top.selected.min(NotifType::ALL.len() - 1);
        let ntype = NotifType::ALL[idx];
        let pos = NotifMode::ALL
            .iter()
            .position(|m| {
                self.notification_modes
                    .get(&ntype)
                    .copied()
                    .unwrap_or(NotifMode::Floating)
                    == *m
            })
            .unwrap_or(0) as isize;
        let next = NotifMode::ALL
            .get(((pos + dir).rem_euclid(NotifMode::ALL.len() as isize)) as usize)
            .copied()
            .unwrap_or(NotifMode::Floating);
        self.notification_modes.insert(ntype, next);
        save_prefs(&self.current_prefs());
        self.notify_typed(
            "System",
            format!("{} notifications: {}", ntype.label(), next.label()),
            NotificationKind::Info,
            true,
            NotifType::System,
        );
    }

    /// Open the OS output-device picker and fetch the device list. The list
    /// arrives asynchronously, so the picker renders "System default" alone
    /// until the daemon answers.
    pub(crate) fn open_audio_device_picker(&mut self) {
        self.audio_devices.clear();
        // Preselect the row that matches the saved device so Enter is a no-op
        // rather than an accidental switch to "System default".
        let current = self.state.audio.audio_device.clone();
        let selected = match &current {
            None => 0,
            Some(cur) => match self.audio_devices.iter().position(|d| d == cur) {
                Some(i) => i + 1,
                None => 0,
            },
        };
        self.pickers.open(PickerId::AudioDevice);
        if let Some(top) = self.pickers.top_mut() {
            top.selected = selected;
        }
        let c = self.client.clone();
        let ipc_tx = self.ipc_tx.clone();
        let tx = self.cmd_tx();
        let _ = tx.try_send(TuiCommand::fire(move || async move {
            match c.list_audio_devices().await {
                Ok(devices) => {
                    let _ = ipc_tx.send(IpcResult::AudioDevices(devices));
                }
                Err(e) => self_err(&ipc_tx, format!("audio devices: {e}")),
            }
        }));
    }

    /// Apply a choice from the output-device picker. Row 0 is "System
    /// default", which clears the saved device so the mixer opens the platform
    /// sink.
    pub(crate) fn apply_audio_device(&mut self, index: usize) {
        let name = if index == 0 {
            None
        } else {
            self.audio_devices.get(index - 1).cloned()
        };
        if name.is_none() && index > 0 {
            // The list had not arrived yet, or the index is stale.
            return;
        }
        let label = name.clone().unwrap_or_else(|| "system default".into());
        self.pickers.close_top();
        let c = self.client.clone();
        let ipc_tx = self.ipc_tx.clone();
        let tx = self.cmd_tx();
        let _ = tx.try_send(TuiCommand::fire(move || async move {
            match c.set_audio_device(name).await {
                Ok(()) => {
                    let _ = ipc_tx.send(IpcResult::Notification(
                        "Audio".to_string(),
                        format!("Output: {label}"),
                        NotificationKind::Success,
                        NotifType::Prefs,
                    ));
                }
                Err(e) => self_err(&ipc_tx, format!("audio output: {e}")),
            }
        }));
    }
}
