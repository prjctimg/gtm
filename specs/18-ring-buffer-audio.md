# Spec 18 — Ring Buffer Audio Decoupling

## Requirement

> Currently the playback is stuttering, the initial playback may be smooth but it gets worse as the track progresses. The stuttering happens when the machine is under considerable usage.

## Root Cause

`SymphoniaSource::next()` (`gtm-audio/src/symphonia.rs:159-249`) performs synchronous disk I/O and codec decoding directly on the cpal audio callback thread. Under heavy system load:
1. Thread scheduling delays cause the audio callback to miss its deadline
2. Disk I/O stalls (page cache misses, HDD seeks at larger file offsets) directly stall audio
3. Per-sample EQ/reverb processing (`eq.rs:127-152,221-243`) adds computational overhead on the callback thread
4. Rodio's `periodic_access(5ms)` locks 5 mutexes on the callback thread (`player.rs:130-157`)
5. No prebuffering — sources decode on-demand

## Changes

### 18a. New file: `gtm-audio/src/ring_buffer.rs`

**`DecodeThread`** — runs on a dedicated `std::thread`:
- Owns the `SymphoniaSource` (or rodio `Decoder`) and performs all decoding
- Applies EQ processing (moves `EqSource` computation off the audio thread)
- Applies reverb processing (moves `ReverbSource` computation off the audio thread)
- Writes decoded PCM f32 samples into a lock-free SPSC ring buffer
- Handles seek requests by restarting the decoder at the new position
- Handles stop/drop by terminating the thread
- Communicates via atomic flags: `seek_request: AtomicU64` (u64::MAX = no request), `running: AtomicBool`

**`RingBufferSource`** — implements `rodio::Source<Item=f32> + Send`:
- Reads from the ring buffer (lock-free SPSC read side)
- Blocks (spins with yield) if buffer is empty — this is the safety net; should rarely happen with proper prebuffering
- Reports `total_duration()` from metadata passed at construction
- Prebuffer threshold: block `play()` until ring buffer has at least 0.5-1 second of samples

**Ring buffer design**:
- SPSC (Single Producer, Single Consumer) — no locks needed
- Producer: decode thread writes, updates write position with Release ordering
- Consumer: RingBufferSource reads, loads read position with Acquire ordering
- Capacity: 44100 * 2 channels * 4 bytes * 3 seconds ≈ 1MB (configurable)
- When full: producer drops samples (shouldn't happen with proper sizing)

### 18b. Modify `gtm-audio/src/mixer.rs`

**`load_active()` / `load_standby()`**:
- Instead of `Self::decode(path)` → `wrap_source(raw)` → `player.append(source)`
- New flow:
  1. Create `DecodeThread::new(path, eq_gains, reverb_config)` on a dedicated thread
  2. Wait for prebuffer threshold (decode thread signals when ready)
  3. Create `RingBufferSource` from the shared ring buffer
  4. `player.append(ring_buffer_source)`

**Seek handling**:
- `cmd_seek()`: Signal decode thread via atomic flag → decode thread restarts decoder → flushes ring buffer → resumes writing
- The `RingBufferSource` will naturally drain and refill from the new position

**EQ/Reverb changes**:
- `wrap_source()` no longer wraps with `EqSource`/`ReverbSource` — these are now in the decode thread
- `set_eq_preset()` / `set_reverb()` signal the decode thread to update its processing parameters via atomics/mutex

### 18c. New file: `gtm-audio/src/decode_thread.rs`

Full implementation of the decode loop:
```rust
fn decode_loop(
    path: String,
    ring_buffer: SharedRingBuffer,
    eq_gains: EqGains,
    reverb_config: Option<ReverbConfig>,
    seek_signal: Arc<AtomicU64>,
    running: Arc<AtomicBool>,
    ready_signal: Arc<AtomicBool>,
) {
    // Open file, create SymphoniaSource
    // Loop: decode sample → apply EQ → apply reverb → write to ring buffer
    // Check seek_signal each iteration → restart decoder if set
    // Check running flag → break if false
}
```

### 18d. `gtm-audio/src/lib.rs`
- Add `pub mod ring_buffer;`
- Add `pub mod decode_thread;`

### 18e. Modify `gtm-audio/src/eq.rs`
- `EqGains` already uses `Arc` for shared state — decode thread can read gains atomically
- `ReverbSource` processing moves to decode thread — extract the reverb logic into a standalone function usable from the decode loop

## Files Touched
- `gtm-audio/src/ring_buffer.rs` (new)
- `gtm-audio/src/decode_thread.rs` (new)
- `gtm-audio/src/mixer.rs` (major refactor of load/seek/stop)
- `gtm-audio/src/lib.rs` (module declarations)
- `gtm-audio/src/eq.rs` (extract reverb processing for reuse)
- `Cargo.toml` (no new dependencies needed — use std atomics)

## Verification
- No stuttering during normal playback on an idle system
- No stuttering during playback under heavy CPU load (stress-ng, compilation)
- EQ changes apply in real-time without audio glitches
- Seek works correctly (jumps to new position, no artifacts)
- Crossfade works correctly between tracks
- Volume changes don't cause pops/clicks
- Track end detection works (auto-advance triggers correctly)
