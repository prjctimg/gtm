use std::time::Instant;

use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Widget;

use crate::theme::AppTheme;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)]
pub enum VisualizerStyle {
    Bars,
    Dots,
    Braille,
    Wave,
}

#[allow(dead_code)]
impl VisualizerStyle {
    pub fn all() -> &'static [VisualizerStyle] {
        &[
            VisualizerStyle::Bars,
            VisualizerStyle::Dots,
            VisualizerStyle::Braille,
            VisualizerStyle::Wave,
        ]
    }

    pub fn name(&self) -> &'static str {
        match self {
            VisualizerStyle::Bars => "Bars",
            VisualizerStyle::Dots => "Dots",
            VisualizerStyle::Braille => "Braille",
            VisualizerStyle::Wave => "Wave",
        }
    }

    pub fn next(&self) -> Self {
        let all = Self::all();
        let idx = all.iter().position(|s| s == self).unwrap_or(0);
        all[(idx + 1) % all.len()]
    }
}

pub struct AudioVisualizer {
    pub enabled: bool,
    pub style: VisualizerStyle,
    bars: Vec<f32>,
    target_bars: Vec<f32>,
    last_tick: Instant,
    seed: f64,
}

impl AudioVisualizer {
    pub fn new() -> Self {
        Self {
            enabled: false,
            style: VisualizerStyle::Braille,
            bars: vec![0.0; 32],
            target_bars: vec![0.0; 32],
            last_tick: Instant::now(),
            seed: 0.0,
        }
    }

    pub fn toggle(&mut self) {
        self.enabled = !self.enabled;
    }

    pub fn is_enabled(&self) -> bool {
        self.enabled
    }

    pub fn tick(&mut self, is_playing: bool, width: u16) {
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

        if is_playing {
            self.seed += dt * 3.0;
            for (i, target) in self.target_bars.iter_mut().enumerate() {
                let phase = self.seed + i as f64 * 0.7;
                let wave1 = (phase * 1.1).sin() * 0.3;
                let wave2 = (phase * 2.3 + 1.0).sin() * 0.2;
                let wave3 = (phase * 0.7 + 2.0).cos() * 0.15;
                let base = 0.3 + wave1 + wave2 + wave3;
                let jitter = (phase * 5.0 + i as f64).sin() * 0.1;
                *target = (base + jitter).clamp(0.05, 1.0) as f32;
            }
        } else {
            for target in self.target_bars.iter_mut() {
                *target = 0.0;
            }
        }

        // Snap bars to target when paused (no sweep/decay animation)
        let decay = if is_playing { 0.25 } else { 1.0 };
        for (bar, target) in self.bars.iter_mut().zip(self.target_bars.iter()) {
            let diff = target - *bar;
            *bar += diff * decay as f32 * (dt * 60.0) as f32;
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

        let lines = match self.style {
            VisualizerStyle::Bars => self.render_bars(num_bars, h, theme),
            VisualizerStyle::Dots => self.render_dots(num_bars, h, theme),
            VisualizerStyle::Braille => self.render_braille(num_bars, h, theme),
            VisualizerStyle::Wave => self.render_wave(num_bars, h, theme),
        };
        Some(lines)
    }

    fn render_bars(&self, num_bars: usize, height: usize, theme: &AppTheme) -> Lines<'_> {
        let chars = ['▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];
        let mut lines: Vec<Line<'static>> = Vec::new();
        for row in (0..height).rev() {
            let mut spans: Vec<Span<'static>> = Vec::new();
            let threshold = (row + 1) as f32 / height as f32;
            for i in 0..num_bars {
                let val = *self.bars.get(i).unwrap_or(&0.0);
                if val >= threshold {
                    let seg = ((val - threshold) * height as f32).min(1.0);
                    let ci = (seg * (chars.len() - 1) as f32).round() as usize;
                    let ch = chars[ci.min(chars.len() - 1)];
                    let color = self.amplitude_color(val, theme);
                    spans.push(Span::styled(ch.to_string(), Style::default().fg(color)));
                } else {
                    spans.push(Span::raw(" "));
                }
            }
            lines.push(Line::from(spans));
        }
        Lines(lines)
    }

    fn render_dots(&self, num_bars: usize, height: usize, theme: &AppTheme) -> Lines<'_> {
        let mut lines: Vec<Line<'static>> = Vec::new();
        for row in (0..height).rev() {
            let mut spans: Vec<Span<'static>> = Vec::new();
            let threshold = (row + 1) as f32 / height as f32;
            for i in 0..num_bars {
                let val = *self.bars.get(i).unwrap_or(&0.0);
                if val >= threshold {
                    let color = self.amplitude_color(val, theme);
                    spans.push(Span::styled("●", Style::default().fg(color)));
                } else {
                    spans.push(Span::raw(" "));
                }
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

    fn render_wave(&self, num_bars: usize, height: usize, theme: &AppTheme) -> Lines<'_> {
        let mid = height / 2;
        let mut lines: Vec<Line<'static>> = Vec::new();
        for row in 0..height {
            let mut spans: Vec<Span<'static>> = Vec::new();
            let dist_from_mid = (row as isize - mid as isize).unsigned_abs() as f32;
            for i in 0..num_bars {
                let val = *self.bars.get(i).unwrap_or(&0.0);
                let wave_pos = val * mid as f32;
                if dist_from_mid <= wave_pos {
                    let proximity = 1.0 - (dist_from_mid / wave_pos.max(1.0));
                    let color = if proximity > 0.7 {
                        theme.accent
                    } else if proximity > 0.4 {
                        theme.fg_bright
                    } else {
                        theme.fg_dim
                    };
                    let ch = if dist_from_mid < 0.5 { '━' } else { '─' };
                    spans.push(Span::styled(ch.to_string(), Style::default().fg(color)));
                } else {
                    spans.push(Span::raw(" "));
                }
            }
            lines.push(Line::from(spans));
        }
        Lines(lines)
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
