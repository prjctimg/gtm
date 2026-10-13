// Shared by every GPU preset. Prepended to each preset's own fragment stage by
// `Preset::source`, so the uniform block has exactly one definition and the
// `.wgsl` files cannot drift out of agreement with `shader.rs`'s `Uniforms`.
//
// Validation (one preset at a time, since a preset file alone is not a module):
//   cat gtm/src/shaders/common.wgsl.inc gtm/src/shaders/<preset>.wgsl \
//     | naga --stdin-file-path check.wgsl

struct Uniforms {
    // Seconds since the visualizer started, for the rise, the sweep and the
    // flicker.
    time: f32,
    // Seconds since the previous frame, so anything that integrates over time is
    // frame-rate independent.
    dt: f32,
    // 1 while audio is playing; the field cools faster at rest.
    playing: f32,
    // Rise speed in rows per second, for the heat field.
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
    // Decimated waveform, interleaved L/R, packed the same way as `bands`.
    // `wave_count` is words rather than samples so it cannot sit inside a
    // fixed-size array.
    wave_count: u32,
    _pad4: vec3<u32>,
    wave: array<vec4<f32>, 16>,
    // Left/right level then left/right slow-falling peak.
    stereo: vec4<f32>,
    // RGB for the cool, hot and peak ends of the theme's ramp, each padded to a
    // vec4 so the struct keeps 16-byte alignment without a manual offset map.
    cool: vec4<f32>,
    warm: vec4<f32>,
    peak: vec4<f32>,
}

@group(0) @binding(0) var<uniform> u: Uniforms;

// The previous frame. Sampled with a load, not a sampler: reading the texture
// being written this pass is undefined, and one frame of lag is invisible.
// Only Flame reads it, but the bind group is shared so every preset pays the
// same layout rather than the presets that ignore it needing their own.
@group(0) @binding(1) var prev: texture_2d<f32>;

/// The level for device column `x`, resampled across the pane's width.
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

/// Waveform sample `i` as (left, right), unpacked from the interleaved array.
fn wave_at(i: u32) -> vec2<f32> {
    if (i >= u.wave_count) {
        return vec2<f32>(0.0);
    }
    let packed = u.wave[i / 4u];
    switch (i % 4u) {
        case 0u: { return vec2<f32>(packed.x, packed.y); }
        case 1u: { return vec2<f32>(packed.z, packed.w); }
        default: { return vec2<f32>(0.0); }
    }
}

/// The theme's accents as colours in 0..1, interpolating the ramp.
fn ramp(h: f32) -> vec3<f32> {
    let t = clamp(h, 0.0, 1.0);
    // `.xyz` because the ramp colours are padded vec4s for alignment; mixing
    // the padded form would return a vec4 this function does not return.
    if (t < 0.5) {
        return mix(u.cool.xyz, u.warm.xyz, t * 2.0);
    }
    return mix(u.warm.xyz, u.peak.xyz, (t - 0.5) * 2.0);
}

/// The pane background: the ramp's dimmest end, so an idle pane is themed
/// rather than black.
fn backdrop() -> vec3<f32> {
    return u.cool.xyz * 0.18;
}

/// A level drawn as colour: the ramp, lifted so a quiet bar is still legible
/// against the background rather than fading into it.
fn level_color(v: f32) -> vec3<f32> {
    return mix(u.cool.xyz * 0.35, ramp(clamp(v * 1.15, 0.0, 1.0)), clamp(v, 0.0, 1.0));
}

struct VsOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
}

/// One oversized triangle covers the viewport with no vertex buffer and no index
/// buffer; three vertices is the smallest a triangle can be drawn from.
@vertex
fn vs(@builtin(vertex_index) i: u32) -> VsOut {
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

/// The device pixel under `uv`.
fn at(uv: vec2<f32>) -> vec2<u32> {
    return vec2<u32>(uv * vec2<f32>(f32(u.size.x), f32(u.size.y)));
}
