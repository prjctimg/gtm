# 02 — gtm-audio: Audio Backend Abstraction

## Purpose

Defines the `AudioBackend` trait and provides implementations using `symphonia` (pure Rust)
and optionally `ffmpeg-next` (subprocess). Audio output via `cpal` for cross-platform playback.

Depends on: `gtm-core`, `symphonia`, `cpal`, `rubato` (resampling), `log`, `thiserror`

Used by: `gtm-daemon`, `gtm-tui`

## AudioBackend Trait

```rust
use async_trait::async_trait;
use gtm_core::Result;

#[async_trait]
pub trait AudioBackend: Send {
    /// Load a file at path, optionally seeking to start_pos seconds.
    /// Decoding begins immediately but output may be paused.
    async fn load(&mut self, path: &str, start_pos: f64) -> Result<()>;

    /// Start/resume playback (audio thread begins writing to cpal).
    async fn play(&mut self) -> Result<()>;

    /// Pause without unloading (audio thread stops writing, decoding continues).
    async fn pause(&mut self) -> Result<()>;

    /// Stop and unload (audio thread joins, decoder dropped).
    async fn stop(&mut self) -> Result<()>;

    /// Seek to absolute position in seconds. May flush decoder.
    async fn seek(&mut self, position_secs: f64) -> Result<()>;

    /// Set volume 0-100 (applied as linear gain on PCM samples).
    async fn set_volume(&mut self, volume: u8) -> Result<()>;

    /// Poll for pending events (non-blocking). Returns None if nothing new.
    async fn poll(&mut self) -> Result<Option<AudioEvent>>;

    // Synchronous getters (backed by atomics, no lock needed)
    fn current_position(&self) -> f64;
    fn duration(&self) -> f64;
    fn is_playing(&self) -> bool;
    fn volume(&self) -> u8;
}

#[derive(Debug, Clone)]
pub enum AudioEvent {
    Position(f64),
    Duration(f64),
    Finished,
    Volume(u8),
    Error(String),
}
```

## AudioError

```rust
#[derive(Debug, thiserror::Error)]
pub enum AudioError {
    #[error("failed to open file: {0}")]
    OpenFailed(String),
    #[error("decode error: {0}")]
    DecodeError(String),
    #[error("output error: {0}")]
    OutputError(String),
    #[error("unsupported format: {0}")]
    UnsupportedFormat(String),
    #[error("seek error: {0}")]
    SeekError(String),
    #[error("ffmpeg not found")]
    FfmpegNotFound,
    #[error("ffmpeg error: {0}")]
    FfmpegError(String),
    #[error("resample error: {0}")]
    ResampleError(String),
}

impl From<AudioError> for gtm_core::CoreError {
    fn from(e: AudioError) -> Self {
        gtm_core::CoreError::Daemon(e.to_string())
    }
}

pub type Result<T> = std::result::Result<T, AudioError>;
```

## SymphoniaBackend (Primary)

```
┌────────────────────   SymphoniaBackend   ────────────────────┐
│                                                                │
│  struct SymphoniaBackend {                                     │
│      // Audio thread communication                             │
│      cmd_tx: Option<Sender<BackendCommand>>,                   │
│      event_rx: Receiver<AudioEvent>,                           │
│      thread_handle: Option<JoinHandle<()>>,                    │
│                                                                │
│      // Shared state (atomics behind Arc)                      │
│      position: Arc<AtomicF64>,                                 │
│      duration: Arc<AtomicF64>,                                 │
│      playing: Arc<AtomicBool>,                                 │
│      volume: Arc<AtomicU8>,                                    │
│  }                                                             │
│                                                                │
│  enum BackendCommand {                                         │
│      Load { path: String, start_pos: f64 },                    │
│      Play,                                                     │
│      Pause,                                                    │
│      Stop,                                                     │
│      Seek(f64),                                                │
│      SetVolume(u8),                                            │
│  }                                                             │
│                                                                │
│  Ring buffer (lock-free, single-producer single-consumer):     │
│    • Audio thread writes decoded samples                       │
│    • cpal callback reads samples                               │
│    • Size: 4096 frames (configurable)                          │
│    • Implementation: crossbeam::array::RingBuffer or custom    │
└────────────────────────────────────────────────────────────────┘
```

### Audio thread decode loop (pseudo-code)

```
fn audio_thread(
    cmd_rx: Receiver<BackendCommand>,
    event_tx: Sender<AudioEvent>,
    position: Arc<AtomicF64>,
    duration: Arc<AtomicF64>,
    playing: Arc<AtomicBool>,
    volume: Arc<AtomicU8>,
) {
    let mut decoder: Option<SymphoniaDecoder> = None;
    let mut stream: Option<cpal::Stream> = None;

    loop {
        // Non-blocking check for commands
        while let Ok(cmd) = cmd_rx.try_recv() {
            match cmd {
                Load { path, start_pos } => {
                    stop_old_stream(&mut stream);
                    decoder = Some(SymphoniaDecoder::open(&path, start_pos));
                    let dur = decoder.as_ref().unwrap().duration();
                    duration.store(dur, Ordering::Relaxed);
                    position.store(start_pos, Ordering::Relaxed);
                    let _ = event_tx.send(AudioEvent::Duration(dur));
                }
                Play => { playing.store(true, Ordering::Release); }
                Pause => { playing.store(false, Ordering::Release); }
                Stop => {
                    playing.store(false, Ordering::Release);
                    stop_old_stream(&mut stream);
                    decoder = None;
                    position.store(0.0, Ordering::Relaxed);
                    let _ = event_tx.send(AudioEvent::Position(0.0));
                }
                Seek(pos) => {
                    if let Some(ref mut dec) = decoder {
                        dec.seek(pos);
                        position.store(pos, Ordering::Relaxed);
                    }
                }
                SetVolume(v) => { volume.store(v, Ordering::Relaxed); }
            }
        }

        // Decode and output if playing
        if let Some(ref mut dec) = decoder {
            if playing.load(Ordering::Acquire) {
                match dec.decode_next_packet() {
                    Ok(samples) => {
                        let vol = volume.load(Ordering::Relaxed) as f64 / 100.0;
                        let scaled: Vec<f32> = samples.iter()
                            .map(|&s| s * vol as f32)
                            .collect();
                        // Write to cpal output stream via ring buffer
                        write_samples(&scaled);
                        let pos = dec.current_position();
                        position.store(pos, Ordering::Relaxed);
                        // Throttle position events to ~10Hz
                        maybe_send_position(&event_tx, pos);
                    }
                    Err(DecodeError::EndOfStream) => {
                        let _ = event_tx.send(AudioEvent::Finished);
                        playing.store(false, Ordering::Release);
                    }
                    Err(e) => {
                        let _ = event_tx.send(AudioEvent::Error(e.to_string()));
                    }
                }
            } else {
                std::thread::sleep(Duration::from_millis(10));
            }
        } else {
            // No decoder loaded — sleep to avoid busy-wait
            std::thread::sleep(Duration::from_millis(50));
        }
    }
}
```

### Crossfade Integration

Crossfade is implemented at the daemon level, not in SymphoniaBackend:

```
1. Daemon detects track will end in crossfade_duration_secs
2. Daemon calls backend.load(next_track_path, 0.0) to preload
3. Daemon starts a fade timer:
   - Over crossfade_duration_secs, interpolate volume from current → 0
   - Simultaneously ramp next track volume from 0 → desired
4. When current track finishes, daemon switches to next track instantly
```

SymphoniaBackend only exposes `set_volume()` — the daemon handles the crossfade
timing. Two SymphoniaBackend instances can run simultaneously during crossfade
if the daemon creates a second one temporarily. However, the simpler approach:
- During crossfade, daemon keeps current backend, spawns a temporary second backend
  for the next track, ramps volumes via set_volume(), then swaps and drops the old one.

## FfmpegBackend (Subprocess)

```
┌────────────────────   FfmpegBackend   ─────────────────────┐
│                                                              │
│  struct FfmpegBackend {                                      │
│      process: Option<Child>,                                 │
│      stream: Option<cpal::Stream>,                           │
│      position: Arc<AtomicF64>,                               │
│      duration: Arc<AtomicF64>,                               │
│      playing: Arc<AtomicBool>,                               │
│      volume: Arc<AtomicU8>,                                  │
│      progress_parser: ProgressParser,                        │
│  }                                                           │
│                                                              │
│  Strategy:                                                    │
│  1. Spawn: ffmpeg -i <file> -f wav -                         │
│  2. Read raw WAV from stdout, parse header (44 bytes)        │
│  3. Parse stderr for "time=HH:MM:SS.mmm" progress lines      │
│  4. Apply volume to PCM before writing to cpal stream        │
│  5. Kill subprocess on stop/drop                             │
└──────────────────────────────────────────────────────────────┘
```

### Volume Handling

```
sample_out = sample_in * (volume / 100.0)

Applied on audio thread read-side of ring buffer (SymphoniaBackend)
or before cpal stream write (FfmpegBackend).

Volume range: 0-100 (integer), mapped to linear 0.0-1.0 f64 gain.
No logarithmic/cubic curve needed — linear is adequate for a music player.
```

## SymphoniaBackend initialization details

```rust
impl SymphoniaBackend {
    /// Create a new backend. Does not open any file.
    pub fn new() -> Self;

    /// Get the supported cpal output configuration.
    /// Attempts to match symphonia's output spec (44100 Hz, f32, stereo).
    pub fn default_output_config() -> Option<cpal::SupportedStreamConfig>;
}
```

## File Structure

```
gtm-audio/
├── Cargo.toml
└── src/
    ├── lib.rs           # re-exports AudioBackend, AudioEvent, AudioError
    ├── backend.rs       # AudioBackend trait, AudioEvent enum
    ├── symphonia.rs     # SymphoniaBackend
    └── ffmpeg.rs        # FfmpegBackend (optional, feature-gated with #[cfg(feature = "ffmpeg")])
```

## Cargo.toml dependencies

```toml
[dependencies]
gtm-core = { path = "../gtm-core" }
symphonia = { version = "0.6", default-features = false, features = ["mp3", "flac", "ogg", "wav", "vorbis", "pcm"] }
cpal = "0.15"
rubato = "0.3"
log = "0.4"
thiserror = "2"
async-trait = "0.1"
crossbeam = "0.8"       # for ring buffer

[features]
ffmpeg = ["dep:ffmpeg-next"]
```
