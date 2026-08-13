# Spec 01: PulseAudio Audio Backend

## Summary

Add a feature-gated PulseAudio audio backend to `gtm-audio` that routes audio through Termux's PulseAudio server. Uses the `pulseaudio` crate (v0.3.1, pure Rust) for dual-stream crossfade playback. Rodio remains the default; PulseAudio is selected via the `pulseaudio` Cargo feature. At runtime, if PulseAudio init fails, the daemon falls back to rodio/cpal.

## Crate choice

**`pulseaudio` v0.3.1** (colinmarc) — pure Rust PulseAudio wire protocol implementation.

- No C `libpulse.so` linking required
- `Client::from_env()` auto-discovers Termux's PulseAudio socket
- `Client::create_playback_stream()` accepts a callback-driven `PlaybackSource`
- `PlaybackStreamParams.cvolume` provides per-stream volume for crossfade
- `Client` is `Clone + Send + Sync`

Rejected alternatives:
- `libpulse-binding` — requires linking against C `libpulse.so`, complicates cross-compilation
- `libpulse-simple-binding` — too limited for dual-stream crossfade

## Architecture

```
PulseAudioMixer
  ├── client: pulse::Client           (shared PA connection)
  ├── player_a: PulsePlayer            (stream A — active)
  ├── player_b: PulsePlayer            (stream B — standby)
  └── is_a_active: bool

PulsePlayer
  ├── shared: SharedRingBuffer         (SPSC ring buffer, producer = decode thread)
  ├── control: Arc<DecodeControl>      (decode thread control flags)
  ├── decode_handle: JoinHandle<()>    (decode thread — reuses existing DecodeThread)
  ├── playback_source: Arc<PlaybackState>  (shared state for PlaybackSource callback)
  ├── playback_handle: JoinHandle<()>  (PA event loop driving PlaybackSource)
  └── stream: Option<PlaybackStream>   (PA stream handle)
```

Data flow:

```
File → DecodeThread (symphonia + EQ/reverb) → SharedRingBuffer → PlaybackSource callback → PulseAudio Server → AAudio/OpenSL ES
```

The `DecodeThread` (`gtm-audio/src/decode_thread.rs`) and `SharedRingBuffer` (`gtm-audio/src/ring_buffer.rs`) are reused unchanged. The only new code replaces the rodio `Player` output endpoint with a PulseAudio `PlaybackSource` callback.

## PlaybackSource implementation

The `pulseaudio::PlaybackSource` trait requires:

```rust
fn poll_read(self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &mut [u8]) -> Poll<usize>;
```

### PlaybackState (shared between callback and mixer)

```rust
struct PlaybackState {
    shared: SharedRingBuffer,          // reads f32 samples from decode thread
    volume: AtomicU32,                 // volume as fixed-point (1.0 = 0x3F800000 bits)
    finished: AtomicBool,             // set when ring buffer is drained
    stop: AtomicBool,                 // signal callback to return 0 (EOF)
}
```

### Callback logic

```rust
impl PlaybackSource for PlaybackState {
    fn poll_read(self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &mut [u8]) -> Poll<usize> {
        // If stopped, return 0 bytes (= EOF, stream terminates)
        if self.stop.load(Ordering::Acquire) {
            return Poll::Ready(0);
        }

        // Try to fill buf with interleaved stereo i16 samples
        let mut written = 0;
        let vol_bits = self.volume.load(Ordering::Relaxed);
        let vol = f32::from_bits(vol_bits);

        while written + 4 <= buf.len() {  // 4 bytes = 1 stereo frame (2 x i16)
            let left = match self.shared.pop() {
                Some(s) => s,
                None => {
                    if self.shared.is_finished() {
                        self.finished.store(true, Ordering::Release);
                        break;
                    }
                    // Buffer empty but decode still running — return what we have
                    // or park if nothing written yet
                    if written == 0 {
                        cx.waker().wake_by_ref();
                        return Poll::Pending;
                    }
                    break;
                }
            };
            let right = match self.shared.pop() {
                Some(s) => s,
                None => left,  // mono fallback
            };

            // Apply volume and convert f32 → i16
            let l_i16 = (left * vol * 32767.0).clamp(-32768.0, 32767.0) as i16;
            let r_i16 = (right * vol * 32767.0).clamp(-32768.0, 32767.0) as i16;

            buf[written..written+2].copy_from_slice(&l_i16.to_le_bytes());
            buf[written+2..written+4].copy_from_slice(&r_i16.to_le_bytes());
            written += 4;
        }

        if written > 0 {
            Poll::Ready(written)
        } else {
            Poll::Pending
        }
    }
}
```

**Note**: The `poll_read` callback runs on the PulseAudio event loop thread (async context). The `SharedRingBuffer::pop()` is lock-free SPSC, safe to call from the consumer thread.

## Stream configuration

```rust
use pulseaudio::protocol::command::{PlaybackStreamParams, SampleSpec, ChannelMap};

fn make_params(volume: u32) -> PlaybackStreamParams {
    PlaybackStreamParams {
        sample_spec: SampleSpec {
            format: SampleFormat::S16LE,
            rate: 44100,
            channels: 2,
        },
        channel_map: ChannelMap::sterEO,
        cvolume: Some(ChannelVolume::new(2, volume)),
        buffer_attr: BufferAttr {
            maxlength: u32::MAX,
            tlength: 4410 * 4,   // ~100ms at 44100 stereo i16
            prebuf: u32::MAX,
            minreq: u32::MAX,
            fragsize: u32::MAX,
        },
        sink_name: None,  // default sink
        sink_index: None,
        sync_id: 0,
        props: Props::default(),
        formats: Vec::new(),
        flags: StreamFlags::empty(),
    }
}
```

**Buffer size**: `tlength = 4410 * 4` (100ms) balances latency and stability on Android. May need tuning.

## PulsePlayer lifecycle

### open()

```rust
impl PulsePlayer {
    fn open(client: &pulse::Client, shared: SharedRingBuffer, control: Arc<DecodeControl>) -> Result<Self> {
        let state = Arc::new(PlaybackState {
            shared,
            volume: AtomicU32::new(0x3F800000),  // 1.0f32.to_bits()
            finished: AtomicBool::new(false),
            stop: AtomicBool::new(false),
        });

        let params = make_params(0x10000);  // PA volume: 1.0 (PA uses u32 fixed-point)
        let stream = client.create_playback_stream(params, state.clone())?;

        Ok(Self {
            shared: state.shared.clone(),
            control,
            decode_handle: None,
            playback_state: state,
            stream: Some(stream),
        })
    }
}
```

### load(path, start_pos)

```rust
fn load(&mut self, path: &str, start_pos: f64, eq_gains: &EqGains, ...) -> AudioResult<()> {
    // 1. Stop old decode thread
    if let Some(ctrl) = &self.control {
        ctrl.signal_stop();
    }
    if let Some(h) = self.decode_handle.take() {
        let _ = h.join();
    }

    // 2. Reset ring buffer and playback state
    self.shared.flush();
    self.playback_state.finished.store(false, Ordering::Release);
    self.playback_state.stop.store(false, Ordering::Release);

    // 3. Probe duration
    let dur = probe_duration(path)?;

    // 4. Start new decode thread
    let (control, source, handle) = DecodeThread::start(path, ...)?;
    self.control = Some(control);
    self.decode_handle = Some(handle);

    // 5. Stream is already connected — PlaybackSource reads from ring buffer
    Ok(())
}
```

### close()

```rust
fn close(&mut self) {
    self.playback_state.stop.store(true, Ordering::Release);
    if let Some(ctrl) = &self.control {
        ctrl.signal_stop();
    }
    if let Some(h) = self.decode_handle.take() {
        let _ = h.join();
    }
    self.stream = None;
}
```

## Crossfade algorithm

Same as `AudioMixer::step_crossfade()` (`gtm-audio/src/mixer.rs:588-635`), but volume is applied via `PlaybackState.volume` atomic instead of `rodio::Player::set_volume()`.

```rust
fn step_crossfade(&mut self) -> bool {
    let elapsed = self.crossfade_start.unwrap().elapsed().as_secs_f64();
    let progress = (elapsed / self.crossfade_duration).min(1.0);
    let eased_out = Self::ease_out(progress, self.crossfade_easing);
    let eased_in = Self::ease_in(progress, self.crossfade_easing);
    let vol = self.volume.load(Ordering::SeqCst) as f64 / 100.0;

    let vol_a = if self.is_a_active { eased_out * vol } else { eased_in * vol };
    let vol_b = if self.is_a_active { eased_in * vol } else { eased_out * vol };

    // Convert to f32 bits for atomic storage
    self.player_a.playback_state.volume.store((vol_a as f32).to_bits(), Ordering::Relaxed);
    self.player_b.playback_state.volume.store((vol_b as f32).to_bits(), Ordering::Relaxed);

    if progress >= 1.0 {
        // Swap active/standby — same logic as AudioMixer
        self.player_a.close();
        self.is_a_active = !self.is_a_active;
        // ... reset crossfade state
        true
    } else {
        false
    }
}
```

## PulseAudioMixer trait implementation

Implements `Mixer` trait (`gtm-audio/src/mixer.rs:21-47`) by delegating to the dual `PulsePlayer` instances. Same method signatures as `AudioMixer`.

Key differences from `AudioMixer`:
- No `rodio::DeviceSinkBuilder` — uses `pulse::Client::from_env()`
- No `rodio::Player` — uses `PulsePlayer` with `PlaybackSource` callback
- Volume stored in `PlaybackState.volume` atomic (not `rodio::Player::set_volume()`)
- Crossfade uses same easing functions (`ease_in`/`ease_out` at `mixer.rs:564-586`)

## Error handling and fallback

```rust
// In gtmd/src/daemon.rs
let mixer: Box<dyn Mixer> = if config.test_mode {
    Box::new(NullMixer::new())
} else {
    match config.audio_backend {
        AudioBackendKind::Rodio => Box::new(AudioMixer::new()?),
        #[cfg(feature = "pulseaudio")]
        AudioBackendKind::PulseAudio => {
            match PulseAudioMixer::new() {
                Ok(m) => Box::new(m),
                Err(e) => {
                    log::warn!("PulseAudio unavailable ({e}), falling back to rodio");
                    Box::new(AudioMixer::new()?)
                }
            }
        }
    }
};
```

## Files to modify

| File | Change |
|---|---|
| `gtm-audio/Cargo.toml` | Add `pulseaudio` feature, add `pulse` dep |
| `gtm-audio/src/pulse_mixer.rs` | **New file** — `PulseAudioMixer`, `PulsePlayer`, `PlaybackState` |
| `gtm-audio/src/lib.rs` | Add conditional `pub mod pulse_mixer` |
| `gtmd/Cargo.toml` | Add `pulseaudio` feature passthrough |
| `gtmd/src/config.rs:12-21` | Expand `AudioBackendKind` enum with `PulseAudio` variant |
| `gtmd/src/daemon.rs:105-112` | Backend selection with fallback logic |

## Testing

1. **Unit test**: `PulseAudioMixer` with mock `SharedRingBuffer` — verify volume scaling, crossfade math
2. **Integration test**: Requires running PulseAudio server. Skip on CI (pulse not installed). Test on Termux device.
3. **Fallback test**: Set `PULSE_SERVER=/dev/null` to force `Client::from_env()` failure, verify rodio fallback
4. **Existing tests**: `cargo test --workspace` with default features (rodio) must continue passing
