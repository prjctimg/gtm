use crate::app::*;
use crate::audio::eq::{PRE_GAIN_MAX_DB, PRE_GAIN_MIN_DB, PRE_GAIN_STEP_DB};
use crate::reactive::{ReactivePalette, extract_palette, from_hex};
use crate::ui::pre_gain_label;

/// The palette the live theme should be derived from.
///
/// The track on air always wins: its artwork is the one the playback surface
/// names. Only while nothing is playing does the highlighted playlist row
/// supply the palette, so scrolling a list previews each track's colours
/// without making them fight a real track's theme. Everywhere else the live
/// palette stays up, which is the idle behaviour every non-playlist view had
/// before the preview existed.
fn palette_in_effect<'a>(
    reactive_theme: bool,
    playing: bool,
    browsing_playlist: bool,
    live: Option<&'a ReactivePalette>,
    browse: Option<&'a ReactivePalette>,
) -> Option<&'a ReactivePalette> {
    if !reactive_theme {
        return None;
    }
    if browsing_playlist && !playing {
        browse
    } else {
        live
    }
}

impl App {
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
            daydream_secs: self.daydream_secs,
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
            discord_id: self.discord_id.clone(),
            auto_fetch_lyrics: self.auto_fetch_lyrics,
            icon_style: self.icon_style.clone(),
            hide_footer: self.hide_footer,
            left_pane_lists: self.left_pane_lists.clone(),
            show_preview: self.show_preview,
            preview_delay_secs: self.preview_delay_secs,
            show_welcome: self.show_welcome,
            resume_on_start: self.resume_on_start,
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

    /// Step the pre-gain by one dB, wrapping at the ends of its range, and push
    /// the value to the daemon so it takes effect on the track already playing.
    ///
    /// A cycle rather than a picker because the useful range is small and the
    /// interesting quantity is a number: `+3 dB` says what to do, where picking
    /// "Pre-Gain" from a list of the same five values would not.
    pub(crate) fn cycle_pre_gain(&mut self) {
        let next = (self.state.audio.pre_gain_db + PRE_GAIN_STEP_DB)
            .clamp(PRE_GAIN_MIN_DB, PRE_GAIN_MAX_DB);
        // Wrapping rather than sticking at the end, so a held key keeps moving
        // rather than sitting on the clamp. A 0 dB default means the first press
        // is always the same step, whichever direction the user expects.
        let next = if next > PRE_GAIN_MAX_DB {
            PRE_GAIN_MIN_DB
        } else {
            next
        };
        self.state.audio.pre_gain_db = next;
        let c = self.client.clone();
        tokio::spawn(async move {
            let _ = c.set_pre_gain(next).await;
        });
        save_prefs(&self.current_prefs());
        self.notify_typed(
            "System",
            format!("Pre-gain: {}", pre_gain_label(next)),
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
        let pal = palette_in_effect(
            self.reactive_theme,
            self.state.current_track.is_some(),
            self.browse_target_is_playlist(),
            self.reactive_palette.as_ref(),
            self.browse_palette.as_ref(),
        );
        let theme = match pal {
            Some(pal) => derive_theme(&base, pal, light, self.reactive_theme_intensity),
            None => base,
        };
        self.theme = theme;
    }

    /// Whether the highlighted row is a track inside the Playlists view.
    ///
    /// The view has three levels: groups, a group's rows, and the tracks of a
    /// named playlist. Only the last two carry tracks; the group list and the
    /// opened-playlist list are names, and neither has a palette to preview.
    fn browse_target_is_playlist(&self) -> bool {
        self.library_category == LIB_PLAYLISTS && self.track_info_kind() == TrackInfoKind::Track
    }

    /// Whether the highlighted playlist row is the current reactive source.
    pub(crate) fn browse_theme_active(&self) -> bool {
        self.reactive_theme
            && self.state.current_track.is_none()
            && self.browse_target_is_playlist()
    }

    /// Re-read the highlighted playlist row's cover into the preview palette.
    ///
    /// Called once per frame because the inputs are asynchronous: the cursor
    /// move sets the row key, the cover reply lands later, and playback can
    /// start or stop without either. The stored key makes the common case a
    /// comparison rather than another image decode; extraction only happens
    /// when the cover belongs to a row whose palette is not already in use.
    ///
    /// A blank cover while a request is still in flight keeps the previous
    /// palette, so fast scrolling does not flash the base theme between rows.
    /// Once the row settles without artwork, the palette is cleared and the
    /// base preset returns.
    pub(crate) fn sync_browse_palette(&mut self) {
        if !self.browse_theme_active() {
            self.clear_browse_palette();
            return;
        }
        let Some(key) = self.browse_cover_key.clone() else {
            self.clear_browse_palette();
            return;
        };
        let Some(bytes) = self.track_popup_cover.as_deref() else {
            if self.popup_slot.version.is_some() || self.spotify_popup_slot.version.is_some() {
                return;
            }
            // A terminal without an image protocol never fetches the card's
            // cover for display, so the palette needs its own one-shot fetch.
            // The latch stops a row with no artwork from being re-asked every
            // frame; a row change clears it in `update_track_popup`.
            if no_image_protocol()
                && self.track_popup_visible
                && self.browse_cover_tried.as_deref() != Some(key.as_str())
            {
                self.update_track_popup();
                if self.browse_cover_key.as_deref() == Some(key.as_str()) {
                    self.browse_cover_tried = Some(key);
                }
                return;
            }
            self.clear_browse_palette();
            return;
        };
        if self.browse_palette_key.as_deref() == Some(key.as_str()) {
            return;
        }
        let pal = extract_palette(bytes);
        let changed = self.browse_palette != pal;
        self.browse_palette = pal;
        self.browse_palette_key = Some(key);
        if changed {
            self.apply_reactive();
            self.data_dirty = true;
        }
    }

    /// Drop a preview palette and put whatever now owns the theme back.
    fn clear_browse_palette(&mut self) {
        if self.browse_palette.is_none() && self.browse_palette_key.is_none() {
            return;
        }
        self.browse_palette = None;
        self.browse_palette_key = None;
        self.apply_reactive();
        self.data_dirty = true;
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

    /// Adopt the palette the daemon extracted for the track on air.
    ///
    /// The daemon is the single extractor: it holds the cover bytes for the
    /// current track already, so the TUI never decodes an image to find a
    /// color. The event arrives on the same stream as every other track fact,
    /// and `gtm.nvim` reads the same broadcast off the pulse socket.
    ///
    /// All three empty means the daemon had no art to read, which is not the
    /// same as "the artwork is black": the previous album's palette stays up
    /// rather than the chrome flashing to a default.
    pub(crate) fn take_palette(&mut self, primary: &str, secondary: &str, tertiary: &str) {
        if !self.reactive_theme {
            return;
        }
        let pal = match (from_hex(primary), from_hex(secondary), from_hex(tertiary)) {
            (Some(p), Some(s), Some(t)) => Some(ReactivePalette {
                primary: p,
                secondary: s,
                tertiary: t,
            }),
            _ => None,
        };
        if self.reactive_palette != pal {
            self.reactive_palette = pal;
            self.apply_reactive();
        }
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
        crate::ui::pickers::settings_rows::rows_for(self.settings_category).len()
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
    pub(crate) fn open_audio_picker(&mut self) {
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

#[cfg(test)]
mod tests {
    use super::*;

    fn palette(primary: [u8; 3]) -> ReactivePalette {
        ReactivePalette {
            primary,
            secondary: [0, 0, 0],
            tertiary: [0, 0, 0],
        }
    }

    #[test]
    fn an_idle_playlist_previews_the_highlighted_row() {
        let live = palette([255, 0, 0]);
        let browse = palette([0, 0, 255]);

        let got = palette_in_effect(true, false, true, Some(&live), Some(&browse));

        assert_eq!(got, Some(&browse));
    }

    #[test]
    fn a_playing_track_keeps_its_own_palette() {
        let live = palette([255, 0, 0]);
        let browse = palette([0, 0, 255]);

        let got = palette_in_effect(true, true, true, Some(&live), Some(&browse));

        assert_eq!(
            got,
            Some(&live),
            "playback must win over the row under the cursor"
        );
    }

    #[test]
    fn an_idle_non_playlist_keeps_the_last_live_palette() {
        let live = palette([255, 0, 0]);

        let got = palette_in_effect(true, false, false, Some(&live), None);

        assert_eq!(
            got,
            Some(&live),
            "leaving the playlist must not reset the existing idle theme"
        );
    }

    #[test]
    fn a_disabled_reactive_theme_ignores_every_palette() {
        let live = palette([255, 0, 0]);
        let browse = palette([0, 0, 255]);

        assert_eq!(
            palette_in_effect(false, true, true, Some(&live), Some(&browse)),
            None
        );
        assert_eq!(
            palette_in_effect(false, false, true, Some(&live), Some(&browse)),
            None
        );
    }
}
