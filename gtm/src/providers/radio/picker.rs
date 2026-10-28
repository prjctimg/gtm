// Copyright (c) 2026
// Author: prjctimg <prjctimg@outlook.com>
// Radio browser picker
//
//
// This is free software released under the GPL-3.0 license.

use crate::ui::*;

/// Rows the radio picker's preview strip takes.
///
/// A station's name is one line; what it does not say is the other five. Four
/// rows leaves room for the details and the blank that separates them from the
/// list above.
const PREVIEW_H: u16 = 5;

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
                app.radio.top_pending || app.radio.cliamp_pending || app.radio.browse_pending,
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
        // corner by picker_panel. The rest has to fit the panel's inner width,
        let block = Self::picker_panel(app, title);
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
        // Rows the preview strip takes off the bottom. Reserved before the
        // viewport is stepped, so the strip is a fixed height rather than one
        // that grows with the panel.
        let preview_h: u16 = if total > 0 && !pending { PREVIEW_H } else { 0 };
        let visible = visible.saturating_sub(preview_h as usize).max(1);
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
                RadioPick::Header(label) => (format!(" {label}"), true),
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
                RadioPick::Cliamp(idx) => app
                    .radio
                    .cliamp
                    .get(*idx)
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
            let row = if is_header {
                format!("{prefix}{text}")
            } else {
                // The panel is a fixed 60 columns now, so a long station name
                // scrolls inside it instead of widening it. Only the
                // highlighted row moves; the rest truncate with an ellipsis, so
                // the list still reads as a column of names at rest.
                let avail = inner.width.saturating_sub(prefix.len() as u16) as usize;
                let body = scroll_text(&text, avail, app.footer_title_scroll, i == sel);
                let row = format!("{prefix}{body}");
                if i == sel {
                    // `scroll_text` counts characters, and the row leads with
                    // an emoji that occupies two cells, so the padded string
                    // can still be a column short of the pane.
                    format!("{row}{}", " ".repeat(row_pad(&row, inner.width)))
                } else {
                    row
                }
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
        f.render_widget(
            Paragraph::new(lines),
            Rect {
                height: inner.height.saturating_sub(preview_h),
                ..inner
            },
        );

        if preview_h > 0 {
            Self::radio_preview(f, inner, app, sel);
        }
    }

    /// The highlighted station's details, under the list.
    ///
    /// This is what the picker shows a resting row now. It used to *sound* the
    /// station instead, by borrowing the transport: which rewrote now-playing on
    /// the way in, displaced whatever the user had paused, and owed both back on
    /// every way out of the picker — of which there were about thirty, most of
    /// them closing the picker without reaching the cleanup. Nothing is borrowed
    /// now, so there is nothing to restore and no way to leave a station playing
    /// behind a picker the user thought they had closed.
    ///
    /// The layout is the queue picker's, deliberately: a strip with the same
    /// border and fill, so the two pickers read as one app.
    fn radio_preview(f: &mut ratatui::Frame, inner: Rect, app: &mut App, sel: usize) {
        let section_bg = if app.transparent_pickers {
            Color::Reset
        } else {
            app.float_bg()
        };
        let area = Rect {
            x: inner.x,
            y: inner.bottom().saturating_sub(PREVIEW_H),
            width: inner.width,
            height: PREVIEW_H,
        };
        let block = Block::default()
            .borders(Borders::TOP)
            .title(" Station ")
            .border_style(Style::default().fg(app.theme.accent))
            .style(Style::default().bg(section_bg));
        f.render_widget(block, area);
        let body = Rect {
            x: area.x,
            y: area.y + 1,
            width: area.width,
            height: area.height.saturating_sub(1),
        };
        let Some(station) = app.radio_station_at(sel) else {
            return;
        };
        // Truncated with an ellipsis rather than cut at a cell, so a long value reads
        // as "there is more of this" instead of as a complete short one. The
        // name gets the same treatment from the list above it.
        let clip = |t: &str, w: usize| -> String {
            if t.chars().count() <= w {
                return t.to_string();
            }
            format!(
                "{}…",
                t.chars().take(w.saturating_sub(1)).collect::<String>()
            )
        };
        let mut lines = vec![Line::from(Span::styled(
            format!("  {}", clip(&station.name, body.width as usize)),
            Style::default()
                .fg(app.theme.fg_bright)
                .add_modifier(Modifier::BOLD),
        ))];
        let details: Vec<(&str, String)> = vec![
            ("about", station.description.clone()),
            ("country", station.country.clone()),
            ("language", station.language.clone()),
            ("codec", station.codec.clone()),
            (
                "bitrate",
                station
                    .bitrate_kbps
                    .map(|k| format!("{k} kbps"))
                    .unwrap_or_default(),
            ),
            (
                "votes",
                if station.votes > 0 {
                    station.votes.to_string()
                } else {
                    String::new()
                },
            ),
            ("tags", station.tags.clone()),
        ];
        // The label column is two spaces plus a nine-wide field, so the value
        // gets whatever is left of the panel.
        let value_w = body.width.saturating_sub(13) as usize;
        for (label, value) in details {
            let value = value.trim();
            if value.is_empty() {
                continue;
            }
            lines.push(Line::from(vec![
                Span::styled(
                    format!("  {:>9} ", label),
                    Style::default().fg(app.theme.fg_dim),
                ),
                Span::styled(
                    format!(" {}", clip(value, value_w)),
                    Style::default().fg(app.theme.fg),
                ),
            ]));
            if lines.len() as u16 >= body.height {
                break;
            }
        }
        f.render_widget(Paragraph::new(lines), body);
    }

    pub(crate) fn radio_row(s: &RadioStation) -> String {
        let mut row = format!("\u{1f3a7} {}\u{2003}", s.name);
        if !s.description.is_empty() {
            row.push_str(&format!("{}\u{2003}", s.description));
        }
        if !s.country.is_empty() {
            row.push_str(&format!(" \u{1f30d}{} ", s.country));
        }
        if !s.language.is_empty() {
            row.push_str(&format!("\u{1f3ac} {} ", s.language));
        }
        if !s.codec.is_empty() {
            row.push_str(&format!(" {} ", s.codec));
        }
        // A zero is not a useful vote count, and directory channels such as
        // cliamp have no voting at all. Only show the badge when it means
        // something.
        if s.votes > 0 {
            row.push_str(&format!(" \u{2b50} {}", s.votes));
        }
        if !s.favicon.is_empty() {
            row.push_str(" \u{1f310}");
        }
        row
    }
}
