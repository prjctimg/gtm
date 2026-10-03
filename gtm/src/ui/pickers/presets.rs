// Copyright (c) 2026
// Author: prjctimg <prjctimg@outlook.com>
// EQ, crossfade, audio output, and the visualizer sample the Look picker draws
//
// This is free software released under the GPL-3.0 license.

use crate::ui::*;

impl Pickers {
    pub(crate) fn render_equalizer(f: &mut ratatui::Frame, area: Rect, app: &mut App) {
        let presets = [
            ("Flat", "Neutral, uncoloured response", EqPreset::Flat),
            ("Normal", "Balanced all-rounder", EqPreset::Normal),
            ("Pop", "Vocal-forward with a lively top end", EqPreset::Pop),
            (
                "Rock",
                "Aggressive mids for punch and drive",
                EqPreset::Rock,
            ),
            ("Jazz", "Smooth mids and sparkly highs", EqPreset::Jazz),
            ("Classical", "Wide, airy and natural", EqPreset::Classical),
            ("Bass", "Deep low-end emphasis", EqPreset::Bass),
            ("Vocal", "Brings voices to the front", EqPreset::Vocal),
            (
                "Electronic",
                "Tight, modern club sound",
                EqPreset::Electronic,
            ),
            ("Hip-Hop", "Heavy bass and crisp highs", EqPreset::HipHop),
            ("Latin", "Warm and rhythmic", EqPreset::Latin),
            ("Acoustic", "Clean and intimate", EqPreset::Acoustic),
            ("Podcast", "Speech clarity over music", EqPreset::Podcast),
            ("Dance", "Pumping lows for the floor", EqPreset::Dance),
            (
                "Headphones",
                "Close-up stereo imaging",
                EqPreset::Headphones,
            ),
            ("Speaker", "Room-filling broad response", EqPreset::Speaker),
        ];

        let sel = app
            .pickers
            .top()
            .map_or(0, |o| o.selected.min(presets.len() - 1));

        let block = Self::picker_panel(app, " Equalizer ", None);
        let inner = block.inner(area);
        f.render_widget(block, area);

        let preview_height: u16 = 5;
        let list_h = inner.height.saturating_sub(preview_height);
        let list_area = Rect {
            x: inner.x,
            y: inner.y,
            width: inner.width,
            height: list_h,
        };
        let preview_area = Rect {
            x: inner.x,
            y: inner.y + list_h,
            width: inner.width,
            height: inner.height.saturating_sub(list_h),
        };

        let visible = list_h as usize;
        let total = presets.len();
        let (scroll_start, scroll_end) = if let Some(top) = app.pickers.top_mut() {
            let (s, e) = step_viewport(top.viewport_offset, sel, visible, total);
            top.viewport_offset = s;
            (s, e)
        } else {
            (0, total)
        };

        let mut list_items: Vec<ListItem> = Vec::new();
        for (i, (name, _desc, _eq)) in presets
            .iter()
            .enumerate()
            .skip(scroll_start)
            .take(scroll_end - scroll_start)
        {
            let is_sel = i == sel;
            let prefix = "   ";
            let style = if is_sel {
                Style::default()
                    .fg(app.theme.selection_fg_readable())
                    .bg(app.theme.selection_bg)
            } else if *name == app.state.audio.eq_preset.label() {
                Style::default().fg(app.theme.success)
            } else {
                Style::default()
            };
            let spans = vec![Span::styled(format!("{prefix}{}", name), Style::default())];
            list_items.push(ListItem::new(Line::from(spans)).style(style));
        }

        let list = List::new(list_items);
        f.render_widget(list, list_area);

        if preview_area.height >= 4 {
            let selected_preset = presets.get(sel);
            let selected_name = selected_preset.map(|p| p.0).unwrap_or("");
            let selected_desc = selected_preset.map(|p| p.1).unwrap_or("");
            let rule = Line::from(vec![
                Span::styled(
                    "\u{2500}".to_string(),
                    Style::default().fg(app.theme.muted_border),
                ),
                Span::styled(
                    format!(" {selected_name} "),
                    Style::default()
                        .fg(app.theme.accent)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    "\u{2500}".repeat(
                        preview_area
                            .width
                            .saturating_sub(selected_name.len() as u16 + 4)
                            .max(1) as usize,
                    ),
                    Style::default().fg(app.theme.muted_border),
                ),
            ]);
            f.render_widget(
                Paragraph::new(rule),
                Rect {
                    x: preview_area.x,
                    y: preview_area.y,
                    width: preview_area.width,
                    height: 1,
                },
            );

            let selected_eq = selected_preset.map(|p| p.2);

            let mut preview_spans = Vec::new();
            if let Some(eq) = selected_eq {
                preview_spans.extend(Self::eq_preset_preview(eq, app));
            }
            f.render_widget(
                Paragraph::new(Line::from(preview_spans)),
                Rect {
                    x: preview_area.x,
                    y: preview_area.y + 1,
                    width: preview_area.width,
                    height: 1,
                },
            );

            // Render description below visualization
            if !selected_desc.is_empty() {
                f.render_widget(
                    Paragraph::new(Line::from(vec![Span::styled(
                        format!("  {selected_desc}"),
                        Style::default().fg(app.theme.fg_dim),
                    )])),
                    Rect {
                        x: preview_area.x,
                        y: preview_area.y + 2,
                        width: preview_area.width,
                        height: 1,
                    },
                );
            }
        }
    }

    pub(crate) fn eq_preset_preview(eq: EqPreset, app: &App) -> Vec<Span<'static>> {
        const BLOCKS: [char; 8] = ['▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];
        const BRAILLE: [char; 8] = ['⠁', '⠃', '⠇', '⡇', '⣇', '⣧', '⣷', '⣿'];
        let chars: &[char; 8] = match app.visualizer.preset {
            VisualizerPreset::Braille | VisualizerPreset::Gradient => &BRAILLE,
            _ => &BLOCKS,
        };
        let mut spans = vec![Span::raw("  ")];
        for g in eq.to_gains() {
            let norm = (((g.clamp(-4.0, 4.0) + 4.0) / 8.0) * 7.0).round() as usize;
            let color = if g > 0.75 {
                app.theme.success
            } else if g < -0.75 {
                app.theme.warning
            } else {
                app.theme.fg_dim
            };
            spans.push(Span::styled(
                chars[norm.min(7)].to_string(),
                Style::default().fg(color),
            ));
        }
        spans
    }

    pub(crate) fn visualizer_preview_lines(
        preset: VisualizerPreset,
        bars: &[f32],
        width: u16,
        app: &App,
    ) -> Vec<Line<'static>> {
        let w = width as usize;
        let mut lines = Vec::new();

        match preset {
            VisualizerPreset::Braille => {
                for row in 0..2 {
                    let mut spans = Vec::with_capacity(w);
                    for &b in bars.iter().take(w) {
                        let level = (b * 4.0) as u32;
                        let ch = match row {
                            0 => {
                                if level >= 4 {
                                    '⣿'
                                } else if level >= 3 {
                                    '⣷'
                                } else if level >= 2 {
                                    '⣧'
                                } else if level >= 1 {
                                    '⣇'
                                } else {
                                    '⠀'
                                }
                            }
                            _ => {
                                if level >= 2 {
                                    '⣿'
                                } else if level >= 1 {
                                    '⡇'
                                } else {
                                    '⠀'
                                }
                            }
                        };
                        spans.push(Span::styled(
                            ch.to_string(),
                            Style::default().fg(app.theme.accent),
                        ));
                    }
                    lines.push(Line::from(spans));
                }
            }
            VisualizerPreset::Blocks | VisualizerPreset::Mirror => {
                let levels = ['▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];
                for row in 0..2 {
                    let mut spans = Vec::with_capacity(w);
                    for &b in bars.iter().take(w) {
                        let idx = ((b * 7.0).round() as usize).min(7);
                        let ch = if row == 0 {
                            levels[idx]
                        } else {
                            levels[7 - idx]
                        };
                        let color = if b > 0.7 {
                            app.theme.warning
                        } else {
                            app.theme.accent
                        };
                        spans.push(Span::styled(ch.to_string(), Style::default().fg(color)));
                    }
                    lines.push(Line::from(spans));
                }
            }
            VisualizerPreset::Gradient => {
                for row in 0..2 {
                    let mut spans = Vec::with_capacity(w);
                    for (i, &b) in bars.iter().take(w).enumerate() {
                        let t = i as f64 / w.max(1) as f64;
                        let color = if t < 0.33 {
                            app.theme.accent
                        } else if t < 0.66 {
                            app.theme.secondary_accent
                        } else {
                            app.theme.tertiary_accent
                        };
                        let ch = if row == 0 {
                            if b > 0.5 {
                                '█'
                            } else if b > 0.25 {
                                '▄'
                            } else {
                                '▁'
                            }
                        } else {
                            if b > 0.5 {
                                '█'
                            } else if b > 0.25 {
                                '▀'
                            } else {
                                '▔'
                            }
                        };
                        spans.push(Span::styled(ch.to_string(), Style::default().fg(color)));
                    }
                    lines.push(Line::from(spans));
                }
            }
            VisualizerPreset::Spectrum => {
                for row in 0..2 {
                    let mut spans = Vec::with_capacity(w);
                    for &b in bars.iter().take(w) {
                        let level = (b * 7.0).round() as usize;
                        let ch = if row == 0 {
                            ['▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'][level.min(7)]
                        } else {
                            ['█', '▇', '▆', '▅', '▄', '▃', '▂', '▁'][level.min(7)]
                        };
                        // The real renderer ramps through `amplitude_color`; a
                        // hardcoded orange here both ignored the theme and
                        // previewed a preset that does not look like itself.
                        let color = crate::visualizer::amplitude_color(b, &app.theme);
                        spans.push(Span::styled(ch.to_string(), Style::default().fg(color)));
                    }
                    lines.push(Line::from(spans));
                }
            }
            VisualizerPreset::BarsDot => {
                for row in 0..2 {
                    let mut spans = Vec::with_capacity(w);
                    for &b in bars.iter().take(w) {
                        let level = (b * 7.0) as u32;
                        let ch = if row == 0 {
                            match level {
                                6.. => '⣿',
                                4.. => '⣷',
                                2.. => '⣧',
                                1.. => '⠇',
                                _ => '⠀',
                            }
                        } else {
                            match level {
                                4.. => '⣿',
                                2.. => '⡇',
                                1.. => '⠁',
                                _ => '⠀',
                            }
                        };
                        spans.push(Span::styled(
                            ch.to_string(),
                            Style::default().fg(app.theme.accent),
                        ));
                    }
                    lines.push(Line::from(spans));
                }
            }
            VisualizerPreset::ClassicPeak => {
                for row in 0..2 {
                    let mut spans = Vec::with_capacity(w);
                    for &b in bars.iter().take(w) {
                        let (ch, color) = if row == 0 {
                            if b > 0.78 {
                                ('⎺', app.theme.accent)
                            } else {
                                (' ', app.theme.bg)
                            }
                        } else if b > 0.35 {
                            ('▏', app.theme.fg_bright)
                        } else {
                            (' ', app.theme.bg)
                        };
                        spans.push(Span::styled(ch.to_string(), Style::default().fg(color)));
                    }
                    lines.push(Line::from(spans));
                }
            }
            VisualizerPreset::Columns => {
                for row in 0..2 {
                    let mut spans = Vec::with_capacity(w);
                    for &b in bars.iter().take(w) {
                        let (ch, color) = if row == 0 {
                            if b > 0.55 {
                                ('█', app.theme.warning)
                            } else {
                                (' ', app.theme.bg)
                            }
                        } else if b > 0.05 {
                            ('█', app.theme.accent)
                        } else {
                            (' ', app.theme.bg)
                        };
                        spans.push(Span::styled(ch.to_string(), Style::default().fg(color)));
                    }
                    lines.push(Line::from(spans));
                }
            }
            VisualizerPreset::Wave => {
                // One-dot braille sweep: dot row follows the sine, column
                // alternates for sub-cell horizontal resolution.
                const ONE_DOT: [char; 8] = ['⠁', '⠂', '⠄', '⡀', '⠈', '⠐', '⠠', '⣀'];
                for _ in 0..2 {
                    let mut spans = Vec::with_capacity(w);
                    for i in 0..w {
                        let t = i as f64 / w.max(1) as f64;
                        let v = (t * std::f64::consts::TAU * 1.5).sin();
                        let y_dot = ((1.0 - v) * 1.5) as usize;
                        let ch = ONE_DOT[y_dot.min(3) + if i % 2 == 0 { 0 } else { 4 }];
                        spans.push(Span::styled(
                            ch.to_string(),
                            Style::default().fg(app.theme.accent),
                        ));
                    }
                    lines.push(Line::from(spans));
                }
            }
            VisualizerPreset::Stereo => {
                let meter = w.saturating_sub(3);
                for (lane, label) in [(0usize, "L "), (1usize, "R ")] {
                    let mut spans = vec![Span::styled(
                        label,
                        Style::default()
                            .fg(app.theme.fg_dim)
                            .add_modifier(ratatui::style::Modifier::BOLD),
                    )];
                    for (x, &b) in bars.iter().take(meter).enumerate() {
                        let lvl = if lane == 0 { b } else { b * 0.72 };
                        let filled = (lvl * meter as f32) as usize;
                        let (ch, color) = if x < filled {
                            ('█', app.theme.accent)
                        } else if x == filled && x < meter {
                            ('▌', app.theme.secondary_accent)
                        } else if x == meter.saturating_sub(1) && lvl > 0.8 {
                            ('·', app.theme.accent)
                        } else {
                            (' ', app.theme.bg)
                        };
                        spans.push(Span::styled(ch.to_string(), Style::default().fg(color)));
                    }
                    lines.push(Line::from(spans));
                }
            }
            VisualizerPreset::Retro => {
                for row in 0..3 {
                    let mut spans = Vec::with_capacity(w);
                    for i in 0..w {
                        let center = i as f64 / w.max(1) as f64;
                        let dist = (center - 0.5).abs() * 2.0;
                        let (ch, color) = match row {
                            0 if dist < 0.62 => ('⠛', app.theme.accent),
                            0 => (' ', app.theme.bg),
                            1 => ('▔', app.theme.fg_dim),
                            _ if dist < 0.4 => ('⣀', app.theme.fg_dim),
                            _ if dist < 0.75 => ('⣀', app.theme.secondary_accent),
                            _ => ('⣀', app.theme.tertiary_accent),
                        };
                        spans.push(Span::styled(ch.to_string(), Style::default().fg(color)));
                    }
                    lines.push(Line::from(spans));
                }
            }
            VisualizerPreset::Flame => {
                for row in 0..2 {
                    let mut spans = Vec::with_capacity(w);
                    for &b in bars.iter().take(w) {
                        let (ch, color) = if row == 0 {
                            if b > 0.6 {
                                ('⠛', app.theme.warning)
                            } else if b > 0.25 {
                                ('⠉', app.theme.secondary_accent)
                            } else {
                                (' ', app.theme.bg)
                            }
                        } else {
                            ('⣀', app.theme.accent)
                        };
                        spans.push(Span::styled(ch.to_string(), Style::default().fg(color)));
                    }
                    lines.push(Line::from(spans));
                }
            }
        }
        lines
    }

    /// Crossfade duration picker.
    pub(crate) fn render_crossfade(f: &mut ratatui::Frame, area: Rect, app: &App) {
        let block = Self::picker_panel(app, " Crossfade Options ", None);
        let inner = block.inner(area);
        f.render_widget(block, area);

        let dur = app
            .state
            .crossfade
            .as_ref()
            .map(|c| c.duration_secs)
            .unwrap_or(0);

        let mut rows: Vec<String> = Vec::new();
        rows.push(" Duration ".to_string());
        for d in CROSSFADE_DURATIONS {
            let cur = if d == dur { "   (current)" } else { "" };
            rows.push(format!("   {d}s{cur}"));
        }

        let sel = app
            .pickers
            .top()
            .map_or(0, |o| o.selected.min(rows.len() - 1));
        let mut lines = Vec::new();
        for (i, row) in rows.iter().enumerate() {
            let is_header = i == 0;
            let is_sel = i == sel;
            let line = if is_header {
                Line::from(Span::styled(
                    row.clone(),
                    Style::default()
                        .fg(app.theme.accent)
                        .add_modifier(Modifier::BOLD),
                ))
            } else {
                let prefix = "   ";
                let style = if is_sel {
                    Style::default()
                        .fg(app.theme.selection_fg_readable())
                        .bg(app.theme.selection_bg)
                } else {
                    Style::default()
                };
                Line::from(Span::styled(format!("{prefix}{row}"), style))
            };
            lines.push(line);
        }

        lines.push(Line::from(""));
        if sel >= 1 && sel < 1 + CROSSFADE_DURATIONS.len() {
            lines.push(Line::from(Span::styled(
                " Select a crossfade duration. 3s is subtle, 30s is ambient.",
                Style::default().fg(app.theme.fg_dim),
            )));
        } else {
            lines.push(Line::from(Span::styled(
                " Choose a crossfade duration.",
                Style::default().fg(app.theme.fg_dim),
            )));
        }

        let para = Paragraph::new(lines);
        f.render_widget(para, inner);
    }
}

// ---------------------------------------------------------------------------
// Audio output device
// ---------------------------------------------------------------------------

/// Label for the entry that hands routing back to the platform.
pub(crate) const DEFAULT_DEVICE_LABEL: &str = "System default";

impl Pickers {
    /// Pick the OS audio output device. The first row is always "System
    /// default", which clears the saved device so the mixer opens the platform
    /// sink — the state a fresh install should be in, and the one that keeps
    /// working when a saved device is unplugged.
    pub(crate) fn render_audio_device(f: &mut ratatui::Frame, area: Rect, app: &mut App) {
        let block = Self::picker_panel(app, " Audio Output ", None);
        let inner = block.inner(area);
        f.render_widget(block, area);

        // Row 0 is the default; the rest are the daemon's list, which is empty
        // until the fetch lands.
        let mut rows: Vec<String> = vec![DEFAULT_DEVICE_LABEL.to_string()];
        rows.extend(app.audio_devices.iter().cloned());

        let current = app.state.audio.audio_device.clone();
        let sel = app
            .pickers
            .top()
            .map_or(0, |o| o.selected.min(rows.len().saturating_sub(1)));

        let visible = inner.height as usize;
        let total = rows.len();
        let (scroll_start, scroll_end) = if let Some(top) = app.pickers.top_mut() {
            let (s, e) = step_viewport(top.viewport_offset, sel, visible, total);
            top.viewport_offset = s;
            (s, e)
        } else {
            (0, total)
        };

        let mut lines = Vec::new();
        for (i, name) in rows
            .iter()
            .enumerate()
            .skip(scroll_start)
            .take(scroll_end.saturating_sub(scroll_start))
        {
            let is_sel = i == sel;
            // The saved device is `None` for the default row, so compare the
            // row's meaning rather than the raw strings.
            let is_cur = match &current {
                None => i == 0,
                Some(cur) => rows.get(i) == Some(cur),
            };
            let prefix = "   ";
            let marker = if is_cur { "  (current)" } else { "" };
            let style = if is_sel {
                Style::default()
                    .fg(app.theme.selection_fg_readable())
                    .bg(app.theme.selection_bg)
            } else {
                Style::default().fg(app.theme.fg)
            };
            lines.push(Line::from(Span::styled(
                format!("{prefix}{name}{marker}"),
                style,
            )));
        }
        if rows.len() == 1 {
            lines.push(Line::from(Span::styled(
                " no output devices reported by this backend",
                Style::default().fg(app.theme.fg_dim),
            )));
        }
        f.render_widget(Paragraph::new(lines), inner);
    }
}
