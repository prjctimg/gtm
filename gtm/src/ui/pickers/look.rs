// Copyright (c) 2026
// Author: prjctimg <prjctimg@outlook.com>
// Unified presentation picker: zen layout, theme, visualizer, progress bar, footer
//
// This is free software released under the GPL-3.0 license.

use crate::app::Look;
use crate::ui::*;

/// One row of the value list: its label, its index into whatever backs the
/// category, and whether it is the value in effect right now.
struct LookItem {
    label: String,
    source: usize,
    current: bool,
}

impl Pickers {
    /// The one picker for every setting that changes how the TUI looks.
    ///
    /// These were five overlays behind five keys, and each drew the same thing: a
    /// list, the current choice marked, and a preview of the value under the
    /// cursor. Only the list and the preview differed, so the difference is two
    /// functions here rather than five renderers.
    ///
    /// They also had no category list of their own — each overlay opened on its
    /// own values. Merging them added one, and it took a row of every picker's
    /// height to do it: five rows to switch between five categories that `Tab`
    /// already switches between. So it is gone again, and the current category
    /// is the block title.
    ///
    /// `Left`/`Right` change the value as the cursor moves, so the preview shows
    /// the value about to be applied rather than one already applied.
    pub(crate) fn render_look(f: &mut ratatui::Frame, area: Rect, app: &mut App) {
        let cat = app.look.cat();
        let hint = Self::look_hint(cat);
        let title = format!(" {} ", cat.label());
        let block = Self::picker_panel(app, &title, Some(hint));
        let inner = block.inner(area);
        f.render_widget(block, area);

        // Values, then preview. No category strip: the five presets this was
        // merged from had none, and re-introducing one as a row of its own is
        // what the merge was supposed to avoid. The category is the block title
        // and `Tab`/`Shift-Tab` cycle it, so the picker is back to one list over
        // one preview — the shape it had before it became one component.
        let items = Self::look_items(app, cat);
        let total = items.len();
        let sel = app
            .pickers
            .top()
            .map_or(0, |o| o.selected)
            .min(total.max(1) - 1);

        // The search line only the one category that filters has, and it is part
        // of the list, so it takes its row out of the visible count.
        let header_h: u16 = if cat == Look::Theme { 1 } else { 0 };
        let preview_h = 5u16.min(inner.height);
        let list_h = inner.height.saturating_sub(preview_h);
        let visible = (list_h.saturating_sub(header_h) as usize).max(1);
        let (scroll_start, scroll_end) = if total == 0 {
            (0, 0)
        } else if let Some(top) = app.pickers.top_mut() {
            let (s, e) = step_viewport(top.viewport_offset, sel, visible, total);
            top.viewport_offset = s;
            (s, e)
        } else {
            (0, total)
        };

        let list_area = Rect {
            height: list_h,
            ..inner
        };
        let mut row_y = list_area.y;
        if header_h > 0 {
            f.render_widget(
                Paragraph::new(Line::from(vec![
                    Span::styled(" > ", Style::default().fg(app.theme.fg_dim)),
                    Span::styled(app.look.query.clone(), Style::default().fg(app.theme.fg)),
                    Span::styled(" ", cursor_span_style(app).unwrap_or_default()),
                ])),
                Rect {
                    y: row_y,
                    height: 1,
                    ..list_area
                },
            );
            row_y += 1;
        }

        if total == 0 {
            f.render_widget(
                Paragraph::new(Line::from(Span::styled(
                    " nothing matches",
                    Style::default().fg(app.theme.fg_dim),
                ))),
                Rect {
                    y: row_y,
                    height: 1,
                    ..list_area
                },
            );
        }

        for (i, item) in items[scroll_start..scroll_end].iter().enumerate() {
            let i = i + scroll_start;
            let selected = i == sel;
            let style = if selected {
                Style::default()
                    .fg(app.theme.selection_fg_readable())
                    .bg(app.theme.selection_bg)
            } else if item.current {
                Style::default().fg(app.theme.accent)
            } else {
                Style::default().fg(app.theme.fg)
            };
            // No leading gutter. It only existed to line the value labels up
            // under the category strip's own marker column, and with the strip
            // gone it is three columns of nothing in front of every name.
            let cur = if item.current && !selected {
                "  \u{2713}"
            } else {
                ""
            };
            let mut spans = vec![Span::styled(format!("{}{cur}", item.label), style)];
            // Themes are the one category whose values are not names, so they get
            // the colours drawn beside the name: it is the only way to pick one
            // by looking at it.
            if cat == Look::Theme
                && let Some(t) = app.themes.get(item.source)
            {
                for c in [
                    t.theme.bg,
                    t.theme.fg,
                    t.theme.accent,
                    t.theme.secondary_accent,
                    t.theme.tertiary_accent,
                    t.theme.border,
                ] {
                    spans.push(Span::styled("  ".to_string(), Style::default().fg(c).bg(c)));
                    spans.push(Span::raw(" "));
                }
                if t.light {
                    spans.push(Span::styled(
                        " \u{2600}",
                        Style::default().fg(app.theme.warning),
                    ));
                }
            }
            f.render_widget(
                Paragraph::new(Line::from(spans)),
                Rect {
                    y: row_y,
                    height: 1,
                    ..list_area
                },
            );
            let row_rect = Rect {
                y: row_y,
                height: 1,
                ..list_area
            };
            app.mouse_map.register(row_rect, MouseZone::PickerItem(i));
            row_y += 1;
        }

        if preview_h >= 3 {
            let preview = Rect {
                y: inner.y + list_h,
                height: preview_h,
                ..inner
            };
            f.render_widget(
                Paragraph::new(Self::look_rule(app, cat, sel)),
                Rect {
                    height: 1,
                    ..preview
                },
            );
            for (row, line) in Self::look_preview(app, cat, sel, preview)
                .iter()
                .enumerate()
                .take(preview.height.saturating_sub(1) as usize)
            {
                f.render_widget(
                    Paragraph::new(line.clone()),
                    Rect {
                        y: preview.y + 1 + row as u16,
                        height: 1,
                        ..preview
                    },
                );
            }
        }
    }

    /// Hint line. Theme puts `enter apply` first because it is the one category
    /// with a search box, so the query is where the eye goes.
    fn look_hint(cat: Look) -> &'static str {
        match cat {
            Look::Theme => "type to filter \u{b7} left/right pick \u{b7} tab category",
            _ => "left/right pick \u{b7} enter apply \u{b7} tab/shift+tab category",
        }
    }

    /// The values one category offers.
    fn look_items(app: &App, cat: Look) -> Vec<LookItem> {
        match cat {
            Look::Layout => [
                ZenSurface::Lyrics,
                ZenSurface::NowPlaying,
                ZenSurface::Visualizer,
            ]
            .into_iter()
            .enumerate()
            .map(|(i, s)| LookItem {
                label: match s {
                    ZenSurface::Lyrics => "Lyrics".into(),
                    ZenSurface::NowPlaying => "Now Playing".into(),
                    ZenSurface::Visualizer => "Visualizer".into(),
                },
                source: i,
                current: app.zen_surface == s,
            })
            .collect(),
            Look::Theme => {
                let q = app.look.query.to_lowercase();
                app.themes
                    .iter()
                    .enumerate()
                    .filter(|(_, t)| q.is_empty() || t.name.to_lowercase().contains(&q))
                    .map(|(i, t)| LookItem {
                        label: t.name.to_string(),
                        source: i,
                        current: i == app.theme_index,
                    })
                    .collect()
            }
            Look::Visualizer => VisualizerPreset::all()
                .iter()
                .enumerate()
                .map(|(i, p)| LookItem {
                    label: p.name().to_string(),
                    source: i,
                    current: VisualizerPreset::all().get(i) == Some(&app.visualizer.preset),
                })
                .collect(),
            Look::Progress => ProgressStyle::all()
                .iter()
                .enumerate()
                .map(|(i, p)| LookItem {
                    label: p.name().to_string(),
                    source: i,
                    current: ProgressStyle::all().get(i) == Some(&app.progress_style),
                })
                .collect(),
            Look::Footer => app
                .footer_presets
                .iter()
                .enumerate()
                .map(|(i, p)| LookItem {
                    label: p.name.to_string(),
                    source: i,
                    current: i == app.footer_preset,
                })
                .collect(),
        }
    }

    /// The rule above the preview, naming what is being previewed.
    fn look_rule(app: &App, cat: Look, sel: usize) -> Line<'static> {
        let text = Self::look_items(app, cat)
            .get(sel)
            .map(|i| i.label.clone())
            .unwrap_or_default();
        Line::from(vec![
            Span::styled(
                "\u{2500} ".to_string(),
                Style::default().fg(app.theme.muted_border),
            ),
            Span::styled(
                format!("{text} "),
                Style::default()
                    .fg(app.theme.accent)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                "\u{2500}".repeat(text.chars().count() + 3),
                Style::default().fg(app.theme.muted_border),
            ),
        ])
    }

    /// The preview lines for the highlighted value.
    ///
    /// Each category previews the thing it actually changes — a bar, a set of
    /// bands, a module list — rather than a sentence about it.
    fn look_preview(app: &App, cat: Look, sel: usize, area: Rect) -> Vec<Line<'static>> {
        let w = area.width.saturating_sub(2) as usize;
        match cat {
            Look::Layout => {
                let what = match app.zen_surface {
                    ZenSurface::Lyrics => "fullscreen lyrics",
                    ZenSurface::NowPlaying => "the now-playing card",
                    ZenSurface::Visualizer => "the fullscreen visualizer",
                };
                vec![Line::from(Span::styled(
                    format!("  z and daydreaming open on {what}"),
                    Style::default().fg(app.theme.fg_dim),
                ))]
            }
            Look::Theme => {
                let n = Self::look_items(app, cat).len();
                vec![
                    Line::from(Span::styled(
                        format!(
                            "  {n} theme{}, applied as the cursor moves",
                            if n == 1 { "" } else { "s" }
                        ),
                        Style::default().fg(app.theme.fg_dim),
                    )),
                    Line::from(Span::styled(
                        "  the app behind this panel is already the one you are on",
                        Style::default().fg(app.theme.fg_dim),
                    )),
                ]
            }
            Look::Visualizer => {
                let preset = VisualizerPreset::all()
                    .get(sel)
                    .copied()
                    .unwrap_or(app.visualizer.preset);
                let bars: Vec<f32> = (0..w)
                    .map(|i| {
                        let t = i as f64 / w.max(1) as f64;
                        ((t * std::f64::consts::TAU).sin() * 0.5 + 0.5) as f32
                    })
                    .collect();
                Self::visualizer_preview_lines(preset, &bars, area.width, app)
                    .into_iter()
                    .skip(1)
                    .collect()
            }
            Look::Progress => {
                let style = ProgressStyle::all()
                    .get(sel)
                    .copied()
                    .unwrap_or(app.progress_style);
                let spans = render_progress_styled(
                    0.6,
                    w,
                    style,
                    app.theme.accent,
                    app.theme.secondary_accent,
                    app.theme.tertiary_accent,
                );
                vec![
                    Line::from(spans),
                    Line::from(Span::styled(
                        "  60% of the way through",
                        Style::default().fg(app.theme.fg_dim),
                    )),
                ]
            }
            Look::Footer => {
                let n = app
                    .footer_presets
                    .get(sel)
                    .map_or(0, |p| p.left.len() + p.right.len());
                let modules = if n == 0 { 0 } else { n };
                vec![
                    Line::from(Span::styled(
                        format!(
                            "  {} module{} across the footer",
                            modules,
                            if modules == 1 { "" } else { "s" }
                        ),
                        Style::default().fg(app.theme.fg_dim),
                    )),
                    Line::from(Span::styled(
                        "  left of the transport, right of the clock",
                        Style::default().fg(app.theme.fg_dim),
                    )),
                ]
            }
        }
    }
}
