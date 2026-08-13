# 11 — gtm-mpris: MPRIS D-Bus Server

## Purpose

Exposes the daemon's playback state and controls via the MPRIS D-Bus specification.
Allows media keys, GNOME/KDE lock screen controls, and external tools (playerctl) to
control gtm.

Depends on: `gtm-core`, `zbus` (feature = "tokio"), `zvariant`, `tracing`

## MprisServer Struct

```rust
use std::sync::Arc;
use tokio::sync::RwLock;
use gtm_core::{DaemonState, DaemonEvent, DaemonRequest};

pub struct MprisServer {
    state: Arc<RwLock<DaemonState>>,
    event_tx: tokio::sync::broadcast::Sender<DaemonEvent>,
    daemon_tx: tokio::sync::mpsc::Sender<DaemonRequest>,
    connection: Option<zbus::Connection>,
}

impl MprisServer {
    /// Create a new MprisServer connected to daemon state.
    pub fn new(
        state: Arc<RwLock<DaemonState>>,
        event_tx: tokio::sync::broadcast::Sender<DaemonEvent>,
        daemon_tx: tokio::sync::mpsc::Sender<DaemonRequest>,
    ) -> Self;

    /// Run the MPRIS server. Registers on session D-Bus and processes events.
    /// This is a long-running async task.
    pub async fn run(&mut self) -> Result<()>;

    /// Handle a D-Bus method call by sending a DaemonRequest.
    async fn send_request(&self, req: DaemonRequest) -> Result<()>;

    /// Bridge daemon events to MPRIS property updates.
    async fn handle_event(&mut self, event: DaemonEvent);

    /// Build the Metadata dict from current track.
    fn build_metadata(state: &DaemonState) -> zvariant::Dict;
}

// Error type
#[derive(Debug, thiserror::Error)]
pub enum MprisError {
    #[error("D-Bus connection error: {0}")]
    Connection(String),
    #[error("zbus error: {0}")]
    Zbus(#[from] zbus::Error),
    #[error("daemon error: {0}")]
    Daemon(String),
}
```

## Architecture

```
┌───────────────────────────────────────────────────────────────┐
│  gtmd Process                                                  │
│                                                                │
│  ┌────────────────────────────────────────┐                   │
│  │  Daemon                                │                   │
│  │  (main loop)                           │                   │
│  │  ┌──────────────────────────────────┐  │                   │
│  │  │  daemon_state: Arc<RwLock<...>>  │──┼── shared state   │
│  │  │  event_tx: broadcast::Sender     │──┼── event stream   │
│  │  │  daemon_rx: mpsc::Receiver       │◀─┼── request chan   │
│  │  └──────────────────────────────────┘  │                   │
│  └─────────────┬──────────────────────────┘                   │
│                │                                               │
│                ▼                                               │
│  ┌────────────────────────────────────────┐                   │
│  │  MprisServer                            │                   │
│  │  (spawned as tokio task)                │                   │
│  │                                         │                   │
│  │  • Connects to session D-Bus            │                   │
│  │  • Registers:                           │                   │
│  │    org.mpris.MediaPlayer2.gtm           │                   │
│  │    /org/mpris/MediaPlayer2              │                   │
│  │                                         │                   │
│  │  • Serves:                              │                   │
│  │    org.mpris.MediaPlayer2               │                   │
│  │    org.mpris.MediaPlayer2.Player        │                   │
│  │                                         │                   │
│  │  • Listens on event_rx for changes      │                   │
│  │  • On daemon event: update properties,  │                   │
│  │    emit PropertiesChanged signal        │                   │
│  │  • On MPRIS method call: send           │                   │
│  │    DaemonRequest via daemon_tx          │                   │
│  └────────────────────────────────────────┘                   │
└───────────────────────────────────────────────────────────────┘
```

## D-Bus Interface — `org.mpris.MediaPlayer2`

### Properties

| Property | Type | Value |
|----------|------|-------|
| `CanQuit` | Boolean | `true` |
| `CanRaise` | Boolean | `false` |
| `HasTrackList` | Boolean | `false` |
| `Identity` | String | `"gtm"` |
| `DesktopEntry` | String | `"gtm"` |
| `SupportedUriSchemes` | Array of String | `["file"]` |
| `SupportedMimeTypes` | Array of String | `["audio/flac","audio/mpeg","audio/ogg","audio/wav","audio/x-m4a","audio/aac","audio/opus"]` |

### Methods

| Method | Implementation |
|--------|---------------|
| `Raise()` | No-op |
| `Quit()` | Send `DaemonRequest::Quit` via daemon_tx |

## D-Bus Interface — `org.mpris.MediaPlayer2.Player`

### Properties

| Property | Type | Source |
|----------|------|--------|
| `PlaybackStatus` | String | `daemon_state.status` → `"Playing"`, `"Paused"`, `"Stopped"` |
| `LoopStatus` | String | `daemon_state.repeat` → `"None"`, `"Track"`, `"Playlist"` |
| `Shuffle` | Boolean | `daemon_state.shuffle` |
| `Metadata` | Dict (a{sv}) | Built from `current_track` |
| `Volume` | Double | `daemon_state.volume / 100.0` |
| `Position` | Int64 (μs) | `daemon_state.time_pos * 1_000_000` |
| `MinimumRate` | Double | `1.0` |
| `MaximumRate` | Double | `1.0` |
| `CanGoNext` | Boolean | `true` |
| `CanGoPrevious` | Boolean | `true` |
| `CanPlay` | Boolean | `true` |
| `CanPause` | Boolean | `true` |
| `CanSeek` | Boolean | `true` |
| `CanControl` | Boolean | `true` |

### Metadata Map Construction

```rust
fn build_metadata(state: &DaemonState) -> zvariant::Dict {
    let mut dict = zvariant::Dict::new(zvariant::Signature::String, zvariant::Signature::Variant);

    if let Some(ref track) = state.current_track {
        dict.insert(
            "mpris:trackid".to_string(),
            zvariant::Value::new(format!("/org/mpris/MediaPlayer2/gtm/track/{}", track.id)),
        );
        dict.insert(
            "mpris:length".to_string(),
            zvariant::Value::new((track.duration * 1_000_000.0) as i64),
        );
        dict.insert("mpris:artUrl".to_string(), zvariant::Value::new(
            track.cover_path.as_ref().map(|p| format!("file://{}", p))
                .unwrap_or_default(),
        ));
        dict.insert("xesam:title".to_string(), zvariant::Value::new(&track.title));
        dict.insert("xesam:artist".to_string(), zvariant::Value::new(vec![&track.artist]));
        dict.insert("xesam:album".to_string(), zvariant::Value::new(&track.album));
        dict.insert("xesam:albumArtist".to_string(), zvariant::Value::new(vec![&track.artist]));
        dict.insert("xesam:trackNumber".to_string(), zvariant::Value::new(track.track_number.unwrap_or(0)));
        dict.insert("xesam:genre".to_string(), zvariant::Value::new(
            track.genre.split(", ").map(|s| s.to_string()).collect::<Vec<_>>(),
        ));
        dict.insert("xesam:year".to_string(), zvariant::Value::new(track.year.unwrap_or(0)));
        dict.insert("xesam:url".to_string(), zvariant::Value::new(format!("file://{}", track.path)));
    }

    dict
}
```

### Methods

| Method | Signature | Implementation |
|--------|-----------|---------------|
| `Next()` | `()` | Send `DaemonRequest::Next` |
| `Previous()` | `()` | Send `DaemonRequest::Prev` |
| `Pause()` | `()` | Send `DaemonRequest::PlayPause` |
| `PlayPause()` | `()` | Send `DaemonRequest::PlayPause` |
| `Stop()` | `()` | Send `DaemonRequest::Stop` |
| `Play()` | `()` | Send `DaemonRequest::PlayPause` |
| `Seek(x)` | `(Int64)` | Position = x / 1_000_000 + current_pos, send `DaemonRequest::Seek` |
| `SetPosition(track_id, pos)` | `(ObjectPath, Int64)` | Validate track_id matches current track, send `Seek(pos / 1_000_000)` |
| `OpenUri(uri)` | `(String)` | Strip `file://`, send `DaemonRequest::Play { path }` |

### Signals

| Signal | Trigger |
|--------|---------|
| `PropertiesChanged` | On any player property change (via event stream) |
| `Seeked(position)` | On PositionChanged event (rate-limited to max 10Hz) |

## Event → Signal Bridge

```
DaemonEvent                     → MPRIS Action
────────────────────────────────────────────────────────
PlaybackStarted                 Update Metadata dict
                                PlaybackStatus → "Playing"
                                Emit PropertiesChanged
                                Emit Seeked(0)

PlaybackPaused                  PlaybackStatus → "Paused"
                                Emit PropertiesChanged

PlaybackStopped                 PlaybackStatus → "Stopped"
                                Clear Metadata dict
                                Emit PropertiesChanged

TrackEnded                      (handled via PlaybackStarted of next track)

PositionChanged                 Update Position property
                                Emit Seeked (throttled to 100ms interval)

VolumeChanged                   Update Volume property
                                Emit PropertiesChanged

ShuffleChanged                  Update Shuffle property
                                Emit PropertiesChanged

RepeatModeChanged               Update LoopStatus property
                                Emit PropertiesChanged
```

## Connection Handling

```rust
impl MprisServer {
    pub async fn run(&mut self) -> Result<()> {
        // 1. Connect to session D-Bus
        let connection = zbus::ConnectionBuilder::session()?
            .name("org.mpris.MediaPlayer2.gtm")?
            .serve_at("/org/mpris/MediaPlayer2", self.root_interface())?
            .serve_at("/org/mpris/MediaPlayer2", self.player_interface())?
            .build()
            .await?;

        self.connection = Some(connection.clone());

        // 2. Subscribe to daemon events
        let mut event_rx = self.event_tx.subscribe();

        // 3. Process events in a loop
        loop {
            tokio::select! {
                event = event_rx.recv() => {
                    if let Ok(ev) = event {
                        self.handle_event(ev).await;
                    }
                }
                _ = tokio::signal::ctrl_c() => break,
            }
        }

        Ok(())
    }
}
```

## Usage

```
# In daemon Cargo.toml:
gtm-mpris = { path = "../gtm-mpris", optional = true }

# In daemon main.rs:
#[cfg(feature = "mpris")]
{
    let mut mpris = MprisServer::new(state.clone(), event_tx.clone(), daemon_tx);
    tokio::spawn(async move { mpris.run().await });
}
```

## File Structure

```
gtm-mpris/
├── Cargo.toml       # zbus (tokio), zvariant, gtm-core, thiserror, tracing
└── src/
    └── lib.rs       # MprisServer struct + zbus interface impls
```
