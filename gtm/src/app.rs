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
use gtm_core::state::{DaemonState, Easing, PlaybackStatus, RepeatMode, Tab};
use gtm_core::track::{Playlist, TrackInfo, YTSearchResult};
use ratatui::layout::Alignment;
use ratatui::Terminal;
use ratatui::widgets::Paragraph;
use ratatui_image::picker::Picker;
use ratatui_image::protocol::StatefulProtocol;
use tokio::sync::mpsc;

use base64::Engine;

use crate::footer;
use crate::keymap::{default_keybindings, KeyContext, KeyboardAction};
use crate::overlay::{OverlayCtx, OverlayId, OverlayManager};
use crate::theme::{AppTheme, THEMES};
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

#[derive(serde::Serialize, serde::Deserialize)]
struct Prefs {
    theme_index: usize,
    #[serde(default)]
    transparent_bg: bool,
    #[serde(default)]
    footer_preset: usize,
}

fn load_prefs() -> Prefs {
    let path = prefs_path();
    let s = match std::fs::read_to_string(path) {
        Ok(s) => s,
        Err(_) => return Prefs { theme_index: 0, transparent_bg: false, footer_preset: 0 },
    };
    if let Ok(p) = serde_json::from_str::<Prefs>(&s) {
        return p;
    }
    if let Ok(idx) = serde_json::from_str::<usize>(&s) {
        return Prefs { theme_index: idx.min(crate::theme::THEMES.len().saturating_sub(1)), transparent_bg: false, footer_preset: 0 };
    }
    Prefs { theme_index: 0, transparent_bg: false, footer_preset: 0 }
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
    Command,
}

#[derive(Debug, Clone)]
pub enum NotificationKind {
    Info,
    #[allow(dead_code)]
    Success,
    Warning,
    #[allow(dead_code)]
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
    pub display_position: f64,
    last_display_position: f64,
    frame_count: u64,
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
    pub volume_input: String,
    pub playlist_cache: Vec<gtm_core::track::Playlist>,
    pub status_message: Option<String>,
    pub notifications: Vec<Notification>,
    pub crossfade_duration: u8,
    pub yt_search_loading: bool,
    pub yt_search_debounce: Option<std::time::Instant>,
    pub pending_volume: Option<u8>,
    pub pending_delete: Option<(i64, String)>,
    pub overlays: OverlayManager,
    pub sleep_timer_remaining: Option<u64>,
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
    pub footer_preset: usize,
    pub footer_title_scroll: usize,
    pub is_ready: bool,
    last_queue_cursor: u128,
    last_track_id_display: Option<i64>,
    prev_tab: Tab,
    prev_track_id: Option<i64>,
    prev_status: gtm_core::state::PlaybackStatus,
    prev_volume: u8,
    prev_cover_id: Option<i64>,
    cover_art_dirty: bool,
    pub suppress_footer_refresh: bool,
    pub cached_footer_spans: Option<(Vec<ratatui::text::Span<'static>>, ratatui::style::Color, ratatui::style::Color)>,
    last_event_time: std::time::Instant,
    pub multiselect_mode: bool,
    pub selected_indices: std::collections::HashSet<usize>,
    pending_motion: Option<char>,
    pub pending_playlist_track_ids: Vec<i64>,
    pub metadata_edit_track_id: Option<i64>,
    pub metadata_fields: [String; 7],
    pub metadata_field_idx: usize,
    pub pending_quit: bool,
    pub np_title_scroll: usize,
    pub track_popup_visible: bool,
    pub track_popup_track_id: Option<i64>,
    pub track_popup_cover: Option<Vec<u8>>,
    pub popup_cover_stateful: Option<StatefulProtocol>,
    last_popup_cover_fetch_id: Option<i64>,
    pub current_lyrics: Option<gtm_core::track::LrcData>,
    pub lyrics_scroll: usize,
    last_lyrics_track_id: Option<i64>,
    pub show_lyrics: bool,
    pub lyrics_manual_scroll: bool,
    pub lyrics_last_scroll_time: std::time::Instant,
}

enum IpcResult {
    RefreshDone(DaemonState, Option<Vec<u8>>, Option<i64>),
    CoverArt(Option<Vec<u8>>, Option<i64>),
    PopupCoverArt(Option<Vec<u8>>, i64),
    Lyrics(Option<gtm_core::track::LrcData>),
    LibraryTracks(Vec<TrackInfo>),
    Playlists(Vec<Playlist>),
    Queue(Vec<TrackInfo>, usize),
    YtResults(Vec<YTSearchResult>),
    Notification(String, NotificationKind),
    Error(String),
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
    SetCrossfadeEasing(gtm_core::state::Easing),
    QueueAdd(String),
    QueueRemove(u128),
    QueueMove(u128, u128),
    QueueClear,
    YtSearch(String),
    YtDownload(String),
    YtResolve(String),
    SetEqPreset(gtm_core::state::EqPreset),
    Search(String),
    AddFavourite(i64),
    RemoveFavourite(i64),
    Refresh,
    RefreshLibrary,
    RefreshQueue,
    RefreshPlaylists,
    RefreshYt,
    RemoveTrack(i64),
    RemoveFromPlaylist(i64, i64),
    FetchLyrics,
}

impl App {
    pub async fn new(socket_path: &Path) -> Result<Self, Box<dyn std::error::Error>> {
        let client = DaemonClient::connect(socket_path).await?;
        let state = DaemonState::new();
        let (cmd_tx, cmd_rx) = mpsc::channel(64);
        let (high_pri_cmd_tx, high_pri_cmd_rx) = mpsc::unbounded_channel();
        let (ipc_tx, ipc_rx) = mpsc::unbounded_channel();
        let keybindings = default_keybindings();
        let prefs = load_prefs();
        let initial_cursor = state.queue_cursor;
        Ok(Self {
            theme: (THEMES[prefs.theme_index].builder)(),
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
            volume_input: String::new(),
            playlist_cache: Vec::new(),
            status_message: None,
            notifications: Vec::new(),
            crossfade_duration: 7,
            pending_volume: None,
            pending_delete: None,
            yt_search_loading: false,
            yt_search_debounce: None,
            overlays: OverlayManager::new(),
            sleep_timer_remaining: None,
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
            theme_index: prefs.theme_index,
            list_scroll: 0,
            viewport_items: 20,
            transparent_bg: prefs.transparent_bg,
            last_action_name: None,
            footer_preset: prefs.footer_preset.min(footer::num_presets().saturating_sub(1)),
            footer_title_scroll: 0,
            is_ready: false,
            last_queue_cursor: initial_cursor,
            last_track_id_display: None,
            prev_tab: Tab::Library,
            prev_track_id: None,
            prev_status: gtm_core::state::PlaybackStatus::Stopped,
            prev_volume: 100,
            prev_cover_id: None,
            cover_art_dirty: false,
            suppress_footer_refresh: false,
            cached_footer_spans: None,
            last_event_time: std::time::Instant::now(),
            multiselect_mode: false,
            selected_indices: std::collections::HashSet::new(),
            pending_motion: None,
            pending_playlist_track_ids: Vec::new(),
            metadata_edit_track_id: None,
            metadata_fields: Default::default(),
            metadata_field_idx: 0,
            pending_quit: false,
            np_title_scroll: 0,
            track_popup_visible: false,
            track_popup_track_id: None,
            track_popup_cover: None,
            popup_cover_stateful: None,
            last_popup_cover_fetch_id: None,
            current_lyrics: None,
            lyrics_scroll: 0,
            last_lyrics_track_id: None,
            show_lyrics: false,
            lyrics_manual_scroll: false,
            lyrics_last_scroll_time: std::time::Instant::now(),
        })
    }

    pub fn cmd_tx(&self) -> mpsc::Sender<TuiCommand> {
        self.cmd_tx.clone()
    }

    pub fn send_high(&self, cmd: TuiCommand) {
        let _ = self.high_pri_cmd_tx.send(cmd);
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
        self.fetch_library_tracks().await;
        self.is_ready = true;

        // Initialize cover image picker (blocking terminal query)
        let picker = tokio::task::spawn_blocking(|| {
            Picker::from_query_stdio().ok()
        })
        .await
        .unwrap_or(None);
        self.cover_picker = picker;

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
            for ev in self.client.drain().await {
                if matches!(ev, gtm_core::ipc::DaemonEvent::PlaybackStarted { .. }) {
                    had_track_change = true;
                }
                self.state.apply_event(&ev);
                events_received = true;
            }
            if events_received {
                self.last_event_time = std::time::Instant::now();
            }
            // Re-seed clock from state after track change events so the
            // local position estimate stays in sync with the daemon.
            if had_track_change {
                self.client.seed_clock_from_state(&self.state).await;
            }

            // Force a state refresh if no events received for 8s while playing
            // to prevent stale Now Playing tab.  Increased from 5s to tolerate
            // brief daemon stalls during rapid prev/next.
            if self.state.status == PlaybackStatus::Playing
                && self.last_event_time.elapsed() > Duration::from_secs(8)
            {
                let c = self.client.clone();
                let ipc_tx2 = self.ipc_tx.clone();
                tokio::spawn(async move {
                    if let Ok(state) = c.get_status().await {
                        let _ = ipc_tx2.send(IpcResult::RefreshDone(state, None, None));
                    }
                });
                self.last_event_time = std::time::Instant::now();
            }

            self.last_queue_cursor = self.state.queue_cursor;

            // If track changed, reset display_position to avoid stale EMA from old track.
            let current_tid = self.state.current_track.as_ref().map(|t| t.id);
            if current_tid != self.last_track_id_display {
                self.last_track_id_display = current_tid;
                let raw = self.client.estimated_position().await;
                self.display_position = raw;
                self.last_display_position = raw;
            }

            // If track changed via daemon event, trigger a cover art fetch.
            // Set last_cover_track_id immediately to prevent redundant spawns.
            // Clear stale cover immediately so we don't show old art on the new track.
            if current_tid != self.last_cover_track_id && current_tid.is_some() {
                let tid = current_tid.unwrap();
                self.last_cover_track_id = Some(tid);
                self.current_cover = None;
                self.cover_stateful = None;
                // Skip cover art in Neovim terminal (no image protocol passthrough)
                if !no_image_protocol() {
                    let client2 = self.client.clone();
                    let ipc_tx2 = self.ipc_tx.clone();
                    tokio::spawn(async move {
                        if let Ok(Some(b64)) = client2.get_cover_art(tid).await {
                            if let Ok(bytes) = base64::engine::general_purpose::STANDARD.decode(&b64) {
                                let _ = ipc_tx2.send(IpcResult::CoverArt(
                                    Some(bytes), Some(tid)
                                ));
                            }
                        }
                    });
                }
                // Auto-fetch lyrics if lyrics pane is visible
                if self.show_lyrics {
                    let client3 = self.client.clone();
                    let ipc_tx3 = self.ipc_tx.clone();
                    self.current_lyrics = None;
                    self.lyrics_scroll = 0;
                    tokio::spawn(async move {
                        if let Ok(lyrics) = client3.get_lyrics(tid).await {
                            let _ = ipc_tx3.send(IpcResult::Lyrics(lyrics));
                        }
                    });
                }
            }

            while let Ok(result) = self.ipc_rx.try_recv() {
                match result {
                    IpcResult::RefreshDone(state, cover, cover_tid) => {
                        // Only apply if the new state is at least as recent as ours.
                        // A stale get_status() response can arrive after a PlaybackStarted
                        // event and overwrite the newer state if we don't guard this.
                        if state.version >= self.state.version {
                            self.state = state;
                            self.client.seed_clock_from_state(&self.state).await;
                            if !no_image_protocol() {
                                if let Some(c) = cover {
                                    self.current_cover = Some(c);
                                    self.last_cover_track_id = cover_tid;
                                } else {
                                    self.current_cover = None;
                                    self.last_cover_track_id = cover_tid;
                                }
                                self.sync_cover_stateful();
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
                    IpcResult::Queue(tracks, cursor) => {
                        self.queue_cache = tracks;
                        self.queue_cursor = cursor;
                    }
                    IpcResult::YtResults(results) => {
                        // Interleave: 1 playlist for every 3 tracks
                        self.yt_results_cache = Self::interleave_yt_results(results);
                        self.yt_search_loading = false;
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
                    IpcResult::Lyrics(lyrics) => {
                        self.current_lyrics = lyrics;
                        self.lyrics_scroll = 0;
                    }
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
                    if let Some(top) = self.overlays.top() {
                        if top.id == OverlayId::YTSearch && !top.query.is_empty() {
                            let q = top.query.clone();
                            let tx = self.cmd_tx();
                            let _ = tx.send(TuiCommand::YtSearch(q)).await;
                        }
                    }
                }
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

            // Auto-scroll lyrics to current playback position
            if !self.lyrics_manual_scroll
                && self.show_lyrics
                && self.state.status == PlaybackStatus::Playing
            {
                if let Some(ref lyrics) = self.current_lyrics {
                    if !lyrics.lines.is_empty() {
                        let pos = self.display_position;
                        let mut current_idx = 0;
                        for (i, line) in lyrics.lines.iter().enumerate() {
                            if line.timestamp <= pos {
                                current_idx = i;
                            } else {
                                break;
                            }
                        }
                        self.lyrics_scroll = current_idx;
                    }
                }
            }

            // Dirty-render: skip redraw if position hasn't changed meaningfully
            // to reduce CPU usage.  Always render every 10th frame as a safety net.
            let frame_count = self.frame_count.wrapping_add(1);
            self.frame_count = frame_count;
            let pos_changed = (self.display_position - self.last_display_position).abs() >= 0.1;
            // Advance title scroll animations (every 3rd frame)
            if frame_count % 3 == 0 {
                self.footer_title_scroll = self.footer_title_scroll.wrapping_add(1);
                self.np_title_scroll = self.np_title_scroll.wrapping_add(1);
            }

            let force_render = pos_changed
                || !self.notifications.is_empty()
                || frame_count % 10 == 0
                || self.cover_art_dirty;
            self.cover_art_dirty = false;
            self.last_display_position = self.display_position;

            if force_render {
                let (cols, rows) = crossterm::terminal::size().unwrap_or((80, 24));
                self.terminal_cols = cols;
                self.terminal_rows = rows;
                if cols < 40 || rows < 10 {
                    let _ = terminal.draw(|f| {
                        let msg = Paragraph::new("Terminal too small (min 40x10)")
                            .alignment(Alignment::Center);
                        f.render_widget(msg, f.area());
                    });
                } else if terminal.draw(|f| ui::render(f, &mut self)).is_ok() {
                    self.suppress_footer_refresh = false;
                }
            }

            if event::poll(Duration::from_millis(16)).unwrap_or(false) {
                if let Ok(Event::Key(key)) = event::read() {
                    if key.kind == KeyEventKind::Press {
                        if !self.handle_key(key).await || self.pending_quit {
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

    /// Update the track popup to show the currently selected track in the library list.
    pub fn update_track_popup(&mut self) {
        let maybe_track_id = {
            let filtered = self.filtered_tracks();
            if self.scroll_offset < filtered.len() {
                Some(filtered[self.scroll_offset].id)
            } else {
                None
            }
        };

        if let Some(tid) = maybe_track_id {
            self.track_popup_track_id = Some(tid);
            self.track_popup_visible = true;

            let current_tid = self.state.current_track.as_ref().map(|t| t.id);
            if current_tid == Some(tid) {
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
            // Spotify: tracks from the spotify audio subdirectory
            tracks.retain(|t| t.path.contains("/audio/spotify") || t.path.contains("\\audio\\spotify"));
        } else if self.library_category == 6 {
            // YouTube: tracks from the youtube audio subdirectory
            tracks.retain(|t| t.path.contains("/audio/youtube") || t.path.contains("\\audio\\youtube"));
        }
        tracks
    }

    /// Unique album names with track counts, sorted by album.
    pub fn unique_albums(&self) -> Vec<(String, usize)> {
        let mut albums: std::collections::BTreeMap<String, usize> = std::collections::BTreeMap::new();
        for t in &self.tracks_cache {
            let key = if t.album.is_empty() { "Unknown Album".into() } else { t.album.clone() };
            *albums.entry(key).or_insert(0) += 1;
        }
        albums.into_iter().collect()
    }

    /// Unique artist names with track counts, sorted by artist.
    pub fn unique_artists(&self) -> Vec<(String, usize)> {
        let mut artists: std::collections::BTreeMap<String, usize> = std::collections::BTreeMap::new();
        for t in &self.tracks_cache {
            let key = if t.artist.is_empty() { "Unknown Artist".into() } else { t.artist.clone() };
            *artists.entry(key).or_insert(0) += 1;
        }
        artists.into_iter().collect()
    }

    async fn fetch_state(&mut self) {
        if let Ok(state) = self.client.get_status().await {
            self.client.seed_clock_from_state(&state).await;
            self.state = state;
            // Fetch cover art if current track changed
            let track_id = self.state.current_track.as_ref().map(|t| t.id);
            if track_id != self.last_cover_track_id {
                if let Some(tid) = track_id {
                    match self.client.get_cover_art(tid).await {
                        Ok(Some(b64)) => {
                            if let Ok(bytes) = base64::engine::general_purpose::STANDARD.decode(&b64) {
                                self.current_cover = Some(bytes);
                                self.last_cover_track_id = track_id;
                            }
                        }
                        _ => {
                            self.current_cover = None;
                            self.cover_stateful = None;
                            self.last_cover_track_id = None;
                        }
                    }
                } else {
                    self.current_cover = None;
                    self.cover_stateful = None;
                    self.last_cover_track_id = track_id;
                }
                self.sync_cover_stateful();
            }
        }
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
            0 => 2,  // Audio: Volume, Mute
            1 => 8,  // YouTube: (all display-only for now)
            2 => 4,  // Playback: Repeat, Shuffle, Crossfade, Easing
            3 => 4,  // System: Theme, Transparent BG, Sync Covers, Footer Preset
            4 => 1,  // Spotify: Status
            _ => 0,
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

    async fn fetch_library_tracks(&mut self) {
        for attempt in 0..3 {
            match self.client.library_get_tracks(None, None).await {
                Ok(DaemonRes::Tracks { tracks, .. }) if !tracks.is_empty() => {
                    self.tracks_cache = tracks;
                    return;
                }
                Ok(DaemonRes::Tracks { .. }) if attempt < 2 => {
                    tokio::time::sleep(Duration::from_millis(500)).await;
                }
                Ok(DaemonRes::Tracks { tracks, .. }) => {
                    // Last attempt — accept even empty tracks
                    self.tracks_cache = tracks;
                }
                Ok(_) | Err(_) if attempt < 2 => {
                    tokio::time::sleep(Duration::from_millis(500)).await;
                }
                Ok(_) | Err(_) => {
                    self.tracks_cache.clear();
                }
            }
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
                    if let Err(e) = client.play(&path, 0.0).await { error_handler(e); }
                });
            }
            TuiCommand::PlayPause => {
                tokio::spawn(async move {
                    if let Err(e) = client.play_pause().await { error_handler(e); }
                });
            }
            TuiCommand::Pause => {
                tokio::spawn(async move {
                    if let Err(e) = client.pause().await { error_handler(e); }
                });
            }
            TuiCommand::Stop => {
                tokio::spawn(async move {
                    if let Err(e) = client.stop().await { error_handler(e); }
                });
            }
            TuiCommand::Next => {
                tokio::spawn(async move {
                    if let Err(e) = client.next().await { error_handler(e); }
                });
            }
            TuiCommand::Prev => {
                tokio::spawn(async move {
                    if let Err(e) = client.prev().await { error_handler(e); }
                });
            }
            TuiCommand::Seek(pos) => {
                tokio::spawn(async move {
                    if let Err(e) = client.seek(pos).await { error_handler(e); }
                });
            }
            TuiCommand::SetVolume(v) => {
                tokio::spawn(async move {
                    if let Err(e) = client.set_volume(v).await { error_handler(e); }
                });
            }
            TuiCommand::ToggleShuffle => {
                tokio::spawn(async move {
                    if let Err(e) = client.toggle_shuffle().await { error_handler(e); }
                });
            }
            TuiCommand::CycleRepeat(m) => {
                tokio::spawn(async move {
                    if let Err(e) = client.cycle_repeat(m).await { error_handler(e); }
                });
            }
            TuiCommand::ToggleMute => {
                tokio::spawn(async move {
                    if let Err(e) = client.toggle_mute().await { error_handler(e); }
                });
            }
            TuiCommand::Crossfade(en, dur) => {
                tokio::spawn(async move {
                    if let Err(e) = client.crossfade(en, dur).await { error_handler(e); }
                });
            }
            TuiCommand::SetCrossfadeEasing(easing) => {
                tokio::spawn(async move {
                    if let Err(e) = client.set_crossfade_easing(easing).await { error_handler(e); }
                });
            }
            TuiCommand::QueueAdd(p) => {
                tokio::spawn(async move {
                    if let Err(e) = client.queue_add(&p, None).await { error_handler2(e); }
                });
            }
            TuiCommand::QueueRemove(i) => {
                tokio::spawn(async move {
                    if let Err(e) = client.queue_rm(i).await { error_handler(e); }
                });
            }
            TuiCommand::QueueMove(from, to) => {
                tokio::spawn(async move {
                    if let Err(e) = client.queue_move(from, to).await { error_handler(e); }
                });
            }
            TuiCommand::QueueClear => {
                tokio::spawn(async move {
                    if let Err(e) = client.queue_clear().await { error_handler(e); }
                });
            }
            TuiCommand::YtSearch(q) => {
                self.yt_search_loading = true;
                let c = client.clone();
                tokio::spawn(async move {
                    if let Err(e) = c.yt_search(&q, None).await { error_handler2(e); }
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
                        .arg("-o")
                        .arg(template.to_string_lossy().as_ref())
                        .arg(&url)
                        .output()
                        .await;
                    let msg = match output {
                        Ok(o) if o.status.success() => {
                            let _ = client2.library_scan(
                                audio_dir.to_string_lossy().as_ref(),
                            ).await;
                            // Also refresh the track cache
                            if let Ok(DaemonRes::Tracks { tracks, .. }) = client2.library_get_tracks(None, None).await {
                                let _ = ipc.send(IpcResult::LibraryTracks(tracks));
                            }
                            // Extract filename from yt-dlp output for a nicer notification
                            let filename = String::from_utf8_lossy(&o.stdout)
                                .lines().next().unwrap_or(&url).to_string();
                            format!("Downloaded: {}", filename)
                        }
                        Ok(o) => format!("Download failed: {}", String::from_utf8_lossy(&o.stderr).lines().last().unwrap_or("unknown error")),
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
                    if let Err(e) = client.yt_resolve_stream(&u).await { error_handler2(e); }
                });
            }
            TuiCommand::SetEqPreset(preset) => {
                tokio::spawn(async move {
                    if let Err(e) = client.set_eq_preset(preset).await { error_handler(e); }
                });
            }
            TuiCommand::Search(q) => {
                tokio::spawn(async move {
                    if let Err(e) = client.search(&q).await { error_handler2(e); }
                });
            }
            TuiCommand::AddFavourite(id) => {
                tokio::spawn(async move {
                    if let Err(e) = client.add_favourite(id).await { error_handler(e); }
                });
            }
            TuiCommand::RemoveFavourite(id) => {
                tokio::spawn(async move {
                    if let Err(e) = client.remove_favourite(id).await { error_handler(e); }
                });
            }
            TuiCommand::Refresh => {
                // drain events on main thread (fast)
                let client2 = self.client.clone();
                let ipc_tx2 = self.ipc_tx.clone();
                tokio::spawn(async move {
                    if let Ok(state) = client2.get_status().await {
                        let mut cover = None;
                        let mut cover_tid = None;
                        let track_id = state.current_track.as_ref().map(|t| t.id);
                        // Skip cover art in Neovim terminal
                        if !no_image_protocol() {
                            if let Some(tid) = track_id {
                                if let Ok(Some(b64)) = client2.get_cover_art(tid).await {
                                    if let Ok(bytes) = base64::engine::general_purpose::STANDARD.decode(&b64) {
                                        cover = Some(bytes);
                                        cover_tid = Some(tid);
                                    }
                                }
                            }
                        }
                        let _ = ipc_tx2.send(IpcResult::RefreshDone(state, cover, cover_tid.or(track_id)));
                    }
                });
                // Auto-poll YT search results while overlay is active
                let ipc_tx3 = self.ipc_tx.clone();
                tokio::spawn(async move {
                    if let Ok(DaemonRes::YtSearchResults { results, .. }) = client.yt_search_poll().await {
                        if !results.is_empty() {
                            let _ = ipc_tx3.send(IpcResult::YtResults(results));
                        }
                    }
                });
            }
            TuiCommand::RefreshPlaylists => {
                tokio::spawn(async move {
                    if let Ok(DaemonRes::Playlists { playlists, .. }) = client.library_get_playlists().await {
                        let _ = ipc_tx.send(IpcResult::Playlists(playlists));
                    }
                });
            }
            TuiCommand::RefreshLibrary => {
                tokio::spawn(async move {
                    if let Ok(DaemonRes::Tracks { tracks, .. }) = client.library_get_tracks(None, None).await {
                        let _ = ipc_tx.send(IpcResult::LibraryTracks(tracks));
                    }
                });
            }
            TuiCommand::RefreshQueue => {
                tokio::spawn(async move {
                    if let Ok(DaemonRes::QueueState { tracks, cursor, .. }) = client.queue_list().await {
                        let _ = ipc_tx.send(IpcResult::Queue(tracks, cursor as usize));
                    }
                });
            }
            TuiCommand::RefreshYt => {
                tokio::spawn(async move {
                    if let Ok(DaemonRes::YtSearchResults { results, .. }) = client.yt_search_poll().await {
                        if !results.is_empty() {
                            let _ = ipc_tx.send(IpcResult::YtResults(results));
                        }
                    }
                });
            }
            TuiCommand::RemoveTrack(track_id) => {
                tokio::spawn(async move {
                    if let Err(e) = client.library_remove_track(track_id).await {
                        error_handler(e);
                    } else {
                        let _ = ipc_tx.send(IpcResult::Notification("Track deleted".to_string(), NotificationKind::Success));
                        if let Ok(DaemonRes::Tracks { tracks, .. }) = client.library_get_tracks(None, None).await {
                            let _ = ipc_tx.send(IpcResult::LibraryTracks(tracks));
                        }
                    }
                });
            }
            TuiCommand::RemoveFromPlaylist(playlist_id, track_id) => {
                tokio::spawn(async move {
                    if let Err(e) = client.library_remove_from_playlist(playlist_id, track_id).await {
                        error_handler(e);
                    } else {
                        let _ = ipc_tx.send(IpcResult::Notification("Removed from playlist".to_string(), NotificationKind::Success));
                        if let Ok(DaemonRes::Playlists { playlists, .. }) = client.library_get_playlists().await {
                            let _ = ipc_tx.send(IpcResult::Playlists(playlists));
                        }
                    }
                });
            }
            TuiCommand::FetchLyrics => {
                let track_id = self.state.current_track.as_ref().map(|t| t.id).unwrap_or(0);
                if track_id == 0 { return; }
                let client2 = self.client.clone();
                let ipc_tx2 = self.ipc_tx.clone();
                tokio::spawn(async move {
                    if let Ok(lyrics) = client2.get_lyrics(track_id).await {
                        let _ = ipc_tx2.send(IpcResult::Lyrics(lyrics));
                    }
                });
            }
        };
    }

    fn set_last_action(&mut self, name: &str) {
        self.last_action_name = Some((name.to_string(), std::time::Instant::now() + std::time::Duration::from_secs(3)));
    }

    fn clamp_overlay_selection(&mut self) {
        if let Some(top) = self.overlays.top_mut() {
            let max = match top.id {
                OverlayId::Queue => self.queue_cache.len().saturating_sub(1),
                OverlayId::YTSearch => self.yt_results_cache.len().saturating_sub(1),
                OverlayId::SearchLibrary => {
                    let q = top.query.to_lowercase();
                    if q.is_empty() {
                        self.tracks_cache.len()
                    } else {
                        self.tracks_cache.iter().filter(|t| {
                            t.title.to_lowercase().contains(&q)
                                || t.artist.to_lowercase().contains(&q)
                        }).count()
                    }.saturating_sub(1)
                }
                OverlayId::Equalizer => 12,
                OverlayId::SleepTimer => 4,
                OverlayId::ThemePicker => THEMES.len().saturating_sub(1),
                OverlayId::CommandPalette => {
                    let commands = crate::ui::COMMAND_PALETTE_COMMANDS;
                    let q = top.query.to_lowercase();
                    if q.is_empty() {
                        commands.len()
                    } else {
                        commands.iter().filter(|c| {
                            let lower = c.0.to_lowercase();
                            let mut qi = 0usize;
                            for ch in lower.chars() {
                                if qi < q.len() && ch == q.as_bytes()[qi] as char {
                                    qi += 1;
                                }
                            }
                            qi == q.len()
                        }).count()
                    }.saturating_sub(1)
                }
                _ => usize::MAX,
            };
            top.selected = top.selected.min(max);
        }
    }

    async fn handle_key(&mut self, key: event::KeyEvent) -> bool {
        // Reset pending_motion if the key is not 'g'
        if key.code != KeyCode::Char('g') {
            self.pending_motion = None;
        }
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
                    if cmd == "quit" || cmd == "q" {
                        return false;
                    }
                    if let Ok(vol) = cmd.parse::<u8>() {
                        if vol > 85 {
                            self.pending_volume = Some(vol);
                        } else {
                            self.send_high(TuiCommand::SetVolume(vol));
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
                // If a volume safety prompt is pending, intercept Enter/Esc
                if self.pending_volume.is_some() {
                    match key.code {
                        KeyCode::Enter => {
                            if let Some(v) = self.pending_volume.take() {
                                self.send_high(TuiCommand::SetVolume(v));
                                self.notify(format!("Volume: {}%", v), NotificationKind::Info);
                            }
                        }
                        KeyCode::Esc => {
                            self.pending_volume = None;
                        }
                        _ => {}
                    }
                    return true;
                }
                // If a delete confirmation is pending, intercept Enter/Esc
                if self.pending_delete.is_some() {
                    match key.code {
                        KeyCode::Enter => {
                            if let Some((track_id, track_name)) = self.pending_delete.take() {
                                let tx = self.cmd_tx();
                                let _ = tx.send(TuiCommand::RemoveTrack(track_id)).await;
                                self.notify(format!("Deleted: {track_name}"), NotificationKind::Success);
                            }
                        }
                        KeyCode::Esc => {
                            self.pending_delete = None;
                        }
                        _ => {}
                    }
                    return true;
                }
                // Handle gg (vim-style double-press) for jump to start
                if key.code == KeyCode::Char('g') {
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
                    Some(KeyboardAction::Quit) => return false,
                    Some(KeyboardAction::QuitDaemon) => {
                        let c = self.client.clone();
                        tokio::spawn(async move { let _ = c.quit().await; });
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
                    Some(KeyboardAction::SwitchTab(tab)) => {
                        self.current_tab = tab;
                        self.suppress_footer_refresh = true;
                        self.dismiss_track_popup();
                        self.refresh_tab().await;
                    }
                    Some(KeyboardAction::OpenOverlay(id)) => {
                        self.overlays.open(id);
                        self.dismiss_track_popup();
                    }
                    Some(KeyboardAction::ToggleHelp) => {
                        self.overlays.open(OverlayId::Help);
                        self.dismiss_track_popup();
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
                                    let idx =
                                        self.queue_cursor.min(self.queue_cache.len() - 1);
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
                        if new_vol > 85 {
                            self.pending_volume = Some(new_vol);
                        } else {
                            self.send_high(TuiCommand::SetVolume(new_vol));
                            self.notify(format!("Volume: {}%", new_vol), NotificationKind::Info);
                        }
                    }
                    Some(KeyboardAction::VolumeDown) => {
                        self.set_last_action("Volume Down");
                        let new_vol = self.state.volume.saturating_sub(5);
                        self.send_high(TuiCommand::SetVolume(new_vol));
                        self.notify(format!("Volume: {}%", new_vol), NotificationKind::Info);
                    }
                    Some(KeyboardAction::SeekForward) => {
                        self.set_last_action("Seek Forward");
                        let pos = self.display_position + 5.0;
                        self.send_high(TuiCommand::Seek(pos));
                    }
                    Some(KeyboardAction::SeekBackward) => {
                        self.set_last_action("Seek Backward");
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
                        self.footer_preset = (self.footer_preset + 1) % footer::num_presets();
                        let name = footer::presets()[self.footer_preset].name;
                        self.set_last_action(&format!("Footer: {}", name));
                        save_prefs(&Prefs {
                            theme_index: self.theme_index,
                            transparent_bg: self.transparent_bg,
                            footer_preset: self.footer_preset,
                        });
                    }
                    Some(KeyboardAction::FocusLeft) => {
                        match self.current_tab {
                            Tab::Library => self.library_pane_focus = true,
                            Tab::Settings => self.settings_pane_focus = true,
                        }
                    }
                    Some(KeyboardAction::FocusRight) => {
                        match self.current_tab {
                            Tab::Library => self.library_pane_focus = false,
                            Tab::Settings => self.settings_pane_focus = false,
                        }
                    }
                    Some(KeyboardAction::FetchLyrics) => {
                        self.show_lyrics = !self.show_lyrics;
                        if self.show_lyrics && self.current_lyrics.is_none() {
                            self.send_high(TuiCommand::FetchLyrics);
                        }
                        self.dismiss_track_popup();
                    }
                    Some(KeyboardAction::EnterFilter) => {
                        self.input_mode = InputMode::Searching;
                        self.dismiss_track_popup();
                    }
                    Some(KeyboardAction::EnterCommand) => {
                        self.input_mode = InputMode::Command;
                        self.dismiss_track_popup();
                    }
                    Some(KeyboardAction::MoveUp) => {
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
                                self.settings_category = self.settings_category.saturating_sub(1);
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
                    Some(KeyboardAction::MoveDown) => {
                        match self.current_tab {
                            Tab::Library if self.library_pane_focus => {
                                let new_cat = (self.library_category + 1).min(LIBRARY_CATEGORIES.len() - 1);
                                if new_cat != self.library_category {
                                    self.browse_detail = None;
                                    self.scroll_offset = 0;
                                }
                                self.library_category = new_cat;
                            }
                            Tab::Settings if self.settings_pane_focus => {
                                self.settings_category = (self.settings_category + 1).min(NUM_SETTINGS_CATEGORIES.saturating_sub(1));
                            }
                            Tab::Settings => {
                                let max = self.settings_options_for_category().saturating_sub(1);
                                self.settings_option = (self.settings_option + 1).min(max);
                            }
                            Tab::Library => {
                                let max_list = self.filtered_tracks().len();
                                self.scroll_offset = (self.scroll_offset + 1).min(max_list.saturating_sub(1));
                                self.update_track_popup();
                            }
                        }
                    }
                    Some(KeyboardAction::Select) => {
                        if self.current_tab == Tab::Library {
                            if self.library_pane_focus {
                                self.library_pane_focus = false;
                            } else if self.browse_detail.is_some() {
                                // In detail view: play the selected track
                                let filtered = self.filtered_tracks();
                                if self.scroll_offset < filtered.len() {
                                    let idx = self.scroll_offset;
                                    let paths: Vec<String> = filtered.iter().map(|t| t.path.clone()).collect();
                                    let path = paths[idx].clone();
                                    let c = self.client.clone();
                                    tokio::spawn(async move {
                                        let _ = c.queue_set(paths, idx as u128).await;
                                        let _ = c.play(&path, 0.0).await;
                                    });
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
                                    self.browse_detail = Some(artists[self.scroll_offset].0.clone());
                                    self.scroll_offset = 0;
                                }
                            } else if self.library_category == 4 {
                                // Playlists: select playlist → show its tracks
                                if self.scroll_offset < self.playlist_cache.len() {
                                    self.browse_detail = Some(self.playlist_cache[self.scroll_offset].name.clone());
                                    self.scroll_offset = 0;
                                }
                            } else {
                                // Default: play track from flat list
                                let filtered = self.filtered_tracks();
                                if self.scroll_offset < filtered.len() {
                                    let idx = self.scroll_offset;
                                    let paths: Vec<String> = filtered.iter().map(|t| t.path.clone()).collect();
                                    let path = paths[idx].clone();
                                    let c = self.client.clone();
                                    tokio::spawn(async move {
                                        let _ = c.queue_set(paths, idx as u128).await;
                                        let _ = c.play(&path, 0.0).await;
                                    });
                                }
                            }
                        } else if self.current_tab == Tab::Settings && !self.settings_pane_focus {
                            let tx = self.cmd_tx();
                            let opt = self.settings_option;
                            match self.settings_category {
                                0 => match opt {
                                    1 => { // Mute toggle
                                        let muted = !self.state.mute;
                                        self.send_high(TuiCommand::SetVolume(if muted { 0 } else { self.state.volume }));
                                        self.state.mute = muted;
                                    }
                                    _ => {}
                                },
                                2 => match opt {
                                    0 => { // Repeat cycle
                                        let next = match self.state.repeat {
                                            gtm_core::state::RepeatMode::Off => gtm_core::state::RepeatMode::One,
                                            gtm_core::state::RepeatMode::One => gtm_core::state::RepeatMode::All,
                                            gtm_core::state::RepeatMode::All => gtm_core::state::RepeatMode::Off,
                                        };
                                        let c = self.client.clone();
                                        tokio::spawn(async move { let _ = c.cycle_repeat(next).await; });
                                        self.state.repeat = next;
                                    }
                                    1 => { // Shuffle toggle
                                        let c = self.client.clone();
                                        tokio::spawn(async move { let _ = c.toggle_shuffle().await; });
                                        self.state.shuffle = !self.state.shuffle;
                                    }
                                    2 => { // Crossfade toggle
                                        let enabled = !self.state.crossfade.as_ref().map(|c| c.enabled).unwrap_or(false);
                                        let dur = self.state.crossfade.as_ref().map(|c| c.duration_secs).unwrap_or(self.crossfade_duration);
                                        let _ = tx.send(TuiCommand::Crossfade(enabled, dur)).await;
                                    }
                                    3 => { // Easing cycle
                                        let current = self.state.crossfade.as_ref().map(|c| c.easing).unwrap_or(Easing::Linear);
                                        let next = match current {
                                            Easing::Linear => Easing::Smoothstep,
                                            Easing::Smoothstep => Easing::Logarithmic,
                                            Easing::Logarithmic => Easing::SlowFadeInFastFadeOut,
                                            Easing::SlowFadeInFastFadeOut => Easing::FastFadeInSlowFadeOut,
                                            Easing::FastFadeInSlowFadeOut => Easing::Linear,
                                        };
                                        let _ = tx.send(TuiCommand::SetCrossfadeEasing(next)).await;
                                        if let Some(ref mut cf) = self.state.crossfade { cf.easing = next; }
                                    }
                                    4 => { // EQ Enabled toggle
                                        let new_enabled = !self.state.eq_enabled;
                                        self.state.eq_enabled = new_enabled;
                                        let c = self.client.clone();
                                        tokio::spawn(async move { let _ = c.set_eq_enabled(new_enabled).await; });
                                    }
                                    _ => {}
                                },
                                3 => match opt {
                                    1 => { // Transparent BG toggle
                                        self.transparent_bg = !self.transparent_bg;
                                        save_prefs(&Prefs { theme_index: self.theme_index, transparent_bg: self.transparent_bg, footer_preset: self.footer_preset });
                                    }
                                    2 => { // Sync Covers
                                        let ipc_tx = self.ipc_tx.clone();
                                        let c = self.client.clone();
                                        tokio::spawn(async move {
                                            match c.library_sync_covers().await {
                                                Ok(DaemonRes::SyncCoversResult { synced, total, .. }) => {
                                                    let msg = format!("Covers synced: {synced}/{total} tracks");
                                                    let _ = ipc_tx.send(IpcResult::Notification(msg, crate::app::NotificationKind::Info));
                                                }
                                                Ok(DaemonRes::Error { message, .. }) => {
                                                    let _ = ipc_tx.send(IpcResult::Notification(format!("Sync failed: {message}"), crate::app::NotificationKind::Error));
                                                }
                                                Ok(_) => {}
                                                Err(e) => {
                                                    let _ = ipc_tx.send(IpcResult::Error(e.to_string()));
                                                }
                                            }
                                        });
                                    }
                                    3 => { // Footer Preset cycle
                                        self.footer_preset = (self.footer_preset + 1) % footer::num_presets();
                                        let name = footer::presets()[self.footer_preset].name;
                                        save_prefs(&Prefs { theme_index: self.theme_index, transparent_bg: self.transparent_bg, footer_preset: self.footer_preset });
                                        self.notify(format!("Footer: {}", name), NotificationKind::Info);
                                    }
                                    _ => {}
                                },
                                _ => {}
                            }
                        }
                    }
                    Some(KeyboardAction::Delete) => {
                        if self.current_tab == Tab::Library && !self.library_pane_focus {
                            let track_data = self.filtered_tracks().get(self.scroll_offset)
                                .map(|t| (t.id, t.title.clone()));
                            if let Some((track_id, track_name)) = track_data {
                                self.pending_delete = Some((track_id, track_name.clone()));
                                self.notify(
                                    format!("Delete \"{track_name}\"? Enter to confirm, Esc to cancel"),
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
                            let msg = if self.multiselect_mode { "Multiselect ON" } else { "Multiselect OFF" };
                            self.notify(msg, NotificationKind::Info);
                        }
                    }
                    Some(KeyboardAction::ToggleSelectAndAdvance) => {
                        if self.multiselect_mode && self.current_tab == Tab::Library && !self.library_pane_focus {
                            if self.selected_indices.contains(&self.scroll_offset) {
                                self.selected_indices.remove(&self.scroll_offset);
                            } else {
                                self.selected_indices.insert(self.scroll_offset);
                            }
                            let max = self.filtered_tracks().len().saturating_sub(1);
                            self.scroll_offset = (self.scroll_offset + 1).min(max);
    
                        }
                    }
                    Some(KeyboardAction::AddToQueue) => {
                        if self.current_tab == Tab::Library && !self.library_pane_focus {
                            let tracks = self.filtered_tracks();
                            let indices: Vec<usize> = if self.multiselect_mode && !self.selected_indices.is_empty() {
                                self.selected_indices.iter().copied().collect()
                            } else {
                                vec![self.scroll_offset]
                            };
                            let mut added = 0;
                            for idx in indices {
                                if let Some(track) = tracks.get(idx) {
                                    let c = self.client.clone();
                                    let path = track.path.clone();
                                    tokio::spawn(async move { let _ = c.queue_add(&path, None).await; });
                                    added += 1;
                                }
                            }
                            self.fetch_queue().await;
                            self.notify(format!("Added {added} track(s) to queue"), NotificationKind::Info);
                        }
                    }
                    Some(KeyboardAction::AddToPlaylist) => {
                        if self.current_tab == Tab::Library && !self.library_pane_focus {
                            let tracks = self.filtered_tracks();
                            let indices: Vec<i64> = if self.multiselect_mode && !self.selected_indices.is_empty() {
                                self.selected_indices.iter().filter_map(|i| tracks.get(*i).map(|t| t.id)).collect()
                            } else {
                                tracks.get(self.scroll_offset).map(|t| vec![t.id]).unwrap_or_default()
                            };
                            if !indices.is_empty() {
                                self.pending_playlist_track_ids = indices;
                                self.overlays.open(OverlayId::PlaylistSelect);
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
                                    if let Some(pl) = self.playlist_cache.iter().find(|p| self.browse_detail.as_deref() == Some(&p.name)) {
                                        let playlist_id = pl.id;
                                        let tx = self.cmd_tx();
                                        let _ = tx.send(TuiCommand::RemoveFromPlaylist(playlist_id, track_id)).await;
                                        self.notify("Removed from playlist", NotificationKind::Info);
                                    }
                                }
                            } else {
                                self.notify("Remove from list only available in playlist view", NotificationKind::Info);
                            }
                        }
                    }
                    Some(KeyboardAction::JumpToStart) => {
                        if self.current_tab == Tab::Library && !self.library_pane_focus {
                            self.scroll_offset = 0;
    
                        }
                    }
                    Some(KeyboardAction::JumpToEnd) => {
                        if self.current_tab == Tab::Library && !self.library_pane_focus {
                            let max = self.filtered_tracks().len().saturating_sub(1);
                            self.scroll_offset = max;
    
                        }
                    }
                    Some(KeyboardAction::EditMetadata) => {
                        if self.current_tab == Tab::Library && !self.library_pane_focus {
                            let track_data = {
                                let tracks = self.filtered_tracks();
                                tracks.get(self.scroll_offset).map(|t| {
                                    (t.id, t.title.clone(), t.artist.clone(), t.album.clone(),
                                     t.genre.clone(), t.year, t.track_number)
                                })
                            };
                            if let Some((id, title, artist, album, genre, year, track_num)) = track_data {
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
                                self.overlays.open(OverlayId::EditMetadata);
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
                            KeyCode::Char('S') => {
                                // Sync covers for tracks missing cover art
                                let c = self.client.clone();
                                let ipc_tx = self.ipc_tx.clone();
                                self.notify("Syncing covers...", NotificationKind::Info);
                                tokio::spawn(async move {
                                    match c.library_sync_covers().await {
                                        Ok(DaemonRes::SyncCoversResult { synced, total, .. }) => {
                                            let msg = format!("Covers synced: {synced}/{total} tracks");
                                            let _ = ipc_tx.send(IpcResult::Notification(msg, crate::app::NotificationKind::Info));
                                        }
                                        Ok(DaemonRes::Error { message, .. }) => {
                                            let _ = ipc_tx.send(IpcResult::Notification(format!("Sync failed: {message}"), crate::app::NotificationKind::Error));
                                        }
                                        Ok(_) => {}
                                        Err(e) => {
                                            let _ = ipc_tx.send(IpcResult::Error(e.to_string()));
                                        }
                                    }
                                });
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
                self.overlays.close_top();
            }
            // Queue move up/down (Ctrl+K/J) must come before plain k/j
            KeyCode::Char('k') if key.modifiers == KeyModifiers::CONTROL => {
                if let Some(top) = self.overlays.top() {
                    if top.id == OverlayId::Queue && !self.queue_cache.is_empty() {
                        let idx = top.selected.min(self.queue_cache.len() - 1);
                        if idx > 0 {
                            let _ = tx.send(TuiCommand::QueueMove(idx as u128, idx.saturating_sub(1) as u128)).await;
                            self.fetch_queue().await;
                        }
                    }
                }
            }
            KeyCode::Char('j') if key.modifiers == KeyModifiers::CONTROL => {
                if let Some(top) = self.overlays.top() {
                    if top.id == OverlayId::Queue && !self.queue_cache.is_empty() {
                        let idx = top.selected.min(self.queue_cache.len() - 1);
                        if idx < self.queue_cache.len() - 1 {
                            let _ = tx.send(TuiCommand::QueueMove(idx as u128, (idx + 1) as u128)).await;
                            self.fetch_queue().await;
                        }
                    }
                }
            }
            KeyCode::Up | KeyCode::Char('k') => {
                let has_input = matches!(self.overlays.top().map(|o| o.id), Some(OverlayId::YTSearch) | Some(OverlayId::SearchLibrary) | Some(OverlayId::CommandPalette));
                let is_metadata = matches!(self.overlays.top().map(|o| o.id), Some(OverlayId::EditMetadata));
                if is_metadata {
                    if self.metadata_field_idx > 0 {
                        self.metadata_field_idx -= 1;
                    }
                    return;
                }
                if has_input && key.code != KeyCode::Up {
                    // Add 'k' to the query instead of navigating
                    if let Some(top) = self.overlays.top_mut() {
                        top.query.push('k');
                    }
                    return;
                }
                if let Some(top) = self.overlays.top_mut() {
                    top.selected = top.selected.saturating_sub(1);
                    if top.id == OverlayId::ThemePicker {
                        let idx = top.selected.min(THEMES.len().saturating_sub(1));
                        self.theme = (THEMES[idx].builder)();
                        self.theme_index = idx;
                        save_prefs(&Prefs { theme_index: idx, transparent_bg: self.transparent_bg, footer_preset: self.footer_preset });
                    }
                }
                self.clamp_overlay_selection();
                self.apply_eq_on_navigation().await;
            }
            KeyCode::Down | KeyCode::Char('j') => {
                let has_input = matches!(self.overlays.top().map(|o| o.id), Some(OverlayId::YTSearch) | Some(OverlayId::SearchLibrary) | Some(OverlayId::CommandPalette));
                let is_metadata = matches!(self.overlays.top().map(|o| o.id), Some(OverlayId::EditMetadata));
                if is_metadata {
                    if self.metadata_field_idx < 6 {
                        self.metadata_field_idx += 1;
                    }
                    return;
                }
                if has_input && key.code != KeyCode::Down {
                    // Add 'j' to the query instead of navigating
                    if let Some(top) = self.overlays.top_mut() {
                        top.query.push('j');
                    }
                    return;
                }
                if let Some(top) = self.overlays.top_mut() {
                    top.selected += 1;
                    if top.id == OverlayId::ThemePicker {
                        let idx = top.selected.min(THEMES.len().saturating_sub(1));
                        self.theme = (THEMES[idx].builder)();
                        self.theme_index = idx;
                        save_prefs(&Prefs { theme_index: idx, transparent_bg: self.transparent_bg, footer_preset: self.footer_preset });
                    }
                }
                self.clamp_overlay_selection();
                self.apply_eq_on_navigation().await;
            }
            KeyCode::Enter if key.modifiers.contains(KeyModifiers::CONTROL) => {
                if matches!(self.overlays.top().map(|o| o.id), Some(OverlayId::EditMetadata)) {
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
                            let _ = client.library_update_metadata(
                                track_id,
                                Some(title), Some(artist),
                                Some(album), Some(genre),
                                year, track_number,
                            ).await;
                            let _ = ipc_tx.send(IpcResult::Notification("Metadata saved".to_string(), NotificationKind::Success));
                        });
                        self.metadata_edit_track_id = None;
                    }
                    self.overlays.close_top();
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
                                self.send_high(TuiCommand::Play(path));
                            }
                        }
                        OverlayId::YTSearch => {
                            if top.query.is_empty() {
                                // Start search
                            } else if !self.yt_results_cache.is_empty() {
                                let idx = top.selected.min(self.yt_results_cache.len() - 1);
                                if self.yt_results_cache[idx].is_playlist {
                                    // Playlist drill-down: search using the playlist URL
                                    let url = self.yt_results_cache[idx].url.clone();
                                    if let Some(top) = self.overlays.top_mut() {
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
                        OverlayId::SleepTimer => {
                            // Set sleep timer from selected preset
                            let presets = [5u64, 10, 15, 30, 60];
                            let idx = top.selected.min(presets.len() - 1);
                            self.sleep_timer_remaining = Some(presets[idx]);
                            self.overlays.close_top();
                        }
                        OverlayId::CommandPalette => {
                            let commands = crate::ui::COMMAND_PALETTE_COMMANDS;
                            let query = top.query.to_lowercase();
                            let filtered: Vec<&(&str, &str)> = if query.is_empty() {
                                commands.iter().collect()
                            } else {
                                commands.iter().filter(|c| {
                                    let lower = c.0.to_lowercase();
                                    let mut qi = 0usize;
                                    for ch in lower.chars() {
                                        if qi < query.len() && ch == query.as_bytes()[qi] as char {
                                            qi += 1;
                                        }
                                    }
                                    qi == query.len()
                                }).collect()
                            };
                            let idx = top.selected.min(filtered.len().saturating_sub(1));
                            if let Some(cmd) = filtered.get(idx) {
                                let raw = cmd.0.to_lowercase();
                                // Strip leading icon (skip to first ASCII letter)
                                let label = raw.chars().skip_while(|c| !c.is_ascii_alphabetic()).collect::<String>();
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
                                    self.overlays.open(OverlayId::Queue);
                                } else if label.starts_with("youtube") {
                                    self.overlays.open(OverlayId::YTSearch);
                                } else if label.starts_with("search lib") {
                                    self.overlays.open(OverlayId::SearchLibrary);
                                } else if label.starts_with("eq") {
                                    self.overlays.open(OverlayId::Equalizer);
                                } else if label.starts_with("sleeptimer") {
                                    self.overlays.open(OverlayId::SleepTimer);
                                } else if label.starts_with("themepicker") {
                                    self.overlays.open(OverlayId::ThemePicker);
                                } else if label.starts_with("sound fx") {
                                    self.overlays.open(OverlayId::SoundEffects);
                                } else if label.starts_with("about") {
                                    self.overlays.open(OverlayId::About);
                                } else if label.starts_with("search") {
                                    self.overlays.open(OverlayId::SearchLibrary);
                                } else if label.starts_with("spotify") {
                                    self.overlays.open(OverlayId::SpotifySearch);
                                } else if label.starts_with("fetch lyrics") {
                                    self.show_lyrics = true;
                                    self.send_high(TuiCommand::FetchLyrics);
                                }
                            }
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
                            tokio::spawn(async move { let _ = c.set_eq_preset(presets[idx]).await; });
                            self.overlays.close_top();
                        }
                        OverlayId::ThemePicker => {
                            let idx = top.selected.min(THEMES.len().saturating_sub(1));
                            self.theme = (THEMES[idx].builder)();
                            self.theme_index = idx;
                            save_prefs(&Prefs { theme_index: idx, transparent_bg: self.transparent_bg, footer_preset: self.footer_preset });
                            self.overlays.close_top();
                        }
                        OverlayId::SearchLibrary => {
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
                            self.overlays.close_top();
                        }
                        OverlayId::SoundEffects => {
                            let sel = top.selected;
                            match sel {
                                1 => {
                                    // Reverb toggle
                                    let new_enabled = !self.state.reverb.enabled;
                                    let room_size = self.state.reverb.room_size;
                                    self.state.reverb.enabled = new_enabled;
                                    let c = self.client.clone();
                                    tokio::spawn(async move { let _ = c.set_reverb(new_enabled, room_size).await; });
                                    self.overlays.close_top();
                                }
                                _ => {
                                    self.overlays.close_top();
                                }
                            }
                        }
                        OverlayId::EditMetadata => {
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
                                        let _ = client.library_update_metadata(
                                            track_id,
                                            Some(title), Some(artist),
                                            Some(album), Some(genre),
                                            year, track_number,
                                        ).await;
                                        let _ = ipc_tx.send(IpcResult::Notification("Metadata saved".to_string(), NotificationKind::Success));
                                    });
                                    self.metadata_edit_track_id = None;
                                }
                                self.overlays.close_top();
                            }
                        }
                        _ => {}
                    }
                }
            }
            KeyCode::Char('d') if key.modifiers == KeyModifiers::CONTROL => {
                // YT search: download selected result
                let idx = self.overlays.top().map_or(0, |o| o.selected.min(self.yt_results_cache.len().saturating_sub(1)));
                if !self.yt_results_cache.is_empty() {
                    let url = self.yt_results_cache[idx].url.clone();
                    let _ = tx.send(TuiCommand::YtDownload(url)).await;
                }
            }
            KeyCode::Char('a') if key.modifiers == KeyModifiers::CONTROL => {
                // YT search: add selected result to queue
                let idx = self.overlays.top().map_or(0, |o| o.selected.min(self.yt_results_cache.len().saturating_sub(1)));
                if !self.yt_results_cache.is_empty() {
                    let url = self.yt_results_cache[idx].url.clone();
                    if self.yt_results_cache[idx].is_playlist {
                        let _ = tx.send(TuiCommand::YtResolve(url)).await;
                    } else {
                        let _ = tx.send(TuiCommand::QueueAdd(url)).await;
                    }
                }
            }
            KeyCode::Char(c) => {
                if let Some(top) = self.overlays.top_mut() {
                    match top.id {
                        OverlayId::YTSearch | OverlayId::SearchLibrary | OverlayId::CommandPalette => {
                            top.query.push(c);
                            if top.id == OverlayId::YTSearch {
                                if c == ' ' {
                                    self.yt_results_cache.clear();
                                    self.yt_search_loading = false;
                                }
                                self.yt_search_debounce = Some(std::time::Instant::now() + Duration::from_millis(500));
                            }
                        }
                        OverlayId::EditMetadata => {
                            self.metadata_fields[self.metadata_field_idx].push(c);
                        }
                        _ => {}
                    }
                }
            }
            KeyCode::Tab => {
                if let Some(top) = self.overlays.top() {
                    if top.id == OverlayId::EditMetadata {
                        self.metadata_field_idx = (self.metadata_field_idx + 1) % 7;
                    }
                }
            }
            KeyCode::Backspace => {
                if let Some(top) = self.overlays.top_mut() {
                    if top.id == OverlayId::EditMetadata {
                        self.metadata_fields[self.metadata_field_idx].pop();
                    } else {
                        top.query.pop();
                    }
                }
            }
            _ => {}
        }
    }

    async fn refresh_tab(&mut self) {
        let tx = self.cmd_tx();
        if self.current_tab == Tab::Library {
            let _ = tx.send(TuiCommand::RefreshLibrary).await;
            let _ = tx.send(TuiCommand::RefreshPlaylists).await;
        }
    }

    async fn apply_eq_on_navigation(&mut self) {
        if let Some(top) = self.overlays.top() {
            if top.id == OverlayId::Equalizer {
                let presets = [
                    gtm_core::state::EqPreset::Flat,
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
    fn interleave_yt_results(mut results: Vec<gtm_core::track::YTSearchResult>) -> Vec<gtm_core::track::YTSearchResult> {
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
