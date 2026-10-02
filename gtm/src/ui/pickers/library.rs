// Copyright (c) 2026
// Author: prjctimg <prjctimg@outlook.com>
// Playlist and track selection pickers
//
//
// This is free software released under the GPL-3.0 license.

use crate::ui::*;

impl Pickers {
    /// The library categories, moved out of the left pane (Alt+.). Up/Down move
    /// the cursor, typing searches the list, Enter switches to the highlighted
    /// category and closes.
    pub(crate) fn render_libraries(f: &mut ratatui::Frame, area: Rect, app: &mut App) {
        let block = Self::picker_panel(
            app,
            " Library ",
            Some("\u{2191}/\u{2193}: move   type: search   Enter: open   Esc: cancel"),
        );
        let inner = block.inner(area);
        f.render_widget(block, area);

        let cats = app.filtered_library_indices();
        let total = cats.len();
        let sel = app
            .pickers
            .top()
            .map_or(0, |o| o.selected.min(total.saturating_sub(1)));

        let cursor_style = cursor_span_style(app);
        let query = app.pickers.top().map_or(String::new(), |o| o.query.clone());
        let search_line = Line::from(vec![
            Span::styled(" > ", Style::default().fg(app.theme.fg_dim)),
            Span::styled(query.as_str(), Style::default().fg(app.theme.fg)),
            Span::styled(" ", cursor_style.unwrap_or_default()),
        ]);

        if total == 0 {
            let mut lines = vec![search_line];
            lines.extend(empty_hint_lines(
                app,
                "No list matches",
                "Hint: type to search your lists",
            ));
            f.render_widget(Paragraph::new(lines), inner);
            return;
        }

        // The search row is painted into its own one-line rect and the rows are
        // offset below it, rather than the whole thing being one Paragraph:
        // mouse zones need the absolute row rect, which a single widget does
        // not hand back.
        f.render_widget(
            Paragraph::new(search_line),
            Rect {
                x: inner.x,
                y: inner.y,
                width: inner.width,
                height: 1,
            },
        );

        let visible = inner.height.saturating_sub(1) as usize;
        let (scroll_start, scroll_end) = match app.pickers.top_mut() {
            Some(top) => {
                let (s, e) = step_viewport(top.viewport_offset, sel, visible, total);
                top.viewport_offset = s;
                (s, e)
            }
            None => (0, total),
        };

        let icons = if use_nerd_fonts() {
            LIBRARY_ICONS_NERD
        } else {
            LIBRARY_ICONS_ASCII
        };
        let row_w = inner.width;
        for (row, &i) in cats[scroll_start..scroll_end].iter().enumerate() {
            let cat = LIBRARY_CATEGORIES[i];
            let count = app.library_count(cat);
            let is_sel = scroll_start + row == sel;
            // The picker cursor and the category currently on air are two
            // different facts and were drawn as one: the highlight keyed off
            // `library_category`, so the cursor had no marker of its own and
            // nothing on screen moved when it did.
            let label = if count > 0 {
                format!(
                    "{} {}  {:<14} {:>4}",
                    if is_sel { ">" } else { " " },
                    icons.get(i).copied().unwrap_or(" "),
                    cat,
                    count
                )
            } else {
                format!(
                    "{} {}  {}",
                    if is_sel { ">" } else { " " },
                    icons.get(i).copied().unwrap_or(" "),
                    cat
                )
            };
            let style = if is_sel {
                Style::default()
                    .fg(app.theme.selection_fg_readable())
                    .bg(app.theme.selection_bg)
            } else if i == app.library_category {
                Style::default().fg(app.theme.accent)
            } else {
                Style::default().fg(app.theme.fg)
            };
            let pad = if is_sel { row_pad(&label, row_w) } else { 0 };
            let line_idx = 1 + row as u16;
            f.render_widget(
                Paragraph::new(Line::from(Span::styled(
                    format!("{label}{}", " ".repeat(pad)),
                    style,
                ))),
                Rect {
                    x: inner.x,
                    y: inner.y + line_idx,
                    width: inner.width,
                    height: 1,
                },
            );
            app.mouse_map.register(
                Rect {
                    x: inner.x,
                    y: inner.y + line_idx,
                    width: inner.width,
                    height: 1,
                },
                MouseZone::PickerItem(scroll_start + row),
            );
        }
    }

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
            let label = track.display_title();
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

impl Pickers {
    /// Everything the daemon knows about the track on air, as `label: value`
    /// rows (`i`).
    ///
    /// A track list shows the name and the artist and stops there, but the
    /// daemon has a great deal more: the file behind a provider URI, the
    /// format it decoded to, where the artwork came from, and whether the
    /// entry is a favourite. The panes that show any of it are the ones that
    /// get dismissed, so the one place to read it all did not exist.
    ///
    /// Read-only and non-navigable: there is nothing to select here, so
    /// `picker_item_count` is zero and the arrow keys do nothing rather than
    /// moving a cursor over a form.
    pub(crate) fn render_track_info(f: &mut ratatui::Frame, area: Rect, app: &App) {
        let block = Self::picker_panel(app, " Track Info ", Some("Esc: close"));
        let inner = block.inner(area);
        f.render_widget(block, area);

        let Some(track) = app.state.current_track.clone() else {
            let msg = Paragraph::new("Nothing is playing")
                .alignment(Alignment::Center)
                .style(Style::default().fg(app.theme.fg_dim));
            f.render_widget(msg, inner);
            return;
        };

        let provider = classify_remote_source(&track.path).map_or("Local", |(key, _)| key);
        let mut rows: Vec<(&str, String)> = Vec::new();
        let mut push = |label: &'static str, value: String| {
            // An empty value is a field the daemon does not have, not a field
            // with an empty value; showing "Genre:" with nothing after it
            // reads as a bug rather than as absent metadata.
            if !value.trim().is_empty() {
                rows.push((label, value));
            }
        };

        push("Title", track.title.clone());
        push("Artist", track.artist.clone());
        push("Album", track.album.clone());
        if let Some(n) = track.track_number {
            push("Track", n.to_string());
        }
        if let Some(y) = track.year {
            push("Year", y.to_string());
        }
        push("Genre", track.genre.clone());
        if track.duration > 0.0 {
            push("Length", format_duration(track.duration as u64));
        }
        if let Some(actual) = track.actual_duration.filter(|a| *a > 0.0) {
            push("Decoded", format_duration(actual as u64));
        }
        if let Some(b) = track.bitrate {
            push("Bitrate", format!("{b} kbps"));
        }
        if let Some(s) = track.samplerate {
            push("Sample rate", format!("{s} kHz"));
        }
        if track.favourite {
            push("Favourite", "yes".to_string());
        }
        push("Source", provider.to_string());
        push("Path", track.path.clone());
        if let Some(p) = track.cover_path.as_ref().filter(|p| !p.is_empty()) {
            push("Cover file", p.clone());
        }
        if let Some(u) = track.cover_url.as_ref().filter(|u| !u.is_empty()) {
            push("Cover URL", u.clone());
        }
        if let Some(id) = track.album_id.as_ref().filter(|i| !i.is_empty()) {
            push("Album ID", id.clone());
        }
        if !track.hash.is_empty() {
            push("Hash", track.hash.clone());
        }

        // Long values (paths, URLs, hashes) are the reason this pane exists, so
        // they are wrapped across the panel rather than truncated at the edge.
        let value_w = inner.width.saturating_sub(16) as usize;
        let label_style = Style::default().fg(app.theme.fg_dim);
        let mut lines: Vec<Line> = Vec::new();
        for (label, value) in rows {
            let wrapped = wrap_text(&value, value_w.max(8));
            for (i, chunk) in wrapped.iter().enumerate() {
                let l = if i == 0 { label } else { "" };
                lines.push(Line::from(vec![
                    Span::styled(format!("{l:>14}  "), label_style),
                    Span::styled(chunk.clone(), Style::default().fg(app.theme.fg_bright)),
                ]));
            }
        }
        f.render_widget(Paragraph::new(lines), inner);
    }
}
