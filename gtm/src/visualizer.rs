// Copyright (c) 2026
// Author: prjctimg <prjctimg@outlook.com>
// Audio visualizer with multiple render presets
//
// The spectrum model ports cliamp's log-frequency band layout (20 Hz–20 kHz,
// 10 bands) with per-column band interpolation, frame-capped easing, decay to
// rest while paused, and per-bar falling peak caps. Render presets add the
// iconic BarsDot / ClassicPeak / Columns / Wave / Stereo / Retro / Flame
// modes on top of the original bar modes.
//
// This is free software released under the GPL-3.0 license.

use std::time::Instant;

use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Widget;

use crate::theme::AppTheme;

// ─── Presets ────────────────────────────────────────────────────────────────

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum VisualizerPreset {
    #[default]
    Braille,
    Blocks,
    Mirror,
    Gradient,
    Spectrum,
    BarsDot,
    ClassicPeak,
    Columns,
    Wave,
    Stereo,
    Retro,
    Flame,
}

impl VisualizerPreset {
    pub fn all() -> &'static [VisualizerPreset] {
        &[
            VisualizerPreset::Braille,
            VisualizerPreset::Blocks,
            VisualizerPreset::Mirror,
            VisualizerPreset::Gradient,
            VisualizerPreset::Spectrum,
            VisualizerPreset::BarsDot,
            VisualizerPreset::ClassicPeak,
            VisualizerPreset::Columns,
            VisualizerPreset::Wave,
            VisualizerPreset::Stereo,
            VisualizerPreset::Retro,
            VisualizerPreset::Flame,
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
            VisualizerPreset::BarsDot => "BarsDot",
            VisualizerPreset::ClassicPeak => "ClassicPeak",
            VisualizerPreset::Columns => "Columns",
            VisualizerPreset::Wave => "Wave",
            VisualizerPreset::Stereo => "Stereo",
            VisualizerPreset::Retro => "Retro",
            VisualizerPreset::Flame => "Flame",
        }
    }
}

// ─── Spectrum band model (ported from cliamp) ───────────────────────────────

/// Number of log-frequency bands the spectrum maps onto before per-column
/// interpolation. Matches cliamp's `DefaultSpectrumBands`.
pub const BAND_COUNT: usize = 10;

/// cliamp legacy spectral band edges (Hz). The 10-band layout uses these
/// anchors directly; extra bands would log-interpolate between them.
const LEGACY_EDGES_HZ: [f64; BAND_COUNT + 1] = [
    20.0, 100.0, 200.0, 400.0, 800.0, 1600.0, 3200.0, 6400.0, 12800.0, 16000.0, 20000.0,
];

/// gtm's decode-side analyzer spans these frequencies across its log-spaced
/// `SPECTRUM_BINS`; mirror them so band edges map onto bin positions.
const SPECTRUM_MIN_HZ: f64 = 30.0;
const SPECTRUM_MAX_HZ: f64 = 16_000.0;

/// Cap on dt fed into easing — long hangs (sleep, paused frame) step like ~1
/// animation frame instead of integrating a huge interval (cliamp
/// `maxSmoothDtFrames`).
const MAX_SMOOTH_DT: f64 = 10.0 / 60.0;

/// Paused spectrum content below this is treated as fully decayed to rest,
/// letting the model fall through to the gentle idle animation.
const PAUSED_DECAY_EPSILON: f32 = 0.01;

/// Idle-pattern animation speed (radians/s drift).
const IDLE_DRIFT: f64 = 1.2;

// ─── ClassicPeak physics (cliamp vis_classic_peak) ──────────────────────────
const CLASSIC_LAUNCH_BASE: f32 = 0.8;
const CLASSIC_LAUNCH_GAIN: f32 = 1.4;
const CLASSIC_LAUNCH_MAX: f32 = 1.7;
const CLASSIC_GRAVITY: f32 = 9.5;
const CLASSIC_APEX_HOLD: f32 = 0.08;
const CLASSIC_PEAK_FALL: f32 = 2.4;

// ─── Stereo L/R meter (cliamp vis_stereo) ───────────────────────────────────
const STEREO_RISE: f32 = 3.2;
const STEREO_FALL: f32 = 0.8;
const STEREO_PEAK_HOLD: f32 = 0.45;
const STEREO_PEAK_FALL: f32 = 0.65;

// ─── Flame simulation (doom-fire, cliamp vis_flame) ─────────────────────────
const FLAME_MAX_COLS: usize = 320;

/// Braille dot bit map: `braille_bit[dot_row][dot_col]`.
const BRAILLE_BIT: [[u32; 2]; 4] = [[0x01, 0x08], [0x02, 0x10], [0x04, 0x20], [0x40, 0x80]];

/// Fractional block glyphs (bottom half), for meter/preview fills.
const BLOCKS: [&str; 8] = ["▁", "▂", "▃", "▄", "▅", "▆", "▇", "█"];

/// Fallback heat palette for Flame/Retro warm hues.
fn heat_color(heat: f32, theme: &AppTheme) -> Color {
    if heat > 0.78 {
        Color::Rgb(255, 250, 200)
    } else if heat > 0.58 {
        theme.tertiary_accent
    } else if heat > 0.38 {
        theme.secondary_accent
    } else if heat > 0.2 {
        theme.accent
    } else {
        theme.fg_dim
    }
}

// ─── Band helpers ───────────────────────────────────────────────────────────

/// Map an Hz edge onto the log-spaced index space of the analyzer's bins.
fn freq_to_bin_pos(freq: f64, bins: usize) -> f64 {
    if bins == 0 {
        return 0.0;
    }
    let last = (bins - 1) as f64;
    let t =
        ((freq / SPECTRUM_MIN_HZ).ln() / (SPECTRUM_MAX_HZ / SPECTRUM_MIN_HZ).ln()).clamp(0.0, 1.0);
    t * last
}

/// Linearly sample the log bins at fractional position `pos`.
fn sample_bin_linear(bins: &[f32], pos: f64) -> f32 {
    let n = bins.len();
    if n == 0 {
        return 0.0;
    }
    if n == 1 {
        return bins[0];
    }
    let last = (n - 1) as f64;
    let pos = pos.clamp(0.0, last);
    let idx = pos.floor() as usize;
    if idx >= n - 1 {
        return bins[n - 1];
    }
    let frac = (pos - idx as f64) as f32;
    bins[idx] * (1.0 - frac) + bins[idx + 1] * frac
}

/// Average the bins over `[lo, hi]` (fractional positions) like cliamp's
/// `averageSpectrumRangeLinear`: a few evenly spread interpolated samples.
fn average_bins_linear(bins: &[f32], lo: f64, hi: f64) -> f32 {
    let n = bins.len();
    if n == 0 {
        return 0.0;
    }
    let last = (n - 1) as f64;
    let lo = lo.clamp(0.0, last);
    let hi = hi.clamp(lo, last);
    let span = hi - lo;
    if span <= 0.0 {
        return sample_bin_linear(bins, lo);
    }
    let samples = 8usize.max((span as usize).min(32));
    let mut sum = 0.0f64;
    for i in 0..samples {
        let t = (i as f64 + 0.5) / samples as f64;
        sum += sample_bin_linear(bins, lo + t * span) as f64;
    }
    (sum / samples as f64) as f32
}

/// Linearly resample the smoothed bands to exactly `total_cols` per-column
/// levels (cliamp `resampleBandsLinear`): adjacent columns vary slightly,
/// giving dense, organic bars across the full panel width.
fn resample_bands_linear(bands: &[f32], total_cols: usize) -> Vec<f32> {
    if total_cols == 0 || bands.is_empty() {
        return Vec::new();
    }
    if bands.len() == total_cols {
        return bands.to_vec();
    }
    let mut out = Vec::with_capacity(total_cols);
    if total_cols == 1 {
        out.push(sample_band_linear(bands, (bands.len() - 1) as f64 / 2.0));
        return out;
    }
    let last = (bands.len() - 1) as f64;
    for col in 0..total_cols {
        let pos = col as f64 / (total_cols - 1) as f64 * last;
        out.push(sample_band_linear(bands, pos));
    }
    out
}

// ─── Visualizer ─────────────────────────────────────────────────────────────

pub struct AudioVisualizer {
    pub enabled: bool,
    pub preset: VisualizerPreset,
    // Column bar model (one level per terminal column)
    bars: Vec<f32>,
    target_cols: Vec<f32>,
    peaks: Vec<f32>,
    // Log-band model (BAND_COUNT entries, smoothed)
    bands: [f32; BAND_COUNT],
    bands_prev: [f32; BAND_COUNT],
    // Waveform ring + stereo flag (mirrors DaemonState)
    wave_samples: Vec<f32>,
    wave_stereo: bool,
    // ClassicPeak per-column physics
    peak_pos: Vec<f32>,
    peak_vel: Vec<f32>,
    peak_hold: Vec<f32>,
    bar_prev: Vec<f32>,
    // Stereo L/R meter
    stereo_level: [f32; 2],
    stereo_peak: [f32; 2],
    stereo_hold: f32,
    // Flame heat field
    heat: Vec<f64>,
    heat_rows: usize,
    heat_cols: usize,
    rng: u64,
    // Motion state
    last_tick: Instant,
    spectrum_offset: f64,
    frame: u64,
    resting: bool,
}

impl AudioVisualizer {
    pub fn new() -> Self {
        Self {
            enabled: false,
            preset: VisualizerPreset::default(),
            bars: vec![0.0; 32],
            target_cols: vec![0.0; 32],
            peaks: vec![0.0; 32],
            bands: [0.0; BAND_COUNT],
            bands_prev: [0.0; BAND_COUNT],
            wave_samples: Vec::new(),
            wave_stereo: false,
            peak_pos: vec![0.0; 32],
            peak_vel: vec![0.0; 32],
            peak_hold: vec![0.0; 32],
            bar_prev: vec![0.0; 32],
            stereo_level: [0.0; 2],
            stereo_peak: [0.0; 2],
            stereo_hold: 0.0,
            heat: Vec::new(),
            heat_rows: 0,
            heat_cols: 0,
            rng: 0xF1A3C0DE0BADCAFE,
            last_tick: Instant::now(),
            spectrum_offset: 0.0,
            frame: 0,
            resting: true,
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

    /// Advance the band/bar model one frame. `width`/`height` come from the
    /// visualizer panel; `audio_levels` is the 64-bin spectrum, `wave_samples`
    /// a decimated interleaved L/R ring (empty when silent) and `wave_stereo`
    /// whether the current source has two distinct channels.
    pub fn tick(
        &mut self,
        is_playing: bool,
        width: u16,
        height: u16,
        audio_levels: &[f32],
        wave_samples: &[f32],
        wave_stereo: bool,
    ) {
        if !self.enabled || width == 0 {
            return;
        }
        let now = Instant::now();
        let dt = now
            .duration_since(self.last_tick)
            .as_secs_f64()
            .clamp(0.0, MAX_SMOOTH_DT);
        self.last_tick = now;
        self.frame = self.frame.wrapping_add(1);
        self.spectrum_offset += dt * IDLE_DRIFT;
        let dt = dt as f32;

        let num = width as usize;
        if self.bars.len() != num {
            self.bars.resize(num, 0.0);
            self.target_cols.resize(num, 0.0);
            self.peaks.resize(num, 0.0);
            self.peak_pos.resize(num, 0.0);
            self.peak_vel.resize(num, 0.0);
            self.peak_hold.resize(num, 0.0);
            self.bar_prev.resize(num, 0.0);
        }

        // 1) Log-band targets (analysis smoothing: fast attack, slow decay).
        if is_playing && !audio_levels.is_empty() {
            self.resting = false;
            self.bands_prev.copy_from_slice(&self.bands);
            let edges = band_edges_hz_to_bins(audio_levels.len());
            for (b, slot) in self.bands.iter_mut().enumerate() {
                let level = average_bins_linear(audio_levels, edges[b], edges[b + 1]);
                let prev = self.bands_prev[b];
                *slot = if level > prev {
                    level * 0.6 + prev * 0.4
                } else {
                    level * 0.25 + prev * 0.75
                };
            }
        } else {
            // Decay to rest; once fully quiet, let the idle animation drive.
            let mut peaked = 0.0f32;
            for slot in self.bands.iter_mut() {
                *slot *= 0.78 + dt * 0.2;
                peaked = peaked.max(*slot);
            }
            if peaked < PAUSED_DECAY_EPSILON {
                self.resting = true;
            }
        }

        // 2) Idle sine-pattern targets once fully rested.
        let mut band_levels = [0.0f32; BAND_COUNT];
        if self.resting {
            for (i, slot) in band_levels.iter_mut().enumerate() {
                let phase = self.spectrum_offset + i as f64 * 0.45;
                let wave_ = 0.5 + 0.5 * phase.sin();
                let jitter = (self.spectrum_offset * 3.7 + i as f64 * 1.3).sin() * 0.02;
                let edge = (i as f64 / (BAND_COUNT - 1) as f64 * std::f64::consts::PI)
                    .sin()
                    .max(0.15);
                *slot = ((0.10 + 0.12 * wave_ + jitter) * edge) as f32;
            }
        } else {
            band_levels.copy_from_slice(&self.bands);
        }

        // 3) Interpolate bands → one target per terminal column.
        let cols = resample_bands_linear(&band_levels, num);
        for (i, c) in cols.into_iter().enumerate() {
            self.target_cols[i] = c.clamp(0.0, 1.0);
        }

        // 4) Per-column easing, frame-capped via the clamped dt.
        let dt60 = dt * 60.0;
        let attack = 0.65;
        let decay = if is_playing { 0.45 } else { 0.25 };
        for i in 0..num {
            let target = self.target_cols[i];
            let diff = target - self.bars[i];
            let rate = if diff > 0.0 { attack } else { decay };
            self.bars[i] = (self.bars[i] + diff * rate * dt60).clamp(0.0, 1.0);
        }

        // 5) Per-bar falling peak caps.
        for i in 0..num {
            let p = self.peaks[i] - dt * 0.55;
            self.peaks[i] = p.max(self.bars[i]).clamp(0.0, 1.0);
        }

        // 6) Waveform ring + stereo meter levels.
        self.wave_samples.clear();
        self.wave_samples.extend_from_slice(wave_samples);
        self.wave_stereo = wave_stereo;
        self.step_stereo(dt);

        // 7) ClassicPeak cap physics.
        self.step_classic_peaks(dt);

        // 8) Flame field (doom-fire propagation).
        self.step_flame(dt, num, height as usize);
    }

    /// Per-lane RMS → smoothed level with a peak cap that holds briefly then
    /// falls (cliamp stereoDriver).
    fn step_stereo(&mut self, dt: f32) {
        let mut acc = [0.0f32; 2];
        let mut n = [0usize; 2];
        // `as_chunks`, clippy's suggested fix for constant-size chunks, is
        // nightly-only; keep the stable chunks_exact form.
        #[allow(clippy::chunks_exact_to_as_chunks)]
        for pair in self.wave_samples.chunks_exact(2) {
            acc[0] += pair[0] * pair[0];
            acc[1] += pair[1] * pair[1];
            n[0] += 1;
            n[1] += 1;
        }
        let mut rms = [0.0f32; 2];
        for (i, n_i) in n.iter().enumerate() {
            rms[i] = if *n_i > 0 {
                (acc[i] / *n_i as f32).sqrt()
            } else {
                0.0
            };
            let diff = rms[i] - self.stereo_level[i];
            let rate = if diff > 0.0 { STEREO_RISE } else { STEREO_FALL };
            self.stereo_level[i] = (self.stereo_level[i] + diff * rate * dt).clamp(0.0, 1.0);
        }
        if self.stereo_hold > 0.0 {
            self.stereo_hold -= dt;
        } else {
            for (i, peak) in self.stereo_peak.iter_mut().enumerate() {
                *peak = (*peak - STEREO_PEAK_FALL * dt).max(rms[i]);
            }
        }
        if rms[0] >= self.stereo_peak[0] || rms[1] >= self.stereo_peak[1] {
            self.stereo_hold = STEREO_PEAK_HOLD;
            self.stereo_peak[0] = self.stereo_peak[0].max(rms[0]);
            self.stereo_peak[1] = self.stereo_peak[1].max(rms[1]);
        }
    }

    /// ClassicPeak caps: a cap rides its bar while the bar climbs; a fresh
    /// rise launches it upward, gravity slows it, an apex hold pauses it, and
    /// it falls back onto the bar body (cliamp classicPeakDriver).
    fn step_classic_peaks(&mut self, dt: f32) {
        let n = self.bars.len();
        for i in 0..n {
            let target = self.bars[i];
            let rise = target - self.bar_prev[i];
            self.bar_prev[i] = target;

            let mut pos = self.peak_pos[i];
            let mut vel = self.peak_vel[i];
            let mut hold = self.peak_hold[i];

            let at_rest = (pos - target).abs() < 0.01;
            if rise > 0.01 && hold <= 0.0 && vel <= 0.0 && at_rest {
                vel = (CLASSIC_LAUNCH_BASE + rise * CLASSIC_LAUNCH_GAIN).min(CLASSIC_LAUNCH_MAX);
            }
            if hold > 0.0 {
                hold -= dt;
            } else if vel > 0.0 {
                pos += vel * dt;
                vel -= CLASSIC_GRAVITY * dt;
                if vel <= 0.0 {
                    hold = CLASSIC_APEX_HOLD;
                }
            } else if pos > target + 0.005 {
                pos -= CLASSIC_PEAK_FALL * dt;
                if pos <= target {
                    pos = target;
                }
            }
            if pos < target {
                pos = target;
            }
            self.peak_pos[i] = pos;
            self.peak_vel[i] = vel;
            self.peak_hold[i] = hold;
        }
    }

    /// Doom-fire propagation: every cell inherits the hotter of the two cells
    /// below with a lateral wind jitter and a random decay; the bottom row is
    /// seeded from the spectrum plus an ember floor.
    fn step_flame(&mut self, dt: f32, width: usize, height: usize) {
        let cols = (width * 2).min(FLAME_MAX_COLS);
        let rows = (height * 4).max(1);
        if cols < 4 || rows < 4 {
            return;
        }
        if self.heat_rows != rows || self.heat_cols != cols {
            self.heat = vec![0.0f64; rows * cols];
            self.heat_rows = rows;
            self.heat_cols = cols;
        }
        let dt60 = (dt * 60.0) as f64;
        let last = cols - 1;
        // Data flows upward: read row y+1 (this frame's state) while writing
        // row y, iterating top → bottom so sources stay unmodified.
        for y in 0..(rows - 1) {
            let src = y + 1;
            for x in 0..cols {
                let jitter = (self.next_rng() % 3) as isize - 1;
                let jx = ((x as isize + jitter).clamp(0, last as isize)) as usize;
                let hot = self.heat[src * cols + x].max(self.heat[src * cols + jx]);
                let decay =
                    1.0 - (0.18 + 0.22 * (self.next_rng() % 100) as f64 / 100.0) * dt60.min(1.5);
                self.heat[y * cols + x] = hot * decay;
            }
        }
        // Top rows cool hardest.
        for x in 0..cols {
            self.heat[x] *= 0.82;
        }
        // Seed the bottom (source) row from the spectrum + ember base.
        let band_last = (BAND_COUNT - 1) as f64;
        for x in 0..cols {
            let t = x as f64 / last as f64 * band_last;
            let band = if self.resting {
                0.0
            } else {
                sample_band_linear_f64(&self.bands, t)
            };
            let sparkle = (self.next_rng() % 100) as f64 / 100.0 * 0.18;
            let base = 0.30 + 0.70 * band + sparkle;
            self.heat[(rows - 1) * cols + x] = base.min(1.05);
        }
    }

    fn next_rng(&mut self) -> u64 {
        self.rng = self
            .rng
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        self.rng >> 33
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
        let num_bars = num_bars.max(1);

        Some(match self.preset {
            VisualizerPreset::Braille => self.render_braille(num_bars, h, theme),
            VisualizerPreset::Blocks => self.render_blocks(num_bars, h, theme),
            VisualizerPreset::Mirror => self.render_mirror(num_bars, h, theme),
            VisualizerPreset::Gradient => self.render_gradient(num_bars, h, theme),
            VisualizerPreset::Spectrum => self.render_spectrum(num_bars, h, theme),
            VisualizerPreset::BarsDot => self.render_bars_dot(num_bars, h, theme),
            VisualizerPreset::ClassicPeak => self.render_classic_peak(num_bars, h, theme),
            VisualizerPreset::Columns => self.render_columns(num_bars, h, theme),
            VisualizerPreset::Wave => self.render_wave(num_bars, h, theme),
            VisualizerPreset::Stereo => self.render_stereo(num_bars, h, theme),
            VisualizerPreset::Retro => self.render_retro(num_bars, h, theme),
            VisualizerPreset::Flame => self.render_flame(num_bars, h, theme),
        })
    }

    /// Braille-stippled bars: each cell maps to a 4×2 dot grid filled from the
    /// bottom to the bar level (cliamp renderBarsDot).
    fn render_bars_dot(&self, num_bars: usize, height: usize, theme: &AppTheme) -> Lines<'static> {
        let dot_rows = height * 4;
        let mut lines: Vec<Line<'static>> = Vec::new();
        for row in 0..height {
            let base = row * 4;
            let mut spans: Vec<Span<'static>> = Vec::new();
            for i in 0..num_bars {
                let val = *self.bars.get(i).unwrap_or(&0.0);
                let mut braille: u32 = 0x2800;
                for (dr, bits) in BRAILLE_BIT.iter().enumerate() {
                    let dot_y = (dot_rows - 1) as f64 - (base + dr) as f64;
                    let norm = dot_y / (dot_rows - 1).max(1) as f64;
                    if norm < val as f64 {
                        braille |= bits[0] | bits[1];
                    }
                }
                let ch = char::from_u32(braille).unwrap_or('⠀');
                spans.push(Span::styled(
                    ch.to_string(),
                    Style::default().fg(self.amplitude_color(val, theme)),
                ));
            }
            lines.push(Line::from(spans));
        }
        Lines(lines)
    }

    /// Classic falling peak caps over thin columns, with launch/gravity/cap
    /// physics (cliamp rendering: cap glyphs `⎺⎻⎼⎽` above `▏` bodies).
    fn render_classic_peak(
        &self,
        num_bars: usize,
        height: usize,
        theme: &AppTheme,
    ) -> Lines<'static> {
        let peak_glyphs: [&str; 4] = ["⎺", "⎻", "⎼", "⎽"];
        let mut lines: Vec<Line<'static>> = Vec::new();
        for row in 0..height {
            let mut spans: Vec<Span<'static>> = Vec::new();
            let row_from_top = height - 1 - row;
            for i in 0..num_bars {
                let val = *self.bars.get(i).unwrap_or(&0.0);
                let cap = *self.peak_pos.get(i).unwrap_or(&0.0);
                let filled = (val * height as f32).floor() as usize;
                let cap_row = (cap * height as f32).floor() as usize;
                let (glyph, color) = if row_from_top < filled {
                    ("█", self.amplitude_color(val, theme))
                } else if cap > val + 0.005 && cap_row == row_from_top && cap_row < height {
                    let idx = if self.peak_vel[i] > 0.2 {
                        0
                    } else if self.peak_hold[i] > 0.0 {
                        1
                    } else {
                        3
                    };
                    (peak_glyphs[idx], theme.accent)
                } else {
                    (" ", theme.bg)
                };
                spans.push(Span::styled(glyph, Style::default().fg(color)));
            }
            lines.push(Line::from(spans));
        }
        Lines(lines)
    }

    /// Dense thin interpolated columns (cliamp renderColumns): one solid glyph
    /// per terminal column with subtle per-column variation.
    fn render_columns(&self, num_bars: usize, height: usize, theme: &AppTheme) -> Lines<'static> {
        let mut lines: Vec<Line<'static>> = Vec::new();
        for row in 0..height {
            let row_from_top = height - 1 - row;
            let mut spans: Vec<Span<'static>> = Vec::new();
            for i in 0..num_bars {
                let val = *self.bars.get(i).unwrap_or(&0.0);
                let filled = (val * height as f32).floor() as usize;
                let (glyph, color) = if row_from_top < filled {
                    ("█", self.amplitude_color(val, theme))
                } else {
                    (" ", theme.bg)
                };
                spans.push(Span::styled(glyph, Style::default().fg(color)));
            }
            lines.push(Line::from(spans));
        }
        Lines(lines)
    }

    /// Braille oscilloscope over the decimated waveform ring (cliamp
    /// renderWave); falls back to a gentle sine sweep when no fresh samples
    /// are available (paused/stopped).
    fn render_wave(&self, num_bars: usize, height: usize, theme: &AppTheme) -> Lines<'static> {
        let dot_rows = height * 4;
        let dot_cols = num_bars * 2;
        if dot_cols == 0 {
            return Lines(Vec::new());
        }
        // Per-dot-column y position.
        let mut ypos = vec![0usize; dot_cols];
        let n = self.wave_samples.len();
        for (x, slot) in ypos.iter_mut().enumerate() {
            let v = if n >= 2 {
                let idx = ((x as f64 * n as f64 / dot_cols as f64) as usize / 2 * 2).min(n - 2);
                self.wave_samples[idx] as f64
            } else {
                // Idle sine sweep.
                let phase = self.spectrum_offset * 1.6 + x as f64 * 0.16;
                0.55 * phase.sin() + 0.15 * (phase * 2.7 + 0.7).sin()
            };
            let y = ((1.0 - v) * (dot_rows - 1) as f64 / 2.0).round();
            *slot = (y as isize).clamp(0, (dot_rows - 1) as isize) as usize;
        }
        let mut lines: Vec<Line<'static>> = Vec::new();
        for row in 0..height {
            let base = row * 4;
            let mut spans: Vec<Span<'static>> = Vec::new();
            for ch in 0..num_bars {
                let mut braille: u32 = 0x2800;
                for (dr, bits) in BRAILLE_BIT.iter().enumerate() {
                    let dot_y = base + dr;
                    for (dc, bit) in bits.iter().enumerate() {
                        let x = ch * 2 + dc;
                        let y = ypos[x];
                        let prev_y = if x > 0 { ypos[x - 1] } else { y };
                        let (lo, hi) = (y.min(prev_y), y.max(prev_y));
                        if dot_y >= lo && dot_y <= hi {
                            braille |= *bit;
                        }
                    }
                }
                let ch_glyph = char::from_u32(braille).unwrap_or('⠀');
                spans.push(Span::styled(
                    ch_glyph.to_string(),
                    Style::default().fg(theme.accent),
                ));
            }
            lines.push(Line::from(spans));
        }
        Lines(lines)
    }

    /// Stereo L/R horizontal LED peak meters (cliamp vis_stereo): one meter
    /// per lane, label + fractional fill + a holding peak marker.
    fn render_stereo(&self, num_bars: usize, height: usize, theme: &AppTheme) -> Lines<'static> {
        let meter_w = num_bars.saturating_sub(3);
        if meter_w == 0 {
            return Lines(vec![Line::from(" "); height]);
        }
        let rows_used = height.min(2);
        let pad_top = height.saturating_sub(rows_used) / 2;

        let mut lines: Vec<Line<'static>> = Vec::new();
        for _ in 0..pad_top {
            lines.push(Line::from(" "));
        }
        for lane in 0..rows_used {
            let mut spans = Vec::with_capacity(3 + meter_w);
            let label = if height == 1 && lane == 0 {
                "ST".to_string()
            } else if lane == 0 {
                "L ".to_string()
            } else {
                "R ".to_string()
            };
            spans.push(Span::styled(
                label,
                Style::default()
                    .fg(theme.fg_dim)
                    .add_modifier(ratatui::style::Modifier::BOLD),
            ));
            let level = self.stereo_level[lane];
            let peak = self.stereo_peak[lane];
            let full = level * meter_w as f32;
            let full_rows = full.floor() as usize;
            let frac = full - full.floor();
            let peak_col = (peak * meter_w as f32).round() as usize;
            for x in 0..meter_w {
                let (glyph, color) = if x < full_rows {
                    let idx = if level > 0.8 {
                        7
                    } else if level > 0.5 {
                        5
                    } else {
                        3
                    };
                    (BLOCKS[idx], self.amplitude_color(level, theme))
                } else if x == full_rows && frac > 0.001 && full_rows < meter_w {
                    (
                        BLOCKS[((frac * 8.0) as usize).min(7)],
                        self.amplitude_color(level, theme),
                    )
                } else if peak > 0.0 && x == peak_col && peak_col >= full_rows {
                    ("·", theme.accent)
                } else {
                    (" ", theme.bg)
                };
                spans.push(Span::styled(glyph, Style::default().fg(color)));
            }
            lines.push(Line::from(spans));
        }
        while lines.len() < height {
            lines.push(Line::from(" "));
        }
        Lines(lines)
    }

    /// 80s synthwave scene: striped sun, horizon, scrolling perspective grid
    /// and an audio-reactive wave, rendered through Braille cells (cliamp
    /// renderRetro).
    fn render_retro(&self, num_bars: usize, height: usize, theme: &AppTheme) -> Lines<'static> {
        let char_cols = num_bars;
        let dot_rows = height * 4;
        let dot_cols = char_cols * 2;
        let mut lines: Vec<Line<'static>> = Vec::new();
        if dot_rows < 4 {
            return lines_empty(height);
        }
        let horizon = (dot_rows * 2 / 5).max(2);
        let floor_rows = dot_rows.saturating_sub(horizon).max(1);
        let center_x = (dot_cols.saturating_sub(1) as f64) / 2.0;
        let mut grid = vec![0u8; dot_rows * dot_cols];

        // Striped setting sun.
        let sun_r = horizon as f64 * 0.85;
        for dy in 0..horizon {
            let row_dist = (horizon - dy) as f64;
            if row_dist > sun_r {
                continue;
            }
            let half_w = (sun_r * sun_r - row_dist * row_dist).sqrt();
            if row_dist < sun_r * 0.5 {
                let sw = (sun_r * 0.15).max(1.0) as usize;
                if ((row_dist as usize) / sw) % 2 == 1 {
                    continue;
                }
            }
            let left = ((center_x - half_w) as isize).max(0) as usize;
            let right = ((center_x + half_w) as usize).min(dot_cols - 1);
            for dx in left..=right {
                grid[dy * dot_cols + dx] = 3;
            }
        }
        // Horizon line.
        for dx in 0..dot_cols {
            grid[horizon * dot_cols + dx] = 1;
        }
        // Perspective vertical lines converging to the vanishing point.
        const NVL: usize = 18;
        for i in 0..=NVL {
            let bottom_x = i as f64 * (dot_cols.saturating_sub(1) as f64) / NVL as f64;
            for dy in (horizon + 1)..dot_rows {
                let t = (dy - horizon) as f64 / (floor_rows - 1) as f64;
                let sx = (center_x + (bottom_x - center_x) * t).round() as isize;
                if sx >= 0 && (sx as usize) < dot_cols {
                    grid[dy * dot_cols + sx as usize] = 1;
                }
            }
        }
        // Scrolling horizontal lines (quadratic perspective).
        let scroll = (self.frame as f64 * 0.08) % 1.0;
        const NHL: usize = 10;
        for i in 0..NHL {
            let mut z = (i as f64 + scroll) / NHL as f64;
            if z > 1.0 {
                z -= 1.0;
            }
            let dy = horizon + 1 + (z * z * (floor_rows.saturating_sub(2).max(1) as f64)) as usize;
            if dy > horizon && dy < dot_rows {
                for dx in 0..dot_cols {
                    grid[dy * dot_cols + dx] = 1;
                }
            }
        }
        // Audio-reactive wave riding the horizon.
        let max_wave = horizon as f64 * 0.85;
        let band_last = (BAND_COUNT - 1) as f64;
        let mut wave_y = vec![0usize; dot_cols];
        for (dx, slot) in wave_y.iter_mut().enumerate() {
            let bf = dx as f64 / (dot_cols.saturating_sub(1).max(1) as f64) * band_last;
            let bi = (bf as usize).min(BAND_COUNT - 1);
            let frac = bf - bi as f64;
            let level = if bi >= BAND_COUNT - 1 {
                self.bands[BAND_COUNT - 1] as f64
            } else {
                let t = (1.0 - (frac * std::f64::consts::PI).cos()) / 2.0;
                self.bands[bi] as f64 * (1.0 - t) + self.bands[bi + 1] as f64 * t
            };
            let level = level.max(0.03);
            let wy = (horizon as f64 - level * max_wave).round() as isize;
            *slot = wy.clamp(0, (dot_rows - 1) as isize) as usize;
        }
        for (dx, &y) in wave_y.iter().enumerate() {
            grid[y * dot_cols + dx] = 2;
            if dx > 0 {
                let (lo, hi) = (y.min(wave_y[dx - 1]), y.max(wave_y[dx - 1]));
                for fy in lo..=hi {
                    grid[fy * dot_cols + dx] = 2;
                }
            }
        }

        // Render braille cells; colour priority: wave > sun > grid.
        for row in 0..height {
            let base = row * 4;
            let mut spans: Vec<Span<'static>> = Vec::new();
            for ch in 0..char_cols {
                let mut braille: u32 = 0x2800;
                let mut has_wave = false;
                let mut has_sun = false;
                for (dr, bits) in BRAILLE_BIT.iter().enumerate() {
                    let dy = base + dr;
                    for (dc, bit) in bits.iter().enumerate() {
                        let dx = ch * 2 + dc;
                        if dy >= dot_rows || dx >= dot_cols {
                            continue;
                        }
                        match grid[dy * dot_cols + dx] {
                            1 => braille |= *bit,
                            2 => {
                                braille |= *bit;
                                has_wave = true;
                            }
                            3 => {
                                braille |= *bit;
                                has_sun = true;
                            }
                            _ => {}
                        }
                    }
                }
                let color = if has_wave {
                    theme.secondary_accent
                } else if has_sun {
                    theme.accent
                } else {
                    theme.fg_dim
                };
                let glyph = char::from_u32(braille).unwrap_or('⠀');
                spans.push(Span::styled(glyph.to_string(), Style::default().fg(color)));
            }
            lines.push(Line::from(spans));
        }
        Lines(lines)
    }

    /// Doom-fire heat field rendered through Braille cells, coloured by heat
    /// (cliamp vis_flame). The simulation runs in `tick`; this only rasterises.
    fn render_flame(&self, num_bars: usize, height: usize, theme: &AppTheme) -> Lines<'static> {
        let mut lines: Vec<Line<'static>> = Vec::new();
        let (rows, cols) = (self.heat_rows, self.heat_cols);
        if rows == 0 || cols == 0 {
            // No field yet (never ticked at this size): steady ember strip.
            let mut spans: Vec<Span<'static>> = Vec::new();
            for i in 0..num_bars {
                let val = *self.bands.get(i % BAND_COUNT).unwrap_or(&0.0);
                spans.push(Span::styled(
                    "⣀",
                    Style::default().fg(heat_color(0.3 + val, theme)),
                ));
            }
            lines.push(Line::from(spans));
            while lines.len() < height {
                lines.push(Line::from(" "));
            }
            return Lines(lines);
        }
        for row in 0..height {
            let base = row * 4;
            let mut spans: Vec<Span<'static>> = Vec::new();
            for ch in 0..num_bars {
                let mut braille: u32 = 0x2800;
                let mut hsum = 0.0f64;
                let mut n = 0usize;
                for (dr, bits) in BRAILLE_BIT.iter().enumerate() {
                    let gy = base + dr;
                    if gy >= rows {
                        continue;
                    }
                    for (dc, bit) in bits.iter().enumerate() {
                        let gx = ch * 2 + dc;
                        if gx >= cols {
                            continue;
                        }
                        let heat = self.heat[gy * cols + gx];
                        hsum += heat;
                        n += 1;
                        // Bottom dots light first as heat climbs.
                        let threshold = 0.12 + dr as f64 * 0.22;
                        if heat >= threshold {
                            braille |= *bit;
                        }
                    }
                }
                let avg = if n > 0 { (hsum / n as f64) as f32 } else { 0.0 };
                let glyph = char::from_u32(braille).unwrap_or('⠀');
                spans.push(Span::styled(
                    glyph.to_string(),
                    Style::default().fg(heat_color(avg, theme)),
                ));
            }
            lines.push(Line::from(spans));
        }
        Lines(lines)
    }

    /// Log-mapped spectrum bars with peak-hold caps. Distinct from Blocks
    /// (fractional smooth columns): solid columns plus a falling `▔` marker
    /// per bar. Glyphs are `&'static str` — no per-frame allocation.
    fn render_spectrum(&self, num_bars: usize, height: usize, theme: &AppTheme) -> Lines<'_> {
        let mut lines: Vec<Line<'static>> = Vec::new();
        for row_from_top in (0..height).rev() {
            let mut spans: Vec<Span<'static>> = Vec::new();
            for i in 0..num_bars {
                let val = *self.bars.get(i).unwrap_or(&0.0);
                let peak = *self.peaks.get(i).unwrap_or(&0.0);
                let filled_rows = (val * height as f32).floor() as usize;
                let peak_row = ((peak * height as f32).floor() as usize).min(height);
                let (glyph, color) = if row_from_top < filled_rows {
                    ("█", self.amplitude_color(val, theme))
                } else if peak > 0.0 && row_from_top == peak_row && peak_row < height {
                    ("▔", theme.accent)
                } else {
                    (" ", theme.bg)
                };
                spans.push(Span::styled(glyph, Style::default().fg(color)));
            }
            lines.push(Line::from(spans));
        }
        Lines(lines)
    }

    /// Block columns with fine fractional heights (`▁…█`).
    fn render_blocks(&self, num_bars: usize, height: usize, theme: &AppTheme) -> Lines<'_> {
        const B: [&str; 8] = ["▁", "▂", "▃", "▄", "▅", "▆", "▇", "█"];
        let mut lines: Vec<Line<'static>> = Vec::new();
        for row_from_top in (0..height).rev() {
            let mut spans: Vec<Span<'static>> = Vec::new();
            for i in 0..num_bars {
                let val = *self.bars.get(i).unwrap_or(&0.0);
                let filled = val * height as f32;
                let full_rows = filled.floor() as usize;
                let frac = filled - filled.floor();
                let glyph = if row_from_top < full_rows {
                    B[7]
                } else if row_from_top == full_rows && frac > 0.0 && full_rows < height {
                    B[((frac * 8.0) as usize).min(7)]
                } else {
                    " "
                };
                let color = if glyph == " " {
                    theme.bg
                } else {
                    self.amplitude_color(val, theme)
                };
                spans.push(Span::styled(glyph, Style::default().fg(color)));
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
                let (glyph, color) = if center_dist <= extent {
                    ("█", self.amplitude_color(val, theme))
                } else {
                    (" ", theme.bg)
                };
                spans.push(Span::styled(glyph, Style::default().fg(color)));
            }
            lines.push(Line::from(spans));
        }
        Lines(lines)
    }

    /// Braille cells; color chosen per-bar by `color_fn`.
    fn render_braille_grid(
        &self,
        num_bars: usize,
        height: usize,
        theme: &AppTheme,
        color_fn: impl Fn(f32, &AppTheme) -> Color,
    ) -> Lines<'_> {
        let mut lines: Vec<Line<'static>> = Vec::new();
        for row in (0..height).rev() {
            let mut spans: Vec<Span<'static>> = Vec::new();
            let threshold = (row + 1) as f32 / height as f32;
            for i in 0..num_bars {
                let val = *self.bars.get(i).unwrap_or(&0.0);
                if val >= threshold {
                    let color = color_fn(val, theme);
                    spans.push(Span::styled("⣿", Style::default().fg(color)));
                } else {
                    spans.push(Span::styled("⠀", Style::default().fg(theme.bg)));
                }
            }
            lines.push(Line::from(spans));
        }
        Lines(lines)
    }

    /// Braille cells with a 4-step color ramp by amplitude.
    fn render_gradient(&self, num_bars: usize, height: usize, theme: &AppTheme) -> Lines<'_> {
        self.render_braille_grid(num_bars, height, theme, |val, theme| {
            if val > 0.75 {
                theme.accent
            } else if val > 0.5 {
                theme.fg_bright
            } else if val > 0.25 {
                theme.fg
            } else {
                theme.fg_dim
            }
        })
    }

    fn render_braille(&self, num_bars: usize, height: usize, theme: &AppTheme) -> Lines<'_> {
        self.render_braille_grid(num_bars, height, theme, |val, theme| {
            self.amplitude_color(val, theme)
        })
    }
}

fn lines_empty(height: usize) -> Lines<'static> {
    Lines((0..height).map(|_| Line::from(" ")).collect())
}

/// Linearly sample `[f32; N]` band values at fractional position `pos`.
fn sample_band_linear(bands: &[f32], pos: f64) -> f32 {
    let n = bands.len();
    if n == 0 {
        return 0.0;
    }
    if n == 1 {
        return bands[0];
    }
    let last = (n - 1) as f64;
    let pos = pos.clamp(0.0, last);
    let idx = pos.floor() as usize;
    if idx >= n - 1 {
        return bands[n - 1];
    }
    let frac = (pos - idx as f64) as f32;
    bands[idx] * (1.0 - frac) + bands[idx + 1] * frac
}

/// Linearly sample `[f32; N]` band values at fractional position `t`.
fn sample_band_linear_f64(bands: &[f32], t: f64) -> f64 {
    let n = bands.len();
    if n == 0 {
        return 0.0;
    }
    if n == 1 {
        return bands[0] as f64;
    }
    let last = (n - 1) as f64;
    let pos = t.clamp(0.0, last);
    let idx = pos.floor() as usize;
    if idx >= n - 1 {
        return bands[n - 1] as f64;
    }
    let frac = pos - idx as f64;
    bands[idx] as f64 * (1.0 - frac) + bands[idx + 1] as f64 * frac
}

/// Band edges (Hz) → fractional bin positions for the current bin count.
fn band_edges_hz_to_bins(bins: usize) -> Vec<f64> {
    LEGACY_EDGES_HZ
        .iter()
        .map(|&hz| freq_to_bin_pos(hz, bins))
        .collect()
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
                // Slice glyphs out of the span instead of `char.to_string()`:
                // zero per-cell allocation on the render hot path.
                for (idx, ch) in span.content.char_indices() {
                    if x >= area.x + area.width {
                        break;
                    }
                    let cell_y = area.y + row as u16;
                    if x >= buf.area.width || cell_y >= buf.area.height {
                        break;
                    }
                    if let Some(cell) = buf.cell_mut((x, cell_y)) {
                        cell.set_symbol(&span.content[idx..idx + ch.len_utf8()])
                            .set_style(span.style);
                    }
                    x += 1;
                }
            }
        }
    }
}

impl AudioVisualizer {
    /// Test helper: pretend a 16 ms frame has elapsed so easing moves a
    /// realistic amount per tick even in tight loops.
    #[cfg(test)]
    fn backdate_tick(&mut self) {
        self.last_tick = std::time::Instant::now() - std::time::Duration::from_millis(16);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preset_cycles_wrap() {
        let mut p = VisualizerPreset::default();
        assert_eq!(p, VisualizerPreset::Braille);
        let names: Vec<&str> = VisualizerPreset::all().iter().map(|x| x.name()).collect();
        assert_eq!(
            names,
            vec![
                "Braille",
                "Blocks",
                "Mirror",
                "Gradient",
                "Spectrum",
                "BarsDot",
                "ClassicPeak",
                "Columns",
                "Wave",
                "Stereo",
                "Retro",
                "Flame",
            ]
        );
        for _ in 0..VisualizerPreset::all().len() {
            p = p.next();
        }
        assert_eq!(p, VisualizerPreset::Braille);
    }

    #[test]
    fn preset_roundtrip() {
        let p = serde_json::from_str::<VisualizerPreset>("\"spectrum\"").unwrap();
        assert_eq!(p, VisualizerPreset::Spectrum);
        assert_eq!(
            serde_json::to_string(&VisualizerPreset::BarsDot).unwrap(),
            "\"barsdot\""
        );
        assert_eq!(
            serde_json::to_string(&VisualizerPreset::Blocks).unwrap(),
            "\"blocks\""
        );
    }

    #[test]
    fn band_edges_map_onto_bins() {
        // 10 bands → 11 edges over a 64-bin spectrum.
        let edges = band_edges_hz_to_bins(64);
        assert_eq!(edges.len(), BAND_COUNT + 1);
        assert!(edges.windows(2).all(|w| w[0] <= w[1]));
        assert_eq!(edges[0], 0.0); // 20 Hz clamps below the 30 Hz floor
        assert!(edges[BAND_COUNT] <= 63.0);
    }

    #[test]
    fn resample_bands_tiles_any_width() {
        let bands = [0.2f32, 0.5, 0.8, 0.1, 0.6, 0.3, 0.9, 0.4, 0.7, 0.2];
        for width in [10usize, 40, 77, 120] {
            let cols = resample_bands_linear(&bands, width);
            assert_eq!(cols.len(), width, "resample must tile the full width");
            assert!(cols.iter().all(|c| (0.0..=1.0).contains(c)));
        }
        // Identity when the target width equals the band count.
        let cols = resample_bands_linear(&bands, bands.len());
        assert_eq!(cols, bands.to_vec());
    }

    #[test]
    fn decaying_bands_reach_rest() {
        let mut v = AudioVisualizer::new();
        v.enabled = true;
        v.bands = [0.9; BAND_COUNT];
        v.resting = false;
        v.backdate_tick();
        for _ in 0..240 {
            v.tick(false, 40, 6, &[], &[], false);
            v.backdate_tick();
        }
        assert!(v.resting, "decay-to-rest must trigger while stopped");
        assert!(v.bands.iter().all(|&b| b < PAUSED_DECAY_EPSILON));
    }

    #[test]
    fn bars_follow_audio_levels() {
        let mut v = AudioVisualizer::new();
        v.enabled = true;
        let levels: Vec<f32> = vec![0.8; 64];
        v.backdate_tick();
        for _ in 0..30 {
            v.tick(true, 40, 6, &levels, &[], false);
            v.backdate_tick();
        }
        assert!(v.bars.iter().any(|&b| b > 0.5));
    }
}
