# 07 — Sleep Timer Improvement

## Goal

Improve the sleep timer with a slider overlay that users can adjust with h/l keys or manually input the value. Maximum time is 3 hours in minutes. The sleep timer must persist even when the client is closed because it tells the daemon to halt playback.

## Current State

Basic sleep timer exists with quick options (5m, 10m, 15m, 30m, 60m). No slider, no manual input, no persistence after client close.

## Required Changes

### 1. Create Sleep Timer Overlay

Create a new overlay with slider and manual input:

```rust
#[derive(Debug, Clone)]
pub struct SleepTimerOverlay {
    pub minutes: u32,
    pub max_minutes: u32,
    pub is_active: bool,
    pub remaining_seconds: Option<u64>,
    pub input_mode: bool,
    pub input_buffer: String,
}

impl SleepTimerOverlay {
    pub fn new() -> Self {
        Self {
            minutes: 30,
            max_minutes: 180,  // 3 hours
            is_active: false,
            remaining_seconds: None,
            input_mode: false,
            input_buffer: String::new(),
        }
    }
    
    pub fn increase(&mut self, amount: u32) {
        self.minutes = (self.minutes + amount).min(self.max_minutes);
    }
    
    pub fn decrease(&mut self, amount: u32) {
        self.minutes = self.minutes.saturating_sub(amount);
    }
    
    pub fn set_minutes(&mut self, minutes: u32) {
        self.minutes = minutes.min(self.max_minutes);
    }
    
    pub fn start(&mut self) {
        self.is_active = true;
        self.remaining_seconds = Some(self.minutes as u64 * 60);
    }
    
    pub fn cancel(&mut self) {
        self.is_active = false;
        self.remaining_seconds = None;
    }
    
    pub fn tick(&mut self) {
        if let Some(remaining) = &mut self.remaining_seconds {
            if *remaining > 0 {
                *remaining -= 1;
            } else {
                self.cancel();
            }
        }
    }
}
```

### 2. Render Slider Overlay

```
┌─────────────────────────────────────────────┐
│              Sleep Timer                      │
├─────────────────────────────────────────────┤
│                                               │
│  Timer: 45 minutes                            │
│                                               │
│  [────────────────●────────────────]         │
│   0m        90m                       180m    │
│                                               │
│  h/- Decrease    l/+ Increase                 │
│  Enter: Set Timer    Esc: Close               │
│  i: Manual Input                             │
│                                               │
└─────────────────────────────────────────────┘
```

#### Slider Rendering

```rust
fn render_slider(&self, f: &mut Frame, area: Rect) {
    let width = area.width as u32;
    let progress = self.minutes as f32 / self.max_minutes as f32;
    let slider_position = (progress * width as f32) as u16;
    
    let slider_line = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Length(slider_position),
            Constraint::Length(1),
            Constraint::Min(0),
        ])
        .split(area);
    
    // Render left part (filled)
    let left = Paragraph::new("─".repeat(slider_position as usize))
        .style(Style::default().fg(Color::Green));
    f.render_widget(left, slider_line[0]);
    
    // Render handle
    let handle = Paragraph::new("●")
        .style(Style::default().fg(Color::White).add_modifier(Modifier::BOLD));
    f.render_widget(handle, slider_line[1]);
    
    // Render right part (empty)
    let right_width = width - slider_position - 1;
    let right = Paragraph::new("─".repeat(right_width as usize))
        .style(Style::default().fg(Color::DarkGray));
    f.render_widget(right, slider_line[2]);
}
```

### 3. Keybindings

```rust
impl SleepTimerOverlay {
    pub fn handle_key(&mut self, key: KeyEvent) -> SleepTimerAction {
        if self.input_mode {
            // Manual input mode
            match key.code {
                KeyCode::Enter => {
                    if let Ok(minutes) = self.input_buffer.parse::<u32>() {
                        self.set_minutes(minutes);
                        self.input_mode = false;
                        self.input_buffer.clear();
                        return SleepTimerAction::Set;
                    }
                    self.input_mode = false;
                    self.input_buffer.clear();
                    SleepTimerAction::None
                }
                KeyCode::Esc => {
                    self.input_mode = false;
                    self.input_buffer.clear();
                    SleepTimerAction::None
                }
                KeyCode::Char(c) => {
                    self.input_buffer.push(c);
                    SleepTimerAction::None
                }
                KeyCode::Backspace => {
                    self.input_buffer.pop();
                    SleepTimerAction::None
                }
                _ => SleepTimerAction::None,
            }
        } else {
            // Slider mode
            match key.code {
                KeyCode::Char('h') | KeyCode::Left => {
                    self.decrease(5);
                    SleepTimerAction::None
                }
                KeyCode::Char('l') | KeyCode::Right => {
                    self.increase(5);
                    SleepTimerAction::None
                }
                KeyCode::Char('-') => {
                    self.decrease(1);
                    SleepTimerAction::None
                }
                KeyCode::Char('+') | KeyCode::Char('=') => {
                    self.increase(1);
                    SleepTimerAction::None
                }
                KeyCode::Enter => {
                    self.start();
                    SleepTimerAction::Start
                }
                KeyCode::Char('i') => {
                    self.input_mode = true;
                    SleepTimerAction::None
                }
                KeyCode::Esc => {
                    SleepTimerAction::Close
                }
                _ => SleepTimerAction::None,
            }
        }
    }
}
```

### 4. Action Enum

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SleepTimerAction {
    None,
    Start,
    Cancel,
    Set,
    Close,
}
```

### 5. Daemon Integration

The sleep timer must persist in the daemon, not the TUI client:

```rust
// In gtmd/src/daemon.rs
impl Daemon {
    pub struct Daemon {
        // ... existing fields
        pub sleep_timer: Option<SleepTimer>,
    }
    
    pub struct SleepTimer {
        pub end_time: Instant,
        pub duration: Duration,
    }
    
    pub async fn set_sleep_timer(&mut self, minutes: u32) {
        let duration = Duration::from_secs(minutes as u64 * 60);
        let end_time = Instant::now() + duration;
        
        self.sleep_timer = Some(SleepTimer {
            end_time,
            duration,
        });
        
        // Spawn background task to handle timer
        let event_tx = self.event_tx.clone();
        let state = self.state.clone();
        
        tokio::spawn(async move {
            tokio::time::sleep(duration).await;
            
            // Stop playback
            let mut state = state.write().await;
            state.playback_status = PlaybackStatus::Stopped;
            
            // Push event
            let _ = event_tx.send(DaemonEvent::PlaybackStopped);
            let _ = event_tx.send(DaemonEvent::SleepTimerExpired);
        });
        
        tracing::info!("Sleep timer set for {} minutes", minutes);
    }
    
    pub async fn cancel_sleep_timer(&mut self) {
        self.sleep_timer = None;
        tracing::info!("Sleep timer cancelled");
    }
}
```

### 6. IPC Commands

Add to `gtm-core/src/ipc.rs`:

```rust
pub enum DaemonReq {
    // ... existing commands
    SetSleepTimer { minutes: u32 },
    CancelSleepTimer,
    GetSleepTimerStatus,
}

pub enum DaemonRes {
    // ... existing responses
    SleepTimerSet { minutes: u32 },
    SleepTimerCancelled,
    SleepTimerStatus { 
        active: bool, 
        remaining_seconds: Option<u64> 
    },
}

pub enum DaemonEvent {
    // ... existing events
    SleepTimerExpired,
    SleepTimerUpdated { remaining_seconds: u64 },
}
```

### 7. TUI State Management

```rust
// In gtm/src/app.rs
impl App {
    pub fn update_sleep_timer(&mut self) {
        if let Some(timer) = &self.state.sleep_timer {
            self.sleep_timer_overlay.is_active = true;
            self.sleep_timer_overlay.remaining_seconds = Some(timer.remaining_seconds);
        } else {
            self.sleep_timer_overlay.is_active = false;
            self.sleep_timer_overlay.remaining_seconds = None;
        }
    }
}
```

### 8. Footer Display

When sleep timer is active, show in footer:

```
┌──────────────────────────────────────────────────────────────────┐
│  [0:06] [1/13] [65%] [ALSA] [▶] [⏰ 45:00]                     │
└──────────────────────────────────────────────────────────────────┘
```

```rust
fn render_sleep_timer_footer(&self, f: &mut Frame, area: Rect) {
    if let Some(remaining) = self.sleep_timer_overlay.remaining_seconds {
        let minutes = remaining / 60;
        let seconds = remaining % 60;
        let text = format!("⏰ {:02}:{:02}", minutes, seconds);
        
        let span = Span::styled(text, Style::default().fg(Color::Yellow));
        f.render_widget(Paragraph::new(span), area);
    }
}
```

### 9. Persistence

The sleep timer persists in the daemon because:
1. Daemon owns the timer
2. Timer runs in daemon background task
3. When TUI client disconnects, daemon keeps running
4. Timer continues counting down
5. When timer expires, daemon stops playback

### 10. Quick Options

Keep quick options for convenience:

```
┌─────────────────────────────────────────────┐
│              Sleep Timer                      │
├─────────────────────────────────────────────┤
│                                               │
│  Timer: 45 minutes                            │
│                                               │
│  [────────────────●────────────────]         │
│   0m        90m                       180m    │
│                                               │
│  Quick Set:                                   │
│  [5m] [10m] [15m] [30m] [60m] [90m] [120m]   │
│                                               │
│  h/- Decrease    l/+ Increase                 │
│  Enter: Set Timer    Esc: Close               │
│  i: Manual Input                             │
│                                               │
└─────────────────────────────────────────────┘
```

```rust
fn render_quick_options(&self, f: &mut Frame, area: Rect) {
    let options = [5, 10, 15, 30, 60, 90, 120];
    
    let spans: Vec<Span> = options
        .iter()
        .map(|&m| {
            let style = if m == self.minutes {
                Style::default().fg(Color::Green).add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::DarkGray)
            };
            Span::styled(format!("[{}m] ", m), style)
        })
        .collect();
    
    let line = Line::from(spans);
    f.render_widget(Paragraph::new(line), area);
}
```

## Files to Modify

- `gtm/src/overlay.rs` — Add SleepTimerOverlay struct and rendering
- `gtm/src/app.rs` — Add sleep timer state management
- `gtm/src/footer.rs` — Add sleep timer display
- `gtm/src/keymap.rs` — Add keybindings for sleep timer
- `gtm-core/src/ipc.rs` — Add sleep timer IPC commands/events
- `gtmd/src/daemon.rs` — Implement sleep timer in daemon
- `gtmd/src/daemon.rs` — Add sleep timer background task

## Implementation Details

### Timer Tick

```rust
impl App {
    fn tick_sleep_timer(&mut self) {
        if let Some(remaining) = &mut self.sleep_timer_overlay.remaining_seconds {
            if *remaining > 0 {
                *remaining -= 1;
                self.sleep_timer_overlay.minutes = (*remaining / 60) as u32;
            }
        }
    }
}
```

### Background Task

```rust
impl Daemon {
    async fn run_sleep_timer(&self, duration: Duration) {
        tokio::time::sleep(duration).await;
        
        // Stop playback
        self.backend.stop().await;
        
        // Update state
        {
            let mut state = self.state.write().await;
            state.playback_status = PlaybackStatus::Stopped;
        }
        
        // Push events
        self.push_event(DaemonEvent::PlaybackStopped).await;
        self.push_event(DaemonEvent::SleepTimerExpired).await;
        
        // Clear timer
        self.sleep_timer = None;
        
        tracing::info!("Sleep timer expired, playback stopped");
    }
}
```

### Client Disconnect Handling

```rust
impl Daemon {
    async fn handle_client_disconnect(&mut self, client_id: ClientId) {
        tracing::info!("Client {} disconnected", client_id);
        
        // Sleep timer continues running in daemon
        // No need to cancel it
        
        // Clean up client resources
        self.clients.retain(|c| c.id != client_id);
    }
}
```

## Checklist

- [ ] SleepTimerOverlay struct created
- [ ] Slider rendered with h/l navigation
- [ ] Manual input mode works
- [ ] Maximum time limited to 3 hours (180 minutes)
- [ ] Quick options (5m, 10m, 15m, 30m, 60m, 90m, 120m) available
- [ ] Enter starts timer
- [ ] Esc closes overlay
- [ ] i enters manual input mode
- [ ] Timer persists after TUI client closes
- [ ] Daemon stops playback when timer expires
- [ ] Sleep timer status shown in footer
- [ ] IPC commands added (SetSleepTimer, CancelSleepTimer, GetSleepTimerStatus)
- [ ] IPC events added (SleepTimerExpired, SleepTimerUpdated)
- [ ] Background task handles timer expiration
- [ ] Client disconnect does not cancel timer
- [ ] `cargo check --workspace` passes
- [ ] `cargo test --workspace` passes

## Visual Design

### Active Timer in Footer
```
[0:06] [1/13] [65%] [ALSA] [▶] [⏰ 45:00]
```

### Timer Expired Notification
```
┌─────────────────────────────────────────────┐
│         Sleep Timer Expired                   │
│                                               │
│  Playback has been stopped.                   │
│                                               │
│                    [OK]                       │
└─────────────────────────────────────────────┘
```

### Cancellation
```
┌─────────────────────────────────────────────┐
│              Sleep Timer                      │
├─────────────────────────────────────────────┤
│                                               │
│  Timer Active: 45:00 remaining                │
│                                               │
│  [Cancel Timer]    [Close]                    │
│                                               │
└─────────────────────────────────────────────┘
```