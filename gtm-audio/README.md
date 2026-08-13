# gtm-audio

Audio backend abstraction for gtm.

## Architecture

Defines the `AudioBackend` trait and provides implementations:

- **`SymphoniaBackend`** (primary): Pure Rust decoding via symphonia, audio output via cpal
- **`FfmpegBackend`** (optional, feature-gated): FFmpeg subprocess fallback for exotic formats

The `SymphoniaBackend` spawns a dedicated audio thread that decodes packets in a loop and writes
samples to the cpal output stream. Position tracking uses `AtomicF64`, and completion is signaled
via a channel.

## Dependencies

`gtm-core`, `symphonia`, `cpal`, `rubato` (resampling), `log`, `thiserror`
