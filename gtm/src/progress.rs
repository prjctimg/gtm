// Copyright (c) 2026
// Author: prjctimg <prjctimg@outlook.com>
// Track progress indicator styles
//
// This is free software released under the GPL-3.0 license.

use ratatui::style::Color;
use ratatui::text::Span;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum ProgressStyle {
    SeekHead,
    Classic,
    Dots,
    /// True color gradient using theme accent colors.
    #[default]
    TrueGradient,
}

impl ProgressStyle {
    pub fn all() -> &'static [ProgressStyle] {
        &[
            ProgressStyle::SeekHead,
            ProgressStyle::Classic,
            ProgressStyle::Dots,
            ProgressStyle::TrueGradient,
        ]
    }

    pub fn name(&self) -> &'static str {
        match self {
            ProgressStyle::SeekHead => "Seek Head",
            ProgressStyle::Classic => "Classic",
            ProgressStyle::Dots => "Dots",
            ProgressStyle::TrueGradient => "True Gradient",
        }
    }

    pub fn next(&self) -> Self {
        let all = Self::all();
        let idx = all.iter().position(|s| s == self).unwrap_or(0);
        all[(idx + 1) % all.len()]
    }

    pub fn is_animated(&self) -> bool {
        matches!(self, ProgressStyle::SeekHead | ProgressStyle::TrueGradient)
    }
}

const SMOOTHING_TAU_SECS: f64 = 0.12;

/// Frame-rate independent exponential smoother for animated progress styles.
/// Owned by the app and advanced once per frame so every bar on screen moves
/// in lockstep.
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
        self.value = value.clamp(0.0, 1.0);
    }

    pub fn smooth(&mut self, target: f64, dt_secs: f64) -> f64 {
        let alpha = 1.0 - (-dt_secs.max(1e-4) / SMOOTHING_TAU_SECS).exp();
        self.value += (target.clamp(0.0, 1.0) - self.value) * alpha;
        self.value = self.value.clamp(0.0, 1.0);
        self.value
    }
}

/// Ratio a render call should draw: the smoothed value for animated styles,
/// the raw position otherwise.
pub fn render_ratio(style: ProgressStyle, raw: f64, smoothed: f64) -> f64 {
    if style.is_animated() { smoothed } else { raw }
}

/// Interpolate between two RGB colors by factor `t` (0.0..=1.0).
fn lerp_color(a: Color, b: Color, t: f64) -> Color {
    match (a, b) {
        (Color::Rgb(r1, g1, b1), Color::Rgb(r2, g2, b2)) => {
            let t = t.clamp(0.0, 1.0);
            Color::Rgb(
                (r1 as f64 + (r2 as f64 - r1 as f64) * t) as u8,
                (g1 as f64 + (g2 as f64 - g1 as f64) * t) as u8,
                (b1 as f64 + (b2 as f64 - b1 as f64) * t) as u8,
            )
        }
        _ => a,
    }
}

/// Render a progress bar as a `Vec<Span>` with per-character coloring.
/// For non-gradient styles this is equivalent to a single styled span;
/// for `TrueGradient` each filled cell is colored by interpolating through
/// `accent -> secondary_accent -> tertiary_accent`.
pub fn render_progress_styled<'a>(
    ratio: f64,
    width: usize,
    style: ProgressStyle,
    accent: Color,
    secondary: Color,
    tertiary: Color,
) -> Vec<Span<'a>> {
    let inner_w = width.saturating_sub(2).max(4);

    match style {
        ProgressStyle::TrueGradient => {
            let eased_filled = (ratio.clamp(0.0, 1.0) * inner_w as f64).round() as usize;

            let mut spans = Vec::with_capacity(inner_w + 2);
            spans.push(Span::raw(" "));
            for i in 0..inner_w {
                if i < eased_filled.saturating_sub(1) {
                    let t = if eased_filled > 1 {
                        i as f64 / (eased_filled - 1) as f64
                    } else {
                        0.0
                    };
                    let color = if t < 0.5 {
                        lerp_color(accent, secondary, t * 2.0)
                    } else {
                        lerp_color(secondary, tertiary, (t - 0.5) * 2.0)
                    };
                    spans.push(Span::styled(
                        "━",
                        ratatui::style::Style::default().fg(color),
                    ));
                } else if i == eased_filled.saturating_sub(1) && eased_filled > 0 {
                    let t = if eased_filled > 1 {
                        (i - 1) as f64 / (eased_filled - 1) as f64
                    } else {
                        0.0
                    };
                    let color = if t < 0.5 {
                        lerp_color(accent, secondary, t * 2.0)
                    } else {
                        lerp_color(secondary, tertiary, (t - 0.5) * 2.0)
                    };
                    spans.push(Span::styled(
                        "●",
                        ratatui::style::Style::default().fg(color),
                    ));
                } else {
                    spans.push(Span::styled(
                        "─",
                        ratatui::style::Style::default().fg(Color::Rgb(60, 60, 60)),
                    ));
                }
            }
            spans.push(Span::raw(" "));
            spans
        }
        _ => {
            // For non-gradient styles, return as a single span (no per-char color)
            let text = render_progress(ratio, width, style);
            vec![Span::raw(text)]
        }
    }
}

pub fn render_progress(ratio: f64, width: usize, style: ProgressStyle) -> String {
    let inner_w = width.saturating_sub(2).max(4);
    let filled = (ratio.clamp(0.0, 1.0) * inner_w as f64).round() as usize;
    let mut line = String::with_capacity(width);
    match style {
        ProgressStyle::SeekHead => {
            let head = filled.min(inner_w.saturating_sub(1));
            line.push('─');
            for i in 0..inner_w {
                if i == head {
                    line.push('●');
                } else {
                    line.push('─');
                }
            }
            line.push('─');
        }
        ProgressStyle::Classic => {
            for i in 0..inner_w {
                line.push(if i < filled { '━' } else { '─' });
            }
        }
        ProgressStyle::Dots => {
            line.push(' ');
            for i in 0..inner_w {
                if i < filled {
                    line.push('●');
                } else {
                    line.push('○');
                }
            }
            line.push(' ');
        }
        ProgressStyle::TrueGradient => {
            let eased_filled = filled;
            line.push(' ');
            for i in 0..inner_w {
                if i < eased_filled.saturating_sub(1) {
                    let dist = (eased_filled - i) as f64 / eased_filled.max(1) as f64;
                    if dist > 0.66 {
                        line.push('━');
                    } else if dist > 0.33 {
                        line.push('╌');
                    } else {
                        line.push('─');
                    }
                } else if i == eased_filled.saturating_sub(1) && eased_filled > 0 {
                    line.push('●');
                } else {
                    line.push('─');
                }
            }
            line.push(' ');
        }
    }
    line
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seek_head_always_visible() {
        for ratio in [0.0f64, 0.5, 1.0] {
            let bar = render_progress(ratio, 22, ProgressStyle::SeekHead);
            assert!(bar.contains('●'), "head missing at ratio {ratio}");
        }
    }

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
}
