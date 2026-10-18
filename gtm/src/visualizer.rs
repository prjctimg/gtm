// Copyright (c) 2026
// Author: prjctimg <prjctimg@outlook.com>
// Audio visualizer with multiple render presets
//
// The spectrum model ports cliamp's log-frequency band layout (20 Hz–20 kHz,
// 10 bands) with per-column band interpolation, frame-capped easing, and decay
// to rest while paused. The render presets are Mirror / Gradient / Wave /
// Stereo / Flame, every one of which has a shader; the cell renderer behind
// them is the fallback for a machine with no GPU to draw with, not a second
// set of looks.
//
// This is free software released under the GPL-3.0 license.

use std::time::Instant;

use image::{Rgba, RgbaImage};
use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Widget;

use crate::theme::{AppTheme, blend_colors};

// ─── Presets ────────────────────────────────────────────────────────────────

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum VisualizerPreset {
    #[default]
    Mirror,
    Gradient,
    Wave,
    Stereo,
    Flame,
}

/// Serialise as the lowercase name; read anything back.
///
/// Reading is deliberately tolerant. `config.toml` names a preset, and a build
/// that removed one would otherwise make the *whole file* fail to parse — taking
/// the queue, the volume, the EQ and the resume position down with it. That is
/// not hypothetical: `SavedState::load` carries a comment about exactly this for
/// `scrobble`.
///
/// A removed or misspelt name therefore falls back to the default rather than
/// erroring. A visualizer preset is a display preference — losing it costs
/// nothing and is visible at once, while a config that will not load costs
/// everything and is silent.
impl serde::Serialize for VisualizerPreset {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(self.name().to_lowercase().as_str())
    }
}

impl<'de> serde::Deserialize<'de> for VisualizerPreset {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let raw = String::deserialize(d)?;
        Ok(VisualizerPreset::from_name(&raw))
    }
}

impl VisualizerPreset {
    pub fn all() -> &'static [VisualizerPreset] {
        &[
            VisualizerPreset::Mirror,
            VisualizerPreset::Gradient,
            VisualizerPreset::Wave,
            VisualizerPreset::Stereo,
            VisualizerPreset::Flame,
        ]
    }

    /// The preset a stored name resolves to, defaulting when it names nothing.
    ///
    /// Separate from [`Self::all`] so the tolerant read a config file goes
    /// through is the same resolution the picker uses, and there is only ever
    /// one list of what a name can mean.
    pub fn from_name(name: &str) -> Self {
        match name.trim().to_lowercase().as_str() {
            "mirror" => Self::Mirror,
            "gradient" => Self::Gradient,
            "wave" => Self::Wave,
            "stereo" => Self::Stereo,
            "flame" => Self::Flame,
            _ => Self::default(),
        }
    }

    pub fn next(&self) -> VisualizerPreset {
        let all = Self::all();
        let idx = all.iter().position(|p| p == self).unwrap_or(0);
        all[(idx + 1) % all.len()]
    }

    /// The GPU preset that draws this one.
    ///
    /// Infallible: the presets that had no shader were the ones the half-block
    /// transport could not carry anyway — a braille dot grid is 2 wide by 4 tall
    /// and comes back as a blur — so removing them left nothing `Option` had
    /// anything to say about. The cell renderer still stands behind this as the
    /// fallback when there is no device to draw with, which is the only way a
    /// frame can still fail to reach a GPU.
    #[cfg(feature = "shader-viz")]
    pub fn gpu(self) -> crate::shader::Preset {
        use crate::shader::Preset;
        match self {
            VisualizerPreset::Flame => Preset::Flame,
            VisualizerPreset::Gradient => Preset::Gradient,
            VisualizerPreset::Mirror => Preset::Mirror,
            VisualizerPreset::Wave => Preset::Wave,
            VisualizerPreset::Stereo => Preset::Stereo,
        }
    }

    pub fn name(&self) -> &'static str {
        match self {
            VisualizerPreset::Mirror => "Mirror",
            VisualizerPreset::Gradient => "Gradient",
            VisualizerPreset::Wave => "Wave",
            VisualizerPreset::Stereo => "Stereo",
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

/// Spectrum content below this is treated as fully decayed to rest, which is
/// the empty state the pane draws nothing in.
const PAUSED_DECAY_EPSILON: f32 = 0.01;

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

/// Share of the accent mixed into each neutral step of a meter ramp.
///
/// `derive_theme` re-tints the accents and leaves `fg`/`fg_bright`/`fg_dim`
/// alone, which is right for text and wrong for a meter: the quiet end of every
/// ramp stayed the same grey whatever was playing, so a reactive theme only
/// reached the top of the bars. Low enough that the steps stay distinguishable
/// from one another at one cell per bar.
const STEP_TINT: f64 = 0.35;

/// Fallback heat palette for Flame.
fn heat_color(heat: f32, theme: &AppTheme) -> Color {
    if heat > 0.78 {
        // Nudging the accent toward cream keeps the "hot" reading while giving
        // the peak the artwork's hue, the one place at the top of the ramp a
        // reactive theme reaches.
        blend_colors(theme.accent, Color::Rgb(255, 250, 200), 0.7)
    } else if heat > 0.58 {
        theme.tertiary_accent
    } else if heat > 0.38 {
        theme.secondary_accent
    } else if heat > 0.2 {
        theme.accent
    } else {
        tinted(theme, theme.fg_dim)
    }
}

/// A neutral theme step pulled toward the current artwork, preserving its
/// luminance so ramp ordering survives. Non-RGB themes have nothing to blend
/// and `blend_colors` hands the step back untouched.
fn tinted(theme: &AppTheme, step: Color) -> Color {
    blend_colors(step, theme.accent, STEP_TINT)
}

/// Three-step ramp by amplitude. Free rather than a method so the visualizer
/// preset picker can preview bars with the same colours the renderer uses.
pub fn amplitude_color(val: f32, theme: &AppTheme) -> Color {
    if val > 0.7 {
        theme.accent
    } else if val > 0.4 {
        tinted(theme, theme.fg_bright)
    } else {
        tinted(theme, theme.fg_dim)
    }
}
// ─── Band helpers ───────────────────────────────────────────────────────────

/// Map an Hz edge onto the log-spaced index space of the analyzer's bins.
fn bin_pos(freq: f64, bins: usize) -> f64 {
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
        out.push(band_at(bands, (bands.len() - 1) as f64 / 2.0) as f32);
        return out;
    }
    let last = (bands.len() - 1) as f64;
    for col in 0..total_cols {
        let pos = col as f64 / (total_cols - 1) as f64 * last;
        out.push(band_at(bands, pos) as f32);
    }
    out
}

// ─── Visualizer ─────────────────────────────────────────────────────────────

pub struct AudioVisualizer {
    pub preset: VisualizerPreset,
    /// The headless GPU device, brought up on the first Flame frame and `None`
    /// on any machine that cannot provide one. Lazily created so a user who
    /// never reaches Flame never pays for the device.
    #[cfg(feature = "shader-viz")]
    gpu: Option<crate::shader::Shader>,
    // Column bar model (one level per terminal column)
    bars: Vec<f32>,
    target_cols: Vec<f32>,
    peaks: Vec<f32>,
    // Log-band model (BAND_COUNT entries, smoothed)
    bands: [f32; BAND_COUNT],
    bands_prev: [f32; BAND_COUNT],
    // Waveform ring (mirrors DaemonState)
    wave_samples: Vec<f32>,
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
    frame: u64,
    resting: bool,
    playing: bool,
}

impl AudioVisualizer {
    pub fn new() -> Self {
        Self {
            preset: VisualizerPreset::default(),
            #[cfg(feature = "shader-viz")]
            gpu: None,
            bars: vec![0.0; 32],
            target_cols: vec![0.0; 32],
            peaks: vec![0.0; 32],
            bands: [0.0; BAND_COUNT],
            bands_prev: [0.0; BAND_COUNT],
            wave_samples: Vec::new(),
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
            frame: 0,
            resting: true,
            playing: false,
        }
    }

    /// Advance the band/bar model one frame. `width`/`height` come from the
    /// visualizer panel; `audio_levels` is the 64-bin spectrum and
    /// `wave_samples` a decimated interleaved L/R ring (empty when silent).
    pub fn tick(
        &mut self,
        is_playing: bool,
        width: u16,
        height: u16,
        audio_levels: &[f32],
        wave_samples: &[f32],
    ) {
        if width == 0 {
            return;
        }
        let now = Instant::now();
        let dt = now
            .duration_since(self.last_tick)
            .as_secs_f64()
            .clamp(0.0, MAX_SMOOTH_DT);
        self.last_tick = now;
        self.frame = self.frame.wrapping_add(1);
        let dt = dt as f32;
        self.playing = is_playing;

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
            let edges = edges_to_bins(audio_levels.len());
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
            // Decay to rest: with no playback the bands fall to zero and the
            // model settles into the state that renders nothing.
            let mut peaked = 0.0f32;
            for slot in self.bands.iter_mut() {
                *slot *= 0.78 + dt * 0.2;
                peaked = peaked.max(*slot);
            }
            if peaked < PAUSED_DECAY_EPSILON {
                self.resting = true;
            }
        }

        // 2) Band targets. At rest they are all zero: nothing is playing, so
        // there is nothing to show.
        let band_levels = if self.resting {
            [0.0f32; BAND_COUNT]
        } else {
            self.bands
        };

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

        // 6) Waveform ring + stereo meter levels. Samples are only kept while
        // playing: a stopped visualizer must not draw the last trace.
        self.wave_samples.clear();
        if is_playing {
            self.wave_samples.extend_from_slice(wave_samples);
        }
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
        // Two samples per lane, by construction rather than by pairing up a
        // flat buffer: the array is filled as L then R, so the split is exact
        // and `.0` is the remainder, which is always empty.
        for pair in self.wave_samples.as_chunks::<2>().0 {
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
        // Seed the bottom (source) row from the spectrum + ember base. With no
        // playback the source is cold and the field burns out.
        let band_last = (BAND_COUNT - 1) as f64;
        for x in 0..cols {
            let t = x as f64 / last as f64 * band_last;
            let band = if self.resting {
                0.0
            } else {
                band_at(&self.bands, t)
            };
            let sparkle = (self.next_rng() % 100) as f64 / 100.0 * 0.18;
            let base = if self.playing {
                0.30 + 0.70 * band + sparkle
            } else {
                0.0
            };
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

    pub fn render(&self, area: Rect, theme: &AppTheme) -> Option<Lines<'static>> {
        if area.width < 4 || area.height < 3 {
            return None;
        }
        // Nothing is playing and the model has settled: draw nothing rather
        // than a synthetic spectrum that reads as live audio.
        if !self.playing && self.resting {
            return None;
        }

        let w = area.width as usize;
        let h = area.height as usize;
        let num_bars = w.min(self.bars.len());
        let num_bars = num_bars.max(1);

        Some(match self.preset {
            VisualizerPreset::Mirror => self.render_mirror(num_bars, h, theme),
            VisualizerPreset::Gradient => self.render_gradient(num_bars, h, theme),
            VisualizerPreset::Wave => self.render_wave(num_bars, h, theme),
            VisualizerPreset::Stereo => self.render_stereo(num_bars, h, theme),
            VisualizerPreset::Flame => self.render_flame(num_bars, h, theme),
        })
    }
    /// Braille oscilloscope over the decimated waveform ring (cliamp
    /// renderWave). No samples means no wave: the ring is only filled while
    /// playing, so a stopped pane stays empty instead of drawing a sweep.
    fn render_wave(&self, num_bars: usize, height: usize, theme: &AppTheme) -> Lines<'static> {
        let dot_rows = height * 4;
        let dot_cols = num_bars * 2;
        let n = self.wave_samples.len();
        if dot_cols == 0 || n < 2 {
            return lines_empty(height);
        }
        // Per-dot-column y position.
        let mut ypos = vec![0usize; dot_cols];
        for (x, slot) in ypos.iter_mut().enumerate() {
            let v = {
                let idx = ((x as f64 * n as f64 / dot_cols as f64) as usize / 2 * 2).min(n - 2);
                self.wave_samples[idx] as f64
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
                    (BLOCKS[idx], amplitude_color(level, theme))
                } else if x == full_rows && frac > 0.001 && full_rows < meter_w {
                    (
                        BLOCKS[((frac * 8.0) as usize).min(7)],
                        amplitude_color(level, theme),
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
    /// Center-symmetric bars blooming outward from the middle axis.
    fn render_mirror(&self, num_bars: usize, height: usize, theme: &AppTheme) -> Lines<'static> {
        let half = height as f32 / 2.0;
        let mut lines: Vec<Line<'static>> = Vec::new();
        for row_from_top in 0..height {
            let mut spans: Vec<Span<'static>> = Vec::new();
            let center_dist = ((row_from_top as f32 + 0.5) - half).abs();
            for i in 0..num_bars {
                let val = *self.bars.get(i).unwrap_or(&0.0);
                let extent = val * half;
                let (glyph, color) = if center_dist <= extent {
                    ("█", amplitude_color(val, theme))
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
    ) -> Lines<'static> {
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
    fn render_gradient(&self, num_bars: usize, height: usize, theme: &AppTheme) -> Lines<'static> {
        // The same four-step ramp every other amplitude-driven preset uses: a
        // second copy of it here is a second answer to "how bright is a value
        // of 0.6", and the two would drift.
        self.render_braille_grid(num_bars, height, theme, amplitude_color)
    }
}

fn lines_empty(height: usize) -> Lines<'static> {
    Lines((0..height).map(|_| Line::from(" ")).collect())
}

/// Linearly sample band values at fractional position `pos`. The one sampler
/// the band path has: callers cast down to `f32` where the bars are `f32`.
fn band_at(bands: &[f32], pos: f64) -> f64 {
    let n = bands.len();
    if n == 0 {
        return 0.0;
    }
    if n == 1 {
        return bands[0] as f64;
    }
    let last = (n - 1) as f64;
    let pos = pos.clamp(0.0, last);
    let idx = pos.floor() as usize;
    if idx >= n - 1 {
        return bands[n - 1] as f64;
    }
    let frac = pos - idx as f64;
    bands[idx] as f64 * (1.0 - frac) + bands[idx + 1] as f64 * frac
}

/// Band edges (Hz) → fractional bin positions for the current bin count.
fn edges_to_bins(bins: usize) -> Vec<f64> {
    LEGACY_EDGES_HZ
        .iter()
        .map(|&hz| bin_pos(hz, bins))
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

// ─── Visualizer extension API ───────────────────────────────────────────────

/// One frame of audio, as every visualizer receives it. The slices borrow the
/// shared audio state, so the per-frame path allocates nothing here.
pub struct AudioFrame<'a> {
    pub playing: bool,
    /// Seconds since the previous frame. The built-in model measures its own
    /// time in `tick` and ignores this; a shader or a foreign visualizer
    /// integrates it.
    pub dt: f64,
    /// Spectrum input: the analyzer's log-spaced bins.
    pub bands: &'a [f32],
    /// Decimated interleaved L/R ring. Empty when silent.
    pub wave: &'a [f32],
    pub stereo: bool,
    /// Pane size in cells.
    pub size: (u16, u16),
    pub theme: &'a AppTheme,
}

/// What a visualizer draws. `Cells` is the terminal-glyph renderer and the
/// universal fallback; `Pixels` is a real image sampled into half-blocks by
/// [`pixels_to_lines`]; `Shader` names a WGSL entry point compiled once.
pub enum Frame {
    Cells(Lines<'static>),
    Pixels(RgbaImage),
    Shader(&'static str),
}

/// A visualizer: consume audio, produce a frame. The twelve presets were one
/// twelve-way `match` inside `render`; a value behind this trait is what makes
/// "write your own" stop meaning "fork the renderer".
pub trait Visualizer: Send + 'static {
    fn update(&mut self, frame: &AudioFrame);
    fn draw(&mut self, frame: &AudioFrame) -> Frame;
}

/// How a registered visualizer is built. One name per factory, so a caller
/// asks for a name and gets an instance rather than a type it must know.
type VisualizerFactory = fn() -> Box<dyn Visualizer>;

/// Factories for the visualizers the client ships. The built-in `Audio` one is
/// registered here and gates on the same `ExtensionId::Visualizer` kill switch
/// that already decides whether spectrum frames move at all.
#[derive(Default)]
pub struct VisualizerRegistry {
    entries: Vec<(&'static str, VisualizerFactory)>,
}

impl VisualizerRegistry {
    /// A registry with the built-in visualizers registered.
    pub fn with_builtins() -> Self {
        let mut registry = Self::default();
        registry.register("Audio", || Box::new(AudioVisualizer::new()));
        registry
    }

    pub fn register(&mut self, name: &'static str, factory: VisualizerFactory) {
        if !self.entries.iter().any(|(n, _)| *n == name) {
            self.entries.push((name, factory));
        }
    }

    pub fn names(&self) -> Vec<&'static str> {
        self.entries.iter().map(|(n, _)| *n).collect()
    }

    pub fn create(&self, name: &str) -> Option<Box<dyn Visualizer>> {
        self.entries
            .iter()
            .find(|(n, _)| *n == name)
            .map(|(_, factory)| factory())
    }
}

/// Route A: sample an RGBA frame into `▀` cells, the foreground the upper pixel
/// and the background the lower one. It works on every terminal the cell
/// renderer works on, which is why the eager shader path falls back to it.
pub fn pixels_to_lines(img: &RgbaImage, out_w: u16, out_h: u16) -> Lines<'static> {
    let (ow, oh) = (out_w.max(1) as u32, out_h.max(1) as u32);
    let (iw, ih) = (img.width().max(1), img.height().max(1));
    let mut lines = Vec::with_capacity(oh as usize);
    for cy in 0..oh {
        // Two source rows per output row: upper pixel top, lower pixel bottom.
        let row_at = |band: u32| {
            (((cy as u64 * 2 + band as u64) * ih as u64) / (oh as u64 * 2)).min(ih as u64 - 1)
                as u32
        };
        let (top_y, bot_y) = (row_at(0), row_at(1));
        let mut spans = Vec::with_capacity(ow as usize);
        for cx in 0..ow {
            let sx = ((cx as u64 * iw as u64) / ow as u64).min(iw as u64 - 1) as u32;
            let top = img.get_pixel(sx, top_y).0;
            let bot = img.get_pixel(sx, bot_y).0;
            let fg = if top[3] == 0 {
                Color::Reset
            } else {
                Color::Rgb(top[0], top[1], top[2])
            };
            let bg = if bot[3] == 0 {
                Color::Reset
            } else {
                Color::Rgb(bot[0], bot[1], bot[2])
            };
            spans.push(Span::styled("▀", Style::default().fg(fg).bg(bg)));
        }
        lines.push(Line::from(spans));
    }
    Lines(lines)
}

/// An RGB tuple for a ratatui colour. Indexed and reset colours have no RGB
/// here, so they map to black; every visualizer path hands back truecolor.
fn rgb_of(c: Color) -> [u8; 3] {
    match c {
        Color::Rgb(r, g, b) => [r, g, b],
        Color::Black => [0, 0, 0],
        Color::White => [255, 255, 255],
        _ => [0, 0, 0],
    }
}

impl Visualizer for AudioVisualizer {
    fn update(&mut self, frame: &AudioFrame) {
        self.tick(
            frame.playing,
            frame.size.0,
            frame.size.1,
            frame.bands,
            frame.wave,
        );
    }

    fn draw(&mut self, frame: &AudioFrame) -> Frame {
        let (w, h) = frame.size;
        // Nothing to show: an empty pane rather than a synthetic spectrum.
        if w < 4 || h < 3 || (!self.playing && self.resting) {
            return Frame::Cells(Lines(Vec::new()));
        }
        // The GPU path first for every preset, and the cell renderer when there
        // is no device to draw with. `Shader::draw` answers `None` on the very
        // first call by design — a device that exists still has a frame in
        // flight — so `None` means "cells this frame", not "give up on the GPU".
        #[cfg(feature = "shader-viz")]
        {
            let preset = self.preset.gpu();
            if self.gpu.is_none()
                && let Some(gpu) = crate::shader::Shader::new()
            {
                self.gpu = Some(gpu);
            }
            // Built before the borrow: the job reads the band and waveform
            // models, which `gpu.draw` needs `&mut` for.
            if let Some(job) = self.shader_job(preset, frame)
                && let Some(gpu) = self.gpu.as_mut()
                && let Some(img) = gpu.draw(job)
            {
                return Frame::Pixels(img);
            }
        }
        // Flame still has a CPU heat field behind the shader, so a machine with
        // no device keeps the effect rather than losing the preset.
        if self.preset == VisualizerPreset::Flame && self.heat_rows > 0 && self.heat_cols > 0 {
            return Frame::Pixels(self.flame_image(frame.theme));
        }
        Frame::Cells(
            self.render(Rect::new(0, 0, w, h), frame.theme)
                .unwrap_or_else(|| Lines(Vec::new())),
        )
    }
}

impl AudioVisualizer {
    /// The GPU preset that draws `self.preset`, or `None` when it has none.
    ///
    /// Only presets whose look survives the trip through a terminal cell go
    /// here. The frame is read back and printed with `▀`, which resolves one
    /// horizontal and two vertical samples per cell — so a bar grid, a trace and
    /// a meter are exact, while a braille dot grid (2 wide by 4 tall) and a
    /// four-wide dot bar cannot be drawn at all through it and would come back
    /// as a blur of half-blocks. Those stay on the cell renderer, where the
    /// glyph is the whole point.
    /// The work item for one GPU frame.
    ///
    /// Built from the same smoothed bands, the same waveform and the same theme
    /// ramp the cell renderer uses, so the two paths cannot drift apart: the GPU
    /// changes how the levels become pixels, not what the levels are.
    #[cfg(feature = "shader-viz")]
    fn shader_job(
        &self,
        preset: crate::shader::Preset,
        frame: &AudioFrame,
    ) -> Option<crate::shader::Draw> {
        let cols = self.heat_cols.clamp(1, 64);
        let bands = resample_bands_linear(&self.bands, cols);
        // The waveform is decimated to the pane's own width, which is what the
        // trace reads; resampling a longer ring would only add jitter.
        let wave: Vec<f32> = frame
            .wave
            .iter()
            .copied()
            .filter(|v| v.is_finite())
            .take(64)
            .collect();
        let t = frame.theme;
        Some(crate::shader::Draw {
            preset,
            playing: self.playing,
            dt: frame.dt as f32,
            bands: std::array::from_fn(|i| bands.get(i).copied().unwrap_or(0.0)),
            band_count: bands.len().min(64) as u32,
            wave: std::array::from_fn(|i| wave.get(i).copied().unwrap_or(0.0)),
            wave_count: wave.len().min(64) as u32,
            stereo: [
                self.stereo_level[0],
                self.stereo_level[1],
                self.stereo_peak[0],
                self.stereo_peak[1],
            ],
            cells: frame.size,
            cool: rgb_of(t.fg_dim),
            warm: rgb_of(t.accent),
            peak: rgb_of(t.secondary_accent),
        })
    }

    /// Rasterise the flame heat field to pixels. `draw` reaches this so Flame
    /// travels the `Frame::Pixels` route with no GPU at all: the same heat
    /// model, a different last step.
    fn flame_image(&self, theme: &AppTheme) -> RgbaImage {
        let rows = self.heat_rows.max(1) as u32;
        let cols = self.heat_cols.max(1) as u32;
        let mut img = RgbaImage::new(cols, rows);
        for y in 0..rows {
            for x in 0..cols {
                let idx = (y * self.heat_cols as u32 + x) as usize;
                let heat = self.heat.get(idx).copied().unwrap_or(0.0) as f32;
                let [r, g, b] = rgb_of(heat_color(heat, theme));
                img.put_pixel(x, y, Rgba([r, g, b, 255]));
            }
        }
        img
    }

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

    /// The surviving set, in cycle order.
    ///
    /// Spelled out rather than derived from `all()` so adding a preset has to
    /// be a decision here, and so a preset that vanished from the enum without
    /// anyone noticing fails here first.
    #[test]
    fn preset_cycles_wrap() {
        let names: Vec<&str> = VisualizerPreset::all().iter().map(|x| x.name()).collect();
        assert_eq!(names, vec!["Mirror", "Gradient", "Wave", "Stereo", "Flame"]);

        let mut p = VisualizerPreset::default();
        for _ in 0..VisualizerPreset::all().len() {
            p = p.next();
        }
        assert_eq!(p, VisualizerPreset::default());
    }

    /// Every preset has a shader, which is why `gpu_preset` is infallible.
    ///
    /// The property the removal bought: if a preset without a shader is ever
    /// added back, the cell renderer would be the only thing drawing it, and
    /// `gpu_preset` would have to become fallible again.
    #[cfg(feature = "shader-viz")]
    #[test]
    fn every_preset_maps_to_a_shader() {
        for p in VisualizerPreset::all() {
            let mapped = p.gpu();
            assert!(
                crate::shader::Preset::ALL.contains(&mapped),
                "{p:?} has no shader"
            );
        }
    }

    #[test]
    fn preset_roundtrip() {
        let p = serde_json::from_str::<VisualizerPreset>("\"wave\"").unwrap();
        assert_eq!(p, VisualizerPreset::Wave);
        assert_eq!(
            serde_json::to_string(&VisualizerPreset::Gradient).unwrap(),
            "\"gradient\""
        );
    }

    /// A name this build does not have resolves to the default, not an error.
    ///
    /// The removed presets are the point: `config.toml` on a user's disk still
    /// says `visualizer_preset = "braille"`, and if that failed to parse, the
    /// whole file would be dropped — queue, volume, EQ and resume position
    /// included. Covered for a name that never existed as well, because a typo
    /// is the same failure.
    #[test]
    fn an_unknown_preset_name_falls_back_instead_of_failing() {
        for name in [
            "braille",
            "blocks",
            "spectrum",
            "barsdot",
            "classicpeak",
            "columns",
            "retro",
            "nonsense",
            "",
            "  Flame  ",
        ] {
            let parsed = serde_json::from_str::<VisualizerPreset>(&format!("\"{name}\""));
            let got = parsed.unwrap_or_else(|e| panic!("{name:?} failed to parse: {e}"));
            if name.trim().eq_ignore_ascii_case("flame") {
                assert_eq!(got, VisualizerPreset::Flame, "{name:?}");
            } else {
                assert_eq!(got, VisualizerPreset::default(), "{name:?}");
            }
        }
    }

    /// Reading is case- and space-tolerant, because a hand-edited config has
    /// both and there is no reason to drop the file over either.
    #[test]
    fn names_resolve_leniently() {
        assert_eq!(VisualizerPreset::from_name("WAVE"), VisualizerPreset::Wave);
        assert_eq!(
            VisualizerPreset::from_name("  stereo "),
            VisualizerPreset::Stereo
        );
        assert_eq!(
            VisualizerPreset::from_name("MIRROR"),
            VisualizerPreset::Mirror
        );
    }

    #[test]
    fn band_edges_map_onto_bins() {
        // 10 bands → 11 edges over a 64-bin spectrum.
        let edges = edges_to_bins(64);
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
        v.bands = [0.9; BAND_COUNT];
        v.resting = false;
        v.backdate_tick();
        for _ in 0..240 {
            v.tick(false, 40, 6, &[], &[]);
            v.backdate_tick();
        }
        assert!(v.resting, "decay-to-rest must trigger while stopped");
        assert!(v.bands.iter().all(|&b| b < PAUSED_DECAY_EPSILON));
        assert!(
            v.render(Rect::new(0, 0, 40, 6), &crate::theme::chadrula())
                .is_none(),
            "a stopped visualizer must draw nothing"
        );
    }

    #[test]
    fn bars_follow_audio_levels() {
        let mut v = AudioVisualizer::new();
        let levels: Vec<f32> = vec![0.8; 64];
        v.backdate_tick();
        for _ in 0..30 {
            v.tick(true, 40, 6, &levels, &[]);
            v.backdate_tick();
        }
        assert!(v.bars.iter().any(|&b| b > 0.5));
        assert!(
            v.render(Rect::new(0, 0, 40, 6), &crate::theme::chadrula())
                .is_some()
        );
    }

    /// The whole point of the reactive theme here: changing only the artwork
    /// colour must change what the bars are drawn in, at every step of the ramp
    /// and not just the accent-coloured top.
    #[test]
    fn the_ramp_follows_the_artwork() {
        let base = crate::theme::chadrula();
        let other = AppTheme {
            accent: Color::Rgb(10, 200, 90),
            ..base
        };
        for val in [0.1, 0.5, 0.9] {
            assert_ne!(
                amplitude_color(val, &base),
                amplitude_color(val, &other),
                "amplitude {val} ignored the accent"
            );
        }
    }

    /// The heat peak follows the accent, so a reactive theme reaches it.
    #[test]
    fn the_heat_peak_follows_the_accent() {
        let base = crate::theme::chadrula();
        let other = AppTheme {
            accent: Color::Rgb(10, 200, 90),
            ..base
        };
        assert_ne!(heat_color(0.9, &base), heat_color(0.9, &other));
        assert_ne!(heat_color(0.9, &base), Color::Rgb(255, 250, 200));
    }
}
