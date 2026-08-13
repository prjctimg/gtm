use std::path::Path;
use std::time::Duration;

use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use gtm_core::client::DaemonClient;
use gtm_core::ipc::DaemonRes;
use gtm_core::state::{DaemonState, Easing, PlaybackStatus, RepeatMode, Tab};
use gtm_core::track::{Playlist, TrackInfo, YTSearchResult};
use ratatui::Terminal;
use tokio::sync::mpsc;

use base64::Engine;

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
}

fn load_prefs() -> Prefs {
    let path = prefs_path();
    let s = match std::fs::read_to_string(path) {
        Ok(s) => s,
        Err(_) => return Prefs { theme_index: 0, transparent_bg: false },
    };
    // Try new format first, then fall back to old format (bare usize)
    if let Ok(p) = serde_json::from_str::<Prefs>(&s) {
        return p;
    }
    if let Ok(idx) = serde_json::from_str::<usize>(&s) {
        return Prefs { theme_index: idx.min(crate::theme::THEMES.len().saturating_sub(1)), transparent_bg: false };
    }
    Prefs { theme_index: 0, transparent_bg: false }
}

fn save_prefs(prefs: &Prefs) {
    if let Ok(s) = serde_json::to_string(prefs) {
        let _ = std::fs::write(prefs_path(), s);
    }
}

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
    pub error_message: Option<String>,
    pub status_message: Option<String>,
    pub notifications: Vec<Notification>,
    pub crossfade_duration: u8,
    pub yt_search_loading: bool,
    pub yt_search_debounce: Option<std::time::Instant>,
    pub pending_volume: Option<u8>,
    pub overlays: OverlayManager,
    pub sleep_timer_remaining: Option<u64>,
    pub playback_speed: f64,
    pub current_cover: Option<Vec<u8>>,
    pub last_cover_track_id: Option<i64>,
    pub cmd_rx: mpsc::Receiver<TuiCommand>,
    cmd_tx: mpsc::Sender<TuiCommand>,
    ipc_rx: mpsc::UnboundedReceiver<IpcResult>,
    ipc_tx: mpsc::UnboundedSender<IpcResult>,
    keybindings: crate::keymap::Keybindings,
    pub theme_index: usize,
    pub show_tag_popup: bool,
    pub hover_start: Option<std::time::Instant>,
    pub show_hover_info: bool,
    pub list_scroll: usize,
    pub viewport_items: usize,
    pub transparent_bg: bool,
    last_queue_cursor: u128,
}

enum IpcResult {
    RefreshDone(DaemonState, Option<Vec<u8>>, Option<i64>),
    CoverArt(Option<Vec<u8>>, Option<i64>),
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
            current_tab: Tab::NowPlaying,
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
            error_message: None,
            status_message: None,
            notifications: Vec::new(),
            crossfade_duration: 7,
            pending_volume: None,
            yt_search_loading: false,
            yt_search_debounce: None,
            overlays: OverlayManager::new(),
            sleep_timer_remaining: None,
            playback_speed: 1.0,
            current_cover: None,
            last_cover_track_id: None,
            cmd_rx,
            cmd_tx,
            ipc_rx,
            ipc_tx,
            keybindings,
            theme_index: prefs.theme_index,
            show_tag_popup: false,
            hover_start: None,
            show_hover_info: false,
            list_scroll: 0,
            viewport_items: 20,
            transparent_bg: prefs.transparent_bg,
            last_queue_cursor: initial_cursor,
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

            // Detect crossfade auto-advance: when queue cursor moves forward
            // while a track is playing, show "Up Next" notification.
            if self.state.status == PlaybackStatus::Playing
                && self.state.queue_cursor > self.last_queue_cursor
                && self.state.queue_cursor > 0
            {
                let idx = self.state.queue_cursor as usize;
                if idx < self.queue_cache.len() {
                    let next = &self.queue_cache[idx];
                    self.notify(format!(" \u{25b6} Up Next: {} — {}", next.artist, next.title), NotificationKind::Info);
                }
            }
            self.last_queue_cursor = self.state.queue_cursor;

            // If track changed via daemon event (not from our own Refresh),
            // trigger a cover art fetch so the UI updates immediately.
            let current_tid = self.state.current_track.as_ref().map(|t| t.id);
            if current_tid != self.last_cover_track_id && current_tid.is_some() {
                let client2 = self.client.clone();
                let ipc_tx2 = self.ipc_tx.clone();
                tokio::spawn(async move {
                    if let Some(tid) = current_tid {
                        if let Ok(Some(b64)) = client2.get_cover_art(tid).await {
                            if let Ok(bytes) = base64::engine::general_purpose::STANDARD.decode(&b64) {
                                let _ = ipc_tx2.send(IpcResult::CoverArt(
                                    Some(bytes), Some(tid)
                                ));
                            }
                        }
                    }
                });
            }

            while let Ok(result) = self.ipc_rx.try_recv() {
                match result {
                    IpcResult::RefreshDone(state, cover, cover_tid) => {
                        self.state = state;
                        self.current_cover = cover;
                        self.last_cover_track_id = cover_tid;
                    }
                    IpcResult::CoverArt(cover, cover_tid) => {
                        self.current_cover = cover;
                        self.last_cover_track_id = cover_tid;
                    }
                    IpcResult::LibraryTracks(tracks) => self.tracks_cache = tracks,
                    IpcResult::Playlists(playlists) => self.playlist_cache = playlists,
                    IpcResult::Queue(tracks, cursor) => {
                        self.queue_cache = tracks;
                        self.queue_cursor = cursor;
                    }
                    IpcResult::YtResults(results) => {
                        self.yt_results_cache = results;
                        self.yt_search_loading = false;
                    }
                    IpcResult::Notification(msg, kind) => {
                        self.notifications.push(Notification {
                            message: msg,
                            kind,
                            expires_at: std::time::Instant::now() + Duration::from_secs(5),
                        });
                    }
                    IpcResult::Error(e) => self.error_message = Some(e),
                }
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

            // Hover timer: show info popup after 3s of no key press (Library tab only)
            if self.current_tab == Tab::Library && !self.library_pane_focus && !self.overlays.is_open() && !self.show_tag_popup {
                match self.hover_start {
                    Some(t) if t.elapsed() >= std::time::Duration::from_secs(3) => {
                        self.show_hover_info = true;
                    }
                    None => self.hover_start = Some(now),
                    _ => {}
                }
            } else {
                self.show_hover_info = false;
            }

            let raw_pos = self.client.estimated_position().await;
            // Monotonic guard: prevent large backward jumps from clock skew.
            // Allow at most 0.5s of regression to avoid visible stutter.
            let raw_pos = raw_pos.max(self.display_position - 0.5);
            // EMA smoothing
            self.display_position = self.display_position * 0.85 + raw_pos * 0.15;

            // Dirty-render: skip redraw if position hasn't changed meaningfully
            // to reduce CPU usage.  Always render every 10th frame as a safety net.
            let frame_count = self.frame_count.wrapping_add(1);
            self.frame_count = frame_count;
            let pos_changed = (self.display_position - self.last_display_position).abs() >= 0.1;
            let force_render = pos_changed
                || !self.notifications.is_empty()
                || frame_count % 10 == 0;
            self.last_display_position = self.display_position;

            if force_render {
                terminal.draw(|f| ui::render(f, &mut self))?;
            }

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

    /// Filtered tracks for the current library view, respecting search query and browse_detail.
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
                            // retry on next refresh
                            self.last_cover_track_id = None;
                        }
                    }
                } else {
                    self.current_cover = None;
                    self.last_cover_track_id = track_id;
                }
            }
        }
    }

    fn settings_options_for_category(&self) -> usize {
        match self.settings_category {
            0 => 2,  // Audio: Volume, Mute
            1 => 8,  // YouTube: (all display-only for now)
            2 => 4,  // Playback: Repeat, Shuffle, Crossfade, Easing
            3 => 3,  // System: Theme, Transparent BG, Sync Covers
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
                        .arg("--write-thumbnail")
                        .arg("--convert-thumbnails")
                        .arg("jpg")
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
                            format!("Downloaded: {}", url)
                        }
                        Ok(o) => format!("Download failed: {}", String::from_utf8_lossy(&o.stderr)),
                        Err(e) => format!("Download error: {e}"),
                    };
                    let _ = ipc.send(IpcResult::Notification(msg, crate::app::NotificationKind::Info));
                });
            }
            TuiCommand::YtResolve(u) => {
                tokio::spawn(async move {
                    if let Err(e) = client.yt_resolve_stream(&u).await { error_handler2(e); }
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
                        // Do NOT call seed_clock_from_state here — that resets the
                        // local clock and causes visible position stutter.  The
                        // clock is kept in sync by apply_clock_events during drain().
                        let mut cover = None;
                        let mut cover_tid = None;
                        let track_id = state.current_track.as_ref().map(|t| t.id);
                        if let Some(tid) = track_id {
                            if let Ok(Some(b64)) = client2.get_cover_art(tid).await {
                                if let Ok(bytes) = base64::engine::general_purpose::STANDARD.decode(&b64) {
                                    cover = Some(bytes);
                                    cover_tid = Some(tid);
                                }
                            } else {
                                // Cover not available yet — retry after a short delay
                                let c2 = client2.clone();
                                let ipc3 = ipc_tx2.clone();
                                tokio::spawn(async move {
                                    tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                                    if let Ok(Some(b64)) = c2.get_cover_art(tid).await {
                                        if let Ok(bytes) = base64::engine::general_purpose::STANDARD.decode(&b64) {
                                            let _ = ipc3.send(IpcResult::CoverArt(Some(bytes), Some(tid)));
                                        }
                                    }
                                });
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
        };
    }

    async fn handle_key(&mut self, key: event::KeyEvent) -> bool {
        self.hover_start = None;
        self.show_hover_info = false;
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
                    Some(KeyboardAction::QuitDaemon) => {
                        let _ = self.client.quit().await;
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
                                self.settings_pane_focus = !self.settings_pane_focus;
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
                                let _ = tx.send(TuiCommand::PlayPause).await;
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
                    Some(KeyboardAction::Stop) => {
                        let tx = self.cmd_tx();
                        let _ = tx.send(TuiCommand::Stop).await;
                    }
                    Some(KeyboardAction::VolumeUp) => {
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
                    Some(KeyboardAction::VolumeDown) => {
                        let tx = self.cmd_tx();
                        let new_vol = self.state.volume.saturating_sub(5);
                        let _ = tx.send(TuiCommand::SetVolume(new_vol)).await;
                        self.notify(format!("Volume: {}%", new_vol), NotificationKind::Info);
                    }
                    Some(KeyboardAction::SeekForward) => {
                        let tx = self.cmd_tx();
                        let pos = self.display_position + 5.0;
                        let _ = tx.send(TuiCommand::Seek(pos)).await;
                    }
                    Some(KeyboardAction::SeekBackward) => {
                        let tx = self.cmd_tx();
                        let pos = (self.display_position - 5.0).max(0.0);
                        let _ = tx.send(TuiCommand::Seek(pos)).await;
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
                    Some(KeyboardAction::ToggleFavourite) => {
                        if let Some(ref track) = self.state.current_track {
                            let tx = self.cmd_tx();
                            let _ = tx.send(TuiCommand::AddFavourite(track.id)).await;
                            self.notify("Favourite toggled", NotificationKind::Info);
                        }
                    }
                    Some(KeyboardAction::ClearQueue) => {
                        let tx = self.cmd_tx();
                        let _ = tx.send(TuiCommand::QueueClear).await;
                        self.notify("Queue cleared", NotificationKind::Info);
                    }
                    Some(KeyboardAction::FocusLeft) => {
                        match self.current_tab {
                            Tab::Library => self.library_pane_focus = true,
                            Tab::Settings => self.settings_pane_focus = true,
                            _ => {}
                        }
                    }
                    Some(KeyboardAction::FocusRight) => {
                        match self.current_tab {
                            Tab::Library => self.library_pane_focus = false,
                            Tab::Settings => self.settings_pane_focus = false,
                            _ => {}
                        }
                    }
                    Some(KeyboardAction::EnterFilter) => {
                        self.input_mode = InputMode::Searching;
                    }
                    Some(KeyboardAction::EnterCommand) => {
                        self.input_mode = InputMode::Command;
                    }
                    Some(KeyboardAction::MoveUp) => {
                        match self.current_tab {
                            Tab::Library if self.library_pane_focus => {
                                self.library_category = self.library_category.saturating_sub(1);
                            }
                            Tab::Settings if self.settings_pane_focus => {
                                self.settings_category = self.settings_category.saturating_sub(1);
                            }
                            Tab::Settings => {
                                self.settings_option = self.settings_option.saturating_sub(1);
                            }
                            _ => {
                                self.scroll_offset = self.scroll_offset.saturating_sub(1);
                                // Auto-scroll: keep selection visible
                                if self.scroll_offset < self.list_scroll {
                                    self.list_scroll = self.list_scroll.saturating_sub(1);
                                }
                            }
                        }
                    }
                    Some(KeyboardAction::MoveDown) => {
                        match self.current_tab {
                            Tab::Library if self.library_pane_focus => {
                                self.library_category = (self.library_category + 1).min(LIBRARY_CATEGORIES.len() - 1);
                            }
                            Tab::Settings if self.settings_pane_focus => {
                                self.settings_category = (self.settings_category + 1).min(NUM_SETTINGS_CATEGORIES.saturating_sub(1));
                            }
                            Tab::Settings => {
                                let max = self.settings_options_for_category().saturating_sub(1);
                                self.settings_option = (self.settings_option + 1).min(max);
                            }
                            _ => {
                                self.scroll_offset += 1;
                                // Auto-scroll: keep selection visible
                                let max_list = self.filtered_tracks().len();
                                if self.scroll_offset >= self.list_scroll + self.viewport_items && self.list_scroll + self.viewport_items < max_list {
                                    self.list_scroll += 1;
                                }
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
                                    let tx = self.cmd_tx();
                                    let _ = self.client.queue_set(paths, idx as u128).await;
                                    let _ = tx.send(TuiCommand::Play(path)).await;
                                }
                            } else if self.library_category == 1 {
                                // Albums: select album → show its tracks
                                let albums = self.unique_albums();
                                if self.scroll_offset < albums.len() {
                                    self.browse_detail = Some(albums[self.scroll_offset].0.clone());
                                    self.scroll_offset = 0;
                                }
                            } else if self.library_category == 2 {
                                // Artists: select artist → show its tracks
                                let artists = self.unique_artists();
                                if self.scroll_offset < artists.len() {
                                    self.browse_detail = Some(artists[self.scroll_offset].0.clone());
                                    self.scroll_offset = 0;
                                }
                            } else if self.library_category == 3 {
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
                                    let tx = self.cmd_tx();
                                    let _ = self.client.queue_set(paths, idx as u128).await;
                                    let _ = tx.send(TuiCommand::Play(path)).await;
                                }
                            }
                        } else if self.current_tab == Tab::Settings && !self.settings_pane_focus {
                            let tx = self.cmd_tx();
                            let opt = self.settings_option;
                            match self.settings_category {
                                0 => match opt {
                                    1 => { // Mute toggle
                                        let muted = !self.state.mute;
                                        let _ = tx.send(TuiCommand::SetVolume(if muted { 0 } else { self.state.volume })).await;
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
                                        let _ = self.client.cycle_repeat(next).await;
                                        self.state.repeat = next;
                                    }
                                    1 => { // Shuffle toggle
                                        let _ = self.client.toggle_shuffle().await;
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
                                    _ => {}
                                },
                                3 => match opt {
                                    1 => { // Transparent BG toggle
                                        self.transparent_bg = !self.transparent_bg;
                                        save_prefs(&Prefs { theme_index: self.theme_index, transparent_bg: self.transparent_bg });
                                    }
                                    2 => { // Sync Covers
                                        let ipc_tx = self.ipc_tx.clone();
                                        let c = self.client.clone();
                                        tokio::spawn(async move {
                                            if let Ok(DaemonRes::SyncCoversResult { synced, total, .. }) = c.library_sync_covers().await {
                                                let msg = format!("Covers synced: {synced}/{total} tracks");
                                                let _ = ipc_tx.send(IpcResult::Notification(msg, crate::app::NotificationKind::Info));
                                            }
                                        });
                                    }
                                    _ => {}
                                },
                                _ => {}
                            }
                        }
                    }
                    Some(KeyboardAction::Delete) => {}
                    None => {
                        match key.code {
                            KeyCode::Char('q') | KeyCode::Esc => {
                                if self.show_tag_popup {
                                    self.show_tag_popup = false;
                                } else if self.browse_detail.is_some() {
                                    self.browse_detail = None;
                                    self.scroll_offset = 0;
                                } else {
                                    return false;
                                }
                            }
                            KeyCode::Char('t') => {
                                self.show_tag_popup = !self.show_tag_popup;
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
                // For overlays with input fields, only allow arrow keys (not vim keys)
                let has_input = matches!(self.overlays.top().map(|o| o.id), Some(OverlayId::YTSearch) | Some(OverlayId::SearchLibrary) | Some(OverlayId::CommandPalette));
                if has_input && key.code != KeyCode::Up { return; }
                if let Some(top) = self.overlays.top_mut() {
                    top.selected = top.selected.saturating_sub(1);
                    if top.id == OverlayId::ThemePicker {
                        let idx = top.selected.min(THEMES.len().saturating_sub(1));
                        self.theme = (THEMES[idx].builder)();
                        self.theme_index = idx;
                        save_prefs(&Prefs { theme_index: idx, transparent_bg: self.transparent_bg });
                    }
                }
            }
            KeyCode::Down | KeyCode::Char('j') => {
                let has_input = matches!(self.overlays.top().map(|o| o.id), Some(OverlayId::YTSearch) | Some(OverlayId::SearchLibrary) | Some(OverlayId::CommandPalette));
                if has_input && key.code != KeyCode::Down { return; }
                if let Some(top) = self.overlays.top_mut() {
                    top.selected += 1;
                    if top.id == OverlayId::ThemePicker {
                        let idx = top.selected.min(THEMES.len().saturating_sub(1));
                        self.theme = (THEMES[idx].builder)();
                        self.theme_index = idx;
                        save_prefs(&Prefs { theme_index: idx, transparent_bg: self.transparent_bg });
                    }
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
    let _ = self.client.set_eq_preset(presets[idx]).await;
                            self.overlays.close_top();
                        }
                        OverlayId::ThemePicker => {
                            let idx = top.selected.min(THEMES.len().saturating_sub(1));
                            self.theme = (THEMES[idx].builder)();
                            self.theme_index = idx;
                            save_prefs(&Prefs { theme_index: idx, transparent_bg: self.transparent_bg });
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
                        OverlayId::YTSearch | OverlayId::SearchLibrary => {
                            top.query.push(c);
                            if top.id == OverlayId::YTSearch {
                                self.yt_search_debounce = Some(std::time::Instant::now() + Duration::from_millis(500));
                            }
                        }
                        _ => {}
                    }
                }
            }
            KeyCode::Backspace => {
                if let Some(top) = self.overlays.top_mut() {
                    top.query.pop();
                    if top.id == OverlayId::YTSearch {
                        self.yt_search_debounce = Some(std::time::Instant::now() + Duration::from_millis(500));
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
}
