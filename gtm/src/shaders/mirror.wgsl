// Mirror: bars reflected about the vertical centre.
//
// One cell is one column, so a bar grid is exactly what the transport can
// carry: the reflection is a function of x, and x is the one axis a cell
// resolves perfectly.

@fragment
fn fs(in: VsOut) -> @location(0) vec4<f32> {
    let px = at(in.uv);
    let w = f32(u.size.x);
    let h = f32(u.size.y);
    let bg = backdrop();

    // Fold the pane about its centre. `x * 2 + 1 - w` is the distance from the
    // middle in half-pixels, so the seam lands on the middle column and the two
    // halves are exact mirrors rather than being off by one.
    let d = abs(f32(px.x) * 2.0 + 1.0 - w) / max(w, 1.0);
    let level = band_at(u32(d * f32(max(u.band_count, 1))));
    if (level <= 0.0) {
        return vec4<f32>(bg, 1.0);
    }

    // `▀` prints the upper half in the foreground and the lower half in the
    // background, so a bar has to be placed twice: once per half, with the
    // boundary falling between the two sampled rows.
    let top_row = px.y * 2u;
    let y = f32(top_row);
    let bar_top = h - level * h;
    let color = level_color(level);

    if (y >= bar_top) {
        // The boundary row: a partial fill rather than a hard edge, so the top
        // of the bar reads as soft instead of as a glitch.
        return vec4<f32>(mix(bg, color, clamp(bar_top + 1.0 - y, 0.0, 1.0)), 1.0);
    }
    if (y + 1.0 > bar_top) {
        return vec4<f32>(mix(bg, color, clamp(bar_top + 1.0 - y, 0.0, 1.0)), 1.0);
    }
    // Fully inside the bar. The lower half is shaded so the two halves of a
    // cell do not read as one flat slab.
    if (y + 1.0 >= bar_top) {
        return vec4<f32>(color * 0.72, 1.0);
    }
    return vec4<f32>(bg, 1.0);
}
