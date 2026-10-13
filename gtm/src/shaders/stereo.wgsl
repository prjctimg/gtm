// Stereo: left and right level meters.
//
// Two half-width columns, each a vertical bar. One cell is one column and the
// transport resolves height to two rows, so a meter is the simplest thing there
// is to draw correctly.

@fragment
fn fs(in: VsOut) -> @location(0) vec4<f32> {
    let px = at(in.uv);
    let w = f32(u.size.x);
    let h = f32(u.size.y);
    let bg = backdrop();

    // Left meter in the left half, right in the right half, with a gap down the
    // middle so the two never merge into one shape.
    let half = w * 0.5;
    let is_left = f32(px.x) < half - 1.0;
    let level = select(u.stereo.z, u.stereo.x, is_left);
    let peak = select(u.stereo.w, u.stereo.y, is_left);

    let top_row = px.y * 2u;
    let y = f32(top_row);
    let bar_top = h - clamp(level, 0.0, 1.0) * h;

    // The peak cap: a dimmer line left behind at the highest recent level,
    // which is what makes a meter readable at a glance rather than only at its
    // peak.
    let peak_y = h - clamp(peak, 0.0, 1.0) * h;
    if (abs(y - peak_y) < 1.0 && peak > 0.01) {
        return vec4<f32>(ramp(0.9), 1.0);
    }

    if (level <= 0.0) {
        return vec4<f32>(bg, 1.0);
    }
    let frac = clamp(bar_top - y, 0.0, 1.0);
    if (y >= bar_top) {
        return vec4<f32>(mix(bg, ramp(0.45 + level * 0.55), frac), 1.0);
    }
    if (y + 1.0 > bar_top) {
        return vec4<f32>(mix(bg, ramp(0.45 + level * 0.55), bar_top + 1.0 - y), 1.0);
    }
    return vec4<f32>(ramp(0.45 + level * 0.55) * 0.7, 1.0);
}
