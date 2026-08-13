# 06 — gtm-tui: Architecture

## Purpose

The TUI binary (`gtm`) connects to the daemon via IPC, mirrors daemon state via events,
and renders a Ratatui-based terminal interface with 6 tabs and modal overlays.

Depends on: `gtm-core`, `gtm-audio`, `ratatui`, `crossterm`, `tokio`, `image`, `base64`

## Event Loop

```rust
#[tokio::main]
async fn main() -> Result<()> {
    // 1. Parse CLI args
    let args = CliArgs::parse();

    // 2. Init crossterm
    crossterm::terminal::enable_raw_mode()?;
    let mut stdout = io::stdout();
    crossterm::execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let mut terminal = Terminal::new(CrosstermBackend::new(stdout))?;

    // 3. Panic hook for restoring terminal
    let panic_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |panic| {
        restore_terminal();
        panic_hook(panic);
    }));

    // 4. Create AppState + DaemonClient
    let mut app = AppState::new(args.socket_path);

    // 5. Connect to daemon (auto-launch if not running)
    app.daemon.ensure_connected().await?;

    // 6. Run event loop
    run_event_loop(&mut terminal, &mut app).await?;

    // 7. Restore terminal
    restore_terminal();
    Ok(())
}

fn restore_terminal() {
    let _ = crossterm::terminal::disable_raw_mode();
    let mut stdout = io::stdout();
    let _ = crossterm::execute!(stdout, LeaveAlternateScreen, DisableMouseCapture);
}
```

### run_event_loop

```
async fn run_event_loop(terminal, app) {
    let render_interval = tokio::time::interval(Duration::from_millis(16)); // ~60fps
    let mut last_render = Instant::now();

    loop {
        tokio::select! {
            // Non-blocking IPC event read
            events = app.daemon.poll_events() => {
                if let Ok(ev_list) = events {
                    for ev in ev_list {
                        app.daemon.apply_event(&mut app.daemon_state, ev);
                    }
                }
            }

            // User keyboard input (10ms poll)
            key_event = poll_key(Duration::from_millis(10)) => {
                if let Some(key) = key_event {
                    if handle_key(key, &mut app) == Action::Quit {
                        break;
                    }
                }
            }

            // 60fps render tick
            _ = render_interval.tick() => {
                render(&mut terminal, &mut app);
                last_render = Instant::now();
            }
        }
    }
}

fn poll_key(timeout: Duration) -> Option<KeyEvent> {
    if crossterm::event::poll(timeout).ok()? {
        crossterm::event::read().ok().and_then(|e| {
            if let crossterm::event::Event::Key(k) = e { Some(k) } else { None }
        })
    } else {
        None
    }
}
```

### Terminal Resize Handling

```
crossterm::event::Event::Resize(width, height) is handled in the key poll loop.
When a resize event is detected:
  1. terminal.autoresize() is called (Ratatui handles it internally)
  2. The next render() call automatically uses the new dimensions
  3. No special handling needed — just re-render on next tick

Kitty images must be re-placed on resize (coordinates change):
  → On Resize event, flag cover image for re-placement
  → In render(), if re-place flag set, call place_image() with new coords
```

## TUI Layout

```
┌──────────────────────────────────────────────────────────┐
│  ▶ Now Playing   Library   Queue   YouTube   Settings  H │  ← TabBar (height=1)
│  ┌────────────────────────────────────────────────────┐  │
│  │                                                    │  │
│  │              Active Tab Content                    │  │  ← Content (fills space)
│  │              (fills entire area)                   │  │
│  │                                                    │  │
│  │                                                    │  │
│  └────────────────────────────────────────────────────┘  │
│  ⏸ 2:34 / 4:20  ████████░░  Volume: 75%  🔀  🔁 All    │  ← Footer (height=1)
└──────────────────────────────────────────────────────────┘

  TabBar: 6 tabs, left-aligned, active highlighted
  Footer: left section (status icon, position/progress bar, time percent)
          right section (volume, shuffle, repeat, sleep timer, clock)
```

## Layout computation (Ratatui)

```rust
fn render(terminal: &mut Terminal<CrosstermBackend<io::Stdout>>, app: &mut AppState) {
    let _ = terminal.draw(|f| {
        let area = f.area();

        // Vertical split: tab bar | content | footer
        let chunks = Layout::vertical([
            Constraint::Length(1),       // TabBar
            Constraint::Min(0),          // Content
            Constraint::Length(1),       // Footer
        ])
        .split(area);

        // 1. TabBar
        render_tab_bar(chunks[0], f.buffer_mut(), app);

        // 2. Active tab content
        match app.active_tab {
            Tab::Library    => app.library.render(chunks[1], f.buffer_mut(), app),
            Tab::Queue      => app.queue.render(chunks[1], f.buffer_mut(), app),
            Tab::NowPlaying => app.now_playing.render(chunks[1], f.buffer_mut(), app),
            Tab::YouTube    => app.youtube.render(chunks[1], f.buffer_mut(), app),
            Tab::Settings   => app.settings.render(chunks[1], f.buffer_mut(), app),
            Tab::Help       => render_help_tab(chunks[1], f.buffer_mut(), app),
        }

        // 3. Overlay on top (if active)
        if let Some(ref mut overlay) = app.overlay {
            overlay.render(chunks[1], f.buffer_mut(), app);
        }

        // 4. Footer
        render_footer(chunks[2], f.buffer_mut(), app);
    });
}
```

## AppState

```rust
pub struct AppState {
    // ─── Daemon connection ───
    pub daemon: DaemonClient,
    pub connected: bool,
    pub daemon_state: DaemonState,
    pub daemon_state_version: u32,

    // ─── Tab routing ───
    pub active_tab: Tab,
    pub mode: UIMode,               // Normal | Filter | Command
    pub overlay: Option<Overlay>,

    // ─── Shared UI state ───
    pub filter_text: String,
    pub feedback_msg: Option<(String, Instant)>,
    pub select_mode: bool,
    pub selected_track_ids: HashSet<i64>,

    // ─── Per-tab view state ───
    pub library: LibraryViewState,
    pub queue: QueueViewState,
    pub now_playing: NowPlayingState,
    pub youtube: YouTubeViewState,
    pub settings: SettingsState,

    // ─── Caches ───
    pub cover_image: Option<ImageData>,
    pub lyrics: Option<LrcData>,
    pub lyrics_line_idx: usize,

    // ─── Scrolling ───
    pub viewer_scroll: u16,
    pub viewer_cursor: usize,

    // ─── Theme ───
    pub theme: Theme,
    pub theme_mode: ThemeMode,

    // ─── Kitty graphics ───
    pub kitty: KittyGraphics,
}

impl AppState {
    pub fn new(socket_path: Option<String>) -> Self;
}
```

## DaemonClient (IPC Transport)

```rust
pub struct DaemonClient {
    socket: Option<UnixStream>,
    reader: BufReader<UnixStream>,
    writer: BufWriter<UnixStream>,
    read_buf: Vec<u8>,              // partial frame buffer
    connected: bool,
    socket_path: PathBuf,

    // Position extrapolation
    extrapolated_pos: f64,
    pos_base_time: Instant,
    pos_base_value: f64,
    extrapolating: bool,

    // Reconnection
    retry_count: u32,
}

impl DaemonClient {
    pub fn new(socket_path: Option<PathBuf>) -> Self;

    /// Connect to daemon Unix socket.
    /// If daemon not running, spawn gtmd process and wait for socket.
    /// Exponential backoff: 100ms, 200ms, 400ms, 800ms, 1.6s, 3.2s max.
    pub async fn ensure_connected(&mut self) -> Result<()>;

    /// Send a request and await the response.
    /// Serializes as JSON line, reads JSON response line.
    /// Timeout: 5s.
    pub async fn request(&mut self, req: &DaemonRequest) -> Result<DaemonResponse>;

    /// Fire-and-forget (no response expected).
    pub async fn send(&mut self, req: &DaemonRequest) -> Result<()>;

    /// Non-blocking read of binary frames from socket.
    /// Returns accumulated events (possibly empty).
    /// Reads all available data, decodes frames, returns events.
    pub async fn poll_events(&mut self) -> Result<Vec<DaemonEvent>>;

    /// Apply a daemon event to the local state mirror.
    /// Updates daemon_state fields, extrapolation base, version counter.
    pub fn apply_event(&mut self, state: &mut DaemonState, event: &DaemonEvent);

    /// Get extrapolated position:
    ///   if playing: base_value + (now - base_time).as_secs_f64()
    ///   else: base_value
    pub fn extrapolated_position(&self) -> f64;
}
```

## Position Extrapolation

```
When playing, the TUI extrapolates position between PositionChanged events
(which arrive at ~10Hz from the daemon):

  displayed_pos = last_known_pos + (now - last_event_time).as_secs_f64()

Implementation:
  On PositionChanged event:
    pos_base_value = event.time_pos
    pos_base_time = Instant::now()
    extrapolating = (state.status == Playing)

  extrapolated_position():
    if extrapolating:
      pos_base_value + (Instant::now() - pos_base_time).as_secs_f64()
    else:
      pos_base_value
```

## Daemon Auto-Launch Protocol

```
When TUI starts and daemon socket is not reachable:

  1. Check if gtmd binary exists in PATH or next to gtm binary
  2. Spawn gtmd:
     gtmd --socket <path> --library <db_path> [--verbose]
  3. Wait for socket file to appear (poll every 100ms, max 5s)
  4. If socket appears: connect
  5. If timeout: show error message and exit

  On disconnect during runtime:
    1. Set connected = false
    2. Show feedback_msg "Connection lost — reconnecting..."
    3. Attempt reconnect with exponential backoff every render tick
    4. On reconnect: re-request GetStatus to sync state
```

## State Mirror Pattern

```
DaemonEvent                     → AppState field update
────────────────────────────────────────────────────────
PlaybackStarted{track}          daemon_state.current_track = track
                                daemon_state.status = Playing
                                daemon_state.time_pos = track.time_pos
                                daemon_state.duration = track.duration

PlaybackPaused                  daemon_state.status = Paused
                                pos_base_time = Instant::now()
                                extrapolating = false

PlaybackStopped                 daemon_state.status = Stopped
                                daemon_state.current_track = None
                                daemon_state.time_pos = 0.0
                                extrapolating = false

PositionChanged{time_pos}       daemon_state.time_pos = time_pos
                                pos_base_value = time_pos
                                pos_base_time = Instant::now()
                                extrapolating = (status == Playing)

VolumeChanged{volume}           daemon_state.volume = volume

QueueChanged{queue, cursor}     daemon_state.queue = queue
                                daemon_state.queue_cursor = cursor

RepeatModeChanged{mode}        daemon_state.repeat = mode

ShuffleChanged{enabled}         daemon_state.shuffle = enabled

SleepTimerTick{remaining}       daemon_state.sleep_timer = Some(remaining)
```

## File Structure

```
gtm-tui/src/
├── main.rs           # binary entrypoint, terminal init, event loop
├── app.rs            # App struct, render(), terminal.draw()
├── state.rs          # AppState, per-tab view states
├── daemon_client.rs  # DaemonClient (IPC transport + state mirror + extrapolation)
├── keymap.rs         # Keybindings, parse_keycode, KeyContext, KeyboardAction
├── theme.rs          # Theme struct, presets, hsl_to_rgb, random generation
├── graphics.rs       # KittyGraphics (probe, transmit, delete, place)
├── icons.rs          # IconSet, NERD_FONT, EMOJI constants
├── footer.rs         # FooterBar, FooterModule enum, rendering
├── tabs/
│   ├── mod.rs        # TabWidget trait, Tab enum
│   ├── library.rs    # LibraryTab
│   ├── queue.rs      # QueueTab
│   ├── now_playing.rs # NowPlayingTab
│   ├── youtube.rs    # YouTubeTab
│   ├── settings.rs   # SettingsTab
│   └── help.rs       # HelpTab
└── overlays/
    ├── mod.rs        # Overlay enum + dispatch
    ├── command_palette.rs
    ├── fuzzy_finder.rs
    ├── queue_picker.rs
    ├── theme_picker.rs
    ├── confirm_dialog.rs
    └── track_detail.rs
```
