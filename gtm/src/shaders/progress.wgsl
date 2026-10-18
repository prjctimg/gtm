// Progress bar: a filled run, a head, and nothing at all after it.
//
// Unlike every other preset this one is not a pane. It is one or two rows drawn
// inline in a text layout, and the part that is not filled is *transparent*
// rather than a dark colour, so the terminal's own background shows through and
// the bar reads against whatever the pane behind it is.
//
// That matters because of the transport. A frame is read back and printed with
// `▀`, which resolves one horizontal and two vertical samples per cell, and
// alpha 0 maps to `Color::Reset`. So a cell is either painted in both halves or
// left alone in both -- there is no way to paint only the top half and leave the
// bottom alone, because both halves become one glyph's fg and bg.

@fragment
fn fs(in: VsOut) -> @location(0) vec4<f32> {
    let px = at(in.uv);
    let w = f32(u.size.x);
    let ratio = clamp(u.progress, 0.0, 1.0);

    // Where the fill ends, in device pixels. Half a cell of slack so a bar at
    // 99% still shows a sliver of head rather than snapping to full a cell
    // early.
    let filled = ratio * w;

    // Past the head: nothing. Transparent, not the backdrop, so the bar has no
    // bar behind it.
    if (f32(px.x) >= filled) {
        return vec4<f32>(0.0, 0.0, 0.0, 0.0);
    }

    // The head itself, one device pixel wide, lifted to the top of the ramp.
    // Without it the bar is a plain run of colour with no indication of exactly
    // where it ends, which is the one thing a progress bar has to say.
    if (f32(px.x) >= filled - 1.0) {
        return vec4<f32>(ramp(1.0), 1.0);
    }

    // The run, shaded along its length from the cool end to the warm end, so the
    // bar carries the same theme ramp the visualizer does and a reactive
    // palette reaches it as well as the pane.
    let t = select(0.0, f32(px.x) / max(filled - 1.0, 1.0), filled > 1.0);
    return vec4<f32>(mix(u.cool.xyz, ramp(0.75), t * 0.85), 1.0);
}