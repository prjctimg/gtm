// Copyright (c) 2026
// Author: prjctimg <prjctimg@outlook.com>
// Radio browser picker
//
//
// This is free software released under the GPL-3.0 license.

use crate::ui::*;

impl Pickers {
    pub(crate) fn render_radio(f: &mut ratatui::Frame, area: Rect, app: &mut App) {
        let picks = app.radio_picks();
        let total = picks.len();
        let sel = app
            .pickers
            .top()
            .map_or(0, |o| o.selected.min(total.saturating_sub(1)));

        let (title, pending, empty_msg) = match app.radio.section {
            RadioSection::Root => (
                " Radio ".to_string(),
                app.radio.top_pending || app.radio.browse_pending,
                "no stations yet \u{2014} press r to refresh",
            ),
            RadioSection::Stations => (
                format!(" Radio: {} ", app.radio.browse_topic),
                app.radio.browse_stations_pending,
                "no stations",
            ),
            RadioSection::Results => (
                " Radio search ".to_string(),
                app.radio.search_pending,
                "type a query, then Enter",
            ),
        };
        // The trailing Esc token is auto-lifted to the panel's top-right
        // corner by picker_panel.
        let hint = format!(
            "Tab: filter {}   Enter: play   s: save   x: remove   r: refresh   Esc: close",
            app.radio.filter.label()
        );

        let block = Self::picker_panel(app, title, Some(&hint));
        let inner = block.inner(area);
        f.render_widget(block, area);

        let mut prepend = vec![Self::picker_query_line(app)];
        if pending {
            prepend.push(Line::from(Span::styled(
                " loading\u{2026}",
                Style::default().fg(app.theme.fg_dim),
            )));
        }

        let visible = inner.height.saturating_sub(prepend.len() as u16).max(1) as usize;
        let (s, e) = if total > 0 {
            if let Some(top) = app.pickers.top_mut() {
                let (a, b) = step_viewport(top.viewport_offset, sel, visible, total);
                top.viewport_offset = a;
                (a, b)
            } else {
                (0, total)
            }
        } else {
            (0, 0)
        };

        let station_at = |i: usize| -> Option<&RadioStation> {
            match app.radio.section {
                RadioSection::Stations => app.radio.browse_stations.get(i),
                RadioSection::Results => app.radio.search.get(i),
                RadioSection::Root => app.radio.top.get(i),
            }
        };

        let mut lines = prepend;
        if total == 0 {
            lines.push(Line::from(Span::styled(
                empty_msg.to_string(),
                Style::default().fg(app.theme.fg_dim),
            )));
        }
        for (k, pick) in picks[s..e].iter().enumerate() {
            let i = s + k;
            let (text, is_header) = match pick {
                RadioPick::Header(label) => (
                    format!(" \u{2500}\u{2500} {} \u{2500}\u{2500}", label),
                    true,
                ),
                RadioPick::Custom(idx) => app
                    .radio
                    .custom
                    .get(*idx)
                    .map(|s| {
                        (
                            format!("{} {}\u{2003}\u{2714} saved", "\u{1f3a7}", s.name),
                            false,
                        )
                    })
                    .unwrap_or_default(),
                RadioPick::Station(idx) => station_at(*idx)
                    .map(|s| (Self::radio_row(s), false))
                    .unwrap_or_default(),
                RadioPick::Tag(idx) => app
                    .radio
                    .browse_tags
                    .get(*idx)
                    .map(|t| {
                        (
                            format!(
                                "\u{1f3f7}\u{fe0f} {}\u{2003}\u{2139}\u{fe0f} {} stations",
                                t.name, t.station_count
                            ),
                            false,
                        )
                    })
                    .unwrap_or_default(),
                RadioPick::Country(idx) => app
                    .radio
                    .browse_countries
                    .get(*idx)
                    .map(|c| {
                        (
                            format!(
                                "\u{1f30d} {}\u{2003}\u{2139}\u{fe0f} {} stations",
                                c.name, c.station_count
                            ),
                            false,
                        )
                    })
                    .unwrap_or_default(),
            };
            let style = if is_header {
                Style::default().fg(app.theme.muted_border)
            } else if i == sel {
                Style::default()
                    .fg(app.theme.selection_fg_readable())
                    .bg(app.theme.selection_bg)
            } else {
                Style::default().fg(app.theme.fg)
            };
            let prefix = if is_header {
                "  "
            } else if i == sel {
                " > "
            } else {
                "   "
            };
            let row = if i == sel && !is_header {
                format!("{prefix}{text}{}", " ".repeat(row_pad(&text, inner.width)))
            } else {
                format!("{prefix}{text}")
            };
            lines.push(Line::from(Span::styled(row, style)));
            let row_rect = Rect {
                x: inner.x,
                y: inner.y + k as u16,
                width: inner.width,
                height: 1,
            };
            app.mouse_map.register(row_rect, MouseZone::PickerItem(i));
        }
        f.render_widget(Paragraph::new(lines), inner);
    }

    pub(crate) fn radio_row(s: &RadioStation) -> String {
        let mut row = format!("\u{1f3a7} {}\u{2003}", s.name);
        if !s.country.is_empty() {
            row.push_str(&format!(" \u{1f30d}{} ", s.country));
        }
        if !s.language.is_empty() {
            row.push_str(&format!("\u{1f3ac} {} ", s.language));
        }
        if !s.codec.is_empty() {
            row.push_str(&format!(" {} ", s.codec));
        }
        row.push_str(&format!(" \u{2b50} {}", s.votes));
        if !s.favicon.is_empty() {
            row.push_str(" \u{1f310}");
        }
        row
    }
}
