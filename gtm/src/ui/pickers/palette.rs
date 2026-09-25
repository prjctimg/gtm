// Copyright (c) 2026
// Author: prjctimg <prjctimg@outlook.com>
// Command palette picker
//
//
// This is free software released under the GPL-3.0 license.

use crate::ui::*;

impl Pickers {
    pub(crate) fn command_palette(f: &mut ratatui::Frame, area: Rect, app: &mut App) {
        let commands = CommandPalette::commands(&app.icon_style);

        let query = app.pickers.top().map_or(String::new(), |o| o.query.clone());
        let filtered: Vec<usize> = commands
            .iter()
            .enumerate()
            .filter_map(|(i, c)| fuzzy_match(&query, c.icon).then_some(i))
            .collect();

        let block = Self::picker_panel(app, " Commands ", None);
        let inner = block.inner(area);
        f.render_widget(block, area);

        let cursor_style = cursor_span_style(app);
        let search_line = Line::from(vec![
            Span::styled(" > ", Style::default().fg(app.theme.fg_dim)),
            Span::styled(query.as_str(), Style::default().fg(app.theme.fg)),
            Span::styled(" ", cursor_style.unwrap_or_default()),
        ]);

        let show_groups = query.is_empty();
        let mut rows: Vec<(Option<&'static str>, Option<usize>)> = Vec::new();
        if show_groups {
            let mut acc = 0usize;
            for (gname, gcount) in COMMAND_GROUPS {
                if acc >= commands.len() {
                    break;
                }
                rows.push((Some(gname), None));
                let end = (acc + gcount).min(commands.len());
                while acc < end {
                    rows.push((None, Some(acc)));
                    acc += 1;
                }
            }
        } else {
            for ci in &filtered {
                rows.push((None, Some(*ci)));
            }
        }

        let sel = app
            .pickers
            .top()
            .map_or(0, |o| o.selected.min(filtered.len().saturating_sub(1)));
        // The viewport + highlight are keyed off the row this selection lives
        // at. When grouped (empty query) the selected field holds a command
        // index that we must map through the header rows; when filtering, the
        // rows are 1:1 with `filtered`, so the selected field is already a row
        // offset. Getting this wrong makes up/down appear to do nothing while
        // search input is present.
        let sel_display = if show_groups {
            rows.iter().position(|(_, c)| *c == Some(sel))
        } else {
            Some(sel).filter(|&s| s < rows.len())
        };
        let total = rows.len();
        let visible = inner.height.saturating_sub(1) as usize;
        let (scroll_start, scroll_end) = if total > 0 {
            match (app.pickers.top_mut(), sel_display) {
                (Some(top), Some(sd)) => {
                    let (s, e) = step_viewport(top.viewport_offset, sd, visible, total);
                    top.viewport_offset = s;
                    (s, e)
                }
                _ => (0, total),
            }
        } else {
            (0, 0)
        };

        let row_w = inner.width;
        let mut lines: Vec<Line> = vec![search_line];
        // Tracks the rendered line offset (past the search row) so mouse zones
        // stay aligned even though group headings occupy multiple lines.
        let mut row_line: u16 = 1;
        for (i, (header, cmd)) in rows.iter().enumerate().take(scroll_end).skip(scroll_start) {
            if let Some(gname) = header {
                lines.push(Line::from(""));
                lines.push(Line::from(Span::styled(
                    format!("  {}", gname),
                    Style::default()
                        .fg(app.theme.accent)
                        .add_modifier(Modifier::BOLD),
                )));
                lines.push(Line::from(""));
                row_line += 3;
                continue;
            }
            let ci = cmd.unwrap_or(0);
            let (name, key) = (&commands[ci].icon, commands[ci].keys);
            let is_sel = Some(i) == sel_display;
            let full = format!(
                " {prefix}{name}  [{key}]",
                prefix = if is_sel { " > " } else { "   " }
            );
            let pad = row_pad(&full, row_w);
            let style = if is_sel {
                Style::default()
                    .fg(app.theme.selection_fg_readable())
                    .bg(app.theme.selection_bg)
            } else {
                Style::default().fg(app.theme.fg)
            };
            let key_style = if is_sel {
                style
            } else {
                Style::default().fg(app.theme.fg_dim)
            };
            lines.push(Line::from(vec![
                Span::styled(format!("{full}{}", " ".repeat(pad)), style),
                Span::styled(" ", key_style),
            ]));
            if let Some(ci) = cmd {
                let row_rect = Rect {
                    x: inner.x,
                    y: inner.y + row_line,
                    width: inner.width,
                    height: 1,
                };
                app.mouse_map.register(row_rect, MouseZone::PickerItem(*ci));
            }
            row_line += 1;
        }

        let para = Paragraph::new(lines);
        f.render_widget(para, inner);
    }
}
