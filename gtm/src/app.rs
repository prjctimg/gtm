//! TUI application state, event loop, and key handling.
//!
//! ```text
//!  ┌──────────────────────────────────────────────────────┐
//!  │  TUI Event Loop (run)                                │
//!  │                                                      │
//!  │  ┌──────────┐    ┌──────────────┐    ┌───────────┐  │
//!  │  │ Drain    │───→│ Render       │───→│ Poll      │  │
//!  │  │ cmd_rx   │    │ (ratatui)    │    │ crossterm │  │
//!  │  │ queue    │    │              │    │ key event │  │
//!  │  └────┬─────┘    └──────────────┘    └─────┬─────┘  │
//!  │       │                                    │        │
//!  │       ▼                                    ▼        │
//!  │  handle_command()                     handle_key()  │
//!  │  ─── via DaemonClient IPC             ─── dispatch  │
//!  │       to gtmd                          keybinding  │
//!  │                                                    │
//!  │  Auto-refresh timer sends Refresh every 250ms      │
//!  └──────────────────────────────────────────────────────┘
//! ```

use std::path::Path;
use std::time::Duration;

use crossterm::event::{self, Event, KeyCode, KeyEventKind};
use gtm_core::client::DaemonClient;
use gtm_core::state::{DaemonState, PlaybackStatus, RepeatMode, Tab};
use gtm_core::track::TrackInfo;
use ratatui::Terminal;
use tokio::sync::mpsc;

use crate::keymap::{default_keybindings, BoundCommand, KeyContext, KeyboardAction};
use crate::ui;

pub enum InputMode {
    Normal,
    Searching,
    Command,
}

pub struct App {
    pub client: DaemonClient,
    pub state: DaemonState,
    pub current_tab: Tab,
    pub input_mode: InputMode,

    // Text input for filter ('/') and command (':') modes
    pub search_query: String,

    // Scrolling state for list views (queue, library, YT)
    pub scroll_offset: usize,

    // Cached data for each tab (fetched lazily from daemon)
    pub tracks_cache: Vec<TrackInfo>,
    pub queue_cache: Vec<TrackInfo>,
    pub queue_cursor: usize,
    pub yt_results_cache: Vec<gtm_core::track::YTSearchResult>,
    pub volume_input: String,
    pub playlist_cache: Vec<gtm_core::track::Playlist>,

    // Status bar messages
    pub error_message: Option<String>,
    pub status_message: Option<String>,
    pub crossfade_duration: u8,

    // Command channel — background timer and key handlers send TuiCommands
    // through cmd_tx, handle_command processes them from cmd_rx.
    pub cmd_rx: mpsc::Receiver<TuiCommand>,
    cmd_tx: mpsc::Sender<TuiCommand>,
    keybindings: crate::keymap::Keybindings,
}

/// Commands sent from key handlers or the auto-refresh timer to the
/// main event loop.  Each variant maps 1:1 to a DaemonClient IPC call
/// or a local cache refresh.
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
            client,
            state,
            current_tab: Tab::NowPlaying,
            input_mode: InputMode::Normal,
            search_query: String::new(),
            scroll_offset: 0,
            tracks_cache: Vec::new(),
            queue_cache: Vec::new(),
            queue_cursor: 0,
            yt_results_cache: Vec::new(),
            volume_input: String::new(),
            playlist_cache: Vec::new(),
            error_message: None,
            status_message: None,
            crossfade_duration: 5,
            cmd_rx,
            cmd_tx,
            keybindings,
        })
    }

    pub fn cmd_tx(&self) -> mpsc::Sender<TuiCommand> {
        self.cmd_tx.clone()
    }

    /// Main TUI event loop:
    ///
    ///   1. Drain any queued TuiCommands → call IPC on daemon
    ///   2. Render current state to screen via ratatui
    ///   3. Poll crossterm for key events → dispatch through keybindings
    ///
    /// A background tokio task sends a Refresh command every 250 ms,
    /// which reads broadcast events from the daemon and updates local state.
    pub async fn run(
        mut self,
        terminal: &mut Terminal<ratatui::backend::CrosstermBackend<std::io::Stdout>>,
    ) -> Result<(), Box<dyn std::error::Error>> {
        // Initial state fetch from daemon
        self.fetch_state().await;
        self.fetch_queue().await;

        // Background timer: refresh state every 250 ms
        let cmd_tx = self.cmd_tx();
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(Duration::from_millis(250)).await;
                let _ = cmd_tx.send(TuiCommand::Refresh).await;
            }
        });

        loop {
            // 1. Process all queued commands (IPC calls + cache refreshes)
            while let Ok(cmd) = self.cmd_rx.try_recv() {
                self.handle_command(cmd).await;
            }

            // 2. Render the UI
            terminal.draw(|f| ui::render(f, &mut self))?;

            // 3. Handle keyboard input
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

    async fn fetch_state(&mut self) {
        if let Ok(state) = self.client.get_status().await {
            self.state = state;
        }
    }

    async fn fetch_queue(&mut self) {
        if let Ok(res) = self.client.queue_list().await {
            if let gtm_core::ipc::DaemonRes::QueueState { tracks, cursor, .. } = res {
                self.queue_cache = tracks;
                self.queue_cursor = cursor as usize;
            }
        }
    }

    async fn handle_command(&mut self, cmd: TuiCommand) {
        let result = match cmd {
            TuiCommand::Play(path) => self.client.play(&path, 0.0).await,
            TuiCommand::PlayPause => self.client.play_pause().await,
            TuiCommand::Pause => self.client.pause().await,
            TuiCommand::Stop => self.client.stop().await,
            TuiCommand::Next => self.client.next().await,
            TuiCommand::Prev => self.client.prev().await,
            TuiCommand::Seek(pos) => self.client.seek(pos).await,
            TuiCommand::SetVolume(v) => self.client.set_volume(v).await,
            TuiCommand::ToggleShuffle => self.client.toggle_shuffle().await,
            TuiCommand::CycleRepeat(m) => self.client.cycle_repeat(m).await,
            TuiCommand::ToggleMute => self.client.toggle_mute().await,
            TuiCommand::Crossfade(en, dur) => self.client.crossfade(en, dur).await,
            TuiCommand::QueueAdd(p) => self.client.queue_add(&p, None).await,
            TuiCommand::QueueRemove(i) => self.client.queue_remove(i).await,
            TuiCommand::QueueClear => self.client.queue_clear().await,
            TuiCommand::YtSearch(q) => self.client.yt_search(&q, None).await.map(|_| 0u32),
            TuiCommand::YtResolve(u) => self.client.yt_resolve_stream(&u).await.map(|_| 0u32),
            TuiCommand::Search(q) => self.client.search(&q).await.map(|_| 0u32),
            TuiCommand::AddFavourite(id) => self.client.add_favourite(id).await,
            TuiCommand::RemoveFavourite(id) => self.client.remove_favourite(id).await,
            TuiCommand::Refresh => {
                let events = self.client.drain_events();
                for ev in &events {
                    self.state.apply_event(ev);
                }
                self.fetch_state().await;
                if self.current_tab == Tab::Queue {
                    self.fetch_queue().await;
                }
                Ok(0u32)
            }
            TuiCommand::RefreshLibrary => {
                if let Ok(res) = self.client.library_get_tracks(None, None).await {
                    if let gtm_core::ipc::DaemonRes::Tracks { tracks, .. } = res {
                        self.tracks_cache = tracks;
                    }
                }
                Ok(0u32)
            }
            TuiCommand::RefreshQueue => {
                self.fetch_queue().await;
                Ok(0u32)
            }
            TuiCommand::RefreshPlaylists => {
                if let Ok(res) = self.client.library_get_playlists().await {
                    if let gtm_core::ipc::DaemonRes::Playlists { playlists, .. } = res {
                        self.playlist_cache = playlists;
                    }
                }
                Ok(0u32)
            }
            TuiCommand::RefreshYt => {
                if let Ok(res) = self.client.yt_search_poll().await {
                    if let gtm_core::ipc::DaemonRes::YtSearchResults { results, .. } = res {
                        self.yt_results_cache = results;
                    }
                }
                Ok(0u32)
            }
        };
        match result {
            Ok(_) => {}
            Err(e) => {
                self.error_message = Some(e.to_string());
            }
        }
    }

    /// Dispatch a key event based on current InputMode:
    ///
    ///   Normal     → keybindings.dispatch() → KeyboardAction
    ///   Searching  → type query, Enter to search, Esc to cancel
    ///   Command    → type command, Enter to execute, Esc to cancel
    ///
    /// Returns false when the app should quit.
    async fn handle_key(&mut self, key: event::KeyEvent) -> bool {
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
                        Tab::YouTube => {
                            let _ = tx.send(TuiCommand::YtSearch(q)).await;
                        }
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
                        let _ = tx.send(TuiCommand::SetVolume(vol)).await;
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
                // Try keybindings dispatch
                match self.keybindings.dispatch(key, KeyContext::Normal) {
                    Some(KeyboardAction::Quit) => return false,
                    Some(KeyboardAction::NextTab) => {
                        self.current_tab = match self.current_tab {
                            Tab::NowPlaying => Tab::Library,
                            Tab::Library => Tab::Queue,
                            Tab::Queue => Tab::YouTube,
                            Tab::YouTube => Tab::Settings,
                            Tab::Settings => Tab::Help,
                            Tab::Help => Tab::NowPlaying,
                        };
                        self.refresh_tab().await;
                    }
                    Some(KeyboardAction::PrevTab) => {
                        self.current_tab = match self.current_tab {
                            Tab::NowPlaying => Tab::Help,
                            Tab::Library => Tab::NowPlaying,
                            Tab::Queue => Tab::Library,
                            Tab::YouTube => Tab::Queue,
                            Tab::Settings => Tab::YouTube,
                            Tab::Help => Tab::Settings,
                        };
                        self.refresh_tab().await;
                    }
                    Some(KeyboardAction::SwitchTab(tab)) => {
                        self.current_tab = tab;
                        self.refresh_tab().await;
                    }
                    // PlayPause toggles based on current state:
                    //   Playing  → Pause
                    //   Paused   → Play (resume current track)
                    //   Stopped  → Play from queue (first item at cursor)
                    Some(KeyboardAction::PlayPause) => {
                        let tx = self.cmd_tx();
                        match self.state.status {
                            PlaybackStatus::Playing => {
                                let _ = tx.send(TuiCommand::Pause).await;
                            }
                            PlaybackStatus::Paused => {
                                if let Some(ref t) = self.state.current_track {
                                    let _ = tx.send(TuiCommand::Play(t.path.clone())).await;
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
                        let tx = self.cmd_tx();
                        let new_vol = (self.state.volume + 5).min(100);
                        let _ = tx.send(TuiCommand::SetVolume(new_vol)).await;
                    }
                    Some(KeyboardAction::VolumeDown) | Some(KeyboardAction::SeekBackward) => {
                        let tx = self.cmd_tx();
                        let new_vol = self.state.volume.saturating_sub(5);
                        let _ = tx.send(TuiCommand::SetVolume(new_vol)).await;
                    }
                    Some(KeyboardAction::ToggleMute) => {
                        let tx = self.cmd_tx();
                        let _ = tx.send(TuiCommand::ToggleMute).await;
                    }
                    Some(KeyboardAction::CycleRepeat) => {
                        let tx = self.cmd_tx();
                        let new_mode = match self.state.repeat {
                            RepeatMode::Off => RepeatMode::One,
                            RepeatMode::One => RepeatMode::All,
                            RepeatMode::All => RepeatMode::Off,
                        };
                        let _ = tx.send(TuiCommand::CycleRepeat(new_mode)).await;
                    }
                    Some(KeyboardAction::ToggleShuffle) => {
                        let tx = self.cmd_tx();
                        let _ = tx.send(TuiCommand::ToggleShuffle).await;
                    }
                    Some(KeyboardAction::EnterFilter) => {
                        self.input_mode = InputMode::Searching;
                    }
                    Some(KeyboardAction::EnterCommand) => {
                        self.input_mode = InputMode::Command;
                    }
                    Some(KeyboardAction::MoveUp) => {
                        self.scroll_offset = self.scroll_offset.saturating_sub(1);
                    }
                    Some(KeyboardAction::MoveDown) => {
                        self.scroll_offset += 1;
                    }
                    Some(KeyboardAction::Select) => {
                        match self.current_tab {
                            Tab::Queue if !self.queue_cache.is_empty() => {
                                let idx = self.scroll_offset;
                                if idx < self.queue_cache.len() {
                                    let tx = self.cmd_tx();
                                    let _ = tx
                                        .send(TuiCommand::Play(
                                            self.queue_cache[idx].path.clone(),
                                        ))
                                        .await;
                                }
                            }
                            Tab::YouTube if !self.yt_results_cache.is_empty() => {
                                let idx = self.scroll_offset;
                                if idx < self.yt_results_cache.len() {
                                    let tx = self.cmd_tx();
                                    let _ = tx
                                        .send(TuiCommand::YtResolve(
                                            self.yt_results_cache[idx].url.clone(),
                                        ))
                                        .await;
                                }
                            }
                            _ => {}
                        }
                    }
                    Some(KeyboardAction::Delete) => {
                        if self.current_tab == Tab::Queue && !self.queue_cache.is_empty() {
                            let idx = self.queue_cursor + self.scroll_offset;
                            if idx < self.queue_cache.len() {
                                let tx = self.cmd_tx();
                                let _ = tx.send(TuiCommand::QueueRemove(idx as u128)).await;
                            }
                        }
                    }
                    None => {
                        // Fallback to direct key handling
                        match key.code {
                            KeyCode::Char('q') | KeyCode::Esc => return false,
                            KeyCode::Char('1') => {
                                self.current_tab = Tab::NowPlaying;
                            }
                            KeyCode::Char('2') => {
                                self.current_tab = Tab::Library;
                                self.refresh_tab().await;
                            }
                            KeyCode::Char('3') => {
                                self.current_tab = Tab::Queue;
                                self.refresh_tab().await;
                            }
                            KeyCode::Char('4') => {
                                self.current_tab = Tab::YouTube;
                            }
                            KeyCode::Char('5') => {
                                self.current_tab = Tab::Settings;
                            }
                            KeyCode::Char('6') => {
                                self.current_tab = Tab::Help;
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

    async fn refresh_tab(&mut self) {
        let tx = self.cmd_tx();
        match self.current_tab {
            Tab::Library => {
                let _ = tx.send(TuiCommand::RefreshLibrary).await;
            }
            Tab::Queue => {
                let _ = tx.send(TuiCommand::RefreshQueue).await;
            }
            _ => {}
        }
    }
}
