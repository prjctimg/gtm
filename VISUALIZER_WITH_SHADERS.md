# Visualizer with shaders

Exploration of moving `gtm`'s audio visualizer off glyph cells onto a GPU
pipeline, what that costs, and what has to be true before it is worth doing.
No code here; the current renderer stays until a shader path can fall back to
it on every failure path.

## Where the visualizer is today

Audio crosses the daemon boundary already, so a shader backend needs nothing new
from `gtmd`:

```
decode -> SpectrumAnalyzer (rustfft, log-spaced, 30 Hz..16 kHz) -> mixer
       -> DaemonEvent::SpectrumChanged  (~60 Hz)
       -> DaemonEvent::WaveformChanged  (~30 Hz, interleaved L/R)
       -> DaemonState { audio_levels, wave_samples, wave_stereo }
       -> AudioVisualizer::tick(..) -> AudioVisualizer::render(..) -> Lines
```

- `gtm/src/visualizer.rs:409` — `AudioVisualizer` holds the whole model: per
  column bars, `BAND_COUNT` (10) smoothed log bands, peak physics, a flame heat
  field, a waveform ring, a frame counter and a resting flag.
- `gtm/src/visualizer.rs:479` — `tick(is_playing, width, height, audio_levels,
  wave_samples, wave_stereo)` is the only place audio enters.
- `gtm/src/visualizer.rs:726` — `render(&self, area, theme) -> Option<Lines>` is
  the only place pixels leave. Twelve presets (`VisualizerPreset`, line 26), all
  of them cells: braille dot grids, block glyphs, per-cell `fg` colours.
- `gtm/src/ui/chrome.rs:523` — the one call site (`tick`, then `render`).
  Zen's visualizer surface and the library pane both go through it.
- Gating: `ExtensionId::Visualizer` (`gtm/src/extensions.rs:34`) zeroes the two
  IPC streams on the client when the user turns the extension off, so the pipe
  goes idle rather than busy.

The app does have one path to real pixels: cover art through `ratatui-image`
(`gtm/src/app/mod.rs:38`), which speaks Kitty/sixel and is gated by
`no_image_protocol()`. That matters below — it is the only proven way this
binary has ever put an image on a screen.

## The problem a shader creates

A shader is a function from numbers to a pixel image. The terminal is a grid of
cells with one glyph and one or two colours each. `Lines` cannot carry an image,
so a shader backend has to choose how pixels reach the screen, and every choice
is a different product:

| Route | Mechanism | Cost | Limit |
|---|---|---|---|
| **A. Cell raster** | Render to texture, copy back, sample two pixels per cell, emit `▀` with per-cell `fg`/`bg` | ~`w*h*2` samples per frame on the CPU; truecolor only | Highest, still sharp enough for bars and gradients |
| **B. Image protocol** | Encode the frame, hand it to the terminal as Kitty/sixel/iTerm2 inline image (`ratatui-image`) | Per-frame image encode (raw RGB payload in Kitty; PNG elsewhere) | Requires a terminal that speaks the protocol |
| **C. Own window** | Give wgpu a real surface (winit) and draw beside the TUI | Native quality | Breaks "reattach from any terminal"; a detached window is a different app |

Route C is out. Route B is the honest way to get native quality, and the crate
is already a dependency with the detection already written. Route A is the
universal path and is the one to ship first, because it works everywhere the
cell renderer works today — which is the whole point of a fallback.

## Pipeline shape

- **Device**: `wgpu::Instance` with no surface at all. Headless needs a Vulkan,
  Metal or GLES driver, not a window system, so the TUI keeps owning the
  terminal. No adapter (containers, CI, locked-down machines) must be an
  ordinary "fall back to cells", never an error.
- **Targets**: an `Rgba8Unorm` texture sized to the pane in device pixels, plus a
  storage texture ping-pong for the feedback presets (Retro, Flame are history
  dependent; the CPU flame field is the reference for what that should look
  like).
- **Passes**: one compute pass writing into the storage texture (bars, wave,
  feedback) and one fullscreen-triangle render pass for the colour mapping. A
  pure "let the fragment shader index the band buffer" version is enough to
  start.
- **Buffers**: one uniform block (`time`, `dt`, `playing`, pane size, and the
  theme colours) and one storage block holding the frame's audio — the same
  `&[f32]` slices `tick` already receives, so the smoothing/decay model in
  `tick` can be shared and only the rasteriser changes.
- **Threading**: the TUI loop is single-threaded and a lost device, a shader
  compile or a readback stall would freeze the UI. Render on a worker thread
  with a double-buffered frame mailbox, drop frames instead of blocking, and
  let `wgpu::DeviceLost` fall back to cells.
- **Pacing**: audio arrives at 30–60 Hz while the TUI redraws on its own
  schedule. Interpolate (the CPU model already smooths) rather than assuming
  one sample per frame, or the motion stutters.
- **Theme**: reactive theming rewrites `AppTheme` colours on every track. They
  go into the uniform block each frame so shaders follow the reactive palette
  instead of hardcoding a gradient.

## API for visualizers we do not write

The current surface is ratatui-bound: `render` returns `Lines`, so anyone
writing a visualizer today writes against the cell model. A shader backend
needs a frame type that can carry either, and a registry so a visualizer is a
value rather than a branch in `render`:

```rust
pub struct AudioFrame<'a> {
    pub playing: bool,
    pub dt: f64,
    pub bands: &'a [f32],      // smoothed, log-spaced, as `tick` sees them
    pub wave: &'a [f32],       // decimated interleaved L/R
    pub stereo: bool,
    pub size: (u16, u16),      // pane in cells
    pub theme: &'a AppTheme,
}

pub enum Frame {
    Cells(Lines<'static>),     // today's renderer
    Pixels(image::RgbaImage),  // route A/B result
    Shader(&'static str),      // WGSL entry point, compiled once
}

pub trait Visualizer: Send + 'static {
    fn update(&mut self, frame: &AudioFrame);
    fn draw(&mut self, frame: &AudioFrame) -> Frame;
}
```

A `VisualizerRegistry` with `register(name, factory)` sits behind the existing
`ExtensionId::Visualizer` gate: the extension already decides whether spectrum
data moves at all, so a third-party visualizer should inherit that decision
rather than add a second one. Shader presets ship as `.wgsl` files embedded with
`include_str!`, so adding one is a file plus a registry line — no Rust.

Making `AudioVisualizer` a trait impl is the useful first step regardless of
whether the GPU work lands: it removes the twelve-way match in `render`, and it
is the only way "write your own visualizer" stops meaning "fork `render`".

## Dependencies

Everything below is client-side (`gtm`). `gtmd` gains nothing: the audio
already crosses IPC.

| Crate | Version | Why | Notes |
|---|---|---|---|
| `wgpu` | 30 | Renderer and compute; the only real option | Pulls `naga`, `wgpu-core`, `wgpu-hal`. Big compile-time and binary cost |
| `pollster` | 1.x | `block_on` for adapter/device init and the per-frame submit on the worker thread | Only needed where the render thread is not already inside a runtime |
| `bytemuck` | 1.25 | `Pod`/`Zeroable` derives for the uniform and storage buffers | Already in the tree transitively via wgpu; needed as a direct dep to derive |
| `image` | 0.25 | Readback container, and PNG encode for route B | Already a dependency |
| `ratatui-image` | 11 | Route B blit; `Picker`/`StatefulImage` already used for covers | Already a dependency |
| `winit` | 0.30 | Only for route C | Do not add |

Feature-gate all of it: `shader-viz = ["dep:wgpu", "dep:pollster", "dep:bytemuck"]`,
off by default. CI and the release build then never compile it, and
`visualizer.rs` keeps the cell renderer as the default branch.

## Risks

- **Size and build time.** `wgpu` is the single heaviest dependency this crate
  could take. Behind a feature it costs nothing to anyone who does not opt in;
  as a default dependency it would slow every build and every release job.
- **No GPU is the common case, not the exception.** CI containers, servers,
  locked-down desktops. "No adapter" must be a silent fallback with the cell
  renderer already on screen, never a blank panel or a panic.
- **Per-frame readback.** Route A copies the texture back every frame. Keep the
  pane small, or only re-render when the audio model actually changed (the
  `resting` flag and frame counter in `AudioVisualizer` already tell you when
  nothing moved).
- **Protocol support.** Route B inherits every quirk ratatui-image already has
  (kitty vs sixel vs iTerm2, `no_image_protocol()`).
- **Licensing.** WGSL presets authored here are GPL-3.0 with the app; wgpu and
  naga are MIT/Apache-2.0, which composes fine.

## Stages

1. `AudioFrame` + `Visualizer` trait + registry; `AudioVisualizer` becomes one
   impl behind the existing extension gate. No behaviour change, no GPU.
2. Port one preset (Flame, since it is the one that genuinely needs a history
   buffer) to `Frame::Pixels` through the CPU, validating the API and the
   route-A sampling.
3. `wgpu` backend behind `shader-viz`: storage buffer of bands and waveform,
   compute + render passes, worker thread, cell raster out. Cells remain the
   fallback for missing adapter, lost device, non-truecolor terminals.
4. Route B: when the image protocol is available, blit the frame through
   `ratatui-image` instead of sampling it into cells.
5. Publish the registry as the extension point, with WGSL presets included.