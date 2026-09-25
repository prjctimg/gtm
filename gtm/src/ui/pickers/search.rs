// Copyright (c) 2026
// Author: prjctimg <prjctimg@outlook.com>
// YouTube and library search pickers
//
//
// This is free software released under the GPL-3.0 license.

use crate::ui::*;

impl Pickers {
    pub(crate) fn render_yt_search(f: &mut ratatui::Frame, area: Rect, app: &mut App) {
        let block = Self::picker_panel(
            app,
            " \u{f16a} Search ",
            Some(" Enter: play   Ctrl+D: download   Ctrl+A: queue   Esc: close"),
        );
        let inner = block.inner(area);
        f.render_widget(block, area);

        let query = app.pickers.top().map_or(String::new(), |o| o.query.clone());
        let cursor = if app
            .pickers
            .top()
            .is_some_and(|o| o.id == PickerId::YTSearch)
        {
            cursor_span_style(app)
        } else {
            None
        };
        let search_line = Line::from(vec![
            Span::styled(" > ", Style::default().fg(app.theme.fg)),
            Span::styled(query.clone(), Style::default().fg(app.theme.fg)),
            match cursor {
                Some(style) => Span::styled(" ", style),
                None => Span::raw(""),
            },
        ]);

        let sel = app.pickers.top().map_or(0, |o| o.selected);
        let total = app.yt_results_cache.len();
        let visible = inner.height.saturating_sub(1) as usize; // reserve 1 line for search
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

        let mut lines: Vec<Line> = vec![search_line];
        // Hint line for yt-dlp host searches. The host table is data-driven
        // (see shared::yt): defaults plus `GTM_YT_HOSTS` overrides, so any
        // configured provider's prefix gets a hint here.
        if let Some(host) = app
            .pickers
            .top()
            .map(|o| o.query.clone())
            .as_deref()
            .and_then(|q| {
                crate::shared::yt::match_yt_host(q, &crate::shared::yt::yt_hosts())
                    .map(|(_, h)| h.name.clone())
            })
        {
            lines.push(Line::from(Span::styled(
                format!(" {host} search (yt-dlp)"),
                Style::default().fg(app.theme.fg_dim),
            )));
        }
        if app.yt_results_cache.is_empty() && app.yt_search_loading {
            let lines_len = lines.len();
            f.render_widget(Paragraph::new(lines), inner);
            let loader_area = Rect {
                x: inner.x,
                y: inner.y + lines_len as u16,
                width: inner.width,
                height: inner.height.saturating_sub(lines_len as u16),
            };
            Render::loader(f, loader_area, app, "Searching…");
            return;
        }
        for i in scroll_start..scroll_end {
            let r = &app.yt_results_cache[i];
            let dur = format_duration(r.duration as u64);
            let icon = if r.is_playlist {
                "\u{f01db} "
            } else {
                "\u{f008} "
            };
            let prefix = if i == sel { " > " } else { "   " };
            let display = match r.artist.as_deref() {
                Some(a) => format!("{a} - {}", r.title),
                None => r.title.clone(),
            };
            let mut content = format!("{prefix}{}{} [{}]", icon, display, dur);
            // Inline download status: a download started for this row shows
            // on the row itself (the finished event is the only toast).
            if app.downloading_urls.contains(&r.url) {
                let dl = app
                    .downloads
                    .values()
                    .find(|d| d.url == r.url)
                    .map(|d| d.percent.clamp(0.0, 100.0) as u64)
                    .unwrap_or(0);
                content.push_str(&format!(" → ⤓ {dl}%"));
            }
            let style = if i == sel {
                Style::default()
                    .fg(app.theme.selection_fg_readable())
                    .bg(app.theme.selection_bg)
            } else {
                Style::default()
            };
            let row = if i == sel {
                format!("{content}{}", " ".repeat(row_pad(&content, inner.width)))
            } else {
                content
            };
            lines.push(Line::from(Span::styled(row, style)));
            let row_rect = Rect {
                x: inner.x,
                y: inner.y + 1 + (i - scroll_start) as u16,
                width: inner.width,
                height: 1,
            };
            app.mouse_map.register(row_rect, MouseZone::PickerItem(i));
        }

        let para = Paragraph::new(lines);
        f.render_widget(para, inner);
    }

    pub(crate) fn render_search_library(f: &mut ratatui::Frame, area: Rect, app: &mut App) {
        let source = app.pickers.top().map_or(PickerSource::All, |o| o.source);
        let title = format!(" Search: {} ", source.label());
        let block = Self::picker_panel(app, &title, None);
        let inner = block.inner(area);
        f.render_widget(block, area);

        let picks = app.search_library_picks();

        let cursor_style = if app
            .pickers
            .top()
            .is_some_and(|o| o.id == PickerId::SearchLibrary)
        {
            cursor_span_style(app)
        } else {
            None
        };
        let search_line = Line::from(vec![
            Span::styled(" > ", Style::default().fg(app.theme.fg)),
            Span::styled(
                app.pickers.top().map_or(String::new(), |o| o.query.clone()),
                Style::default().fg(app.theme.fg),
            ),
            match cursor_style {
                Some(style) => Span::styled(" ", style),
                None => Span::raw(""),
            },
        ]);

        let preview_h: u16 = 7;
        let results_area = Rect {
            x: inner.x,
            y: inner.y,
            width: inner.width,
            height: inner.height.saturating_sub(preview_h),
        };

        let sel = app
            .pickers
            .top()
            .map_or(0, |o| o.selected.min(picks.len().saturating_sub(1)));
        let total = picks.len();
        let visible = results_area.height.saturating_sub(1) as usize;
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

        let mut lines: Vec<Line> = vec![search_line];
        for (i, pick) in picks.iter().enumerate().take(scroll_end).skip(scroll_start) {
            let prefix = if i == sel { " > " } else { "   " };
            let style = if i == sel {
                Style::default()
                    .fg(app.theme.selection_fg_readable())
                    .bg(app.theme.selection_bg)
            } else {
                Style::default()
            };
            let text = match pick {
                LibraryPick::Track(idx) => {
                    let t = &app.tracks_cache[*idx];
                    let artist = if t.artist.is_empty() {
                        String::new()
                    } else {
                        format!("{} - ", t.artist)
                    };
                    format!("{}\u{266b} {}{}", prefix, artist, t.title,)
                }
                LibraryPick::Artist(name) => format!("{}\u{1f465} {}", prefix, name),
                LibraryPick::Album(album) => format!("{}\u{1f4bf} {}", prefix, album),
                LibraryPick::Playlist(i) => match app.playlist_cache.get(*i) {
                    Some(p) if !p.name.is_empty() => {
                        format!("{}\u{1f4dc} {}", prefix, p.name)
                    }
                    _ => format!("{}\u{1f4dc} (missing playlist)", prefix),
                },
                LibraryPick::Radio(i) => {
                    let station = app
                        .radio
                        .custom
                        .get(*i)
                        .map(|s| s.name.as_str())
                        .unwrap_or("");
                    format!("{}\u{1f3a7} {}", prefix, station)
                }
            };
            let row = if i == sel {
                format!("{text}{}", " ".repeat(row_pad(&text, results_area.width)))
            } else {
                text
            };
            lines.push(Line::from(Span::styled(row, style)));
            let row_rect = Rect {
                x: results_area.x,
                y: results_area.y + 1 + (i - scroll_start) as u16,
                width: results_area.width,
                height: 1,
            };
            app.mouse_map.register(row_rect, MouseZone::PickerItem(i));
        }

        let para = Paragraph::new(lines);
        f.render_widget(para, results_area);

        if preview_h > 0 {
            let preview_area = Rect {
                x: inner.x,
                y: inner.y + inner.height - preview_h,
                width: inner.width,
                height: preview_h,
            };
            Self::render_search_preview(f, preview_area, app, &picks, sel);
        }
    }

    pub(crate) fn render_search_preview(
        f: &mut ratatui::Frame,
        area: Rect,
        app: &mut App,
        picks: &[LibraryPick],
        sel: usize,
    ) {
        let rule = Line::from(Span::styled(
            "\u{2500}".repeat(area.width as usize),
            Style::default().fg(app.theme.muted_border),
        ));
        f.render_widget(Paragraph::new(rule), area);

        let body = Rect {
            x: area.x,
            y: area.y.saturating_add(1),
            width: area.width,
            height: area.height.saturating_sub(1),
        };
        let cover_w = 20u16.min(body.width.saturating_sub(24).max(8));
        let hchunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Length(cover_w), Constraint::Min(0)])
            .split(body);

        let cover_area = Rect {
            x: hchunks[0].x + 1,
            y: hchunks[0].y,
            width: hchunks[0].width.saturating_sub(1),
            height: hchunks[0].height,
        };
        let is_artist = picks
            .get(sel)
            .is_some_and(|p| matches!(p, LibraryPick::Artist(_)));
        if is_artist {
            Render::cover(
                f,
                cover_area,
                app.artist_cover_stateful.as_mut(),
                None,
                app.theme.fg_dim,
                Some("\u{1f465}"),
            );
        } else {
            Render::cover(
                f,
                cover_area,
                app.picker_preview_stateful.as_mut(),
                app.picker_preview_cover.as_deref(),
                app.theme.fg_dim,
                Some("\u{266b}"),
            );
        }

        let meta_area = hchunks[1];
        let mut meta_lines = Vec::new();
        let mut push = |key: &str, value: &str| {
            meta_lines.push(Line::from(vec![
                Span::styled(format!("{key:>9} "), Style::default().fg(app.theme.fg_dim)),
                Span::styled(value.to_string(), Style::default().fg(app.theme.fg_bright)),
            ]));
        };
        match picks.get(sel) {
            Some(LibraryPick::Track(i)) => {
                let t = &app.tracks_cache[*i];
                let display_title = if t.title.is_empty() {
                    std::path::Path::new(&t.path)
                        .file_stem()
                        .map(|s| s.to_string_lossy().to_string())
                        .unwrap_or_default()
                } else {
                    t.title.clone()
                };
                push("Title", &display_title);
                push(
                    "Artist",
                    if t.artist.is_empty() {
                        "Unknown"
                    } else {
                        &t.artist
                    },
                );
                push(
                    "Album",
                    if t.album.is_empty() {
                        "Unknown"
                    } else {
                        &t.album
                    },
                );
                push("Length", &format_duration(t.duration as u64));
            }
            Some(LibraryPick::Artist(name)) => {
                let count = app
                    .tracks_cache
                    .iter()
                    .filter(|t| t.artist.eq_ignore_ascii_case(name))
                    .count();
                push("Artist", name);
                push("Tracks", &count.to_string());
            }
            Some(LibraryPick::Album(album)) => {
                let count = app
                    .tracks_cache
                    .iter()
                    .filter(|t| t.album.eq_ignore_ascii_case(album))
                    .count();
                push("Album", album);
                push("Tracks", &count.to_string());
            }
            Some(LibraryPick::Playlist(i)) => {
                if let Some(p) = app.playlist_cache.get(*i) {
                    push("Playlist", &p.name);
                    push("Tracks", &p.track_count.to_string());
                }
            }
            Some(LibraryPick::Radio(i)) => {
                if let Some(s) = app.radio.custom.get(*i) {
                    push("Station", &s.name);
                    push("URL", &s.url);
                }
            }
            None => {
                push("", "No results");
            }
        }
        f.render_widget(Paragraph::new(meta_lines), meta_area);
    }
}
