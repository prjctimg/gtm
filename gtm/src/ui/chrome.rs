// Copyright (c) 2026
// Author: prjctimg <prjctimg@outlook.com>
// Main TUI chrome: layout, panes and the render loop
//
//
// This is free software released under the GPL-3.0 license.

use crate::ui::*;

impl Render {
    pub(crate) fn upnext_card(f: &mut ratatui::Frame, area: Rect, app: &mut App) {
        let (display_title, artist, album, has_album, has_cover, source_label) = {
            let u = match app.upnext.as_ref() {
                Some(u) => u,
                None => return,
            };
            let display_title = if u.track.title.is_empty() {
                std::path::Path::new(&u.track.path)
                    .file_stem()
                    .map(|s| s.to_string_lossy().to_string())
                    .unwrap_or_default()
            } else {
                u.track.title.clone()
            };
            let artist = if u.track.artist.is_empty() {
                "Unknown".to_string()
            } else {
                u.track.artist.clone()
            };
            let album = u.track.album.clone();
            let has_album = !album.is_empty();
            let has_cover = u.cover.is_some();
            // Provider comes from the shared classifier so the card, the footer
            // source module and the daemon agree on what a path is.
            let source = classify_remote_source(&u.track.path).map_or("Local", |(key, _)| key);
            let source_label: String = if use_nerd_fonts() {
                match provider_icon(source) {
                    Some(g) => format!(" {g}"),
                    None => " ♪".to_string(),
                }
            } else {
                match source {
                    "Spotify" => " ♫".to_string(),
                    "YouTube" => " ▶".to_string(),
                    _ => " ♪".to_string(),
                }
            };
            (
                display_title,
                artist,
                album,
                has_album,
                has_cover,
                source_label,
            )
        };

        let bg = Block::default().style(Style::default().bg(app.float_bg()));
        f.render_widget(bg, area);

        let border_color = app.theme.notification_border;
        f.render_widget(
            Block::default().style(Style::default().bg(border_color)),
            Rect {
                x: area.x,
                y: area.y,
                width: 1,
                height: area.height,
            },
        );

        let inner = area.inner(Margin {
            horizontal: 2,
            vertical: 1,
        });

        let cover_w = COVER_W.min(inner.width.saturating_sub(2));
        let cover_h = COVER_H.min(inner.height);
        if cover_w > 0 {
            let cover_area = Rect {
                x: inner.x,
                y: inner.y,
                width: cover_w,
                height: cover_h,
            };
            if has_cover {
                if let Some(u) = app.upnext.as_mut() {
                    let (stateful, bytes): (Option<&mut StatefulProtocol>, Option<&[u8]>) =
                        if u.cover_stateful.is_some() {
                            (u.cover_stateful.as_mut(), None)
                        } else {
                            (None, u.cover.as_deref())
                        };
                    Render::cover(
                        f,
                        cover_area,
                        stateful,
                        bytes,
                        app.theme.fg_dim,
                        Some("\u{266b}"),
                    );
                }
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
            x: inner.x + cover_w + 1,
            y: inner.y,
            width: inner.width.saturating_sub(cover_w + 1),
            height: inner.height,
        };
        let text_w = text_area.width.saturating_sub(1) as usize;
        let animated = scroll_text(&display_title, text_w.max(4), app.np_title_scroll, false);
        let mut lines: Vec<Line> = vec![
            Line::from(Span::styled(
                "UP NEXT",
                Style::default()
                    .fg(app.theme.accent)
                    .add_modifier(Modifier::BOLD),
            )),
            Line::from(Span::styled(
                format!(" {}", animated),
                Style::default()
                    .fg(app.theme.fg_bright)
                    .add_modifier(Modifier::BOLD),
            )),
            Line::from(Span::styled(
                format!(" {}", artist),
                Style::default().fg(app.theme.fg),
            )),
        ];
        if has_album {
            lines.push(Line::from(Span::styled(
                format!(" {}", album),
                Style::default().fg(app.theme.fg),
            )));
        }
        lines.push(Line::from(Span::styled(
            format!(" {}", source_label.trim_start()),
            Style::default().fg(app.theme.fg_dim),
        )));
        f.render_widget(Paragraph::new(lines), text_area);
    }

    pub(crate) fn notification_overlay(f: &mut ratatui::Frame, area: Rect, app: &mut App) {
        let now = std::time::Instant::now();

        let slide_duration_ms: f32 = 300.0;
        for n in &mut app.notifications {
            if n.animation_progress < 1.0 {
                let elapsed = now
                    .duration_since(n.expires_at - NOTIFICATION_LIFETIME)
                    .as_millis() as f32;
                n.animation_progress = (elapsed / slide_duration_ms).min(1.0);
            }
        }

        app.notifications
            .retain(|n| now < n.expires_at + NOTIFICATION_EXIT_DURATION);

        let max_notif_width = 42u16;
        let padding = 1u16;
        let gap = 1u16;

        // Cards anchor to the top-center of the screen and stack downward.
        let center_x = |w: u16| area.x + area.width.saturating_sub(w) / 2;
        let mut y_top = area.y + padding;

        // The Up Next card sits at the very top; it carries cover art, so it is
        // skipped while a picker is open to keep images from leaking over it.
        if app.upnext.is_some() && !app.pickers.is_open() {
            let card_w = 42u16;
            let card_h = 7u16;
            let card_x = center_x(card_w);
            if y_top.saturating_add(card_h) <= area.bottom() {
                let card_area = Rect {
                    x: card_x,
                    y: y_top,
                    width: card_w,
                    height: card_h,
                };
                Render::upnext_card(f, card_area, app);
                y_top = y_top.saturating_add(card_h + gap);
            }
        }

        let mut regular: Vec<_> = app
            .notifications
            .iter()
            .filter(|n| !n.is_volume && !n.trivial)
            .collect();
        let volume: Vec<_> = app.notifications.iter().filter(|n| n.is_volume).collect();

        regular.truncate(5);

        for n in regular.iter() {
            let text_area_w = max_notif_width.saturating_sub(3 + padding * 2);
            let wrapped = wrap_text(&n.message, text_area_w as usize);
            let line_count = wrapped.len() as u16;
            let has_title = !n.title.is_empty();
            let title_rows = if has_title { 2 } else { 0 };
            let card_h = line_count + padding * 2 + title_rows;

            let card_y = y_top;
            if card_y.saturating_add(card_h) > area.bottom() {
                break;
            }

            let final_y = card_y;
            let start_y = area.y.saturating_sub(card_h + gap);
            let leaving = now.saturating_duration_since(n.expires_at);
            let y = if leaving > std::time::Duration::ZERO {
                let exit_progress = cubic_ease_in(
                    (leaving.as_millis() as f32 / NOTIFICATION_EXIT_DURATION.as_millis() as f32)
                        .min(1.0),
                );
                (final_y as f32 + (start_y as f32 - final_y as f32) * exit_progress) as u16
            } else {
                let progress = cubic_ease_out(n.animation_progress);
                (start_y as f32 + (final_y as f32 - start_y as f32) * progress) as u16
            };

            let card_area = Rect {
                x: center_x(max_notif_width),
                y,
                width: max_notif_width,
                height: card_h,
            };

            let bg = Block::default().style(Style::default().bg(app.float_bg()));
            f.render_widget(bg, card_area);

            let border_color = app.theme.notification_border;
            f.render_widget(
                Block::default().style(Style::default().bg(border_color)),
                Rect {
                    x: card_area.x,
                    y: card_area.y,
                    width: 1,
                    height: card_area.height,
                },
            );

            let inner = card_area.inner(Margin {
                horizontal: padding + 1,
                vertical: padding,
            });
            let inner = Rect {
                x: inner.x + 1,
                y: inner.y,
                width: inner.width.saturating_sub(1),
                height: inner.height,
            };
            let mut lines: Vec<Line> = Vec::with_capacity(1 + line_count as usize);
            if has_title {
                lines.push(Line::from(Span::styled(
                    n.title.clone(),
                    Style::default()
                        .fg(app.theme.accent)
                        .add_modifier(Modifier::BOLD),
                )));
                lines.push(Line::raw(""));
            }
            for l in wrapped.iter() {
                lines.push(Line::raw(l));
            }
            let para = Paragraph::new(lines).style(Style::default().fg(app.theme.fg_bright));
            f.render_widget(para, inner);

            y_top = card_y.saturating_add(card_h + gap);
        }

        let mut y_bar = area.y + padding;
        for n in volume.iter() {
            let bar_h = 10u16;
            let bar_w = 5u16;

            let final_y = y_bar;
            let start_y = area.y.saturating_sub(bar_h + 2 + gap);
            let leaving = now.saturating_duration_since(n.expires_at);
            let y = if leaving > std::time::Duration::ZERO {
                let exit_progress = cubic_ease_in(
                    (leaving.as_millis() as f32 / NOTIFICATION_EXIT_DURATION.as_millis() as f32)
                        .min(1.0),
                );
                (final_y as f32 + (start_y as f32 - final_y as f32) * exit_progress) as u16
            } else {
                let progress = cubic_ease_out(n.animation_progress);
                (start_y as f32 + (final_y as f32 - start_y as f32) * progress) as u16
            };

            if y.saturating_add(bar_h + 2) > area.bottom() {
                break;
            }
            let bar_area = Rect {
                x: area.x + area.width.saturating_sub(bar_w + padding),
                y,
                width: bar_w,
                height: bar_h + 2,
            };

            f.render_widget(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(app.theme.border))
                    .style(Style::default().bg(app.float_bg())),
                bar_area,
            );

            let fill_h = ((n.volume_value as f64 / 100.0) * bar_h as f64) as u16;
            if fill_h > 0 {
                let fill_area = Rect {
                    x: bar_area.x + 1,
                    y: bar_area.y + 1 + (bar_h - fill_h),
                    width: bar_w.saturating_sub(2),
                    height: fill_h,
                };
                let vol_color = app.theme.volume_color(n.volume_value);
                f.render_widget(
                    Block::default().style(Style::default().bg(vol_color)),
                    fill_area,
                );
            }

            let label_area = Rect {
                x: bar_area.x,
                y: bar_area.y + bar_h + 1,
                width: bar_w,
                height: 1,
            };
            let label = Paragraph::new(format!("{:>3}%", n.volume_value))
                .style(Style::default().fg(app.theme.fg_bright));
            f.render_widget(label, label_area);

            y_bar = final_y.saturating_add(bar_h + 2 + gap);
        }
    }

    pub(crate) fn content(f: &mut ratatui::Frame, area: Rect, app: &mut App) {
        if !app.is_ready {
            fill_pane(f, area, app);
            Render::loader(f, area, app, "Loading library\u{2026}");
            return;
        }
        Render::library(f, area, app);
    }

    /// Zen mode: render exactly one fullscreen surface at a time — the
    /// enlarged cover + centered progress, the visualizer, or the lyrics.
    pub(crate) fn zen(f: &mut ratatui::Frame, area: Rect, app: &mut App) {
        match app.zen_surface {
            ZenSurface::Cover => Render::zen_cover(f, area, app),
            ZenSurface::Visualizer => Render::zen_visualizer(f, area, app),
            ZenSurface::Lyrics => Render::zen_lyrics(f, area, app),
        }
    }

    /// Centered "title — artist" header used by the Zen cover and lyrics
    /// surfaces.
    pub(crate) fn zen_track_header(
        f: &mut ratatui::Frame,
        app: &App,
        track: &TrackInfo,
        area: Rect,
    ) {
        let title = if track.title.is_empty() {
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
            format!("  \u{2014}  {}", track.artist)
        };
        let header = Line::from(vec![
            Span::styled(
                title,
                Style::default()
                    .fg(app.theme.secondary_accent)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(artist, Style::default().fg(app.theme.fg)),
        ]);
        f.render_widget(Paragraph::new(header).alignment(Alignment::Center), area);
    }

    /// Zen surface 1: enlarged cover art centered on screen with the track
    /// title above and the progress bar / elapsed time centered underneath.
    pub(crate) fn zen_cover(f: &mut ratatui::Frame, area: Rect, app: &mut App) {
        let track = app.state.current_track.clone();
        let vchunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(2),
                Constraint::Min(0),
                Constraint::Length(5),
            ])
            .split(area);

        if let Some(t) = &track {
            Render::zen_track_header(f, app, t, vchunks[0]);
        }

        // Enlarged cover centered in the middle band. Half-block art keeps
        // the image square at a 1:2 cell aspect, so width = height * 2.
        let mid = vchunks[1];
        let max_w = mid.width.saturating_sub(4);
        let max_h = mid.height.saturating_sub(2);
        let mut w = max_w.min(max_h.saturating_mul(2));
        let mut h = w / 2;
        if h > max_h {
            h = max_h;
            w = h.saturating_mul(2);
        }
        h = h.max(1);
        w = w.max(2);
        let cover_area = Rect {
            x: mid.x + mid.width.saturating_sub(w) / 2,
            y: mid.y + mid.height.saturating_sub(h) / 2,
            width: w,
            height: h,
        };
        Render::cover(
            f,
            cover_area,
            app.np_cover.stateful.as_mut(),
            app.np_cover.image.as_deref(),
            app.theme.fg_dim,
            Some(" \u{266b} "),
        );

        // Centered progress bar with elapsed / total underneath; hidden for
        // live streams (mirrors the now-playing pane).
        let prog = vchunks[2];
        let dur = if app.state.duration > 0.0 {
            app.state.duration as u64
        } else {
            track.as_ref().map_or(0, |t| t.duration as u64)
        };
        let live = track.as_ref().is_some_and(|t| is_live_stream(&t.path));
        if dur > 0 && !live {
            let pos = app.display_position as u64;
            let ratio = (pos as f64 / dur as f64).clamp(0.0, 1.0);
            let bar_w = (prog.width as usize).min(64).saturating_sub(2).max(4);
            let bar = Render::progress_variant(ratio, bar_w, app);
            f.render_widget(
                Paragraph::new(Line::from(Span::styled(
                    bar,
                    Style::default().fg(app.theme.secondary_accent),
                )))
                .alignment(Alignment::Center),
                prog,
            );
            let time = format!(" {} / {}", format_duration(pos), format_duration(dur));
            f.render_widget(
                Paragraph::new(Line::from(Span::styled(
                    time,
                    Style::default().fg(app.theme.fg_dim),
                )))
                .alignment(Alignment::Center),
                Rect {
                    x: prog.x,
                    y: prog.y + 2,
                    width: prog.width,
                    height: 1,
                },
            );
        }
    }

    /// Zen surface 2: the audio visualizer stretched across the full screen.
    pub(crate) fn zen_visualizer(f: &mut ratatui::Frame, area: Rect, app: &mut App) {
        if !app.visualizer.is_enabled() {
            let msg = Paragraph::new(Line::from(Span::styled(
                "Visualizer disabled \u{2014} press Ctrl+V to enable",
                Style::default().fg(app.theme.fg_dim),
            )))
            .alignment(Alignment::Center);
            f.render_widget(msg, area);
            return;
        }
        let inner = Rect {
            x: area.x + 1,
            y: area.y + 1,
            width: area.width.saturating_sub(2),
            height: area.height.saturating_sub(2),
        };
        app.visualizer.tick(
            app.state.status == PlaybackStatus::Playing,
            inner.width,
            inner.height,
            &app.state.audio_levels,
            &app.state.wave_samples,
            app.state.wave_stereo,
        );
        if let Some(lines) = app.visualizer.render(inner, &app.theme) {
            f.render_widget(lines, inner);
        }
    }

    /// Zen surface 3: full-screen lyrics for the active track, sharing the
    /// exact same body rendering as the normal lyrics pane.
    pub(crate) fn zen_lyrics(f: &mut ratatui::Frame, area: Rect, app: &mut App) {
        if let Some(t) = app.state.current_track.clone() {
            Render::zen_track_header(
                f,
                app,
                &t,
                Rect {
                    x: area.x,
                    y: area.y,
                    width: area.width,
                    height: 1,
                },
            );
        }

        let Some(ref lyrics) = app.lyrics.current else {
            let msg = if app.lyrics.fetching {
                Line::from(vec![
                    Span::styled("Fetching lyrics ", Style::default().fg(app.theme.accent)),
                    Span::styled(
                        opencode_spinner(app.frame_count as usize),
                        Style::default()
                            .fg(app.theme.accent)
                            .add_modifier(Modifier::BOLD),
                    ),
                ])
            } else {
                Line::from(Span::styled(
                    "Press [l] to search",
                    Style::default().fg(app.theme.fg_dim),
                ))
            };
            f.render_widget(Paragraph::new(msg).alignment(Alignment::Center), area);
            return;
        };

        if lyrics.lines.is_empty() {
            f.render_widget(
                Paragraph::new("No lyrics found")
                    .alignment(Alignment::Center)
                    .style(Style::default().fg(app.theme.fg_dim)),
                area,
            );
            return;
        }

        let body = Rect {
            x: area.x.saturating_add(2),
            y: area.y.saturating_add(3),
            width: area.width.saturating_sub(4),
            height: area.height.saturating_sub(4),
        };
        Render::lyrics_body(f, body, app, lyrics);
    }

    pub(crate) fn footer_help(f: &mut ratatui::Frame, area: Rect, app: &App) {
        if app.pickers.is_open() || app.hide_help_bar {
            return;
        }
        let text = " [?] Help  [:] Command palette  [q] Quit ";
        let para = Paragraph::new(text)
            .alignment(Alignment::Right)
            .style(Style::default().fg(app.theme.fg_dim).bg(app.chrome_bg()));
        f.render_widget(para, area);
    }

    pub(crate) fn cover(
        f: &mut ratatui::Frame,
        area: Rect,
        cover_stateful: Option<&mut StatefulProtocol>,
        current_cover: Option<&[u8]>,
        placeholder_fg: Color,
        placeholder: Option<&str>,
    ) {
        if std::env::var("NVIM").is_ok() || std::env::var("ZELLIJ").is_ok() {
            let placeholder = Paragraph::new(Span::styled(
                " \u{266b} Cover art unavailable in this terminal ",
                Style::default().fg(placeholder_fg),
            ));
            f.render_widget(placeholder, area);
            return;
        }
        if area.width == 0 || area.height == 0 {
            return;
        }
        if let Some(protocol) = cover_stateful {
            let image = StatefulImage::new();
            f.render_stateful_widget(image, area, protocol);
        } else if let Some(cover_bytes) = current_cover {
            Render::cover_block(f, area, cover_bytes);
        } else if let Some(glyph) = placeholder {
            let placeholder = Paragraph::new(Line::from(Span::styled(
                format!("{:^width$}", glyph, width = area.width as usize),
                Style::default().fg(placeholder_fg),
            )))
            .alignment(Alignment::Center);
            f.render_widget(placeholder, area);
        }
    }

    pub(crate) fn cover_block(f: &mut ratatui::Frame, area: Rect, cover_bytes: &[u8]) {
        let img = match image::load_from_memory(cover_bytes) {
            Ok(img) => img.into_rgba8(),
            Err(_) => return,
        };
        let disp_w = (area.width as u32).max(1);
        let disp_h = (area.height as u32 * 2).max(1);

        let src_w = img.width() as f64;
        let src_h = img.height() as f64;
        let target_ratio = disp_w as f64 / disp_h as f64;
        let source_ratio = src_w / src_h;

        let cropped = if (source_ratio - target_ratio).abs() < 0.01 {
            img
        } else if source_ratio > target_ratio {
            let new_w = (src_h * target_ratio) as u32;
            let offset = ((src_w as u32 - new_w) / 2).min(img.width() - 1);
            image::imageops::crop_imm(&img, offset, 0, new_w, img.height()).to_image()
        } else {
            let new_h = (src_w / target_ratio) as u32;
            let offset = ((img.height() - new_h) / 2).min(img.height() - 1);
            image::imageops::crop_imm(&img, 0, offset, img.width(), new_h).to_image()
        };

        let thumb = image::imageops::resize(
            &cropped,
            disp_w,
            disp_h,
            image::imageops::FilterType::CatmullRom,
        );
        for y in 0..area.height as u32 {
            let mut spans = Vec::with_capacity(disp_w as usize);
            for x in 0..disp_w {
                let top = thumb.get_pixel(x, y * 2);
                let bot = if y * 2 + 1 < disp_h {
                    *thumb.get_pixel(x, y * 2 + 1)
                } else {
                    image::Rgba([0, 0, 0, 255])
                };
                let fg = ratatui::style::Color::Rgb(top[0], top[1], top[2]);
                let bg = ratatui::style::Color::Rgb(bot[0], bot[1], bot[2]);
                spans.push(Span::styled("\u{2580}", Style::default().fg(fg).bg(bg)));
            }
            let row = Rect {
                x: area.x,
                y: area.y + y as u16,
                width: area.width,
                height: 1,
            };
            f.render_widget(Paragraph::new(Line::from(spans)), row);
        }
    }

    pub(crate) fn evolving<W: ratatui::widgets::Widget>(
        f: &mut ratatui::Frame,
        area: Rect,
        widget: W,
        key: &'static str,
        app: &mut App,
        on_track_change: bool,
    ) {
        // Dust/thanos-style evolve-into is reserved for genuine auto-advances;
        // a manual Next/Prev shouldn't dissolve the pane. (First frame still
        // evolves so the startup animation is preserved.)
        let start = app.track_anim_trigger
            && app.auto_track_advance
            && (on_track_change || app.frame_count == 0);
        if !start && !app.anim_fx.is_running() {
            f.render_widget(widget, area);
            return;
        }
        let mut buf = ratatui::buffer::Buffer::empty(area);
        widget.render(area, &mut buf);
        if start {
            app.anim_fx.add_unique_effect(
                key,
                tachyonfx::fx::evolve_into(
                    tachyonfx::fx::EvolveSymbolSet::Circles,
                    (350, tachyonfx::Interpolation::QuadInOut),
                )
                .with_area(area)
                .with_filter(tachyonfx::CellFilter::All),
            );
        }
        app.anim_fx
            .process_effects(tachyonfx::Duration::from_millis(16), &mut buf, area);
        f.buffer_mut().merge(&buf);
    }

    pub(crate) fn pane_header(
        f: &mut ratatui::Frame,
        area: Rect,
        app: &App,
        label: &str,
        focused: bool,
        sep: bool,
        left_rule: bool,
    ) -> Rect {
        if left_rule {
            let rule = Block::default()
                .borders(Borders::LEFT)
                .border_style(Style::default().fg(app.theme.muted_border));
            f.render_widget(rule, area);
        }
        let inset: u16 = if left_rule { 1 } else { 0 };
        let text_x = area.x + inset;
        let text_w = area.width.saturating_sub(inset);
        if focused {
            let bar = Paragraph::new(Span::styled(
                "\u{258e}",
                Style::default().fg(app.theme.accent),
            ));
            f.render_widget(
                bar,
                Rect {
                    x: area.x,
                    y: area.y,
                    width: 1,
                    height: 1,
                },
            );
        }
        let label_style = if focused {
            Style::default()
                .fg(app.theme.accent)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default()
                .fg(app.theme.fg_bright)
                .add_modifier(Modifier::BOLD)
        };
        let header = Paragraph::new(Line::from(Span::styled(format!(" {label} "), label_style)));
        f.render_widget(
            header,
            Rect {
                x: text_x,
                y: area.y,
                width: text_w,
                height: 1,
            },
        );
        let mut content = Rect {
            x: text_x + 1,
            y: area.y.saturating_add(1),
            width: text_w.saturating_sub(2),
            height: area.height.saturating_sub(1),
        };
        if sep && content.height > 0 {
            let rule = Line::from(Span::styled(
                "\u{2500}".repeat(content.width as usize),
                Style::default().fg(app.theme.muted_border),
            ));
            f.render_widget(
                Paragraph::new(rule),
                Rect {
                    x: content.x,
                    y: content.y,
                    width: content.width,
                    height: 1,
                },
            );
            content = Rect {
                x: content.x,
                y: content.y.saturating_add(1),
                width: content.width,
                height: content.height.saturating_sub(1),
            };
        }
        content
    }

    pub(crate) fn library(f: &mut ratatui::Frame, area: Rect, app: &mut App) {
        let is_narrow = app.terminal_cols < 60;
        let is_small_height = app.terminal_rows < 22;
        // Visualizer needs at least 80 columns for useful display and is a
        // configurable extension (see `[extensions]` in the TUI config).
        let show_vis = app.visualizer.is_enabled()
            && app.extensions.is_enabled(ExtensionId::Visualizer)
            && app.terminal_cols >= 80;
        // Lyrics in third pane only when >= 100 columns; otherwise show in results pane
        let lyrics_third_pane = app.lyrics.show && app.terminal_cols >= 100;
        let np_height: u16 = if is_narrow {
            5
        } else if is_small_height {
            6
        } else {
            (area.height / 3).clamp(8, 14)
        };

        let lib_width: u16 = if is_narrow {
            (app.terminal_cols / 3)
                .max(12)
                .min(area.width.saturating_sub(2))
        } else {
            28u16.min(area.width.saturating_sub(2))
        };

        let lyrics_full_height = lyrics_third_pane;

        let (left_area, lyrics_area) = if lyrics_full_height {
            let lyrics_w = area.width / 3;
            let left_w = area.width - lyrics_w;
            let h = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([Constraint::Length(left_w), Constraint::Length(lyrics_w)])
                .split(area);
            // The last content row is reserved for the library stats line.
            let lyrics = Rect {
                height: h[1].height.saturating_sub(1),
                ..h[1]
            };
            (h[0], Some(lyrics))
        } else {
            (area, None)
        };

        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(np_height), Constraint::Min(1)])
            .split(left_area);

        let (np_area, vis_area) = if show_vis {
            let vis_w = app.terminal_cols / 3;
            let h = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([Constraint::Min(0), Constraint::Length(vis_w)])
                .split(chunks[0]);
            (h[0], Some(h[1]))
        } else {
            (chunks[0], None)
        };

        let panes = if is_narrow {
            if app.library_pane_focus {
                Layout::default()
                    .direction(Direction::Horizontal)
                    .constraints([Constraint::Min(0), Constraint::Length(0)])
                    .split(chunks[1])
                    .to_vec()
            } else {
                Layout::default()
                    .direction(Direction::Horizontal)
                    .constraints([Constraint::Length(0), Constraint::Min(0)])
                    .split(chunks[1])
                    .to_vec()
            }
        } else {
            Layout::default()
                .direction(Direction::Horizontal)
                .constraints([Constraint::Length(lib_width), Constraint::Min(0)])
                .split(chunks[1])
                .to_vec()
        };

        let left_focus = app.library_pane_focus;

        {
            let np_inner = Render::pane_header(f, np_area, app, "", false, true, false);
            fill_pane(f, np_inner, app);

            if let Some(track) = app.state.current_track.clone() {
                let inner = np_inner;
                let avail_h = inner.height.saturating_sub(2);
                let cover_h = if is_small_height {
                    avail_h.clamp(2, 5)
                } else {
                    avail_h.min(12)
                };
                let cover_w = cover_h * 2;

                // A live stream reports the track on air over ICY (or through
                // the station's tracklist), and the daemon's synthesised track
                // carries only the station name. Prefer the live title and the
                // artist half the tracklist supplies, falling back to the
                // station's own naming.
                let (display_title, display_artist, is_live) = match app.live_track() {
                    Some((title, artist)) => (title, artist, true),
                    None => {
                        let title = if track.title.is_empty() {
                            // Never surface a raw provider URI (e.g. a queued
                            // `spotify:track:` entry) as the title.
                            if track.path.starts_with("spotify:") {
                                "Spotify Track".to_string()
                            } else {
                                std::path::Path::new(&track.path)
                                    .file_stem()
                                    .map(|s| s.to_string_lossy().to_string())
                                    .unwrap_or_default()
                            }
                        } else {
                            track.title.clone()
                        };
                        let artist = if track.artist.is_empty() {
                            " ".to_string()
                        } else {
                            track.artist.clone()
                        };
                        (title, artist, false)
                    }
                };
                // A live track has no album: the daemon stamps the literal
                // "Radio" there, which is noise next to the artist.
                let has_album = !track.album.is_empty() && !is_live;

                // Progress: 1 row (available when dur > 0 AND not a live stream)
                let dur = if app.state.duration > 0.0 {
                    app.state.duration as u64
                } else {
                    track.duration as u64
                };
                let has_progress = dur > 0 && !is_live_stream(&track.path);

                // Show cover + details side-by-side whenever there is enough
                // horizontal room. On small-height terminals the cover is
                // scaled down but never stacked onto a single-line row: the
                // cover stays left with the track details to its right.
                if inner.width >= cover_w + 16 && (avail_h >= 5 || is_small_height) {
                    let hchunks = Layout::default()
                        .direction(Direction::Horizontal)
                        .constraints([
                            Constraint::Length(cover_w),
                            Constraint::Length(2),
                            Constraint::Min(0),
                        ])
                        .split(inner);

                    let cover_area = Rect {
                        x: hchunks[0].x,
                        y: hchunks[0].y + 1,
                        width: cover_w.min(hchunks[0].width),
                        height: cover_h.min(hchunks[0].height.saturating_sub(1)),
                    };
                    Render::cover(
                        f,
                        cover_area,
                        app.np_cover.stateful.as_mut(),
                        app.np_cover.image.as_deref(),
                        app.theme.fg_dim,
                        Some(" \u{266b} "),
                    );

                    let info_area = hchunks[2];

                    let info_rows =
                        2u16 + if has_album { 1 } else { 0 } + if has_progress { 1 } else { 0 };
                    let content_h: u16 = info_rows;
                    let offset = cover_h.saturating_sub(content_h) / 2;
                    let vchunks = Layout::default()
                        .direction(Direction::Vertical)
                        .constraints([
                            Constraint::Length(offset),
                            Constraint::Length(content_h),
                            Constraint::Min(0),
                        ])
                        .split(info_area);
                    let content_area = vchunks[1];

                    let mut info_constraints = vec![Constraint::Length(1), Constraint::Length(1)];
                    if has_album {
                        info_constraints.push(Constraint::Length(1));
                    }
                    if has_progress {
                        info_constraints.push(Constraint::Length(1));
                        info_constraints.push(Constraint::Length(1)); // Extra line for elapsed time
                    }
                    let info_chunks = Layout::default()
                        .direction(Direction::Vertical)
                        .constraints(info_constraints)
                        .split(content_area);

                    let title_text = display_title.to_string();
                    let title_avail = info_chunks[0].width as usize;
                    let animated_title =
                        scroll_text(&title_text, title_avail, app.np_title_scroll, true);
                    let title_para = Paragraph::new(Line::from(vec![Span::styled(
                        &animated_title,
                        Style::default()
                            .fg(app.theme.secondary_accent)
                            .add_modifier(Modifier::BOLD),
                    )]));
                    Render::evolving(f, info_chunks[0], title_para, "np", app, true);

                    let artist_para = Paragraph::new(Line::from(vec![Span::styled(
                        display_artist,
                        Style::default().fg(app.theme.fg_bright),
                    )]));
                    f.render_widget(artist_para, info_chunks[1]);

                    let mut info_row = 2;
                    if has_album {
                        let album_para = Paragraph::new(Line::from(vec![Span::styled(
                            &track.album,
                            Style::default().fg(app.theme.fg_bright),
                        )]));
                        f.render_widget(album_para, info_chunks[info_row]);
                        info_row += 1;
                    }
                    if has_progress {
                        let pos = app.display_position as u64;
                        let ratio = (pos as f64 / dur as f64).clamp(0.0, 1.0);
                        let bar_w =
                            (info_chunks[info_row].width / 3).saturating_sub(2).max(4) as usize;
                        let progress_str = Render::progress_variant(ratio, bar_w, app);
                        let time_str =
                            format!(" {} / {}", format_duration(pos), format_duration(dur));
                        // Progress bar on first line
                        let prog_para = Paragraph::new(Line::from(vec![Span::styled(
                            progress_str,
                            Style::default().fg(app.theme.secondary_accent),
                        )]));
                        f.render_widget(prog_para, info_chunks[info_row]);
                        // Elapsed time on second line
                        if info_row + 1 < info_chunks.len() {
                            let time_para = Paragraph::new(Line::from(vec![Span::styled(
                                time_str,
                                Style::default().fg(app.theme.fg_dim),
                            )]));
                            f.render_widget(time_para, info_chunks[info_row + 1]);
                        }
                    }
                } else if inner.width >= 12 {
                    // Compact layout still keeps the cover left with the
                    // details to its right (scaled to a slim column) whenever
                    // there is any horizontal room at all.
                    let slim_cover_h = inner.height.saturating_sub(2).clamp(2, 4);
                    let slim_cover_w = slim_cover_h * 2;
                    let hchunks = Layout::default()
                        .direction(Direction::Horizontal)
                        .constraints([
                            Constraint::Length(slim_cover_w),
                            Constraint::Length(1),
                            Constraint::Min(0),
                        ])
                        .split(inner);
                    let cover_area = Rect {
                        x: hchunks[0].x,
                        y: hchunks[0].y,
                        width: slim_cover_w.min(hchunks[0].width),
                        height: slim_cover_h.min(hchunks[0].height),
                    };
                    Render::cover(
                        f,
                        cover_area,
                        app.np_cover.stateful.as_mut(),
                        app.np_cover.image.as_deref(),
                        app.theme.fg_dim,
                        Some(" \u{266b} "),
                    );
                    let info_area = hchunks[2];
                    let title_text = display_title.to_string();
                    let title_avail = info_area.width as usize;
                    let animated_title =
                        scroll_text(&title_text, title_avail, app.np_title_scroll, true);
                    let lines = vec![
                        Line::from(vec![Span::styled(
                            &animated_title,
                            Style::default()
                                .fg(app.theme.secondary_accent)
                                .add_modifier(Modifier::BOLD),
                        )]),
                        Line::from(vec![Span::styled(
                            display_artist,
                            Style::default().fg(app.theme.fg_bright),
                        )]),
                    ];
                    Render::evolving(f, info_area, Paragraph::new(lines), "np", app, true);
                } else {
                    let title_text = display_title.to_string();
                    let title_avail = inner.width as usize;
                    let animated_title =
                        scroll_text(&title_text, title_avail, app.np_title_scroll, true);
                    let title_para = Paragraph::new(Line::from(vec![Span::styled(
                        &animated_title,
                        Style::default()
                            .fg(app.theme.secondary_accent)
                            .add_modifier(Modifier::BOLD),
                    )]));
                    let title_area = Rect {
                        x: inner.x,
                        y: inner.y,
                        width: inner.width,
                        height: 1,
                    };
                    Render::evolving(f, title_area, title_para, "np", app, true);

                    let row_offset = 1u16;
                    if !track.album.is_empty() {
                        let album_para = Paragraph::new(Line::from(vec![Span::styled(
                            &track.album,
                            Style::default().fg(app.theme.fg_bright),
                        )]));
                        let album_area = Rect {
                            x: inner.x,
                            y: inner.y + row_offset,
                            width: inner.width,
                            height: 1,
                        };
                        f.render_widget(album_para, album_area);
                    }
                }
            } else {
                let inner = np_inner;
                let lines = vec![Line::from(Span::styled(
                    "It's awfully quiet here…",
                    Style::default()
                        .fg(app.theme.fg_bright)
                        .add_modifier(Modifier::BOLD),
                ))];
                let msg = Paragraph::new(lines);
                Render::evolving(f, inner, msg, "idle", app, false);
            }
        }

        if let Some(vis_a) = vis_area
            && vis_a.width >= 4
            && vis_a.height >= 3
        {
            app.visualizer.tick(
                app.state.status == PlaybackStatus::Playing,
                vis_a.width.saturating_sub(2),
                vis_a.height.saturating_sub(1),
                &app.state.audio_levels,
                &app.state.wave_samples,
                app.state.wave_stereo,
            );
            let vis_header = Paragraph::new(Line::from(Span::styled(
                " ",
                Style::default()
                    .fg(app.theme.fg_dim)
                    .add_modifier(Modifier::BOLD),
            )));
            f.render_widget(
                vis_header,
                Rect {
                    x: vis_a.x,
                    y: vis_a.y,
                    width: vis_a.width,
                    height: 1,
                },
            );
            let vis_inner = Rect {
                x: vis_a.x + 1,
                y: vis_a.y + 1,
                width: vis_a.width.saturating_sub(2),
                height: vis_a.height.saturating_sub(1),
            };
            if let Some(lines) = app.visualizer.render(vis_inner, &app.theme) {
                f.render_widget(lines, vis_inner);
            }
        }

        let lib_icons = if use_nerd_fonts() {
            LIBRARY_ICONS_NERD
        } else {
            LIBRARY_ICONS_ASCII
        };
        let visible_cats = app.visible_library_indices();
        let left_items: Vec<ListItem> = visible_cats
            .iter()
            .map(|&i| {
                let cat = LIBRARY_CATEGORIES[i];
                let icon = lib_icons.get(i).unwrap_or(&" ");
                let count = match cat {
                    "All Tracks" => app.tracks_cache.len(),
                    "Liked" => app.tracks_cache.iter().filter(|t| t.favourite).count(),
                    "Albums" => app.unique_albums().len(),
                    "Artists" => app.unique_artists().len(),
                    "Playlists" => app.playlist_cache.len(),
                    "Spotify" => app.spotify.playlists.len(),
                    "Radio" => app.radio.custom.len(),
                    "Most Played" => app.most_played_cache.len(),
                    "Recently Played" => app.recently_played_cache.len(),
                    "Recently Added" => app.recently_added_cache.len(),
                    "Genres" => app.unique_genres().len(),
                    "Folders" => app.unique_folders().len(),
                    _ => 0,
                };
                let label = if count > 0 {
                    format!(" {icon}  {:<14} {:>4}", cat, count)
                } else {
                    format!(" {icon}  {}", cat)
                };
                let is_active = i == app.library_category;
                let style = if is_active && left_focus {
                    Style::default()
                        .fg(app.theme.selection_fg_readable())
                        .bg(app.theme.selection_bg)
                } else if is_active {
                    Style::default().fg(app.theme.accent)
                } else {
                    Style::default().fg(app.theme.fg)
                };
                ListItem::new(label).style(style)
            })
            .collect();

        let left_inner = Render::pane_header(f, panes[0], app, " ", left_focus, false, false);
        fill_pane(f, left_inner, app);

        let track_info_h: u16 = if app.show_preview && app.track_popup_visible && !is_small_height {
            let avail_h = left_inner.height.saturating_sub(1);
            let need = info_block_h();
            // Reserve at least 4 rows for the category list so "Spotify" never gets clipped.
            let max_card = avail_h.saturating_sub(4);
            need.min(max_card.max(6))
        } else {
            0
        };
        let left_vchunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Min(4),
                Constraint::Length(
                    if app.show_preview && app.track_popup_visible && !is_small_height {
                        1
                    } else {
                        0
                    },
                ),
                Constraint::Length(track_info_h),
            ])
            .split(left_inner);
        let left_list_area = left_vchunks[0];
        let info_sep_area = left_vchunks[1];
        let left_info_area = left_vchunks[2];

        f.render_widget(List::new(left_items), left_list_area);

        if app.library_category < LIBRARY_CATEGORIES.len()
            && let Some(row) = app
                .visible_library_indices()
                .iter()
                .position(|&i| i == app.library_category)
        {
            let indicator_y = left_list_area.y + row as u16;
            if indicator_y < left_list_area.y + left_list_area.height {
                let indicator_area = Rect {
                    x: left_list_area.x + 1,
                    y: indicator_y,
                    width: 1,
                    height: 1,
                };
                let indicator =
                    Paragraph::new("▎").style(Style::default().fg(app.theme.sidebar_active_border));
                f.render_widget(indicator, indicator_area);
            }
        }

        let category_label = LIBRARY_CATEGORIES
            .get(app.library_category)
            .unwrap_or(&"All Tracks");

        // Total rows in the active right-pane list, threaded out of the category
        // branches so mouse hit zones only cover real rows.
        let mut lib_total_rows: usize = 0;
        let (right_lines, _stats_line) = if app.browse_detail.is_some() && app.library_category == 5
        {
            let tracks = &app.spotify.playlist_tracks_cache;
            let total_len = app.spotify_playlist_rows();
            let st_line = format!(
                " {} {} (+ play all / shuffle) ",
                tracks.len(),
                plural(tracks.len(), "track", "tracks")
            );
            let reserve = 3usize;
            let available = panes[1].height.saturating_sub(reserve as u16) as usize;
            app.viewport_items = available;
            let sel = app.list_pos().min(total_len.saturating_sub(1));
            let (list_scroll, end) = step_viewport(app.list_scroll, sel, available, total_len);
            app.list_scroll = list_scroll;

            let pane_w = panes[1].width as usize;
            let mut lines = vec![Line::from("")];
            const ACTION_ROWS: usize = App::SPOTIFY_PLAYLIST_ROWS;
            // Rows 0/1: virtual actions (Play All / Shuffle), then the tracks.
            let action_help = [("▶  Play All", "  Enter"), ("🔀  Shuffle", "  Enter / S")];
            for (ai, (action, key_hint)) in action_help.iter().enumerate() {
                let real_i = ai;
                let is_sel = real_i == sel && !left_focus;
                let style = if is_sel {
                    Style::default()
                        .fg(app.theme.selection_fg_readable())
                        .bg(app.theme.selection_bg)
                } else {
                    Style::default().fg(app.theme.fg_bright)
                };
                let prefix = if is_sel { " > " } else { "   " };
                let content = format!("{prefix}{action}");
                let pad = row_pad(&content, panes[1].width);
                let hint_style = if is_sel {
                    Style::default()
                        .fg(app.theme.selection_fg_readable())
                        .bg(app.theme.selection_bg)
                } else {
                    Style::default().fg(app.theme.fg_dim)
                };
                lines.push(Line::from(vec![
                    Span::styled(format!("{content}{}", " ".repeat(pad)), style),
                    Span::styled(format!("{key_hint:>10}"), hint_style),
                ]));
            }
            if tracks.is_empty() {
                lines.push(Line::from(Span::styled(
                    " No tracks: run Settings > Spotify > Sync Now, then press Enter again",
                    Style::default().fg(app.theme.fg_dim),
                )));
                lib_total_rows = total_len;
                (lines, st_line)
            } else {
                let start = app
                    .list_scroll
                    .saturating_sub(ACTION_ROWS)
                    .min(tracks.len());
                let stop = end.saturating_sub(ACTION_ROWS).min(tracks.len());
                for (i, tr) in tracks[start..stop].iter().enumerate() {
                    // True cursor row of this track: the two virtual action
                    // rows (Play All / Shuffle) sit above the track list, so
                    // the highlight must never alias onto an action row
                    // (which previously produced two highlighters at once).
                    let real_i = ACTION_ROWS + start + i;
                    let is_sel = real_i == sel && !left_focus;
                    let is_multiselected = app.multiselect_mode
                        && tr.uri.as_deref().is_some_and(|u| app.row_is_selected(u));
                    let label = tr.name.clone();
                    let avail = pane_w.saturating_sub(2);
                    let display_label = scroll_text(&label, avail, app.footer_title_scroll, is_sel);
                    let dur = tr
                        .duration_ms
                        .map(|d| format_duration_short(d / 1000))
                        .unwrap_or_default();
                    let prefix = if is_sel { " > " } else { "   " };
                    let name_pad = avail.saturating_sub(10);
                    let style = if is_sel {
                        Style::default()
                            .fg(app.theme.selection_fg_readable())
                            .bg(app.theme.selection_bg)
                    } else if is_multiselected {
                        Style::default().bg(app.theme.warning).fg(app.theme.bg)
                    } else {
                        Style::default().fg(app.theme.fg)
                    };
                    let dur_style = if is_sel {
                        Style::default()
                            .fg(app.theme.selection_fg_readable())
                            .bg(app.theme.selection_bg)
                    } else {
                        Style::default().fg(app.theme.fg_dim)
                    };
                    let checkbox = if is_multiselected { "☑ " } else { "" };
                    let head = format!(
                        "{prefix}{checkbox}{:<width$}",
                        display_label,
                        width = name_pad.saturating_sub(checkbox.len())
                    );
                    let tail = format!("  {:>6}", dur);
                    let pad = row_pad(&format!("{head}{tail}"), panes[1].width);
                    lines.push(Line::from(vec![
                        Span::styled(head, style),
                        Span::styled(format!("{tail}{}", " ".repeat(pad)), dur_style),
                    ]));
                }
                lib_total_rows = total_len;
                (lines, st_line)
            }
        } else if app.browse_detail.is_some() {
            let (total_len, hours, mins) = {
                let f = app.filtered_tracks();
                let total_dur: u64 = f.iter().map(|t| t.duration as u64).sum();
                (f.len(), total_dur / 3600, (total_dur % 3600) / 60)
            };
            let st_line = format!(
                " {} {} | {}h {}m ",
                total_len,
                plural(total_len, "track", "tracks"),
                hours,
                mins
            );

            let reserve = 3usize;
            let available = panes[1].height.saturating_sub(reserve as u16) as usize;
            app.viewport_items = available;
            let sel = app.list_pos().min(total_len.saturating_sub(1));
            let (list_scroll, end) = step_viewport(app.list_scroll, sel, available, total_len);
            app.list_scroll = list_scroll;

            let filtered = app.filtered_tracks();

            let pane_w = panes[1].width as usize;
            let mut lines = vec![Line::from("")];
            if filtered.is_empty() {
                lines.push(Line::from(Span::styled(
                    " No tracks found for this selection",
                    Style::default().fg(app.theme.fg_dim),
                )));
                (lines, " 0 tracks | 0h 0m ".to_string())
            } else {
                for (i, track) in filtered[app.list_scroll..end].iter().enumerate() {
                    let real_i = app.list_scroll + i;
                    let is_current =
                        app.state.current_track.as_ref().map(|t| t.id) == Some(track.id);
                    let is_sel = real_i == sel && !left_focus;
                    let is_multiselected = app.multiselect_mode && app.row_is_selected(&track.path);
                    let label = track.title.clone();
                    let avail = pane_w.saturating_sub(2);
                    let display_label = scroll_text(&label, avail, app.footer_title_scroll, is_sel);
                    let prefix = if is_current { "\u{25b6} " } else { "  " };
                    let checkbox = if is_multiselected { "☑ " } else { "" };
                    let row = format!("{}{}{}", prefix, checkbox, display_label);
                    let style = if is_sel {
                        Style::default()
                            .fg(app.theme.selection_fg_readable())
                            .bg(app.theme.selection_bg)
                    } else if is_current {
                        Style::default()
                            .fg(app.theme.accent)
                            .add_modifier(Modifier::BOLD)
                    } else if is_multiselected {
                        Style::default().bg(app.theme.warning).fg(app.theme.bg)
                    } else {
                        Style::default()
                    };
                    let row = if is_sel {
                        let pad = row_pad(&row, panes[1].width);
                        format!("{row}{}", " ".repeat(pad))
                    } else {
                        row
                    };
                    lines.push(Line::from(Span::styled(row, style)));
                }
                lib_total_rows = total_len;
                (lines, st_line)
            }
        } else if app.library_category == 2 {
            let albums = app.unique_albums();
            let total_len = albums.len();
            let sel = app.list_pos().min(total_len.saturating_sub(1));
            let st_line = format!(" {} {} ", total_len, plural(total_len, "album", "albums"));
            let reserve = 3usize;
            let available = panes[1].height.saturating_sub(reserve as u16) as usize;
            app.viewport_items = available;
            let (list_scroll, end) = step_viewport(app.list_scroll, sel, available, total_len);
            app.list_scroll = list_scroll;
            let mut lines = vec![Line::from("")];
            for (i, (name, _count)) in albums[app.list_scroll..end].iter().enumerate() {
                let real_i = app.list_scroll + i;
                let is_sel = real_i == sel && !left_focus;
                let prefix = if is_sel { " > " } else { "   " };
                let style = if is_sel {
                    Style::default()
                        .fg(app.theme.selection_fg_readable())
                        .bg(app.theme.selection_bg)
                } else {
                    Style::default().fg(app.theme.fg)
                };
                let row = format!("{}{}", prefix, name);
                let row = if is_sel {
                    let pad = row_pad(&row, panes[1].width);
                    format!("{row}{}", " ".repeat(pad))
                } else {
                    row
                };
                lines.push(Line::from(Span::styled(row, style)));
            }
            {
                lib_total_rows = total_len;
                (lines, st_line)
            }
        } else if app.library_category == 3 {
            let artists = app.unique_artists();
            let total_len = artists.len();
            let sel = app.list_pos().min(total_len.saturating_sub(1));
            let st_line = format!(" {} {} ", total_len, plural(total_len, "artist", "artists"));
            let reserve = 3usize;
            let available = panes[1].height.saturating_sub(reserve as u16) as usize;
            app.viewport_items = available;
            let (list_scroll, end) = step_viewport(app.list_scroll, sel, available, total_len);
            app.list_scroll = list_scroll;
            let mut lines = vec![Line::from("")];
            for (i, (name, _count)) in artists[app.list_scroll..end].iter().enumerate() {
                let real_i = app.list_scroll + i;
                let is_sel = real_i == sel && !left_focus;
                let prefix = if is_sel { " > " } else { "   " };
                let style = if is_sel {
                    Style::default()
                        .fg(app.theme.selection_fg_readable())
                        .bg(app.theme.selection_bg)
                } else {
                    Style::default().fg(app.theme.fg)
                };
                let row = format!("{}{}", prefix, name);
                let row = if is_sel {
                    let pad = row_pad(&row, panes[1].width);
                    format!("{row}{}", " ".repeat(pad))
                } else {
                    row
                };
                lines.push(Line::from(Span::styled(row, style)));
            }
            {
                lib_total_rows = total_len;
                (lines, st_line)
            }
        } else if app.library_category == 4 {
            let playlists = &app.playlist_cache;
            let total_len = playlists.len();
            let sel = app.list_pos().min(total_len.saturating_sub(1));
            let st_line = format!(
                " {} {} ",
                total_len,
                plural(total_len, "playlist", "playlists")
            );
            let reserve = 3usize;
            let available = panes[1].height.saturating_sub(reserve as u16) as usize;
            app.viewport_items = available;
            let (list_scroll, end) = step_viewport(app.list_scroll, sel, available, total_len);
            app.list_scroll = list_scroll;
            let mut lines = vec![Line::from("")];
            for (i, pl) in playlists[app.list_scroll..end].iter().enumerate() {
                let real_i = app.list_scroll + i;
                let is_sel = real_i == sel && !left_focus;
                let prefix = if is_sel { " > " } else { "   " };
                let style = if is_sel {
                    Style::default()
                        .fg(app.theme.selection_fg_readable())
                        .bg(app.theme.selection_bg)
                } else {
                    Style::default().fg(app.theme.fg)
                };
                let row = format!("{}{}", prefix, pl.name);
                let row = if is_sel {
                    let pad = row_pad(&row, panes[1].width);
                    format!("{row}{}", " ".repeat(pad))
                } else {
                    row
                };
                lines.push(Line::from(Span::styled(row, style)));
            }
            {
                lib_total_rows = total_len;
                (lines, st_line)
            }
        } else if app.library_category == 5 {
            let playlists = &app.spotify.playlists;
            let total_len = playlists.len();
            let sel = app.list_pos().min(total_len.saturating_sub(1));
            let st_line = format!(
                " {} {} ",
                total_len,
                plural(total_len, "playlist", "playlists")
            );
            let reserve = 3usize;
            let available = panes[1].height.saturating_sub(reserve as u16) as usize;
            app.viewport_items = available;
            let (list_scroll, end) = step_viewport(app.list_scroll, sel, available, total_len);
            app.list_scroll = list_scroll;
            let mut lines = vec![Line::from("")];
            if playlists.is_empty() {
                lines.push(Line::from(Span::styled(
                    " No synced playlists: link an account in Settings > Spotify",
                    Style::default().fg(app.theme.fg_dim),
                )));
            } else {
                for (i, pl) in playlists[app.list_scroll..end].iter().enumerate() {
                    let real_i = app.list_scroll + i;
                    let is_sel = real_i == sel && !left_focus;
                    let prefix = if is_sel { " > " } else { "   " };
                    let style = if is_sel {
                        Style::default()
                            .fg(app.theme.selection_fg_readable())
                            .bg(app.theme.selection_bg)
                    } else {
                        Style::default().fg(app.theme.fg)
                    };
                    let row = format!("{}{}", prefix, pl.name);
                    let row = if is_sel {
                        let pad = row_pad(&row, panes[1].width);
                        format!("{row}{}", " ".repeat(pad))
                    } else {
                        row
                    };
                    lines.push(Line::from(Span::styled(row, style)));
                }
            }
            {
                lib_total_rows = total_len;
                (lines, st_line)
            }
        } else if app.library_category == 6 {
            let stations = &app.radio.custom;
            let total_len = stations.len();
            let sel = app.list_pos().min(total_len.saturating_sub(1));
            let st_line = format!(
                " {} {} ",
                total_len,
                plural(total_len, "station", "stations")
            );
            let reserve = 3usize;
            let available = panes[1].height.saturating_sub(reserve as u16) as usize;
            app.viewport_items = available;
            let (list_scroll, end) = step_viewport(app.list_scroll, sel, available, total_len);
            app.list_scroll = list_scroll;
            let mut lines = vec![Line::from("")];
            if stations.is_empty() {
                lines.extend(empty_hint_lines(
                    app,
                    "No custom stations yet",
                    "Hint: open Radio Browser with / or r, then save one with s",
                ));
            } else {
                for (i, s) in stations[app.list_scroll..end].iter().enumerate() {
                    let real_i = app.list_scroll + i;
                    let is_sel = real_i == sel && !left_focus;
                    let prefix = if is_sel { " > " } else { "   " };
                    let style = if is_sel {
                        Style::default()
                            .fg(app.theme.selection_fg_readable())
                            .bg(app.theme.selection_bg)
                    } else {
                        Style::default().fg(app.theme.fg)
                    };
                    let row = format!("{}{}", prefix, s.name);
                    let row = if is_sel {
                        let pad = row_pad(&row, panes[1].width);
                        format!("{row}{}", " ".repeat(pad))
                    } else {
                        row
                    };
                    lines.push(Line::from(Span::styled(row, style)));
                }
            }
            {
                lib_total_rows = total_len;
                (lines, st_line)
            }
        } else if app.library_category == 10 {
            let genres = app.unique_genres();
            let total_len = genres.len();
            let sel = app.list_pos().min(total_len.saturating_sub(1));
            let st_line = format!(" {} {} ", total_len, plural(total_len, "genre", "genres"));
            let reserve = 3usize;
            let available = panes[1].height.saturating_sub(reserve as u16) as usize;
            app.viewport_items = available;
            let (list_scroll, end) = step_viewport(app.list_scroll, sel, available, total_len);
            app.list_scroll = list_scroll;
            let mut lines = vec![Line::from("")];
            if genres.is_empty() {
                lines.extend(empty_hint_lines(
                    app,
                    "No genres yet",
                    "Hint: import tagged audio files, then browse them here by genre",
                ));
            } else {
                for (i, (name, _count)) in genres[app.list_scroll..end].iter().enumerate() {
                    let real_i = app.list_scroll + i;
                    let is_sel = real_i == sel && !left_focus;
                    let prefix = if is_sel { " > " } else { "   " };
                    let style = if is_sel {
                        Style::default()
                            .fg(app.theme.selection_fg_readable())
                            .bg(app.theme.selection_bg)
                    } else {
                        Style::default().fg(app.theme.fg)
                    };
                    let row = format!("{}{}", prefix, name);
                    let row = if is_sel {
                        let pad = row_pad(&row, panes[1].width);
                        format!("{row}{}", " ".repeat(pad))
                    } else {
                        row
                    };
                    lines.push(Line::from(Span::styled(row, style)));
                }
            }
            {
                lib_total_rows = total_len;
                (lines, st_line)
            }
        } else if app.library_category == 11 {
            let folders = app.unique_folders();
            let total_len = folders.len();
            let sel = app.list_pos().min(total_len.saturating_sub(1));
            let st_line = format!(" {} {} ", total_len, plural(total_len, "folder", "folders"));
            let reserve = 3usize;
            let available = panes[1].height.saturating_sub(reserve as u16) as usize;
            app.viewport_items = available;
            let (list_scroll, end) = step_viewport(app.list_scroll, sel, available, total_len);
            app.list_scroll = list_scroll;
            let mut lines = vec![Line::from("")];
            for (i, (dir, _count)) in folders[app.list_scroll..end].iter().enumerate() {
                let real_i = app.list_scroll + i;
                let is_sel = real_i == sel && !left_focus;
                let prefix = if is_sel { " > " } else { "   " };
                let style = if is_sel {
                    Style::default()
                        .fg(app.theme.selection_fg_readable())
                        .bg(app.theme.selection_bg)
                } else {
                    Style::default().fg(app.theme.fg)
                };
                let name = folder_name(dir);
                let row = format!("{}{}", prefix, name);
                let row = if is_sel {
                    let pad = row_pad(&row, panes[1].width);
                    format!("{row}{}", " ".repeat(pad))
                } else {
                    row
                };
                lines.push(Line::from(Span::styled(row, style)));
            }
            {
                lib_total_rows = total_len;
                (lines, st_line)
            }
        } else if app.library_category == 12 {
            // Top Charts: three-level navigation
            // Level 0: Chart sources (Spotify, Apple Music, …)
            // Level 1: Charts for selected source
            // Level 2: Tracks for selected chart
            let sources = &app.charts.sources;
            let charts = &app.charts.charts;
            let chart_tracks = &app.charts.chart_tracks;

            if app.charts.selected_source.is_none() {
                // Level 0: Show sources
                let total_len = sources.len();
                let sel = app.list_pos().min(total_len.saturating_sub(1));
                let st_line = format!(" {} {} ", total_len, plural(total_len, "source", "sources"));
                let reserve = 3usize;
                let available = panes[1].height.saturating_sub(reserve as u16) as usize;
                app.viewport_items = available;
                let (list_scroll, end) = step_viewport(app.list_scroll, sel, available, total_len);
                app.list_scroll = list_scroll;
                let mut lines = vec![Line::from("")];
                if sources.is_empty() {
                    lines.extend(empty_hint_lines(
                        app,
                        "No chart sources available",
                        "Hint: free charts (iTunes) load automatically",
                    ));
                } else {
                    for (i, src) in sources[app.list_scroll..end].iter().enumerate() {
                        let real_i = app.list_scroll + i;
                        let is_sel = real_i == sel && !left_focus;
                        let prefix = if is_sel { " > " } else { "   " };
                        let style = if is_sel {
                            Style::default()
                                .fg(app.theme.selection_fg_readable())
                                .bg(app.theme.selection_bg)
                        } else {
                            Style::default().fg(app.theme.fg)
                        };
                        let configured = if src.configured { "●" } else { "○" };
                        let row = format!("{}{} {} ({})", prefix, configured, src.display, src.id);
                        let row = if is_sel {
                            let pad = row_pad(&row, panes[1].width);
                            format!("{row}{}", " ".repeat(pad))
                        } else {
                            row
                        };
                        lines.push(Line::from(Span::styled(row, style)));
                    }
                }
                {
                    lib_total_rows = total_len;
                    (lines, st_line)
                }
            } else if app.charts.selected_chart.is_none() {
                // Level 1: Show charts for selected source
                let total_len = charts.len();
                let sel = app.list_pos().min(total_len.saturating_sub(1));
                let src_name = sources
                    .get(app.charts.selected_source.unwrap_or(0))
                    .map(|s| s.display.as_str())
                    .unwrap_or("Charts");
                let st_line = format!(" {} {} ", total_len, plural(total_len, "chart", "charts"));
                let reserve = 3usize;
                let available = panes[1].height.saturating_sub(reserve as u16) as usize;
                app.viewport_items = available;
                let (list_scroll, end) = step_viewport(app.list_scroll, sel, available, total_len);
                app.list_scroll = list_scroll;
                let mut lines = vec![Line::from("")];
                if charts.is_empty() {
                    lines.extend(empty_hint_lines(
                        app,
                        &format!("No charts for {}", src_name),
                        "Hint: press Enter on a source to load its charts",
                    ));
                } else {
                    for (i, ch) in charts[app.list_scroll..end].iter().enumerate() {
                        let real_i = app.list_scroll + i;
                        let is_sel = real_i == sel && !left_focus;
                        let prefix = if is_sel { " > " } else { "   " };
                        let style = if is_sel {
                            Style::default()
                                .fg(app.theme.selection_fg_readable())
                                .bg(app.theme.selection_bg)
                        } else {
                            Style::default().fg(app.theme.fg)
                        };
                        let track_info = ch
                            .track_count
                            .map(|c| format!(" [{c} tracks]"))
                            .unwrap_or_default();
                        let row = format!("{}{}{}", prefix, ch.title, track_info);
                        let row = if is_sel {
                            let pad = row_pad(&row, panes[1].width);
                            format!("{row}{}", " ".repeat(pad))
                        } else {
                            row
                        };
                        lines.push(Line::from(Span::styled(row, style)));
                    }
                }
                {
                    lib_total_rows = total_len;
                    (lines, st_line)
                }
            } else {
                // Level 2: Show tracks for selected chart
                let total_len = chart_tracks.len();
                let sel = app.list_pos().min(total_len.saturating_sub(1));
                let st_line = format!(" {} {} ", total_len, plural(total_len, "track", "tracks"));
                let reserve = 3usize;
                let available = panes[1].height.saturating_sub(reserve as u16) as usize;
                app.viewport_items = available;
                let (list_scroll, end) = step_viewport(app.list_scroll, sel, available, total_len);
                app.list_scroll = list_scroll;
                let pane_w = panes[1].width as usize;
                let mut lines = vec![Line::from("")];
                if chart_tracks.is_empty() {
                    lines.extend(empty_hint_lines(
                        app,
                        "No tracks in this chart",
                        "Hint: press Backspace to go back",
                    ));
                } else {
                    for (i, track) in chart_tracks[app.list_scroll..end].iter().enumerate() {
                        let real_i = app.list_scroll + i;
                        let is_sel = real_i == sel && !left_focus;
                        let is_multiselected =
                            app.multiselect_mode && app.row_is_selected(&track.uri);
                        let avail = pane_w.saturating_sub(2);
                        let label = track.title.clone();
                        let display_label =
                            scroll_text(&label, avail, app.footer_title_scroll, is_sel);
                        let artists = &track.artists;
                        let prefix = if is_sel { " > " } else { "   " };
                        let checkbox = if is_multiselected { "☑ " } else { "" };
                        let style = if is_sel {
                            Style::default()
                                .fg(app.theme.selection_fg_readable())
                                .bg(app.theme.selection_bg)
                        } else if is_multiselected {
                            Style::default().fg(app.theme.accent)
                        } else {
                            Style::default().fg(app.theme.fg)
                        };
                        let row = format!(
                            "{}{}{} \u{2014} {}",
                            prefix, checkbox, display_label, artists
                        );
                        let row = if is_sel {
                            let pad = row_pad(&row, panes[1].width);
                            format!("{row}{}", " ".repeat(pad))
                        } else {
                            row
                        };
                        lines.push(Line::from(Span::styled(row, style)));
                    }
                }
                {
                    lib_total_rows = total_len;
                    (lines, st_line)
                }
            }
        } else {
            let (total_len, total_dur) = {
                let f = app.filtered_tracks();
                let dur: u64 = f.iter().map(|t| t.duration as u64).sum();
                (f.len(), dur)
            };
            let hours = total_dur / 3600;
            let mins = (total_dur % 3600) / 60;
            let st_line = format!(
                " {} {} | {}h {}m ",
                total_len,
                plural(total_len, "track", "tracks"),
                hours,
                mins
            );

            let reserve = 3usize;
            let available = panes[1].height.saturating_sub(reserve as u16) as usize;
            app.viewport_items = available;
            let sel = app.list_pos().min(total_len.saturating_sub(1));
            let (list_scroll, end) = step_viewport(app.list_scroll, sel, available, total_len);
            app.list_scroll = list_scroll;

            let filtered = app.filtered_tracks();
            let pane_w = panes[1].width as usize;

            let mut lines = vec![Line::from("")];
            if filtered.is_empty() {
                let (headline, hint) = match app.library_category {
                    7 => (
                        "No most-played tracks yet",
                        "Hint: play counts build up as you listen",
                    ),
                    8 => (
                        "Nothing played recently",
                        "Hint: play any track and it will show up here",
                    ),
                    9 => (
                        "No recent additions",
                        "Hint: add music to your library to see it here",
                    ),
                    _ => (
                        "No tracks in this list",
                        "Hint: add music to your library to get started",
                    ),
                };
                lines.extend(empty_hint_lines(app, headline, hint));
            } else {
                for (i, track) in filtered[app.list_scroll..end].iter().enumerate() {
                    let real_i = app.list_scroll + i;
                    let is_current =
                        app.state.current_track.as_ref().map(|t| t.id) == Some(track.id);
                    let is_sel = real_i == sel && !left_focus;
                    let is_multiselected = app.multiselect_mode && app.row_is_selected(&track.path);
                    let label = track.title.clone();
                    let avail = pane_w.saturating_sub(2);
                    let display_label = scroll_text(&label, avail, app.footer_title_scroll, is_sel);
                    let prefix = if is_current { "\u{25b6} " } else { "  " };
                    let checkbox = if is_multiselected { "☑ " } else { "" };
                    let row = format!("{}{}{}", prefix, checkbox, display_label);
                    let style = if is_sel {
                        Style::default()
                            .fg(app.theme.selection_fg_readable())
                            .bg(app.theme.selection_bg)
                    } else if is_current {
                        Style::default()
                            .fg(app.theme.accent)
                            .add_modifier(Modifier::BOLD)
                    } else if is_multiselected {
                        Style::default().bg(app.theme.warning).fg(app.theme.bg)
                    } else {
                        Style::default()
                    };
                    let row = if is_sel {
                        let pad = row_pad(&row, panes[1].width);
                        format!("{row}{}", " ".repeat(pad))
                    } else {
                        row
                    };
                    lines.push(Line::from(Span::styled(row, style)));
                }
            }
            {
                lib_total_rows = total_len;
                (lines, st_line)
            }
        };

        if app.show_preview
            && app.track_popup_visible
            && left_info_area.height >= info_block_h()
            && (info_sep_area.height > 0 || left_info_area.height > 0)
        {
            // Narrow + lyrics: the middle pane is given over to lyrics, so the
            // info block is repurposed to show the currently-highlighted list
            // contents (the selected row and its neighbours) instead of the
            // now-playing track card.
            if is_narrow && app.lyrics.show {
                Render::list_in_info(f, left_info_area, app);
            } else {
                Render::info_in_pane(f, info_sep_area, left_info_area, app);
            }
        }

        // On narrow/medium screens lyrics take over the results pane entirely,
        // so skip rendering the list underneath and registering hit zones for
        // rows that are not visible.
        let lyrics_results_pane = app.lyrics.show && lyrics_area.is_none();
        if !lyrics_results_pane {
            let right_para = Paragraph::new(right_lines);
            let header_label = if let Some(detail) = app.browse_detail.as_deref() {
                format!("▶ {detail}")
            } else {
                category_label.to_string()
            };
            let right_inner =
                Render::pane_header(f, panes[1], app, &header_label, !left_focus, false, true);
            fill_pane(f, right_inner, app);
            Render::evolving(f, right_inner, right_para, "lib", app, false);

            // Mouse hit zones for the visible library rows: rows start
            // below one leading blank line.
            if lib_total_rows > 0 {
                let avail = right_inner.height.saturating_sub(2) as usize;
                let visible_rows = lib_total_rows
                    .saturating_sub(app.list_scroll)
                    .min(app.viewport_items)
                    .min(avail);
                for v in 0..visible_rows {
                    let rect = Rect {
                        x: right_inner.x,
                        y: right_inner.y + 1 + v as u16,
                        width: right_inner.width,
                        height: 1,
                    };
                    app.mouse_map
                        .register(rect, MouseZone::ListItem(app.list_scroll + v));
                }
            }
        }

        {
            let stats_line = library_stats_line(app);
            if !stats_line.trim().is_empty() {
                let stats_area = Rect {
                    x: area.x + area.width.saturating_sub(stats_line.len() as u16 + 1),
                    y: area.y + area.height.saturating_sub(1),
                    width: (stats_line.len() as u16 + 1).min(area.width),
                    height: 1,
                };
                let stats_para = Paragraph::new(Span::styled(
                    stats_line,
                    Style::default().fg(app.theme.fg_dim),
                ));
                f.render_widget(stats_para, stats_area);
            }
        }

        if let Some(lyrics_area) = lyrics_area {
            Render::lyrics_pane(f, lyrics_area, app);
        } else if app.lyrics.show && !lyrics_third_pane {
            // Medium-width screens (60-99 cols): show lyrics in the results pane
            // instead of a separate third pane.
            let base = panes
                .get(1)
                .filter(|p| p.width > 1)
                .copied()
                .or_else(|| panes.iter().copied().find(|p| p.width > 1))
                .unwrap_or(chunks[1]);
            let lyrics = Rect {
                height: base.height.saturating_sub(1),
                ..base
            };
            Render::lyrics_pane(f, lyrics, app);
        }
    }

    pub(crate) fn footer(f: &mut ratatui::Frame, area: Rect, app: &mut App) {
        if app.hide_footer {
            return;
        }
        match app.input_mode {
            InputMode::Normal => {
                if !app.search_query.is_empty() {
                    f.render_widget(
                        Paragraph::new(format!(" > {}  [Esc] clear filter", app.search_query))
                            .style(Style::default().fg(app.theme.fg_bright).bg(app.chrome_bg())),
                        area,
                    );
                    return;
                }
                if app.footer_cache.suppress_refresh
                    && let Some(ref cached) = app.footer_cache.last
                {
                    footer_draw(f, area, cached);
                    Render::footer_help(f, area, app);
                    return;
                }
                let rendered = footer_render(app);
                if let Some(ref out) = rendered {
                    footer_draw(f, area, out);
                } else {
                    f.render_widget(
                        Paragraph::new("").style(Style::default().bg(app.chrome_bg())),
                        area,
                    );
                }
                app.footer_cache.last = rendered;
                Render::footer_help(f, area, app);
            }
            InputMode::Searching => {
                f.render_widget(
                    Paragraph::new(format!(" > {}_", app.search_query))
                        .style(Style::default().fg(app.theme.fg_bright).bg(app.chrome_bg())),
                    area,
                );
            }
        }
    }

    pub fn progress_variant(ratio: f64, width: usize, app: &App) -> String {
        let ratio = render_ratio(app.progress_style, ratio, app.progress_smoother.value());
        render_progress(ratio, width, app.progress_style)
    }

    pub fn progress_variant_styled<'a>(
        ratio: f64,
        width: usize,
        app: &App,
    ) -> Vec<ratatui::text::Span<'a>> {
        let ratio = render_ratio(app.progress_style, ratio, app.progress_smoother.value());
        render_progress_styled(
            ratio,
            width,
            app.progress_style,
            app.theme.accent,
            app.theme.secondary_accent,
            app.theme.tertiary_accent,
        )
    }

    pub(crate) fn lyrics_pane(f: &mut ratatui::Frame, area: Rect, app: &mut App) {
        let inner = Render::pane_header(f, area, app, "LYRICS", app.lyrics.pane_focus, false, true);
        fill_pane(f, inner, app);

        let Some(ref lyrics) = app.lyrics.current else {
            if app.lyrics.fetching {
                let mut spans = vec![Span::styled(
                    "Fetching lyrics ",
                    Style::default().fg(app.theme.accent),
                )];
                spans.push(Span::styled(
                    opencode_spinner(app.frame_count as usize),
                    Style::default()
                        .fg(app.theme.accent)
                        .add_modifier(Modifier::BOLD),
                ));
                let msg = Paragraph::new(Line::from(spans)).alignment(Alignment::Center);
                f.render_widget(msg, inner);
            } else {
                let msg = Paragraph::new("Press [l] to search")
                    .alignment(Alignment::Center)
                    .style(Style::default().fg(app.theme.fg_dim));
                f.render_widget(msg, inner);
            }
            return;
        };

        if lyrics.lines.is_empty() {
            let msg = Paragraph::new("No lyrics found")
                .alignment(Alignment::Center)
                .style(Style::default().fg(app.theme.fg_dim));
            f.render_widget(msg, inner);
            return;
        }

        let header_area = if inner.height >= 4 {
            let header_h = 1;
            let title = lyrics.title.clone().unwrap_or_else(|| {
                app.state
                    .current_track
                    .as_ref()
                    .map(|t| t.title.clone())
                    .unwrap_or_default()
            });
            let artist = lyrics.artist.clone().unwrap_or_else(|| {
                app.state
                    .current_track
                    .as_ref()
                    .map(|t| t.artist.clone())
                    .unwrap_or_default()
            });
            let album = lyrics.album.clone().unwrap_or_else(|| {
                app.state
                    .current_track
                    .as_ref()
                    .map(|t| t.album.clone())
                    .unwrap_or_default()
            });
            let header_text = if !album.is_empty() && !artist.is_empty() {
                format!("{} — {} · {}", title, artist, album)
            } else if !artist.is_empty() {
                format!("{} — {}", title, artist)
            } else {
                title.clone()
            };
            if !header_text.is_empty() {
                let header_para = Paragraph::new(Line::from(Span::styled(
                    header_text,
                    Style::default()
                        .fg(app.theme.accent)
                        .add_modifier(Modifier::BOLD),
                )))
                .alignment(Alignment::Center);
                let hdr_rect = Rect {
                    x: inner.x,
                    y: inner.y,
                    width: inner.width,
                    height: header_h,
                };
                f.render_widget(header_para, hdr_rect);
                Some(hdr_rect)
            } else {
                None
            }
        } else {
            None
        };
        let lyrics_inner = if let Some(hdr) = header_area {
            Rect {
                x: inner.x.saturating_add(1),
                y: hdr.y + hdr.height + 1,
                width: inner.width.saturating_sub(2),
                height: inner.height.saturating_sub(hdr.height + 2),
            }
        } else {
            Rect {
                x: inner.x.saturating_add(1),
                y: inner.y.saturating_add(1),
                width: inner.width.saturating_sub(2),
                height: inner.height.saturating_sub(2),
            }
        };

        Render::lyrics_body(f, lyrics_inner, app, lyrics);
    }

    /// Body of the lyrics pane: wrap the lyric lines, emphasize the active
    /// line (with karaoke word timing when available) and scroll to the
    /// anchor. Shared by the normal lyrics pane and the Zen-mode fullscreen
    /// lyrics surface so both render identically.
    pub(crate) fn lyrics_body(
        f: &mut ratatui::Frame,
        lyrics_inner: Rect,
        app: &App,
        lyrics: &LrcData,
    ) {
        let total = lyrics.lines.len();
        let width = lyrics_inner.width.max(1) as usize;
        let synced = lyrics_are_synced(&lyrics.lines);
        // User-adjustable offset applied to matching and the timestamp gutter.
        let offset = app.lyrics.offset_secs;
        let anchor = app.lyrics.scroll.min(total.saturating_sub(1));
        let mut row_offsets = Vec::with_capacity(total);
        let mut text = Vec::with_capacity(total);
        let mut cumulative = 0usize;
        for (i, line) in lyrics.lines.iter().enumerate() {
            let text_style = if !synced {
                Style::default().fg(app.theme.fg)
            } else {
                // Past lines stay readable, the active (current) line matching
                // the playback timestamp is emphasized, future lines fade out.
                let d = i as isize - anchor as isize;
                if d == 0 {
                    Style::default()
                        .fg(app.theme.accent)
                        .add_modifier(Modifier::BOLD)
                } else if d < 0 {
                    Style::default().fg(app.theme.fg)
                } else {
                    Style::default()
                        .fg(app.theme.fg_dim)
                        .add_modifier(Modifier::DIM)
                }
            };
            // Right-aligned timestamp range gutter on synced lines so the
            // actively playing verse is clearly time-bounded.
            let ts_prefix = if synced && line.timestamp >= 0.0 {
                let ts = format_duration((line.timestamp + offset).max(0.0) as u64);
                let range = match lyrics.lines.get(i + 1) {
                    Some(next) if next.timestamp >= 0.0 => {
                        format!(
                            "{ts}-{}",
                            format_duration((next.timestamp + offset).max(0.0) as u64)
                        )
                    }
                    _ => ts,
                };
                format!("  [{range}]")
            } else {
                String::new()
            };
            let row_text = format!("{}{}", line.text, ts_prefix);
            row_offsets.push(cumulative);
            cumulative += row_text.chars().count().max(1).div_ceil(width);
            let ts_style = if i == anchor && synced {
                Style::default().fg(app.theme.accent)
            } else {
                Style::default().fg(app.theme.fg_dim)
            };
            // Karaoke: the active line lights up word-by-word when the source
            // carries per-word timings (enhanced LRC). Future words stay dim.
            if i == anchor && synced && !line.words.is_empty() {
                let pos = app.raw_position + offset;
                let mut spans: Vec<Span> = Vec::with_capacity(line.words.len() + 1);
                for w in &line.words {
                    let lit = pos >= w.time;
                    spans.push(Span::styled(
                        w.text.clone(),
                        if lit {
                            Style::default()
                                .fg(app.theme.accent)
                                .add_modifier(Modifier::BOLD)
                        } else {
                            Style::default().fg(app.theme.fg_dim)
                        },
                    ));
                }
                spans.push(Span::styled(ts_prefix, ts_style));
                text.push(Line::from(spans));
            } else {
                text.push(Line::from(vec![
                    Span::styled(line.text.clone(), text_style),
                    Span::styled(ts_prefix, ts_style),
                ]));
            }
        }
        let total_rows = cumulative;
        // Untimed lyrics can't highlight: reserve the bottom row for a hint
        // instead of faking emphasis.
        let hint_area = if !synced && lyrics_inner.height > 1 {
            Some(Rect {
                x: lyrics_inner.x,
                y: lyrics_inner.y + lyrics_inner.height - 1,
                width: lyrics_inner.width,
                height: 1,
            })
        } else {
            None
        };
        let scroll_view = if hint_area.is_some() {
            Rect {
                y: lyrics_inner.y,
                height: lyrics_inner.height - 1,
                ..lyrics_inner
            }
        } else {
            lyrics_inner
        };
        let visible = scroll_view.height as usize;
        let bottom = total_rows.saturating_sub(visible);
        let scroll_display = if total_rows <= visible {
            0
        } else if app.lyrics.manual_scroll {
            if anchor == total - 1 {
                bottom
            } else {
                // Center active line: start = cur - h/2
                row_offsets[anchor].saturating_sub(visible / 2).min(bottom)
            }
        } else {
            row_offsets[anchor].saturating_sub(visible / 2).min(bottom)
        };

        let para = Paragraph::new(text)
            .wrap(Wrap { trim: false })
            .scroll((scroll_display as u16, 0));
        f.render_widget(para, scroll_view);
        if let Some(h) = hint_area {
            f.render_widget(
                Paragraph::new(Line::from(Span::styled(
                    "no timing available — focus lyrics, then [ / ] to offset",
                    Style::default().fg(app.theme.fg_dim),
                ))),
                h,
            );
        }
    }

    /// Narrow + lyrics: show the currently highlighted list row and its
    /// neighbours in the info block, so the buried middle pane's contents stay
    /// usable while lyrics take the main area. This replaces the now-playing
    /// track-info card ("l" swaps it back when lyrics are dismissed).
    pub(crate) fn list_in_info(f: &mut ratatui::Frame, area: Rect, app: &mut App) {
        let rows: Vec<&TrackInfo> = app.filtered_tracks();
        let total = rows.len();
        let sel = app.list_pos().min(total.saturating_sub(1));
        let visible = area.height.saturating_sub(2);
        let avail = area.width.saturating_sub(2) as usize;

        let mut lines = vec![Line::from(Span::styled(
            format!(" ▶ {}", app.track_sort.label()),
            Style::default()
                .fg(app.theme.accent)
                .add_modifier(Modifier::BOLD),
        ))];
        if rows.is_empty() {
            lines.push(Line::from(Span::styled(
                " No tracks",
                Style::default().fg(app.theme.fg_dim),
            )));
        } else {
            // Window so the selected row stays centered in the block.
            let rows_h = (visible.saturating_sub(1) as usize).max(1);
            let half = rows_h / 2;
            let win_start = sel.saturating_sub(half);
            let end = (win_start + rows_h).min(total);
            let avail_disp = avail.saturating_sub(2);
            for (real_i, track) in rows[win_start..end].iter().enumerate() {
                let real_i = win_start + real_i;
                let is_sel = real_i == sel;
                let is_multiselected = app.multiselect_mode && app.row_is_selected(&track.path);
                let prefix = if is_sel { " > " } else { "   " };
                let label = if track.title.is_empty() {
                    std::path::Path::new(&track.path)
                        .file_stem()
                        .map(|s| s.to_string_lossy().to_string())
                        .unwrap_or_default()
                } else {
                    track.title.clone()
                };
                let display = scroll_text(&label, avail_disp, app.footer_title_scroll, is_sel);
                let checkbox = if is_multiselected { "☑ " } else { "" };
                let row = format!(
                    "{prefix}{checkbox}{:<width$}",
                    display,
                    width = avail_disp.saturating_sub(checkbox.len())
                );
                let style = if is_sel {
                    Style::default()
                        .fg(app.theme.selection_fg_readable())
                        .bg(app.theme.selection_bg)
                } else if is_multiselected {
                    Style::default().bg(app.theme.warning).fg(app.theme.bg)
                } else {
                    Style::default().fg(app.theme.fg)
                };
                lines.push(Line::from(Span::styled(row, style)));
            }
        }
        let para = Paragraph::new(lines);
        f.render_widget(para, area);
    }

    pub(crate) fn info_in_pane(f: &mut ratatui::Frame, sep_area: Rect, area: Rect, app: &mut App) {
        let fields = match track_info_fields(app) {
            Some(fields) => fields,
            None => return,
        };
        let has_cover = fields.has_cover;
        let can_cover = !no_image_protocol() && area.width > COVER_W + 1;

        let sep_style = Style::default().fg(app.theme.fg_dim);
        let sep_label = String::new();
        f.render_widget(
            Paragraph::new(Line::from(Span::styled(sep_label, sep_style))),
            sep_area,
        );

        if can_cover {
            // Adaptive cover: use as much of the card height for cover as
            // fits while keeping at least 4 rows for the text block.
            let card_h = area.height;
            let text_need: u16 = if fields.album.is_some() { 4 } else { 3 };
            let cover_h_avail = card_h.saturating_sub(text_need).saturating_sub(2);
            let cover_h_eff = COVER_H
                .min(cover_h_avail.max(4))
                .min(area.height.saturating_sub(text_need + 2));
            let cover_w_eff = (cover_h_eff * 2).min(area.width.saturating_sub(2));
            let split = Layout::default()
                .direction(Direction::Vertical)
                .constraints([
                    Constraint::Length(cover_h_eff),
                    Constraint::Length(1),
                    Constraint::Min(0),
                ])
                .split(area);

            let cover_hpad = area.width.saturating_sub(cover_w_eff) / 2;
            let cover_area = Rect {
                x: area.x + cover_hpad,
                y: split[0].y,
                width: cover_w_eff.min(area.width),
                height: cover_h_eff,
            };
            if has_cover {
                Render::cover(
                    f,
                    cover_area,
                    app.popup_cover_stateful.as_mut(),
                    app.track_popup_cover.as_deref(),
                    app.theme.fg_dim,
                    None,
                );
            }

            let text_area = split[2];
            let pad = "  ";
            let title_avail = text_area.width.saturating_sub(pad.len() as u16) as usize;
            let animated_title = scroll_text(&fields.title, title_avail, app.np_title_scroll, true);
            let mut lines = vec![
                Line::from(Span::styled(
                    format!("{pad}{}", animated_title),
                    Style::default()
                        .fg(app.theme.fg_bright)
                        .add_modifier(Modifier::BOLD),
                )),
                Line::from(Span::styled(
                    format!("{pad}{}", fields.artist),
                    Style::default().fg(app.theme.fg),
                )),
            ];
            if let Some(album) = &fields.album {
                lines.push(Line::from(Span::styled(
                    format!("{pad}{}", album),
                    Style::default().fg(app.theme.fg),
                )));
            }
            lines.push(Line::from(Span::styled(
                format!("{pad}{}", fields.meta.trim()),
                Style::default().fg(app.theme.fg_dim),
            )));
            let para = Paragraph::new(lines);
            f.render_widget(para, text_area);
        } else {
            let title_avail = area.width.saturating_sub(2) as usize;
            let animated_title = scroll_text(&fields.title, title_avail, app.np_title_scroll, true);
            let lines = vec![
                Line::from(Span::styled(
                    format!("  {}", animated_title),
                    Style::default()
                        .fg(app.theme.fg_bright)
                        .add_modifier(Modifier::BOLD),
                )),
                Line::from(Span::styled(
                    format!("  {}", fields.artist),
                    Style::default().fg(app.theme.fg),
                )),
                Line::from(if let Some(album) = &fields.album {
                    Span::styled(format!("  {}", album), Style::default().fg(app.theme.fg))
                } else {
                    Span::raw("")
                }),
                Line::from(""),
                Line::from(Span::styled(
                    fields.meta.clone(),
                    Style::default().fg(app.theme.fg_dim),
                )),
            ];
            let para = Paragraph::new(lines);
            f.render_widget(para, area);
        }
    }

    pub(crate) fn loader(f: &mut ratatui::Frame, area: Rect, app: &App, label: &str) {
        if area.width == 0 || area.height == 0 {
            return;
        }
        let mut lines: Vec<Line> = Vec::new();
        for _ in 0..(area.height as usize / 2).saturating_sub(1) {
            lines.push(Line::from(""));
        }
        lines.push(Line::from(Span::styled(
            opencode_spinner(app.frame_count as usize),
            Style::default()
                .fg(app.theme.accent)
                .add_modifier(Modifier::BOLD),
        )));
        lines.push(Line::from(Span::styled(
            label.to_string(),
            Style::default().fg(app.theme.fg_dim),
        )));
        let para = Paragraph::new(lines).alignment(Alignment::Center);
        f.render_widget(para, area);
    }

    pub(crate) fn health_panel(f: &mut ratatui::Frame, area: Rect, app: &App) {
        let panel_width = area.width.min(60);
        let panel_height = area.height.min(20);
        let x = (area.width.saturating_sub(panel_width)) / 2;
        let y = (area.height.saturating_sub(panel_height)) / 2;
        let rect = Rect::new(area.x + x, area.y + y, panel_width, panel_height);

        f.render_widget(Clear, rect);

        let block = Block::default()
            .title(Line::from(Span::styled(
                " Health Check ",
                Style::default()
                    .fg(app.theme.accent)
                    .add_modifier(Modifier::BOLD),
            )))
            .borders(Borders::ALL)
            .style(Style::default().fg(app.theme.fg).bg(app.float_bg()));

        let inner = block.inner(rect);
        f.render_widget(block, rect);

        if let Some(ref report) = app.health_report {
            let mut lines = Vec::new();
            lines.push(Line::from(vec![
                Span::styled(
                    format!("Daemon v{}", report.version),
                    Style::default()
                        .fg(app.theme.accent)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    format!("  uptime {}", format_uptime(report.daemon_uptime_secs)),
                    Style::default().fg(app.theme.fg_dim),
                ),
            ]));
            lines.push(Line::from(""));

            for c in &report.components {
                let (icon, color) = match c.status {
                    HealthStatus::Ok => ("✓", app.theme.success),
                    HealthStatus::Degraded => ("⚠", app.theme.warning),
                    HealthStatus::Error => ("✗", app.theme.error),
                };
                let mut spans = vec![
                    Span::styled(format!(" {icon} "), Style::default().fg(color)),
                    Span::styled(
                        c.name.clone(),
                        Style::default()
                            .fg(app.theme.fg_bright)
                            .add_modifier(Modifier::BOLD),
                    ),
                ];
                if let Some(ref msg) = c.message {
                    spans.push(Span::styled(
                        format!(": {msg}"),
                        Style::default().fg(app.theme.fg_dim),
                    ));
                }
                lines.push(Line::from(spans));
            }

            let para = Paragraph::new(lines).scroll((0, 0));
            f.render_widget(para, inner);
        } else {
            let loading =
                Paragraph::new(" Loading...").style(Style::default().fg(app.theme.fg_dim));
            f.render_widget(loading, inner);
        }

        let help = Paragraph::new("").style(Style::default().fg(app.theme.fg_dim));
        let help_area = Rect::new(
            rect.x,
            rect.y + rect.height.saturating_sub(1),
            rect.width,
            1,
        );
        f.render_widget(help, help_area);
    }
}

pub fn run_tui(
    socket: Option<String>,
    setup_service: Option<String>,
) -> Result<(), Box<dyn std::error::Error>> {
    let socket_path = socket
        .map(PathBuf::from)
        .unwrap_or_else(resolve_command_socket);

    let _original_stderr = redirect_stderr();

    let rt = tokio::runtime::Runtime::new()?;
    rt.block_on(async {
        color_eyre::install()?;

        ensure_daemon_running(&socket_path).await?;

        enable_raw_mode()?;
        let mut stdout = std::io::stdout();
        crossterm::execute!(
            stdout,
            EnterAlternateScreen,
            EnableBracketedPaste,
            EnableMouseCapture
        )?;
        let mut terminal = Terminal::new(CrosstermBackend::new(stdout))?;
        terminal.clear()?;

        let panic_hook = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |panic| {
            let _ = disable_raw_mode();
            let mut stdout = std::io::stdout();
            let _ = crossterm::execute!(
                stdout,
                DisableMouseCapture,
                DisableBracketedPaste,
                LeaveAlternateScreen
            );
            panic_hook(panic);
        }));

        let res = async {
            let app = App::new(&socket_path, setup_service).await?;
            app.run(&mut terminal).await
        }
        .await;

        let _ = disable_raw_mode();
        let mut stdout = std::io::stdout();
        let _ = crossterm::execute!(
            stdout,
            DisableMouseCapture,
            DisableBracketedPaste,
            LeaveAlternateScreen
        );

        res
    })
}

pub fn render(f: &mut ratatui::Frame, app: &mut App) {
    let area = f.area();
    app.mouse_map.clear();
    if area.width < 20 || area.height < 6 {
        let msg = Paragraph::new("Terminal too small (min 20x6)")
            .alignment(Alignment::Center)
            .style(Style::default().fg(app.theme.fg_dim));
        f.render_widget(msg, area);
        return;
    }
    // Zen mode: exactly one fullscreen surface at a time — no chrome,
    // no footer, no pickers.
    if app.zen {
        Render::zen(f, area, app);
        app.track_anim_trigger = false;
        return;
    }
    f.render_widget(
        ratatui::widgets::Block::default()
            .style(ratatui::style::Style::default().bg(app.surface_bg())),
        area,
    );
    let footer_height = if app.hide_footer { 0 } else { 1 };
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(0), Constraint::Length(footer_height)])
        .split(area);

    Render::content(f, chunks[0], app);
    if !app.hide_footer {
        Render::footer(f, chunks[1], app);
    }

    // "gtm" brand badge pinned to the top-right corner with the themed
    // accent background (restored from the pre-tabless UI).
    let brand_w: u16 = 7.min(chunks[0].width);
    let brand = Paragraph::new(Span::styled(
        "  gtm  ",
        Style::default()
            .fg(readable_fg(app.theme.fg, app.theme.accent))
            .bg(app.theme.accent)
            .add_modifier(Modifier::BOLD),
    ));
    f.render_widget(
        brand,
        Rect {
            x: chunks[0].right().saturating_sub(brand_w),
            y: chunks[0].y,
            width: brand_w,
            height: 1,
        },
    );

    if app.pickers.is_open() {
        dim_background(f, area);
        Pickers::render_picker(f, area, app);
    }

    Render::notification_overlay(f, area, app);

    // Render pending prompt if any
    Render::pending_prompt(f, area, app);

    if app.show_health_panel {
        Render::health_panel(f, area, app);
    }

    app.track_anim_trigger = false;
}

impl Render {
    pub fn pending_prompt(f: &mut ratatui::Frame, area: Rect, app: &mut App) {
        render_pending_prompt(f, area, app);
    }
}

pub(crate) fn render_pending_prompt(f: &mut ratatui::Frame, area: Rect, app: &App) {
    let Some(prompt) = &app.pending_prompt else {
        return;
    };
    let theme = &app.theme;
    // Dim the background
    dim_background(f, area);

    // Calculate prompt area
    let prompt_w = (area.width * 3 / 4).clamp(40, 60);
    let prompt_h = 5u16;
    let prompt_x = (area.width - prompt_w) / 2;
    let prompt_y = (area.height - prompt_h) / 2;

    let prompt_area = Rect {
        x: prompt_x,
        y: prompt_y,
        width: prompt_w,
        height: prompt_h,
    };

    // Background
    let bg = Block::default()
        .style(Style::default().bg(theme.elevated_bg))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme.accent));
    f.render_widget(bg, prompt_area);

    let inner = prompt_area.inner(Margin {
        horizontal: 2,
        vertical: 1,
    });

    // Message
    let lines = vec![
        Line::from(Span::styled(
            prompt.message.clone(),
            Style::default().fg(theme.fg_bright),
        )),
        Line::from(""),
        Line::from(Span::styled(
            "[y] Yes  [n] No  [Esc] Cancel",
            Style::default().fg(theme.fg_dim),
        )),
    ];

    let para = Paragraph::new(lines).alignment(Alignment::Center);
    f.render_widget(para, inner);
}
