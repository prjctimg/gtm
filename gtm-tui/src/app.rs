use std::path::Path;
use std::time::Duration;

use crossterm::event::{self, Event, KeyCode, KeyEventKind};
use gtm_core::client::DaemonClient;
use gtm_core::state::{DaemonState, PlaybackStatus, RepeatMode, Tab};
use gtm_core::track::TrackInfo;
use ratatui::Terminal;
use tokio::sync::mpsc;

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
    pub search_query: String,
    pub scroll_offset: usize,
    pub tracks_cache: Vec<TrackInfo>,
    pub queue_cache: Vec<TrackInfo>,
    pub queue_cursor: usize,
    pub yt_results_cache: Vec<gtm_core::track::YTSearchResult>,
    pub volume_input: String,
    pub playlist_cache: Vec<gtm_core::track::Playlist>,
    pub error_message: Option<String>,
    pub status_message: Option<String>,
    pub crossfade_duration: u8,
    pub cmd_rx: mpsc::Receiver<TuiCommand>,
    cmd_tx: mpsc::Sender<TuiCommand>,
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
        })
    }

    pub fn cmd_tx(&self) -> mpsc::Sender<TuiCommand> {
        self.cmd_tx.clone()
    }

    pub async fn run(
        mut self,
        terminal: &mut Terminal<ratatui::backend::CrosstermBackend<std::io::Stdout>>,
    ) -> Result<(), Box<dyn std::error::Error>> {
        // Initial state fetch
        self.fetch_state().await;
        self.fetch_queue().await;

        // Spawn daemon event poller
        let cmd_tx = self.cmd_tx();
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(Duration::from_millis(250)).await;
                let _ = cmd_tx.send(TuiCommand::Refresh).await;
            }
        });

        loop {
            // Process commands
            while let Ok(cmd) = self.cmd_rx.try_recv() {
                self.handle_command(cmd).await;
            }

            // Draw UI
            terminal.draw(|f| ui::render(f, &mut self))?;

            // Handle keyboard input
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
                // Drain any pending events from the client buffer
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
            InputMode::Normal => match key.code {
                KeyCode::Char('q') | KeyCode::Esc => return false,
                KeyCode::Char('1') => self.current_tab = Tab::NowPlaying,
                KeyCode::Char('2') => {
                    self.current_tab = Tab::Library;
                    let tx = self.cmd_tx();
                    let _ = tx.send(TuiCommand::RefreshLibrary).await;
                }
                KeyCode::Char('3') => {
                    self.current_tab = Tab::Queue;
                    let tx = self.cmd_tx();
                    let _ = tx.send(TuiCommand::RefreshQueue).await;
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
                KeyCode::Char(' ') | KeyCode::Char('p') => {
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
                                let idx = self.queue_cursor.min(self.queue_cache.len() - 1);
                                let path = self.queue_cache[idx].path.clone();
                                let _ = tx.send(TuiCommand::Play(path)).await;
                            }
                        }
                    }
                }
                KeyCode::Char('s') => {
                    self.input_mode = InputMode::Searching;
                }
                KeyCode::Char(':') => {
                    self.input_mode = InputMode::Command;
                }
                KeyCode::Char('n') => {
                    let tx = self.cmd_tx();
                    let _ = tx.send(TuiCommand::Next).await;
                }
                KeyCode::Char('b') => {
                    let tx = self.cmd_tx();
                    let _ = tx.send(TuiCommand::Prev).await;
                }
                KeyCode::Char('+') | KeyCode::Right => {
                    let tx = self.cmd_tx();
                    let new_vol = (self.state.volume + 5).min(100);
                    let _ = tx.send(TuiCommand::SetVolume(new_vol)).await;
                }
                KeyCode::Char('-') | KeyCode::Left => {
                    let tx = self.cmd_tx();
                    let new_vol = self.state.volume.saturating_sub(5);
                    let _ = tx.send(TuiCommand::SetVolume(new_vol)).await;
                }
                KeyCode::Char('m') => {
                    let tx = self.cmd_tx();
                    let _ = tx.send(TuiCommand::ToggleMute).await;
                }
                KeyCode::Char('r') => {
                    let tx = self.cmd_tx();
                    let new_mode = match self.state.repeat {
                        RepeatMode::Off => RepeatMode::One,
                        RepeatMode::One => RepeatMode::All,
                        RepeatMode::All => RepeatMode::Off,
                    };
                    let _ = tx.send(TuiCommand::CycleRepeat(new_mode)).await;
                }
                KeyCode::Char('h') => {
                    let tx = self.cmd_tx();
                    let _ = tx.send(TuiCommand::ToggleShuffle).await;
                }
                KeyCode::Up => {
                    self.scroll_offset = self.scroll_offset.saturating_sub(1);
                }
                KeyCode::Down => {
                    self.scroll_offset += 1;
                }
                KeyCode::Delete | KeyCode::Char('d') => {
                    if self.current_tab == Tab::Queue && !self.queue_cache.is_empty() {
                        let idx = self.queue_cursor + self.scroll_offset;
                        if idx < self.queue_cache.len() {
                            let tx = self.cmd_tx();
                            let _ = tx.send(TuiCommand::QueueRemove(idx as u128)).await;
                        }
                    }
                }
                KeyCode::Enter => {
                    if self.current_tab == Tab::Queue && !self.queue_cache.is_empty() {
                        let idx = self.scroll_offset;
                        if idx < self.queue_cache.len() {
                            let tx = self.cmd_tx();
                            let _ = tx
                                .send(TuiCommand::Play(self.queue_cache[idx].path.clone()))
                                .await;
                        }
                    }
                    if self.current_tab == Tab::YouTube && !self.yt_results_cache.is_empty() {
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
                }
                _ => {}
            },
        }
        true
    }
}
