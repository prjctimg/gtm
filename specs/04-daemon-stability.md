# 04 — Daemon Stability

## Goal

Fix daemon stability issues where playback commands (prev/next) cause the daemon connection to die. Ensure the daemon always returns to a graceful idle state after handling commands. Show error notifications to users when commands fail.

## Current State

When sending playback commands like prev/next, the daemon connection seems to die. Subsequent commands are buffered and sent on reconnect. The daemon should never crash or disconnect after handling a command.

## Required Changes

### 1. Analyze Current Command Handling

Review the current implementation to identify why connections die:

```rust
// In gtmd/src/daemon.rs
async fn dispatch(&mut self, client_id: ClientId, req: DaemonReq) {
    match req {
        DaemonReq::Next => {
            self.cmd_next().await;
            // Is there a panic or unwrap here?
        }
        // ...
    }
}
```

Potential issues:
- Unwrap/expect calls that panic
- Missing error handling
- Resource cleanup issues
- State machine transitions that leave daemon in invalid state

### 2. Implement Robust Error Handling

Add proper error handling to all command handlers:

```rust
async fn cmd_next(&mut self) -> Result<(), DaemonError> {
    // 1. Validate state
    if self.state.status != PlaybackStatus::Playing {
        return Err(DaemonError::NotPlaying);
    }
    
    // 2. Execute command
    self.backend.next()
        .map_err(|e| DaemonError::AudioError(e))?;
    
    // 3. Update state
    self.state.queue_cursor += 1;
    self.state.time_pos = 0.0;
    
    // 4. Push event
    self.push_event(DaemonEvent::QueueIndexChanged { 
        index: self.state.queue_cursor 
    }).await;
    
    // 5. Return to idle state
    self.state.status = PlaybackStatus::Playing;
    
    Ok(())
}
```

### 3. Daemon State Machine

Implement explicit state machine with proper transitions:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DaemonState {
    Idle,
    ProcessingCommand,
    Playing,
    Paused,
    Stopped,
    Error(DaemonError),
}

impl Daemon {
    async fn handle_command(&mut self, client_id: ClientId, req: DaemonReq) {
        // Transition to ProcessingCommand
        self.state.current_state = DaemonState::ProcessingCommand;
        
        let result = match req {
            DaemonReq::Next => self.cmd_next().await,
            DaemonReq::Prev => self.cmd_prev().await,
            DaemonReq::Seek(pos) => self.cmd_seek(pos).await,
            // ...
        };
        
        match result {
            Ok(()) => {
                // Return to appropriate state
                self.state.current_state = match self.state.playback_status {
                    PlaybackStatus::Playing => DaemonState::Playing,
                    PlaybackStatus::Paused => DaemonState::Paused,
                    PlaybackStatus::Stopped => DaemonState::Stopped,
                };
                
                // Send success response
                self.send_response(client_id, DaemonRes::Ok).await;
            }
            Err(e) => {
                // Transition to error state
                self.state.current_state = DaemonState::Error(e.clone());
                
                // Send error response
                self.send_response(client_id, DaemonRes::Error(e.to_string())).await;
                
                // Push error event for notification
                self.push_event(DaemonEvent::CommandFailed { 
                    command: req.name(),
                    error: e.to_string() 
                }).await;
                
                // Auto-recover after brief delay
                tokio::time::sleep(Duration::from_millis(100)).await;
                self.state.current_state = match self.state.playback_status {
                    PlaybackStatus::Playing => DaemonState::Playing,
                    PlaybackStatus::Paused => DaemonState::Paused,
                    PlaybackStatus::Stopped => DaemonState::Stopped,
                };
            }
        }
    }
}
```

### 4. Connection Management

Ensure connections are not dropped unexpectedly:

```rust
impl Daemon {
    async fn accept_client(&mut self, stream: UnixStream) {
        let client_id = self.next_client_id;
        self.next_client_id += 1;
        
        let (reader, writer) = stream.into_split();
        
        // Spawn reader task with proper error handling
        let req_tx = self.req_tx.clone();
        tokio::spawn(async move {
            if let Err(e) = Self::handle_client_reader(client_id, reader, req_tx).await {
                tracing::error!("Client {} reader error: {}", client_id, e);
                // Don't panic, just clean up
            }
        });
        
        // Spawn writer task with proper error handling
        let event_rx = self.event_tx.subscribe();
        tokio::spawn(async move {
            if let Err(e) = Self::handle_client_writer(writer, event_rx).await {
                tracing::error!("Client {} writer error: {}", client_id, e);
                // Don't panic, just clean up
            }
        });
        
        tracing::info!("Client {} connected", client_id);
    }
    
    async fn handle_client_reader(
        client_id: ClientId,
        mut reader: tokio::net::unix::OwnedReadHalf,
        req_tx: mpsc::UnboundedSender<(ClientId, DaemonReq)>,
    ) -> Result<(), DaemonError> {
        let mut buffer = String::new();
        
        loop {
            buffer.clear();
            match reader.read_line(&mut buffer).await {
                Ok(0) => {
                    // Connection closed
                    tracing::info!("Client {} disconnected", client_id);
                    break;
                }
                Ok(_) => {
                    match serde_json::from_str::<DaemonReq>(&buffer) {
                        Ok(req) => {
                            if let Err(e) = req_tx.send((client_id, req)) {
                                tracing::error!("Failed to send request: {}", e);
                                break;
                            }
                        }
                        Err(e) => {
                            tracing::error!("Invalid request from client {}: {}", client_id, e);
                            // Continue reading, don't disconnect
                        }
                    }
                }
                Err(e) => {
                    tracing::error!("Read error from client {}: {}", client_id, e);
                    break;
                }
            }
        }
        
        Ok(())
    }
}
```

### 5. Command Response Guarantee

Ensure every command gets a response:

```rust
impl Daemon {
    async fn dispatch_with_response(&mut self, client_id: ClientId, req: DaemonReq) {
        // Always send a response, even on panic
        let result = std::panic::AssertUnwindSafe(async {
            self.handle_command(client_id, req).await
        });
        
        if let Err(e) = tokio::task::spawn(result).await {
            // Handle panic
            tracing::error!("Command panicked: {:?}", e);
            self.send_response(client_id, DaemonRes::Error("Internal error".to_string())).await;
        }
    }
}
```

### 6. Idle State Management

Ensure daemon returns to idle after every command:

```rust
impl Daemon {
    async fn return_to_idle(&mut self) {
        // Ensure all resources are properly released
        // Ensure state is consistent
        // Ensure no pending operations
        
        self.state.current_state = match self.state.playback_status {
            PlaybackStatus::Playing => DaemonState::Playing,
            PlaybackStatus::Paused => DaemonState::Paused,
            PlaybackStatus::Stopped => DaemonState::Idle,
        };
        
        tracing::trace!("Daemon returned to idle state");
    }
}
```

### 7. Error Notification in TUI

Show error notifications to users when commands fail:

```rust
// In gtm/src/app.rs
impl App {
    fn handle_event(&mut self, event: DaemonEvent) {
        match event {
            DaemonEvent::CommandFailed { command, error } => {
                self.notification = Some(Notification {
                    message: format!("{} failed: {}", command, error),
                    level: NotificationLevel::Error,
                    timestamp: Instant::now(),
                    duration: Duration::from_secs(3),
                });
            }
            // ...
        }
    }
}
```

Add notification widget to TUI:

```rust
// In gtm/src/ui.rs
fn render_notification(f: &mut Frame, area: Rect, notification: &Notification) {
    let color = match notification.level {
        NotificationLevel::Error => Color::Red,
        NotificationLevel::Warning => Color::Yellow,
        NotificationLevel::Info => Color::Green,
    };
    
    let paragraph = Paragraph::new(notification.message.clone())
        .style(Style::default().fg(color).add_modifier(Modifier::BOLD))
        .alignment(Alignment::Center);
    
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Plain)
        .style(Style::default().fg(color));
    
    f.render_widget(paragraph.block(block), area);
}
```

### 8. Reconnection Logic

Handle daemon restarts gracefully:

```rust
// In gtm-core/src/client.rs
impl DaemonClient {
    async fn ensure_connected(&mut self) -> Result<(), ClientError> {
        if self.connected {
            return Ok(());
        }
        
        // Try to reconnect
        for attempt in 0..MAX_RECONNECT_ATTEMPTS {
            match self.connect().await {
                Ok(()) => {
                    self.connected = true;
                    return Ok(());
                }
                Err(e) => {
                    tracing::warn!("Reconnect attempt {} failed: {}", attempt + 1, e);
                    tokio::time::sleep(Duration::from_millis(100 * (attempt as u64 + 1))).await;
                }
            }
        }
        
        Err(ClientError::ConnectionFailed)
    }
}
```

## Files to Modify

- `gtmd/src/daemon.rs` — Fix command handling, add error recovery, state machine
- `gtm-core/src/ipc.rs` — Add CommandFailed event, error types
- `gtm-core/src/client.rs` — Add reconnection logic
- `gtm/src/app.rs` — Handle error notifications
- `gtm/src/ui.rs` — Render notification widget
- `gtm/src/notification.rs` — New file: Notification types and rendering

## Implementation Details

### Error Types

```rust
// In gtm-core/src/error.rs
#[derive(Debug, Clone, thiserror::Error)]
pub enum DaemonError {
    #[error("Not playing")]
    NotPlaying,
    
    #[error("Audio error: {0}")]
    AudioError(String),
    
    #[error("Invalid seek position: {0}")]
    InvalidSeekPosition(f64),
    
    #[error("Queue error: {0}")]
    QueueError(String),
    
    #[error("Internal error")]
    Internal,
}

impl Serialize for DaemonError {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.to_string())
    }
}
```

### Command Validation

```rust
impl Daemon {
    fn validate_command(&self, req: &DaemonReq) -> Result<(), DaemonError> {
        match req {
            DaemonReq::Next => {
                if self.state.queue_cursor >= self.state.queue.len() {
                    return Err(DaemonError::QueueError("At end of queue".to_string()));
                }
                Ok(())
            }
            DaemonReq::Prev => {
                if self.state.queue_cursor == 0 {
                    return Err(DaemonError::QueueError("At start of queue".to_string()));
                }
                Ok(())
            }
            DaemonReq::Seek(pos) => {
                if *pos < 0.0 || *pos > self.state.duration {
                    return Err(DaemonError::InvalidSeekPosition(*pos));
                }
                Ok(())
            }
            _ => Ok(()),
        }
    }
}
```

### State Recovery

```rust
impl Daemon {
    async fn recover_from_error(&mut self) {
        // Log the error
        tracing::error!("Recovering from error: {:?}", self.state.current_state);
        
        // Stop playback if in error state
        if let DaemonState::Error(_) = self.state.current_state {
            let _ = self.backend.stop();
        }
        
        // Reset to stable state
        self.state.current_state = DaemonState::Idle;
        self.state.playback_status = PlaybackStatus::Stopped;
        
        // Notify clients
        self.push_event(DaemonEvent::PlaybackStopped).await;
    }
}
```

## Checklist

- [ ] All command handlers have proper error handling
- [ ] No unwrap/expect calls in command handlers
- [ ] Daemon returns to idle state after every command
- [ ] State machine implemented with explicit transitions
- [ ] Every command gets a response (success or error)
- [ ] Connection does not die on command execution
- [ ] Error notifications shown in TUI
- [ ] Notification widget implemented
- [ ] Reconnection logic works
- [ ] State recovery works after errors
- [ ] Command validation prevents invalid operations
- [ ] No panics in daemon code
- [ ] `cargo check --workspace` passes
- [ ] `cargo test --workspace` passes
- [ ] Manual testing of prev/next/seek commands
- [ ] Verify daemon stays alive after rapid commands

## Testing Strategy

1. **Unit Tests**: Test each command handler individually
2. **Integration Tests**: Test command sequences (play → next → prev → seek)
3. **Stress Tests**: Send rapid commands to test stability
4. **Error Injection**: Simulate audio backend failures
5. **Connection Tests**: Test client disconnect/reconnect scenarios

## Monitoring

Add logging for debugging:

```rust
impl Daemon {
    async fn handle_command(&mut self, client_id: ClientId, req: DaemonReq) {
        let start = Instant::now();
        tracing::debug!("Handling command from client {}: {:?}", client_id, req);
        
        // ... handle command ...
        
        let duration = start.elapsed();
        tracing::debug!("Command handled in {:?}", duration);
    }
}
```