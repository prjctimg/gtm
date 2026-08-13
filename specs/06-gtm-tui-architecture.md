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

IPC calls never block the render loop. All daemon communication runs in a background task.

## DaemonClient (non-blocking)

```rust
pub struct DaemonClient {
    cmd_tx: mpsc::UnboundedSender<DaemonReq>,
    event_queue: Arc<Mutex<Vec<DaemonEvent>>>,
    connected: bool,
}

impl DaemonClient {
    pub async fn send(&self, req: DaemonReq) -> Result<DaemonRes>;
    pub async fn send_fire(&self, req: DaemonReq);
    pub fn drain(&mut self) -> Vec<DaemonEvent>;
    pub async fn get_status(&self) -> Result<DaemonState>;
}
```

## State Mirror

`App.state` is a `DaemonState` mirror updated via `apply_event()`. No IPC calls needed for position/status updates.

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

```
┌──────────────────────────────────────────────────────────────┐
│ [1] Now Playing  [2] Library  [3] Settings    gtm 0.7.34    │  Tab Bar (3 lines)
│  (notification line)                                          │  1 line
├──────────────────────────────────────────────────────────────┤
│                                                               │
│  Content Area (per-tab)                                      │  Min(0)
│                                                               │
│  ┌─ NOW PLAYING ──────────────────────────────────────────┐  │
│  │  ┌──┐  NOW PLAYING                                     │  │
│  │  │▀▀│  Codeine Crazy (Official Audio)                   │  │
│  │  │▀▀│  Artist: Future                                   │  │
│  │  │▀▀│  Format: [FLAC | 24-bit/96kHz]                   │  │
│  │  └──┘                                                   │  │
│  │      00:45                          5:52                │  │
│  │      ▓▓▓▓▓▓░░░░░░  (visualizer bars)                   │  │
│  └─────────────────────────────────────────────────────────┘  │
│                                                               │
│  ┌─ LIBRARY ───────────────────────────────────────────────┐  │
│  │  #  │ Title / Artist / Album      │ Dur   │ Bitrate     │  │
│  │  >01│ Future - Codeine Crazy      │ 05:41 │ 128kbps     │  │
│  │   02│ Juice WRLD - Stay High      │ 03:48 │ 320kbps     │  │
│  └─────────────────────────────────────────────────────────┘  │
│                                                               │
│  ┌─ SETTINGS ──────────────────────────────────────────────┐  │
│  │  ♫ Audio       Cookie Source    [ chromium  ▶ ]         │  │
│  │  ▶ YouTube     Auto Download    [ ● ] On                │  │
│  │  ✧ Appearance  Clear History    [Clear]                  │  │
│  └─────────────────────────────────────────────────────────┘  │
│                                                               │
│  Overlays float above content (Alt+key)                       │
│  > Queue, YTSearch, SearchLibrary, Spotify, Equalizer, ...   │
│                                                               │
├──────────────────────────────────────────────────────────────┤
│  [0:06] [1/13] [65%] [ALSA] [▶]                             │  Footer (3 lines)
└──────────────────────────────────────────────────────────────┘
```

- **Cyberdeck TUI** theme: deep charcoal `#141313` background, off-white `#e5e2e1` text, neon green `#00e639` accents
- 3 tabs: NowPlaying (visualizer progress, album art, metadata), Library (track table + sidebar), Settings (sidebar + bracket-style panel)
- Footer uses multi-segment colored blocks for status info
- All borders use `BorderType::Plain` (sharp corners)
- High information density with minimal padding
