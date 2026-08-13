# 06 — TUI Architecture

## Event Loop

```
┌─────────────────────────────────────────────────────────────────┐
│                      TUI Main Loop                              │
│                                                                  │
│  loop {                                                          │
│      // 1. Drain pending command results                         │
│      while let Some(err) = result_rx.try_recv() {                │
│          self.error_message = err;                               │
│      }                                                           │
│                                                                  │
│      // 2. Drain events from IPC worker                          │
│      for ev in self.client.drain().await {                       │
│          self.state.apply_event(&ev);                            │
│      }                                                           │
│                                                                  │
│      // 3. Render UI                                             │
│      terminal.draw(|f| ui::render(f, &mut self))?;               │
│                                                                  │
│      // 4. Handle key input                                      │
│      if event::poll(Duration::from_millis(50))? {                │
│          if let Event::Key(key) = event::read()? {               │
│              if !self.handle_key(key).await { break; }           │
│          }                                                       │
│      }                                                           │
│  }                                                               │
└─────────────────────────────────────────────────────────────────┘
```

Key difference from current: IPC calls never block the render loop. All daemon communication runs in a background task (see `01-ipc-redesign.md`).

## DaemonClient (non-blocking)

```rust
pub struct DaemonClient {
    cmd_tx: mpsc::UnboundedSender<DaemonReq>,
    event_queue: Arc<Mutex<Vec<DaemonEvent>>>,
    connected: bool,
}

impl DaemonClient {
    /// Fire a request and optionally wait for response via oneshot
    pub async fn send(&self, req: DaemonReq) -> Result<DaemonRes>;
    
    /// Fire a request with no response needed
    pub async fn send_fire(&self, req: DaemonReq);
    
    /// Drain buffered events
    pub fn drain(&mut self) -> Vec<DaemonEvent>;
    
    /// Get current daemon state (sends GetStatus, awaits response)
    pub async fn get_status(&self) -> Result<DaemonState>;
}
```

## State Mirror

`App.state` is a `DaemonState` mirror updated via `apply_event()`. No IPC calls needed for position/status updates — they come through the pulse event stream.

### Event → State Mapping

| DaemonEvent | State Mutation |
|---|---|
| `PlaybackStarted { track, time_pos, duration }` | `status = Playing; current_track = track; time_pos; duration` |
| `PlaybackPaused` | `status = Paused` |
| `PlaybackStopped` | `status = Stopped; time_pos = 0` |
| `PositionChanged { time_pos }` | `state.time_pos = time_pos` |
| `DurationChanged { duration }` | `state.duration = duration` |
| `VolumeChanged { volume }` | `state.volume = volume` |
| `QueueChanged { queue, cursor }` | `state.queue = queue; state.queue_cursor = cursor` |
| `QueueIndexChanged { index }` | `state.queue_cursor = index` |
| `RepeatModeChanged { mode }` | `state.repeat = mode` |
| `ShuffleChanged { enabled }` | `state.shuffle = enabled` |

## Position Extrapolation

When no position events arrive (e.g., between render frames), extrapolate:

```rust
let now = Instant::now();
let elapsed = now.duration_since(last_event_time).as_secs_f64();
let display_pos = if state.status == Playing {
    state.time_pos + elapsed
} else {
    state.time_pos
};
```

## Layout Structure

The TUI uses a **rigid 4-row grid** layout with sharp (plain) borders throughout, following the Cyberdeck TUI design system:

```
┌─────────────────────────────────────────────────┐
│ > GTM    [1]NowPlaying  [2]Library  [3]Settings │  3 lines (Tab Bar)
│  (notification line — single row)               │  1 line
├─────────────────────────────────────────────────┤
│                                                  │
│  Content Area (per-tab)                          │  Min(0)
│                                                  │
│  ┌─ NOW PLAYING ─────────────────────────────┐  │
│  │  Title:   Song Title                       │  │
│  │  Artist:  Artist Name                      │  │
│  │  Album:   Album Name                       │  │
│  │  [####------------] 1:23 / 4:56            │  │
│  └────────────────────────────────────────────┘  │
│                                                  │
│  ┌─ LIBRARY ──────────────────────────────────┐  │
│  │  1. All Tracks    │  ▶ Artist - Title [dur] │  │
│  │  2. Albums        │  ...                    │  │
│  └────────────────────────────────────────────┘  │
│                                                  │
│  ┌─ SETTINGS ─────────────────────────────────┐  │
│  │  Volume: 80% [########--------]             │  │
│  │  Repeat: OFF                                │  │
│  └────────────────────────────────────────────┘  │
│                                                  │
│  Overlays float above content (Alt+key)          │
│  > Queue, YTSearch, SearchLibrary, Spotify,      │
│    Equalizer, CommandPalette, About, SleepTimer,  │
│    ThemePicker, SoundEffects                     │
│                                                  │
├─────────────────────────────────────────────────┤
│  > [Vol:80%] [Queue:12] [RPT] [SHF]             │  3 lines (Footer)
└─────────────────────────────────────────────────┘
```

- **Cyberdeck TUI** theme: deep charcoal `#141313` background, off-white `#e5e2e1` text, neon green `#00e639` accents
- 3 tabs only: NowPlaying, Library, Settings
- 10 overlays: Queue, YTSearch, SearchLibrary, SpotifySearch, Equalizer, CommandPalette, About, SleepTimer, ThemePicker, SoundEffects
- All borders use `BorderType::Plain` (sharp corners — no rounded borders)
- All controls use bracket notation `[Key]Action` — no emoji
- Overlays accessible via Alt+key from any tab
- Footer shows playback status, volume, queue count, repeat/shuffle indicators
- High information density with minimal padding
