// Copyright (c) 2026
// Author: prjctimg <prjctimg@outlook.com>
// About, help, notifications and sleep timer
//
//
// This is free software released under the GPL-3.0 license.

use crate::ui::*;

impl Pickers {
    pub(crate) fn render_about(f: &mut ratatui::Frame, area: Rect, app: &App) {
        let block = Self::picker_panel(app, " About ", None);
        let inner = block.inner(area);
        f.render_widget(block, area);

        let version = option_env!("CARGO_PKG_VERSION").unwrap_or("0.1.0");
        let commit = option_env!("VERGEN_GIT_SHA").unwrap_or("unknown");
        // Nightly CI builds bake a `+nightly` suffix into `CARGO_PKG_VERSION`;
        // surface that so users can tell a nightly build from a release.
        let nightly = version.contains("+nightly");

        let mut lines = vec![
            Line::from(Span::styled(
                format!(
                    "gtm {version} ({:.7}{})",
                    commit,
                    if nightly { " nightly" } else { "" }
                ),
                Style::default()
                    .fg(app.theme.accent)
                    .add_modifier(Modifier::BOLD),
            )),
            Line::from(""),
            Line::from(Span::styled(
                "Copyright (C) 2026, prjctimg",
                Style::default().fg(app.theme.fg_dim),
            )),
            Line::from(Span::styled(
                "License GPL-3.0",
                Style::default().fg(app.theme.fg_dim),
            )),
            Line::from(""),
        ];

        // Decorative visualization under the text, cycling presets every 5s.
        let presets = VisualizerPreset::all();
        if !presets.is_empty() {
            let preset = presets[app.about_viz.preset % presets.len()];
            let width = inner.width.saturating_sub(2) as usize;
            if width > 0 && inner.height as usize > lines.len() + 4 {
                lines.extend(Self::visualizer_preview_lines(
                    preset,
                    &app.about_viz.bars,
                    width as u16,
                    app,
                ));
            }
        }

        let p = Paragraph::new(lines).alignment(Alignment::Center);
        f.render_widget(p, inner);
    }

    pub(crate) fn render_help(f: &mut ratatui::Frame, area: Rect, app: &mut App) {
        let block = Self::picker_panel(
            app,
            " Keybindings ",
            Some(
                "Esc: close   ?: toggle   \u{2191}/\u{2193}/jk: browse   gg/G: top/bottom   0/$: first/last",
            ),
        );
        let inner = block.inner(area);
        f.render_widget(block, area);

        let filtered: Vec<(&str, &str)> = HELP_LINES.to_vec();

        let mut lines: Vec<Line> = Vec::new();

        let total = filtered.len();
        let sel = app
            .pickers
            .top()
            .map_or(0, |o| o.selected.min(total.saturating_sub(1)));
        let visible = inner.height as usize;
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

        for (i, (kind, line)) in filtered
            .iter()
            .enumerate()
            .take(scroll_end)
            .skip(scroll_start)
        {
            let style = if i == sel {
                Style::default()
                    .fg(app.theme.selection_fg_readable())
                    .bg(app.theme.selection_bg)
            } else if *kind == "topic" {
                Style::default()
                    .fg(app.theme.accent)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(app.theme.fg)
            };
            lines.push(Line::from(Span::styled(*line, style)));
        }

        let para = Paragraph::new(lines);
        f.render_widget(para, inner);
    }

    pub(crate) fn render_notifications(f: &mut ratatui::Frame, area: Rect, app: &mut App) {
        let block = Self::picker_panel(app, " Notifications ", None);
        let inner = block.inner(area);
        f.render_widget(block, area);

        if app.notification_history.is_empty() {
            let para = Paragraph::new(Line::from(Span::styled(
                " No notifications yet",
                Style::default().fg(app.theme.fg_dim),
            )));
            f.render_widget(para, inner);
            return;
        }

        let preview_h: u16 = (inner.height / 3)
            .clamp(3, 6)
            .min(inner.height.saturating_sub(3));
        let list_h = inner.height.saturating_sub(preview_h + 1);
        let list_area = Rect {
            x: inner.x,
            y: inner.y,
            width: inner.width,
            height: list_h,
        };
        let preview_area = Rect {
            x: inner.x,
            y: inner.y + list_h + 1,
            width: inner.width,
            height: preview_h,
        };

        let total = app.notification_history.len();
        let sel = app.pickers.top().map_or(0, |o| o.selected.min(total - 1));
        let visible = list_area.height as usize;
        let (scroll_start, scroll_end) = if let Some(top) = app.pickers.top_mut() {
            let (s, e) = step_viewport(top.viewport_offset, sel, visible, total);
            top.viewport_offset = s;
            (s, e)
        } else {
            (0, total.min(visible))
        };

        let now = std::time::Instant::now();
        let row_w = inner.width;
        let mut lines: Vec<Line> = Vec::new();
        for (i, rec) in app
            .notification_history
            .iter()
            .enumerate()
            .take(scroll_end)
            .skip(scroll_start)
        {
            let is_sel = i == sel;
            let (glyph, color) = match rec.kind {
                NotificationKind::Info => ("\u{2139} ", app.theme.accent),
                NotificationKind::Success => ("\u{2713} ", app.theme.success),
                NotificationKind::Warning => ("\u{26a0} ", app.theme.warning),
                NotificationKind::Error => ("\u{2717} ", app.theme.error),
            };
            let age = Self::relative_age(now.saturating_duration_since(rec.at));
            let style = if is_sel {
                Style::default()
                    .fg(app.theme.selection_fg_readable())
                    .bg(app.theme.selection_bg)
            } else {
                Style::default().fg(app.theme.fg)
            };
            let glyph_style = if is_sel {
                style
            } else {
                Style::default().fg(color)
            };
            let age_style = if is_sel {
                style
            } else {
                Style::default().fg(app.theme.fg_dim)
            };
            let head = format!(
                " {title}: {message}",
                title = rec.title,
                message = rec.message
            );
            let tail = format!("{age:>6}");
            let pad = row_pad(&format!("{glyph}{head}{tail}"), row_w);
            lines.push(Line::from(vec![
                Span::styled(glyph.to_string(), glyph_style),
                Span::styled(format!("{head}{}", " ".repeat(pad)), style),
                Span::styled(tail, age_style),
            ]));
            let row_rect = Rect {
                x: list_area.x,
                y: list_area.y + (i - scroll_start) as u16,
                width: list_area.width,
                height: 1,
            };
            app.mouse_map.register(row_rect, MouseZone::PickerItem(i));
        }

        let para = Paragraph::new(lines);
        f.render_widget(para, list_area);

        // Preview pane below list: wrapped full message of the highlighted notification.
        if preview_area.height >= 2 {
            // Divider
            let rule = Paragraph::new(Line::from(Span::styled(
                "─".repeat(preview_area.width as usize),
                Style::default().fg(app.theme.muted_border),
            )));
            f.render_widget(
                rule,
                Rect {
                    x: preview_area.x,
                    y: preview_area.y.saturating_sub(1),
                    width: preview_area.width,
                    height: 1,
                },
            );
            if let Some(rec) = app.notification_history.get(sel) {
                let (glyph, color) = match rec.kind {
                    NotificationKind::Info => ("ℹ ", app.theme.accent),
                    NotificationKind::Success => ("✓ ", app.theme.success),
                    NotificationKind::Warning => ("⚠ ", app.theme.warning),
                    NotificationKind::Error => ("✗ ", app.theme.error),
                };
                let title_style = Style::default().fg(color).add_modifier(Modifier::BOLD);
                let msg_style = Style::default().fg(app.theme.fg);
                // Update preview on scroll: reads sel each frame, so preview follows highlight.
                let preview_text = format!("{}{}: {}", glyph, rec.title, rec.message);
                let preview_para = Paragraph::new(preview_text)
                    .style(msg_style)
                    .wrap(Wrap { trim: false });
                // Kind-colored header line
                let header = Paragraph::new(Line::from(vec![
                    Span::styled(glyph.to_string(), title_style),
                    Span::styled(rec.title.clone(), title_style),
                ]));
                let header_h = 1;
                f.render_widget(
                    header,
                    Rect {
                        x: preview_area.x,
                        y: preview_area.y,
                        width: preview_area.width,
                        height: header_h,
                    },
                );
                // Reserve one trailing row (when the preview pane is tall
                // enough) for the copy hint.
                let hint_h = u16::from(preview_area.height >= 4);
                let msg_area = Rect {
                    x: preview_area.x,
                    y: preview_area.y + header_h,
                    width: preview_area.width,
                    height: preview_area.height.saturating_sub(header_h + hint_h),
                };
                f.render_widget(preview_para, msg_area);
                if hint_h > 0 {
                    let hint = Paragraph::new(Line::from(Span::styled(
                        " y — copy notification to clipboard",
                        Style::default().fg(app.theme.fg_dim),
                    )));
                    f.render_widget(
                        hint,
                        Rect {
                            x: preview_area.x,
                            y: preview_area.y + preview_area.height - hint_h,
                            width: preview_area.width,
                            height: hint_h,
                        },
                    );
                }
            }
        }
    }

    pub(crate) fn render_notification_settings(f: &mut ratatui::Frame, area: Rect, app: &mut App) {
        let block = Self::picker_panel(app, " Notification Settings ", None);
        let inner = block.inner(area);
        f.render_widget(block, area);

        let sel = app
            .pickers
            .top()
            .map_or(0, |o| o.selected.min(NotifType::ALL.len() - 1));

        let mut lines: Vec<Line> = Vec::new();
        for (i, ntype) in NotifType::ALL.iter().enumerate() {
            let mode = app
                .notification_modes
                .get(ntype)
                .copied()
                .unwrap_or(NotifMode::Floating);
            let is_sel = i == sel;
            let style = if is_sel {
                Style::default()
                    .fg(app.theme.selection_fg_readable())
                    .bg(app.theme.selection_bg)
            } else {
                Style::default().fg(app.theme.fg)
            };
            let mode_style = if is_sel {
                style
            } else {
                Style::default().fg(app.theme.accent)
            };
            lines.push(Line::from(vec![
                Span::styled(format!(" {:<26}", ntype.label()), style),
                Span::styled(mode.label().to_string(), mode_style),
            ]));
        }
        let para = Paragraph::new(lines);
        f.render_widget(para, inner);
    }

    pub(crate) fn relative_age(elapsed: std::time::Duration) -> String {
        let s = elapsed.as_secs();
        if s < 60 {
            format!("{s}s")
        } else if s < 3600 {
            format!("{}m", s / 60)
        } else {
            format!("{}h", s / 3600)
        }
    }

    pub(crate) fn render_sleep_timer(f: &mut ratatui::Frame, area: Rect, app: &App) {
        if app.sleep_timer.input_mode {
            let block = Self::picker_panel(app, " Sleep Timer: Manual Input ", None);
            let inner = block.inner(area);
            f.render_widget(block, area);
            let cursor_style = cursor_span_style(app);
            let label = Paragraph::new(Line::from(vec![
                Span::styled(" Enter minutes: ", Style::default().fg(app.theme.fg_dim)),
                Span::styled(
                    app.sleep_timer.input_buf.as_str(),
                    Style::default()
                        .fg(app.theme.accent)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(" ", cursor_style.unwrap_or_default()),
            ]));
            f.render_widget(label, inner);
            return;
        }

        let is_active = app.sleep_timer.remaining.is_some();
        let help = if is_active {
            "↑/↓: option   ←/→ or h/l: ±5   -/+: ±1   i: input   Enter: set   c: cancel"
        } else {
            "↑/↓: option   ←/→ or h/l: ±5   -/+: ±1   i: input   Enter: set"
        };
        let block = Self::picker_panel(app, " Sleep Timer ", Some(help));
        let inner = block.inner(area);
        f.render_widget(block, area);

        let mins = app.sleep_timer.minutes;
        let focus = app.sleep_timer.focus;

        let focus_style = |app: &App, active: bool| {
            if active {
                Style::default()
                    .fg(app.theme.selection_fg_readable())
                    .bg(app.theme.selection_bg)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(app.theme.fg)
            }
        };

        let mut lines: Vec<Line> = Vec::new();

        // Row 0: the time slider (focus 0). Left/Right and h/l adjust it;
        // Enter arms the timer with the current minutes.
        lines.push(Line::from(vec![Span::styled(
            format!("  Timer: {} minutes", mins),
            focus_style(app, focus == 0),
        )]));
        lines.push(Line::from(""));

        let slider_w = inner.width.saturating_sub(4) as u32;
        let pos = if slider_w > 0 {
            ((mins as f32 / 180.0) * slider_w as f32) as u32
        } else {
            0
        };
        let filled: String = "─".repeat(pos as usize);
        let empty: String = "─".repeat((slider_w.saturating_sub(pos + 1)) as usize);
        lines.push(Line::from(vec![
            Span::styled("  ", Style::default()),
            Span::styled(filled, Style::default().fg(app.theme.success)),
            Span::styled(
                "●",
                Style::default()
                    .fg(app.theme.accent)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(empty, Style::default().fg(app.theme.fg_dim)),
        ]));
        lines.push(Line::from(vec![
            Span::styled("  0m", Style::default().fg(app.theme.fg_dim)),
            Span::styled(
                format!("{:.0}m", 180.0),
                Style::default().fg(app.theme.fg_dim),
            ),
        ]));
        lines.push(Line::from(""));

        // Rows 1..=7: quick presets. `focus - 1` indexes into this list, and
        // the focused chip is highlighted (the value matches `mins` whenever
        // the user adjusted the slider to a preset).
        let quick_opts = [5u32, 10, 15, 30, 60, 90, 120];
        let mut spans: Vec<Span> = vec![Span::styled("  ", Style::default())];
        for (i, &m) in quick_opts.iter().enumerate() {
            let focused = (1..=7).contains(&focus) && focus - 1 == i;
            let style = if focused {
                Style::default()
                    .fg(app.theme.selection_fg_readable())
                    .bg(app.theme.accent)
                    .add_modifier(Modifier::BOLD)
            } else if m == mins {
                Style::default()
                    .fg(app.theme.selection_fg_readable())
                    .bg(app.theme.selection_bg)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(app.theme.fg_dim)
            };
            spans.push(Span::styled(format!("[{}m] ", m), style));
        }
        lines.push(Line::from(spans));
        lines.push(Line::from(""));

        // Row 8: the "stop immediately" checkbox.
        let boxed = if app.sleep_timer.stop_immediately {
            "[x]"
        } else {
            "[ ]"
        };
        lines.push(Line::from(vec![Span::styled(
            format!("  {} End playback immediately when the time is up", boxed),
            focus_style(app, focus == 8),
        )]));
        lines.push(Line::from(""));

        if is_active && let Some(remaining) = app.sleep_timer.remaining {
            let r_mins = remaining / 60;
            let r_secs = remaining % 60;
            lines.push(Line::from(Span::styled(
                format!("  Active: {:02}:{:02} remaining", r_mins, r_secs),
                Style::default()
                    .fg(app.theme.success)
                    .add_modifier(Modifier::BOLD),
            )));
        }
        lines.push(Line::from(""));

        let paragraph = Paragraph::new(lines);
        f.render_widget(paragraph, inner);
    }
}
