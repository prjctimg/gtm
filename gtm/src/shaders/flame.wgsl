// gtm flame visualizer — GPU edition.
//
// The history that makes a flame a flame lives here rather than on the CPU: a
// history texture is sampled at an offset that advances with time, and the
// per-column audio bands inject heat at the bottom row. Nothing here needs a
// second buffer to survive the frame — the previous frame is the history.
//
// Validation: `naga --bulk-validate gtm/src/shaders`

struct Uniforms {
    // Seconds since the visualizer started, for the rise and the flicker.
    time: f32,
    // Seconds since the previous frame, so cooling is frame-rate independent.
    dt: f32,
    // 1 while audio is playing; the field cools faster at rest.
    playing: f32,
    // Rise speed in rows per second.
    rise: f32,
    _pad0: f32,
    _pad1: f32,
    _pad2: f32,
    _pad3: f32,
    // Pane size in device pixels.
    size: vec2<u32>,
    band_count: u32,
    // Pixels per band column.
    band_width: u32,
    // Log-spaced band levels, four per vec4. The uniform address space wants a
    // 16-byte stride for every array element, so a bare `array<f32, N>` is
    // rejected; packing four at a time is the same memory with the alignment
    // the address space requires.
    bands: array<vec4<f32>, 16>,
    // RGB for the cool, hot and peak ends of the theme's ramp, each padded to
    // a vec4 so the struct keeps 16-byte alignment without a manual offset map.
    cool: vec4<f32>,
    warm: vec4<f32>,
    peak: vec4<f32>,
}

@group(0) @binding(0) var<uniform> u: Uniforms;

// The previous frame. Sampled with a load, not a sampler: reading the texture
// being written this pass is undefined, and one frame of lag is invisible.
@group(0) @binding(1) var prev: texture_2d<f32>;

struct VsOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
}

@vertex
fn vs(@builtin(vertex_index) i: u32) -> VsOut {
    // One oversized triangle covers the viewport with no vertex buffer and no
    // index buffer; three vertices is the smallest a triangle can be drawn from.
    var p = array<vec2<f32>, 3>(
        vec2<f32>(-1.0, -1.0),
        vec2<f32>(3.0, -1.0),
        vec2<f32>(-1.0, 3.0),
    );
    var out: VsOut;
    out.pos = vec4<f32>(p[i], 0.0, 1.0);
    out.uv = p[i] * vec2<f32>(0.5, -0.5) + vec2<f32>(0.5, 0.5);
    return out;
}

/// The band level for a column, resampled across the pane's width.
fn band_at(x: u32) -> f32 {
    if (u.band_count == 0u) {
        return 0.0;
    }
    let idx = min(x / max(u.band_width, 1u), u.band_count - 1u);
    let packed = u.bands[idx / 4u];
    switch (idx % 4u) {
        case 0u: { return packed.x; }
        case 1u: { return packed.y; }
        case 2u: { return packed.z; }
        default: { return packed.w; }
    }
}

/// Colour for a heat value, interpolated across the theme's own three accents
/// so the shader follows the reactive palette instead of a fixed gradient.
fn heat_color(h: f32) -> vec3<f32> {
    let t = clamp(h, 0.0, 1.0);
    // `.xyz` because the ramp colors are padded vec4s for alignment; mixing
    // the padded form would return a vec4 this function does not return.
    if (t < 0.5) {
        return mix(u.cool.xyz, u.warm.xyz, t * 2.0);
    }
    return mix(u.warm.xyz, u.peak.xyz, (t - 0.5) * 2.0);
}

@fragment
fn fs(in: VsOut) -> @location(0) vec4<f32> {
    let px = vec2<u32>(in.uv * vec2<f32>(f32(u.size.x), f32(u.size.y)));
    let size = vec2<i32>(i32(u.size.x), i32(u.size.y));

    // Rise: the heat that was one row below this frame has moved up. `rise` is
    // clamped to at least one row so a slow frame cannot sample outside.
    let rise = max(i32(round(u.rise * u.dt)), 1);
    let src = vec2<i32>(i32(px.x), i32(px.y) + rise);
    var heat = 0.0;
    if (src.y >= 0 && src.y < size.y && src.x >= 0 && src.x < size.x) {
        heat = textureLoad(prev, src, 0).r;
    }

    // Inject at the bottom row from the band levels.
    let bottom = size.y - 1;
    if (i32(px.y) == bottom) {
        let drive = band_at(px.x);
        // A little per-column variance so a flat spectrum still flickers.
        let jitter = fract(sin(f32(px.x) * 12.9898 + u.time) * 43758.5453);
        heat = drive * (0.85 + 0.30 * jitter);
    } else {
        // Buoyancy: hotter material rises faster, so the column stretches and
        // thins the way a real flame does.
        let lift = heat * 0.06 * (0.5 + u.playing);
        if (lift > 0.0) {
            heat = heat - lift;
        }
    }

    // Cooling. Frame-rate independent, and faster when nothing is playing so a
    // stopped pane burns out instead of holding a frozen plume.
    let cool_rate = select(0.55, 0.22, u.playing > 0.5);
    heat = heat - heat * cool_rate * max(u.dt, 0.001);
    if (u.playing < 0.5) {
        heat = heat - u.dt * 0.08;
    }

    heat = clamp(heat, 0.0, 1.0);
    return vec4<f32>(heat_color(heat), 1.0);
}
