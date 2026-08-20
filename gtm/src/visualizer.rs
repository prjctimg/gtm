// Copyright (c) 2026 - present
// Author: prjctimg <prjctimg@outlook.com>
// Audio visualizer with multiple render presets
//
// This is free software released under the GPL-3.0 license.

use std::time::Instant;

use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Widget;

use crate::theme::AppTheme;

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum VisualizerPreset {
    #[default]
    Braille,
    Blocks,
    Mirror,
    Gradient,
    Spectrum,
}

impl VisualizerPreset {
    pub fn all() -> &'static [VisualizerPreset] {
        &[
            VisualizerPreset::Braille,
            VisualizerPreset::Blocks,
            VisualizerPreset::Mirror,
            VisualizerPreset::Gradient,
            VisualizerPreset::Spectrum,
        ]
    }

    pub fn next(&self) -> VisualizerPreset {
        let all = Self::all();
        let idx = all.iter().position(|p| p == self).unwrap_or(0);
        all[(idx + 1) % all.len()]
    }

    pub fn name(&self) -> &'static str {
        match self {
            VisualizerPreset::Braille => "Braille",
            VisualizerPreset::Blocks => "Blocks",
            VisualizerPreset::Mirror => "Mirror",
            VisualizerPreset::Gradient => "Gradient",
            VisualizerPreset::Spectrum => "Spectrum",
        }
    }
}

pub struct AudioVisualizer {
    pub enabled: bool,
    pub preset: VisualizerPreset,
    bars: Vec<f32>,
    target_bars: Vec<f32>,
    last_tick: Instant,
    spectrum_offset: f64,
}

impl AudioVisualizer {
    pub fn new() -> Self {
        Self {
            enabled: false,
            preset: VisualizerPreset::default(),
            bars: vec![0.0; 32],
            target_bars: vec![0.0; 32],
            last_tick: Instant::now(),
            spectrum_offset: 0.0,
        }
    }

    pub fn toggle(&mut self) {
        self.enabled = !self.enabled;
    }

    pub fn cycle_preset(&mut self) {
        self.preset = self.preset.next();
    }

    pub fn is_enabled(&self) -> bool {
        self.enabled
    }

    pub fn tick(&mut self, is_playing: bool, width: u16, audio_levels: &[f32]) {
        if !self.enabled || width == 0 {
            return;
        }
        let now = Instant::now();
        let dt = now.duration_since(self.last_tick).as_secs_f64();
        self.last_tick = now;

        let num_bars = width.max(4) as usize;
        if self.bars.len() != num_bars {
            self.bars.resize(num_bars, 0.0);
            self.target_bars.resize(num_bars, 0.0);
        }

        if is_playing && !audio_levels.is_empty() {
            let bins = audio_levels.len();
            for (i, target) in self.target_bars.iter_mut().enumerate() {
                // Map bar index to spectrum bin index (log-ish mapping: lower frequencies get more bars)
                let ratio = i as f64 / num_bars as f64;
                let idx = (ratio * bins as f64 * 0.8) as usize; // compress high end slightly
                let idx = idx.min(bins - 1);
                let level = audio_levels[idx];
                *target = level.clamp(0.0, 1.0);
            }
        } else {
            for target in self.target_bars.iter_mut() {
                *target = 0.0;
            }
        }

        // Exponential smoothing: faster attack, slower decay for natural feel
        let attack = 0.4;
        let decay = if is_playing { 0.15 } else { 0.8 };
        for (bar, target) in self.bars.iter_mut().zip(self.target_bars.iter()) {
            let diff = target - *bar;
            let rate = if diff > 0.0 { attack } else { decay };
            *bar += diff * rate as f32 * (dt * 60.0) as f32;
            *bar = bar.clamp(0.0, 1.0);
        }
    }

    fn amplitude_color(&self, val: f32, theme: &AppTheme) -> Color {
        if val > 0.7 {
            theme.accent
        } else if val > 0.4 {
            theme.fg_bright
        } else {
            theme.fg_dim
        }
    }

    pub fn render(&self, area: Rect, theme: &AppTheme) -> Option<Lines<'_>> {
        if !self.enabled || area.width < 4 || area.height < 3 {
            return None;
        }

        let w = area.width as usize;
        let h = area.height as usize;
        let num_bars = w.min(self.bars.len());

        Some(match self.preset {
            VisualizerPreset::Braille => self.render_braille(num_bars, h, theme),
            VisualizerPreset::Blocks => self.render_blocks(num_bars, h, theme),
            VisualizerPreset::Mirror => self.render_mirror(num_bars, h, theme),
            VisualizerPreset::Gradient => self.render_gradient(num_bars, h, theme),
            VisualizerPreset::Spectrum => self.render_spectrum(num_bars, h, theme),
        })
    }

    /// Block columns with fine fractional heights (`▁…█`).
    fn render_blocks(&self, num_bars: usize, height: usize, theme: &AppTheme) -> Lines<'_> {
        const BLOCKS: [char; 8] = ['▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];
        let mut lines: Vec<Line<'static>> = Vec::new();
        for row_from_top in (0..height).rev() {
            let mut spans: Vec<Span<'static>> = Vec::new();
            for i in 0..num_bars {
                let val = *self.bars.get(i).unwrap_or(&0.0);
                let filled = val * height as f32;
                let full_rows = filled.floor() as usize;
                let frac = filled - filled.floor();
                let ch = if row_from_top < full_rows {
                    BLOCKS[7]
                } else if row_from_top == full_rows && frac > 0.0 && full_rows < height {
                    BLOCKS[((frac * 8.0) as usize).min(7)]
                } else {
                    ' '
                };
                let color = if ch == ' ' {
                    theme.bg
                } else {
                    self.amplitude_color(val, theme)
                };
                spans.push(Span::styled(ch.to_string(), Style::default().fg(color)));
            }
            lines.push(Line::from(spans));
        }
        Lines(lines)
    }

    /// Center-symmetric bars blooming outward from the middle axis.
    fn render_mirror(&self, num_bars: usize, height: usize, theme: &AppTheme) -> Lines<'_> {
        let half = height as f32 / 2.0;
        let mut lines: Vec<Line<'static>> = Vec::new();
        for row_from_top in 0..height {
            let mut spans: Vec<Span<'static>> = Vec::new();
            let center_dist = ((row_from_top as f32 + 0.5) - half).abs();
            for i in 0..num_bars {
                let val = *self.bars.get(i).unwrap_or(&0.0);
                let extent = val * half;
                let ch = if center_dist <= extent { '█' } else { ' ' };
                let color = if ch == ' ' {
                    theme.bg
                } else {
                    self.amplitude_color(val, theme)
                };
                spans.push(Span::styled(ch.to_string(), Style::default().fg(color)));
            }
            lines.push(Line::from(spans));
        }
        Lines(lines)
    }

    /// Braille cells with a 4-step color ramp by amplitude.
    fn render_gradient(&self, num_bars: usize, height: usize, theme: &AppTheme) -> Lines<'_> {
        let mut lines: Vec<Line<'static>> = Vec::new();
        let braille_fill = '⣿';
        let braille_empty = '⠀';
        for row in (0..height).rev() {
            let mut spans: Vec<Span<'static>> = Vec::new();
            let threshold = (row + 1) as f32 / height as f32;
            for i in 0..num_bars {
                let val = *self.bars.get(i).unwrap_or(&0.0);
                if val >= threshold {
                    let color = if val > 0.75 {
                        theme.accent
                    } else if val > 0.5 {
                        theme.fg_bright
                    } else if val > 0.25 {
                        theme.fg
                    } else {
                        theme.fg_dim
                    };
                    spans.push(Span::styled(
                        braille_fill.to_string(),
                        Style::default().fg(color),
                    ));
                } else {
                    spans.push(Span::styled(
                        braille_empty.to_string(),
                        Style::default().fg(theme.bg),
                    ));
                }
            }
            lines.push(Line::from(spans));
        }
        Lines(lines)
    }

    /// Scrolling decay band: bars sampled at a seed-driven offset so the
    /// waveform appears to travel across the screen.
    fn render_spectrum(&self, num_bars: usize, height: usize, theme: &AppTheme) -> Lines<'_> {
        const BLOCKS: [char; 8] = ['▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];
        let shift = (self.spectrum_offset * 2.0) as usize % num_bars.max(1);
        let mut lines: Vec<Line<'static>> = Vec::new();
        for row_from_top in (0..height).rev() {
            let mut spans: Vec<Span<'static>> = Vec::new();
            for i in 0..num_bars {
                let src = (i + shift) % num_bars;
                let val = *self.bars.get(src).unwrap_or(&0.0);
                let filled = val * height as f32;
                let full_rows = filled.floor() as usize;
                let frac = filled - filled.floor();
                let ch = if row_from_top < full_rows {
                    BLOCKS[7]
                } else if row_from_top == full_rows && frac > 0.0 && full_rows < height {
                    BLOCKS[((frac * 8.0) as usize).min(7)]
                } else {
                    ' '
                };
                let color = if ch == ' ' {
                    theme.bg
                } else {
                    self.amplitude_color(val, theme)
                };
                spans.push(Span::styled(ch.to_string(), Style::default().fg(color)));
            }
            lines.push(Line::from(spans));
        }
        Lines(lines)
    }

    fn render_braille(&self, num_bars: usize, height: usize, theme: &AppTheme) -> Lines<'_> {
        let mut lines: Vec<Line<'static>> = Vec::new();
        let braille_fill = '⣿';
        let braille_empty = '⠀';
        for row in (0..height).rev() {
            let mut spans: Vec<Span<'static>> = Vec::new();
            let threshold = (row + 1) as f32 / height as f32;
            for i in 0..num_bars {
                let val = *self.bars.get(i).unwrap_or(&0.0);
                if val >= threshold {
                    let color = self.amplitude_color(val, theme);
                    spans.push(Span::styled(
                        braille_fill.to_string(),
                        Style::default().fg(color),
                    ));
                } else {
                    spans.push(Span::styled(
                        braille_empty.to_string(),
                        Style::default().fg(theme.bg),
                    ));
                }
            }
            lines.push(Line::from(spans));
        }
        Lines(lines)
    }
}

impl Default for AudioVisualizer {
    fn default() -> Self {
        Self::new()
    }
}

pub struct Lines<'a>(pub Vec<Line<'a>>);

impl<'a> Widget for Lines<'a> {
    fn render(self, area: Rect, buf: &mut ratatui::buffer::Buffer) {
        for (row, line) in self.0.iter().enumerate() {
            if row as u16 >= area.height {
                break;
            }
            let mut x = area.x;
            for span in &line.spans {
                for ch in span.content.chars() {
                    if x >= area.x + area.width {
                        break;
                    }
                    let cell_y = area.y + row as u16;
                    if x >= buf.area.width || cell_y >= buf.area.height {
                        break;
                    }
                    if let Some(cell) = buf.cell_mut((x, cell_y)) {
                        cell.set_symbol(&ch.to_string()).set_style(span.style);
                    }
                    x += 1;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preset_cycles_through_all_and_wraps() {
        let mut p = VisualizerPreset::default();
        assert_eq!(p, VisualizerPreset::Braille);
        let names: Vec<&str> = VisualizerPreset::all().iter().map(|x| x.name()).collect();
        assert_eq!(
            names,
            vec!["Braille", "Blocks", "Mirror", "Gradient", "Spectrum"]
        );
        for _ in 0..VisualizerPreset::all().len() {
            p = p.next();
        }
        assert_eq!(p, VisualizerPreset::Braille);
    }

    #[test]
    fn preset_serde_round_trip() {
        let p = serde_json::from_str::<VisualizerPreset>("\"spectrum\"").unwrap();
        assert_eq!(p, VisualizerPreset::Spectrum);
        assert_eq!(
            serde_json::to_string(&VisualizerPreset::Blocks).unwrap(),
            "\"blocks\""
        );
    }
}
