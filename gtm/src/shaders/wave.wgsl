// Wave: an oscilloscope trace.
//
// A trace is a function of x at one row per cell, which is the shape the
// transport handles best — no sub-cell detail is needed to draw a line.

@fragment
fn fs(in: VsOut) -> @location(0) vec4<f32> {
    let px = at(in.uv);
    let w = f32(u.size.x);
    let h = f32(u.size.y);
    let bg = backdrop();

    if (u.wave_count == 0u) {
        return vec4<f32>(bg, 1.0);
    }

    // The waveform is interleaved L/R; the trace is their mean, which is what
    // keeps a stereo signal from drawing two overlapping lines.
    let n = f32(u.wave_count);
    let xf = (f32(px.x) + 0.5) / max(w, 1.0);
    let pos = clamp(xf * n, 0.0, n - 1.0);
    let i = u32(pos);
    let frac = pos - f32(i);
    let a = wave_at(i);
    let b = wave_at(min(i + 1u, u.wave_count - 1u));
    let s = mix((a.x + a.y) * 0.5, (b.x + b.y) * 0.5, frac);

    // Two sampled rows per cell, so the trace is drawn at whatever row its
    // amplitude lands on and the vertical error is at most half a cell.
    let mid = h * 0.5;
    let trace_y = mid - s * mid * 0.85;
    let y = f32(px.y) * 2.0;

    // A one-row line, thickened to two so it survives the sampling.
    let d = abs(y - trace_y);
    if (d < 1.0) {
        return vec4<f32>(ramp(0.35 + abs(s) * 0.65), 1.0);
    }
    if (d < 2.5) {
        return vec4<f32>(mix(bg, ramp(0.5), 1.0 - (d - 1.0) / 1.5), 1.0);
    }
    return vec4<f32>(bg, 1.0);
}
