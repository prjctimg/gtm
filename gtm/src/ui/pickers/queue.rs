// Copyright (c) 2026
// Author: prjctimg <prjctimg@outlook.com>
// Queue picker and up-next preview
//
//
// This is free software released under the GPL-3.0 license.

use crate::ui::*;

impl Pickers {
    pub(crate) fn render_queue(f: &mut ratatui::Frame, area: Rect, app: &mut App) {
        let sel = app.pickers.top().map_or(0, |o| o.selected);

        let (title, hint) = if app.queue.move_index.is_some() {
            let from = app.queue.move_index.unwrap_or(0);
            let to = app.queue.move_target;
            (
                format!(" Queue (MOVE MODE: {} -> {}) ", from + 1, to + 1),
                Some(" Enter: confirm   Esc: cancel   Ctrl+K/Ctrl+J: adjust position"),
            )
        } else {
            (
                " Queue ".to_string(),
                Some(" Enter: play   Ctrl+K/Ctrl+J: move   Ctrl+D: remove   Esc: close"),
            )
        };

        let block = Self::picker_panel(app, title, hint);
        let inner = block.inner(area);
        f.render_widget(block, area);

        let total = app.queue.cache.len();
        if total == 0 {
            let p = Paragraph::new("Queue is empty").style(Style::default().fg(app.theme.fg_dim));
            f.render_widget(p, inner);
            return;
        }

        let preview_h: u16 = 7;
        let list_area = Rect {
            x: inner.x,
            y: inner.y,
            width: inner.width,
            height: inner.height.saturating_sub(preview_h),
        };

        let visible = list_area.height as usize;
        let (scroll_start, scroll_end) = if let Some(top) = app.pickers.top_mut() {
            let (s, e) = step_viewport(top.viewport_offset, sel, visible, total);
            top.viewport_offset = s;
            (s, e)
        } else {
            (0, total)
        };

        let mut lines = Vec::new();

        for i in scroll_start..scroll_end {
            let track = &app.queue.cache[i];
            let is_current = i == app.queue.cursor;
            let is_sel = i == sel;
            let prefix = if is_sel { " > " } else { "   " };
            let icon = if is_current { "\u{25b6} " } else { "\u{266b} " };
            let label = if track.title.is_empty() {
                std::path::Path::new(&track.path)
                    .file_stem()
                    .map(|s| s.to_string_lossy().to_string())
                    .unwrap_or_else(|| track.path.clone())
            } else {
                track.title.clone()
            };
            let artist = if track.artist.is_empty() {
                String::new()
            } else {
                format!(" - {}", track.artist)
            };
            let dur = format_duration_short(track.duration as u64);
            let row = format!("{prefix}{icon}{label}{artist} [{}]", dur);

            let row = if is_sel {
                format!("{row}{}", " ".repeat(row_pad(&row, inner.width)))
            } else {
                row
            };
            let style = if is_sel {
                Style::default()
                    .fg(app.theme.selection_fg_readable())
                    .bg(app.theme.selection_bg)
            } else if is_current {
                Style::default()
                    .fg(app.theme.accent)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default()
            };

            lines.push(Line::from(Span::styled(row, style)));
            let row_rect = Rect {
                x: inner.x,
                y: list_area.y + (i - scroll_start) as u16,
                width: inner.width,
                height: 1,
            };
            app.mouse_map.register(row_rect, MouseZone::PickerItem(i));
        }

        let para = Paragraph::new(lines);
        f.render_widget(para, list_area);

        if preview_h > 0 {
            let preview_area = Rect {
                x: inner.x,
                y: inner.y + inner.height - preview_h,
                width: inner.width,
                height: preview_h,
            };
            Self::render_upnext_preview(f, preview_area, app, app.queue.cursor + 1);
        }
    }

    pub(crate) fn render_upnext_preview(f: &mut ratatui::Frame, area: Rect, app: &mut App, next_idx: usize) {
        app.update_upnext_cover();
        // Use the same transparent/filled background as the picker panel so the
        // "Up Next" strip never shows a mismatched solid background over the
        // rest of the (possibly transparent) queue picker.
        let section_bg = if app.transparent_pickers {
            ratatui::style::Color::Reset
        } else {
            app.float_bg()
        };
        let block = Block::default()
            .borders(Borders::TOP)
            .title(" Up Next ")
            .border_style(Style::default().fg(app.theme.accent))
            .style(Style::default().bg(section_bg));
        f.render_widget(block, area);
        let inner = Rect {
            x: area.x,
            y: area.y + 1,
            width: area.width,
            height: area.height.saturating_sub(1),
        };
        match app.queue.cache.get(next_idx) {
            Some(track) => {
                let label = if track.title.is_empty() {
                    std::path::Path::new(&track.path)
                        .file_stem()
                        .map(|s| s.to_string_lossy().to_string())
                        .unwrap_or_else(|| track.path.clone())
                } else {
                    track.title.clone()
                };
                let artist = if track.artist.is_empty() {
                    "Unknown artist".to_string()
                } else {
                    track.artist.clone()
                };
                let album = if track.album.is_empty() {
                    None
                } else {
                    Some(track.album.clone())
                };
                let cover_w = 20u16.min(inner.width.saturating_sub(24).max(8));
                let cover_h = COVER_H.min(inner.height);
                let has_cover = app.queue.preview_cover.is_some();
                if cover_w > 0 && cover_h > 0 {
                    let cover_area = Rect {
                        x: inner.x + 1,
                        y: inner.y,
                        width: cover_w,
                        height: cover_h,
                    };
                    if has_cover {
                        Render::cover(
                            f,
                            cover_area,
                            app.queue.preview_cover_stateful.as_mut(),
                            app.queue.preview_cover.as_deref(),
                            app.theme.fg_dim,
                            Some("\u{266b}"),
                        );
                    } else {
                        Render::cover(
                            f,
                            cover_area,
                            None,
                            None,
                            app.theme.fg_dim,
                            Some("\u{266b}"),
                        );
                    }
                }
                let text_area = Rect {
                    x: inner.x + 1 + cover_w + 1,
                    y: inner.y,
                    width: inner.width.saturating_sub(cover_w + 2),
                    height: inner.height,
                };
                let mut lines: Vec<Line> = vec![
                    Line::from(Span::styled(
                        format!("  {}", label),
                        Style::default()
                            .fg(app.theme.fg_bright)
                            .add_modifier(Modifier::BOLD),
                    )),
                    Line::from(Span::styled(
                        format!("  {}", artist),
                        Style::default().fg(app.theme.fg),
                    )),
                ];
                if let Some(album) = album {
                    lines.push(Line::from(Span::styled(
                        format!("  {}", album),
                        Style::default().fg(app.theme.fg_dim),
                    )));
                }
                f.render_widget(Paragraph::new(lines), text_area);
            }
            None => {
                let p = Paragraph::new("Nothing queued after this track")
                    .style(Style::default().fg(app.theme.fg_dim).bg(section_bg));
                f.render_widget(p, inner);
            }
        }
    }

    pub(crate) fn render_scroll_rows(
        f: &mut ratatui::Frame,
        area: Rect,
        app: &mut App,
        title: &str,
        hint: &str,
        prepend: Vec<Line<'static>>,
        rows: Vec<String>,
        empty_msg: &str,
    ) {
        let block = if hint.is_empty() {
            Self::picker_panel(app, title, None)
        } else {
            Self::picker_panel(app, title, Some(hint))
        };
        let inner = block.inner(area);
        f.render_widget(block, area);

        let total = rows.len();
        let sel = app
            .pickers
            .top()
            .map_or(0, |o| o.selected.min(total.saturating_sub(1)));
        let prepend_h = prepend.len() as u16;
        let visible = inner.height.saturating_sub(prepend_h).max(1) as usize;
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

        let mut lines = prepend;
        if total == 0 {
            lines.push(Line::from(Span::styled(
                empty_msg.to_string(),
                Style::default().fg(app.theme.fg_dim),
            )));
        }
        for (k, text) in rows[s..e].iter().enumerate() {
            let i = s + k;
            let prefix = if i == sel { " > " } else { "   " };
            let style = if i == sel {
                Style::default()
                    .fg(app.theme.selection_fg_readable())
                    .bg(app.theme.selection_bg)
            } else {
                Style::default()
            };
            let row = if i == sel {
                format!("{prefix}{text}{}", " ".repeat(row_pad(text, inner.width)))
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
}
