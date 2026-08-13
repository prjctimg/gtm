// Copyright (c) 2025 - present
// Author: prjctimg <prjctimg@outlook.com>
// Application state machine: input handling, IPC dispatch, crossfade
//
// This is free software released under the GPL-3.0 license.

use std::path::Path;
use std::time::Duration;

use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use gtm_core::client::DaemonClient;
use gtm_core::ipc::DaemonRes;
use gtm_core::spotify::{SpotifyPlaylist, SpotifyStatus, SpotifyTrack};
use gtm_core::state::{DaemonState, Easing, PlaybackStatus, RepeatMode, Tab};
use gtm_core::track::{Playlist, TrackInfo, YTSearchResult};
use ratatui::layout::Alignment;
use ratatui::widgets::Paragraph;
use ratatui::Terminal;
use ratatui_image::picker::Picker;
use ratatui_image::protocol::StatefulProtocol;
use tachyonfx::EffectManager;
use tokio::sync::mpsc;

use base64::Engine;

use crate::footer;
use crate::keymap::{default_keybindings, KeyContext, KeyboardAction};
use crate::picker::{PickerId, PickerManager};
use crate::theme::{AppTheme, ThemeEntry};
use crate::ui;

fn prefs_path() -> std::path::PathBuf {
    let config = std::env::var("XDG_CONFIG_HOME")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| {
            let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".into());
            std::path::PathBuf::from(home).join(".config")
        });
    let dir = config.join("gtm");
    let _ = std::fs::create_dir_all(&dir);
    dir.join("prefs.json")
}

#[derive(serde::Serialize, serde::Deserialize, Clone)]
struct Prefs {
    #[serde(default = "default_theme_name")]
    theme_name: String,
    #[serde(default)]
    transparent_bg: bool,
    #[serde(default = "default_footer_preset_name")]
    footer_preset_name: String,
    #[serde(default)]
    progress_style: crate::progress::ProgressStyle,
}

fn default_theme_name() -> String {
    "Chadrula".into()
}

fn default_footer_preset_name() -> String {
    "Default".into()
}

impl Default for Prefs {
    fn default() -> Self {
        Self {
            theme_name: default_theme_name(),
            transparent_bg: false,
            footer_preset_name: default_footer_preset_name(),
            progress_style: crate::progress::ProgressStyle::default(),
        }
    }
}

fn load_prefs() -> Prefs {
    let path = prefs_path();
    let Ok(s) = std::fs::read_to_string(path) else {
        return Prefs::default();
    };
    if let Ok(p) = serde_json::from_str::<Prefs>(&s) {
        return p;
    }
    // Legacy v2 format: { theme_index: usize, footer_preset: usize, ... }
    #[derive(serde::Deserialize)]
    struct OldPrefs {
        #[serde(default)]
        theme_index: Option<usize>,
        #[serde(default)]
        transparent_bg: bool,
        #[serde(default)]
        footer_preset: Option<usize>,
        #[serde(default)]
        progress_style: crate::progress::ProgressStyle,
    }
    if let Ok(old) = serde_json::from_str::<OldPrefs>(&s) {
        return Prefs {
            theme_name: old
                .theme_index
                .and_then(|i| {
                    crate::theme::builtin_themes()
                        .get(i)
                        .map(|t| t.name.to_string())
                })
                .unwrap_or_else(default_theme_name),
            transparent_bg: old.transparent_bg,
            footer_preset_name: old
                .footer_preset
                .and_then(|i| footer::presets().get(i).map(|p| p.name.to_string()))
                .unwrap_or_else(default_footer_preset_name),
            progress_style: old.progress_style,
        };
    }
    // Legacy v1 format: bare integer theme_index.
    if let Ok(idx) = serde_json::from_str::<usize>(&s) {
        return Prefs {
            theme_name: crate::theme::builtin_themes()
                .get(idx)
                .map(|t| t.name.to_string())
                .unwrap_or_else(default_theme_name),
            ..Default::default()
        };
    }
    Prefs::default()
}

fn save_prefs(prefs: &Prefs) {
    if let Ok(s) = serde_json::to_string(prefs) {
        let _ = std::fs::write(prefs_path(), s);
    }
}

pub const NUM_SETTINGS_CATEGORIES: usize = 5;
pub const LIBRARY_CATEGORIES: &[&str] = &[
    "All Tracks",
    "Liked",
    "Albums",
    "Artists",
    "Playlists",
    "Spotify",
    "YouTube",
];

/// Returns true if the terminal doesn't support image protocols (Neovim, Zellij, etc.).
pub fn no_image_protocol() -> bool {
    std::env::var("NVIM").is_ok() || std::env::var("ZELLIJ").is_ok()
}

pub enum InputMode {
    Normal,
    Searching,
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

pub struct App {
    pub theme: AppTheme,
    pub themes: Vec<ThemeEntry>,
    pub client: DaemonClient,
    pub state: DaemonState,
    pub display_position: f64,
    last_display_position: f64,
    pub frame_count: u64,
    pub current_tab: Tab,
    pub input_mode: InputMode,
    pub search_query: String,
    pub scroll_offset: usize,
    pub library_category: usize,
    pub library_pane_focus: bool,
    pub settings_category: usize,
    pub settings_pane_focus: bool,
    pub settings_option: usize,
    pub tracks_cache: Vec<TrackInfo>,
    pub queue_cache: Vec<TrackInfo>,
    pub queue_cursor: usize,
    pub browse_detail: Option<String>,
    pub yt_results_cache: Vec<gtm_core::track::YTSearchResult>,
    pub playlist_cache: Vec<gtm_core::track::Playlist>,
    pub playlist_tracks_cache: Vec<TrackInfo>,
    pub spotify_status: Option<SpotifyStatus>,
    pub spotify_playlists: Vec<SpotifyPlaylist>,
    pub spotify_playlist_tracks_cache: Vec<SpotifyTrack>,
    pub spotify_token_input: String,
    pub cookie_file: Option<String>,
    pub notifications: Vec<Notification>,
    pub crossfade_duration: u8,
    pub yt_search_loading: bool,
    pub yt_search_debounce: Option<std::time::Instant>,
    pub yt_search_poll_deadline: Option<std::time::Instant>,
    pub pending_delete: Option<(i64, String)>,
    pub pickers: PickerManager,
    pub sleep_timer_remaining: Option<u64>,
    pub sleep_timer_minutes: u32,
    pub sleep_timer_input_mode: bool,
    pub sleep_timer_input_buf: String,
    pub playback_speed: f64,
    pub current_cover: Option<Vec<u8>>,
    pub last_cover_track_id: Option<i64>,
    pub cover_picker: Option<Picker>,
    pub cover_stateful: Option<StatefulProtocol>,
    pub terminal_cols: u16,
    pub terminal_rows: u16,
    pub cmd_rx: mpsc::Receiver<TuiCommand>,
    cmd_tx: mpsc::Sender<TuiCommand>,
    high_pri_cmd_rx: mpsc::UnboundedReceiver<TuiCommand>,
    high_pri_cmd_tx: mpsc::UnboundedSender<TuiCommand>,
    ipc_rx: mpsc::UnboundedReceiver<IpcResult>,
    ipc_tx: mpsc::UnboundedSender<IpcResult>,
    keybindings: crate::keymap::Keybindings,
    pub theme_index: usize,
    pub list_scroll: usize,
    pub viewport_items: usize,
    pub transparent_bg: bool,
    pub last_action_name: Option<(String, std::time::Instant)>,
    pub footer_presets: Vec<footer::FooterPreset>,
    pub footer_preset: usize,
    pub footer_title_scroll: usize,
    pub is_ready: bool,
    last_queue_cursor: u64,
    last_track_path_display: Option<String>,
    prev_tab: Tab,
    prev_track_id: Option<i64>,
    prev_status: gtm_core::state::PlaybackStatus,
    prev_volume: u8,
    prev_cover_id: Option<i64>,
    cover_art_dirty: bool,
    pub footer_cache: crate::footer::FooterCache,
    last_event_time: std::time::Instant,
    pub multiselect_mode: bool,
    pub progress_style: crate::progress::ProgressStyle,
    pub visualizer: crate::visualizer::AudioVisualizer,
    pub selected_indices: std::collections::HashSet<usize>,
    pending_motion: Option<char>,
    pub pending_playlist_track_ids: Vec<i64>,
    pub metadata_edit_track_id: Option<i64>,
    pub metadata_fields: [String; 7],
    pub metadata_field_idx: usize,
    pub pending_quit: bool,
    pub np_title_scroll: usize,
    /// Set on the first frame and on each track change; the render layer
    /// (re)starts the library/Now-Playing evolve animation once per trigger.
    pub track_anim_trigger: bool,
    /// Tachyonfx effect manager carrying the running evolve animation; kept
    /// alive across refresh frames until the effect completes.
    pub anim_fx: EffectManager<&'static str>,
    pub track_popup_visible: bool,
    pub track_popup_track_id: Option<i64>,
    pub track_popup_cover: Option<Vec<u8>>,
    pub popup_cover_stateful: Option<StatefulProtocol>,
    last_popup_cover_fetch_id: Option<i64>,
    pub current_lyrics: Option<gtm_core::track::LrcData>,
    pub lyrics_scroll: usize,
    pub lyrics_fetching: bool,
    pub show_lyrics: bool,
    /// Whether the lyrics pane holds focus (B7).  While true, MoveUp/Down,
    /// PageUp/Down, Top/Bottom scroll the lyrics and take over from the
    /// time-sync driver until focus is released.
    pub lyrics_pane_focus: bool,
    pub show_health_panel: bool,
    pub health_report: Option<gtm_core::ipc::HealthReport>,
    pub hide_help_bar: bool,
    pub lyrics_manual_scroll: bool,
}

enum IpcResult {
    RefreshDone(Box<DaemonState>, Option<Vec<u8>>, Option<i64>),
    CoverArt(Option<Vec<u8>>, Option<i64>),
    PopupCoverArt(Option<Vec<u8>>, i64),
    CoverPicker(Option<Picker>),
    Lyrics(Option<gtm_core::track::LrcData>),
    LibraryTracks(Vec<TrackInfo>),
    PlaylistTracks(Vec<TrackInfo>),
    Playlists(Vec<Playlist>),
    Queue(Vec<TrackInfo>, usize),
    YtResults(String, Vec<YTSearchResult>),
    Notification(String, NotificationKind),
    Error(String),
    HealthReport(gtm_core::ipc::HealthReport),
    SpotifyStatus(SpotifyStatus),
    SpotifyPlaylists(Vec<SpotifyPlaylist>),
    SpotifyTracks(Vec<SpotifyTrack>),
}

fn spawn_sync_and_wait(
    c: DaemonClient,
    kind: gtm_core::ipc::SyncKind,
    label: &'static str,
    ipc_tx: mpsc::UnboundedSender<IpcResult>,
) {
    tokio::spawn(async move {
        let kick = match kind {
            gtm_core::ipc::SyncKind::Covers => c.library_sync_covers().await,
            gtm_core::ipc::SyncKind::Lyrics => c.library_sync_lyrics().await,
            gtm_core::ipc::SyncKind::Metadata => c.library_sync_metadata(None).await,
        };
        if let Err(e) = kick {
            let _ = ipc_tx.send(IpcResult::Error(format!("{label} sync failed: {e}")));
            return;
        }
        let deadline = std::time::Instant::now() + Duration::from_secs(1800);
        loop {
            tokio::time::sleep(Duration::from_millis(400)).await;
            match c.library_sync_status().await {
                Ok(st) if !st.running => {
                    let msg = format!("{label} synced: {}/{} tracks", st.synced, st.total);
                    let _ = ipc_tx.send(IpcResult::Notification(msg, NotificationKind::Info));
                    if let Ok(DaemonRes::Tracks { tracks, .. }) =
                        c.library_get_tracks(None, None).await
                    {
                        let _ = ipc_tx.send(IpcResult::LibraryTracks(tracks));
                    }
                    break;
                }
                Ok(_) if std::time::Instant::now() >= deadline => {
                    let _ = ipc_tx.send(IpcResult::Error(format!("{label} sync timed out")));
                    break;
                }
                Ok(_) => {}
                Err(e) => {
                    let _ = ipc_tx.send(IpcResult::Error(e.to_string()));
                    break;
                }
            }
        }
    });
}

pub enum TuiCommand {
    Play(String),
    PlayPause,
    Pause,
    Stop,
    Next,
    Prev,
    Seek(f64),
    SetVolume(u8),
    SetMasterVolume(u8),
    ToggleShuffle,
    CycleRepeat(RepeatMode),
    ToggleMute,
    Crossfade(bool, u8),
    SetCrossfadeEasing(gtm_core::state::Easing),
    QueueAdd(String),
    QueueMove(u64, u64),
    QueueClear,
    YtSearch(String),
    YtDownload(String),
    YtResolve(String),
    SetEqPreset(gtm_core::state::EqPreset),
    Search(String),
    AddFavourite(i64),
    Refresh,
    RefreshLibrary,
    RefreshYt,
    RemoveTrack(i64),
    RemoveFromPlaylist(i64, i64),
    FetchLyrics,
    SetSleepTimer(u32),
    CancelSleepTimer,
    CheckHealth,
}

impl App {
    pub async fn new(socket_path: &Path) -> Result<Self, Box<dyn std::error::Error>> {
        let client = DaemonClient::connect(socket_path).await?;
        let state = DaemonState::new();
        let (cmd_tx, cmd_rx) = mpsc::channel(64);
        let (high_pri_cmd_tx, high_pri_cmd_rx) = mpsc::unbounded_channel();
        let (ipc_tx, ipc_rx) = mpsc::unbounded_channel();
        let keybindings = default_keybindings();
        let prefs = tokio::task::spawn_blocking(load_prefs)
            .await
            .unwrap_or_else(|_| Prefs::default());
        let initial_cursor = state.queue_cursor;

        // Build the merged theme + footer preset tables (built-ins overridden
        // by user-supplied files under ~/.config/gtm/). Resolve the persisted
        // prefs by name so adding/removing a built-in never shifts the saved
        // theme off its slot.
        let themes = crate::theme::merged_themes();
        let theme_index = themes
            .iter()
            .position(|t| t.name == prefs.theme_name)
            .unwrap_or(0);
        let theme = themes[theme_index].theme;
        let footer_presets = footer::merged_presets();
        let footer_preset = footer_presets
            .iter()
            .position(|p| p.name == prefs.footer_preset_name)
            .unwrap_or(0);

        Ok(Self {
            theme,
            themes,
            client,
            state,
            display_position: 0.0,
            last_display_position: 0.0,
            frame_count: 0,
            current_tab: Tab::Library,
            input_mode: InputMode::Normal,
            search_query: String::new(),
            scroll_offset: 0,
            library_category: 0,
            library_pane_focus: false,
            settings_category: 0,
            settings_pane_focus: false,
            settings_option: 0,
            tracks_cache: Vec::new(),
            queue_cache: Vec::new(),
            queue_cursor: 0,
            browse_detail: None,
            yt_results_cache: Vec::new(),
            playlist_cache: Vec::new(),
            playlist_tracks_cache: Vec::new(),
            spotify_status: None,
            spotify_playlists: Vec::new(),
            spotify_playlist_tracks_cache: Vec::new(),
            spotify_token_input: String::new(),
            cookie_file: None,
            notifications: Vec::new(),
            crossfade_duration: 7,
            pending_delete: None,
            yt_search_loading: false,
            yt_search_debounce: None,
            yt_search_poll_deadline: None,
            pickers: PickerManager::new(),
            sleep_timer_remaining: None,
            sleep_timer_minutes: 30,
            sleep_timer_input_mode: false,
            sleep_timer_input_buf: String::new(),
            playback_speed: 1.0,
            current_cover: None,
            last_cover_track_id: None,
            cover_picker: None,
            cover_stateful: None,
            terminal_cols: 80,
            terminal_rows: 24,
            cmd_rx,
            cmd_tx,
            high_pri_cmd_rx,
            high_pri_cmd_tx,
            ipc_rx,
            ipc_tx,
            keybindings,
            theme_index,
            list_scroll: 0,
            viewport_items: 20,
            transparent_bg: prefs.transparent_bg,
            last_action_name: None,
            footer_presets,
            footer_preset,
            footer_title_scroll: 0,
            is_ready: false,
            last_queue_cursor: initial_cursor,
            last_track_path_display: None,
            prev_tab: Tab::Library,
            prev_track_id: None,
            prev_status: gtm_core::state::PlaybackStatus::Stopped,
            prev_volume: 100,
            prev_cover_id: None,
            cover_art_dirty: false,
            footer_cache: crate::footer::FooterCache::default(),
            last_event_time: std::time::Instant::now(),
            multiselect_mode: false,
            progress_style: prefs.progress_style,
            visualizer: crate::visualizer::AudioVisualizer::new(),
            selected_indices: std::collections::HashSet::new(),
            pending_motion: None,
            pending_playlist_track_ids: Vec::new(),
            metadata_edit_track_id: None,
            metadata_fields: Default::default(),
            metadata_field_idx: 0,
            pending_quit: false,
            np_title_scroll: 0,
            track_anim_trigger: false,
            anim_fx: EffectManager::default(),
            track_popup_visible: false,
            track_popup_track_id: None,
            track_popup_cover: None,
            popup_cover_stateful: None,
            last_popup_cover_fetch_id: None,
            current_lyrics: None,
            lyrics_scroll: 0,
            lyrics_fetching: false,
            show_lyrics: false,
            lyrics_pane_focus: false,
            show_health_panel: false,
            health_report: None,
            hide_help_bar: false,
            lyrics_manual_scroll: false,
        })
    }

    pub fn cmd_tx(&self) -> mpsc::Sender<TuiCommand> {
        self.cmd_tx.clone()
    }

    pub fn send_high(&self, cmd: TuiCommand) {
        let _ = self.high_pri_cmd_tx.send(cmd);
    }

    /// Snapshot of the user-facing preferences resolved from current indices.
    /// Stored by name (not index) so adding/removing built-in themes or
    /// footer presets doesn't shift what was saved.
    fn current_prefs(&self) -> Prefs {
        Prefs {
            theme_name: self
                .themes
                .get(self.theme_index)
                .map(|t| t.name.to_string())
                .unwrap_or_else(default_theme_name),
            transparent_bg: self.transparent_bg,
            footer_preset_name: self
                .footer_presets
                .get(self.footer_preset)
                .map(|p| p.name.to_string())
                .unwrap_or_else(default_footer_preset_name),
            progress_style: self.progress_style,
        }
    }

    /// Cycle footer preset, persist, and announce the new name.
    fn cycle_footer_preset(&mut self) {
        let n = self.footer_presets.len().max(1);
        self.footer_preset = (self.footer_preset + 1) % n;
        if let Some(p) = self.footer_presets.get(self.footer_preset) {
            self.set_last_action(&format!("Footer: {}", p.name));
        }
        save_prefs(&self.current_prefs());
    }

    /// Apply a theme picker selection by index, persist by name, and refresh
    /// the live `theme` field.
    fn apply_theme_index(&mut self, idx: usize) {
        let idx = idx.min(self.themes.len().saturating_sub(1));
        self.theme = self.themes[idx].theme;
        self.theme_index = idx;
        save_prefs(&self.current_prefs());
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
                }) = c.queue_list().await
                {
                    let _ = ipc_tx.send(IpcResult::Queue(tracks, cursor as usize));
                }
            });
        }
        {
            let c = self.client.clone();
            let ipc_tx = self.ipc_tx.clone();
            tokio::spawn(async move {
                if let Ok(DaemonRes::Tracks { tracks, .. }) = c.library_get_tracks(None, None).await
                {
                    if !tracks.is_empty() {
                        let _ = ipc_tx.send(IpcResult::LibraryTracks(tracks));
                    }
                }
            });
        }
        {
            let c = self.client.clone();
            let ipc_tx = self.ipc_tx.clone();
            tokio::spawn(async move {
                if let Ok(status) = c.spotify_status().await {
                    let _ = ipc_tx.send(IpcResult::SpotifyStatus(status));
                }
                if let Ok(playlists) = c.spotify_playlists().await {
                    let _ = ipc_tx.send(IpcResult::SpotifyPlaylists(playlists));
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

        loop {
            let mut events_received = false;
            let mut had_track_change = false;
            let mut had_sleep_expired = false;
            for ev in self.client.drain().await {
                if matches!(ev, gtm_core::ipc::DaemonEvent::PlaybackStarted { .. }) {
                    had_track_change = true;
                }
                if matches!(ev, gtm_core::ipc::DaemonEvent::SleepTimerExpired) {
                    had_sleep_expired = true;
                }
                self.state.apply_event(&ev);
                events_received = true;
            }
            if events_received {
                self.last_event_time = std::time::Instant::now();
            }
            if had_sleep_expired {
                self.sleep_timer_remaining = None;
                self.notify(
                    "Sleep timer expired — playback stopped",
                    NotificationKind::Info,
                );
            }
            // Sync sleep_timer_remaining from daemon state
            if let Some(secs) = self.state.sleep_timer {
                self.sleep_timer_remaining = Some(secs as u64);
            } else if self.sleep_timer_remaining.is_some() && self.state.sleep_timer.is_none() {
                self.sleep_timer_remaining = None;
            }
            // Re-seed clock from state after track change events so the
            // local position estimate stays in sync with the daemon.
            if had_track_change {
                self.client.seed_clock_from_state(&self.state).await;
            }

            // Force a state refresh if no events received for 8s to prevent
            // stale state from broadcast lag. Works in all playback states,
            // not just Playing, to catch lag when paused/stopped too.
            if self.last_event_time.elapsed() > Duration::from_secs(8) && self.client.is_connected()
            {
                let c = self.client.clone();
                let ipc_tx2 = self.ipc_tx.clone();
                tokio::spawn(async move {
                    if let Ok(state) = c.get_status().await {
                        let _ = ipc_tx2.send(IpcResult::RefreshDone(Box::new(state), None, None));
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
            let track_changed = current_path.as_deref() != self.last_track_path_display.as_deref();
            if track_changed {
                self.last_track_path_display = current_path;
                let raw = self.client.estimated_position().await;
                self.display_position = raw;
                self.last_display_position = raw;
                self.track_anim_trigger = true;
            }

            // Clear stale cover immediately so we don't show old art on the
            // new track, then trigger a cover fetch + lyrics auto-fetch.
            if track_changed {
                self.current_cover = None;
                self.cover_stateful = None;
                // Skip cover art in Neovim terminal (no image protocol passthrough)
                if !no_image_protocol() {
                    if let Some(tid) = current_tid {
                        if Some(tid) != self.last_cover_track_id {
                            self.last_cover_track_id = Some(tid);
                            let client2 = self.client.clone();
                            let ipc_tx2 = self.ipc_tx.clone();
                            tokio::spawn(async move {
                                if let Ok(Some(b64)) = client2.get_cover_art(tid).await {
                                    if let Ok(bytes) =
                                        base64::engine::general_purpose::STANDARD.decode(&b64)
                                    {
                                        let _ = ipc_tx2
                                            .send(IpcResult::CoverArt(Some(bytes), Some(tid)));
                                    }
                                }
                            });
                        }
                    }
                }
                // Auto-fetch lyrics if lyrics pane is visible
                if self.show_lyrics {
                    let client3 = self.client.clone();
                    let ipc_tx3 = self.ipc_tx.clone();
                    let tpath = self.state.current_track.as_ref().map(|t| t.path.clone());
                    self.current_lyrics = None;
                    self.lyrics_fetching = true;
                    self.lyrics_scroll = 0;
                    tokio::spawn(async move {
                        let result = client3
                            .get_lyrics(current_tid.unwrap_or(0), tpath.as_deref())
                            .await;
                        let _ = ipc_tx3.send(IpcResult::Lyrics(result.unwrap_or(None)));
                    });
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
                        if state.version >= self.state.version || restarted {
                            self.state = *state;
                            self.client.seed_clock_from_state(&self.state).await;
                            // Cover art is fetched on track-change events only.
                            // Do not clear last_cover_track_id when a periodic
                            // RefreshDone carries no cover, or the track-change
                            // guard would re-download art (and re-fetch lyrics)
                            // every second.
                            if !no_image_protocol() {
                                if let Some(c) = cover {
                                    self.current_cover = Some(c);
                                    self.last_cover_track_id = cover_tid;
                                    self.sync_cover_stateful();
                                }
                            }
                        }
                    }
                    IpcResult::CoverArt(cover, cover_tid) => {
                        if !no_image_protocol() {
                            self.current_cover = cover;
                            self.last_cover_track_id = cover_tid;
                            self.sync_cover_stateful();
                            self.cover_art_dirty = true;
                        }
                    }
                    IpcResult::LibraryTracks(tracks) => self.tracks_cache = tracks,
                    IpcResult::Playlists(playlists) => self.playlist_cache = playlists,
                    IpcResult::PlaylistTracks(tracks) => self.playlist_tracks_cache = tracks,
                    IpcResult::Queue(tracks, cursor) => {
                        self.queue_cache = tracks.clone();
                        self.queue_cursor = cursor;
                        if self.queue_cache.is_empty() && self.state.current_track.is_none() {
                            self.browse_detail = None;
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
                    IpcResult::Notification(msg, kind) => {
                        self.notifications.push(Notification {
                            message: msg,
                            kind,
                            expires_at: std::time::Instant::now() + Duration::from_secs(5),
                        });
                    }
                    IpcResult::Error(e) => {
                        self.notify(e, NotificationKind::Error);
                    }
                    IpcResult::PopupCoverArt(cover, track_id) => {
                        if !no_image_protocol() && self.track_popup_track_id == Some(track_id) {
                            self.track_popup_cover = cover;
                            self.sync_popup_cover_stateful();
                        }
                    }
                    IpcResult::CoverPicker(picker) => {
                        self.cover_picker = picker;
                    }
                    IpcResult::Lyrics(lyrics) => {
                        self.current_lyrics = lyrics;
                        self.lyrics_fetching = false;
                        // Snap to the lyric line matching the current playback
                        // position so opening lyrics mid-track doesn't start
                        // with the first line highlighted.
                        self.lyrics_scroll = self.current_lyric_index();
                    }
                    IpcResult::HealthReport(report) => {
                        self.health_report = Some(report);
                        self.show_health_panel = true;
                    }
                    IpcResult::SpotifyStatus(s) => self.spotify_status = Some(s),
                    IpcResult::SpotifyPlaylists(p) => self.spotify_playlists = p,
                    IpcResult::SpotifyTracks(t) => self.spotify_playlist_tracks_cache = t,
                }
            }

            while let Ok(cmd) = self.high_pri_cmd_rx.try_recv() {
                self.handle_command(cmd);
            }
            while let Ok(cmd) = self.cmd_rx.try_recv() {
                self.handle_command(cmd);
            }

            // Expire stale notifications
            let now = std::time::Instant::now();
            self.notifications.retain(|n| n.expires_at > now);

            // YT search debounce: auto-search 500ms after last keystroke
            if let Some(deadline) = self.yt_search_debounce {
                if now >= deadline {
                    self.yt_search_debounce = None;
                    if let Some(top) = self.pickers.top() {
                        if top.id == PickerId::YTSearch && !top.query.is_empty() {
                            let q = top.query.clone();
                            let tx = self.cmd_tx();
                            let _ = tx.send(TuiCommand::YtSearch(q)).await;
                        }
                    }
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
                if self.yt_search_poll_deadline.is_none() {
                    self.yt_search_poll_deadline = Some(now + Duration::from_millis(500));
                }
                if now >= self.yt_search_poll_deadline.unwrap_or(now) {
                    self.yt_search_poll_deadline = Some(now + Duration::from_millis(700));
                    let tx = self.cmd_tx();
                    let _ = tx.send(TuiCommand::RefreshYt).await;
                }
            } else {
                self.yt_search_poll_deadline = None;
            }

            // Detect state changes
            if self.current_tab != self.prev_tab {
                self.prev_tab = self.current_tab;
            }
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
            if self.last_cover_track_id != self.prev_cover_id {
                self.prev_cover_id = self.last_cover_track_id;
            }

            let raw_pos = self.client.estimated_position().await;
            // Monotonic guard: prevent large backward jumps from clock skew.
            // Allow at most 0.5s of regression to avoid visible stutter.
            let raw_pos = raw_pos.max(self.display_position - 0.5);
            // EMA smoothing
            self.display_position = self.display_position * 0.85 + raw_pos * 0.15;

            // Auto-scroll lyrics to current playback position.  Not gated on
            // `status == Playing` so a mirrored-status desync (e.g. after a
            // daemon restart) can't freeze the highlight at the first line.
            if !self.lyrics_manual_scroll && self.show_lyrics && self.current_lyrics.is_some() {
                self.lyrics_scroll = self.current_lyric_index();
            }

            // Dirty-render: skip redraw if position hasn't changed meaningfully
            // to reduce CPU usage.  Always render every 10th frame as a safety net.
            let frame_count = self.frame_count.wrapping_add(1);
            self.frame_count = frame_count;
            let pos_changed = (self.display_position - self.last_display_position).abs() >= 0.1;
            // Advance title scroll animations (every 3rd frame)
            if frame_count.is_multiple_of(3) {
                self.footer_title_scroll = self.footer_title_scroll.wrapping_add(1);
                self.np_title_scroll = self.np_title_scroll.wrapping_add(1);
            }

            let force_render = pos_changed
                || !self.notifications.is_empty()
                || frame_count.is_multiple_of(10)
                || self.cover_art_dirty
                || self.track_anim_trigger
                || self.anim_fx.is_running();
            self.cover_art_dirty = false;
            self.last_display_position = self.display_position;

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

            if event::poll(Duration::from_millis(16)).unwrap_or(false) {
                if let Ok(Event::Key(key)) = event::read() {
                    if key.kind == KeyEventKind::Press
                        && (!self.handle_key(key).await || self.pending_quit)
                    {
                        break;
                    }
                }
            }
        }
        Ok(())
    }

    /// Index of the time-synced lyric line for the current playback position.
    /// Untimed lines (timestamp < 0) are skipped for matching but keep their
    /// index so the highlight tracks timed lines correctly.
    pub fn current_lyric_index(&self) -> usize {
        let Some(ref lyrics) = self.current_lyrics else {
            return 0;
        };
        if lyrics.lines.is_empty() {
            return 0;
        }
        let pos = self.display_position;
        let mut current_idx = 0;
        for (i, line) in lyrics.lines.iter().enumerate() {
            if line.timestamp < 0.0 {
                continue;
            }
            if line.timestamp <= pos {
                current_idx = i;
            } else {
                break;
            }
        }
        current_idx
    }

    pub fn notify(&mut self, message: impl Into<String>, kind: NotificationKind) {
        let expires_at = std::time::Instant::now() + std::time::Duration::from_secs(4);
        self.notifications.push(Notification {
            message: message.into(),
            kind,
            expires_at,
        });
    }

    /// Update the track popup to show the currently selected track in the library list.
    pub fn update_track_popup(&mut self) {
        let maybe_track = {
            let filtered = self.filtered_tracks();
            if self.scroll_offset < filtered.len() {
                let t = filtered[self.scroll_offset];
                Some((t.id, t.path.clone()))
            } else {
                None
            }
        };

        if let Some((tid, path)) = maybe_track {
            self.track_popup_track_id = Some(tid);
            self.track_popup_visible = true;

            let current_is_selected = self
                .state
                .current_track
                .as_ref()
                .is_some_and(|t| t.path == path);
            if current_is_selected {
                self.track_popup_cover = self.current_cover.clone();
                self.sync_popup_cover_stateful();
                self.last_popup_cover_fetch_id = None;
            } else if self.last_popup_cover_fetch_id != Some(tid) && !no_image_protocol() {
                self.last_popup_cover_fetch_id = Some(tid);
                self.track_popup_cover = None;
                let client2 = self.client.clone();
                let ipc_tx2 = self.ipc_tx.clone();
                tokio::spawn(async move {
                    if let Ok(Some(b64)) = client2.get_cover_art(tid).await {
                        if let Ok(bytes) = base64::engine::general_purpose::STANDARD.decode(&b64) {
                            let _ = ipc_tx2.send(IpcResult::PopupCoverArt(Some(bytes), tid));
                        }
                    }
                });
            }
        } else {
            self.track_popup_visible = false;
            self.track_popup_track_id = None;
            self.track_popup_cover = None;
            self.last_popup_cover_fetch_id = None;
        }
    }

    /// Dismiss the track popup.
    pub fn dismiss_track_popup(&mut self) {
        self.track_popup_visible = false;
        self.track_popup_track_id = None;
        self.track_popup_cover = None;
        self.popup_cover_stateful = None;
        self.last_popup_cover_fetch_id = None;
    }

    /// Filtered tracks for the current library view, respecting search query, browse_detail, and category.
    pub fn filtered_tracks(&self) -> Vec<&TrackInfo> {
        if self.library_category == 4 && self.browse_detail.is_some() {
            return self.playlist_tracks_cache.iter().collect();
        }
        let mut tracks: Vec<&TrackInfo> = self.tracks_cache.iter().collect();
        if !self.search_query.is_empty() {
            let q = self.search_query.to_lowercase();
            tracks.retain(|t| {
                t.title.to_lowercase().contains(&q)
                    || t.artist.to_lowercase().contains(&q)
                    || t.album.to_lowercase().contains(&q)
            });
        }
        if let Some(ref detail) = self.browse_detail {
            let detail_lower = detail.to_lowercase();
            tracks.retain(|t| {
                t.album.to_lowercase().contains(&detail_lower)
                    || t.artist.to_lowercase().contains(&detail_lower)
                    || t.title.to_lowercase().contains(&detail_lower)
            });
        }
        if self.library_category == 1 {
            tracks.retain(|t| t.favourite);
        } else if self.library_category == 5 {
            // Spotify: category renders the synced playlist browser, not a flat
            // TrackInfo list — resolve/play goes through the daemon.
            tracks.clear();
        } else if self.library_category == 6 {
            // YouTube: tracks from the youtube audio subdirectory
            tracks.retain(|t| {
                t.path.contains("/audio/youtube") || t.path.contains("\\audio\\youtube")
            });
        }
        tracks
    }

    /// Play the track highlighted in the current library view, replacing the
    /// queue with the filtered list and starting at that row.
    fn play_filtered_highlighted(&self) {
        let filtered = self.filtered_tracks();
        if self.scroll_offset >= filtered.len() {
            return;
        }
        let idx = self.scroll_offset;
        let paths: Vec<String> = filtered.iter().map(|t| t.path.clone()).collect();
        let path = paths[idx].clone();
        let c = self.client.clone();
        tokio::spawn(async move {
            let _ = c.queue_set(paths, idx as u64).await;
            let _ = c.play(&path, 0.0).await;
        });
    }

    /// Unique album names with track counts, sorted by album.
    pub fn unique_albums(&self) -> Vec<(String, usize)> {
        let mut albums: std::collections::BTreeMap<String, usize> =
            std::collections::BTreeMap::new();
        for t in &self.tracks_cache {
            let key = if t.album.is_empty() {
                "Unknown Album".into()
            } else {
                t.album.clone()
            };
            *albums.entry(key).or_insert(0) += 1;
        }
        albums.into_iter().collect()
    }

    /// Length of the list currently visible in the library right pane,
    /// depending on the active category and drill-down state.
    pub fn library_list_len(&self) -> usize {
        if self.browse_detail.is_some() {
            if self.library_category == 5 {
                return self.spotify_playlist_tracks_cache.len();
            }
            return self.filtered_tracks().len();
        }
        match self.library_category {
            2 => self.unique_albums().len(),
            3 => self.unique_artists().len(),
            4 => self.playlist_cache.len(),
            5 => self.spotify_playlists.len(),
            _ => self.filtered_tracks().len(),
        }
    }

    /// Unique artist names with track counts, sorted by artist.
    pub fn unique_artists(&self) -> Vec<(String, usize)> {
        let mut artists: std::collections::BTreeMap<String, usize> =
            std::collections::BTreeMap::new();
        for t in &self.tracks_cache {
            let key = if t.artist.is_empty() {
                "Unknown Artist".into()
            } else {
                t.artist.clone()
            };
            *artists.entry(key).or_insert(0) += 1;
        }
        artists.into_iter().collect()
    }

    fn sync_cover_stateful(&mut self) {
        match (&self.current_cover, &self.cover_picker) {
            (Some(bytes), Some(picker)) => {
                if let Ok(img) = image::load_from_memory(bytes) {
                    self.cover_stateful = Some(picker.new_resize_protocol(img));
                } else {
                    self.cover_stateful = None;
                }
            }
            _ => self.cover_stateful = None,
        }
    }

    fn sync_popup_cover_stateful(&mut self) {
        match (&self.track_popup_cover, &self.cover_picker) {
            (Some(bytes), Some(picker)) => {
                if let Ok(img) = image::load_from_memory(bytes) {
                    self.popup_cover_stateful = Some(picker.new_resize_protocol(img));
                } else {
                    self.popup_cover_stateful = None;
                }
            }
            _ => self.popup_cover_stateful = None,
        }
    }

    fn settings_options_for_category(&self) -> usize {
        match self.settings_category {
            0 => 3, // Audio: Master Volume, Volume, Mute
            1 => 8, // YouTube
            2 => 4, // Playback: Repeat, Shuffle, Crossfade, Easing
            3 => 6, // System: Theme, Transparent BG, Sync Covers, Sync Lyrics, Sync Metadata, Footer Preset
            4 => 6, // Spotify: Status, Account, Playlists, Link, Sync, Unlink
            _ => 0,
        }
    }

    async fn fetch_queue(&mut self) {
        if let Ok(DaemonRes::QueueState {
            queue: tracks,
            cursor,
            ..
        }) = self.client.queue_list().await
        {
            self.queue_cache = tracks;
            self.queue_cursor = cursor as usize;
        }
    }

    fn handle_command(&mut self, cmd: TuiCommand) {
        // All IPC calls are spawned as background tasks to avoid blocking the UI loop.
        let client = self.client.clone();
        let ipc_tx = self.ipc_tx.clone();
        let err_tx = ipc_tx.clone();
        let error_handler = move |e: gtm_core::CoreError| {
            let _ = err_tx.send(IpcResult::Error(e.to_string()));
        };
        let err_tx2 = ipc_tx.clone();
        let error_handler2 = move |e: gtm_core::CoreError| {
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
                tokio::spawn(async move {
                    if let Err(e) = client.next().await {
                        error_handler(e);
                    }
                });
            }
            TuiCommand::Prev => {
                tokio::spawn(async move {
                    if let Err(e) = client.prev().await {
                        error_handler(e);
                    }
                });
            }
            TuiCommand::Seek(pos) => {
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
            TuiCommand::SetMasterVolume(v) => {
                tokio::spawn(async move {
                    if let Err(e) = client.set_master_volume(v).await {
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
            TuiCommand::Crossfade(en, dur) => {
                tokio::spawn(async move {
                    if let Err(e) = client.crossfade(en, dur, None).await {
                        error_handler(e);
                    }
                });
            }
            TuiCommand::SetCrossfadeEasing(_easing) => {
                // TODO: pass easing through crossfade command
            }
            TuiCommand::QueueAdd(p) => {
                tokio::spawn(async move {
                    if let Err(e) = client.queue_add(&p, None).await {
                        error_handler2(e);
                    }
                });
            }
            TuiCommand::QueueMove(from, to) => {
                tokio::spawn(async move {
                    if let Err(e) = client.queue_move(from, to).await {
                        error_handler(e);
                    }
                });
            }
            TuiCommand::QueueClear => {
                tokio::spawn(async move {
                    if let Err(e) = client.queue_clear().await {
                        error_handler(e);
                    }
                });
            }
            TuiCommand::YtSearch(q) => {
                self.yt_search_loading = true;
                self.yt_results_cache.clear();
                let c = client.clone();
                tokio::spawn(async move {
                    if let Err(e) = c.yt_search(&q, None).await {
                        error_handler2(e);
                    }
                });
            }
            TuiCommand::YtDownload(url) => {
                self.notify("Download started…", NotificationKind::Info);
                let ipc = ipc_tx.clone();
                let client2 = self.client.clone();
                tokio::spawn(async move {
                    let audio_dir = std::env::var("XDG_DATA_HOME")
                        .map(|d| std::path::PathBuf::from(d).join("gtm").join("audio"))
                        .unwrap_or_else(|_| {
                            let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".into());
                            std::path::PathBuf::from(home).join(".local/share/gtm/audio")
                        });
                    std::fs::create_dir_all(&audio_dir).ok();
                    let template = audio_dir.join("%(title)s.%(ext)s");
                    let output = tokio::process::Command::new("yt-dlp")
                        .arg("--extract-audio")
                        .arg("--audio-format")
                        .arg("mp3")
                        .arg("--restrict-filenames")
                        .arg("-o")
                        .arg(template.to_string_lossy().as_ref())
                        .arg(&url)
                        .output()
                        .await;
                    let msg = match output {
                        Ok(o) if o.status.success() => {
                            let _ = client2
                                .library_scan(audio_dir.to_string_lossy().as_ref())
                                .await;
                            // Also refresh the track cache
                            if let Ok(DaemonRes::Tracks { tracks, .. }) =
                                client2.library_get_tracks(None, None).await
                            {
                                let _ = ipc.send(IpcResult::LibraryTracks(tracks));
                            }
                            // Try to fetch lyrics for the newly downloaded track
                            let stdout_text = String::from_utf8_lossy(&o.stdout).to_string();
                            let filename = stdout_text.lines().next().unwrap_or(&url).to_string();
                            // Find the track by filename substring match in library
                            if let Ok(DaemonRes::Tracks { tracks, .. }) =
                                client2.library_get_tracks(None, None).await
                            {
                                if let Some(track) = tracks
                                    .iter()
                                    .find(|t| {
                                        filename.contains(&t.title)
                                            || t.path.contains(filename.trim_end_matches(".mp3"))
                                    })
                                    .or_else(|| tracks.last())
                                {
                                    if let Ok(Some(lyrics_data)) =
                                        client2.get_lyrics(track.id, Some(&track.path)).await
                                    {
                                        if !lyrics_data.lines.is_empty() {
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
                                    }
                                }
                            }
                            format!("Downloaded: {}", filename)
                        }
                        Ok(o) => format!(
                            "Download failed: {}",
                            String::from_utf8_lossy(&o.stderr)
                                .lines()
                                .last()
                                .unwrap_or("unknown error")
                        ),
                        Err(e) => format!("Download error: {e}"),
                    };
                    let kind = if msg.starts_with("Downloaded") {
                        crate::app::NotificationKind::Success
                    } else {
                        crate::app::NotificationKind::Error
                    };
                    let _ = ipc.send(IpcResult::Notification(msg, kind));
                });
            }
            TuiCommand::YtResolve(u) => {
                tokio::spawn(async move {
                    if let Err(e) = client.yt_resolve_stream(&u).await {
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
                    if let Err(e) = client.add_favourite(id).await {
                        error_handler(e);
                    }
                });
            }
            TuiCommand::Refresh => {
                let client2 = self.client.clone();
                let ipc_tx2 = self.ipc_tx.clone();
                tokio::spawn(async move {
                    if let Ok(state) = client2.get_status().await {
                        // Cover art is fetched on track-change events, not
                        // here, to avoid an extra IPC call every second.
                        let _ = ipc_tx2.send(IpcResult::RefreshDone(Box::new(state), None, None));
                    }
                    // Also refresh queue to recover from initial background
                    // spawn failures that leave queue_cache empty.
                    if let Ok(DaemonRes::QueueState {
                        queue: tracks,
                        cursor,
                        ..
                    }) = client2.queue_list().await
                    {
                        let _ = ipc_tx2.send(IpcResult::Queue(tracks, cursor as usize));
                    }
                });
            }
            TuiCommand::RefreshLibrary => {
                tokio::spawn(async move {
                    if let Ok(DaemonRes::Tracks { tracks, .. }) =
                        client.library_get_tracks(None, None).await
                    {
                        let _ = ipc_tx.send(IpcResult::LibraryTracks(tracks));
                    }
                });
            }
            TuiCommand::RefreshYt => {
                tokio::spawn(async move {
                    if let Ok(DaemonRes::YtSearchResults { query, results }) =
                        client.yt_search_poll().await
                    {
                        let _ = ipc_tx.send(IpcResult::YtResults(query, results));
                    }
                });
            }
            TuiCommand::RemoveTrack(track_id) => {
                tokio::spawn(async move {
                    if let Err(e) = client.library_remove_track(track_id).await {
                        error_handler(e);
                    } else {
                        let _ = ipc_tx.send(IpcResult::Notification(
                            "Track deleted".to_string(),
                            NotificationKind::Success,
                        ));
                        if let Ok(DaemonRes::Tracks { tracks, .. }) =
                            client.library_get_tracks(None, None).await
                        {
                            let _ = ipc_tx.send(IpcResult::LibraryTracks(tracks));
                        }
                    }
                });
            }
            TuiCommand::RemoveFromPlaylist(playlist_id, track_id) => {
                tokio::spawn(async move {
                    if let Err(e) = client
                        .library_remove_from_playlist(playlist_id, track_id)
                        .await
                    {
                        error_handler(e);
                    } else {
                        let _ = ipc_tx.send(IpcResult::Notification(
                            "Removed from playlist".to_string(),
                            NotificationKind::Success,
                        ));
                        if let Ok(DaemonRes::Playlists { playlists, .. }) =
                            client.library_get_playlists().await
                        {
                            let _ = ipc_tx.send(IpcResult::Playlists(playlists));
                        }
                    }
                });
            }
            TuiCommand::FetchLyrics => {
                let track_path = self.state.current_track.as_ref().map(|t| t.path.clone());
                let track_id = self.state.current_track.as_ref().map(|t| t.id).unwrap_or(0);
                let client2 = self.client.clone();
                let ipc_tx2 = self.ipc_tx.clone();
                tokio::spawn(async move {
                    let result = tokio::time::timeout(
                        Duration::from_secs(5),
                        client2.get_lyrics(track_id, track_path.as_deref()),
                    )
                    .await;
                    let _ = ipc_tx2.send(IpcResult::Lyrics(match result {
                        Ok(r) => r.unwrap_or(None),
                        Err(_) => None,
                    }));
                });
            }
            TuiCommand::SetSleepTimer(minutes) => {
                let client = self.client.clone();
                let ipc_tx = self.ipc_tx.clone();
                tokio::spawn(async move {
                    if let Err(e) = client.set_sleep_timer(minutes).await {
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
        };
    }

    fn set_last_action(&mut self, name: &str) {
        self.last_action_name = Some((
            name.to_string(),
            std::time::Instant::now() + std::time::Duration::from_secs(3),
        ));
    }

    fn clamp_picker_selection(&mut self) {
        if let Some(top) = self.pickers.top_mut() {
            let max = match top.id {
                PickerId::Queue => self.queue_cache.len().saturating_sub(1),
                PickerId::YTSearch => self.yt_results_cache.len().saturating_sub(1),
                PickerId::SearchLibrary => {
                    let q = top.query.to_lowercase();
                    if q.is_empty() {
                        self.tracks_cache.len()
                    } else {
                        self.tracks_cache
                            .iter()
                            .filter(|t| {
                                t.title.to_lowercase().contains(&q)
                                    || t.artist.to_lowercase().contains(&q)
                            })
                            .count()
                    }
                    .saturating_sub(1)
                }
                PickerId::Equalizer => {
                    let presets = [
                        gtm_core::state::EqPreset::Flat,
                        gtm_core::state::EqPreset::Normal,
                        gtm_core::state::EqPreset::Pop,
                        gtm_core::state::EqPreset::Rock,
                        gtm_core::state::EqPreset::Jazz,
                        gtm_core::state::EqPreset::Classical,
                        gtm_core::state::EqPreset::Bass,
                        gtm_core::state::EqPreset::Vocal,
                        gtm_core::state::EqPreset::Electronic,
                        gtm_core::state::EqPreset::HipHop,
                        gtm_core::state::EqPreset::Latin,
                        gtm_core::state::EqPreset::Acoustic,
                        gtm_core::state::EqPreset::Podcast,
                        gtm_core::state::EqPreset::Dance,
                        gtm_core::state::EqPreset::Headphones,
                        gtm_core::state::EqPreset::Speaker,
                    ];
                    presets.len().saturating_sub(1)
                }
                PickerId::SleepTimer => 4,
                PickerId::ThemePicker => {
                    let q = top.query.to_lowercase();
                    if q.is_empty() {
                        self.themes.len()
                    } else {
                        self.themes
                            .iter()
                            .filter(|entry| {
                                let lower = entry.name.to_lowercase();
                                let mut qi = 0usize;
                                for ch in lower.chars() {
                                    if qi < q.len() && ch == q.as_bytes()[qi] as char {
                                        qi += 1;
                                    }
                                }
                                qi == q.len()
                            })
                            .count()
                    }
                    .saturating_sub(1)
                }
                PickerId::CommandPalette => {
                    let commands = crate::ui::COMMAND_PALETTE_COMMANDS;
                    let q = top.query.to_lowercase();
                    if q.is_empty() {
                        commands.len()
                    } else {
                        commands
                            .iter()
                            .filter(|c| {
                                let lower = c.0.to_lowercase();
                                let mut qi = 0usize;
                                for ch in lower.chars() {
                                    if qi < q.len() && ch == q.as_bytes()[qi] as char {
                                        qi += 1;
                                    }
                                }
                                qi == q.len()
                            })
                            .count()
                    }
                    .saturating_sub(1)
                }
                _ => usize::MAX,
            };
            top.selected = top.selected.min(max);
        }
    }

    fn help_picker_total(&self) -> usize {
        let help_lines = [
            ("topic", "Playback"),
            ("key", "   Space        Play / Pause"),
            ("key", "   n / Ctrl+N   Next track"),
            ("key", "   p / Ctrl+P   Previous track"),
            ("key", "   s            Stop"),
            ("key", "   . / ,        Seek forward / back"),
            ("", ""),
            ("topic", "Volume"),
            ("key", "   + / =        Volume up"),
            ("key", "   -            Volume down"),
            ("key", "   m            Toggle mute"),
            ("", ""),
            ("topic", "Queue & Library"),
            ("key", "   Enter        Play selected / drill-down"),
            ("key", "   d / Del      Remove item"),
            ("key", "   F            Toggle favourite"),
            ("key", "   D            Clear queue"),
            ("key", "   /            Filter mode"),
            ("", ""),
            ("topic", "Navigation"),
            ("key", "   Tab          Toggle left/right pane focus"),
            ("key", "   j/k / arrows Move up/down"),
            ("key", "   h/l          Focus left/right pane"),
            ("key", "   ?            Toggle this help"),
            ("", ""),
            ("topic", "Overlays (Alt+key)"),
            ("key", "   Alt+Q        Queue"),
            ("key", "   Alt+Y        YouTube Search"),
            ("key", "   Alt+F        Search Library"),
            ("key", "   Alt+A        About"),
            ("key", "   Alt+C        Theme Picker"),
            ("key", "   Alt+E        Equalizer"),
            ("key", "   Alt+P        Command Palette"),
            ("key", "   Alt+Z        Sleep Timer"),
            ("key", "   Alt+X        Sound Effects"),
            ("key", "   Alt+S        Spotify Search"),
            ("", ""),
            ("topic", "Other"),
            ("key", "   q            Quit"),
            ("key", "   Q / Ctrl+Q   Quit & stop daemon"),
            ("key", "   S            Toggle shuffle"),
            ("key", "   r / R        Cycle repeat"),
            ("key", "   :            Command palette"),
            ("key", "   Alt+F        Cycle footer preset"),
            ("", ""),
            ("topic", "Help"),
            ("key", "   ?            Toggle this help"),
            ("key", "   gg / G       Jump to top / bottom"),
            ("key", "   0 / $        Jump to first / last line"),
            ("key", "   /            Search"),
            ("key", "   n / N        Next / previous match"),
            ("key", "   Esc / q      Close"),
        ];
        if let Some(top) = self.pickers.top() {
            if top.id == PickerId::Help {
                let q = top.query.to_lowercase();
                if q.is_empty() {
                    help_lines.len()
                } else {
                    help_lines
                        .iter()
                        .filter(|(_, l)| l.to_lowercase().contains(&q))
                        .count()
                }
            } else {
                0
            }
        } else {
            0
        }
    }

    async fn handle_key(&mut self, key: event::KeyEvent) -> bool {
        // Reset pending_motion if the key is not 'g'
        if key.code != KeyCode::Char('g') {
            self.pending_motion = None;
        }
        // If an picker is open, Esc closes it; keys pass through to picker
        if self.pickers.is_open() {
            return match key.code {
                KeyCode::Esc => {
                    self.pickers.close_top();
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
                    let q = self.search_query.clone();
                    self.input_mode = InputMode::Normal;
                    match self.current_tab {
                        Tab::Library => {
                            // Commit the filter without clearing it and play
                            // the highlighted row in the filtered list.
                            self.play_filtered_highlighted();
                        }
                        _ => {
                            let tx = self.cmd_tx();
                            let _ = tx.send(TuiCommand::Search(q)).await;
                            self.search_query.clear();
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
                // If a delete confirmation is pending, intercept Enter/Esc
                if self.pending_delete.is_some() {
                    match key.code {
                        KeyCode::Enter => {
                            if let Some((track_id, track_name)) = self.pending_delete.take() {
                                let tx = self.cmd_tx();
                                let _ = tx.send(TuiCommand::RemoveTrack(track_id)).await;
                                self.notify(
                                    format!("Deleted: {track_name}"),
                                    NotificationKind::Success,
                                );
                            }
                        }
                        KeyCode::Esc => {
                            self.pending_delete = None;
                        }
                        _ => {}
                    }
                    return true;
                }
                // Health panel: Esc closes it
                if self.show_health_panel && key.code == KeyCode::Esc {
                    self.show_health_panel = false;
                    return true;
                }
                // Handle gg (vim-style double-press) for jump to start
                if key.code == KeyCode::Char('g')
                    && self.current_tab == Tab::Library
                    && !self.library_pane_focus
                {
                    if self.pending_motion == Some('g') {
                        // Second 'g' — execute jump to start
                        self.pending_motion = None;
                        self.scroll_offset = 0;

                        return true;
                    } else {
                        // First 'g' — wait for second press
                        self.pending_motion = Some('g');
                        return true;
                    }
                }
                // In multiselect mode, Tab toggles selection and advances
                if key.code == KeyCode::Tab
                    && self.multiselect_mode
                    && self.current_tab == Tab::Library
                    && !self.library_pane_focus
                {
                    if self.selected_indices.contains(&self.scroll_offset) {
                        self.selected_indices.remove(&self.scroll_offset);
                    } else {
                        self.selected_indices.insert(self.scroll_offset);
                    }
                    let max = self.filtered_tracks().len().saturating_sub(1);
                    self.scroll_offset = (self.scroll_offset + 1).min(max);
                    let count = self.selected_indices.len();
                    self.notify(format!("{count} selected"), NotificationKind::Info);
                    return true;
                }
                match self.keybindings.dispatch(key, KeyContext::Normal) {
                    Some(KeyboardAction::Quit) => {
                        if self.browse_detail.is_some() {
                            self.browse_detail = None;
                            self.scroll_offset = 0;
                            if self.library_category == 5 {
                                self.spotify_playlist_tracks_cache.clear();
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
                    Some(KeyboardAction::NextTab) => {
                        match self.current_tab {
                            Tab::Library => {
                                self.library_pane_focus = !self.library_pane_focus;
                            }
                            Tab::Settings => {
                                self.settings_pane_focus = !self.settings_pane_focus;
                            }
                        }
                        self.dismiss_track_popup();
                    }
                    Some(KeyboardAction::PrevTab) => {
                        match self.current_tab {
                            Tab::Library => {
                                self.library_pane_focus = !self.library_pane_focus;
                            }
                            Tab::Settings => {
                                self.settings_pane_focus = !self.settings_pane_focus;
                            }
                        }
                        self.dismiss_track_popup();
                    }
                    Some(KeyboardAction::OpenOverlay(id)) => {
                        self.pickers.open(id);
                        self.dismiss_track_popup();
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
                                if !self.queue_cache.is_empty() {
                                    let idx = self.queue_cursor.min(self.queue_cache.len() - 1);
                                    let path = self.queue_cache[idx].path.clone();
                                    self.send_high(TuiCommand::Play(path));
                                }
                            }
                        }
                    }
                    Some(KeyboardAction::Next) => {
                        self.set_last_action("Next");
                        self.send_high(TuiCommand::Next);
                    }
                    Some(KeyboardAction::Prev) => {
                        self.set_last_action("Previous");
                        self.send_high(TuiCommand::Prev);
                    }
                    Some(KeyboardAction::Stop) => {
                        self.set_last_action("Stop");
                        self.send_high(TuiCommand::Stop);
                    }
                    Some(KeyboardAction::VolumeUp) => {
                        let new_vol = (self.state.volume + 5).min(100);
                        self.send_high(TuiCommand::SetVolume(new_vol));
                        self.notify(format!("Volume: {}%", new_vol), NotificationKind::Info);
                    }
                    Some(KeyboardAction::VolumeDown) => {
                        self.set_last_action("Volume Down");
                        let new_vol = self.state.volume.saturating_sub(5);
                        self.send_high(TuiCommand::SetVolume(new_vol));
                        self.notify(format!("Volume: {}%", new_vol), NotificationKind::Info);
                    }
                    Some(KeyboardAction::SeekForward) => {
                        self.set_last_action("Seek Forward");
                        let pos = (self.display_position + 5.0).min(self.state.duration);
                        self.send_high(TuiCommand::Seek(pos));
                    }
                    Some(KeyboardAction::SeekBackward) => {
                        self.set_last_action("SeekBackward");
                        let pos = (self.display_position - 5.0).max(0.0);
                        self.send_high(TuiCommand::Seek(pos));
                    }
                    Some(KeyboardAction::ToggleMute) => {
                        self.set_last_action("Toggle Mute");
                        self.send_high(TuiCommand::ToggleMute);
                        if self.state.mute {
                            self.notify("Unmuted", NotificationKind::Info);
                        } else {
                            self.notify("Muted", NotificationKind::Warning);
                        }
                    }
                    Some(KeyboardAction::CycleRepeat) => {
                        self.set_last_action("Cycle Repeat");
                        let new_mode = match self.state.repeat {
                            RepeatMode::Off => RepeatMode::One,
                            RepeatMode::One => RepeatMode::All,
                            RepeatMode::All => RepeatMode::Off,
                        };
                        self.send_high(TuiCommand::CycleRepeat(new_mode));
                        self.notify(format!("Repeat: {:?}", new_mode), NotificationKind::Info);
                    }
                    Some(KeyboardAction::ToggleShuffle) => {
                        self.set_last_action("Toggle Shuffle");
                        self.send_high(TuiCommand::ToggleShuffle);
                        if self.state.shuffle {
                            self.notify("Shuffle OFF", NotificationKind::Info);
                        } else {
                            self.notify("Shuffle ON", NotificationKind::Info);
                        }
                    }
                    Some(KeyboardAction::ToggleFavourite) => {
                        self.set_last_action("Toggle Favourite");
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
                            self.notify("Favourite toggled", NotificationKind::Info);
                        }
                    }
                    Some(KeyboardAction::ClearQueue) => {
                        self.set_last_action("Clear Queue");
                        let tx = self.cmd_tx();
                        let _ = tx.send(TuiCommand::QueueClear).await;
                        self.notify("Queue cleared", NotificationKind::Info);
                    }
                    Some(KeyboardAction::CycleFooterPreset) => {
                        self.cycle_footer_preset();
                    }
                    Some(KeyboardAction::CycleProgressStyle) => {
                        self.progress_style = self.progress_style.next();
                        self.notify(
                            format!("Progress: {}", self.progress_style.name()),
                            NotificationKind::Info,
                        );
                    }
                    Some(KeyboardAction::ToggleVisualizer) => {
                        self.visualizer.toggle();
                        let state = if self.visualizer.is_enabled() {
                            "ON"
                        } else {
                            "OFF"
                        };
                        self.notify(format!("Visualizer: {}", state), NotificationKind::Info);
                    }
                    Some(KeyboardAction::CheckHealth) => {
                        self.send_high(TuiCommand::CheckHealth);
                    }
                    Some(KeyboardAction::FocusLeft) => {
                        if self.show_lyrics && self.current_tab == Tab::Library {
                            if self.lyrics_pane_focus {
                                // lyrics → right (track) pane
                                self.lyrics_pane_focus = false;
                                self.lyrics_manual_scroll = false;
                                self.library_pane_focus = false;
                            } else {
                                self.library_pane_focus = true;
                            }
                        } else {
                            match self.current_tab {
                                Tab::Library => self.library_pane_focus = true,
                                Tab::Settings => self.settings_pane_focus = true,
                            }
                        }
                    }
                    Some(KeyboardAction::FocusRight) => {
                        if self.show_lyrics && self.current_tab == Tab::Library {
                            if self.library_pane_focus {
                                // left → right (track) pane
                                self.library_pane_focus = false;
                            } else {
                                // right pane → lyrics pane
                                self.lyrics_pane_focus = true;
                            }
                        } else {
                            match self.current_tab {
                                Tab::Library => self.library_pane_focus = false,
                                Tab::Settings => self.settings_pane_focus = false,
                            }
                        }
                    }
                    Some(KeyboardAction::Back) => {
                        if self.lyrics_pane_focus {
                            // Exit lyrics focus back to the track pane.
                            self.lyrics_pane_focus = false;
                            self.lyrics_manual_scroll = false;
                        } else {
                            let is_narrow = self.terminal_cols < 60;
                            if is_narrow && self.show_lyrics {
                                self.show_lyrics = false;
                            } else if self.browse_detail.is_some() {
                                self.browse_detail = None;
                                self.scroll_offset = 0;
                                if self.library_category == 5 {
                                    self.spotify_playlist_tracks_cache.clear();
                                }
                            } else if !self.library_pane_focus && self.current_tab == Tab::Library {
                                self.library_pane_focus = true;
                            }
                        }
                    }
                    Some(KeyboardAction::FetchLyrics) => {
                        self.show_lyrics = !self.show_lyrics;
                        if !self.show_lyrics {
                            self.lyrics_pane_focus = false;
                            self.lyrics_manual_scroll = false;
                        }
                        if self.show_lyrics && self.current_lyrics.is_none() {
                            self.lyrics_fetching = true;
                            self.send_high(TuiCommand::FetchLyrics);
                        }
                        self.dismiss_track_popup();
                    }
                    Some(KeyboardAction::EnterFilter) => {
                        self.input_mode = InputMode::Searching;
                        self.dismiss_track_popup();
                    }
                    Some(KeyboardAction::MoveUp) => {
                        if self.lyrics_pane_focus && self.show_lyrics {
                            self.lyrics_manual_scroll = true;
                            self.lyrics_scroll = self.lyrics_scroll.saturating_sub(1);
                        } else {
                            match self.current_tab {
                                Tab::Library if self.library_pane_focus => {
                                    let new_cat = self.library_category.saturating_sub(1);
                                    if new_cat != self.library_category {
                                        self.browse_detail = None;
                                        self.scroll_offset = 0;
                                    }
                                    self.library_category = new_cat;
                                }
                                Tab::Settings if self.settings_pane_focus => {
                                    self.settings_category =
                                        self.settings_category.saturating_sub(1);
                                }
                                Tab::Settings => {
                                    self.settings_option = self.settings_option.saturating_sub(1);
                                }
                                Tab::Library => {
                                    self.scroll_offset = self.scroll_offset.saturating_sub(1);
                                    self.update_track_popup();
                                }
                            }
                        }
                    }
                    Some(KeyboardAction::MoveDown) => {
                        if self.lyrics_pane_focus && self.show_lyrics {
                            self.lyrics_manual_scroll = true;
                            let max = self
                                .current_lyrics
                                .as_ref()
                                .map(|l| l.lines.len().saturating_sub(1))
                                .unwrap_or(0);
                            self.lyrics_scroll = (self.lyrics_scroll + 1).min(max);
                        } else {
                            match self.current_tab {
                                Tab::Library if self.library_pane_focus => {
                                    let new_cat = (self.library_category + 1)
                                        .min(LIBRARY_CATEGORIES.len() - 1);
                                    if new_cat != self.library_category {
                                        self.browse_detail = None;
                                        self.scroll_offset = 0;
                                    }
                                    self.library_category = new_cat;
                                }
                                Tab::Settings if self.settings_pane_focus => {
                                    self.settings_category = (self.settings_category + 1)
                                        .min(NUM_SETTINGS_CATEGORIES.saturating_sub(1));
                                }
                                Tab::Settings => {
                                    let max =
                                        self.settings_options_for_category().saturating_sub(1);
                                    self.settings_option = (self.settings_option + 1).min(max);
                                }
                                Tab::Library => {
                                    let max_list = self.library_list_len().saturating_sub(1);
                                    self.scroll_offset = (self.scroll_offset + 1).min(max_list);
                                    self.update_track_popup();
                                }
                            }
                        }
                    }
                    Some(KeyboardAction::PageUp) => {
                        if self.lyrics_pane_focus && self.show_lyrics {
                            self.lyrics_manual_scroll = true;
                            let page = self.viewport_items.max(1);
                            self.lyrics_scroll = self.lyrics_scroll.saturating_sub(page);
                        } else if self.current_tab == Tab::Library && !self.library_pane_focus {
                            let page = self.viewport_items.max(1);
                            self.scroll_offset = self.scroll_offset.saturating_sub(page);
                            self.update_track_popup();
                        }
                    }
                    Some(KeyboardAction::PageDown) => {
                        if self.lyrics_pane_focus && self.show_lyrics {
                            self.lyrics_manual_scroll = true;
                            let page = self.viewport_items.max(1);
                            let max = self
                                .current_lyrics
                                .as_ref()
                                .map(|l| l.lines.len().saturating_sub(1))
                                .unwrap_or(0);
                            self.lyrics_scroll = (self.lyrics_scroll + page).min(max);
                        } else if self.current_tab == Tab::Library && !self.library_pane_focus {
                            let page = self.viewport_items.max(1);
                            let max_list = self.library_list_len().saturating_sub(1);
                            self.scroll_offset = (self.scroll_offset + page).min(max_list);
                            self.update_track_popup();
                        }
                    }
                    Some(KeyboardAction::Top) => {
                        if self.lyrics_pane_focus && self.show_lyrics {
                            self.lyrics_manual_scroll = true;
                            self.lyrics_scroll = 0;
                        } else if self.current_tab == Tab::Library && !self.library_pane_focus {
                            self.scroll_offset = 0;
                        }
                    }
                    Some(KeyboardAction::Bottom) => {
                        if self.lyrics_pane_focus && self.show_lyrics {
                            self.lyrics_manual_scroll = true;
                            let max = self
                                .current_lyrics
                                .as_ref()
                                .map(|l| l.lines.len().saturating_sub(1))
                                .unwrap_or(0);
                            self.lyrics_scroll = max;
                        } else if self.current_tab == Tab::Library && !self.library_pane_focus {
                            let max_list = self.library_list_len().saturating_sub(1);
                            self.scroll_offset = max_list;
                        }
                    }
                    Some(KeyboardAction::Select) => {
                        if self.current_tab == Tab::Library {
                            if self.library_pane_focus {
                                self.library_pane_focus = false;
                            } else if self.browse_detail.is_some() {
                                // In detail view: play the selected track
                                if self.library_category == 5 {
                                    // Spotify playlist: resolve track to a playable
                                    // local stream (via YouTube) and enqueue it.
                                    if self.scroll_offset < self.spotify_playlist_tracks_cache.len()
                                    {
                                        let playlist_id =
                                            self.browse_detail.clone().unwrap_or_default();
                                        let track_index = self.spotify_playlist_tracks_cache
                                            [self.scroll_offset]
                                            .index;
                                        let c = self.client.clone();
                                        let ipc_tx2 = self.ipc_tx.clone();
                                        tokio::spawn(async move {
                                            match c.spotify_resolve(&playlist_id, track_index).await
                                            {
                                                Ok(()) => {
                                                    let _ = ipc_tx2.send(IpcResult::Notification(
                                                        "Spotify track resolved & queued"
                                                            .to_string(),
                                                        NotificationKind::Success,
                                                    ));
                                                }
                                                Err(e) => {
                                                    let _ = ipc_tx2.send(IpcResult::Error(
                                                        format!("Spotify resolve failed: {e}"),
                                                    ));
                                                }
                                            }
                                        });
                                    }
                                } else {
                                    self.play_filtered_highlighted();
                                }
                            } else if self.library_category == 2 {
                                // Albums: select album → show its tracks
                                let albums = self.unique_albums();
                                if self.scroll_offset < albums.len() {
                                    self.browse_detail = Some(albums[self.scroll_offset].0.clone());
                                    self.scroll_offset = 0;
                                }
                            } else if self.library_category == 3 {
                                // Artists: select artist → show its tracks
                                let artists = self.unique_artists();
                                if self.scroll_offset < artists.len() {
                                    self.browse_detail =
                                        Some(artists[self.scroll_offset].0.clone());
                                    self.scroll_offset = 0;
                                }
                            } else if self.library_category == 4 {
                                // Playlists: select playlist → show its tracks
                                if self.scroll_offset < self.playlist_cache.len() {
                                    let playlist = self.playlist_cache[self.scroll_offset].clone();
                                    self.browse_detail = Some(playlist.name.clone());
                                    self.scroll_offset = 0;
                                    self.playlist_tracks_cache.clear();
                                    let c = self.client.clone();
                                    let ipc_tx2 = self.ipc_tx.clone();
                                    let pid = playlist.id;
                                    tokio::spawn(async move {
                                        if let Ok(DaemonRes::Tracks { tracks }) =
                                            c.library_get_playlist_tracks(pid).await
                                        {
                                            let _ = ipc_tx2.send(IpcResult::PlaylistTracks(tracks));
                                        }
                                    });
                                }
                            } else if self.library_category == 5 {
                                // Spotify: select playlist → show its cached tracks
                                if self.scroll_offset < self.spotify_playlists.len() {
                                    let playlist =
                                        self.spotify_playlists[self.scroll_offset].clone();
                                    self.browse_detail = Some(playlist.id.clone());
                                    self.scroll_offset = 0;
                                    self.spotify_playlist_tracks_cache.clear();
                                    let c = self.client.clone();
                                    let ipc_tx2 = self.ipc_tx.clone();
                                    let pid = playlist.id;
                                    tokio::spawn(async move {
                                        match c.spotify_playlist_tracks(&pid).await {
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
                                    });
                                }
                            } else {
                                // Default: play track from flat list
                                self.play_filtered_highlighted();
                            }
                        } else if self.current_tab == Tab::Settings && !self.settings_pane_focus {
                            let tx = self.cmd_tx();
                            let opt = self.settings_option;
                            match self.settings_category {
                                0 => match opt {
                                    0 => {
                                        // Master Volume: cycle in 10% increments
                                        let current = self.state.master_volume;
                                        let new_vol = if current >= 100 {
                                            50
                                        } else {
                                            (current + 10).min(100)
                                        };
                                        self.send_high(TuiCommand::SetMasterVolume(new_vol));
                                        self.notify(
                                            format!("Master Volume: {}%", new_vol),
                                            NotificationKind::Info,
                                        );
                                    }
                                    1 => {
                                        // Mute toggle
                                        let muted = !self.state.mute;
                                        self.send_high(TuiCommand::SetVolume(if muted {
                                            0
                                        } else {
                                            self.state.volume
                                        }));
                                        self.state.mute = muted;
                                    }
                                    _ => {}
                                },
                                1 => {
                                    if opt == 1 {
                                        // Cookie File: set directory
                                        let c = self.client.clone();
                                        let current = self.cookie_file.clone();
                                        let new_path = if current.is_some() {
                                            None
                                        } else {
                                            // Default to common cookie path
                                            let home = std::env::var("HOME").unwrap_or_default();
                                            Some(format!("{home}/.cookies/youtube.txt"))
                                        };
                                        let display = new_path
                                            .clone()
                                            .unwrap_or_else(|| "(none)".to_string());
                                        self.cookie_file = new_path.clone();
                                        let cf = new_path.clone();
                                        tokio::spawn(async move {
                                            let _ =
                                                c.yt_set_config(None, cf, None, None, None).await;
                                        });
                                        self.notify(
                                            format!("Cookie file: {display}"),
                                            crate::app::NotificationKind::Info,
                                        );
                                    }
                                }
                                2 => match opt {
                                    0 => {
                                        // Repeat cycle
                                        let next = match self.state.repeat {
                                            gtm_core::state::RepeatMode::Off => {
                                                gtm_core::state::RepeatMode::One
                                            }
                                            gtm_core::state::RepeatMode::One => {
                                                gtm_core::state::RepeatMode::All
                                            }
                                            gtm_core::state::RepeatMode::All => {
                                                gtm_core::state::RepeatMode::Off
                                            }
                                        };
                                        let c = self.client.clone();
                                        tokio::spawn(async move {
                                            let _ = c.cycle_repeat(next).await;
                                        });
                                        self.state.repeat = next;
                                    }
                                    1 => {
                                        // Shuffle toggle
                                        let c = self.client.clone();
                                        tokio::spawn(async move {
                                            let _ = c.toggle_shuffle().await;
                                        });
                                        self.state.shuffle = !self.state.shuffle;
                                    }
                                    2 => {
                                        // Crossfade toggle
                                        let enabled = !self
                                            .state
                                            .crossfade
                                            .as_ref()
                                            .map(|c| c.enabled)
                                            .unwrap_or(false);
                                        let dur = self
                                            .state
                                            .crossfade
                                            .as_ref()
                                            .map(|c| c.duration_secs)
                                            .unwrap_or(self.crossfade_duration);
                                        let _ = tx.send(TuiCommand::Crossfade(enabled, dur)).await;
                                    }
                                    3 => {
                                        // Easing cycle
                                        let current = self
                                            .state
                                            .crossfade
                                            .as_ref()
                                            .map(|c| c.easing)
                                            .unwrap_or(Easing::Linear);
                                        let next = match current {
                                            Easing::Linear => Easing::Smoothstep,
                                            Easing::Smoothstep => Easing::Logarithmic,
                                            Easing::Logarithmic => Easing::SlowFadeInFastFadeOut,
                                            Easing::SlowFadeInFastFadeOut => {
                                                Easing::FastFadeInSlowFadeOut
                                            }
                                            Easing::FastFadeInSlowFadeOut => Easing::EqualPower,
                                            Easing::EqualPower => Easing::Exponential,
                                            Easing::Exponential => Easing::Linear,
                                        };
                                        let _ = tx.send(TuiCommand::SetCrossfadeEasing(next)).await;
                                        if let Some(ref mut cf) = self.state.crossfade {
                                            cf.easing = next;
                                        }
                                    }
                                    4 => {
                                        // EQ Enabled toggle
                                        let new_enabled = !self.state.eq_enabled;
                                        self.state.eq_enabled = new_enabled;
                                        let c = self.client.clone();
                                        tokio::spawn(async move {
                                            let _ = c.set_eq_enabled(new_enabled).await;
                                        });
                                    }
                                    _ => {}
                                },
                                3 => match opt {
                                    1 => {
                                        // Transparent BG toggle
                                        self.transparent_bg = !self.transparent_bg;
                                        save_prefs(&self.current_prefs());
                                    }
                                    2 => {
                                        // Sync Covers
                                        spawn_sync_and_wait(
                                            self.client.clone(),
                                            gtm_core::ipc::SyncKind::Covers,
                                            "Covers",
                                            self.ipc_tx.clone(),
                                        );
                                    }
                                    3 => {
                                        // Sync Lyrics
                                        spawn_sync_and_wait(
                                            self.client.clone(),
                                            gtm_core::ipc::SyncKind::Lyrics,
                                            "Lyrics",
                                            self.ipc_tx.clone(),
                                        );
                                    }
                                    4 => {
                                        // Sync Metadata
                                        spawn_sync_and_wait(
                                            self.client.clone(),
                                            gtm_core::ipc::SyncKind::Metadata,
                                            "Metadata",
                                            self.ipc_tx.clone(),
                                        );
                                    }
                                    5 => {
                                        // Footer Preset cycle
                                        self.cycle_footer_preset();
                                        if let Some(p) = self.footer_presets.get(self.footer_preset)
                                        {
                                            self.notify(
                                                format!("Footer: {}", p.name),
                                                NotificationKind::Info,
                                            );
                                        }
                                    }
                                    _ => {}
                                },
                                4 => match opt {
                                    3 => {
                                        // Link Spotify account: open token input picker
                                        self.spotify_token_input.clear();
                                        self.pickers.open(PickerId::SpotifySearch);
                                    }
                                    4 => {
                                        // Sync playlists now
                                        let c = self.client.clone();
                                        let ipc_tx = self.ipc_tx.clone();
                                        tokio::spawn(async move {
                                            match c.spotify_sync().await {
                                                Ok(()) => {
                                                    let _ = ipc_tx.send(IpcResult::Notification(
                                                        "Spotify sync complete".to_string(),
                                                        NotificationKind::Success,
                                                    ));
                                                }
                                                Err(e) => {
                                                    let _ = ipc_tx.send(IpcResult::Error(format!(
                                                        "Spotify sync failed: {e}"
                                                    )));
                                                }
                                            }
                                            if let Ok(status) = c.spotify_status().await {
                                                let _ =
                                                    ipc_tx.send(IpcResult::SpotifyStatus(status));
                                            }
                                            if let Ok(playlists) = c.spotify_playlists().await {
                                                let _ = ipc_tx
                                                    .send(IpcResult::SpotifyPlaylists(playlists));
                                            }
                                        });
                                    }
                                    5 => {
                                        // Unlink account
                                        let c = self.client.clone();
                                        let ipc_tx = self.ipc_tx.clone();
                                        tokio::spawn(async move {
                                            match c.spotify_clear().await {
                                                Ok(status) => {
                                                    let _ = ipc_tx
                                                        .send(IpcResult::SpotifyStatus(status));
                                                    let _ = ipc_tx.send(IpcResult::Notification(
                                                        "Spotify account unlinked".to_string(),
                                                        NotificationKind::Info,
                                                    ));
                                                }
                                                Err(e) => {
                                                    let _ = ipc_tx.send(IpcResult::Error(format!(
                                                        "Spotify unlink failed: {e}"
                                                    )));
                                                }
                                            }
                                        });
                                    }
                                    _ => {}
                                },
                                _ => {}
                            }
                        }
                    }
                    Some(KeyboardAction::Delete) => {
                        if self.current_tab == Tab::Library && !self.library_pane_focus {
                            let track_data = self
                                .filtered_tracks()
                                .get(self.scroll_offset)
                                .map(|t| (t.id, t.title.clone()));
                            if let Some((track_id, track_name)) = track_data {
                                self.pending_delete = Some((track_id, track_name.clone()));
                                self.notify(
                                    format!(
                                        "Delete \"{track_name}\"? Enter to confirm, Esc to cancel"
                                    ),
                                    NotificationKind::Info,
                                );
                            }
                        }
                    }
                    Some(KeyboardAction::ToggleMultiselect) => {
                        if self.current_tab == Tab::Library && !self.library_pane_focus {
                            self.multiselect_mode = !self.multiselect_mode;
                            if !self.multiselect_mode {
                                self.selected_indices.clear();
                            }
                            let msg = if self.multiselect_mode {
                                "Multiselect ON"
                            } else {
                                "Multiselect OFF"
                            };
                            self.notify(msg, NotificationKind::Info);
                        }
                    }
                    Some(KeyboardAction::AddToQueue) => {
                        if self.current_tab == Tab::Library && !self.library_pane_focus {
                            let tracks = self.filtered_tracks();
                            let indices: Vec<usize> =
                                if self.multiselect_mode && !self.selected_indices.is_empty() {
                                    self.selected_indices.iter().copied().collect()
                                } else {
                                    vec![self.scroll_offset]
                                };
                            let mut added = 0;
                            for idx in indices {
                                if let Some(track) = tracks.get(idx) {
                                    let c = self.client.clone();
                                    let path = track.path.clone();
                                    tokio::spawn(async move {
                                        let _ = c.queue_add(&path, None).await;
                                    });
                                    added += 1;
                                }
                            }
                            self.fetch_queue().await;
                            self.notify(
                                format!("Added {added} track(s) to queue"),
                                NotificationKind::Info,
                            );
                        }
                    }
                    Some(KeyboardAction::AddToPlaylist) => {
                        if self.current_tab == Tab::Library && !self.library_pane_focus {
                            let tracks = self.filtered_tracks();
                            let indices: Vec<i64> =
                                if self.multiselect_mode && !self.selected_indices.is_empty() {
                                    self.selected_indices
                                        .iter()
                                        .filter_map(|i| tracks.get(*i).map(|t| t.id))
                                        .collect()
                                } else {
                                    tracks
                                        .get(self.scroll_offset)
                                        .map(|t| vec![t.id])
                                        .unwrap_or_default()
                                };
                            if !indices.is_empty() {
                                self.pending_playlist_track_ids = indices;
                                self.pickers.open(PickerId::PlaylistSelect);
                            }
                        }
                    }
                    Some(KeyboardAction::DeleteFromList) => {
                        if self.current_tab == Tab::Library && !self.library_pane_focus {
                            if self.library_category == 4 && self.browse_detail.is_some() {
                                // In playlist view — remove selected track from playlist
                                let filtered = self.filtered_tracks();
                                if let Some(track) = filtered.get(self.scroll_offset) {
                                    let track_id = track.id;
                                    if let Some(pl) = self
                                        .playlist_cache
                                        .iter()
                                        .find(|p| self.browse_detail.as_deref() == Some(&p.name))
                                    {
                                        let playlist_id = pl.id;
                                        let tx = self.cmd_tx();
                                        let _ = tx
                                            .send(TuiCommand::RemoveFromPlaylist(
                                                playlist_id,
                                                track_id,
                                            ))
                                            .await;
                                        self.notify(
                                            "Removed from playlist",
                                            NotificationKind::Info,
                                        );
                                    }
                                }
                            } else {
                                self.notify(
                                    "Remove from list only available in playlist view",
                                    NotificationKind::Info,
                                );
                            }
                        }
                    }
                    Some(KeyboardAction::JumpToEnd) => {
                        if self.current_tab == Tab::Library && !self.library_pane_focus {
                            let max = self.library_list_len().saturating_sub(1);
                            self.scroll_offset = max;
                        }
                    }
                    Some(KeyboardAction::EditMetadata) => {
                        if self.current_tab == Tab::Library && !self.library_pane_focus {
                            let track_data = {
                                let tracks = self.filtered_tracks();
                                tracks.get(self.scroll_offset).map(|t| {
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
                            if let Some((id, title, artist, album, genre, year, track_num)) =
                                track_data
                            {
                                self.metadata_edit_track_id = Some(id);
                                self.metadata_fields = [
                                    title,
                                    artist,
                                    album,
                                    String::new(),
                                    genre,
                                    year.map_or(String::new(), |y| y.to_string()),
                                    track_num.map_or(String::new(), |n| n.to_string()),
                                ];
                                self.metadata_field_idx = 0;
                                self.pickers.open(PickerId::EditMetadata);
                            }
                        }
                    }
                    None => {
                        match key.code {
                            KeyCode::Char('q') => {
                                if self.browse_detail.is_some() {
                                    self.browse_detail = None;
                                    self.scroll_offset = 0;
                                } else {
                                    return false;
                                }
                            }
                            KeyCode::Esc => {
                                if self.browse_detail.is_some() {
                                    self.browse_detail = None;
                                    self.scroll_offset = 0;
                                }
                            }
                            KeyCode::Char('c') if self.current_tab == Tab::Settings => {
                                // Toggle crossfade in Settings tab
                                let enabled = !self
                                    .state
                                    .crossfade
                                    .as_ref()
                                    .map(|c| c.enabled)
                                    .unwrap_or(false);
                                let dur = self
                                    .state
                                    .crossfade
                                    .as_ref()
                                    .map(|c| c.duration_secs)
                                    .unwrap_or(self.crossfade_duration);
                                let tx = self.cmd_tx();
                                let _ = tx.send(TuiCommand::Crossfade(enabled, dur)).await;
                            }
                            KeyCode::Char('C') if self.current_tab == Tab::Settings => {
                                // Cycle crossfade duration (3, 5, 7, 10, 15, 30)
                                let dur = self
                                    .state
                                    .crossfade
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
                                let enabled = self
                                    .state
                                    .crossfade
                                    .as_ref()
                                    .map(|c| c.enabled)
                                    .unwrap_or(true);
                                let tx = self.cmd_tx();
                                let _ = tx.send(TuiCommand::Crossfade(enabled, new_dur)).await;
                            }
                            KeyCode::Char('S') => {
                                // Sync covers for tracks missing cover art
                                self.notify("Syncing covers...", NotificationKind::Info);
                                spawn_sync_and_wait(
                                    self.client.clone(),
                                    gtm_core::ipc::SyncKind::Covers,
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

    async fn handle_picker_key(&mut self, key: event::KeyEvent) {
        let tx = self.cmd_tx();

        if matches!(self.pickers.top().map(|o| o.id), Some(PickerId::SleepTimer)) {
            if self.sleep_timer_input_mode {
                match key.code {
                    KeyCode::Esc => {
                        self.sleep_timer_input_mode = false;
                        self.sleep_timer_input_buf.clear();
                    }
                    KeyCode::Enter => {
                        if let Ok(m) = self.sleep_timer_input_buf.parse::<u32>() {
                            self.sleep_timer_minutes = m.min(180);
                        }
                        self.sleep_timer_input_mode = false;
                        self.sleep_timer_input_buf.clear();
                    }
                    KeyCode::Backspace => {
                        self.sleep_timer_input_buf.pop();
                    }
                    KeyCode::Char(c) if c.is_ascii_digit() => {
                        self.sleep_timer_input_buf.push(c);
                    }
                    _ => {}
                }
                return;
            }
            match key.code {
                KeyCode::Esc => {
                    self.sleep_timer_remaining = None;
                    self.sleep_timer_minutes = 30;
                    self.sleep_timer_input_mode = false;
                    self.sleep_timer_input_buf.clear();
                    self.pickers.close_top();
                    return;
                }
                KeyCode::Char('h') | KeyCode::Left => {
                    self.sleep_timer_minutes = self.sleep_timer_minutes.saturating_sub(5);
                    return;
                }
                KeyCode::Char('l') | KeyCode::Right => {
                    self.sleep_timer_minutes = (self.sleep_timer_minutes + 5).min(180);
                    return;
                }
                KeyCode::Char('-') => {
                    self.sleep_timer_minutes = self.sleep_timer_minutes.saturating_sub(1);
                    return;
                }
                KeyCode::Char('+') | KeyCode::Char('=') => {
                    self.sleep_timer_minutes = (self.sleep_timer_minutes + 1).min(180);
                    return;
                }
                KeyCode::Enter => {
                    let mins = self.sleep_timer_minutes;
                    self.sleep_timer_remaining = Some(mins as u64);
                    self.send_high(TuiCommand::SetSleepTimer(mins));
                    self.notify(
                        format!("Sleep timer set: {} min", mins),
                        NotificationKind::Info,
                    );
                    self.pickers.close_top();
                    return;
                }
                KeyCode::Char('i') => {
                    self.sleep_timer_input_mode = true;
                    self.sleep_timer_input_buf.clear();
                    return;
                }
                KeyCode::Char('c') => {
                    self.sleep_timer_remaining = None;
                    self.send_high(TuiCommand::CancelSleepTimer);
                    self.notify("Sleep timer cancelled", NotificationKind::Info);
                    return;
                }
                KeyCode::Up | KeyCode::Char('j') => {
                    let quick_opts = [5u32, 10, 15, 30, 60, 90, 120];
                    if let Some(top) = self.pickers.top_mut() {
                        top.selected = (top.selected + 1) % quick_opts.len();
                        self.sleep_timer_minutes = quick_opts[top.selected];
                    }
                    return;
                }
                KeyCode::Down | KeyCode::Char('k') => {
                    let quick_opts = [5u32, 10, 15, 30, 60, 90, 120];
                    if let Some(top) = self.pickers.top_mut() {
                        top.selected = if top.selected == 0 {
                            quick_opts.len() - 1
                        } else {
                            top.selected - 1
                        };
                        self.sleep_timer_minutes = quick_opts[top.selected];
                    }
                    return;
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
                if let Some(top) = self.pickers.top() {
                    match top.id {
                        PickerId::SleepTimer => self.sleep_timer_remaining = None,
                        PickerId::SpotifySearch => self.spotify_token_input.clear(),
                        _ => {}
                    }
                }
                self.pickers.close_top();
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
            KeyCode::Char('/') if is_help && !ctrl_or_alt => {
                if let Some(top) = self.pickers.top_mut() {
                    top.query.clear();
                }
            }
            KeyCode::Char('n') if is_help && !ctrl_or_alt => {
                let total = self.help_picker_total();
                if let Some(top) = self.pickers.top_mut() {
                    if total > 0 {
                        top.selected = (top.selected + 1).min(total - 1);
                    }
                }
            }
            KeyCode::Char('N') if is_help && !ctrl_or_alt => {
                if let Some(top) = self.pickers.top_mut() {
                    top.selected = top.selected.saturating_sub(1);
                }
            }
            // Queue move up/down (Ctrl+K/J) must come before plain k/j
            KeyCode::Char('k') if key.modifiers == KeyModifiers::CONTROL => {
                if let Some(top) = self.pickers.top() {
                    if top.id == PickerId::Queue && !self.queue_cache.is_empty() {
                        let idx = top.selected.min(self.queue_cache.len() - 1);
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
            }
            KeyCode::Char('j') if key.modifiers == KeyModifiers::CONTROL => {
                if let Some(top) = self.pickers.top() {
                    if top.id == PickerId::Queue && !self.queue_cache.is_empty() {
                        let idx = top.selected.min(self.queue_cache.len() - 1);
                        if idx < self.queue_cache.len() - 1 {
                            let _ = tx
                                .send(TuiCommand::QueueMove(idx as u64, (idx + 1) as u64))
                                .await;
                            self.fetch_queue().await;
                        }
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
                );
                let is_metadata = matches!(
                    self.pickers.top().map(|o| o.id),
                    Some(PickerId::EditMetadata)
                );
                if is_metadata {
                    if self.metadata_field_idx > 0 {
                        self.metadata_field_idx -= 1;
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
                if let Some(top) = self.pickers.top_mut() {
                    top.selected = top.selected.saturating_sub(1);
                    let is_theme = top.id == PickerId::ThemePicker;
                    let selected = top.selected;
                    if is_theme {
                        self.apply_theme_index(selected);
                    }
                }
                self.clamp_picker_selection();
                self.apply_eq_on_navigation().await;
            }
            KeyCode::Down | KeyCode::Char('j') => {
                let has_input = matches!(
                    self.pickers.top().map(|o| o.id),
                    Some(PickerId::YTSearch)
                        | Some(PickerId::SearchLibrary)
                        | Some(PickerId::CommandPalette)
                        | Some(PickerId::ThemePicker)
                );
                let is_metadata = matches!(
                    self.pickers.top().map(|o| o.id),
                    Some(PickerId::EditMetadata)
                );
                if is_metadata {
                    if self.metadata_field_idx < 6 {
                        self.metadata_field_idx += 1;
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
                if let Some(top) = self.pickers.top_mut() {
                    top.selected += 1;
                    let is_theme = top.id == PickerId::ThemePicker;
                    let selected = top.selected;
                    if is_theme {
                        self.apply_theme_index(selected);
                    }
                }
                self.clamp_picker_selection();
                self.apply_eq_on_navigation().await;
            }
            KeyCode::Enter if key.modifiers.contains(KeyModifiers::CONTROL) => {
                if matches!(
                    self.pickers.top().map(|o| o.id),
                    Some(PickerId::EditMetadata)
                ) {
                    if let Some(track_id) = self.metadata_edit_track_id {
                        let title = self.metadata_fields[0].clone();
                        let artist = self.metadata_fields[1].clone();
                        let album = self.metadata_fields[2].clone();
                        let genre = self.metadata_fields[4].clone();
                        let year = self.metadata_fields[5].parse::<i32>().ok();
                        let track_number = self.metadata_fields[6].parse::<i32>().ok();
                        let client = self.client.clone();
                        let ipc_tx = self.ipc_tx.clone();
                        tokio::spawn(async move {
                            let patch = gtm_core::MetadataPatch {
                                title: Some(title),
                                artist: Some(artist),
                                album: Some(album),
                                genre: Some(genre),
                                year,
                                track_number,
                            };
                            let _ = client.library_update_metadata(track_id, patch).await;
                            let _ = ipc_tx.send(IpcResult::Notification(
                                "Metadata saved".to_string(),
                                NotificationKind::Success,
                            ));
                        });
                        self.metadata_edit_track_id = None;
                    }
                    self.pickers.close_top();
                }
            }
            KeyCode::Enter => {
                // Dispatch based on picker type
                if let Some(top) = self.pickers.top() {
                    match top.id {
                        PickerId::SpotifySearch => {
                            let token = self.spotify_token_input.clone();
                            if token.trim().is_empty() {
                                self.notify(
                                    "Enter a Spotify access token to link your account",
                                    NotificationKind::Info,
                                );
                            } else {
                                let c = self.client.clone();
                                let ipc_tx = self.ipc_tx.clone();
                                tokio::spawn(async move {
                                    match c.spotify_set_token(&token).await {
                                        Ok(status) => {
                                            let _ = ipc_tx.send(IpcResult::SpotifyStatus(status));
                                            let _ = ipc_tx.send(IpcResult::Notification(
                                                "Spotify account linked".to_string(),
                                                NotificationKind::Success,
                                            ));
                                        }
                                        Err(e) => {
                                            let _ = ipc_tx.send(IpcResult::Error(format!(
                                                "Spotify link failed: {e}"
                                            )));
                                        }
                                    }
                                });
                                self.spotify_token_input.clear();
                                self.pickers.close_top();
                            }
                        }
                        PickerId::Queue => {
                            if !self.queue_cache.is_empty() {
                                let idx = top.selected.min(self.queue_cache.len() - 1);
                                let path = self.queue_cache[idx].path.clone();
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
                        PickerId::CommandPalette => {
                            let commands = crate::ui::COMMAND_PALETTE_COMMANDS;
                            let query = top.query.to_lowercase();
                            let filtered: Vec<&(&str, &str)> = if query.is_empty() {
                                commands.iter().collect()
                            } else {
                                commands
                                    .iter()
                                    .filter(|c| {
                                        let lower = c.0.to_lowercase();
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
                                let raw = cmd.0.to_lowercase();
                                // Strip leading icon (skip to first ASCII letter)
                                let label = raw
                                    .chars()
                                    .skip_while(|c| !c.is_ascii_alphabetic())
                                    .collect::<String>();
                                if label.starts_with("play/pause") {
                                    self.send_high(TuiCommand::PlayPause);
                                } else if label.starts_with("next track") {
                                    self.send_high(TuiCommand::Next);
                                } else if label.starts_with("prev track") {
                                    self.send_high(TuiCommand::Prev);
                                } else if label.starts_with("volume up") {
                                    let new_vol = (self.state.volume + 5).min(100);
                                    self.send_high(TuiCommand::SetVolume(new_vol));
                                } else if label.starts_with("volume down") {
                                    let new_vol = self.state.volume.saturating_sub(5);
                                    self.send_high(TuiCommand::SetVolume(new_vol));
                                } else if label.starts_with("mute") {
                                    self.send_high(TuiCommand::ToggleMute);
                                } else if label.starts_with("repeat") {
                                    let new_mode = match self.state.repeat {
                                        RepeatMode::Off => RepeatMode::One,
                                        RepeatMode::One => RepeatMode::All,
                                        RepeatMode::All => RepeatMode::Off,
                                    };
                                    self.send_high(TuiCommand::CycleRepeat(new_mode));
                                } else if label.starts_with("shuffle") {
                                    self.send_high(TuiCommand::ToggleShuffle);
                                } else if label.starts_with("quit") {
                                    self.pending_quit = true;
                                } else if label.starts_with("tab cycle") {
                                    self.current_tab = match self.current_tab {
                                        Tab::Library => Tab::Settings,
                                        Tab::Settings => Tab::Library,
                                    };
                                } else if label.starts_with("library") {
                                    self.current_tab = Tab::Library;
                                } else if label.starts_with("settings") {
                                    self.current_tab = Tab::Settings;
                                } else if label.starts_with("queue") {
                                    self.pickers.open(PickerId::Queue);
                                } else if label.starts_with("youtube") {
                                    self.pickers.open(PickerId::YTSearch);
                                } else if label.starts_with("search lib") {
                                    self.pickers.open(PickerId::SearchLibrary);
                                } else if label.starts_with("eq") {
                                    self.pickers.open(PickerId::Equalizer);
                                } else if label.starts_with("sleeptimer") {
                                    self.pickers.open(PickerId::SleepTimer);
                                } else if label.starts_with("themepicker") {
                                    self.pickers.open_with_selection(
                                        PickerId::ThemePicker,
                                        self.theme_index,
                                    );
                                } else if label.starts_with("sound fx") {
                                    self.pickers.open(PickerId::SoundEffects);
                                } else if label.starts_with("about") {
                                    self.pickers.open(PickerId::About);
                                } else if label.starts_with("search") {
                                    self.pickers.open(PickerId::SearchLibrary);
                                } else if label.starts_with("spotify") {
                                    self.pickers.open(PickerId::SpotifySearch);
                                } else if label.starts_with("fetch lyrics") {
                                    self.show_lyrics = true;
                                    self.send_high(TuiCommand::FetchLyrics);
                                } else if label.starts_with("progress style") {
                                    self.progress_style = self.progress_style.next();
                                    self.notify(
                                        format!("Progress: {}", self.progress_style.name()),
                                        crate::app::NotificationKind::Info,
                                    );
                                } else if label.starts_with("visualizer") {
                                    self.visualizer.toggle();
                                    let state = if self.visualizer.is_enabled() {
                                        "ON"
                                    } else {
                                        "OFF"
                                    };
                                    self.notify(
                                        format!("Visualizer: {}", state),
                                        crate::app::NotificationKind::Info,
                                    );
                                }
                            }
                            self.pickers.close_top();
                        }
                        PickerId::Equalizer => {
                            // Apply selected EQ preset
                            let presets = [
                                gtm_core::state::EqPreset::Flat,
                                gtm_core::state::EqPreset::Normal,
                                gtm_core::state::EqPreset::Pop,
                                gtm_core::state::EqPreset::Rock,
                                gtm_core::state::EqPreset::Jazz,
                                gtm_core::state::EqPreset::Classical,
                                gtm_core::state::EqPreset::Bass,
                                gtm_core::state::EqPreset::Vocal,
                                gtm_core::state::EqPreset::Electronic,
                                gtm_core::state::EqPreset::HipHop,
                                gtm_core::state::EqPreset::Latin,
                                gtm_core::state::EqPreset::Acoustic,
                                gtm_core::state::EqPreset::Podcast,
                                gtm_core::state::EqPreset::Dance,
                                gtm_core::state::EqPreset::Headphones,
                                gtm_core::state::EqPreset::Speaker,
                            ];
                            let idx = top.selected.min(presets.len() - 1);
                            let c = self.client.clone();
                            tokio::spawn(async move {
                                let _ = c.set_eq_preset(presets[idx]).await;
                            });
                            self.pickers.close_top();
                        }
                        PickerId::ThemePicker => {
                            let idx = top.selected;
                            self.apply_theme_index(idx);
                            self.pickers.close_top();
                        }
                        PickerId::SearchLibrary => {
                            let q = top.query.to_lowercase();
                            let filtered: Vec<&gtm_core::track::TrackInfo> = if q.is_empty() {
                                self.tracks_cache.iter().collect()
                            } else {
                                self.tracks_cache
                                    .iter()
                                    .filter(|t| {
                                        t.title.to_lowercase().contains(&q)
                                            || t.artist.to_lowercase().contains(&q)
                                    })
                                    .collect()
                            };
                            if !filtered.is_empty() {
                                let idx = top.selected.min(filtered.len() - 1);
                                let path = filtered[idx].path.clone();
                                self.send_high(TuiCommand::Play(path));
                            }
                            self.pickers.close_top();
                        }
                        PickerId::SoundEffects => {
                            let sel = top.selected;
                            match sel {
                                1 => {
                                    // Reverb toggle
                                    let new_enabled = !self.state.reverb.enabled;
                                    let room_size = self.state.reverb.room_size;
                                    self.state.reverb.enabled = new_enabled;
                                    let c = self.client.clone();
                                    tokio::spawn(async move {
                                        let _ = c.set_reverb(new_enabled, room_size).await;
                                    });
                                    self.pickers.close_top();
                                }
                                _ => {
                                    self.pickers.close_top();
                                }
                            }
                        }
                        PickerId::EditMetadata => {
                            if self.metadata_field_idx < 6 {
                                self.metadata_field_idx += 1;
                            } else {
                                if let Some(track_id) = self.metadata_edit_track_id {
                                    let title = self.metadata_fields[0].clone();
                                    let artist = self.metadata_fields[1].clone();
                                    let album = self.metadata_fields[2].clone();
                                    let genre = self.metadata_fields[4].clone();
                                    let year = self.metadata_fields[5].parse::<i32>().ok();
                                    let track_number = self.metadata_fields[6].parse::<i32>().ok();
                                    let client = self.client.clone();
                                    let ipc_tx = self.ipc_tx.clone();
                                    tokio::spawn(async move {
                                        let patch = gtm_core::MetadataPatch {
                                            title: Some(title),
                                            artist: Some(artist),
                                            album: Some(album),
                                            genre: Some(genre),
                                            year,
                                            track_number,
                                        };
                                        let _ =
                                            client.library_update_metadata(track_id, patch).await;
                                        let _ = ipc_tx.send(IpcResult::Notification(
                                            "Metadata saved".to_string(),
                                            NotificationKind::Success,
                                        ));
                                    });
                                    self.metadata_edit_track_id = None;
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
                    let url = self.yt_results_cache[idx].url.clone();
                    let _ = tx.send(TuiCommand::YtDownload(url)).await;
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
            KeyCode::Char(c) if !ctrl_or_alt => {
                if let Some(top) = self.pickers.top_mut() {
                    match top.id {
                        PickerId::YTSearch
                        | PickerId::SearchLibrary
                        | PickerId::CommandPalette
                        | PickerId::ThemePicker
                        | PickerId::Help => {
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
                            self.metadata_fields[self.metadata_field_idx].push(c);
                        }
                        PickerId::SpotifySearch => {
                            self.spotify_token_input.push(c);
                        }
                        _ => {}
                    }
                }
            }
            KeyCode::Tab => {
                if let Some(top) = self.pickers.top() {
                    if top.id == PickerId::EditMetadata {
                        self.metadata_field_idx = (self.metadata_field_idx + 1) % 7;
                    }
                }
            }
            KeyCode::Backspace => {
                if let Some(top) = self.pickers.top_mut() {
                    match top.id {
                        PickerId::EditMetadata => {
                            self.metadata_fields[self.metadata_field_idx].pop();
                        }
                        PickerId::SpotifySearch => {
                            self.spotify_token_input.pop();
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

    async fn apply_eq_on_navigation(&mut self) {
        if let Some(top) = self.pickers.top() {
            if top.id == PickerId::Equalizer {
                let presets = [
                    gtm_core::state::EqPreset::Flat,
                    gtm_core::state::EqPreset::Normal,
                    gtm_core::state::EqPreset::Pop,
                    gtm_core::state::EqPreset::Rock,
                    gtm_core::state::EqPreset::Jazz,
                    gtm_core::state::EqPreset::Classical,
                    gtm_core::state::EqPreset::Bass,
                    gtm_core::state::EqPreset::Vocal,
                    gtm_core::state::EqPreset::Electronic,
                    gtm_core::state::EqPreset::HipHop,
                    gtm_core::state::EqPreset::Latin,
                    gtm_core::state::EqPreset::Acoustic,
                    gtm_core::state::EqPreset::Podcast,
                    gtm_core::state::EqPreset::Dance,
                    gtm_core::state::EqPreset::Headphones,
                    gtm_core::state::EqPreset::Speaker,
                ];
                let idx = top.selected.min(presets.len() - 1);
                self.send_high(TuiCommand::SetEqPreset(presets[idx]));
                self.state.eq_preset = presets[idx];
            }
        }
    }

    /// Interleave YT search results: insert one playlist entry after every 3 track entries.
    fn interleave_yt_results(
        mut results: Vec<gtm_core::track::YTSearchResult>,
    ) -> Vec<gtm_core::track::YTSearchResult> {
        let tracks: Vec<_> = results.drain(..).filter(|r| !r.is_playlist).collect();
        let playlists: Vec<_> = results; // remaining are playlists
        let mut out = Vec::with_capacity(tracks.len() + playlists.len());
        let mut pl_idx = 0;
        for (i, track) in tracks.into_iter().enumerate() {
            out.push(track);
            if (i + 1) % 3 == 0 && pl_idx < playlists.len() {
                out.push(playlists[pl_idx].clone());
                pl_idx += 1;
            }
        }
        // Append remaining playlists
        while pl_idx < playlists.len() {
            out.push(playlists[pl_idx].clone());
            pl_idx += 1;
        }
        out
    }
}
