// Copyright (c) 2025 - present
// Author: prjctimg <prjctimg@outlook.com>
// Track progress indicator styles
//
// This is free software released under the GPL-3.0 license.

use ratatui::style::Color;
use ratatui::text::Span;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum ProgressStyle {
    #[default]
    Braille,
    SeekHead,
    Classic,
    Dots,
    Arrows,
    Blocks,
    Gradient,
    /// True color gradient using theme accent colors.
    TrueGradient,
}

impl ProgressStyle {
    pub fn all() -> &'static [ProgressStyle] {
        &[
            ProgressStyle::Braille,
            ProgressStyle::SeekHead,
            ProgressStyle::Classic,
            ProgressStyle::Dots,
            ProgressStyle::Arrows,
            ProgressStyle::Blocks,
            ProgressStyle::Gradient,
            ProgressStyle::TrueGradient,
        ]
    }

    pub fn name(&self) -> &'static str {
        match self {
            ProgressStyle::Braille => "Braille",
            ProgressStyle::SeekHead => "Seek Head",
            ProgressStyle::Classic => "Classic",
            ProgressStyle::Dots => "Dots",
            ProgressStyle::Arrows => "Arrows",
            ProgressStyle::Blocks => "Blocks",
            ProgressStyle::Gradient => "Gradient",
            ProgressStyle::TrueGradient => "True Gradient",
        }
    }

    pub fn next(&self) -> Self {
        let all = Self::all();
        let idx = all.iter().position(|s| s == self).unwrap_or(0);
        all[(idx + 1) % all.len()]
    }
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
    let filled = (ratio.clamp(0.0, 1.0) * inner_w as f64).round() as usize;

    match style {
        ProgressStyle::TrueGradient => {
            let mut spans = Vec::with_capacity(inner_w + 2);
            spans.push(Span::raw(" "));
            for i in 0..inner_w {
                if i < filled {
                    let t = if filled > 1 {
                        i as f64 / (filled - 1) as f64
                    } else {
                        0.0
                    };
                    // Three-stop gradient: accent -> secondary -> tertiary
                    let color = if t < 0.5 {
                        lerp_color(accent, secondary, t * 2.0)
                    } else {
                        lerp_color(secondary, tertiary, (t - 0.5) * 2.0)
                    };
                    spans.push(Span::styled(
                        "█",
                        ratatui::style::Style::default().fg(color),
                    ));
                } else {
                    spans.push(Span::styled(
                        "░",
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
        ProgressStyle::Braille => {
            line.push('⡀');
            for i in 0..inner_w {
                if i < filled {
                    line.push('⣿');
                } else {
                    line.push('⣀');
                }
            }
            line.push('⠤');
        }
        ProgressStyle::SeekHead => {
            line.push('─');
            for i in 0..inner_w {
                if i == filled {
                    line.push('●');
                } else {
                    line.push('─');
                }
            }
            line.push('─');
        }
        ProgressStyle::Classic => {
            for i in 0..inner_w {
                line.push(if i < filled { '█' } else { '░' });
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
        ProgressStyle::Arrows => {
            for i in 0..inner_w {
                if i < filled {
                    line.push('━');
                } else if i == filled {
                    line.push('▶');
                } else {
                    line.push('─');
                }
            }
        }
        ProgressStyle::Blocks => {
            line.push(' ');
            for i in 0..inner_w {
                if i < filled {
                    line.push('█');
                } else {
                    line.push('░');
                }
            }
            line.push(' ');
        }
        ProgressStyle::Gradient | ProgressStyle::TrueGradient => {
            line.push(' ');
            for i in 0..inner_w {
                if i < filled {
                    let dist = (filled - i) as f64 / filled.max(1) as f64;
                    if dist > 0.66 {
                        line.push('█');
                    } else if dist > 0.33 {
                        line.push('▓');
                    } else {
                        line.push('▒');
                    }
                } else {
                    line.push('░');
                }
            }
            line.push(' ');
        }
    }
    line
}
