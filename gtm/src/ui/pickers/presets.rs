// Copyright (c) 2026
// Author: prjctimg <prjctimg@outlook.com>
// EQ, theme, crossfade and visualizer presets
//
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
            let prefix = if is_sel { " > " } else { "   " };
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
                        let color = ratatui::style::Color::Rgb(
                            (b * 200.0) as u8,
                            (b * 120.0 + 40.0) as u8,
                            (120.0 - b * 80.0) as u8,
                        );
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

    pub(crate) fn render_theme(f: &mut ratatui::Frame, area: Rect, app: &mut App) {
        let query = app.pickers.top().map_or(String::new(), |o| o.query.clone());
        let filtered: Vec<_> = app
            .themes
            .iter()
            .enumerate()
            .filter(|(_, entry)| fuzzy_match(&query, &entry.name))
            .collect();

        let total = filtered.len();
        let sel = app
            .pickers
            .top()
            .map_or(0, |o| o.selected.min(total.saturating_sub(1)));

        let block = Self::picker_panel(app, " Theme ", None);
        let inner = block.inner(area);
        f.render_widget(block, area);

        let cursor_style = cursor_span_style(app);
        let search_line = Line::from(vec![
            Span::styled(" > ", Style::default().fg(app.theme.fg_dim)),
            Span::styled(query.as_str(), Style::default().fg(app.theme.fg)),
            Span::styled(" ", cursor_style.unwrap_or_default()),
        ]);

        // Each theme row is padded with a blank line so the swatch runs don't
        // visually merge into one continuous color band; the math must count
        // rows at 2 cells each (the search line stays 1 high).
        let item_h: u16 = 2;
        let visible = (inner.height.saturating_sub(1) / item_h) as usize;
        let (scroll_start, scroll_end) = if total > 0 {
            if let Some(top) = app.pickers.top_mut() {
                let (s, e) = step_viewport(top.viewport_offset, sel, visible, total);
                top.viewport_offset = s;
                (s, e)
            } else {
                (0, total)
            }
        } else {
            (0, 0)
        };

        let mut list_items: Vec<ListItem> = vec![ListItem::new(search_line)];
        let row_w = inner.width as usize;
        for (visible_idx, &(i, entry)) in filtered[scroll_start..scroll_end].iter().enumerate() {
            let is_active = i == app.theme_index;
            let prefix = if i == sel { " > " } else { "   " };
            let check = if is_active { " \u{2713}" } else { "" };
            let light_badge = if entry.light { " \u{2600}" } else { "" };
            let name_part = format!("{}{}{}", prefix, entry.name, light_badge);

            let colors = [
                entry.theme.bg,
                entry.theme.fg,
                entry.theme.accent,
                entry.theme.secondary_accent,
                entry.theme.tertiary_accent,
                entry.theme.border,
            ];
            let swatch_run_w = colors.len() * 2 + colors.len() + 1;
            let left_w = name_part.chars().count() + check.chars().count() + 1;
            let pad = row_w.saturating_sub(left_w + swatch_run_w + 2);

            let name_style = if i == sel {
                Style::default()
                    .fg(app.theme.selection_fg_readable())
                    .bg(app.theme.selection_bg)
            } else {
                Style::default()
            };
            let mut spans: Vec<Span> = vec![Span::styled(
                format!("{name_part}{:pad$}", "", pad = pad),
                name_style,
            )];
            for c in colors {
                spans.push(Span::styled("  ", Style::default().fg(c).bg(c)));
                spans.push(Span::raw(" "));
            }
            spans.push(Span::styled(check, Style::default()));
            let style = if i == sel {
                Style::default()
            } else if is_active {
                Style::default()
                    .fg(app.theme.accent)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default()
            };
            list_items.push(ListItem::new(vec![Line::from(spans), Line::from("")]).style(style));
            let row_rect = Rect {
                x: inner.x,
                y: inner.y + 1 + (visible_idx as u16) * item_h,
                width: inner.width,
                height: item_h,
            };
            app.mouse_map.register(row_rect, MouseZone::PickerItem(i));
        }

        let list = List::new(list_items);
        f.render_widget(list, inner);
    }

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
                let prefix = if is_sel { " > " } else { "   " };
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

    pub(crate) fn render_visualizer_preset(f: &mut ratatui::Frame, area: Rect, app: &mut App) {
        let block = Self::picker_panel(app, " Visualizer Preset ", None);
        let inner = block.inner(area);
        f.render_widget(block, area);

        let current = app.visualizer.preset;
        let presets = VisualizerPreset::all();

        let sel = app
            .pickers
            .top()
            .map_or(0, |o| o.selected.min(presets.len() - 1));

        let preview_height: u16 = 4;
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

        let mut lines = Vec::new();
        for (i, preset) in presets
            .iter()
            .enumerate()
            .skip(scroll_start)
            .take(scroll_end - scroll_start)
        {
            let is_sel = i == sel;
            let is_cur = *preset == current;
            let prefix = if is_sel { " > " } else { "   " };
            let cur = if is_cur { "   (current)" } else { "" };
            let style = if is_sel {
                Style::default()
                    .fg(app.theme.selection_fg_readable())
                    .bg(app.theme.selection_bg)
            } else if is_cur {
                Style::default().fg(app.theme.accent)
            } else {
                Style::default()
            };
            lines.push(Line::from(Span::styled(
                format!("{prefix}{}{cur}", preset.name()),
                style,
            )));
        }

        let list = List::new(lines.into_iter().map(ListItem::new).collect::<Vec<_>>());
        f.render_widget(list, list_area);

        if preview_area.height >= 3 {
            let selected_name = presets.get(sel).map(|p| p.name()).unwrap_or("");
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

            let sel_preset = presets.get(sel).copied().unwrap_or(current);
            let bar_w = preview_area.width.saturating_sub(4) as usize;
            let sample_bars: Vec<f32> = (0..bar_w)
                .map(|i| {
                    let t = i as f64 / bar_w.max(1) as f64;
                    ((t * std::f64::consts::TAU).sin() * 0.5 + 0.5) as f32
                })
                .collect();

            let preview_lines =
                Self::visualizer_preview_lines(sel_preset, &sample_bars, preview_area.width, app);
            for (row, line) in preview_lines
                .iter()
                .enumerate()
                .take(preview_area.height.saturating_sub(1) as usize)
            {
                f.render_widget(
                    Paragraph::new(line.clone()),
                    Rect {
                        x: preview_area.x,
                        y: preview_area.y + 1 + row as u16,
                        width: preview_area.width,
                        height: 1,
                    },
                );
            }
        }
    }

    pub(crate) fn render_progress_style(f: &mut ratatui::Frame, area: Rect, app: &mut App) {
        let block = Self::picker_panel(app, " Progress Style ", None);
        let inner = block.inner(area);
        f.render_widget(block, area);

        let current = app.progress_style;
        let styles = ProgressStyle::all();

        let sel = app
            .pickers
            .top()
            .map_or(0, |o| o.selected.min(styles.len() - 1));

        let preview_height: u16 = 4;
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
        let total = styles.len();
        let (scroll_start, scroll_end) = if let Some(top) = app.pickers.top_mut() {
            let (s, e) = step_viewport(top.viewport_offset, sel, visible, total);
            top.viewport_offset = s;
            (s, e)
        } else {
            (0, total)
        };

        let mut lines = Vec::new();
        for (i, style) in styles
            .iter()
            .enumerate()
            .skip(scroll_start)
            .take(scroll_end - scroll_start)
        {
            let is_sel = i == sel;
            let is_cur = *style == current;
            let prefix = if is_sel { " > " } else { "   " };
            let cur = if is_cur { "   (current)" } else { "" };
            let line_style = if is_sel {
                Style::default()
                    .fg(app.theme.selection_fg_readable())
                    .bg(app.theme.selection_bg)
            } else if is_cur {
                Style::default().fg(app.theme.accent)
            } else {
                Style::default()
            };
            lines.push(Line::from(Span::styled(
                format!("{prefix}{}{cur}", style.name()),
                line_style,
            )));
        }

        let list = List::new(lines.into_iter().map(ListItem::new).collect::<Vec<_>>());
        f.render_widget(list, list_area);

        if preview_area.height >= 3 {
            let selected_name = styles.get(sel).map(|s| s.name()).unwrap_or("");
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

            let sel_style = styles.get(sel).copied().unwrap_or(current);
            let preview_w = preview_area.width.saturating_sub(2) as usize;
            let spans = render_progress_styled(
                0.6,
                preview_w,
                sel_style,
                app.theme.accent,
                app.theme.secondary_accent,
                app.theme.tertiary_accent,
            );
            f.render_widget(
                Paragraph::new(Line::from(spans)),
                Rect {
                    x: preview_area.x,
                    y: preview_area.y + 1,
                    width: preview_area.width,
                    height: 1,
                },
            );

            let preview_label = Line::from(Span::styled(
                format!("  60% filled, {} chars wide", preview_w),
                Style::default().fg(app.theme.fg_dim),
            ));
            f.render_widget(
                Paragraph::new(preview_label),
                Rect {
                    x: preview_area.x,
                    y: preview_area.y + 2,
                    width: preview_area.width,
                    height: 1,
                },
            );
        }
    }

    pub(crate) fn render_footer_preset(f: &mut ratatui::Frame, area: Rect, app: &mut App) {
        let block = Self::picker_panel(app, " Footer Preset ", None);
        let inner = block.inner(area);
        f.render_widget(block, area);

        let current = app.footer_preset;
        let presets = &app.footer_presets;

        let sel = app
            .pickers
            .top()
            .map_or(0, |o| o.selected.min(presets.len().saturating_sub(1)));

        let preview_height: u16 = 4;
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

        let mut lines = Vec::new();
        for (i, preset) in presets
            .iter()
            .enumerate()
            .skip(scroll_start)
            .take(scroll_end.saturating_sub(scroll_start))
        {
            let is_sel = i == sel;
            let is_cur = i == current;
            let prefix = if is_sel { " > " } else { "   " };
            let cur = if is_cur { "   (current)" } else { "" };
            let line_style = if is_sel {
                Style::default()
                    .fg(app.theme.selection_fg_readable())
                    .bg(app.theme.selection_bg)
            } else if is_cur {
                Style::default().fg(app.theme.accent)
            } else {
                Style::default()
            };
            lines.push(Line::from(Span::styled(
                format!("{prefix}{}{cur}", preset.name),
                line_style,
            )));
        }

        let list = List::new(lines.into_iter().map(ListItem::new).collect::<Vec<_>>());
        f.render_widget(list, list_area);

        if preview_area.height >= 3 {
            let layout = presets
                .get(sel)
                .map(|p| {
                    let mut n = p.left.len() + p.right.len();
                    if n == 0 {
                        n = 1;
                    }
                    format!(
                        "left {} \u{2022} right {} \u{2022} {} module{}",
                        p.left.len(),
                        p.right.len(),
                        n,
                        if n == 1 { "" } else { "s" }
                    )
                })
                .unwrap_or_default();
            let rule = Line::from(vec![
                Span::styled(
                    "\u{2500}".to_string(),
                    Style::default().fg(app.theme.muted_border),
                ),
                Span::styled(format!(" {layout} "), Style::default().fg(app.theme.fg_dim)),
                Span::styled(
                    "\u{2500}".repeat(
                        preview_area
                            .width
                            .saturating_sub(layout.len() as u16 + 4)
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

            // Live preview of the selected preset rendered as brand-badge
            // style per-module swatches, so the user sees the real layout.
            let preview_label = Line::from(Span::styled(
                "  dragging the selection previews the footer layout",
                Style::default().fg(app.theme.fg_dim),
            ));
            f.render_widget(
                Paragraph::new(preview_label),
                Rect {
                    x: preview_area.x,
                    y: preview_area.y + 1,
                    width: preview_area.width,
                    height: 1,
                },
            );
        }
    }
}
