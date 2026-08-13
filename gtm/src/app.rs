use std::path::Path;
use std::time::Duration;

use crossterm::event::{self, Event, KeyCode, KeyEventKind};
use gtm_core::client::DaemonClient;
use gtm_core::ipc::DaemonRes;
use gtm_core::state::{DaemonState, PlaybackStatus, RepeatMode, Tab};
use gtm_core::track::TrackInfo;
use ratatui::Terminal;
use tokio::sync::mpsc;

use base64::Engine;

use crate::keymap::{default_keybindings, KeyContext, KeyboardAction};
use crate::overlay::{OverlayCtx, OverlayId, OverlayManager};
use crate::theme::AppTheme;
use crate::ui;

pub const NUM_SETTINGS_CATEGORIES: usize = 5;
pub const LIBRARY_CATEGORIES: &[&str] = &[
    "All Tracks",
    "Albums",
    "Artists",
    "Playlists",
    "Recently Added",
    "Most Played",
    "Least Played",
    "Spotify",
    "Downloads",
];

pub enum InputMode {
    Normal,
    Searching,
    Command,
}

#[derive(Debug, Clone)]
pub enum NotificationKind {
    Info,
    Success,
    Warning,
    Error,
}

#[derive(Debug, Clone)]
pub struct Notification {
    pub message: String,
    pub kind: NotificationKind,
    pub expires_at: std::time::Instant,
}

#[allow(dead_code)]
pub struct App {
    pub theme: AppTheme,
    pub client: DaemonClient,
    pub state: DaemonState,
    pub current_tab: Tab,
    pub input_mode: InputMode,
    pub search_query: String,
    pub scroll_offset: usize,
    pub library_category: usize,
    pub library_pane_focus: bool,
    pub settings_category: usize,
    pub settings_option_scroll: usize,
    pub tracks_cache: Vec<TrackInfo>,
    pub queue_cache: Vec<TrackInfo>,
    pub queue_cursor: usize,
    pub yt_results_cache: Vec<gtm_core::track::YTSearchResult>,
    pub volume_input: String,
    pub playlist_cache: Vec<gtm_core::track::Playlist>,
    pub error_message: Option<String>,
    pub status_message: Option<String>,
    pub notifications: Vec<Notification>,
    pub crossfade_duration: u8,
    pub pending_volume: Option<u8>,
    pub overlays: OverlayManager,
    pub sleep_timer_remaining: Option<u64>,
    pub playback_speed: f64,
    pub current_cover: Option<Vec<u8>>,
    pub last_cover_track_id: Option<i64>,
    pub cmd_rx: mpsc::Receiver<TuiCommand>,
    cmd_tx: mpsc::Sender<TuiCommand>,
    keybindings: crate::keymap::Keybindings,
}

#[allow(dead_code)]
pub enum TuiCommand {
    Play(String),
    PlayPause,
    Pause,
    Stop,
    Next,
    Prev,
    Seek(f64),
    SetVolume(u8),
    ToggleShuffle,
    CycleRepeat(RepeatMode),
    ToggleMute,
    Crossfade(bool, u8),
    QueueAdd(String),
    QueueRemove(u128),
    QueueClear,
    YtSearch(String),
    YtResolve(String),
    Search(String),
    AddFavourite(i64),
    RemoveFavourite(i64),
    Refresh,
    RefreshLibrary,
    RefreshQueue,
    RefreshPlaylists,
    RefreshYt,
}

impl App {
    pub async fn new(socket_path: &Path) -> Result<Self, Box<dyn std::error::Error>> {
        let client = DaemonClient::connect(socket_path).await?;
        let state = DaemonState::new();
        let (cmd_tx, cmd_rx) = mpsc::channel(64);
        let keybindings = default_keybindings();
        Ok(Self {
            theme: AppTheme::default(),
            client,
            state,
            current_tab: Tab::NowPlaying,
            input_mode: InputMode::Normal,
            search_query: String::new(),
            scroll_offset: 0,
            library_category: 0,
            library_pane_focus: false,
            settings_category: 0,
            settings_option_scroll: 0,
            tracks_cache: Vec::new(),
            queue_cache: Vec::new(),
            queue_cursor: 0,
            yt_results_cache: Vec::new(),
            volume_input: String::new(),
            playlist_cache: Vec::new(),
            error_message: None,
            status_message: None,
            notifications: Vec::new(),
            crossfade_duration: 7,
            pending_volume: None,
            overlays: OverlayManager::new(),
            sleep_timer_remaining: None,
            playback_speed: 1.0,
            current_cover: None,
            last_cover_track_id: None,
            cmd_rx,
            cmd_tx,
            keybindings,
        })
    }

    pub fn cmd_tx(&self) -> mpsc::Sender<TuiCommand> {
        self.cmd_tx.clone()
    }

    #[allow(dead_code)]
    pub fn overlay_ctx(&self) -> OverlayCtx<'_> {
        OverlayCtx {
            state: &self.state,
            tracks_cache: &self.tracks_cache,
            queue_cache: &self.queue_cache,
            queue_cursor: self.queue_cursor,
            yt_results_cache: &self.yt_results_cache,
            playlist_cache: &self.playlist_cache,
            op: &self.overlays,
        }
    }

    pub async fn run(
        mut self,
        terminal: &mut Terminal<ratatui::backend::CrosstermBackend<std::io::Stdout>>,
    ) -> Result<(), Box<dyn std::error::Error>> {
        self.fetch_state().await;
        // Clamp volume to safe level on startup so the safety prompt
        // doesn't appear immediately when the TUI opens.
        if self.state.volume > 85 {
            let _ = self.client.set_volume(85).await;
            self.state.volume = 85;
        }
        self.fetch_queue().await;

        let cmd_tx = self.cmd_tx();
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(Duration::from_millis(250)).await;
                let _ = cmd_tx.send(TuiCommand::Refresh).await;
            }
        });

        loop {
            for ev in self.client.drain().await {
                self.state.apply_event(&ev);
            }

            while let Ok(cmd) = self.cmd_rx.try_recv() {
                self.handle_command(cmd).await;
            }

            // Expire stale notifications
            let now = std::time::Instant::now();
            self.notifications.retain(|n| n.expires_at > now);

            terminal.draw(|f| ui::render(f, &mut self))?;

            if event::poll(Duration::from_millis(50))? {
                if let Event::Key(key) = event::read()? {
                    if key.kind == KeyEventKind::Press {
                        if !self.handle_key(key).await {
                            break;
                        }
                    }
                }
            }
        }
        Ok(())
    }

    pub fn notify(&mut self, message: impl Into<String>, kind: NotificationKind) {
        let expires_at = std::time::Instant::now() + std::time::Duration::from_secs(4);
        self.notifications.push(Notification {
            message: message.into(),
            kind,
            expires_at,
        });
    }

    async fn fetch_state(&mut self) {
        if let Ok(state) = self.client.get_status().await {
            self.state = state;
            // Fetch cover art if current track changed
            let track_id = self.state.current_track.as_ref().map(|t| t.id);
            if track_id != self.last_cover_track_id {
                self.last_cover_track_id = track_id;
                if let Some(tid) = track_id {
                    if let Ok(Some(b64)) = self.client.get_cover_art(tid).await {
                        if let Ok(bytes) = base64::engine::general_purpose::STANDARD.decode(&b64) {
                            self.current_cover = Some(bytes);
                        }
                    }
                } else {
                    self.current_cover = None;
                }
            }
        }
    }

    async fn fetch_queue(&mut self) {
        if let Ok(DaemonRes::QueueState { tracks, cursor, .. }) =
            self.client.queue_list().await
        {
            self.queue_cache = tracks;
            self.queue_cursor = cursor as usize;
        }
    }

    async fn handle_command(&mut self, cmd: TuiCommand) {
        match cmd {
            TuiCommand::Play(path) => {
                if let Err(e) = self.client.play(&path, 0.0).await {
                    self.error_message = Some(e.to_string());
                }
            }
            TuiCommand::PlayPause => {
                if let Err(e) = self.client.play_pause().await {
                    self.error_message = Some(e.to_string());
                }
            }
            TuiCommand::Pause => {
                if let Err(e) = self.client.pause().await {
                    self.error_message = Some(e.to_string());
                }
            }
            TuiCommand::Stop => {
                if let Err(e) = self.client.stop().await {
                    self.error_message = Some(e.to_string());
                }
            }
            TuiCommand::Next => {
                if let Err(e) = self.client.next().await {
                    self.error_message = Some(e.to_string());
                }
            }
            TuiCommand::Prev => {
                if let Err(e) = self.client.prev().await {
                    self.error_message = Some(e.to_string());
                }
            }
            TuiCommand::Seek(pos) => {
                if let Err(e) = self.client.seek(pos).await {
                    self.error_message = Some(e.to_string());
                }
            }
            TuiCommand::SetVolume(v) => {
                if let Err(e) = self.client.set_volume(v).await {
                    self.error_message = Some(e.to_string());
                }
            }
            TuiCommand::ToggleShuffle => {
                if let Err(e) = self.client.toggle_shuffle().await {
                    self.error_message = Some(e.to_string());
                }
            }
            TuiCommand::CycleRepeat(m) => {
                if let Err(e) = self.client.cycle_repeat(m).await {
                    self.error_message = Some(e.to_string());
                }
            }
            TuiCommand::ToggleMute => {
                if let Err(e) = self.client.toggle_mute().await {
                    self.error_message = Some(e.to_string());
                }
            }
            TuiCommand::Crossfade(en, dur) => {
                if let Err(e) = self.client.crossfade(en, dur).await {
                    self.error_message = Some(e.to_string());
                }
            }
            TuiCommand::QueueAdd(p) => {
                if let Err(e) = self.client.queue_add(&p, None).await {
                    self.error_message = Some(e.to_string());
                }
            }
            TuiCommand::QueueRemove(i) => {
                if let Err(e) = self.client.queue_rm(i).await {
                    self.error_message = Some(e.to_string());
                }
            }
            TuiCommand::QueueClear => {
                if let Err(e) = self.client.queue_clear().await {
                    self.error_message = Some(e.to_string());
                }
            }
            TuiCommand::YtSearch(q) => {
                if let Err(e) = self.client.yt_search(&q, None).await {
                    self.error_message = Some(e.to_string());
                }
            }
            TuiCommand::YtResolve(u) => {
                if let Err(e) = self.client.yt_resolve_stream(&u).await {
                    self.error_message = Some(e.to_string());
                }
            }
            TuiCommand::Search(q) => {
                if let Err(e) = self.client.search(&q).await {
                    self.error_message = Some(e.to_string());
                }
            }
            TuiCommand::AddFavourite(id) => {
                if let Err(e) = self.client.add_favourite(id).await {
                    self.error_message = Some(e.to_string());
                }
            }
            TuiCommand::RemoveFavourite(id) => {
                if let Err(e) = self.client.remove_favourite(id).await {
                    self.error_message = Some(e.to_string());
                }
            }
            TuiCommand::Refresh => {
                for ev in self.client.drain().await {
                    self.state.apply_event(&ev);
                }
                self.fetch_state().await;
            }
            TuiCommand::RefreshLibrary => {
                if let Ok(DaemonRes::Tracks { tracks, .. }) =
                    self.client.library_get_tracks(None, None).await
                {
                    self.tracks_cache = tracks;
                }
            }
            TuiCommand::RefreshQueue => {
                self.fetch_queue().await;
            }
            TuiCommand::RefreshPlaylists => {
                if let Ok(DaemonRes::Playlists { playlists, .. }) =
                    self.client.library_get_playlists().await
                {
                    self.playlist_cache = playlists;
                }
            }
            TuiCommand::RefreshYt => {
                if let Ok(DaemonRes::YtSearchResults { results, .. }) =
                    self.client.yt_search_poll().await
                {
                    self.yt_results_cache = results;
                }
            }
        };
    }

    async fn handle_key(&mut self, key: event::KeyEvent) -> bool {
        // If an overlay is open, Esc closes it; keys pass through to overlay
        if self.overlays.is_open() {
            return match key.code {
                KeyCode::Esc => {
                    self.overlays.close_top();
                    true
                }
                _ => {
                    // Pass key to overlay handler
                    self.handle_overlay_key(key).await;
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
                    let q = self.search_query.clone();
                    self.input_mode = InputMode::Normal;
                    let tx = self.cmd_tx();
                    match self.current_tab {
                        Tab::Library => {
                            let _ = tx.send(TuiCommand::RefreshLibrary).await;
                        }
                        _ => {
                            let _ = tx.send(TuiCommand::Search(q)).await;
                        }
                    }
                    self.search_query.clear();
                }
                KeyCode::Char(c) => {
                    self.search_query.push(c);
                }
                KeyCode::Backspace => {
                    self.search_query.pop();
                }
                _ => {}
            },
            InputMode::Command => match key.code {
                KeyCode::Esc => {
                    self.input_mode = InputMode::Normal;
                }
                KeyCode::Enter => {
                    let cmd = self.search_query.clone().trim().to_lowercase();
                    self.input_mode = InputMode::Normal;
                    self.search_query.clear();
                    let tx = self.cmd_tx();
                    if cmd == "quit" || cmd == "q" {
                        return false;
                    }
                    if let Ok(vol) = cmd.parse::<u8>() {
                        if vol > 85 {
                            self.pending_volume = Some(vol);
                            self.overlays.open(OverlayId::VolumeConfirm);
                        } else {
                            let _ = tx.send(TuiCommand::SetVolume(vol)).await;
                        }
                    }
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
                match self.keybindings.dispatch(key, KeyContext::Normal) {
                    Some(KeyboardAction::Quit) => return false,
                    Some(KeyboardAction::NextTab) => {
                        // In Library/Settings tabs, Tab toggles pane focus first
                        match self.current_tab {
                            Tab::Library => {
                                self.library_pane_focus = !self.library_pane_focus;
                            }
                            Tab::Settings => {
                                self.settings_category = (self.settings_category + 1) % NUM_SETTINGS_CATEGORIES;
                            }
                            _ => {
                                self.current_tab = match self.current_tab {
                                    Tab::NowPlaying => Tab::Library,
                                    _ => Tab::NowPlaying,
                                };
                                self.refresh_tab().await;
                            }
                        }
                    }
                    Some(KeyboardAction::PrevTab) => {
                        match self.current_tab {
                            Tab::Library => {
                                self.library_pane_focus = !self.library_pane_focus;
                            }
                            Tab::Settings => {
                                self.settings_category = self.settings_category.saturating_sub(1);
                            }
                            _ => {
                                self.current_tab = Tab::Settings;
                                self.refresh_tab().await;
                            }
                        }
                    }
                    Some(KeyboardAction::SwitchTab(tab)) => {
                        self.current_tab = tab;
                        self.refresh_tab().await;
                    }
                    Some(KeyboardAction::OpenOverlay(id)) => {
                        self.overlays.open(id);
                    }
                    Some(KeyboardAction::PlayPause) => {
                        let tx = self.cmd_tx();
                        match self.state.status {
                            PlaybackStatus::Playing => {
                                let _ = tx.send(TuiCommand::Pause).await;
                            }
                            PlaybackStatus::Paused => {
                                if let Some(ref t) = self.state.current_track {
                                    let _ = tx
                                        .send(TuiCommand::Play(t.path.clone()))
                                        .await;
                                }
                            }
                            PlaybackStatus::Stopped => {
                                if !self.queue_cache.is_empty() {
                                    let idx =
                                        self.queue_cursor.min(self.queue_cache.len() - 1);
                                    let path = self.queue_cache[idx].path.clone();
                                    let _ = tx.send(TuiCommand::Play(path)).await;
                                }
                            }
                        }
                    }
                    Some(KeyboardAction::Next) => {
                        let tx = self.cmd_tx();
                        let _ = tx.send(TuiCommand::Next).await;
                    }
                    Some(KeyboardAction::Prev) => {
                        let tx = self.cmd_tx();
                        let _ = tx.send(TuiCommand::Prev).await;
                    }
                    Some(KeyboardAction::VolumeUp) | Some(KeyboardAction::SeekForward) => {
                        let new_vol = (self.state.volume + 5).min(100);
                        if new_vol > 85 {
                            self.pending_volume = Some(new_vol);
                            self.overlays.open(OverlayId::VolumeConfirm);
                        } else {
                            let tx = self.cmd_tx();
                            let _ = tx.send(TuiCommand::SetVolume(new_vol)).await;
                            self.notify(format!("Volume: {}%", new_vol), NotificationKind::Info);
                        }
                    }
                    Some(KeyboardAction::VolumeDown) | Some(KeyboardAction::SeekBackward) => {
                        let tx = self.cmd_tx();
                        let new_vol = self.state.volume.saturating_sub(5);
                        let _ = tx.send(TuiCommand::SetVolume(new_vol)).await;
                        self.notify(format!("Volume: {}%", new_vol), NotificationKind::Info);
                    }
                    Some(KeyboardAction::ToggleMute) => {
                        let tx = self.cmd_tx();
                        let _ = tx.send(TuiCommand::ToggleMute).await;
                        if self.state.mute {
                            self.notify("Unmuted", NotificationKind::Info);
                        } else {
                            self.notify("Muted", NotificationKind::Warning);
                        }
                    }
                    Some(KeyboardAction::CycleRepeat) => {
                        let tx = self.cmd_tx();
                        let new_mode = match self.state.repeat {
                            RepeatMode::Off => RepeatMode::One,
                            RepeatMode::One => RepeatMode::All,
                            RepeatMode::All => RepeatMode::Off,
                        };
                        let _ = tx.send(TuiCommand::CycleRepeat(new_mode)).await;
                        self.notify(format!("Repeat: {:?}", new_mode), NotificationKind::Info);
                    }
                    Some(KeyboardAction::ToggleShuffle) => {
                        let tx = self.cmd_tx();
                        let _ = tx.send(TuiCommand::ToggleShuffle).await;
                        if self.state.shuffle {
                            self.notify("Shuffle OFF", NotificationKind::Info);
                        } else {
                            self.notify("Shuffle ON", NotificationKind::Info);
                        }
                    }
                    Some(KeyboardAction::EnterFilter) => {
                        self.input_mode = InputMode::Searching;
                    }
                    Some(KeyboardAction::EnterCommand) => {
                        self.input_mode = InputMode::Command;
                    }
                    Some(KeyboardAction::MoveUp) => {
                        if self.current_tab == Tab::Library && self.library_pane_focus {
                            self.library_category = self.library_category.saturating_sub(1);
                        } else {
                            self.scroll_offset = self.scroll_offset.saturating_sub(1);
                        }
                    }
                    Some(KeyboardAction::MoveDown) => {
                        if self.current_tab == Tab::Library && self.library_pane_focus {
                            self.library_category = (self.library_category + 1).min(LIBRARY_CATEGORIES.len() - 1);
                        } else {
                            self.scroll_offset += 1;
                        }
                    }
                    Some(KeyboardAction::Select) => {
                        if self.current_tab == Tab::Library {
                            if self.library_pane_focus {
                                // Left pane: move focus to right pane
                                self.library_pane_focus = false;
                            } else {
                                // Right pane: play selected track
                                if self.scroll_offset < self.tracks_cache.len() {
                                    let path = self.tracks_cache[self.scroll_offset].path.clone();
                                    let tx = self.cmd_tx();
                                    let _ = tx.send(TuiCommand::Play(path)).await;
                                }
                            }
                        }
                    }
                    Some(KeyboardAction::Delete) => {}
                    None => {
                        match key.code {
                            KeyCode::Char('q') | KeyCode::Esc => return false,
                            KeyCode::Char('c') if self.current_tab == Tab::Settings => {
                                // Toggle crossfade in Settings tab
                                let enabled = !self.state.crossfade
                                    .as_ref()
                                    .map(|c| c.enabled)
                                    .unwrap_or(false);
                                let dur = self.state.crossfade
                                    .as_ref()
                                    .map(|c| c.duration_secs)
                                    .unwrap_or(self.crossfade_duration);
                                let tx = self.cmd_tx();
                                let _ = tx.send(TuiCommand::Crossfade(enabled, dur)).await;
                            }
                            KeyCode::Char('C') if self.current_tab == Tab::Settings => {
                                // Cycle crossfade duration (3, 5, 7, 10, 15, 30)
                                let dur = self.state.crossfade
                                    .as_ref()
                                    .map(|c| c.duration_secs)
                                    .unwrap_or(self.crossfade_duration);
                                let new_dur = match dur {
                                    0..=3 => 5,
                                    4..=7 => 10,
                                    8..=14 => 15,
                                    15..=29 => 30,
                                    _ => 3,
                                };
                                let enabled = self.state.crossfade
                                    .as_ref()
                                    .map(|c| c.enabled)
                                    .unwrap_or(true);
                                let tx = self.cmd_tx();
                                let _ = tx.send(TuiCommand::Crossfade(enabled, new_dur)).await;
                            }
                            _ => {}
                        }
                    }
                    _ => {}
                }
            }
        }
        true
    }

    async fn handle_overlay_key(&mut self, key: event::KeyEvent) {
        let tx = self.cmd_tx();
        match key.code {
            KeyCode::Esc => {
                if let Some(top) = self.overlays.top() {
                    if top.id == OverlayId::SleepTimer {
                        self.sleep_timer_remaining = None;
                    }
                }
                self.pending_volume = None;
                self.overlays.close_top();
            }
            KeyCode::Up | KeyCode::Char('k') => {
                if let Some(top) = self.overlays.top_mut() {
                    top.selected = top.selected.saturating_sub(1);
                }
            }
            KeyCode::Down | KeyCode::Char('j') => {
                if let Some(top) = self.overlays.top_mut() {
                    top.selected += 1;
                }
            }
            KeyCode::Enter => {
                // Dispatch based on overlay type
                if let Some(top) = self.overlays.top() {
                    match top.id {
                        OverlayId::Queue => {
                            if !self.queue_cache.is_empty() {
                                let idx = top.selected.min(self.queue_cache.len() - 1);
                                let path = self.queue_cache[idx].path.clone();
                                let _ = tx.send(TuiCommand::Play(path)).await;
                            }
                        }
                        OverlayId::YTSearch => {
                            if top.query.is_empty() {
                                // Start search
                            } else if !self.yt_results_cache.is_empty() {
                                let idx = top.selected.min(self.yt_results_cache.len() - 1);
                                let url = self.yt_results_cache[idx].url.clone();
                                let _ = tx.send(TuiCommand::YtResolve(url)).await;
                            }
                        }
                        OverlayId::VolumeConfirm => {
                            // Volume safety confirmed
                            if let Some(v) = self.pending_volume.take() {
                                let _ = tx.send(TuiCommand::SetVolume(v)).await;
                            }
                            self.overlays.close_top();
                        }
                        OverlayId::SleepTimer => {
                            // Set sleep timer from selected preset
                            let presets = [5u64, 10, 15, 30, 60];
                            let idx = top.selected.min(presets.len() - 1);
                            self.sleep_timer_remaining = Some(presets[idx]);
                            self.overlays.close_top();
                        }
                        OverlayId::CommandPalette => {
                            self.overlays.close_top();
                        }
                        OverlayId::Equalizer => {
                            // Apply selected EQ preset
                            let presets = [
                                gtm_core::state::EqPreset::Flat,
                                gtm_core::state::EqPreset::Pop,
                                gtm_core::state::EqPreset::Rock,
                                gtm_core::state::EqPreset::Jazz,
                                gtm_core::state::EqPreset::Classical,
                                gtm_core::state::EqPreset::Bass,
                                gtm_core::state::EqPreset::Vocal,
                            ];
                            let idx = top.selected.min(presets.len() - 1);
                            let _ = tx.send(TuiCommand::SetVolume(0)).await; // placeholder
                            let _ = self.client.set_eq_preset(presets[idx]).await;
                            self.overlays.close_top();
                        }
                        OverlayId::ThemePicker => {
                            self.overlays.close_top();
                        }
                        OverlayId::SoundEffects => {
                            // Toggle crossfade selected
                            self.overlays.close_top();
                        }
                        _ => {}
                    }
                }
            }
            KeyCode::Char(c) => {
                if let Some(top) = self.overlays.top_mut() {
                    match top.id {
                        OverlayId::YTSearch | OverlayId::SearchLibrary => {
                            top.query.push(c);
                        }
                        _ => {}
                    }
                }
            }
            KeyCode::Backspace => {
                if let Some(top) = self.overlays.top_mut() {
                    top.query.pop();
                }
            }
            _ => {}
        }
    }

    async fn refresh_tab(&mut self) {
        let tx = self.cmd_tx();
        if self.current_tab == Tab::Library {
            let _ = tx.send(TuiCommand::RefreshLibrary).await;
        }
    }
}
