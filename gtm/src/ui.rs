// Copyright (c) 2026
// Author: prjctimg <prjctimg@outlook.com>
// TUI rendering: layout, widgets, and theme application
//
// This is free software released under the GPL-3.0 license.

use std::borrow::Cow;
use std::path::PathBuf;

use tokio::io::{AsyncReadExt, AsyncWriteExt};

use crate::app::{
    App, InputMode, LIBRARY_CATEGORIES, LibraryPick, NotificationKind, TrackInfoKind,
    no_image_protocol,
};
use crate::footer::format_duration;
use crate::picker::{Picker, PickerId, PickerSource};
use crossterm::event::{
    DisableBracketedPaste, DisableMouseCapture, EnableBracketedPaste, EnableMouseCapture,
};
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use gtm_core::global::EqPreset;
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Alignment, Constraint, Direction, Layout, Margin, Rect};
use ratatui::style::Color;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, Padding, Paragraph, Wrap};
use ratatui_image::StatefulImage;
use ratatui_image::protocol::StatefulProtocol;

/// Grouped render helpers: previously free `render_*` functions.
pub struct Render;

impl Render {
    fn upnext_card(f: &mut ratatui::Frame, area: Rect, app: &mut App) {
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
            let source = if u.track.path.contains("/audio/spotify")
                || u.track.path.starts_with("spotify:")
            {
                "Spotify"
            } else if u.track.path.contains("/audio/youtube")
                || u.track.path.starts_with("youtube:")
            {
                "YouTube"
            } else {
                "Local"
            };
            let source_label = if use_nerd_fonts() {
                match source {
                    "Spotify" => " \u{f1bc} Spotify",
                    "YouTube" => " \u{f167} YouTube",
                    _ => " \u{f3b5} Local",
                }
            } else {
                match source {
                    "Spotify" => " ♫ Spotify",
                    "YouTube" => " ▶ YouTube",
                    _ => " ♪ Local",
                }
            };
            (
                display_title,
                artist,
                album,
                has_album,
                has_cover,
                source_label.to_string(),
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
                if let Some(protocol) = app.upnext.as_mut().and_then(|u| u.cover_stateful.as_mut())
                {
                    let image = StatefulImage::new();
                    f.render_stateful_widget(image, cover_area, protocol);
                } else if let Some(bytes) = app.upnext.as_ref().and_then(|u| u.cover.as_ref()) {
                    Render::cover_block(f, cover_area, bytes);
                } else {
                    Render::cover(f, cover_area, None, None, app.theme.fg_dim);
                }
            } else {
                Render::cover(f, cover_area, None, None, app.theme.fg_dim);
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

    fn notification_overlay(f: &mut ratatui::Frame, area: Rect, app: &mut App) {
        let now = std::time::Instant::now();

        // Absolutely never draw cover art (or any floating card) on top of an
        // active picker/floating window. With a picker on screen we skip the
        // whole overlay so images can't leak over it.
        if app.pickers.is_open() {
            return;
        }

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

        let mut y_bottom = area.y + area.height - padding;

        if app.upnext.is_some() {
            let card_w = 42u16;
            let card_h = 7u16;
            let card_x = area.x + area.width.saturating_sub(card_w + padding);
            let card_y = y_bottom.saturating_sub(card_h);
            if card_y >= area.y {
                let card_area = Rect {
                    x: card_x,
                    y: card_y,
                    width: card_w,
                    height: card_h,
                };
                Render::upnext_card(f, card_area, app);
                y_bottom = card_y.saturating_sub(gap);
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

            let card_y = y_bottom.saturating_sub(card_h);
            if card_y < area.y {
                break;
            }

            let final_x = area.x + area.width.saturating_sub(max_notif_width + padding);
            let start_x = area.x + area.width;
            let leaving = now.saturating_duration_since(n.expires_at);
            let x = if leaving > std::time::Duration::ZERO {
                let exit_progress = cubic_ease_in(
                    (leaving.as_millis() as f32 / NOTIFICATION_EXIT_DURATION.as_millis() as f32)
                        .min(1.0),
                );
                (final_x as f32 + (start_x as f32 - final_x as f32) * exit_progress) as u16
            } else {
                let progress = cubic_ease_out(n.animation_progress);
                (start_x as f32 + (final_x as f32 - start_x as f32) * progress) as u16
            };

            let card_area = Rect {
                x,
                y: card_y,
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

            y_bottom = card_y.saturating_sub(gap);
        }

        for n in volume.iter() {
            let bar_h = 10u16;
            let bar_w = 5u16;

            let final_x = area.x + area.width.saturating_sub(bar_w + padding);
            let start_x = area.x + area.width;
            let leaving = now.saturating_duration_since(n.expires_at);
            let x = if leaving > std::time::Duration::ZERO {
                let exit_progress = cubic_ease_in(
                    (leaving.as_millis() as f32 / NOTIFICATION_EXIT_DURATION.as_millis() as f32)
                        .min(1.0),
                );
                (final_x as f32 + (start_x as f32 - final_x as f32) * exit_progress) as u16
            } else {
                let progress = cubic_ease_out(n.animation_progress);
                (start_x as f32 + (final_x as f32 - start_x as f32) * progress) as u16
            };

            let bar_y = y_bottom.saturating_sub(bar_h + 2);
            if bar_y < area.y {
                break;
            }
            let bar_area = Rect {
                x,
                y: bar_y,
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

            y_bottom = bar_y.saturating_sub(gap);
        }
    }

    fn content(f: &mut ratatui::Frame, area: Rect, app: &mut App) {
        if !app.is_ready {
            fill_pane(f, area, app);
            Render::loader(f, area, app, "Loading library\u{2026}");
            return;
        }
        Render::library(f, area, app);
    }

    fn footer_help(f: &mut ratatui::Frame, area: Rect, app: &App) {
        if app.pickers.is_open() || app.hide_help_bar {
            return;
        }
        let text = " [?] Help  [:] Command palette  [q] Quit ";
        let para = Paragraph::new(text)
            .alignment(Alignment::Right)
            .style(Style::default().fg(app.theme.fg_dim).bg(app.chrome_bg()));
        f.render_widget(para, area);
    }

    fn cover(
        f: &mut ratatui::Frame,
        area: Rect,
        cover_stateful: Option<&mut StatefulProtocol>,
        current_cover: Option<&[u8]>,
        placeholder_fg: Color,
    ) {
        if std::env::var("NVIM").is_ok() || std::env::var("ZELLIJ").is_ok() {
            let placeholder = Paragraph::new(Span::styled(
                " \u{266b} Cover art unavailable in this terminal ",
                Style::default().fg(placeholder_fg),
            ));
            f.render_widget(placeholder, area);
            return;
        }
        if let Some(protocol) = cover_stateful {
            let image = StatefulImage::new();
            f.render_stateful_widget(image, area, protocol);
        } else if let Some(cover_bytes) = current_cover {
            Render::cover_block(f, area, cover_bytes);
        } else {
            let placeholder = Paragraph::new(Span::styled(
                " \u{266b} ",
                Style::default().fg(placeholder_fg),
            ));
            f.render_widget(placeholder, area);
        }
    }

    fn cover_block(f: &mut ratatui::Frame, area: Rect, cover_bytes: &[u8]) {
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

    fn evolving<W: ratatui::widgets::Widget>(
        f: &mut ratatui::Frame,
        area: Rect,
        widget: W,
        key: &'static str,
        app: &mut App,
        animate_on_track_change: bool,
    ) {
        // Dust/thanos-style evolve-into is reserved for genuine auto-advances;
        // a manual Next/Prev shouldn't dissolve the pane. (First frame still
        // evolves so the startup animation is preserved.)
        let start = app.track_anim_trigger
            && app.auto_track_advance
            && (animate_on_track_change || app.frame_count == 0);
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

    fn pane_header(
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

    fn library(f: &mut ratatui::Frame, area: Rect, app: &mut App) {
        let is_narrow = app.terminal_cols < 60;
        // Small-height terminals: compress the Now Playing section to a
        // side-by-side cover + details row so it can never crowd out the
        // list panes, and drop the extra track-info card in the left pane.
        let is_small_height = app.terminal_rows < 22;
        let show_vis = app.visualizer.is_enabled() && app.terminal_cols >= 80;
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

        let lyrics_takes_full_height = app.show_lyrics && !is_narrow;

        let (left_area, lyrics_area) = if lyrics_takes_full_height {
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

                let display_title = if track.title.is_empty() {
                    std::path::Path::new(&track.path)
                        .file_stem()
                        .map(|s| s.to_string_lossy().to_string())
                        .unwrap_or_default()
                } else {
                    track.title.clone()
                };
                let display_artist = if track.artist.is_empty() {
                    " "
                } else {
                    &track.artist
                };
                let has_album = !track.album.is_empty();

                // Progress: 1 row (available when dur > 0)
                let dur = if app.state.duration > 0.0 {
                    app.state.duration as u64
                } else {
                    track.duration as u64
                };
                let has_progress = dur > 0;

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
                        let bar_w = 14usize;
                        let progress_str = crate::ui::Render::progress_variant(ratio, bar_w, app);
                        let time_str = format!(
                            " {} / {}",
                            crate::footer::format_duration(pos),
                            crate::footer::format_duration(dur)
                        );
                        let prog_para = Paragraph::new(Line::from(vec![
                            Span::styled(
                                progress_str,
                                Style::default().fg(app.theme.secondary_accent),
                            ),
                            Span::styled(time_str, Style::default().fg(app.theme.fg_dim)),
                        ]));
                        f.render_widget(prog_para, info_chunks[info_row]);
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
                app.state.status == gtm_core::global::PlaybackStatus::Playing,
                vis_a.width,
                &app.state.audio_levels,
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
        let left_items: Vec<ListItem> = LIBRARY_CATEGORIES
            .iter()
            .enumerate()
            .map(|(i, cat)| {
                let icon = lib_icons.get(i).unwrap_or(&" ");
                let count = match *cat {
                    "All Tracks" => app.tracks_cache.len(),
                    "Liked" => app.tracks_cache.iter().filter(|t| t.favourite).count(),
                    "Albums" => app.unique_albums().len(),
                    "Artists" => app.unique_artists().len(),
                    "Playlists" => app.playlist_cache.len(),
                    "Spotify" => app.spotify_playlists.len(),
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

        let track_info_h: u16 = if app.track_popup_visible && !is_small_height {
            let avail_h = left_inner.height.saturating_sub(1);
            let need = track_info_block_height();
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
                Constraint::Length(if app.track_popup_visible && !is_small_height {
                    1
                } else {
                    0
                }),
                Constraint::Length(track_info_h),
            ])
            .split(left_inner);
        let left_list_area = left_vchunks[0];
        let left_track_info_sep_area = left_vchunks[1];
        let left_track_info_area = left_vchunks[2];

        f.render_widget(List::new(left_items), left_list_area);

        if app.library_category < LIBRARY_CATEGORIES.len() {
            let indicator_y = left_list_area.y + app.library_category as u16;
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
            let tracks = &app.spotify_playlist_tracks_cache;
            let total_len = tracks.len();
            let st_line = format!(" {} {} ", total_len, plural(total_len, "track", "tracks"));
            let reserve = 3usize;
            let available = panes[1].height.saturating_sub(reserve as u16) as usize;
            app.viewport_items = available;
            let sel = app.list_pos().min(total_len.saturating_sub(1));
            let (list_scroll, end) = step_viewport(app.list_scroll, sel, available, total_len);
            app.list_scroll = list_scroll;

            let pane_w = panes[1].width as usize;
            let mut lines = vec![Line::from("")];
            if tracks.is_empty() {
                lines.push(Line::from(Span::styled(
                    " No tracks: run Settings > Spotify > Sync Now, then press Enter again",
                    Style::default().fg(app.theme.fg_dim),
                )));
                lib_total_rows = total_len;
                (lines, st_line)
            } else {
                for (i, tr) in tracks[app.list_scroll..end].iter().enumerate() {
                    let real_i = app.list_scroll + i;
                    let is_sel = real_i == sel && !left_focus;
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
                    let head = format!("{prefix}{:<width$}", display_label, width = name_pad);
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
                    let label = track.title.clone();
                    let avail = pane_w.saturating_sub(2);
                    let display_label = scroll_text(&label, avail, app.footer_title_scroll, is_sel);
                    let prefix = if is_current { "\u{25b6} " } else { "  " };
                    let row = format!("{}{}", prefix, display_label);
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
            let playlists = &app.spotify_playlists;
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
            for (i, track) in filtered[app.list_scroll..end].iter().enumerate() {
                let real_i = app.list_scroll + i;
                let is_current = app.state.current_track.as_ref().map(|t| t.id) == Some(track.id);
                let is_sel = real_i == sel && !left_focus;
                let label = track.title.clone();
                let avail = pane_w.saturating_sub(2);
                let display_label = scroll_text(&label, avail, app.footer_title_scroll, is_sel);
                let prefix = if is_current { "\u{25b6} " } else { "  " };
                let row = format!("{}{}", prefix, display_label);
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
        };

        if app.track_popup_visible
            && left_track_info_area.height >= track_info_block_height()
            && (left_track_info_sep_area.height > 0 || left_track_info_area.height > 0)
        {
            // Narrow + lyrics: the middle pane is given over to lyrics, so the
            // info block is repurposed to show the currently-highlighted list
            // contents (the selected row and its neighbours) instead of the
            // now-playing track card.
            if is_narrow && app.show_lyrics {
                Render::highlighted_list_in_info(f, left_track_info_area, app);
            } else {
                Render::track_info_in_pane(f, left_track_info_sep_area, left_track_info_area, app);
            }
        }

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
                    .register(rect, crate::mouse::MouseZone::ListItem(app.list_scroll + v));
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
        } else if app.show_lyrics && is_narrow {
            // Narrow screens: lyrics act as a tab replacing the list/queue
            // area entirely ('l' toggles, Esc/Back returns to the list).
            let base = panes
                .iter()
                .copied()
                .find(|p| p.width > 1)
                .unwrap_or(chunks[1]);
            let lyrics = Rect {
                height: base.height.saturating_sub(1),
                ..base
            };
            Render::lyrics_pane(f, lyrics, app);
        }
    }

    fn footer(f: &mut ratatui::Frame, area: Rect, app: &mut App) {
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
                    crate::footer::draw(f, area, cached);
                    Render::footer_help(f, area, app);
                    return;
                }
                let rendered = crate::footer::render(app);
                if let Some(ref out) = rendered {
                    crate::footer::draw(f, area, out);
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
        let ratio =
            crate::progress::render_ratio(app.progress_style, ratio, app.progress_smoother.value());
        crate::progress::render_progress(ratio, width, app.progress_style)
    }

    pub fn progress_variant_styled<'a>(
        ratio: f64,
        width: usize,
        app: &App,
    ) -> Vec<ratatui::text::Span<'a>> {
        let ratio =
            crate::progress::render_ratio(app.progress_style, ratio, app.progress_smoother.value());
        crate::progress::render_progress_styled(
            ratio,
            width,
            app.progress_style,
            app.theme.accent,
            app.theme.secondary_accent,
            app.theme.tertiary_accent,
        )
    }

    fn lyrics_pane(f: &mut ratatui::Frame, area: Rect, app: &mut App) {
        let inner = Render::pane_header(f, area, app, "LYRICS", app.lyrics_pane_focus, false, true);
        fill_pane(f, inner, app);

        let Some(ref lyrics) = app.current_lyrics else {
            if app.lyrics_fetching {
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
                x: inner.x,
                y: hdr.y + hdr.height,
                width: inner.width,
                height: inner.height.saturating_sub(hdr.height + 1),
            }
        } else {
            inner
        };

        let total = lyrics.lines.len();
        let width = lyrics_inner.width.max(1) as usize;
        let synced = crate::app::lyrics_are_synced(&lyrics.lines);
        let anchor = app.lyrics_scroll.min(total - 1);
        let mut row_offsets = Vec::with_capacity(total);
        let mut text = Vec::with_capacity(total);
        let mut cumulative = 0usize;
        for (i, line) in lyrics.lines.iter().enumerate() {
            let text_style = if !synced {
                Style::default().fg(app.theme.fg)
            } else {
                let d = i as isize - anchor as isize;
                if d == 0 {
                    Style::default()
                        .fg(app.theme.accent)
                        .add_modifier(Modifier::BOLD)
                } else if d < 0 {
                    Style::default().fg(app.theme.fg_dim)
                } else if d <= 2 {
                    Style::default().fg(app.theme.fg)
                } else {
                    Style::default().fg(app.theme.fg_dim)
                }
            };
            row_offsets.push(cumulative);
            cumulative += (line.text.chars().count().max(1)).div_ceil(width);
            text.push(Line::from(Span::styled(line.text.clone(), text_style)));
        }
        let total_rows = cumulative;
        let visible = lyrics_inner.height as usize;
        let bottom = total_rows.saturating_sub(visible);
        let scroll_display = if total_rows <= visible {
            0
        } else if app.lyrics_manual_scroll {
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
        f.render_widget(para, lyrics_inner);
    }

    /// Narrow + lyrics: show the currently highlighted list row and its
    /// neighbours in the info block, so the buried middle pane's contents stay
    /// usable while lyrics take the main area. This replaces the now-playing
    /// track-info card ("l" swaps it back when lyrics are dismissed).
    fn highlighted_list_in_info(f: &mut ratatui::Frame, area: Rect, app: &mut App) {
        let rows: Vec<&gtm_core::track::TrackInfo> = app.filtered_tracks();
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
                let row = format!("{prefix}{:<width$}", display, width = avail_disp);
                let style = if is_sel {
                    Style::default()
                        .fg(app.theme.selection_fg_readable())
                        .bg(app.theme.selection_bg)
                } else {
                    Style::default().fg(app.theme.fg)
                };
                lines.push(Line::from(Span::styled(row, style)));
            }
        }
        let para = Paragraph::new(lines);
        f.render_widget(para, area);
    }

    fn track_info_in_pane(f: &mut ratatui::Frame, sep_area: Rect, area: Rect, app: &mut App) {
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
                if let Some(ref mut protocol) = app.popup_cover_stateful {
                    let image = StatefulImage::new();
                    f.render_stateful_widget(image, cover_area, protocol);
                } else if let Some(ref cover_bytes) = app.track_popup_cover {
                    Render::cover_block(f, cover_area, cover_bytes);
                }
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

    fn loader(f: &mut ratatui::Frame, area: Rect, app: &App, label: &str) {
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

    fn health_panel(f: &mut ratatui::Frame, area: Rect, app: &App) {
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
                    format!(
                        "  uptime {}",
                        crate::footer::format_uptime(report.daemon_uptime_secs)
                    ),
                    Style::default().fg(app.theme.fg_dim),
                ),
            ]));
            lines.push(Line::from(""));

            for c in &report.components {
                let (icon, color) = match c.status {
                    gtm_core::ipc::HealthStatus::Ok => ("✓", app.theme.success),
                    gtm_core::ipc::HealthStatus::Degraded => ("⚠", app.theme.warning),
                    gtm_core::ipc::HealthStatus::Error => ("✗", app.theme.error),
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

        let help = Paragraph::new(" [Esc] Close").style(Style::default().fg(app.theme.fg_dim));
        let help_area = Rect::new(
            rect.x,
            rect.y + rect.height.saturating_sub(1),
            rect.width,
            1,
        );
        f.render_widget(help, help_area);
    }
}

pub fn run_tui(socket: Option<String>) -> Result<(), Box<dyn std::error::Error>> {
    let socket_path = socket
        .map(PathBuf::from)
        .unwrap_or_else(gtm_core::resolve_command_socket);

    let _original_stderr = gtm_core::log::redirect_stderr_to_log();

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
            let app = App::new(&socket_path).await?;
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

async fn ensure_daemon_running(
    socket_path: &std::path::Path,
) -> Result<(), Box<dyn std::error::Error>> {
    if socket_path.exists() {
        if let Ok(mut stream) = tokio::net::UnixStream::connect(socket_path).await {
            let ping = serde_json::to_string(&gtm_core::ipc::WireReq {
                id: 0,
                cmd: "ping".to_string(),
                params: serde_json::to_value(gtm_core::ipc::DaemonReq::Ping).unwrap(),
            })? + "\n";
            let _ = stream.write_all(ping.as_bytes()).await;
            let mut buf = [0u8; 256];
            if let Ok(Ok(n)) =
                tokio::time::timeout(std::time::Duration::from_millis(100), stream.read(&mut buf))
                    .await
                && n > 0
            {
                return Ok(());
            }
        }
        let _ = std::fs::remove_file(socket_path);
    }

    if let Some(parent) = socket_path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }

    let gtmd_path = find_gtmd_binary()?;
    let socket_arg = format!("--socket={}", socket_path.display());

    let mut child = std::process::Command::new(&gtmd_path)
        .arg(&socket_arg)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|e| format!("Failed to start gtmd at {gtmd_path:?}: {e}"))?;

    std::thread::spawn(move || {
        let _ = child.wait();
    });

    for _ in 0..120 {
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        if socket_path.exists()
            && let Ok(mut stream) = tokio::net::UnixStream::connect(socket_path).await
        {
            let ping = serde_json::to_string(&gtm_core::ipc::WireReq {
                id: 0,
                cmd: "ping".to_string(),
                params: serde_json::to_value(gtm_core::ipc::DaemonReq::Ping).unwrap(),
            })? + "\n";
            let _ = stream.write_all(ping.as_bytes()).await;
            let mut buf = [0u8; 256];
            if let Ok(Ok(n)) =
                tokio::time::timeout(std::time::Duration::from_millis(500), stream.read(&mut buf))
                    .await
                && n > 0
            {
                return Ok(());
            }
        }
    }

    Ok(())
}

fn find_gtmd_binary() -> Result<std::path::PathBuf, Box<dyn std::error::Error>> {
    if let Ok(exe) = std::env::current_exe()
        && let Some(parent) = exe.parent()
    {
        let candidate = parent.join("gtmd");
        if candidate.exists() {
            return Ok(candidate);
        }
    }

    if let Ok(paths) = std::env::var("PATH") {
        for dir in std::env::split_paths(&paths) {
            let candidate = dir.join("gtmd");
            if candidate.exists() {
                return Ok(candidate);
            }
        }
    }

    let candidate = std::path::PathBuf::from("/usr/bin/gtmd");
    if candidate.exists() {
        return Ok(candidate);
    }

    Err("gtmd binary not found".into())
}

// ─── Layout ───

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
    f.render_widget(
        ratatui::widgets::Block::default()
            .style(ratatui::style::Style::default().bg(app.surface_bg())),
        area,
    );
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(0), Constraint::Length(1)])
        .split(area);

    Render::content(f, chunks[0], app);
    Render::footer(f, chunks[1], app);

    // "gtm" brand badge pinned to the top-right corner with the themed
    // accent background (restored from the pre-tabless UI).
    let brand_w: u16 = 7.min(chunks[0].width);
    let brand = Paragraph::new(Span::styled(
        "  gtm  ",
        Style::default()
            .fg(crate::theme::readable_fg(app.theme.fg, app.theme.accent))
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

    if app.show_health_panel {
        Render::health_panel(f, area, app);
    }

    app.track_anim_trigger = false;
}

fn cubic_ease_out(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    1.0 - (1.0 - t).powi(3)
}

fn cubic_ease_in(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    t * t * t
}

fn dim_color(c: Color) -> Color {
    match c {
        Color::Rgb(r, g, b) => Color::Rgb(
            (r as f32 * 0.7) as u8,
            (g as f32 * 0.7) as u8,
            (b as f32 * 0.7) as u8,
        ),
        other => other,
    }
}

fn dim_background(f: &mut ratatui::Frame, area: Rect) {
    for y in area.top()..area.bottom() {
        for x in area.left()..area.right() {
            if let Some(cell) = f.buffer_mut().cell_mut((x, y)) {
                let fg = dim_color(cell.fg);
                let bg = dim_color(cell.bg);
                cell.set_fg(fg);
                cell.set_bg(bg);
            }
        }
    }
}

const NOTIFICATION_LIFETIME: std::time::Duration = std::time::Duration::from_millis(1500);

const NOTIFICATION_EXIT_DURATION: std::time::Duration = std::time::Duration::from_millis(300);

fn wrap_text(text: &str, max_chars: usize) -> Vec<String> {
    if max_chars == 0 {
        return vec![text.to_string()];
    }
    let mut lines = Vec::new();
    let mut current = String::new();
    for word in text.split(' ') {
        if current.len() + word.len() + 1 > max_chars && !current.is_empty() {
            lines.push(std::mem::take(&mut current));
        }
        if !current.is_empty() {
            current.push(' ');
        }
        current.push_str(word);
    }
    if !current.is_empty() {
        lines.push(current);
    }
    if lines.is_empty() {
        lines.push(String::new());
    }
    lines
}

// ─── Content Area ───

const LIBRARY_ICONS_NERD: &[&str] = &[
    "\u{f001}", "\u{f004}", "\u{f025}", "\u{f007}", "\u{f03a}", "\u{f1bc}",
];

const LIBRARY_ICONS_ASCII: &[&str] = &["♫", "♥", "▤", "♪", "≡", "☊"];

pub(crate) fn use_nerd_fonts() -> bool {
    !matches!(std::env::var("GTM_NERD_FONTS"), Ok(v) if v == "0" || v == "false" || v == "no")
}

pub const COVER_W: u16 = 24;
pub const COVER_H: u16 = 12;

fn row_pad(content: &str, width: u16) -> usize {
    (width as usize).saturating_sub(content.chars().count())
}

fn cursor_span_style(app: &App) -> Option<Style> {
    let phase = (app.frame_count % 64) as f32 / 64.0;
    let t = (1.0 - (phase * std::f32::consts::TAU).cos()) * 0.5;
    let bg = crate::theme::blend_colors(app.theme.selection_bg, app.float_bg(), (t * 0.85) as f64);
    Some(
        Style::default()
            .fg(app.theme.selection_fg_readable())
            .bg(bg),
    )
}

fn step_viewport(offset: usize, sel: usize, visible: usize, total: usize) -> (usize, usize) {
    if total <= visible || visible == 0 {
        return (0, total);
    }
    let max_start = total - visible;
    let mut o = offset.min(max_start);
    if sel < o {
        o = sel;
    } else if sel >= o + visible {
        o = (sel + 1).saturating_sub(visible).min(max_start);
    }
    (o, (o + visible).min(total))
}

fn fill_pane(f: &mut ratatui::Frame, area: Rect, app: &App) {
    f.render_widget(
        ratatui::widgets::Block::default()
            .style(ratatui::style::Style::default().bg(app.pane_surface_bg())),
        area,
    );
}

const SETTINGS_ICONS_NERD: &[&str] = &["\u{f16a}", "\u{f04b}", "\u{f013}", "\u{f1bc}"];
const SETTINGS_ICONS_ASCII: &[&str] = &["YT", "▶", "⚙", "★"];
const SETTINGS_CATEGORIES: &[&str] = &["YouTube", "Playback", "System", "Spotify"];

// ─── Overlay Rendering ───

pub(crate) struct Pickers;

impl Pickers {
    fn picker_content_hint(top: &Picker, app: &App) -> (u16, u16) {
        match top.id {
            PickerId::Queue => {
                let w = app
                    .queue_cache
                    .iter()
                    .map(|t| t.artist.len() as u16 + t.title.len() as u16 + 14)
                    .max()
                    .unwrap_or(46)
                    .clamp(44, 72);
                let h = (app.queue_cache.len() as u16 + 6).clamp(18, 30);
                (w, h)
            }
            PickerId::YTSearch => {
                let w = app
                    .yt_results_cache
                    .iter()
                    .map(|r| {
                        let a = r.artist.as_deref().map(|a| a.len()).unwrap_or(0);
                        (a + r.title.len() + 22) as u16
                    })
                    .max()
                    .unwrap_or(52)
                    .clamp(48, 84);
                let h = (app.yt_results_cache.len() as u16 + 6).clamp(20, 32);
                (w, h)
            }
            PickerId::SearchLibrary => {
                let n = app.search_library_picks().len();
                let w = app
                    .tracks_cache
                    .iter()
                    .map(|t| t.artist.len() as u16 + t.title.len() as u16 + 14)
                    .max()
                    .unwrap_or(46)
                    .clamp(44, 72);
                let h = (n as u16 + 12).clamp(24, 34);
                (w, h)
            }
            PickerId::ThemePicker => (58, 24),
            PickerId::CommandPalette => (46, 18),
            PickerId::PlaylistSelect => (48, 20),
            PickerId::PlaylistTrackSelect => (64, 26),
            PickerId::SpotifySearch => (60, 28),
            PickerId::SpotifyLink => (60, 12),
            PickerId::Crossfade => (58, 20),
            PickerId::VisualizerPreset => (48, 14),
            PickerId::FooterPreset => (52, 16),
            PickerId::ProgressStyle => (48, 18),
            PickerId::Settings => (64, 28),
            _ => (56, 22),
        }
    }

    fn render_picker(f: &mut ratatui::Frame, area: Rect, app: &mut App) {
        app.update_picker_preview();
        app.update_artist_cover();
        let Some(top) = app.pickers.top() else {
            return;
        };
        let top_id = top.id;
        let top_help = top.id == PickerId::Help;

        let picker_area = if top_help {
            area
        } else {
            let square_min = 40
                .min(area.width.saturating_sub(2))
                .min(area.height.saturating_sub(2));
            let (content_w, content_h) = Self::picker_content_hint(top, app);
            let picker_width = content_w.max(square_min).min(area.width.saturating_sub(2));
            let scrolling = matches!(
                top.id,
                PickerId::Queue
                    | PickerId::YTSearch
                    | PickerId::SearchLibrary
                    | PickerId::CommandPalette
                    | PickerId::ThemePicker
                    | PickerId::PlaylistSelect
                    | PickerId::PlaylistTrackSelect
                    | PickerId::SpotifySearch
            );
            let picker_height = if scrolling {
                let height_cap = (area.height.saturating_sub(2) / 2).max(10);
                content_h.max((square_min / 2).max(10)).min(height_cap)
            } else {
                content_h.max(10).min(area.height.saturating_sub(2))
            };
            let picker_x = area.width.saturating_sub(picker_width) / 2;
            let picker_y = area.height.saturating_sub(picker_height) / 2;

            Rect {
                x: picker_x,
                y: picker_y,
                width: picker_width,
                height: picker_height,
            }
        };

        f.render_widget(Clear, picker_area);
        app.mouse_map.set_picker_area(picker_area);

        match top_id {
            PickerId::Queue => Self::render_queue_picker(f, picker_area, app),
            PickerId::YTSearch => Self::render_yt_search_picker(f, picker_area, app),
            PickerId::SearchLibrary => Self::render_search_library_picker(f, picker_area, app),
            PickerId::About => Self::render_about_picker(f, picker_area, app),
            PickerId::SleepTimer => Self::render_sleep_timer_picker(f, picker_area, app),
            PickerId::CommandPalette => Self::render_command_palette_picker(f, picker_area, app),
            PickerId::Equalizer => Self::render_equalizer_picker(f, picker_area, app),
            PickerId::ThemePicker => Self::render_theme_picker_picker(f, picker_area, app),
            PickerId::Help => Self::render_help_picker(f, picker_area, app),
            PickerId::PlaylistSelect => Self::render_playlist_select_picker(f, picker_area, app),
            PickerId::PlaylistTrackSelect => {
                Self::render_playlist_track_select_picker(f, picker_area, app)
            }
            PickerId::EditMetadata => Self::render_edit_metadata_picker(f, picker_area, app),
            PickerId::Crossfade => Self::render_crossfade_picker(f, picker_area, app),
            PickerId::VisualizerPreset => {
                Self::render_visualizer_preset_picker(f, picker_area, app)
            }
            PickerId::FooterPreset => Self::render_footer_preset_picker(f, picker_area, app),
            PickerId::ProgressStyle => Self::render_progress_style_picker(f, picker_area, app),
            PickerId::Settings => Self::render_settings_picker(f, picker_area, app),
            PickerId::Notifications => Self::render_notifications_picker(f, picker_area, app),
            PickerId::SpotifyLink => {
                let block = Self::picker_panel(
                    app,
                    " Spotify Link ",
                    Some(" Enter: authorize   Esc: cancel"),
                );
                let inner = block.inner(picker_area);
                f.render_widget(block, picker_area);

                if app.spotify_oauth_pending {
                    let lines = vec![
                        Line::from(Span::styled(
                            "Waiting for you to finish login in your browser…",
                            Style::default().fg(app.theme.fg_bright),
                        )),
                        Line::from(""),
                        Line::from(Span::styled(
                            "A browser window should have opened to authorize gtm.",
                            Style::default().fg(app.theme.fg_dim),
                        )),
                        Line::from(Span::styled(
                            "Once you approve, playlists sync automatically.",
                            Style::default().fg(app.theme.fg_dim),
                        )),
                        Line::from(""),
                        Line::from(Span::styled(
                            "Press Esc to cancel.",
                            Style::default().fg(app.theme.fg_dim),
                        )),
                    ];
                    let p = Paragraph::new(lines);
                    f.render_widget(p, inner);
                } else {
                    let input_cursor = cursor_span_style(app);
                    let mut lines = vec![
                        Line::from(Span::styled(
                            "Enter your Spotify app Client ID, then press Enter.",
                            Style::default().fg(app.theme.fg),
                        )),
                        Line::from(Span::styled(
                            "Tab switches field; a browser opens to authorize gtm.",
                            Style::default().fg(app.theme.fg_dim),
                        )),
                        Line::from(""),
                    ];

                    // Client ID field (active = field 0). Masked so the secret
                    // isn't echoed to the terminal while typing.
                    let cid_active = app.spotify_link_field == 0;
                    let cid_label = if cid_active {
                        app.theme.fg_bright
                    } else {
                        app.theme.fg_dim
                    };
                    let cid_text = if app.spotify_link_input.is_empty() {
                        "[ client id ]".to_string()
                    } else {
                        "•".repeat(app.spotify_link_input.chars().count())
                    };
                    let mut cid_spans = vec![
                        Span::styled(" Client ID: ", Style::default().fg(cid_label)),
                        Span::styled(cid_text, Style::default().fg(app.theme.accent)),
                    ];
                    if cid_active && let Some(cur) = input_cursor {
                        cid_spans.push(Span::styled(" ", cur));
                    }
                    lines.push(Line::from(cid_spans));

                    // Port field (active = field 1)
                    let port_active = app.spotify_link_field == 1;
                    let port_label = if port_active {
                        app.theme.fg_bright
                    } else {
                        app.theme.fg_dim
                    };
                    let mut port_spans = vec![
                        Span::styled(" Port:      ", Style::default().fg(port_label)),
                        Span::styled(
                            app.spotify_oauth_port.clone(),
                            Style::default().fg(app.theme.accent),
                        ),
                    ];
                    if port_active && let Some(cur) = input_cursor {
                        port_spans.push(Span::styled(" ", cur));
                    }
                    lines.push(Line::from(port_spans));

                    let p = Paragraph::new(lines);
                    f.render_widget(p, inner);
                }
            }
            PickerId::SpotifySearch => {
                let help = if app.spotify_status.as_ref().is_none_or(|s| !s.linked) {
                    " Enter: link   Esc: close"
                } else {
                    " Enter: play   Ctrl+D: download   Esc: close"
                };
                let block = Self::picker_panel(app, " \u{f1bc} Search ", Some(help));
                let inner = block.inner(picker_area);
                f.render_widget(block, picker_area);

                let query = app.pickers.top().map_or(String::new(), |o| o.query.clone());
                let cursor_style = cursor_span_style(app);

                if app.spotify_status.as_ref().is_none_or(|s| !s.linked) {
                    let token_input = app.spotify_token_input.clone();
                    let masked = "•".repeat(token_input.chars().count());
                    let cursor_style = cursor_span_style(app);
                    let lines = vec![
                        Line::from(Span::styled(
                            "Paste your Spotify access token and press Enter:",
                            Style::default().fg(app.theme.fg),
                        )),
                        Line::from(""),
                        Line::from(vec![
                            Span::styled(" > ", Style::default().fg(app.theme.fg_dim)),
                            Span::styled(masked, Style::default().fg(app.theme.fg)),
                            Span::styled(
                                if app.spotify_token_input.is_empty() {
                                    String::new()
                                } else {
                                    " ".to_string()
                                },
                                cursor_style.unwrap_or_default(),
                            ),
                        ]),
                        Line::from(""),
                        Line::from(Span::styled(
                            "Get a token from https://developer.spotify.com/dashboard",
                            Style::default().fg(app.theme.fg_dim),
                        )),
                    ];
                    let p = Paragraph::new(lines);
                    f.render_widget(p, inner);
                } else {
                    let search_line = Line::from(vec![
                        Span::styled(" > ", Style::default().fg(app.theme.fg_dim)),
                        Span::styled(query.as_str(), Style::default().fg(app.theme.fg)),
                        Span::styled(" ", cursor_style.unwrap_or_default()),
                    ]);

                    let sel = app.pickers.top().map_or(0, |o| o.selected);
                    let total = app.spotify_search_results.len();
                    let preview_h: u16 = if total > 0 { 7 } else { 0 };
                    let visible = inner.height.saturating_sub(preview_h) as usize;
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

                    let results_area = Rect {
                        x: inner.x,
                        y: inner.y,
                        width: inner.width,
                        height: inner.height.saturating_sub(preview_h),
                    };

                    let mut lines: Vec<Line> = vec![search_line];
                    if query.is_empty() {
                        lines.push(Line::from(Span::styled(
                            "  Type to search your synced playlists...",
                            Style::default().fg(app.theme.fg_dim),
                        )));
                    } else if total == 0 {
                        lines.push(Line::from(Span::styled(
                            "  No results found",
                            Style::default().fg(app.theme.fg_dim),
                        )));
                    } else {
                        for i in scroll_start..scroll_end {
                            let (_, _pl_name, track) = &app.spotify_search_results[i];
                            let prefix = if i == sel { " > " } else { "   " };
                            let dur = track
                                .duration_ms
                                .map(|ms| format_duration_short(ms / 1000))
                                .unwrap_or_default();
                            let content =
                                format!("{}{} - {} [{}]", prefix, track.artists, track.name, dur);
                            let style = if i == sel {
                                Style::default()
                                    .fg(app.theme.selection_fg_readable())
                                    .bg(app.theme.selection_bg)
                            } else {
                                Style::default()
                            };
                            let pad = row_pad(&content, inner.width);
                            lines.push(Line::from(Span::styled(
                                format!("{content}{}", " ".repeat(pad)),
                                style,
                            )));
                        }
                    }

                    let para = Paragraph::new(lines);
                    f.render_widget(para, results_area);

                    if preview_h > 0 && total > 0 && !query.is_empty() {
                        let preview_area = Rect {
                            x: inner.x,
                            y: inner.y + inner.height - preview_h,
                            width: inner.width,
                            height: preview_h,
                        };
                        let rule = Line::from(Span::styled(
                            "\u{2500}".repeat(preview_area.width as usize),
                            Style::default().fg(app.theme.muted_border),
                        ));
                        f.render_widget(
                            Paragraph::new(rule),
                            Rect {
                                x: preview_area.x,
                                y: preview_area.y,
                                width: preview_area.width,
                                height: 1,
                            },
                        );
                        let body = Rect {
                            x: preview_area.x,
                            y: preview_area.y + 1,
                            width: preview_area.width,
                            height: preview_area.height.saturating_sub(1),
                        };
                        let (_, _, track) = &app.spotify_search_results[sel.min(total - 1)];
                        let mut meta_lines = Vec::new();
                        let mut push = |key: &str, value: &str| {
                            meta_lines.push(Line::from(vec![
                                Span::styled(
                                    format!("{key:>9} "),
                                    Style::default().fg(app.theme.fg_dim),
                                ),
                                Span::styled(
                                    value.to_string(),
                                    Style::default().fg(app.theme.fg_bright),
                                ),
                            ]));
                        };
                        push("Track", &track.name);
                        push("Artist", &track.artists);
                        if let Some(ref album) = track.album {
                            push("Album", album);
                        }
                        if let Some(ms) = track.duration_ms {
                            push("Length", &format_duration_short(ms / 1000));
                        }
                        f.render_widget(Paragraph::new(meta_lines), body);
                    }
                }
            }
        }
    }

    fn picker_panel<'a>(
        app: &App,
        title: impl Into<Cow<'a, str>>,
        help: Option<&'a str>,
    ) -> Block<'a> {
        let mut block = Block::default()
            .title(Line::from(Span::styled(
                title.into(),
                Style::default()
                    .fg(app.theme.accent)
                    .add_modifier(Modifier::BOLD),
            )))
            .padding(Padding::horizontal(1))
            .style(Style::default().bg(if app.transparent_pickers {
                ratatui::style::Color::Reset
            } else {
                app.float_bg()
            }));
        match help {
            Some(h) => {
                block = block.title_bottom(Line::from(Span::styled(
                    h,
                    Style::default().fg(app.theme.fg_dim),
                )));
            }
            None => {
                block = block.title_bottom(
                    Line::from(Span::styled(
                        " \u{2699} ",
                        Style::default().fg(app.theme.fg_dim),
                    ))
                    .right_aligned(),
                );
            }
        }
        block
    }

    fn render_queue_picker(f: &mut ratatui::Frame, area: Rect, app: &mut App) {
        let sel = app.pickers.top().map_or(0, |o| o.selected);

        let (title, hint) = if app.queue_move_index.is_some() {
            let from = app.queue_move_index.unwrap_or(0);
            let to = app.queue_move_target;
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

        let total = app.queue_cache.len();
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
            let track = &app.queue_cache[i];
            let is_current = i == app.queue_cursor;
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
            app.mouse_map
                .register(row_rect, crate::mouse::MouseZone::PickerItem(i));
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
            Self::render_queue_upnext_preview(f, preview_area, app, app.queue_cursor + 1);
        }
    }

    fn render_queue_upnext_preview(
        f: &mut ratatui::Frame,
        area: Rect,
        app: &mut App,
        next_idx: usize,
    ) {
        app.update_queue_preview_cover();
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
        match app.queue_cache.get(next_idx) {
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
                let has_cover = app.queue_preview_cover.is_some();
                if cover_w > 0 && cover_h > 0 {
                    let cover_area = Rect {
                        x: inner.x + 1,
                        y: inner.y,
                        width: cover_w,
                        height: cover_h,
                    };
                    if has_cover {
                        if let Some(ref mut protocol) = app.queue_preview_cover_stateful {
                            let image = StatefulImage::new();
                            f.render_stateful_widget(image, cover_area, protocol);
                        } else if let Some(ref bytes) = app.queue_preview_cover {
                            Render::cover_block(f, cover_area, bytes);
                        }
                    } else {
                        let glyph = Paragraph::new(Line::from(Span::styled(
                            "\u{266b}",
                            Style::default().fg(app.theme.fg_dim),
                        )))
                        .alignment(Alignment::Center);
                        f.render_widget(glyph, cover_area);
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

    fn render_yt_search_picker(f: &mut ratatui::Frame, area: Rect, app: &mut App) {
        let block = Self::picker_panel(
            app,
            " \u{f167} Search ",
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
        if app.yt_results_cache.is_empty() && app.yt_search_loading {
            f.render_widget(Paragraph::new(lines), inner);
            let loader_area = Rect {
                x: inner.x,
                y: inner.y + 1,
                width: inner.width,
                height: inner.height.saturating_sub(1),
            };
            Render::loader(f, loader_area, app, "Searching YouTube\u{2026}");
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
            let content = format!("{prefix}{}{} [{}]", icon, display, dur);
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
            app.mouse_map
                .register(row_rect, crate::mouse::MouseZone::PickerItem(i));
        }

        let para = Paragraph::new(lines);
        f.render_widget(para, inner);
    }

    fn render_search_library_picker(f: &mut ratatui::Frame, area: Rect, app: &mut App) {
        let source = app.pickers.top().map_or(PickerSource::All, |o| o.source);
        let title = format!(" Search: {} ", source.label());
        let block =
            Self::picker_panel(app, &title, Some(" Tab: filter   Enter: open   Esc: close"));
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
                LibraryPick::Playlist(i) => {
                    format!("{}\u{1f4dc} {}", prefix, app.playlist_cache[*i].name)
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
            app.mouse_map
                .register(row_rect, crate::mouse::MouseZone::PickerItem(i));
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

    fn render_search_preview(
        f: &mut ratatui::Frame,
        area: Rect,
        app: &mut App,
        picks: &[crate::app::LibraryPick],
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
            if let Some(protocol) = app.artist_cover_stateful.as_mut() {
                let image = StatefulImage::new();
                f.render_stateful_widget(image, cover_area, protocol);
            } else {
                let placeholder = Paragraph::new(Line::from(Span::styled(
                    format!("{:^width$}", "\u{1f465}", width = cover_w as usize),
                    Style::default().fg(app.theme.fg_dim),
                )));
                f.render_widget(placeholder, cover_area);
            }
        } else if let Some(protocol) = app.picker_preview_stateful.as_mut() {
            let image = StatefulImage::new();
            f.render_stateful_widget(image, cover_area, protocol);
        } else if let Some(bytes) = app.picker_preview_cover.as_deref() {
            Render::cover_block(f, cover_area, bytes);
        } else {
            let placeholder = Paragraph::new(Line::from(Span::styled(
                format!("{:^width$}", "\u{266b}", width = cover_w as usize),
                Style::default().fg(app.theme.fg_dim),
            )));
            f.render_widget(placeholder, cover_area);
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
                let p = &app.playlist_cache[*i];
                push("Playlist", &p.name);
                push("Tracks", &p.track_count.to_string());
            }
            None => {
                push("", "No results");
            }
        }
        f.render_widget(Paragraph::new(meta_lines), meta_area);
    }

    fn render_settings_picker(f: &mut ratatui::Frame, area: Rect, app: &App) {
        let block = Self::picker_panel(
            app,
            " Settings ",
            Some("Tab: switch pane   Enter: act   ←/→: cycle values"),
        );
        let inner = block.inner(area);
        f.render_widget(block, area);

        let panes = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Length(18), Constraint::Min(0)])
            .split(inner);

        let settings_icons = if use_nerd_fonts() {
            SETTINGS_ICONS_NERD
        } else {
            SETTINGS_ICONS_ASCII
        };
        let settings_focus = app.settings_pane_focus;

        let left_items: Vec<ListItem> = SETTINGS_CATEGORIES
            .iter()
            .enumerate()
            .map(|(i, cat)| {
                let icon = settings_icons.get(i).unwrap_or(&" ");
                let is_active = i == app.settings_category;
                let style = if is_active && settings_focus {
                    Style::default()
                        .fg(app.theme.selection_fg_readable())
                        .bg(app.theme.selection_bg)
                } else if is_active {
                    Style::default().fg(app.theme.accent)
                } else {
                    Style::default().fg(app.theme.fg)
                };
                ListItem::new(format!(" {} {}", icon, cat)).style(style)
            })
            .collect();
        f.render_widget(List::new(left_items), panes[0]);

        if app.settings_category < SETTINGS_CATEGORIES.len() {
            let indicator_y = panes[0].y + app.settings_category as u16;
            if indicator_y < panes[0].y + panes[0].height {
                let indicator_area = Rect {
                    x: panes[0].x + 1,
                    y: indicator_y,
                    width: 1,
                    height: 1,
                };
                let indicator =
                    Paragraph::new("▎").style(Style::default().fg(app.theme.sidebar_active_border));
                f.render_widget(indicator, indicator_area);
            }
        }

        let items: Vec<String> = match app.settings_category {
            0 => vec![
                "Cookie Source  chromium".to_string(),
                format!(
                    "Cookie File    {}",
                    app.cookie_file.as_deref().unwrap_or("(none)")
                ),
                "JS Runtime     deno".to_string(),
                "Auto Download  read-only".to_string(),
            ],
            1 => {
                let crossfade_on = app
                    .state
                    .crossfade
                    .as_ref()
                    .map(|c| c.enabled)
                    .unwrap_or(false);
                let crossfade_dur = app
                    .state
                    .crossfade
                    .as_ref()
                    .map(|c| c.duration_secs)
                    .unwrap_or(0);
                let reverb_on = app.state.reverb.enabled;
                vec![
                    format!("Repeat         {:?}  ▶", app.state.repeat),
                    format!(
                        "Shuffle        {}",
                        if app.state.shuffle { "On" } else { "Off" }
                    ),
                    if crossfade_on {
                        format!("Crossfade      On  {}s  ▶", crossfade_dur)
                    } else {
                        "Crossfade      Off  ▶".to_string()
                    },
                    format!(
                        "EQ Enabled     {}",
                        if app.state.eq_enabled { "On" } else { "Off" }
                    ),
                    format!("Reverb         {}", if reverb_on { "On" } else { "Off" }),
                ]
            }
            2 => {
                let theme_name = app
                    .themes
                    .get(app.theme_index)
                    .map(|t| t.name.as_ref())
                    .unwrap_or("Chadrula");
                vec![
                    format!("Theme          {}  ▶", theme_name),
                    format!(
                        "Transparent BG {}",
                        if app.transparent_bg { "On" } else { "Off" }
                    ),
                    format!(
                        "Transparent Pickers {}",
                        if app.transparent_pickers { "On" } else { "Off" }
                    ),
                    "Sync Covers    Enter  ▶".to_string(),
                    "Sync Lyrics    Enter  ▶".to_string(),
                    "Sync Metadata  Enter  ▶".to_string(),
                    format!(
                        "Footer Preset  {}  ▶",
                        app.footer_presets
                            .get(app.footer_preset)
                            .map(|p| p.name.as_ref())
                            .unwrap_or("Default")
                    ),
                    format!("Visualizer     {}  ▶", app.visualizer.preset.name()),
                    format!(
                        "Reactive Theme {}",
                        if app.reactive_theme { "On" } else { "Off" }
                    ),
                    "Clear Lyrics Cache  Enter".to_string(),
                    "Clear Cover Cache    Enter  ▶".to_string(),
                ]
            }
            3 => {
                let st = app.spotify_status.clone().unwrap_or_default();
                let connected = if st.linked {
                    "Connected"
                } else {
                    "Disconnected"
                };
                let user = st.user.as_deref().unwrap_or("(none)");
                let status_label = if !st.linked {
                    connected.to_string()
                } else if let Some(err) = st.error.as_deref() {
                    let mut e = err.chars().take(14).collect::<String>();
                    if err.chars().count() > 14 {
                        e.push('…');
                    }
                    format!("{connected}: {e}")
                } else if st.premium {
                    if st.playing {
                        "Playing ▶".to_string()
                    } else {
                        "Paused  ❚❚".to_string()
                    }
                } else {
                    "Unavailable (Premium)".to_string()
                };
                let device_label = st
                    .device
                    .clone()
                    .filter(|d| !d.is_empty())
                    .unwrap_or_else(|| "(none)".to_string());
                vec![
                    format!("Status         {status_label}"),
                    format!("Account        {user:<10}"),
                    format!("Playlists      {:>3}", st.playlists),
                    "Link Account   Enter".to_string(),
                    "Sync Now       Enter".to_string(),
                    "Unlink         Enter".to_string(),
                    format!("Device         {device_label}"),
                ]
            }
            _ => vec![],
        };

        let category_label = SETTINGS_CATEGORIES
            .get(app.settings_category)
            .unwrap_or(&"");
        let right_title = format!(" {category_label} ");
        let right_block = Block::default()
            .borders(Borders::TOP | Borders::RIGHT | Borders::BOTTOM)
            .border_style(Style::default().fg(if settings_focus {
                app.theme.accent
            } else {
                app.theme.fg_dim
            }))
            .title(Span::styled(
                right_title,
                Style::default().fg(app.theme.fg_bright),
            ));
        let right_inner = right_block.inner(panes[1]);
        f.render_widget(right_block, panes[1]);

        let mut lines = Vec::new();
        let sel = app.settings_option;
        for (i, item) in items.iter().enumerate() {
            let is_sel = i == sel && !settings_focus;
            let style = if is_sel {
                Style::default()
                    .fg(app.theme.selection_fg_readable())
                    .bg(app.theme.selection_bg)
            } else {
                Style::default().fg(app.theme.fg)
            };
            lines.push(Line::from(Span::styled(item, style)));
        }
        lines.push(Line::from(""));
        match (app.settings_category, sel) {
            (1, 1) => lines.push(Line::from(Span::styled(
                " Press Enter to toggle cookie path.",
                Style::default().fg(app.theme.fg_dim),
            ))),
            (2, 0) => lines.push(Line::from(Span::styled(
                format!(" Press Enter to cycle (current: {:?}).", app.state.repeat),
                Style::default().fg(app.theme.fg_dim),
            ))),
            (2, 1) => lines.push(Line::from(Span::styled(
                " Press Enter to toggle shuffle.",
                Style::default().fg(app.theme.fg_dim),
            ))),
            (2, 2) => lines.push(Line::from(Span::styled(
                " Press Enter to open crossfade picker.",
                Style::default().fg(app.theme.fg_dim),
            ))),
            (2, 3) => {
                let eq_on = app.state.eq_enabled;
                lines.push(Line::from(Span::styled(
                    if eq_on {
                        " Press Enter to disable EQ."
                    } else {
                        " Press Enter to enable EQ."
                    },
                    Style::default().fg(app.theme.fg_dim),
                )));
            }
            (2, 4) => {
                let rev_on = app.state.reverb.enabled;
                lines.push(Line::from(Span::styled(
                    if rev_on {
                        " Press Enter to disable reverb."
                    } else {
                        " Press Enter to enable reverb."
                    },
                    Style::default().fg(app.theme.fg_dim),
                )));
            }
            (3, 0) => lines.push(Line::from(Span::styled(
                " Press Enter to open Theme Picker.",
                Style::default().fg(app.theme.fg_dim),
            ))),
            (3, 1) => lines.push(Line::from(Span::styled(
                " Press Enter to toggle transparent background.",
                Style::default().fg(app.theme.fg_dim),
            ))),
            (3, 2) => lines.push(Line::from(Span::styled(
                " Press Enter to toggle transparent pickers.",
                Style::default().fg(app.theme.fg_dim),
            ))),
            (3, 3) => lines.push(Line::from(Span::styled(
                " Download missing cover art from Deezer.",
                Style::default().fg(app.theme.fg_dim),
            ))),
            (3, 4) => lines.push(Line::from(Span::styled(
                " Fetch and save lyrics for all tracks.",
                Style::default().fg(app.theme.fg_dim),
            ))),
            (3, 5) => lines.push(Line::from(Span::styled(
                " Resolve and embed clean tags into files.",
                Style::default().fg(app.theme.fg_dim),
            ))),
            (3, 6) => lines.push(Line::from(Span::styled(
                " Press Enter to open Footer Preset picker.",
                Style::default().fg(app.theme.fg_dim),
            ))),
            (3, 7) => lines.push(Line::from(Span::styled(
                " Press Enter to open visualizer picker.",
                Style::default().fg(app.theme.fg_dim),
            ))),
            (3, 8) => lines.push(Line::from(Span::styled(
                " Press Enter to toggle reactive theme.",
                Style::default().fg(app.theme.fg_dim),
            ))),
            (3, 9) => lines.push(Line::from(Span::styled(
                " Clear cached lyrics for all tracks.",
                Style::default().fg(app.theme.fg_dim),
            ))),
            (3, 10) => lines.push(Line::from(Span::styled(
                " Clear downloaded cover art cache.",
                Style::default().fg(app.theme.fg_dim),
            ))),
            (4, 0) => lines.push(Line::from(Span::styled(
                " Spotify integration status.",
                Style::default().fg(app.theme.fg_dim),
            ))),
            (4, 3) => lines.push(Line::from(Span::styled(
                " Press Enter to paste a Spotify access token.",
                Style::default().fg(app.theme.fg_dim),
            ))),
            (4, 4) => lines.push(Line::from(Span::styled(
                " Re-fetch playlists from Spotify.",
                Style::default().fg(app.theme.fg_dim),
            ))),
            (4, 5) => lines.push(Line::from(Span::styled(
                " Remove token and disconnect.",
                Style::default().fg(app.theme.fg_dim),
            ))),
            _ => {}
        }

        let right_para = Paragraph::new(lines);
        f.render_widget(right_para, right_inner);
    }
}

// ─── Footer ───

const TRACK_INFO_CARD_H: u16 = 16;

const TRACK_INFO_TEXT_H: u16 = 6;

fn track_info_block_height() -> u16 {
    if no_image_protocol() {
        TRACK_INFO_TEXT_H
    } else {
        TRACK_INFO_CARD_H
    }
}

fn library_stats_line(app: &App) -> String {
    if app.browse_detail.is_some() {
        if app.library_category == 5 {
            let n = app.spotify_playlist_tracks_cache.len();
            return format!(" {} {} ", n, plural(n, "track", "tracks"));
        }
        let f = app.filtered_tracks();
        let total_dur: u64 = f.iter().map(|t| t.duration as u64).sum();
        return format!(
            " {} {} | {}h {}m ",
            f.len(),
            plural(f.len(), "track", "tracks"),
            total_dur / 3600,
            (total_dur % 3600) / 60
        );
    }
    match app.library_category {
        2 => {
            let n = app.unique_albums().len();
            format!(" {} {} ", n, plural(n, "album", "albums"))
        }
        3 => {
            let n = app.unique_artists().len();
            format!(" {} {} ", n, plural(n, "artist", "artists"))
        }
        4 => {
            let n = app.playlist_cache.len();
            format!(" {} {} ", n, plural(n, "playlist", "playlists"))
        }
        5 => {
            let n = app.spotify_playlists.len();
            format!(" {} {} ", n, plural(n, "playlist", "playlists"))
        }
        _ => {
            let f = app.filtered_tracks();
            let total_dur: u64 = f.iter().map(|t| t.duration as u64).sum();
            format!(
                " {} {} | {}h {}m | {} ",
                f.len(),
                plural(f.len(), "track", "tracks"),
                total_dur / 3600,
                (total_dur % 3600) / 60,
                app.track_sort.label()
            )
        }
    }
}

fn source_label(use_nerd: bool, source: &str) -> String {
    if use_nerd {
        match source {
            "Spotify" => " \u{f1bc} Spotify".to_string(),
            "YouTube" => " \u{f167} YouTube".to_string(),
            _ => " \u{f3b5} Local".to_string(),
        }
    } else {
        match source {
            "Spotify" => " ♫ Spotify".to_string(),
            "YouTube" => " ▶ YouTube".to_string(),
            _ => " ♪ Local".to_string(),
        }
    }
}

struct TrackInfoFields {
    title: String,
    artist: String,
    album: Option<String>,
    meta: String,
    has_cover: bool,
}

fn track_info_fields(app: &App) -> Option<TrackInfoFields> {
    let use_nerd = use_nerd_fonts();
    match app.track_info_kind() {
        TrackInfoKind::Track => {
            let track = app
                .track_popup_track_id
                .and_then(|id| app.tracks_cache.iter().find(|t| t.id == id))?;
            let title = if track.title.is_empty() {
                std::path::Path::new(&track.path)
                    .file_stem()
                    .map(|s| s.to_string_lossy().to_string())
                    .unwrap_or_default()
            } else {
                track.title.clone()
            };
            let artist = if track.artist.is_empty() {
                "Unknown".to_string()
            } else {
                track.artist.clone()
            };
            let album = if track.album.is_empty() {
                None
            } else {
                Some(track.album.clone())
            };
            let source = if track.path.contains("/audio/spotify")
                || track.path.starts_with("spotify:")
            {
                "Spotify"
            } else if track.path.contains("/audio/youtube") || track.path.starts_with("youtube:") {
                "YouTube"
            } else {
                "Local"
            };
            let meta = format!(
                " {} | {}",
                format_duration(track.duration as u64),
                source_label(use_nerd, source).trim_start()
            );
            let fav = if track.favourite { " \u{2665}" } else { "" };
            let meta = format!("{}{}", meta, fav);
            Some(TrackInfoFields {
                title,
                artist,
                album,
                meta,
                has_cover: app.track_popup_cover.is_some(),
            })
        }
        TrackInfoKind::Album => {
            let albums = app.unique_albums();
            let pos = app.list_pos();
            let (name, count) = albums.get(pos)?;
            let artist = app
                .tracks_cache
                .iter()
                .find(|t| {
                    let album: &str = if t.album.is_empty() {
                        "Unknown Album"
                    } else {
                        &t.album
                    };
                    album == name
                })
                .map(|t| {
                    if t.artist.is_empty() {
                        "Unknown".to_string()
                    } else {
                        t.artist.clone()
                    }
                })
                .unwrap_or_default();
            Some(TrackInfoFields {
                title: name.clone(),
                artist,
                album: None,
                meta: format!(
                    " [{} {}] | {}",
                    *count,
                    plural(*count, "track", "tracks"),
                    source_label(use_nerd, "Local").trim_start()
                ),
                has_cover: app.track_popup_cover.is_some(),
            })
        }
        TrackInfoKind::Artist => {
            let artists = app.unique_artists();
            let pos = app.list_pos();
            let (name, count) = artists.get(pos)?;
            Some(TrackInfoFields {
                title: name.clone(),
                artist: String::new(),
                album: None,
                meta: format!(
                    " [{} {}] | {}",
                    *count,
                    plural(*count, "track", "tracks"),
                    source_label(use_nerd, "Local").trim_start()
                ),
                has_cover: app.track_popup_cover.is_some(),
            })
        }
        TrackInfoKind::Playlist => {
            let playlists = &app.playlist_cache;
            let pos = app.list_pos();
            let pl = playlists.get(pos)?;
            let tc = pl.track_count as usize;
            Some(TrackInfoFields {
                title: pl.name.clone(),
                artist: String::new(),
                album: None,
                meta: format!(
                    " [{} {}] | {}",
                    tc,
                    plural(tc, "track", "tracks"),
                    source_label(use_nerd, "Local").trim_start()
                ),
                has_cover: false,
            })
        }
        TrackInfoKind::SpotifyPlaylist => {
            let playlists = &app.spotify_playlists;
            let pos = app.list_pos();
            let pl = playlists.get(pos)?;
            let tc = pl.tracks.len();
            Some(TrackInfoFields {
                title: pl.name.clone(),
                artist: pl.owner.clone(),
                album: None,
                meta: format!(
                    " [{} {}] | {}",
                    tc,
                    plural(tc, "track", "tracks"),
                    source_label(use_nerd, "Spotify").trim_start()
                ),
                has_cover: false,
            })
        }
        TrackInfoKind::SpotifyTrack => {
            let tracks = &app.spotify_playlist_tracks_cache;
            let pos = app.list_pos();
            let st = tracks.get(pos)?;
            let dur = st
                .duration_ms
                .map(|ms| format!(" [{}]", format_duration(ms / 1000)))
                .unwrap_or_default();
            Some(TrackInfoFields {
                title: st.name.clone(),
                artist: st.artists.clone(),
                album: None,
                meta: format!(
                    "{} | {}",
                    dur,
                    source_label(use_nerd, "Spotify").trim_start()
                ),
                has_cover: false,
            })
        }
    }
}

pub const COMMAND_PALETTE_COMMANDS: &[(&str, &str, &str)] = &[
    ("\u{25b6}\u{fe0f} Play/Pause", "Space", "play/pause"),
    ("\u{23ed}\u{fe0f} Next Track", "n", "next track"),
    ("\u{23ee}\u{fe0f} Prev Track", "p", "prev track"),
    ("\u{23f9}\u{fe0f} Stop", "s", "stop"),
    ("\u{23e9}\u{fe0f} Seek Forward", ".", "seek forward"),
    ("\u{23ea}\u{fe0f} Seek Backward", ",", "seek backward"),
    ("\u{1f50a} Volume Up", "+", "volume up"),
    ("\u{1f509} Volume Down", "-", "volume down"),
    ("\u{1f507} Mute: Toggle", "m", "mute"),
    ("\u{1f501} Repeat Mode", "r", "repeat"),
    ("\u{1f500} Shuffle Library", "S", "shuffle"),
    ("\u{2764}\u{fe0f} Toggle Favourite", "f", "toggle favourite"),
    ("\u{1f50d} Search Track", "/", "search"),
    ("\u{1f50e} Search Library", "Alt+/", "search lib"),
    ("\u{1f4cb} Queue", "Alt+Q", "queue"),
    ("\u{25b6}\u{fe0f} YouTube Search", "Alt+Y", "youtube"),
    ("\u{1f3b5} Spotify", "Alt+S", "spotify"),
    ("\u{1f4dd} Fetch Lyrics", "l", "fetch lyrics"),
    ("\u{1f5d1} Clear Queue", "D", "clear queue"),
    ("\u{2611}\u{fe0f} Multiselect", "v", "multiselect"),
    ("\u{2795} Add to Queue", "a", "add to queue"),
    ("\u{1f4dc} Add to Playlist", "A", "add to playlist"),
    ("\u{274c} Delete from List", "x", "delete from list"),
    ("\u{2b07}\u{fe0f} Jump to End", "G", "jump to end"),
    ("\u{270f}\u{fe0f} Edit Metadata", "e", "edit metadata"),
    ("\u{27a1}\u{fe0f} Tab Cycle", "Tab", "tab cycle"),
    ("\u{2b05}\u{fe0f} Prev Tab", "Shift+Tab", "prev tab"),
    ("\u{2699}\u{fe0f} Settings", "Alt+,", "settings"),
    ("\u{1f39a} Equalizer", "Alt+E", "eq"),
    ("\u{23f0}\u{fe0f} Sleep Timer", "Alt+Z", "sleeptimer"),
    ("\u{1f3a8} Theme", "Alt+C", "themepicker"),
    ("\u{2139}\u{fe0f} About", "Alt+A", "about"),
    ("\u{1f514} Notifications", "Alt+N", "notifications"),
    ("\u{1f3a8} Progress Style", "Alt+P", "progress style"),
    ("\u{1f3b6} Visualizer: Toggle", "Ctrl+V", "visualizer"),
    ("\u{1f3b6} Visualizer Preset", "Alt+V", "visualizer preset"),
    ("\u{23f9}\u{fe0f} Quit", "q", "quit"),
    ("\u{23f9}\u{fe0f} Quit Daemon", "Q/Ctrl+Q", "quit daemon"),
    ("\u{2753} Toggle Help", "?", "toggle help"),
    ("\u{1f6ab} Hide Help Bar", "Ctrl+H", "hide help bar"),
    ("\u{1fa7a} Health Check", "Alt+H", "health check"),
];

pub const COMMAND_GROUPS: &[(&str, usize)] = &[
    ("Playback", 12),
    ("Library & Queue", 13),
    ("View & Overlays", 11),
    ("System", 5),
];

pub const HELP_LINES: &[(&str, &str)] = &[
    ("topic", "── Playback ──"),
    ("", "   Space       Play / Pause"),
    ("", "   n           Next Track"),
    ("", "   p           Previous Track"),
    ("", "   s           Stop"),
    ("", "   .           Seek Forward"),
    ("", "   ,           Seek Backward"),
    ("", "   + / -       Volume Up / Down"),
    ("", "   m           Mute Toggle"),
    ("", "   r           Repeat Mode"),
    ("", "   S           Shuffle Library"),
    ("", "   f           Toggle Favourite"),
    ("topic", "── Navigation ──"),
    ("", "   Tab         Switch Pane"),
    ("", "   Shift+Tab   Switch Pane (back)"),
    ("", "   /           Search Track"),
    ("", "   Alt+Q       Queue"),
    ("", "   Alt+/       Search Library"),
    ("", "   Alt+Y       YouTube Search"),
    ("", "   Alt+S       Spotify"),
    ("topic", "── View ──"),
    ("", "   ?           Toggle Help"),
    ("", "   Ctrl+H      Hide Help Bar"),
    ("", "   Ctrl+V      Visualizer Toggle"),
    ("", "   Alt+V       Visualizer Preset"),
    ("", "   Alt+P       Progress Style"),
    ("", "   Alt+C       Theme Picker"),
    ("", "   Alt+A       About"),
    ("", "   Alt+E       Equalizer"),
    ("", "   Alt+Z       Sleep Timer"),
    ("", "   l           Fetch Lyrics"),
    ("topic", "── Queue ──"),
    ("", "   a           Add to Queue"),
    ("", "   A           Add to Playlist"),
    ("", "   x           Delete from List"),
    ("", "   D           Clear Queue"),
    ("", "   v           Multiselect"),
    ("", "   e           Edit Metadata"),
    ("topic", "── System ──"),
    ("", "   q           Quit"),
    ("", "   Q / Ctrl+Q  Quit Daemon"),
    ("", "   Alt+H       Health Check"),
    ("", "   Alt+,       Settings"),
];

pub const CROSSFADE_DURATIONS: [u8; 5] = [3, 5, 10, 15, 30];

impl Pickers {
    fn render_about_picker(f: &mut ratatui::Frame, area: Rect, app: &App) {
        let block = Self::picker_panel(app, " About ", Some("Esc: close"));
        let inner = block.inner(area);
        f.render_widget(block, area);

        let version = option_env!("CARGO_PKG_VERSION").unwrap_or("0.1.0");
        let commit = option_env!("VERGEN_GIT_SHA").unwrap_or("unknown");
        let build_date = option_env!("VERGEN_BUILD_DATE").unwrap_or("unknown");
        let lib_count = app.tracks_cache.len();
        let queue_count = app.queue_cache.len();

        let arch = std::env::consts::ARCH;
        let cpus = std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(1);
        let mem_kb = crate::footer::read_process_memory_kb();
        let mem_str = mem_kb
            .map(|kb| {
                if kb > 1024 * 1024 {
                    format!("{} GB", kb / (1024 * 1024))
                } else {
                    format!("{} MB", kb / 1024)
                }
            })
            .unwrap_or_else(|| "unknown".into());
        // Compiler/linker provenance.
        let compiler = option_env!("VERGEN_RUSTC_SEMVER").unwrap_or("unknown");
        let linker = option_env!("GTM_LINKER").unwrap_or("unknown");
        // Audio / rendering backends (unused after removing backends line).
        let _ratatui_ver = option_env!("GTM_RATATUI_VERSION").unwrap_or("unknown");
        let _rodio_ver = option_env!("GTM_RODIO_VERSION").unwrap_or("unknown");
        let _symphonia_ver = option_env!("GTM_SYMPHONIA_VERSION").unwrap_or("unknown");

        let lines = vec![
            Line::from(Span::styled(
                format!(" gtm {version}"),
                Style::default()
                    .fg(app.theme.accent)
                    .add_modifier(Modifier::BOLD),
            )),
            Line::from(""),
            Line::from(Span::styled(
                " Copyright (C) 2026, prjctimg",
                Style::default().fg(app.theme.fg_dim),
            )),
            Line::from(Span::styled(
                " License GPL-3.0",
                Style::default().fg(app.theme.fg_dim),
            )),
            Line::from(""),
            Line::from(Span::styled(
                " Build",
                Style::default().fg(app.theme.fg_dim),
            )),
            Line::from(Span::styled(
                format!("   Commit:  {:.7}", commit),
                Style::default().fg(app.theme.fg),
            )),
            Line::from(Span::styled(
                format!("   Date:    {}", build_date),
                Style::default().fg(app.theme.fg),
            )),
            Line::from(Span::styled(
                format!("   Compiler: {}", compiler),
                Style::default().fg(app.theme.fg),
            )),
            Line::from(Span::styled(
                format!("   Linker:  {}", linker),
                Style::default().fg(app.theme.fg),
            )),
            Line::from(Span::styled(
                format!(
                    "   System:   {} {} \u{2022} {} CPU \u{2022} {} RAM",
                    std::env::consts::OS,
                    arch,
                    cpus,
                    mem_str
                ),
                Style::default().fg(app.theme.fg),
            )),
            Line::from(""),
            Line::from(Span::styled(
                " Status",
                Style::default().fg(app.theme.fg_dim),
            )),
            Line::from(Span::styled(
                format!("   Playing:  {:?}", app.state.status),
                Style::default().fg(app.theme.warning),
            )),
            Line::from(Span::styled(
                format!("   Volume:   {}%", app.state.volume),
                Style::default().fg(app.theme.volume_color(app.state.volume)),
            )),
            Line::from(Span::styled(
                format!(
                    "   Queue:    {} {}",
                    queue_count,
                    plural(queue_count, "track", "tracks")
                ),
                Style::default().fg(app.theme.fg_bright),
            )),
            Line::from(Span::styled(
                format!(
                    "   Library:  {} {}",
                    lib_count,
                    plural(lib_count, "track", "tracks")
                ),
                Style::default().fg(app.theme.fg_bright),
            )),
            Line::from(Span::styled(
                format!(
                    "   Shuffle:  {}",
                    if app.state.shuffle { "ON" } else { "OFF" }
                ),
                Style::default().fg(app.theme.fg_bright),
            )),
            Line::from(Span::styled(
                format!("   Repeat:   {:?}", app.state.repeat),
                Style::default().fg(app.theme.fg_bright),
            )),
        ];

        let p = Paragraph::new(lines).alignment(Alignment::Left);
        f.render_widget(p, inner);
    }

    fn render_help_picker(f: &mut ratatui::Frame, area: Rect, app: &mut App) {
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

    fn render_notifications_picker(f: &mut ratatui::Frame, area: Rect, app: &mut App) {
        let block = Self::picker_panel(
            app,
            " Notifications ",
            Some("\u{2191}/\u{2193}: browse   Esc: close"),
        );
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
            app.mouse_map
                .register(row_rect, crate::mouse::MouseZone::PickerItem(i));
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
                let msg_area = Rect {
                    x: preview_area.x,
                    y: preview_area.y + header_h,
                    width: preview_area.width,
                    height: preview_area.height.saturating_sub(header_h),
                };
                f.render_widget(preview_para, msg_area);
            }
        }
    }

    fn relative_age(elapsed: std::time::Duration) -> String {
        let s = elapsed.as_secs();
        if s < 60 {
            format!("{s}s")
        } else if s < 3600 {
            format!("{}m", s / 60)
        } else {
            format!("{}h", s / 3600)
        }
    }

    fn render_sleep_timer_picker(f: &mut ratatui::Frame, area: Rect, app: &App) {
        if app.sleep_timer.input_mode {
            let block = Self::picker_panel(
                app,
                " Sleep Timer: Manual Input ",
                Some("Enter: set   Esc: back"),
            );
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
            "h/-: decrease   l/+: increase   i: input   Enter: set   c: cancel   Esc: close"
        } else {
            "h/-: decrease   l/+: increase   i: input   Enter: set   Esc: close"
        };
        let block = Self::picker_panel(app, " Sleep Timer ", Some(help));
        let inner = block.inner(area);
        f.render_widget(block, area);

        let mins = app.sleep_timer.minutes;

        let mut lines: Vec<Line> = Vec::new();

        lines.push(Line::from(vec![Span::styled(
            format!("  Timer: {} minutes", mins),
            Style::default()
                .fg(app.theme.fg)
                .add_modifier(Modifier::BOLD),
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

        let quick_opts = [5u32, 10, 15, 30, 60, 90, 120];
        let sel = app
            .pickers
            .top()
            .map_or(0, |o| o.selected.min(quick_opts.len() - 1));
        let mut spans: Vec<Span> = vec![Span::styled("  ", Style::default())];
        for (i, &m) in quick_opts.iter().enumerate() {
            let style = if i == sel {
                Style::default()
                    .fg(app.theme.selection_fg_readable())
                    .bg(app.theme.selection_bg)
                    .add_modifier(Modifier::BOLD)
            } else if m == mins {
                Style::default().fg(app.theme.accent)
            } else {
                Style::default().fg(app.theme.fg_dim)
            };
            spans.push(Span::styled(format!("[{}m] ", m), style));
        }
        lines.push(Line::from(spans));
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

    fn render_command_palette_picker(f: &mut ratatui::Frame, area: Rect, app: &mut App) {
        let commands = COMMAND_PALETTE_COMMANDS;

        let query = app.pickers.top().map_or(String::new(), |o| o.query.clone());
        let q = query.to_lowercase();
        let filtered: Vec<usize> = if q.is_empty() {
            (0..commands.len()).collect()
        } else {
            commands
                .iter()
                .enumerate()
                .filter_map(|(i, c)| {
                    let lower = c.0.to_lowercase();
                    let mut qi = 0usize;
                    for ch in lower.chars() {
                        if qi < q.len() && ch == q.as_bytes()[qi] as char {
                            qi += 1;
                        }
                    }
                    if qi == q.len() { Some(i) } else { None }
                })
                .collect()
        };

        let block = Self::picker_panel(
            app,
            " Commands ",
            Some("type: filter   \u{2191}/\u{2193}: navigate   Enter: run   Esc: close"),
        );
        let inner = block.inner(area);
        f.render_widget(block, area);

        let cursor_style = cursor_span_style(app);
        let search_line = Line::from(vec![
            Span::styled(" > ", Style::default().fg(app.theme.fg_dim)),
            Span::styled(query.as_str(), Style::default().fg(app.theme.fg)),
            Span::styled(" ", cursor_style.unwrap_or_default()),
        ]);

        let show_groups = q.is_empty();
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
                    format!("  \u{2500}\u{2500} {} \u{2500}\u{2500}", gname),
                    Style::default()
                        .fg(app.theme.accent)
                        .add_modifier(Modifier::BOLD),
                )));
                lines.push(Line::from(""));
                row_line += 3;
                continue;
            }
            let ci = cmd.unwrap_or(0);
            let (name, key, _) = commands[ci];
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
                app.mouse_map
                    .register(row_rect, crate::mouse::MouseZone::PickerItem(*ci));
            }
            row_line += 1;
        }

        let para = Paragraph::new(lines);
        f.render_widget(para, inner);
    }

    fn render_equalizer_picker(f: &mut ratatui::Frame, area: Rect, app: &mut App) {
        let presets = [
            ("Flat", "Neutral, uncoloured response", EqPreset::Flat),
            ("Normal", "Balanced all-rounder", EqPreset::Normal),
            ("Pop", "Vocal-forward with a lively top end", EqPreset::Pop),
            (
                "Rock",
                "Aggressive mids for punch and drive",
                EqPreset::Rock,
            ),
            ("Jazz", "Smooth mids and sparkly highs", EqPreset::Jazz),
            ("Classical", "Wide, airy and natural", EqPreset::Classical),
            ("Bass", "Deep low-end emphasis", EqPreset::Bass),
            ("Vocal", "Brings voices to the front", EqPreset::Vocal),
            (
                "Electronic",
                "Tight, modern club sound",
                EqPreset::Electronic,
            ),
            ("Hip-Hop", "Heavy bass and crisp highs", EqPreset::HipHop),
            ("Latin", "Warm and rhythmic", EqPreset::Latin),
            ("Acoustic", "Clean and intimate", EqPreset::Acoustic),
            ("Podcast", "Speech clarity over music", EqPreset::Podcast),
            ("Dance", "Pumping lows for the floor", EqPreset::Dance),
            (
                "Headphones",
                "Close-up stereo imaging",
                EqPreset::Headphones,
            ),
            ("Speaker", "Room-filling broad response", EqPreset::Speaker),
        ];

        let sel = app
            .pickers
            .top()
            .map_or(0, |o| o.selected.min(presets.len() - 1));

        let block = Self::picker_panel(
            app,
            " Equalizer ",
            Some("\u{2191}/\u{2193}: preset (applies live)   Enter: apply   Esc: close"),
        );
        let inner = block.inner(area);
        f.render_widget(block, area);

        let preview_height: u16 = 4;
        let list_h = inner.height.saturating_sub(preview_height);
        let list_area = Rect {
            x: inner.x,
            y: inner.y,
            width: inner.width,
            height: list_h,
        };
        let preview_area = Rect {
            x: inner.x,
            y: inner.y + list_h,
            width: inner.width,
            height: inner.height.saturating_sub(list_h),
        };

        let visible = list_h as usize;
        let total = presets.len();
        let (scroll_start, scroll_end) = if let Some(top) = app.pickers.top_mut() {
            let (s, e) = step_viewport(top.viewport_offset, sel, visible, total);
            top.viewport_offset = s;
            (s, e)
        } else {
            (0, total)
        };

        let mut list_items: Vec<ListItem> = Vec::new();
        for (i, (name, desc, _eq)) in presets
            .iter()
            .enumerate()
            .skip(scroll_start)
            .take(scroll_end - scroll_start)
        {
            let is_sel = i == sel;
            let prefix = if is_sel { " > " } else { "   " };
            let style = if is_sel {
                Style::default()
                    .fg(app.theme.selection_fg_readable())
                    .bg(app.theme.selection_bg)
            } else if *name == app.state.eq_preset.label() {
                Style::default().fg(app.theme.success)
            } else {
                Style::default()
            };
            let mut spans = vec![Span::styled(format!("{prefix}{}", name), Style::default())];
            spans.push(Span::styled(
                format!("  \u{2014} {desc}"),
                Style::default().fg(app.theme.fg_dim),
            ));
            list_items.push(ListItem::new(Line::from(spans)).style(style));
        }

        let list = List::new(list_items);
        f.render_widget(list, list_area);

        if preview_area.height >= 3 {
            let selected_name = presets.get(sel).map(|p| p.0).unwrap_or("");
            let rule = Line::from(vec![
                Span::styled(
                    "\u{2500}".to_string(),
                    Style::default().fg(app.theme.muted_border),
                ),
                Span::styled(
                    format!(" {selected_name} "),
                    Style::default()
                        .fg(app.theme.accent)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    "\u{2500}".repeat(
                        preview_area
                            .width
                            .saturating_sub(selected_name.len() as u16 + 4)
                            .max(1) as usize,
                    ),
                    Style::default().fg(app.theme.muted_border),
                ),
            ]);
            f.render_widget(
                Paragraph::new(rule),
                Rect {
                    x: preview_area.x,
                    y: preview_area.y,
                    width: preview_area.width,
                    height: 1,
                },
            );

            let selected_eq = presets.get(sel).map(|p| p.2);

            let mut preview_spans = Vec::new();
            if let Some(eq) = selected_eq {
                preview_spans.extend(Self::eq_preset_preview(eq, app));
            }
            f.render_widget(
                Paragraph::new(Line::from(preview_spans)),
                Rect {
                    x: preview_area.x,
                    y: preview_area.y + 1,
                    width: preview_area.width,
                    height: 1,
                },
            );
        }
    }

    fn eq_preset_preview(eq: EqPreset, app: &App) -> Vec<Span<'static>> {
        const BLOCKS: [char; 8] = ['▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];
        const BRAILLE: [char; 8] = ['⠁', '⠃', '⠇', '⡇', '⣇', '⣧', '⣷', '⣿'];
        let chars: &[char; 8] = match app.visualizer.preset {
            crate::visualizer::VisualizerPreset::Braille
            | crate::visualizer::VisualizerPreset::Gradient => &BRAILLE,
            _ => &BLOCKS,
        };
        let mut spans = vec![Span::raw("  ")];
        for g in eq.to_gains() {
            let norm = (((g.clamp(-4.0, 4.0) + 4.0) / 8.0) * 7.0).round() as usize;
            let color = if g > 0.75 {
                app.theme.success
            } else if g < -0.75 {
                app.theme.warning
            } else {
                app.theme.fg_dim
            };
            spans.push(Span::styled(
                chars[norm.min(7)].to_string(),
                Style::default().fg(color),
            ));
        }
        spans
    }

    fn visualizer_preview_lines(
        preset: crate::visualizer::VisualizerPreset,
        bars: &[f32],
        width: u16,
        app: &App,
    ) -> Vec<Line<'static>> {
        let w = width as usize;
        let mut lines = Vec::new();

        match preset {
            crate::visualizer::VisualizerPreset::Braille => {
                for row in 0..2 {
                    let mut spans = Vec::with_capacity(w);
                    for &b in bars.iter().take(w) {
                        let level = (b * 4.0) as u32;
                        let ch = match row {
                            0 => {
                                if level >= 4 {
                                    '⣿'
                                } else if level >= 3 {
                                    '⣷'
                                } else if level >= 2 {
                                    '⣧'
                                } else if level >= 1 {
                                    '⣇'
                                } else {
                                    '⠀'
                                }
                            }
                            _ => {
                                if level >= 2 {
                                    '⣿'
                                } else if level >= 1 {
                                    '⡇'
                                } else {
                                    '⠀'
                                }
                            }
                        };
                        spans.push(Span::styled(
                            ch.to_string(),
                            Style::default().fg(app.theme.accent),
                        ));
                    }
                    lines.push(Line::from(spans));
                }
            }
            crate::visualizer::VisualizerPreset::Blocks
            | crate::visualizer::VisualizerPreset::Mirror => {
                let levels = ['▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];
                for row in 0..2 {
                    let mut spans = Vec::with_capacity(w);
                    for &b in bars.iter().take(w) {
                        let idx = ((b * 7.0).round() as usize).min(7);
                        let ch = if row == 0 {
                            levels[idx]
                        } else {
                            levels[7 - idx]
                        };
                        let color = if b > 0.7 {
                            app.theme.warning
                        } else {
                            app.theme.accent
                        };
                        spans.push(Span::styled(ch.to_string(), Style::default().fg(color)));
                    }
                    lines.push(Line::from(spans));
                }
            }
            crate::visualizer::VisualizerPreset::Gradient => {
                for row in 0..2 {
                    let mut spans = Vec::with_capacity(w);
                    for (i, &b) in bars.iter().take(w).enumerate() {
                        let t = i as f64 / w.max(1) as f64;
                        let color = if t < 0.33 {
                            app.theme.accent
                        } else if t < 0.66 {
                            app.theme.secondary_accent
                        } else {
                            app.theme.tertiary_accent
                        };
                        let ch = if row == 0 {
                            if b > 0.5 {
                                '█'
                            } else if b > 0.25 {
                                '▄'
                            } else {
                                '▁'
                            }
                        } else {
                            if b > 0.5 {
                                '█'
                            } else if b > 0.25 {
                                '▀'
                            } else {
                                '▔'
                            }
                        };
                        spans.push(Span::styled(ch.to_string(), Style::default().fg(color)));
                    }
                    lines.push(Line::from(spans));
                }
            }
            crate::visualizer::VisualizerPreset::Spectrum => {
                for row in 0..2 {
                    let mut spans = Vec::with_capacity(w);
                    for &b in bars.iter().take(w) {
                        let level = (b * 7.0).round() as usize;
                        let ch = if row == 0 {
                            ['▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'][level.min(7)]
                        } else {
                            ['█', '▇', '▆', '▅', '▄', '▃', '▂', '▁'][level.min(7)]
                        };
                        let color = ratatui::style::Color::Rgb(
                            (b * 200.0) as u8,
                            (b * 120.0 + 40.0) as u8,
                            (120.0 - b * 80.0) as u8,
                        );
                        spans.push(Span::styled(ch.to_string(), Style::default().fg(color)));
                    }
                    lines.push(Line::from(spans));
                }
            }
        }
        lines
    }

    fn render_theme_picker_picker(f: &mut ratatui::Frame, area: Rect, app: &mut App) {
        let query = app.pickers.top().map_or(String::new(), |o| o.query.clone());
        let q = query.to_lowercase();

        let filtered: Vec<_> = if q.is_empty() {
            app.themes.iter().enumerate().collect()
        } else {
            app.themes
                .iter()
                .enumerate()
                .filter(|(_, entry)| {
                    let lower = entry.name.to_lowercase();
                    let mut qi = 0usize;
                    for ch in lower.chars() {
                        if qi < q.len() && ch == q.as_bytes()[qi] as char {
                            qi += 1;
                        }
                    }
                    qi == q.len()
                })
                .collect()
        };

        let total = filtered.len();
        let sel = app
            .pickers
            .top()
            .map_or(0, |o| o.selected.min(total.saturating_sub(1)));

        let block = Self::picker_panel(
            app,
            " Theme ",
            Some("type: filter   \u{2191}/\u{2193}: preview   Enter: apply   Esc: close"),
        );
        let inner = block.inner(area);
        f.render_widget(block, area);

        let cursor_style = cursor_span_style(app);
        let search_line = Line::from(vec![
            Span::styled(" > ", Style::default().fg(app.theme.fg_dim)),
            Span::styled(query.as_str(), Style::default().fg(app.theme.fg)),
            Span::styled(" ", cursor_style.unwrap_or_default()),
        ]);

        let visible = inner.height.saturating_sub(1).checked_div(2).unwrap_or(0) as usize;
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

        let mut list_items: Vec<ListItem> = vec![ListItem::new(search_line)];
        let row_w = inner.width as usize;
        let mut first_row = true;
        for (visible_idx, &(i, entry)) in filtered[scroll_start..scroll_end].iter().enumerate() {
            if !first_row {
                let rule = Span::styled(
                    "  ".to_string() + &"\u{2500}".repeat(row_w.saturating_sub(6)),
                    Style::default().fg(app.theme.fg_dim),
                );
                list_items.push(ListItem::new(Line::from(rule)));
            }
            first_row = false;

            let is_active = i == app.theme_index;
            let prefix = if i == sel { " > " } else { "   " };
            let check = if is_active { " \u{2713}" } else { "" };
            let light_badge = if entry.light { " \u{2600}" } else { "" };
            let name_part = format!("{}{}{}", prefix, entry.name, light_badge);

            let colors = [
                entry.theme.bg,
                entry.theme.fg,
                entry.theme.accent,
                entry.theme.secondary_accent,
                entry.theme.tertiary_accent,
                entry.theme.border,
            ];
            let swatch_run_w = colors.len() * 2 + colors.len() + 1;
            let left_w = name_part.chars().count() + check.chars().count() + 1;
            let pad = row_w.saturating_sub(left_w + swatch_run_w + 2);

            let name_style = if i == sel {
                Style::default()
                    .fg(app.theme.selection_fg_readable())
                    .bg(app.theme.selection_bg)
            } else {
                Style::default()
            };
            let mut spans: Vec<Span> = vec![Span::styled(
                format!("{name_part}{:pad$}", "", pad = pad),
                name_style,
            )];
            for c in colors {
                spans.push(Span::styled("  ", Style::default().fg(c).bg(c)));
                spans.push(Span::raw(" "));
            }
            spans.push(Span::styled(check, Style::default()));
            let style = if i == sel {
                Style::default()
            } else if is_active {
                Style::default()
                    .fg(app.theme.accent)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default()
            };
            list_items.push(ListItem::new(Line::from(spans)).style(style));
            let row_rect = Rect {
                x: inner.x,
                y: inner.y + 1 + (2 * visible_idx) as u16,
                width: inner.width,
                height: 1,
            };
            app.mouse_map
                .register(row_rect, crate::mouse::MouseZone::PickerItem(i));
        }

        let list = List::new(list_items);
        f.render_widget(list, inner);
    }

    fn render_crossfade_picker(f: &mut ratatui::Frame, area: Rect, app: &App) {
        let block = Self::picker_panel(
            app,
            " Crossfade Options ",
            Some("\u{2191}/\u{2193}: navigate   Enter: apply   Esc: close"),
        );
        let inner = block.inner(area);
        f.render_widget(block, area);

        let dur = app
            .state
            .crossfade
            .as_ref()
            .map(|c| c.duration_secs)
            .unwrap_or(0);

        let mut rows: Vec<String> = Vec::new();
        rows.push(" Duration ".to_string());
        for d in CROSSFADE_DURATIONS {
            let cur = if d == dur { "   (current)" } else { "" };
            rows.push(format!("   {d}s{cur}"));
        }

        let sel = app
            .pickers
            .top()
            .map_or(0, |o| o.selected.min(rows.len() - 1));
        let mut lines = Vec::new();
        for (i, row) in rows.iter().enumerate() {
            let is_header = i == 0;
            let is_sel = i == sel;
            let line = if is_header {
                Line::from(Span::styled(
                    row.clone(),
                    Style::default()
                        .fg(app.theme.accent)
                        .add_modifier(Modifier::BOLD),
                ))
            } else {
                let prefix = if is_sel { " > " } else { "   " };
                let style = if is_sel {
                    Style::default()
                        .fg(app.theme.selection_fg_readable())
                        .bg(app.theme.selection_bg)
                } else {
                    Style::default()
                };
                Line::from(Span::styled(format!("{prefix}{row}"), style))
            };
            lines.push(line);
        }

        lines.push(Line::from(""));
        if sel >= 1 && sel < 1 + CROSSFADE_DURATIONS.len() {
            lines.push(Line::from(Span::styled(
                " Select a crossfade duration. 3s is subtle, 30s is ambient.",
                Style::default().fg(app.theme.fg_dim),
            )));
        } else {
            lines.push(Line::from(Span::styled(
                " Choose a crossfade duration.",
                Style::default().fg(app.theme.fg_dim),
            )));
        }

        let para = Paragraph::new(lines);
        f.render_widget(para, inner);
    }

    fn render_visualizer_preset_picker(f: &mut ratatui::Frame, area: Rect, app: &mut App) {
        let block = Self::picker_panel(
            app,
            " Visualizer Preset ",
            Some("\u{2191}/\u{2193}: preview   Enter: apply   Esc: close"),
        );
        let inner = block.inner(area);
        f.render_widget(block, area);

        let current = app.visualizer.preset;
        let presets = crate::visualizer::VisualizerPreset::all();

        let sel = app
            .pickers
            .top()
            .map_or(0, |o| o.selected.min(presets.len() - 1));

        let preview_height: u16 = 4;
        let list_h = inner.height.saturating_sub(preview_height);
        let list_area = Rect {
            x: inner.x,
            y: inner.y,
            width: inner.width,
            height: list_h,
        };
        let preview_area = Rect {
            x: inner.x,
            y: inner.y + list_h,
            width: inner.width,
            height: inner.height.saturating_sub(list_h),
        };

        let visible = list_h as usize;
        let total = presets.len();
        let (scroll_start, scroll_end) = if let Some(top) = app.pickers.top_mut() {
            let (s, e) = step_viewport(top.viewport_offset, sel, visible, total);
            top.viewport_offset = s;
            (s, e)
        } else {
            (0, total)
        };

        let mut lines = Vec::new();
        for (i, preset) in presets
            .iter()
            .enumerate()
            .skip(scroll_start)
            .take(scroll_end - scroll_start)
        {
            let is_sel = i == sel;
            let is_cur = *preset == current;
            let prefix = if is_sel { " > " } else { "   " };
            let cur = if is_cur { "   (current)" } else { "" };
            let style = if is_sel {
                Style::default()
                    .fg(app.theme.selection_fg_readable())
                    .bg(app.theme.selection_bg)
            } else if is_cur {
                Style::default().fg(app.theme.accent)
            } else {
                Style::default()
            };
            lines.push(Line::from(Span::styled(
                format!("{prefix}{}{cur}", preset.name()),
                style,
            )));
        }

        let list = List::new(lines.into_iter().map(ListItem::new).collect::<Vec<_>>());
        f.render_widget(list, list_area);

        if preview_area.height >= 3 {
            let selected_name = presets.get(sel).map(|p| p.name()).unwrap_or("");
            let rule = Line::from(vec![
                Span::styled(
                    "\u{2500}".to_string(),
                    Style::default().fg(app.theme.muted_border),
                ),
                Span::styled(
                    format!(" {selected_name} "),
                    Style::default()
                        .fg(app.theme.accent)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    "\u{2500}".repeat(
                        preview_area
                            .width
                            .saturating_sub(selected_name.len() as u16 + 4)
                            .max(1) as usize,
                    ),
                    Style::default().fg(app.theme.muted_border),
                ),
            ]);
            f.render_widget(
                Paragraph::new(rule),
                Rect {
                    x: preview_area.x,
                    y: preview_area.y,
                    width: preview_area.width,
                    height: 1,
                },
            );

            let sel_preset = presets.get(sel).copied().unwrap_or(current);
            let bar_w = preview_area.width.saturating_sub(4) as usize;
            let sample_bars: Vec<f32> = (0..bar_w)
                .map(|i| {
                    let t = i as f64 / bar_w.max(1) as f64;
                    ((t * std::f64::consts::TAU).sin() * 0.5 + 0.5) as f32
                })
                .collect();

            let preview_lines =
                Self::visualizer_preview_lines(sel_preset, &sample_bars, preview_area.width, app);
            for (row, line) in preview_lines
                .iter()
                .enumerate()
                .take(preview_area.height.saturating_sub(1) as usize)
            {
                f.render_widget(
                    Paragraph::new(line.clone()),
                    Rect {
                        x: preview_area.x,
                        y: preview_area.y + 1 + row as u16,
                        width: preview_area.width,
                        height: 1,
                    },
                );
            }
        }
    }

    fn render_progress_style_picker(f: &mut ratatui::Frame, area: Rect, app: &mut App) {
        let block = Self::picker_panel(
            app,
            " Progress Style ",
            Some("\u{2191}/\u{2193}: preview   Enter: apply   Esc: close"),
        );
        let inner = block.inner(area);
        f.render_widget(block, area);

        let current = app.progress_style;
        let styles = crate::progress::ProgressStyle::all();

        let sel = app
            .pickers
            .top()
            .map_or(0, |o| o.selected.min(styles.len() - 1));

        let preview_height: u16 = 4;
        let list_h = inner.height.saturating_sub(preview_height);
        let list_area = Rect {
            x: inner.x,
            y: inner.y,
            width: inner.width,
            height: list_h,
        };
        let preview_area = Rect {
            x: inner.x,
            y: inner.y + list_h,
            width: inner.width,
            height: inner.height.saturating_sub(list_h),
        };

        let visible = list_h as usize;
        let total = styles.len();
        let (scroll_start, scroll_end) = if let Some(top) = app.pickers.top_mut() {
            let (s, e) = step_viewport(top.viewport_offset, sel, visible, total);
            top.viewport_offset = s;
            (s, e)
        } else {
            (0, total)
        };

        let mut lines = Vec::new();
        for (i, style) in styles
            .iter()
            .enumerate()
            .skip(scroll_start)
            .take(scroll_end - scroll_start)
        {
            let is_sel = i == sel;
            let is_cur = *style == current;
            let prefix = if is_sel { " > " } else { "   " };
            let cur = if is_cur { "   (current)" } else { "" };
            let line_style = if is_sel {
                Style::default()
                    .fg(app.theme.selection_fg_readable())
                    .bg(app.theme.selection_bg)
            } else if is_cur {
                Style::default().fg(app.theme.accent)
            } else {
                Style::default()
            };
            lines.push(Line::from(Span::styled(
                format!("{prefix}{}{cur}", style.name()),
                line_style,
            )));
        }

        let list = List::new(lines.into_iter().map(ListItem::new).collect::<Vec<_>>());
        f.render_widget(list, list_area);

        if preview_area.height >= 3 {
            let selected_name = styles.get(sel).map(|s| s.name()).unwrap_or("");
            let rule = Line::from(vec![
                Span::styled(
                    "\u{2500}".to_string(),
                    Style::default().fg(app.theme.muted_border),
                ),
                Span::styled(
                    format!(" {selected_name} "),
                    Style::default()
                        .fg(app.theme.accent)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    "\u{2500}".repeat(
                        preview_area
                            .width
                            .saturating_sub(selected_name.len() as u16 + 4)
                            .max(1) as usize,
                    ),
                    Style::default().fg(app.theme.muted_border),
                ),
            ]);
            f.render_widget(
                Paragraph::new(rule),
                Rect {
                    x: preview_area.x,
                    y: preview_area.y,
                    width: preview_area.width,
                    height: 1,
                },
            );

            let sel_style = styles.get(sel).copied().unwrap_or(current);
            let preview_w = preview_area.width.saturating_sub(2) as usize;
            let spans = crate::progress::render_progress_styled(
                0.6,
                preview_w,
                sel_style,
                app.theme.accent,
                app.theme.secondary_accent,
                app.theme.tertiary_accent,
            );
            f.render_widget(
                Paragraph::new(Line::from(spans)),
                Rect {
                    x: preview_area.x,
                    y: preview_area.y + 1,
                    width: preview_area.width,
                    height: 1,
                },
            );

            let preview_label = Line::from(Span::styled(
                format!("  60% filled, {} chars wide", preview_w),
                Style::default().fg(app.theme.fg_dim),
            ));
            f.render_widget(
                Paragraph::new(preview_label),
                Rect {
                    x: preview_area.x,
                    y: preview_area.y + 2,
                    width: preview_area.width,
                    height: 1,
                },
            );
        }
    }

    fn render_footer_preset_picker(f: &mut ratatui::Frame, area: Rect, app: &mut App) {
        let block = Self::picker_panel(
            app,
            " Footer Preset ",
            Some("\u{2191}/\u{2193}: preview   Enter: apply   Esc: close"),
        );
        let inner = block.inner(area);
        f.render_widget(block, area);

        let current = app.footer_preset;
        let presets = &app.footer_presets;

        let sel = app
            .pickers
            .top()
            .map_or(0, |o| o.selected.min(presets.len().saturating_sub(1)));

        let preview_height: u16 = 4;
        let list_h = inner.height.saturating_sub(preview_height);
        let list_area = Rect {
            x: inner.x,
            y: inner.y,
            width: inner.width,
            height: list_h,
        };
        let preview_area = Rect {
            x: inner.x,
            y: inner.y + list_h,
            width: inner.width,
            height: inner.height.saturating_sub(list_h),
        };

        let visible = list_h as usize;
        let total = presets.len();
        let (scroll_start, scroll_end) = if let Some(top) = app.pickers.top_mut() {
            let (s, e) = step_viewport(top.viewport_offset, sel, visible, total);
            top.viewport_offset = s;
            (s, e)
        } else {
            (0, total)
        };

        let mut lines = Vec::new();
        for (i, preset) in presets
            .iter()
            .enumerate()
            .skip(scroll_start)
            .take(scroll_end.saturating_sub(scroll_start))
        {
            let is_sel = i == sel;
            let is_cur = i == current;
            let prefix = if is_sel { " > " } else { "   " };
            let cur = if is_cur { "   (current)" } else { "" };
            let line_style = if is_sel {
                Style::default()
                    .fg(app.theme.selection_fg_readable())
                    .bg(app.theme.selection_bg)
            } else if is_cur {
                Style::default().fg(app.theme.accent)
            } else {
                Style::default()
            };
            lines.push(Line::from(Span::styled(
                format!("{prefix}{}{cur}", preset.name),
                line_style,
            )));
        }

        let list = List::new(lines.into_iter().map(ListItem::new).collect::<Vec<_>>());
        f.render_widget(list, list_area);

        if preview_area.height >= 3 {
            let layout = presets
                .get(sel)
                .map(|p| {
                    let mut n = p.left.len() + p.right.len();
                    if n == 0 {
                        n = 1;
                    }
                    format!(
                        "left {} \u{2022} right {} \u{2022} {} module{}",
                        p.left.len(),
                        p.right.len(),
                        n,
                        if n == 1 { "" } else { "s" }
                    )
                })
                .unwrap_or_default();
            let rule = Line::from(vec![
                Span::styled(
                    "\u{2500}".to_string(),
                    Style::default().fg(app.theme.muted_border),
                ),
                Span::styled(format!(" {layout} "), Style::default().fg(app.theme.fg_dim)),
                Span::styled(
                    "\u{2500}".repeat(
                        preview_area
                            .width
                            .saturating_sub(layout.len() as u16 + 4)
                            .max(1) as usize,
                    ),
                    Style::default().fg(app.theme.muted_border),
                ),
            ]);
            f.render_widget(
                Paragraph::new(rule),
                Rect {
                    x: preview_area.x,
                    y: preview_area.y,
                    width: preview_area.width,
                    height: 1,
                },
            );

            // Live preview of the selected preset rendered as brand-badge
            // style per-module swatches, so the user sees the real layout.
            let preview_label = Line::from(Span::styled(
                "  dragging the selection previews the footer layout",
                Style::default().fg(app.theme.fg_dim),
            ));
            f.render_widget(
                Paragraph::new(preview_label),
                Rect {
                    x: preview_area.x,
                    y: preview_area.y + 1,
                    width: preview_area.width,
                    height: 1,
                },
            );
        }
    }
}

fn format_duration_short(secs: u64) -> String {
    let h = secs / 3600;
    let m = (secs % 3600) / 60;
    let s = secs % 60;
    if h > 0 {
        format!("{h}:{m:02}:{s:02}")
    } else {
        format!("{m:02}:{s:02}")
    }
}

// ─── Aesthetic Helpers ───

fn plural(count: usize, singular: &'static str, plural: &'static str) -> &'static str {
    if count == 1 { singular } else { plural }
}

const LOADER_BRAILLE: [&str; 10] = [
    "\u{280b}", "\u{2819}", "\u{2839}", "\u{2838}", "\u{283c}", "\u{2834}", "\u{2826}", "\u{2827}",
    "\u{2807}", "\u{280f}",
];

fn opencode_spinner(frame: usize) -> &'static str {
    LOADER_BRAILLE[(frame / 2) % LOADER_BRAILLE.len()]
}

pub use crate::theme::readable_fg;

/// Frames (at ~60 fps) spent stationary after each full marquee loop before
/// the title starts animating again.
const SCROLL_HOLD_FRAMES: usize = 300;
/// Frames per scroll step; larger = slower animation.
const SCROLL_SPEED: usize = 6;

fn scroll_text(text: &str, max_width: usize, frame: usize, is_selected: bool) -> String {
    if text.chars().count() <= max_width {
        return format!("{:<width$}", text, width = max_width);
    }
    if !is_selected {
        let truncated: String = text.chars().take(max_width.saturating_sub(1)).collect();
        return format!("{}…", truncated);
    }
    // Cycle the marquee through n offsets, then hold still for
    // `SCROLL_HOLD_FRAMES / SCROLL_SPEED` steps before looping again.
    let chars: Vec<char> = text.chars().collect();
    let n = chars.len();
    let hold_steps = SCROLL_HOLD_FRAMES / SCROLL_SPEED;
    let step = frame / SCROLL_SPEED;
    let pos = step % (n + hold_steps).max(1);
    let scroll = if pos < n { pos } else { 0 };
    let scrolled: String = chars
        .iter()
        .skip(scroll)
        .chain(chars.iter().take(scroll))
        .collect();
    scrolled.chars().take(max_width).collect()
}

// ─── Library Motion Overlays ───

impl Pickers {
    fn render_playlist_select_picker(f: &mut ratatui::Frame, area: Rect, app: &App) {
        let help = if app.playlist_creating {
            "type: name   Enter: create & add   Esc: back"
        } else {
            "\u{2191}/\u{2193}: choose   n: new   Enter: add   Esc: cancel"
        };
        let block = Self::picker_panel(app, " Select Playlist ", Some(help));
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

    /// Multi-select track picker shown right after a playlist is created:
    /// every track is listed and `Space` toggles a persistent highlight, with
    /// `Ctrl+Enter` committing the selection. Highlights survive scrolling.
    fn render_playlist_track_select_picker(f: &mut ratatui::Frame, area: Rect, app: &mut App) {
        let selected = app.selected_playlist_track_ids.len();
        let hint = format!(
            "Space/Tab: toggle   \u{2191}/\u{2193}: navigate   Ctrl+Enter: add {} to playlist   Esc: cancel",
            if selected > 0 {
                format!("({selected} selected)")
            } else {
                String::new()
            }
        );
        let block = Self::picker_panel(app, " Add Tracks ", Some(hint.as_str()));
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
            let is_picked = app.selected_playlist_track_ids.contains(&track.id);
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
            app.mouse_map
                .register(row_rect, crate::mouse::MouseZone::PickerItem(i));
            items.push(ListItem::new(content).style(style));
        }

        let list = List::new(items);
        f.render_widget(list, inner);
    }

    fn render_edit_metadata_picker(f: &mut ratatui::Frame, area: Rect, app: &mut App) {
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
            if let Some(ref mut protocol) = app.metadata.cover_stateful {
                let image = StatefulImage::new();
                f.render_stateful_widget(image, c_area, protocol);
            } else if let Some(ref cover_bytes) = app.metadata.cover {
                Render::cover_block(f, c_area, cover_bytes);
            } else {
                let placeholder = Paragraph::new(Line::from(Span::styled(
                    " \u{266b} no cover ",
                    Style::default().fg(app.theme.fg_dim),
                )))
                .alignment(Alignment::Center);
                f.render_widget(placeholder, c_area);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::scroll_text;

    #[test]
    fn scroll_text_handles_multibyte_utf8() {
        let text = "Artist \u{2014} T\u{e9}t\u{e9} Song Title That Is Quite Long";
        for frame in 0..600 {
            for width in [8usize, 16, 24] {
                let out = scroll_text(text, width, frame, true);
                assert!(out.chars().count() <= width, "frame {frame} width {width}");
            }
            let out = scroll_text(text, 16, frame, false);
            assert!(out.chars().count() <= 16);
        }
    }

    #[test]
    fn scroll_text_pads_when_fits() {
        assert_eq!(scroll_text("ab", 4, 0, true), "ab  ");
    }

    #[test]
    fn scroll_text_empty_never_panics() {
        let out = scroll_text("", 0, 1, true);
        assert_eq!(out, "");
    }
}
