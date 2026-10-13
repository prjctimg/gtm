// Gradient: a bar grid on the theme's ramp.
//
// The ramp is the difference from the plain bar presets, and colour is the one
// thing the `▀` transport carries perfectly — a cell has a single foreground
// colour, so a per-column colour is exact rather than approximated.

@fragment
fn fs(in: VsOut) -> @location(0) vec4<f32> {
    let px = at(in.uv);
    let level = band_at(px.x);
    let height = f32(u.size.y);

    // Draw the bar twice, once per half of the cell, placing the boundary where
    // the bar's top edge falls between the two sampled rows.
    let top_row = px.y * 2u;
    let bar_top = height - level * height;
    let y = f32(top_row);
    let bg = backdrop();

    if (level <= 0.0) {
        return vec4<f32>(bg, 1.0);
    }
    // Fade the leading edge rather than clipping it: a hard boundary at an
    // arbitrary row reads as a glitch, a partial one reads as a soft top.
    let frac = clamp(bar_top - y, 0.0, 1.0);
    if (y >= bar_top) {
        return vec4<f32>(mix(bg, level_color(level), frac), 1.0);
    }
    if (y + 1.0 > bar_top) {
        return vec4<f32>(mix(bg, level_color(level), bar_top + 1.0 - y), 1.0);
    }
    // The lower half is the same bar, a shade darker so the two halves of the
    // cell do not read as one flat block.
    if (y + 1.0 >= bar_top) {
        return vec4<f32>(level_color(level) * 0.72, 1.0);
    }
    return vec4<f32>(bg, 1.0);
}
