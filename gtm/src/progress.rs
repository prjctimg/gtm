// Copyright (c) 2026
// Author: prjctimg <prjctimg@outlook.com>
// Track progress indicator
//
// This is free software released under the GPL-3.0 license.

use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};

const SMOOTHING_TAU_SECS: f64 = 0.12;

/// Frame-rate independent exponential smoother for the bar.
///
/// Owned by the app and advanced once per frame so every bar on screen moves in
/// lockstep — two bars at different ratios is a rendering bug, not two bars.
#[derive(Debug, Clone)]
pub struct ProgressSmoother {
    value: f64,
}

impl Default for ProgressSmoother {
    fn default() -> Self {
        Self::new()
    }
}

impl ProgressSmoother {
    pub fn new() -> Self {
        Self { value: 0.0 }
    }

    pub fn value(&self) -> f64 {
        self.value
    }

    pub fn reset(&mut self, value: f64) {
        self.value = value;
    }

    pub fn smooth(&mut self, target: f64, dt_secs: f64) -> f64 {
        let alpha = 1.0 - (-dt_secs / SMOOTHING_TAU_SECS).exp();
        self.value += (target - self.value) * alpha;
        self.value
    }
}

/// The bar itself, and the one place that knows how it is drawn.
///
/// There is no style picker any more. Four glyph styles became one shader
/// (`shaders/progress.wgsl`), so there is one look, and a picker over a single
/// entry is a picker that cannot choose.
pub struct ProgressBar {
    /// Lazily created on the first draw. `None` on a machine with no usable GPU,
    /// which is not an error state: [`Self::draw`] falls back to cells and the
    /// bar is still a bar.
    #[cfg(feature = "shader-viz")]
    gpu: Option<crate::shader::Shader>,
}

impl Default for ProgressBar {
    fn default() -> Self {
        Self::new()
    }
}

impl ProgressBar {
    pub fn new() -> Self {
        Self {
            #[cfg(feature = "shader-viz")]
            gpu: None,
        }
    }

    /// The bar, `width` cells wide, `ratio` of the way through.
    ///
    /// Returns a [`Line`] rather than a `String` because the GPU path cannot
    /// produce one: a frame is read back and printed with `▀`, which resolves
    /// to a glyph carrying a foreground *and* a background colour per cell. Two
    /// coloured halves cannot be expressed as text.
    ///
    /// `accent` and `dim` are only read by the fallback. On the GPU path they
    /// come from the theme ramp the shader was given, which is the same palette
    /// the visualizer uses.
    pub fn draw(&mut self, ratio: f64, width: usize, accent: Color, dim: Color) -> Line<'static> {
        let width = width.max(1);
        let ratio = ratio.clamp(0.0, 1.0);

        #[cfg(feature = "shader-viz")]
        {
            if self.gpu.is_none() {
                self.gpu = crate::shader::Shader::new();
            }
            if let Some(gpu) = self.gpu.as_mut() {
                let job = crate::shader::Draw {
                    preset: crate::shader::Preset::Progress,
                    playing: true,
                    dt: 0.0,
                    bands: [0.0; crate::shader::BANDS],
                    band_count: 0,
                    wave: [0.0; crate::shader::WAVE],
                    wave_count: 0,
                    stereo: [0.0; 4],
                    cells: (width as u16, 1),
                    progress: ratio as f32,
                    // The ramp the shader shades the run along. The bar is drawn
                    // on whatever pane it sits in, so it borrows the pane's
                    // background rather than painting one of its own.
                    cool: rgb(accent),
                    warm: rgb(accent),
                    peak: rgb(accent),
                };
                // `Shader::draw` answers `None` on the very first call by
                // design — a device that exists still has a frame in flight —
                // so `None` means "cells this frame", not "give up on the GPU".
                if let Some(img) = gpu.draw(job) {
                    let lines = crate::visualizer::pixels_to_lines(&img, width as u16, 1);
                    // One cell row was asked for, so the readback is one line.
                    // Concatenated rather than returned as `Lines` because the
                    // caller splices the bar into a line it composes itself.
                    return match lines.0.into_iter().next() {
                        Some(line) => line,
                        None => Self::cells(ratio, width, accent, dim),
                    };
                }
            }
        }

        Self::cells(ratio, width, accent, dim)
    }

    /// The bar without a GPU.
    ///
    /// A run of `━` in the accent, a `●` head, and a `─` track after it. It is
    /// not the old `Seek Head` style brought back — it is the minimum the shader
    /// draws expressed in glyphs, so the fallback reads as the same bar rather
    /// than as a different one.
    fn cells(ratio: f64, width: usize, accent: Color, dim: Color) -> Line<'static> {
        let filled = (ratio * width as f64).round() as usize;
        let head = filled.min(width.saturating_sub(1));
        let mut spans = Vec::with_capacity(width);
        for i in 0..width {
            let (ch, color) = if i < head {
                ('━', accent)
            } else if i == head {
                ('●', accent)
            } else {
                ('─', dim)
            };
            spans.push(Span::styled(ch.to_string(), Style::default().fg(color)));
        }
        Line::from(spans)
    }
}

/// A `Color` as the 0–255 triple the shader uniform carries.
fn rgb(c: Color) -> [u8; 3] {
    match c {
        Color::Rgb(r, g, b) => [r, g, b],
        // Indexed and Reset have no components to send. Black is the honest
        // answer: it renders as "not the accent", and the bar is drawn from the
        // accent on every theme that can produce one at all.
        _ => [0, 0, 0],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn smoother_converges() {
        let mut s = ProgressSmoother::new();
        let mut v = 0.0;
        for _ in 0..200 {
            v = s.smooth(1.0, 1.0 / 60.0);
        }
        assert!((v - 1.0).abs() < 0.01);
    }

    #[test]
    fn smoother_reset_snaps() {
        let mut s = ProgressSmoother::new();
        s.smooth(0.9, 1.0 / 60.0);
        s.reset(0.1);
        assert!((s.value() - 0.1).abs() < f64::EPSILON);
    }

    #[test]
    fn smoother_is_frame_rate_independent() {
        // The same elapsed time in different step counts has to land in the
        // same place, or the bar moves at a different speed on a faster machine.
        let mut coarse = ProgressSmoother::new();
        for _ in 0..10 {
            coarse.smooth(1.0, 0.1);
        }
        let mut fine = ProgressSmoother::new();
        for _ in 0..100 {
            fine.smooth(1.0, 0.01);
        }
        assert!((coarse.value() - fine.value()).abs() < 1e-6);
    }

    /// The bar is always exactly as wide as it was asked for.
    ///
    /// It is drawn into a fixed-width row beside the elapsed and total times, so
    /// a bar one cell narrower shifts everything after it, and one wider is
    /// clipped. The width is a contract with the layout, not a hint.
    #[test]
    fn the_bar_is_exactly_the_width_asked_for() {
        let mut bar = ProgressBar::new();
        for width in [1usize, 4, 12, 40] {
            for ratio in [0.0f64, 0.37, 1.0] {
                let line = bar.draw(ratio, width, Color::Rgb(1, 2, 3), Color::Rgb(4, 5, 6));
                assert_eq!(
                    line.width(),
                    width,
                    "{width}-cell bar came out {} cells at ratio {ratio}",
                    line.width()
                );
            }
        }
    }

    /// A bar is visible at every ratio, including none and all.
    ///
    /// The regression this pins: a head computed as `filled - 1` is negative at
    /// ratio 0, so an unwrapped `filled` indexing left the bar blank until a
    /// percent of the track had passed. Zero has to read as zero.
    #[test]
    fn something_is_drawn_at_every_ratio() {
        let mut bar = ProgressBar::new();
        for ratio in [0.0f64, 0.01, 0.5, 0.99, 1.0] {
            let line = bar.draw(ratio, 20, Color::Rgb(200, 120, 40), Color::Rgb(60, 60, 60));
            let text: String = line
                .spans
                .iter()
                .map(|s| s.content.as_ref())
                .collect::<String>();
            assert!(text.contains('●'), "no head at ratio {ratio}: {text:?}");
        }
    }

    /// The filled run never exceeds the bar, and never overruns it by rounding.
    #[test]
    fn the_fill_stays_inside_the_bar() {
        let mut bar = ProgressBar::new();
        for ratio in [0.0f64, 0.999, 1.0, 1.5, -1.0] {
            let line = bar.draw(ratio, 8, Color::Rgb(9, 9, 9), Color::Rgb(3, 3, 3));
            let heads = line
                .spans
                .iter()
                .filter(|s| s.content.as_ref() == "●")
                .count();
            assert_eq!(heads, 1, "ratio {ratio} produced {heads} heads");
        }
    }
}
