// Copyright (c) 2025 - present
// Author: prjctimg <prjctimg@outlook.com>
// Track progress indicator styles
//
// This is free software released under the GPL-3.0 license.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ProgressStyle {
    Braille,
    SeekHead,
    Classic,
    Dots,
    Arrows,
    Blocks,
    Gradient,
}

impl Default for ProgressStyle {
    fn default() -> Self {
        ProgressStyle::Braille
    }
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
        }
    }

    pub fn next(&self) -> Self {
        let all = Self::all();
        let idx = all.iter().position(|s| s == self).unwrap_or(0);
        all[(idx + 1) % all.len()]
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
            line.push('[');
            for i in 0..inner_w {
                line.push(if i < filled { '█' } else { '░' });
            }
            line.push(']');
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
        ProgressStyle::Gradient => {
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
