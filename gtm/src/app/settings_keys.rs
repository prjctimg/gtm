// Copyright (c) 2026
// Author: prjctimg <prjctimg@outlook.com>
// Settings pane key handling
//
// This is free software released under the GPL-3.0 license.

//! Key handling for the settings pane.
//!
//! This is one function rather than four hundred lines inline in the dispatch
//! chain, for one reason: the row list and this handler had to agree, and three
//! separate `match`es over row numbers is how they stopped. The Spotify
//! transport rows were the visible symptom — the handler passed `opt - 4` as a
//! semantic index, the callee matched `8 | 9 | 10 | _`, and all three of Next,
//! Previous and Shuffle ran `set_repeat`.
//!
//! Every `opt` below is now a position in
//! [`settings_rows::PLAYBACK_ROWS`] / `SYSTEM_ROWS` / `SPOTIFY_ROWS`, checked by
//! a test against those arrays.

use crate::app::*;

impl App {
    pub(crate) fn settings_key(
        &mut self,
        key: event::KeyEvent,
        tx: &tokio::sync::mpsc::Sender<TuiCommand>,
    ) {
        let focus = self.settings_pane_focus;
        let category = self.settings_category;
        let opt = self.settings_option;
        match key.code {
            event::KeyCode::Esc => {
                self.pickers.close_top();
            }
            event::KeyCode::Tab => {
                self.settings_pane_focus = !focus;
            }
            // ← and → did exactly the same thing for every row: each was a
            // two-state toggle, so there was nothing to step through. They were
            // the same 45 lines twice. The pane header used to advertise
            // "←/→: cycle values"; it now says nothing, because a toggle has
            // no direction.
            event::KeyCode::Left
            | event::KeyCode::Right
            | event::KeyCode::Char('h')
            | event::KeyCode::Char('l')
                if !focus =>
            {
                self.settings_adjust(opt, tx);
            }
            event::KeyCode::Up | event::KeyCode::Char('k') => {
                if focus {
                    self.settings_category = category.saturating_sub(1);
                    self.settings_option = 0;
                } else {
                    self.settings_option = opt.saturating_sub(1);
                }
            }
            event::KeyCode::Down | event::KeyCode::Char('j') => {
                if focus {
                    self.settings_category = (category + 1).min(NUM_SETTINGS_CATEGORIES - 1);
                    self.settings_option = 0;
                } else {
                    let max = self.category_options().saturating_sub(1);
                    self.settings_option = (opt + 1).min(max);
                }
            }
            event::KeyCode::Enter if !focus => self.settings_activate(opt, tx),
            _ => {}
        }
    }

    /// Flip the value of the highlighted row. Every row this reaches is a
    /// two-state one, which is why ← and → can share it.
    fn settings_adjust(&mut self, opt: usize, tx: &tokio::sync::mpsc::Sender<TuiCommand>) {
        match self.settings_category {
            0 => match opt {
                0 => self.set_repeat(self.next_repeat(), tx),
                1 => {
                    let next = !self.state.shuffle;
                    self.state.shuffle = next;
                    let c = self.client.clone();
                    spawn(tx, move || async move {
                        let _ = c.toggle_shuffle().await;
                    });
                }
                3 => {
                    let next = !self.state.audio.eq_enabled;
                    self.state.audio.eq_enabled = next;
                    let c = self.client.clone();
                    spawn(tx, move || async move {
                        let _ = c.set_eq_enabled(next).await;
                    });
                }
                4 => {
                    let next = !self.state.audio.reverb.enabled;
                    let room = self.state.audio.reverb.room_size;
                    self.state.audio.reverb.enabled = next;
                    let c = self.client.clone();
                    spawn(tx, move || async move {
                        let _ = c.set_reverb(next, room).await;
                    });
                }
                5 => self.cycle_pre_gain(),
                6 => self.cycle_cover_provider(),
                _ => {}
            },
            1 => match opt {
                3 => {
                    self.transparent_bg = !self.transparent_bg;
                    save_prefs(&self.current_prefs());
                }
                4 => {
                    self.transparent_pickers = !self.transparent_pickers;
                    save_prefs(&self.current_prefs());
                }
                5 => self.toggle_hide_footer(),
                6 => self.toggle_reactive_theme(tx),
                7 => self.cycle_reactive_intensity(),
                _ => {}
            },
            _ => {}
        }
    }

    /// Run the highlighted row's action.
    fn settings_activate(&mut self, opt: usize, tx: &tokio::sync::mpsc::Sender<TuiCommand>) {
        match self.settings_category {
            0 => match opt {
                0 => self.set_repeat(self.next_repeat(), tx),
                1 => {
                    let next = !self.state.shuffle;
                    self.state.shuffle = next;
                    let c = self.client.clone();
                    spawn(tx, move || async move {
                        let _ = c.toggle_shuffle().await;
                    });
                }
                2 => self.pickers.open(PickerId::Crossfade),
                3 => {
                    let next = !self.state.audio.eq_enabled;
                    self.state.audio.eq_enabled = next;
                    let c = self.client.clone();
                    spawn(tx, move || async move {
                        let _ = c.set_eq_enabled(next).await;
                    });
                }
                4 => {
                    let next = !self.state.audio.reverb.enabled;
                    let room = self.state.audio.reverb.room_size;
                    self.state.audio.reverb.enabled = next;
                    let c = self.client.clone();
                    spawn(tx, move || async move {
                        let _ = c.set_reverb(next, room).await;
                    });
                }
                5 => self.cycle_cover_provider(),
                _ => {}
            },
            1 => match opt {
                0 => self.pickers.open(PickerId::ThemePicker),
                1 => self.cycle_theme_mode(),
                2 => self.open_audio_picker(),
                3 => {
                    self.transparent_bg = !self.transparent_bg;
                    save_prefs(&self.current_prefs());
                }
                4 => {
                    self.transparent_pickers = !self.transparent_pickers;
                    save_prefs(&self.current_prefs());
                }
                5 => self.toggle_hide_footer(),
                6 => self.toggle_reactive_theme(tx),
                7 => self.cycle_reactive_intensity(),
                8 => self.pickers.open(PickerId::VisualizerPreset),
                9 => self.cycle_daydream(),
                10 => self.pickers.open(PickerId::FooterPreset),
                11 => sync_and_wait(
                    self.client.clone(),
                    SyncKind::Covers,
                    "Covers",
                    self.ipc_tx.clone(),
                ),
                12 => sync_and_wait(
                    self.client.clone(),
                    SyncKind::Lyrics,
                    "Lyrics",
                    self.ipc_tx.clone(),
                ),
                13 => sync_and_wait(
                    self.client.clone(),
                    SyncKind::Metadata,
                    "Metadata",
                    self.ipc_tx.clone(),
                ),
                14 | 15 => {
                    let (what, label) = if opt == 14 {
                        (CacheKind::Lyrics, "lyrics")
                    } else {
                        (CacheKind::Covers, "cover art")
                    };
                    let c = self.client.clone();
                    let ipc = self.ipc_tx.clone();
                    spawn(tx, move || async move {
                        match c.clear_cache(what).await {
                            Ok(()) => {
                                let _ = ipc.send(IpcResult::Notification(
                                    "Cache".to_string(),
                                    format!("Cleared {label} cache"),
                                    NotificationKind::Success,
                                    NotifType::Prefs,
                                ));
                            }
                            Err(e) => {
                                let _ =
                                    ipc.send(IpcResult::Error(format!("Clear {label} cache: {e}")));
                            }
                        }
                    });
                }
                16 => self.open_settings_overlay(),
                _ => {}
            },
            2 => match opt {
                0 => {} // Status is display-only.
                1 => self.open_spotify_link(),
                2 => self.unlink_spotify(tx.clone()),
                _ => {}
            },
            _ => {}
        }
    }

    /// Step the idle time before the visualizer takes over.
    ///
    /// Cycles through a ladder rather than typing a number: the useful choices
    /// are "soon", "after a while" and "never", and off has to be reachable
    /// without editing the config file.
    fn cycle_daydream(&mut self) {
        const STEPS: [u64; 5] = [15, 30, 60, 180, 0];
        let cur = self.daydream_secs;
        let next = STEPS
            .iter()
            .position(|s| *s == cur)
            .map_or(STEPS[0], |i| STEPS[(i + 1) % STEPS.len()]);
        self.daydream_secs = next;
        save_prefs(&self.current_prefs());
    }

    fn next_repeat(&self) -> RepeatMode {
        match self.state.repeat {
            RepeatMode::Off => RepeatMode::One,
            RepeatMode::One => RepeatMode::All,
            RepeatMode::All => RepeatMode::Off,
        }
    }

    /// The daemon is told through the command channel, because repeat is a
    /// transport control the daemon has to apply before the next track change.
    fn set_repeat(&mut self, next: RepeatMode, tx: &tokio::sync::mpsc::Sender<TuiCommand>) {
        self.state.repeat = next;
        let c = self.client.clone();
        spawn(tx, move || async move {
            let _ = c.cycle_repeat(next).await;
        });
    }

    /// Turn the reactive theme on or off, fetching a palette the first time it
    /// is switched on with no artwork to read one from.
    fn toggle_reactive_theme(&mut self, tx: &tokio::sync::mpsc::Sender<TuiCommand>) {
        self.reactive_theme = !self.reactive_theme;
        if self.reactive_theme && self.reactive_palette.is_none() {
            if let Some(c) = self.np_cover.image.clone() {
                let itx = self.ipc_tx.clone();
                self.reactive_gen = Some(self.next_cover_gen());
                let pal_gen = self.reactive_gen.expect("just set");
                self.request_reactive_palette(&c, pal_gen, itx);
            } else if let Some(tid) = self.state.current_track.as_ref().map(|t| t.id) {
                let fetch_gen = self.next_cover_gen();
                self.np_cover.pending_gen = Some(fetch_gen);
                let client = self.client.clone();
                let ipc = self.ipc_tx.clone();
                spawn(tx, move || async move {
                    if let Ok(Some(b64)) = client.art().cover(tid).await
                        && let Ok(bytes) = base64::engine::general_purpose::STANDARD.decode(&b64)
                    {
                        let _ = ipc.send(IpcResult::CoverArt(Some(bytes), Some(tid), fetch_gen));
                    }
                });
            }
        }
        self.apply_reactive();
        save_prefs(&self.current_prefs());
    }
}

/// Fire a daemon command on the priority channel, dropping it if the channel is
/// closed. `try_send` rather than `send`: a full queue means the daemon is
/// behind, and blocking the key handler on it is worse than losing a redundant
/// command.
fn spawn<F, Fut>(tx: &tokio::sync::mpsc::Sender<TuiCommand>, f: F)
where
    F: FnOnce() -> Fut + Send + 'static,
    Fut: std::future::Future<Output = ()> + Send + 'static,
{
    let _ = tx.try_send(TuiCommand::fire(f));
}
