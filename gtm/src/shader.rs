// Copyright (c) 2026
// Author: prjctimg <prjctimg@outlook.com>
// Headless wgpu backend for the shader presets
//
// This is free software released under the GPL-3.0 license.

//! A GPU visualizer that keeps the terminal.
//!
//! No surface, no window, no second app: the device is created headless and
//! the frame is read back into an [`RgbaImage`] for the cell rasteriser. A
//! terminal owns the screen and keeps owning it.
//!
//! Every failure path lands back on the cell renderer rather than raising. No
//! adapter (a CI container, a server, a locked-down desktop) is the common case
//! and not an error; a lost device mid-session is the same. The caller asks
//! [`Shader::draw`] for a frame and gets `None` when the GPU cannot serve one.

use std::sync::mpsc::{Receiver, SyncSender, TryRecvError, sync_channel};

use bytemuck::{Pod, Zeroable};
use image::RgbaImage;
use wgpu::util::DeviceExt;

/// Rows per second the heat climbs. Scaled by the frame delta, so a
/// frame-rate change does not change how fast the plume rises.
const RISE: f32 = 90.0;

/// Bands the uniform carries, packed four to a `vec4` because the uniform
/// address space requires a 16-byte stride.
const BANDS: usize = 64;

/// Waveform samples, interleaved L/R, packed the same way. A multiple of four so
/// the packing never has a ragged tail.
const WAVE: usize = 64;

/// Device pixels per cell. Two matches the `▀` rasteriser's two vertical samples
/// per cell; more would be read back and thrown away.
const SCALE: u32 = 2;

/// How many finished frames may wait for the TUI before the mailbox is full.
const MAILBOX: usize = 2;

/// How long the render thread waits for a submitted frame before treating the
/// device as gone and letting the caller fall back.
const MAP_TIMEOUT: std::time::Duration = std::time::Duration::from_millis(250);

/// The presets that render on the GPU, and the WGSL behind each.
///
/// Only presets whose look is an *image* live here. A preset whose identity is a
/// particular terminal glyph — the block ramp in `Blocks`, the peak marker in
/// `Spectrum`, the caps in `ClassicPeak` — cannot survive the round trip: the
/// frame is read back as pixels and printed through `▀` half-blocks, so a bar
/// drawn as `▅` would come back as a stack of block pairs with no memory of
/// being a `▅`. Those stay on the cell renderer, where the glyph is the point.
///
/// A preset is absent from this list rather than mapped to a fallback, because
/// an absent preset and a broken one must not look the same.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Preset {
    Flame,
    Gradient,
    Mirror,
    Wave,
    Stereo,
}

impl Preset {
    /// Every GPU preset, for warming pipelines.
    pub const ALL: [Preset; 5] = [
        Preset::Flame,
        Preset::Gradient,
        Preset::Mirror,
        Preset::Wave,
        Preset::Stereo,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Preset::Flame => "flame",
            Preset::Gradient => "gradient",
            Preset::Mirror => "mirror",
            Preset::Wave => "wave",
            Preset::Stereo => "stereo",
        }
    }

    /// The WGSL module for this preset: the shared block, then its own fragment
    /// stage.
    ///
    /// WGSL has no include, so the concatenation happens here. One definition of
    /// the uniform is worth more than five copies of it that can drift apart
    /// from `Uniforms` without anything noticing until the pixels go wrong.
    pub fn source(self) -> String {
        match self {
            Preset::Flame => include_str!("shaders/flame.wgsl").to_string(),
            Preset::Gradient => concat!(
                include_str!("shaders/common.wgsl.inc"),
                include_str!("shaders/gradient.wgsl")
            )
            .to_string(),
            Preset::Mirror => concat!(
                include_str!("shaders/common.wgsl.inc"),
                include_str!("shaders/mirror.wgsl")
            )
            .to_string(),
            Preset::Wave => concat!(
                include_str!("shaders/common.wgsl.inc"),
                include_str!("shaders/wave.wgsl")
            )
            .to_string(),
            Preset::Stereo => concat!(
                include_str!("shaders/common.wgsl.inc"),
                include_str!("shaders/stereo.wgsl")
            )
            .to_string(),
        }
    }
}

/// One frame's worth of work, handed to the render thread.
///
/// `Copy` because one of these crosses the channel every frame and the mailbox
/// is the only thing standing between the TUI thread and a stalled driver.
#[derive(Clone, Copy, Debug)]
pub struct Draw {
    /// Which shader renders this frame. The device is shared; the pipeline is
    /// not, so this is what selects one.
    pub preset: Preset,
    pub playing: bool,
    /// Seconds since the previous frame, so cooling and rise are frame-rate
    /// independent.
    pub dt: f32,
    /// Log-spaced band levels, already smoothed by the shared audio model.
    pub bands: [f32; BANDS],
    pub band_count: u32,
    /// Decimated waveform for the oscilloscope, already interleaved L/R and
    /// normalised to -1..1. Empty when the stream is silent.
    pub wave: [f32; WAVE],
    pub wave_count: u32,
    /// Left and right levels and their slow-falling peaks, for the meters.
    pub stereo: [f32; 4],
    /// Pane size in cells.
    pub cells: (u16, u16),
    /// Theme ramp ends, 0–255, so the shader follows the reactive palette
    /// rather than a fixed gradient.
    pub cool: [u8; 3],
    pub warm: [u8; 3],
    pub peak: [u8; 3],
}

/// The uniform block, laid out to match `flame.wgsl`'s `Uniforms`.
///
/// Every field is a scalar or a padded `vec4`, so the Rust offsets and the WGSL
/// offsets agree without a manual offset table on either side.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
struct Uniforms {
    time: f32,
    dt: f32,
    playing: f32,
    rise: f32,
    pad: [f32; 4],
    size: [u32; 2],
    band_count: u32,
    band_width: u32,
    bands: [[f32; 4]; 16],
    /// `wave_count` then three words of padding: the array below is a fixed
    /// size, so the count cannot sit inside it.
    wave_count: u32,
    pad2: [u32; 3],
    wave: [[f32; 4]; WAVE / 4],
    /// Left/right level and left/right peak.
    stereo: [f32; 4],
    cool: [f32; 4],
    warm: [f32; 4],
    peak: [f32; 4],
}

impl Uniforms {
    /// `None` on a pane too small for two pixels per cell; the caller keeps the
    /// cell renderer rather than allocating a one-pixel target.
    fn from(job: &Draw, w: u32, h: u32, time: f32) -> Option<Self> {
        if w == 0 || h == 0 {
            return None;
        }
        Some(Self {
            time,
            dt: job.dt.max(0.0),
            playing: f32::from(u8::from(job.playing)),
            rise: RISE,
            pad: [0.0; 4],
            size: [w, h],
            band_count: job.band_count,
            band_width: (w / job.band_count.max(1)).max(1),
            bands: std::array::from_fn(|i| pack(&job.bands, i)),
            // Samples, not floats: one `vec4` carries two of them, so the
            // ceiling is half the array. Capped before the rounding, not after —
            // capping after let a full buffer report twice what it held and the
            // sampler read past the end.
            wave_count: ((job.wave_count as usize).min(WAVE / 2) & !1) as u32,
            pad2: [0; 3],
            wave: std::array::from_fn(|i| pack(&job.wave, i)),
            stereo: job.stereo,
            cool: pad(job.cool),
            warm: pad(job.warm),
            peak: pad(job.peak),
        })
    }
}

/// Four consecutive samples as a `vec4`, zero-padded past the end of the slice.
///
/// The uniform address space wants a 16-byte stride per array element, so an
/// `array<f32, N>` is not expressible; this is the packing that replaces it.
fn pack(src: &[f32], i: usize) -> [f32; 4] {
    std::array::from_fn(|k| src.get(i * 4 + k).copied().unwrap_or(0.0))
}

fn pad(c: [u8; 3]) -> [f32; 4] {
    [
        f32::from(c[0]) / 255.0,
        f32::from(c[1]) / 255.0,
        f32::from(c[2]) / 255.0,
        0.0,
    ]
}

/// Two targets that swap roles each frame, so one can be sampled while the
/// other is written. Sampling and writing the same texture in one pass is
/// undefined, and for a feedback effect the two frames are the point.
///
/// Holds one bind group per target rather than a single one that is rebuilt:
/// a bind group names the view it was made from, so alternating between two of
/// them is what makes the swap free. Rebuilding each frame would allocate a
/// bind group sixty times a second for no reason.
struct Targets {
    a: (wgpu::Texture, wgpu::TextureView),
    b: (wgpu::Texture, wgpu::TextureView),
    /// Bind group that samples `a`, and one that samples `b`.
    binds: [wgpu::BindGroup; 2],
    /// Index of the target written this frame.
    dst: usize,
}

impl Targets {
    /// The view written this frame, and the bind group that samples the other.
    fn pass(&self) -> (&wgpu::Texture, &wgpu::TextureView, &wgpu::BindGroup) {
        let src = 1 - self.dst;
        let (tex, view) = if self.dst == 0 { &self.a } else { &self.b };
        (tex, view, &self.binds[src])
    }
}

/// The GPU side, living entirely on the render thread.
struct Worker {
    device: wgpu::Device,
    queue: wgpu::Queue,
    layout: wgpu::BindGroupLayout,
    uniforms: wgpu::Buffer,
    targets: Option<Targets>,
    /// One pipeline per preset, built on first use.
    ///
    /// Compiled lazily because a session that never reaches a preset should
    /// never pay for it, and kept because compilation is the expensive part:
    /// building this on every switch would stutter the frame the user is
    /// looking at.
    pipelines: [Option<wgpu::RenderPipeline>; Preset::ALL.len()],
    done: SyncSender<RgbaImage>,
    clock: f32,
}

impl Worker {
    /// The pipeline for `preset`, compiling it on first use.
    ///
    /// Returned by clone rather than by reference: compiling needs `&mut self`,
    /// and a borrow of the cache would then overlap every other use of the
    /// worker for the rest of the frame. A `RenderPipeline` is a handle, so the
    /// clone costs a refcount rather than a pipeline.
    ///
    /// `None` when the module is rejected: an invalid shader falls back to the
    /// cells rather than taking the whole pane down, which is what a panic in
    /// the render thread would do.
    fn pipeline(&mut self, preset: Preset) -> Option<wgpu::RenderPipeline> {
        let slot = Preset::ALL.iter().position(|p| *p == preset)?;
        if self.pipelines[slot].is_none() {
            self.pipelines[slot] = Some(self.build(preset));
        }
        self.pipelines[slot].clone()
    }

    fn build(&self, preset: Preset) -> wgpu::RenderPipeline {
        let module = self
            .device
            .create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some(preset.name()),
                source: wgpu::ShaderSource::Wgsl(preset.source().into()),
            });
        let layout = self
            .device
            .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some(preset.name()),
                bind_group_layouts: &[Some(&self.layout)],
                immediate_size: 0,
            });
        self.device
            .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(preset.name()),
                layout: Some(&layout),
                vertex: wgpu::VertexState {
                    module: &module,
                    entry_point: Some("vs"),
                    compilation_options: Default::default(),
                    buffers: &[],
                },
                fragment: Some(wgpu::FragmentState {
                    module: &module,
                    entry_point: Some("fs"),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: wgpu::TextureFormat::Rgba8Unorm,
                        // The flame writes a heat value rather than a colour and
                        // reads it back as next frame's history, so there is
                        // nothing to blend; the rest write final colour, which
                        // still does not need blending into an empty target.
                        blend: None,
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                primitive: wgpu::PrimitiveState::default(),
                depth_stencil: None,
                multisample: wgpu::MultisampleState::default(),
                multiview_mask: None,
                cache: None,
            })
    }
}

impl Worker {
    /// Run until the job channel closes. Returns false when the device can no
    /// longer serve a frame, which ends the thread.
    fn run(mut self, jobs: Receiver<Draw>) {
        while let Ok(job) = jobs.recv() {
            if !self.draw(job) {
                break;
            }
        }
    }

    /// One frame. `false` means the device is gone and the caller must fall
    /// back for the rest of the session.
    fn draw(&mut self, job: Draw) -> bool {
        let (w, h) = (
            u32::from(job.cells.0).saturating_mul(SCALE),
            u32::from(job.cells.1).saturating_mul(SCALE),
        );
        let Some(uniform) = Uniforms::from(&job, w, h, self.clock) else {
            return true;
        };
        self.clock += job.dt;
        self.resize(w, h);
        // Pipeline first: resolving it may compile the module, which needs
        // `&mut self`, so it cannot overlap the borrow of the targets.
        let Some(pipeline) = self.pipeline(job.preset) else {
            return true;
        };
        let Some(targets) = &self.targets else {
            return true;
        };
        self.queue
            .write_buffer(&self.uniforms, 0, bytemuck::bytes_of(&uniform));

        let (tex, view, bind) = targets.pass();
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("flame"),
            });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("flame"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&pipeline);
            // Samples the *other* target, so the pass reads last frame's
            // history and writes this frame's without ever touching both.
            pass.set_bind_group(0, bind, &[]);
            pass.draw(0..3, 0..1);
            // The pass ends on drop; wgpu 30 has no explicit `end`.
        }

        // Row alignment is 256 bytes for a copy out of a texture, so the
        // readback is padded even though the image is not.
        let row = w * 4;
        let padded =
            row.div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT) * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
        let buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("readback"),
            size: u64::from(padded * h),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: tex,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(padded),
                    rows_per_image: Some(h),
                },
            },
            wgpu::Extent3d {
                width: w,
                height: h,
                depth_or_array_layers: 1,
            },
        );
        self.queue.submit(Some(encoder.finish()));

        // Mapping is asynchronous; poll until it lands. This blocks only the
        // render thread, which is the whole reason there is one.
        let slice = buffer.slice(..);
        let (tx, rx) = std::sync::mpsc::sync_channel(0);
        slice.map_async(wgpu::MapMode::Read, move |r| {
            let _ = tx.send(r.is_ok());
        });
        // Bounded: a device that never answers ends this frame as a failure and
        // the caller falls back, rather than parking the render thread forever.
        if self
            .device
            .poll(wgpu::PollType::Wait {
                submission_index: None,
                timeout: Some(MAP_TIMEOUT),
            })
            .is_err()
        {
            return false;
        }
        if rx.recv() != Ok(true) {
            return false;
        }

        let mut img = RgbaImage::new(w, h);
        {
            let Ok(view) = slice.get_mapped_range() else {
                return false;
            };
            for y in 0..h as usize {
                let from = &view[y * padded as usize..y * padded as usize + row as usize];
                let start = y * w as usize;
                img.as_mut()[start..start + w as usize].copy_from_slice(bytemuck::cast_slice(from));
            }
        }
        buffer.unmap();
        if let Some(t) = &mut self.targets {
            // Next frame writes what this one sampled, so the two alternate and
            // the effect always reads the frame before it.
            t.dst = 1 - t.dst;
        }
        // A full mailbox means the TUI is not keeping up; dropping the frame is
        // the correct response and is not an error.
        let _ = self.done.try_send(img);
        true
    }

    /// Rebuild the targets when the pane changes size, and rebind, because a
    /// bind group names the view it was made from.
    fn resize(&mut self, w: u32, h: u32) {
        let current = self
            .targets
            .as_ref()
            .and_then(|t| t.a.0.size().width.eq(&w).then(|| t.a.0.size().height))
            .is_some_and(|hh| hh == h);
        if current {
            return;
        }
        let mk = |label: &str| {
            let texture = self.device.create_texture(&wgpu::TextureDescriptor {
                label: Some(label),
                size: wgpu::Extent3d {
                    width: w,
                    height: h,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8Unorm,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                    | wgpu::TextureUsages::TEXTURE_BINDING
                    | wgpu::TextureUsages::COPY_SRC,
                view_formats: &[],
            });
            let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
            (texture, view)
        };
        let (a0, av0) = mk("flame-a");
        let (b0, bv0) = mk("flame-b");
        // Two bind groups, one per source view. A bind group names the view it
        // was made from, so this is what lets the pass alternate between them
        // instead of rebuilding one per frame.
        let binds = [self.bind_group(&av0), self.bind_group(&bv0)];
        self.targets = Some(Targets {
            a: (a0, av0),
            b: (b0, bv0),
            binds,
            dst: 0,
        });
    }

    fn bind_group(&self, view: &wgpu::TextureView) -> wgpu::BindGroup {
        self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("flame"),
            layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: self.uniforms.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(view),
                },
            ],
        })
    }
}

/// The GPU side of one visualizer preset.
pub struct Shader {
    /// Work items for the render thread, or `None` once it has stopped.
    jobs: Option<SyncSender<Draw>>,
    /// Frames the render thread has finished.
    frames: Receiver<RgbaImage>,
    /// Accumulated seconds, so the shader's clock stays monotonic across the
    /// frames the mailbox drops.
    clock: f32,
}

impl Shader {
    /// Bring up a device and a render thread, or `None` when this machine
    /// cannot provide one.
    ///
    /// `None` is the normal answer on a box without a usable GPU, and it is not
    /// an error: the caller keeps the cell renderer it already had.
    pub fn new() -> Option<Self> {
        // No display handle: this binary never presents, it reads back.
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::LowPower,
            force_fallback_adapter: false,
            compatible_surface: None,
            apply_limit_buckets: false,
        }))
        .ok()?;

        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("gtm visualizer"),
            required_features: wgpu::Features::empty(),
            // No limits are raised: the visualizer asks for a small texture and
            // the downlevel defaults are far above what it needs.
            required_limits: wgpu::Limits::downlevel_defaults(),
            memory_hints: wgpu::MemoryHints::Performance,
            trace: wgpu::Trace::Off,
            experimental_features: wgpu::ExperimentalFeatures::disabled(),
        }))
        .ok()?;

        // Shared by every preset: one uniform block and one history texture, so
        // a bind group is built once per resize rather than once per preset.
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("viz"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: false },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
            ],
        });

        let uniforms = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("uniforms"),
            contents: bytemuck::bytes_of(&Uniforms::zeroed()),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });
        let (jobs_tx, jobs_rx) = sync_channel::<Draw>(MAILBOX);
        let (done_tx, done_rx) = sync_channel::<RgbaImage>(MAILBOX);
        let worker = Worker {
            device,
            queue,
            layout,
            pipelines: std::array::from_fn(|_| None),
            uniforms,
            targets: None,
            done: done_tx,
            clock: 0.0,
        };
        // The thread owns the device outright. Nothing it touches belongs to
        // the frame loop, so a driver stall delays a frame and nothing else.
        std::thread::Builder::new()
            .name("gtm-visualizer".into())
            .spawn(move || worker.run(jobs_rx))
            .ok()?;
        Some(Self {
            jobs: Some(jobs_tx),
            frames: done_rx,
            clock: 0.0,
        })
    }

    /// Render one frame and return it, or `None` to mean "use the cells".
    ///
    /// Never blocks on the GPU. A frame already finished is taken and the next
    /// job posted; otherwise the job goes out and `None` comes back, so a slow
    /// device costs frames rather than latency and a device that has stopped
    /// answering costs nothing beyond the fallback.
    pub fn draw(&mut self, job: Draw) -> Option<RgbaImage> {
        self.clock = (self.clock + job.dt).min(86_400.0);
        match self.frames.try_recv() {
            Ok(img) => {
                self.post(job);
                Some(img)
            }
            Err(TryRecvError::Empty) => {
                self.post(job);
                None
            }
            // The render thread is gone: the device died, or the build never
            // produced one. Every later frame falls back too.
            Err(TryRecvError::Disconnected) => {
                self.jobs = None;
                None
            }
        }
    }

    fn post(&mut self, job: Draw) {
        use std::sync::mpsc::TrySendError;
        let Some(jobs) = &self.jobs else { return };
        match jobs.try_send(job) {
            Ok(()) => {}
            // Full means a frame is still in flight. Dropping this one is the
            // point of the mailbox, not a failure.
            Err(TrySendError::Full(_)) => {}
            // Disconnected means the render thread has stopped, which usually
            // means the device is gone. Stop submitting so the fallback is the
            // only thing left in the path.
            Err(TrySendError::Disconnected(_)) => self.jobs = None,
        }
    }
}

impl std::fmt::Debug for Shader {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Shader")
            .field("alive", &self.jobs.is_some())
            .field("clock", &self.clock)
            .finish()
    }
}

/// The uniform block is `Pod` only because every field has an explicit type
/// here; a 16-byte multiple keeps it inside the address space's rules.
const _: () = assert!(std::mem::size_of::<Uniforms>().is_multiple_of(16));
