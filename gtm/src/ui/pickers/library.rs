// Copyright (c) 2026
// Author: prjctimg <prjctimg@outlook.com>
// Playlist and track selection pickers
//
//
// This is free software released under the GPL-3.0 license.

use crate::ui::*;

impl Pickers {
    pub(crate) fn render_playlist_select(f: &mut ratatui::Frame, area: Rect, app: &App) {
        let help = if app.playlist_creating {
            None
        } else {
            Some("\u{2191}/\u{2193}: choose   n: new   Enter: add   Esc: cancel")
        };
        let block = Self::picker_panel(app, " Select Playlist ", help);
        let inner = block.inner(area);
        f.render_widget(block, area);

        if app.playlist_creating {
            let cursor_style = cursor_span_style(app);
            let para = Paragraph::new(Line::from(vec![
                Span::styled(" Name: ", Style::default().fg(app.theme.fg_dim)),
                Span::styled(
                    app.pickers.top().map_or(String::new(), |o| o.query.clone()),
                    Style::default()
                        .fg(app.theme.accent)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(" ", cursor_style.unwrap_or_default()),
            ]));
            f.render_widget(para, inner);
            return;
        }

        let total = app.playlist_cache.len() + 1;
        let sel = app
            .pickers
            .top()
            .map_or(0, |o| o.selected.min(total.saturating_sub(1)));
        let visible = inner.height as usize;
        let offset = app.pickers.top().map_or(0, |o| o.viewport_offset);
        let (scroll_start, scroll_end) = step_viewport(offset, sel, visible, total);

        let row_w = inner.width;
        let mut items: Vec<ListItem> = Vec::new();
        for i in scroll_start..scroll_end {
            let is_sel = i == sel;
            let style = if is_sel {
                Style::default()
                    .fg(app.theme.selection_fg_readable())
                    .bg(app.theme.selection_bg)
            } else if i == 0 {
                Style::default().fg(app.theme.accent)
            } else {
                Style::default().fg(app.theme.fg)
            };
            let content = if i == 0 {
                "  + Create New Playlist".to_string()
            } else {
                match app.playlist_cache.get(i - 1) {
                    Some(pl) => format!(
                        "{}{} ({} {})",
                        if is_sel { " > " } else { "   " },
                        pl.name,
                        pl.track_count,
                        plural(pl.track_count as usize, "track", "tracks")
                    ),
                    None => continue,
                }
            };
            let content = if is_sel {
                let pad = row_pad(&content, row_w);
                format!("{content}{}", " ".repeat(pad))
            } else {
                content
            };
            items.push(ListItem::new(content).style(style));
        }

        let list = List::new(items);
        f.render_widget(list, inner);
    }

    pub(crate) fn render_track_select(f: &mut ratatui::Frame, area: Rect, app: &mut App) {
        let selected = app.selected_track_ids.len();
        let hint = format!(
            "Space/Tab: toggle   \u{2191}/\u{2193}: navigate   Ctrl+Enter: add {} to playlist   Esc: cancel",
            if selected > 0 {
                format!("({selected} selected)")
            } else {
                String::new()
            }
        );
        let block = Self::picker_panel(app, " Add Tracks ", Some(&hint));
        let inner = block.inner(area);
        f.render_widget(block, area);

        let tracks = &app.tracks_cache;
        let total = tracks.len();
        if total == 0 {
            let p =
                Paragraph::new("No tracks in library").style(Style::default().fg(app.theme.fg_dim));
            f.render_widget(p, inner);
            return;
        }

        let sel = app
            .pickers
            .top()
            .map_or(0, |o| o.selected.min(total.saturating_sub(1)));
        let visible = inner.height.saturating_sub(2) as usize;
        let (scroll_start, scroll_end) = if let Some(top) = app.pickers.top_mut() {
            let (s, e) = step_viewport(top.viewport_offset, sel, visible, total);
            top.viewport_offset = s;
            (s, e)
        } else {
            (0, total)
        };

        let row_w = inner.width;
        let mut items: Vec<ListItem> = Vec::new();
        for i in scroll_start..scroll_end {
            let Some(track) = tracks.get(i) else { continue };
            let is_sel = i == sel;
            let is_picked = app.selected_track_ids.contains(&track.id);
            let mark = if is_picked { " \u{2713} " } else { "   " };
            let label = if track.title.is_empty() {
                std::path::Path::new(&track.path)
                    .file_stem()
                    .map(|s| s.to_string_lossy().to_string())
                    .unwrap_or_default()
            } else {
                track.title.clone()
            };
            let artist = if track.artist.is_empty() {
                String::new()
            } else {
                format!(" - {}", track.artist)
            };
            let dur = format_duration_short(track.duration as u64);
            let content = format!("{mark}{label}{artist} [{}]", dur);
            let style = if is_picked {
                Style::default()
                    .fg(app.theme.accent)
                    .add_modifier(Modifier::BOLD)
            } else if is_sel {
                Style::default()
                    .fg(app.theme.selection_fg_readable())
                    .bg(app.theme.selection_bg)
            } else {
                Style::default().fg(app.theme.fg)
            };
            let pad = if is_sel { row_pad(&content, row_w) } else { 0 };
            let content = format!("{content}{}", " ".repeat(pad));
            let row_rect = Rect {
                x: inner.x,
                y: inner.y + 1 + (i - scroll_start) as u16,
                width: inner.width,
                height: 1,
            };
            app.mouse_map.register(row_rect, MouseZone::PickerItem(i));
            items.push(ListItem::new(content).style(style));
        }

        let list = List::new(items);
        f.render_widget(list, inner);
    }

    pub(crate) fn render_edit_metadata(f: &mut ratatui::Frame, area: Rect, app: &mut App) {
        let block = Self::picker_panel(
            app,
            " Edit Metadata ",
            Some(
                "Tab/\u{2191}/\u{2193}: field   Enter: next/save   Ctrl+S: sync cover   Esc: cancel",
            ),
        );
        let inner = block.inner(area);
        f.render_widget(block, area);

        let field_names = [
            "Title",
            "Artist",
            "Album",
            "Album Artist",
            "Genre",
            "Year",
            "Track #",
        ];

        const COVER_W_EDIT: u16 = 24;
        let vchunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(0)])
            .split(inner);
        let content = vchunks[0];
        let cover_col_w = if content.width > COVER_W_EDIT + 2 {
            COVER_W_EDIT
        } else {
            0
        };
        let hchunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Min(0), Constraint::Length(cover_col_w)])
            .split(content);
        let list_area = hchunks[0];
        let cover_area = hchunks[1];

        let mut lines: Vec<Line> = Vec::new();
        let cursor_style = cursor_span_style(app);
        for (i, name) in field_names.iter().enumerate() {
            let value = app.metadata.fields.get(i).map(|s| s.as_str()).unwrap_or("");
            let is_active = i == app.metadata.field_idx;
            let prefix = if is_active { " > " } else { "   " };
            let style = if is_active {
                Style::default()
                    .fg(app.theme.selection_fg_readable())
                    .bg(app.theme.selection_bg)
            } else {
                Style::default().fg(app.theme.fg)
            };
            let cursor_span = if is_active {
                Span::styled(" ", cursor_style.unwrap_or_default())
            } else {
                Span::raw(" ")
            };
            lines.push(Line::from(vec![
                Span::styled(format!("{}{}: ", prefix, name), style),
                Span::styled(value.to_string(), style),
                cursor_span,
            ]));
        }

        let para = Paragraph::new(lines);
        f.render_widget(para, list_area);

        if cover_area.width > 0 {
            let cover_h = 12u16.min(cover_area.height);
            let c_area = Rect {
                x: cover_area.x,
                y: cover_area.y + cover_area.height.saturating_sub(cover_h),
                width: cover_area.width,
                height: cover_h,
            };
            Render::cover(
                f,
                c_area,
                app.metadata.cover_stateful.as_mut(),
                app.metadata.cover.as_deref(),
                app.theme.fg_dim,
                Some(" \u{266b} no cover "),
            );
        }
    }
}
