// Copyright (c) 2026
// Author: prjctimg <prjctimg@outlook.com>
// TUI rendering: layout, widgets, and theme application
//
// This is free software released under the GPL-3.0 license.

use std::borrow::Cow;
use std::path::PathBuf;

use crate::app::fuzzy_match;
use crate::app::{
    App, InputMode, LIBRARY_CATEGORIES, LibraryPick, NotifMode, NotifType, NotificationKind,
    RadioPick, RadioSection, TrackInfoKind, ZenSurface, folder_name, lyrics_are_synced,
    no_image_protocol, setup_selection,
};
use crate::extensions::ExtensionId;
use crate::footer::{
    classify_remote_source, draw as footer_draw, format_duration, format_uptime, is_live_stream,
    render as footer_render,
};
use crate::mouse::MouseZone;
use crate::picker::{Picker, PickerId, PickerSource};
use crate::progress::{ProgressStyle, render_progress, render_progress_styled, render_ratio};
use crate::shared::daemon::ensure_daemon_running;
use crate::shared::global::{EqPreset, PlaybackStatus};
use crate::shared::ipc::HealthStatus;
use crate::shared::log::redirect_stderr;
use crate::shared::radio::RadioStation;
use crate::shared::resolve_command_socket;
use crate::shared::spotify::SpotifySearchKind;
use crate::shared::track::{LrcData, TrackInfo};
use crate::theme::blend_colors;
pub use crate::theme::readable_fg;
use crate::visualizer::VisualizerPreset;
use crossterm::event::{
    DisableBracketedPaste, DisableMouseCapture, EnableBracketedPaste, EnableMouseCapture,
};
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
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

    fn notification_overlay(f: &mut ratatui::Frame, area: Rect, app: &mut App) {
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

    fn content(f: &mut ratatui::Frame, area: Rect, app: &mut App) {
        if !app.is_ready {
            fill_pane(f, area, app);
            Render::loader(f, area, app, "Loading library\u{2026}");
            return;
        }
        Render::library(f, area, app);
    }

    /// Zen mode: render exactly one fullscreen surface at a time — the
    /// enlarged cover + centered progress, the visualizer, or the lyrics.
    fn zen(f: &mut ratatui::Frame, area: Rect, app: &mut App) {
        match app.zen_surface {
            ZenSurface::Cover => Render::zen_cover(f, area, app),
            ZenSurface::Visualizer => Render::zen_visualizer(f, area, app),
            ZenSurface::Lyrics => Render::zen_lyrics(f, area, app),
        }
    }

    /// Centered "title — artist" header used by the Zen cover and lyrics
    /// surfaces.
    fn zen_track_header(f: &mut ratatui::Frame, app: &App, track: &TrackInfo, area: Rect) {
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
    fn zen_cover(f: &mut ratatui::Frame, area: Rect, app: &mut App) {
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
    fn zen_visualizer(f: &mut ratatui::Frame, area: Rect, app: &mut App) {
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
    fn zen_lyrics(f: &mut ratatui::Frame, area: Rect, app: &mut App) {
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

                let display_title = if track.title.is_empty() {
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
                let display_artist = if track.artist.is_empty() {
                    " "
                } else {
                    &track.artist
                };
                let has_album = !track.album.is_empty();

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

    fn footer(f: &mut ratatui::Frame, area: Rect, app: &mut App) {
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

    fn lyrics_pane(f: &mut ratatui::Frame, area: Rect, app: &mut App) {
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
    fn lyrics_body(f: &mut ratatui::Frame, lyrics_inner: Rect, app: &App, lyrics: &LrcData) {
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
    fn list_in_info(f: &mut ratatui::Frame, area: Rect, app: &mut App) {
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

    fn info_in_pane(f: &mut ratatui::Frame, sep_area: Rect, area: Rect, app: &mut App) {
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

fn render_pending_prompt(f: &mut ratatui::Frame, area: Rect, app: &App) {
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

impl Render {
    pub fn pending_prompt(f: &mut ratatui::Frame, area: Rect, app: &mut App) {
        render_pending_prompt(f, area, app);
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
    "\u{f001}",
    "\u{f004}",
    "\u{f025}",
    "\u{f007}",
    "\u{f03a}",
    "\u{f04c7}",
    "\u{f0439}", // Radio: nf-md-radio (official MDI)
    "\u{f0128}", // Most Played: nf-md-chart_bar (verified: U+F0120 was tray-arrow-down)
    "\u{f02da}", // Recently Played: nf-md-history (official MDI)
    "\u{f1da}",
    "\u{f04fb}", // Genres: nf-md-tag_multiple (official MDI)
    "\u{f07b}",
    "\u{f0535}", // Top Charts: nf-md-trending_up (official MDI)
];

const LIBRARY_ICONS_ASCII: &[&str] = &[
    "♫", "♥", "▤", "♪", "≡", "☊", "◉", "▥", "◆", "♫", "◎", "▽", "#",
];

pub(crate) fn use_nerd_fonts() -> bool {
    !matches!(std::env::var("GTM_NERD_FONTS"), Ok(v) if v == "0" || v == "false" || v == "no")
}

/// Brand/source glyph for a provider name, verified against the glyphs present
/// in the pinned JetBrainsMono Nerd Font (no tofu). Returns `None` for any
/// provider without a distinct glyph — including community providers — so
/// callers keep their own ASCII/emoji fallback and new providers never risk
/// a tofu box.
pub(crate) fn provider_icon(name: &str) -> Option<&'static str> {
    match name {
        "Spotify" => Some("\u{f04c7}"),   // nf-md-spotify
        "YouTube" => Some("\u{f16a}"),    // nf-fa-youtube
        "Podcast" => Some("\u{f0994}"),   // nf-md-podcast
        "Radio" => Some("\u{f0439}"),     // nf-md-radio
        "SoundCloud" => Some("\u{f1be}"), // nf-fa-soundcloud
        "Bandcamp" => Some("\u{f2d5}"),   // nf-fa-bandcamp
        "Mixcloud" => Some("\u{f289}"),   // nf-fa-mixcloud
        "Twitch" => Some("\u{f1e8}"),     // nf-fa-twitch
        "Last.fm" => Some("\u{f001}"),    // nf-md-music (no brand glyph in font)
        "Local" => Some("\u{f0a0}"),      // nf-fa-hdd
        _ => None,
    }
}

/// Human-readable label for the persisted cover-art provider string.
pub(crate) fn cover_provider_label(v: &str) -> String {
    match v.trim().to_ascii_lowercase().as_str() {
        "deezer" => "Deezer".into(),
        "musicbrainz" | "mb" => "MusicBrainz".into(),
        "spotify" => "Spotify".into(),
        _ => "Auto".into(),
    }
}

/// Human-readable label for the persisted theme-following mode.
pub(crate) fn theme_mode_label(v: &str) -> String {
    match v.trim().to_ascii_lowercase().as_str() {
        "dark" => "Dark".into(),
        "light" => "Light".into(),
        "manual" => "Manual".into(),
        _ => "Auto".into(),
    }
}

pub const COVER_W: u16 = 24;
pub const COVER_H: u16 = 12;

fn row_pad(content: &str, width: u16) -> usize {
    (width as usize).saturating_sub(content.chars().count())
}

/// Push list-relevant empty-state help into `lines`: a headline plus an
/// action hint, both dimmed. Each list explains how to populate itself.
fn empty_hint_lines(app: &App, headline: &str, hint: &str) -> Vec<Line<'static>> {
    let dim = Style::default().fg(app.theme.fg_dim);
    vec![
        Line::from(Span::styled(format!(" {headline}"), dim)),
        Line::from(Span::styled(format!(" {hint}"), dim)),
    ]
}

fn cursor_span_style(app: &App) -> Option<Style> {
    let phase = (app.frame_count % 64) as f32 / 64.0;
    let t = (1.0 - (phase * std::f32::consts::TAU).cos()) * 0.5;
    let bg = blend_colors(app.theme.selection_bg, app.float_bg(), (t * 0.85) as f64);
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

/// Shared "waiting for the OAuth browser flow" view used by both the Spotify
/// Setup walkthrough (`SpotifyLink`) and the Alt+s (`SpotifySearch`) picker
/// while an authorization request is pending or has failed.
fn spotify_waiting_lines(app: &App) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    if app.spotify.oauth_pending {
        lines.push(Line::from(Span::styled(
            "Waiting for you to finish login in your browser…",
            Style::default().fg(app.theme.fg_bright),
        )));
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            "A browser window should have opened to authorize gtm.",
            Style::default().fg(app.theme.fg_dim),
        )));
        lines.push(Line::from(Span::styled(
            "Once you approve, playlists sync automatically.",
            Style::default().fg(app.theme.fg_dim),
        )));
        lines.push(Line::from(Span::styled(
            "Still stuck? Add this redirect to your app:",
            Style::default().fg(app.theme.fg_dim),
        )));
        lines.push(Line::from(Span::styled(
            format!(
                "http://127.0.0.1:{}/login",
                app.spotify.oauth_port.parse::<u16>().unwrap_or(8990)
            ),
            Style::default().fg(app.theme.accent),
        )));
        lines.push(Line::from(""));
    }
    if let Some(err) = app.spotify.oauth_error.as_deref() {
        lines.push(Line::from(Span::styled(
            err.to_string(),
            Style::default().fg(app.theme.error),
        )));
        lines.push(Line::from(""));
    }
    if let Some(url) = app.spotify.oauth_url.as_deref() {
        lines.push(Line::from(Span::styled(
            "If your browser did not open, copy this URL:",
            Style::default().fg(app.theme.fg_dim),
        )));
        lines.push(Line::from(Span::styled(
            url.to_string(),
            Style::default().fg(app.theme.accent),
        )));
        lines.push(Line::from(""));
    }
    lines.push(Line::from(Span::styled(
        "Press Esc to cancel.",
        Style::default().fg(app.theme.fg_dim),
    )));
    lines
}

fn fill_pane(f: &mut ratatui::Frame, area: Rect, app: &App) {
    f.render_widget(
        ratatui::widgets::Block::default()
            .style(ratatui::style::Style::default().bg(app.pane_surface_bg())),
        area,
    );
}

const SETTINGS_ICONS_NERD: &[&str] = &["\u{f16a}", "\u{f04b}", "\u{f013}", "\u{f04c7}"];
const SETTINGS_ICONS_ASCII: &[&str] = &["YT", "▶", "⚙", "★"];
const SETTINGS_CATEGORIES: &[&str] = &["YouTube", "Playback", "System", "Spotify"];

// ─── Overlay Rendering ───

/// Inline icon glyph for a `gtm setup` service, matching the configured icon
/// style (mdi brand/monochrome glyphs vs. emoji).
fn service_icon_glyph(icon_style: &str, service: &str) -> &'static str {
    if icon_style == "mdi" {
        return provider_icon(service).unwrap_or("\u{f001}");
    }
    match service {
        "Spotify" => "\u{1f3a7}",
        "Last.fm" => "\u{1f3b5}",
        "YouTube" => "\u{25b6}\u{fe0f}", // same ▶️ as command palette's emoji YouTube Search
        _ => "",
    }
}

pub(crate) struct Pickers;

impl Pickers {
    fn picker_content_hint(top: &Picker, app: &App) -> (u16, u16) {
        match top.id {
            PickerId::Queue => {
                let w = app
                    .queue
                    .cache
                    .iter()
                    .map(|t| t.artist.len() as u16 + t.title.len() as u16 + 14)
                    .max()
                    .unwrap_or(46)
                    .clamp(44, 72);
                let h = (app.queue.cache.len() as u16 + 6).clamp(18, 30);
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
            PickerId::NotificationSettings => (60, 14),
            PickerId::ProgressStyle => (48, 18),
            PickerId::Settings => (64, 28),
            PickerId::Setup => (58, 24),
            PickerId::LastfmAuth => (60, 16),
            PickerId::YoutubeSetup => (58, 8),
            PickerId::PodcastFeeds => {
                let w = app
                    .podcast
                    .feeds
                    .iter()
                    .map(|f| f.title.len() as u16 + 24)
                    .max()
                    .unwrap_or(56)
                    .clamp(52, 84);
                (w, (app.podcast.feeds.len() as u16 + 6).clamp(16, 28))
            }
            PickerId::PodcastEpisodes => {
                let w = app
                    .podcast
                    .episodes
                    .iter()
                    .map(|e| e.title.len() as u16 + 16)
                    .max()
                    .unwrap_or(58)
                    .clamp(52, 86);
                (w, (app.podcast.episodes.len() as u16 + 6).clamp(18, 30))
            }
            PickerId::PodcastSubscribe => (56, 8),
            PickerId::LoadStream => (56, 8),
            PickerId::Radio => {
                // One merged panel: height follows the filtered row count;
                // width fits the longest name across every sub-list (custom
                // stations, top stations, tags, countries).
                let n = app.radio_picks().len();
                let w = app
                    .radio
                    .custom
                    .iter()
                    .map(|s| s.name.len())
                    .chain(app.radio.top.iter().map(|s| s.name.len()))
                    .chain(app.radio.browse_tags.iter().map(|t| t.name.len()))
                    .chain(app.radio.browse_countries.iter().map(|c| c.name.len()))
                    .max()
                    .map_or(64, |v| v as u16 + 48)
                    .clamp(64, 100);
                (w, (n as u16 + 9).clamp(16, 36))
            }
            _ => (56, 22),
        }
    }

    fn render_picker(f: &mut ratatui::Frame, area: Rect, app: &mut App) {
        app.update_picker_preview();
        app.update_artist_cover();
        app.update_spot_preview();
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
                    | PickerId::PodcastFeeds
                    | PickerId::PodcastEpisodes
                    | PickerId::Radio
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
            PickerId::Queue => Self::render_queue(f, picker_area, app),
            PickerId::YTSearch => Self::render_yt_search(f, picker_area, app),
            PickerId::SearchLibrary => Self::render_search_library(f, picker_area, app),
            PickerId::About => Self::render_about(f, picker_area, app),
            PickerId::SleepTimer => Self::render_sleep_timer(f, picker_area, app),
            PickerId::CommandPalette => Self::command_palette(f, picker_area, app),
            PickerId::Equalizer => Self::render_equalizer(f, picker_area, app),
            PickerId::ThemePicker => Self::render_theme(f, picker_area, app),
            PickerId::Help => Self::render_help(f, picker_area, app),
            PickerId::PlaylistSelect => Self::render_playlist_select(f, picker_area, app),
            PickerId::PlaylistTrackSelect => Self::render_track_select(f, picker_area, app),
            PickerId::EditMetadata => Self::render_edit_metadata(f, picker_area, app),
            PickerId::Crossfade => Self::render_crossfade(f, picker_area, app),
            PickerId::VisualizerPreset => Self::render_visualizer_preset(f, picker_area, app),
            PickerId::FooterPreset => Self::render_footer_preset(f, picker_area, app),
            PickerId::ProgressStyle => Self::render_progress_style(f, picker_area, app),
            PickerId::Settings => Self::render_settings(f, picker_area, app),
            PickerId::Notifications => Self::render_notifications(f, picker_area, app),
            PickerId::NotificationSettings => {
                Self::render_notification_settings(f, picker_area, app)
            }
            PickerId::PodcastFeeds => Self::render_podcast_feeds(f, picker_area, app),
            PickerId::PodcastEpisodes => Self::render_podcast_episodes(f, picker_area, app),
            PickerId::PodcastSubscribe => Self::render_podcast_subscribe(f, picker_area, app),
            PickerId::LoadStream => Self::render_load_stream(f, picker_area, app),
            PickerId::Radio => Self::render_radio(f, picker_area, app),
            PickerId::Setup => Self::render_setup(f, picker_area, app),
            PickerId::LastfmAuth => Self::render_lastfm_setup(f, picker_area, app),
            PickerId::YoutubeSetup => Self::render_youtube_setup(f, picker_area, app),
            PickerId::SpotifyLink => {
                let block = Self::picker_panel(app, " Spotify Link ", None);
                let inner = block.inner(picker_area);
                f.render_widget(block, picker_area);

                if app.spotify.oauth_pending || app.spotify.oauth_error.is_some() {
                    let p = Paragraph::new(spotify_waiting_lines(app));
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
                    let cid_active = app.spotify.link_field == 0;
                    let cid_label = if cid_active {
                        app.theme.fg_bright
                    } else {
                        app.theme.fg_dim
                    };
                    let cid_text = if app.spotify.link_input.is_empty() {
                        "[ client id ]".to_string()
                    } else {
                        "•".repeat(app.spotify.link_input.chars().count())
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
                    let port_active = app.spotify.link_field == 1;
                    let port_label = if port_active {
                        app.theme.fg_bright
                    } else {
                        app.theme.fg_dim
                    };
                    let mut port_spans = vec![
                        Span::styled(" Port:      ", Style::default().fg(port_label)),
                        Span::styled(
                            app.spotify.oauth_port.clone(),
                            Style::default().fg(app.theme.accent),
                        ),
                    ];
                    if port_active && let Some(cur) = input_cursor {
                        port_spans.push(Span::styled(" ", cur));
                    }
                    lines.push(Line::from(port_spans));
                    lines.push(Line::from(""));
                    lines.push(Line::from(Span::styled(
                        format!(
                            "Redirect: http://127.0.0.1:{}/login",
                            app.spotify.oauth_port.parse::<u16>().unwrap_or(8990)
                        ),
                        Style::default().fg(app.theme.fg_bright),
                    )));
                    lines.push(Line::from(Span::styled(
                        "Register it in your Spotify app dashboard.",
                        Style::default().fg(app.theme.fg_dim),
                    )));

                    let p = Paragraph::new(lines);
                    f.render_widget(p, inner);
                }
            }
            PickerId::SpotifySearch => {
                let help = if app.spotify.status.as_ref().is_none_or(|s| !s.linked) {
                    None
                } else {
                    Some(" Enter: play   Ctrl+D: download   Esc: close")
                };
                let src = app.pickers.top().map_or(PickerSource::All, |o| o.source);
                let title = format!(" \u{f04c7} Search: {} ", src.label());
                let block = Self::picker_panel(app, &title, help);
                let inner = block.inner(picker_area);
                f.render_widget(block, picker_area);

                let query = app.pickers.top().map_or(String::new(), |o| o.query.clone());
                let cursor_style = cursor_span_style(app);

                if app.spotify.status.as_ref().is_none_or(|s| !s.linked) {
                    if app.spotify.oauth_pending || app.spotify.oauth_error.is_some() {
                        let p = Paragraph::new(spotify_waiting_lines(app));
                        f.render_widget(p, inner);
                    } else {
                        let lines = vec![
                            Line::from(Span::styled(
                                "Spotify is not linked yet.",
                                Style::default().fg(app.theme.fg),
                            )),
                            Line::from(""),
                            Line::from(Span::styled(
                                "Press Enter to open your browser and authorize gtm.",
                                Style::default().fg(app.theme.fg_dim),
                            )),
                            Line::from(Span::styled(
                                "Esc closes this picker.",
                                Style::default().fg(app.theme.fg_dim),
                            )),
                        ];
                        let p = Paragraph::new(lines);
                        f.render_widget(p, inner);
                    }
                } else {
                    let search_line = Line::from(vec![
                        Span::styled(" > ", Style::default().fg(app.theme.fg_dim)),
                        Span::styled(query.as_str(), Style::default().fg(app.theme.fg)),
                        Span::styled(" ", cursor_style.unwrap_or_default()),
                    ]);

                    let picks = app.spot_picks();
                    let total = picks.len();
                    let sel = app
                        .pickers
                        .top()
                        .map_or(0, |o| o.selected.min(total.saturating_sub(1)));
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
                            "  Type to search tracks, albums, playlists & artists...",
                            Style::default().fg(app.theme.fg_dim),
                        )));
                    } else if total == 0 {
                        // Only claim "no results" once the search has actually
                        // answered; a pending one shows the spinner instead.
                        lines.push(Line::from(Span::styled(
                            if app.spotify.search_loading {
                                "  Searching…"
                            } else {
                                "  No results found"
                            },
                            Style::default().fg(app.theme.fg_dim),
                        )));
                    } else {
                        for row in scroll_start..scroll_end {
                            let i = picks[row];
                            let (_, _pl_name, track) = &app.spotify.search_results[i];
                            let prefix = if row == sel { " > " } else { "   " };
                            let tag = match track.kind {
                                Some(SpotifySearchKind::Album) => Some("[Album]"),
                                Some(SpotifySearchKind::Artist) => Some("[Artist]"),
                                Some(SpotifySearchKind::Playlist) => Some("[Playlist]"),
                                _ => None,
                            };
                            let body = match track.kind {
                                Some(SpotifySearchKind::Artist) => track.name.clone(),
                                _ => format!("{} - {}", track.artists, track.name),
                            };
                            let dur = track
                                .duration_ms
                                .map(|ms| format_duration_short(ms / 1000))
                                .unwrap_or_default();
                            let content = format!(
                                "{prefix}{body} [{}]",
                                tag.map(|t| t.to_string()).unwrap_or(dur)
                            );
                            let style = if row == sel {
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
                            // Register rows so clicking selects them, matching
                            // the library search picker.
                            app.mouse_map.register(
                                Rect {
                                    x: results_area.x,
                                    y: results_area.y + 1 + (row - scroll_start) as u16,
                                    width: results_area.width,
                                    height: 1,
                                },
                                MouseZone::PickerItem(row),
                            );
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
                        let (_, _, track) = &app.spotify.search_results[picks[sel.min(total - 1)]];
                        let cover_w = 20u16.min(body.width.saturating_sub(24).max(8));
                        let (cover_area, meta_area) =
                            if (app.spotify.preview_cover_stateful.is_some()
                                || app.spotify.preview_cover.is_some())
                                && body.width >= cover_w + 8
                            {
                                let hchunks = Layout::default()
                                    .direction(Direction::Horizontal)
                                    .constraints([Constraint::Length(cover_w), Constraint::Min(0)])
                                    .split(body);
                                (
                                    Rect {
                                        x: hchunks[0].x + 1,
                                        y: hchunks[0].y,
                                        width: hchunks[0].width.saturating_sub(1),
                                        height: hchunks[0].height,
                                    },
                                    hchunks[1],
                                )
                            } else {
                                (body, body)
                            };
                        Render::cover(
                            f,
                            cover_area,
                            app.spotify.preview_cover_stateful.as_mut(),
                            app.spotify.preview_cover.as_deref(),
                            app.theme.fg_dim,
                            Some("\u{1f3b5}"),
                        );
                        if meta_area != body {
                            f.render_widget(
                                Paragraph::new(Line::from(Span::styled(
                                    "\u{2502}".repeat(meta_area.width as usize),
                                    Style::default().fg(app.theme.muted_border),
                                ))),
                                Rect {
                                    x: meta_area.x.saturating_sub(1),
                                    y: meta_area.y,
                                    width: 1,
                                    height: meta_area.height,
                                },
                            );
                        }
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
                        match track.kind {
                            Some(SpotifySearchKind::Album) => {
                                push("Album", &track.name);
                                if !track.artists.is_empty() {
                                    push("Artist", &track.artists);
                                }
                            }
                            Some(SpotifySearchKind::Artist) => {
                                push("Artist", &track.name);
                            }
                            Some(SpotifySearchKind::Playlist) => {
                                push("Playlist", &track.name);
                                if !track.artists.is_empty() {
                                    push("Owner", &track.artists);
                                }
                                if let Some(ref album) = track.album {
                                    push("Tracks", album);
                                }
                            }
                            _ => {
                                push("Track", &track.name);
                                push("Artist", &track.artists);
                                if let Some(ref album) = track.album {
                                    push("Album", album);
                                }
                                if let Some(ms) = track.duration_ms {
                                    push("Length", &format_duration_short(ms / 1000));
                                }
                            }
                        }
                        f.render_widget(Paragraph::new(meta_lines), meta_area);
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
        // The "Esc" affordance lives at the top-right corner, inline with the
        // picker title. Any trailing "Esc: …" token is lifted off the hint and
        // the chip itself is always the bare "Esc" label, padded with two
        // spaces either side so it reads as a button.
        let bottom_hint = match help {
            Some(h) => {
                let trimmed = h.trim_end();
                match trimmed.rfind("Esc:") {
                    Some(pos) if !trimmed[pos + 4..].contains(':') => {
                        Some(trimmed[..pos].trim_end())
                    }
                    _ => Some(trimmed),
                }
            }
            None => None,
        };
        let bottom_hint = bottom_hint.filter(|h| !h.is_empty());
        // Always a bare "Esc" — never "Esc: close" — with 2-space padding.
        let esc_label = "  Esc  ".to_string();

        let mut block = Block::default()
            .title(Line::from(Span::styled(
                title.into(),
                Style::default()
                    .fg(app.theme.accent)
                    .add_modifier(Modifier::BOLD),
            )))
            .title(
                Line::from(Span::styled(
                    esc_label,
                    Style::default().fg(app.theme.fg_dim),
                ))
                .right_aligned(),
            )
            .padding(Padding {
                left: 1,
                right: 1,
                top: 1,
                bottom: 1,
            })
            .style(Style::default().bg(if app.transparent_pickers {
                ratatui::style::Color::Reset
            } else {
                app.float_bg()
            }));
        if let Some(h) = bottom_hint {
            block = block.title_bottom(Line::from(Span::styled(
                h,
                Style::default().fg(app.theme.fg_dim),
            )));
        }
        block
    }

    fn render_queue(f: &mut ratatui::Frame, area: Rect, app: &mut App) {
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

    fn render_upnext_preview(f: &mut ratatui::Frame, area: Rect, app: &mut App, next_idx: usize) {
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

    fn render_yt_search(f: &mut ratatui::Frame, area: Rect, app: &mut App) {
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

    fn render_search_library(f: &mut ratatui::Frame, area: Rect, app: &mut App) {
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

    /// Prompt line shown at the top of query pickers (radio search).
    fn picker_query_line(app: &App) -> Line<'static> {
        let q = app.pickers.top().map_or(String::new(), |o| o.query.clone());
        Line::from(vec![
            Span::styled(" > ", Style::default().fg(app.theme.fg)),
            Span::styled(q, Style::default().fg(app.theme.fg)),
            match cursor_span_style(app) {
                Some(style) => Span::styled(" ", style),
                None => Span::raw(""),
            },
        ])
    }

    /// Shared row list for the remote-service pickers: a stackable panel with
    /// optional leading lines, a scrollable row region and row mouse zones.
    #[allow(clippy::too_many_arguments)]
    fn render_scroll_rows(
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

    /// `gtm setup` service chooser. Enter opens the matching setup flow.
    fn render_setup(f: &mut ratatui::Frame, area: Rect, app: &mut App) {
        let block = Self::picker_panel(app, "Setup", None);
        let inner = block.inner(area);
        f.render_widget(block, area);

        // (name, description, live status)
        let services: [(&str, &str, String); 3] = [
            ("Spotify", "OAuth link", {
                if app.spotify.status.as_ref().is_some_and(|s| s.linked) {
                    "✓ linked".to_string()
                } else {
                    "not linked".to_string()
                }
            }),
            ("Last.fm", "API key + OAuth", {
                if app.setup.lastfm_status.as_ref().is_some_and(|s| s.ready) {
                    "✓ ready".to_string()
                } else {
                    "not linked".to_string()
                }
            }),
            ("YouTube", "cookie file", {
                if app.cookie_file.is_some() {
                    "✓ cookies set".to_string()
                } else {
                    "no cookies".to_string()
                }
            }),
        ];
        let (sel, _) = setup_selection(app);
        let mut lines = Vec::new();
        lines.push(Line::from(Span::styled(
            "Which service do you want to set up?",
            Style::default().fg(app.theme.fg_dim),
        )));
        lines.push(Line::from(""));
        for (i, (name, desc, status)) in services.iter().enumerate() {
            let style = if i == sel {
                Style::default()
                    .fg(app.theme.accent)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(app.theme.fg)
            };
            lines.push(Line::from(vec![
                Span::styled(if i == sel { "▸ " } else { "  " }, style),
                Span::styled(
                    format!("{} {name:<22}", service_icon_glyph(&app.icon_style, name)),
                    style,
                ),
                Span::styled(*desc, Style::default().fg(app.theme.fg_dim)),
                Span::styled("  · ", Style::default().fg(app.theme.fg_dim)),
                Span::styled(status.as_str(), style),
            ]));
        }
        f.render_widget(Paragraph::new(lines), inner);
    }

    /// Last.fm setup: API key/secret form, then the OAuth browser flow with a
    /// loopback callback (or a manual token paste).
    fn render_lastfm_setup(f: &mut ratatui::Frame, area: Rect, app: &mut App) {
        let status_line = match app.setup.lastfm_status.as_ref() {
            Some(st) if st.ready => {
                if st.enabled {
                    "✓ authorized — scrobbling enabled".to_string()
                } else {
                    "✓ authorized — scrobbling disabled".to_string()
                }
            }
            Some(st) if st.api_key.is_some() => "API key set, not yet authorized".to_string(),
            Some(_) | None if app.setup.lastfm_error.is_some() => {
                format!("⚠ {}", app.setup.lastfm_error.as_deref().unwrap_or(""))
            }
            _ => "Enter API key and secret, then authorize in the browser".to_string(),
        };
        let block = Self::picker_panel(
            app,
            "Last.fm Setup",
            Some(" Enter: authorize   Tab: field   Esc: close"),
        );
        let inner = block.inner(area);
        f.render_widget(block, area);

        let mut lines = Vec::new();
        let focus = app.setup.lastfm_focus;
        let fields: [(&str, String); 2] = [
            (
                " API key    ",
                if app.setup.lastfm_api_key.is_empty() {
                    "[ api key ]".into()
                } else {
                    "•".repeat(app.setup.lastfm_api_key.chars().count())
                },
            ),
            (
                " API secret ",
                if app.setup.lastfm_api_secret.is_empty() {
                    "[ api secret ]".into()
                } else {
                    "•".repeat(app.setup.lastfm_api_secret.chars().count())
                },
            ),
        ];
        for (idx, (label, value)) in fields.iter().enumerate() {
            let label_style = if idx == focus {
                Style::default()
                    .fg(app.theme.accent)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(app.theme.fg_dim)
            };
            let value_style = if idx == focus {
                Style::default()
                    .fg(app.theme.fg_bright)
                    .add_modifier(Modifier::UNDERLINED)
            } else {
                Style::default().fg(app.theme.fg)
            };
            lines.push(Line::from(vec![
                Span::styled(label.to_string(), label_style),
                Span::styled(format!("[{value}]"), value_style),
            ]));
        }
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            status_line,
            Style::default().fg(app.theme.fg_dim),
        )));
        if let Some(url) = app.setup.lastfm_auth_url.as_deref() {
            lines.push(Line::from(Span::styled(
                "Open in browser:",
                Style::default().fg(app.theme.fg_dim),
            )));
            lines.push(Line::from(Span::styled(
                url,
                Style::default().fg(app.theme.accent),
            )));
            if app.setup.lastfm_pending {
                lines.push(Line::from(Span::styled(
                    "Waiting for the callback… (or press p to paste a token)",
                    Style::default().fg(app.theme.fg_dim),
                )));
            }
        }
        f.render_widget(Paragraph::new(lines), inner);
    }

    fn render_podcast_feeds(f: &mut ratatui::Frame, area: Rect, app: &mut App) {
        let mut rows = Vec::new();
        for feed in &app.podcast.feeds {
            rows.push(format!(
                "\u{1f4e1} {} \u{2003}[{} episodes]",
                feed.title, feed.episodes
            ));
        }
        let mut prepend = Vec::new();
        if let Some(st) = app.podcast.status.as_ref() {
            prepend.push(Line::from(Span::styled(
                format!(" {} feeds, {} episodes", st.feeds, st.episodes),
                Style::default().fg(app.theme.fg_dim),
            )));
        }
        Self::render_scroll_rows(
            f,
            area,
            app,
            " Podcasts ",
            "",
            prepend,
            rows,
            if app.podcast.feeds_pending {
                " loading feeds\u{2026}"
            } else {
                "no subscriptions \u{2014} press a to add a feed URL"
            },
        );
    }

    fn render_podcast_episodes(f: &mut ratatui::Frame, area: Rect, app: &mut App) {
        let mut rows = Vec::new();
        for ep in &app.podcast.episodes {
            let dur = ep
                .duration_secs
                .map(format_duration_short)
                .unwrap_or_else(|| "--:--".to_string());
            rows.push(format!("\u{266b} [{dur}] {}", ep.title));
        }
        let title = app
            .podcast
            .episodes
            .first()
            .map(|e| format!(" {} ", e.feed_title))
            .unwrap_or_else(|| " Episodes ".into());
        Self::render_scroll_rows(
            f,
            area,
            app,
            &title,
            "",
            Vec::new(),
            rows,
            "no episodes \u{2014} press r in the feed list to refresh",
        );
    }

    fn render_podcast_subscribe(f: &mut ratatui::Frame, area: Rect, app: &mut App) {
        let block = Self::picker_panel(app, " Subscribe ", None);
        let inner = block.inner(area);
        f.render_widget(block, area);
        let mut lines = vec![Line::from(Span::styled(
            " Feed URL ",
            Style::default().fg(app.theme.fg_dim),
        ))];
        lines.push(Line::from(vec![
            Span::styled(" ", Style::default().fg(app.theme.fg)),
            Span::styled(
                app.podcast.subscribe_url.clone(),
                Style::default()
                    .fg(app.theme.fg_bright)
                    .add_modifier(Modifier::UNDERLINED),
            ),
            match cursor_span_style(app) {
                Some(style) => Span::styled(" ", style),
                None => Span::raw(""),
            },
        ]));
        lines.push(Line::from(Span::styled(
            " expects an RSS or Atom feed URL (e.g. https://feeds.example.com/show.xml)",
            Style::default().fg(app.theme.fg_dim),
        )));
        f.render_widget(Paragraph::new(lines), inner);
    }

    fn render_load_stream(f: &mut ratatui::Frame, area: Rect, app: &mut App) {
        let block = Self::picker_panel(app, " Stream ", None);
        let inner = block.inner(area);
        f.render_widget(block, area);
        let url = app
            .pickers
            .top()
            .map(|p| p.query.clone())
            .unwrap_or_default();
        let mut lines = vec![Line::from(Span::styled(
            " Stream URL ",
            Style::default().fg(app.theme.fg_dim),
        ))];
        lines.push(Line::from(vec![
            Span::styled(" ", Style::default().fg(app.theme.fg)),
            Span::styled(
                url,
                Style::default()
                    .fg(app.theme.fg_bright)
                    .add_modifier(Modifier::UNDERLINED),
            ),
            match cursor_span_style(app) {
                Some(style) => Span::styled(" ", style),
                None => Span::raw(""),
            },
        ]));
        lines.push(Line::from(Span::styled(
            " accepts an http(s):// stream URL; M3U/PLS playlists are resolved server-side",
            Style::default().fg(app.theme.fg_dim),
        )));
        f.render_widget(Paragraph::new(lines), inner);
    }

    /// YouTube cookie-file path form (single text input). The daemon's yt-dlp
    /// uses the file to lift age/consent restrictions; Enter with an empty box
    /// clears the configured path.
    fn render_youtube_setup(f: &mut ratatui::Frame, area: Rect, app: &mut App) {
        let block = Self::picker_panel(app, " YouTube Cookies ", None);
        let inner = block.inner(area);
        f.render_widget(block, area);
        let path = app.setup.youtube_cookie_input.clone();
        let mut lines = vec![Line::from(vec![
            Span::styled("  ", Style::default().fg(app.theme.fg)),
            Span::styled(
                "Cookie file path (Netscape format, e.g. ~/.cookies/youtube.txt):",
                Style::default().fg(app.theme.fg_dim),
            ),
        ])];
        lines.push(Line::from(vec![
            Span::styled(" ", Style::default().fg(app.theme.fg)),
            Span::styled(
                path,
                Style::default()
                    .fg(app.theme.fg_bright)
                    .add_modifier(Modifier::UNDERLINED),
            ),
            match cursor_span_style(app) {
                Some(style) => Span::styled(" ", style),
                None => Span::raw(""),
            },
        ]));
        lines.push(Line::from(Span::styled(
            " lets yt-dlp / the daemon access age-restricted and member-only media; empty Enter clears it",
            Style::default().fg(app.theme.fg_dim),
        )));
        f.render_widget(Paragraph::new(lines), inner);
    }

    fn render_radio(f: &mut ratatui::Frame, area: Rect, app: &mut App) {
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

    fn radio_row(s: &RadioStation) -> String {
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

    fn render_search_preview(
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

    fn render_settings(f: &mut ratatui::Frame, area: Rect, app: &App) {
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
                let reverb_on = app.state.audio.reverb.enabled;
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
                        if app.state.audio.eq_enabled {
                            "On"
                        } else {
                            "Off"
                        }
                    ),
                    format!("Reverb         {}", if reverb_on { "On" } else { "Off" }),
                    format!(
                        "Cover Source   {}  ▶",
                        cover_provider_label(&app.cover_provider)
                    ),
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
                    format!(
                        "Reactive Intensity {:.0}%  ▶",
                        app.reactive_theme_intensity * 100.0
                    ),
                    format!(
                        "Hide Footer    {}",
                        if app.hide_footer { "On" } else { "Off" }
                    ),
                    "Clear Lyrics Cache  Enter".to_string(),
                    "Clear Cover Cache    Enter  ▶".to_string(),
                    format!("Cover Cache     {} MB  ▶", app.cover_cache_mb),
                    "Notification Settings  Enter  ▶".to_string(),
                    format!("Theme Mode     {}  ▶", theme_mode_label(&app.theme_mode)),
                ]
            }
            3 => {
                let st = app.spotify.status.clone().unwrap_or_default();
                let connected = if st.linked {
                    "Connected"
                } else {
                    "Disconnected"
                };
                let user = st.user.as_deref().unwrap_or("(none)");
                let status_label = if !st.linked {
                    connected.to_string()
                } else if st.needs_relink {
                    "Relink required".to_string()
                } else if let Some(err) = st.error.as_deref() {
                    let mut e = err.chars().take(28).collect::<String>();
                    if err.chars().count() > 28 {
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
                    "Next            Enter".to_string(),
                    "Previous        Enter".to_string(),
                    format!("Shuffle     {}  ▶", if st.shuffle { "On" } else { "Off" }),
                    format!("Repeat     {}  ▶", st.repeat),
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
            (0, 1) => lines.push(Line::from(Span::styled(
                " Press Enter to toggle cookie path.",
                Style::default().fg(app.theme.fg_dim),
            ))),
            (1, 0) => lines.push(Line::from(Span::styled(
                format!(" Press Enter to cycle (current: {:?}).", app.state.repeat),
                Style::default().fg(app.theme.fg_dim),
            ))),
            (1, 1) => lines.push(Line::from(Span::styled(
                " Press Enter to toggle shuffle.",
                Style::default().fg(app.theme.fg_dim),
            ))),
            (1, 2) => lines.push(Line::from(Span::styled(
                " Press Enter to open crossfade picker.",
                Style::default().fg(app.theme.fg_dim),
            ))),
            (1, 3) => {
                let eq_on = app.state.audio.eq_enabled;
                lines.push(Line::from(Span::styled(
                    if eq_on {
                        " Press Enter to disable EQ."
                    } else {
                        " Press Enter to enable EQ."
                    },
                    Style::default().fg(app.theme.fg_dim),
                )));
            }
            (1, 4) => {
                let rev_on = app.state.audio.reverb.enabled;
                lines.push(Line::from(Span::styled(
                    if rev_on {
                        " Press Enter to disable reverb."
                    } else {
                        " Press Enter to enable reverb."
                    },
                    Style::default().fg(app.theme.fg_dim),
                )));
            }
            (1, 5) => lines.push(Line::from(Span::styled(
                " Press Enter to cycle the cover art source.",
                Style::default().fg(app.theme.fg_dim),
            ))),
            (2, 0) => lines.push(Line::from(Span::styled(
                " Press Enter to open Theme Picker.",
                Style::default().fg(app.theme.fg_dim),
            ))),
            (2, 1) => lines.push(Line::from(Span::styled(
                " Press Enter to toggle transparent background.",
                Style::default().fg(app.theme.fg_dim),
            ))),
            (2, 2) => lines.push(Line::from(Span::styled(
                " Press Enter to toggle transparent pickers.",
                Style::default().fg(app.theme.fg_dim),
            ))),
            (2, 3) => lines.push(Line::from(Span::styled(
                " Download missing cover art from Deezer.",
                Style::default().fg(app.theme.fg_dim),
            ))),
            (2, 4) => lines.push(Line::from(Span::styled(
                " Fetch and save lyrics for all tracks.",
                Style::default().fg(app.theme.fg_dim),
            ))),
            (2, 5) => lines.push(Line::from(Span::styled(
                " Resolve and embed clean tags into files.",
                Style::default().fg(app.theme.fg_dim),
            ))),
            (2, 6) => lines.push(Line::from(Span::styled(
                " Press Enter to open Footer Preset picker.",
                Style::default().fg(app.theme.fg_dim),
            ))),
            (2, 7) => lines.push(Line::from(Span::styled(
                " Press Enter to open visualizer picker.",
                Style::default().fg(app.theme.fg_dim),
            ))),
            (2, 8) => lines.push(Line::from(Span::styled(
                " Press Enter to toggle reactive theme.",
                Style::default().fg(app.theme.fg_dim),
            ))),
            (2, 9) => lines.push(Line::from(Span::styled(
                " Clear cached lyrics for all tracks.",
                Style::default().fg(app.theme.fg_dim),
            ))),
            (2, 10) => lines.push(Line::from(Span::styled(
                " Clear downloaded cover art cache.",
                Style::default().fg(app.theme.fg_dim),
            ))),
            (2, 12) => lines.push(Line::from(Span::styled(
                " Press Enter to cycle theme mode (auto/dark/light).",
                Style::default().fg(app.theme.fg_dim),
            ))),
            (3, 0) => lines.push(Line::from(Span::styled(
                " Spotify integration status.",
                Style::default().fg(app.theme.fg_dim),
            ))),
            (3, 3) => lines.push(Line::from(Span::styled(
                " Press Enter to paste a Spotify access token.",
                Style::default().fg(app.theme.fg_dim),
            ))),
            (3, 4) => lines.push(Line::from(Span::styled(
                " Re-fetch playlists from Spotify.",
                Style::default().fg(app.theme.fg_dim),
            ))),
            (3, 5) => lines.push(Line::from(Span::styled(
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

const INFO_CARD_H: u16 = 16;

const INFO_TEXT_H: u16 = 6;

fn info_block_h() -> u16 {
    if no_image_protocol() {
        INFO_TEXT_H
    } else {
        INFO_CARD_H
    }
}

fn library_stats_line(app: &App) -> String {
    if app.browse_detail.is_some() {
        if app.library_category == 5 {
            let n = app.spotify.playlist_tracks_cache.len();
            return format!(
                " {} {} (+ play all / shuffle) ",
                n,
                plural(n, "track", "tracks")
            );
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
            let n = app.spotify.playlists.len();
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
        match provider_icon(source) {
            Some(g) => format!(" {g} {source}"),
            None => " ♪ Local".to_string(),
        }
    } else {
        match source {
            "Spotify" => " ♫ Spotify",
            "YouTube" => " ▶ YouTube",
            "Local" => " ♪ Local",
            other => other,
        }
        .into()
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
                .popup_track_id
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
            } else if let Some((src, _)) = classify_remote_source(&track.path) {
                src
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
            let playlists = &app.spotify.playlists;
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
            let st = app.selected_spotify_track()?;
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

#[derive(Debug, Clone)]
pub struct Command {
    pub icon: &'static str,
    pub keys: &'static str,
    pub hint: &'static str,
}

pub struct CommandPalette;

impl CommandPalette {
    pub fn commands(icon_style: &str) -> &'static [Command] {
        if icon_style == "mdi" {
            Self::commands_mdi()
        } else {
            Self::commands_emoji()
        }
    }

    fn commands_mdi() -> &'static [Command] {
        &[
            Command {
                icon: "\u{f04ba} Play/Pause",
                keys: "Space",
                hint: "play/pause",
            },
            Command {
                icon: "\u{f04ad} Next Track",
                keys: "n",
                hint: "next track",
            },
            Command {
                icon: "\u{f04a8} Prev Track",
                keys: "p",
                hint: "prev track",
            },
            Command {
                icon: "\u{f04cd} Stop",
                keys: "s",
                hint: "stop",
            },
            Command {
                icon: "\u{f04e2} Seek Forward",
                keys: ".",
                hint: "seek forward",
            },
            Command {
                icon: "\u{f04e0} Seek Backward",
                keys: ",",
                hint: "seek backward",
            },
            Command {
                icon: "\u{f057e} Volume Up",
                keys: "+",
                hint: "volume up",
            },
            Command {
                icon: "\u{f057d} Volume Down",
                keys: "-",
                hint: "volume down",
            },
            Command {
                icon: "\u{f0580} Mute: Toggle",
                keys: "m",
                hint: "mute",
            },
            Command {
                icon: "\u{f04ab} Mono: Toggle",
                keys: "Alt+1",
                hint: "toggle mono",
            },
            Command {
                icon: "\u{f0577} Repeat Mode",
                keys: "r",
                hint: "repeat",
            },
            Command {
                icon: "\u{f0578} Shuffle Library",
                keys: "S",
                hint: "shuffle",
            },
            Command {
                icon: "\u{f0493} Toggle Favourite",
                keys: "f",
                hint: "toggle favourite",
            },
            Command {
                icon: "\u{f04a6} Love / Un-love on Last.fm",
                keys: "*",
                hint: "love last.fm",
            },
            Command {
                icon: "\u{f04ec} Toggle Last.fm Scrobbling",
                keys: "&",
                hint: "toggle scrobbling",
            },
            Command {
                icon: "\u{f057a} Search",
                keys: "/",
                hint: "search this list",
            },
            Command {
                icon: "\u{f057a} Search Library",
                keys: "Alt+/",
                hint: "search lib",
            },
            Command {
                icon: "\u{f056e} Queue",
                keys: "Alt+Q",
                hint: "queue",
            },
            Command {
                icon: "\u{f16a} YouTube Search",
                keys: "Alt+Y",
                hint: "youtube",
            },
            Command {
                icon: "\u{f04c7} Spotify",
                keys: "Alt+S",
                hint: "spotify",
            },
            Command {
                icon: "\u{f1dd} Fetch Lyrics",
                keys: "l",
                hint: "fetch lyrics",
            },
            Command {
                icon: "\u{f156} Clear Queue",
                keys: "D",
                hint: "clear queue",
            },
            Command {
                icon: "\u{f285} Multiselect",
                keys: "v",
                hint: "multiselect",
            },
            Command {
                icon: "\u{f285} Multiselect Up",
                keys: "Shift+Up",
                hint: "multiselect up",
            },
            Command {
                icon: "\u{f285} Multiselect Down",
                keys: "Shift+Down",
                hint: "multiselect down",
            },
            Command {
                icon: "\u{f055e} Add to Queue",
                keys: "a",
                hint: "add to queue",
            },
            Command {
                icon: "\u{f055e} Add to Playlist",
                keys: "A",
                hint: "add to playlist",
            },
            Command {
                icon: "\u{f156} Delete from List",
                keys: "x",
                hint: "delete from list",
            },
            Command {
                icon: "\u{f045d} Jump to End",
                keys: "G",
                hint: "jump to end",
            },
            Command {
                icon: "\u{f0493} Edit Metadata",
                keys: "e",
                hint: "edit metadata",
            },
            Command {
                icon: "\u{f0493} Tab Cycle",
                keys: "Tab",
                hint: "tab cycle",
            },
            Command {
                icon: "\u{f0493} Prev Tab",
                keys: "Shift+Tab",
                hint: "prev tab",
            },
            Command {
                icon: "\u{f0493} Settings",
                keys: "Alt+,",
                hint: "settings",
            },
            Command {
                icon: "\u{f0570} Equalizer",
                keys: "Alt+E",
                hint: "eq",
            },
            Command {
                icon: "\u{f04b2} Sleep Timer",
                keys: "Alt+Z",
                hint: "sleeptimer",
            },
            Command {
                icon: "\u{f0493} Theme",
                keys: "Alt+C",
                hint: "themepicker",
            },
            Command {
                icon: "\u{f051d} Notifications",
                keys: "Alt+N",
                hint: "notifications",
            },
            Command {
                icon: "\u{f0493} Progress Style",
                keys: "Alt+P",
                hint: "progress style",
            },
            Command {
                icon: "\u{f0570} Visualizer: Toggle",
                keys: "Ctrl+V",
                hint: "visualizer",
            },
            Command {
                icon: "\u{f0570} Visualizer Preset",
                keys: "Alt+V",
                hint: "visualizer preset",
            },
            Command {
                icon: "\u{f04db} Quit",
                keys: "q",
                hint: "quit",
            },
            Command {
                icon: "\u{f04db} Quit Daemon",
                keys: "Q/Ctrl+Q",
                hint: "quit daemon",
            },
            Command {
                icon: "\u{f051d} Toggle Help",
                keys: "?",
                hint: "toggle help",
            },
            Command {
                icon: "\u{f051d} Hide Help Bar",
                keys: "Ctrl+H",
                hint: "hide help bar",
            },
            Command {
                icon: "\u{f0493} Health Check",
                keys: "Alt+H",
                hint: "health check",
            },
            Command {
                icon: "\u{f0493} Setup Services",
                keys: "Alt+X",
                hint: "setup",
            },
            Command {
                icon: "\u{f043b} Radio Browser",
                keys: "Alt+R",
                hint: "radio browse",
            },
            Command {
                icon: "\u{f056d} Play Stream URL",
                keys: "Alt+O",
                hint: "play stream url",
            },
        ]
    }

    fn commands_emoji() -> &'static [Command] {
        &[
            Command {
                icon: "\u{25b6}\u{fe0f} Play/Pause",
                keys: "Space",
                hint: "play/pause",
            },
            Command {
                icon: "\u{23ed}\u{fe0f} Next Track",
                keys: "n",
                hint: "next track",
            },
            Command {
                icon: "\u{23ee}\u{fe0f} Prev Track",
                keys: "p",
                hint: "prev track",
            },
            Command {
                icon: "\u{23f9}\u{fe0f} Stop",
                keys: "s",
                hint: "stop",
            },
            Command {
                icon: "\u{23e9}\u{fe0f} Seek Forward",
                keys: ".",
                hint: "seek forward",
            },
            Command {
                icon: "\u{23ea}\u{fe0f} Seek Backward",
                keys: ",",
                hint: "seek backward",
            },
            Command {
                icon: "\u{1f50a} Volume Up",
                keys: "+",
                hint: "volume up",
            },
            Command {
                icon: "\u{1f509} Volume Down",
                keys: "-",
                hint: "volume down",
            },
            Command {
                icon: "\u{1f507} Mute: Toggle",
                keys: "m",
                hint: "mute",
            },
            Command {
                icon: "\u{1f508} Mono: Toggle",
                keys: "Alt+1",
                hint: "toggle mono",
            },
            Command {
                icon: "\u{1f501} Repeat Mode",
                keys: "r",
                hint: "repeat",
            },
            Command {
                icon: "\u{1f500} Shuffle Library",
                keys: "S",
                hint: "shuffle",
            },
            Command {
                icon: "\u{2764}\u{fe0f} Toggle Favourite",
                keys: "f",
                hint: "toggle favourite",
            },
            Command {
                icon: "\u{1f49e} Love / Un-love on Last.fm",
                keys: "*",
                hint: "love last.fm",
            },
            Command {
                icon: "\u{1f504} Toggle Last.fm Scrobbling",
                keys: "&",
                hint: "toggle scrobbling",
            },
            Command {
                icon: "\u{1f50d} Search",
                keys: "/",
                hint: "search this list",
            },
            Command {
                icon: "\u{1f50e} Search Library",
                keys: "Alt+/",
                hint: "search lib",
            },
            Command {
                icon: "\u{1f4cb} Queue",
                keys: "Alt+Q",
                hint: "queue",
            },
            Command {
                icon: "\u{25b6}\u{fe0f} YouTube Search",
                keys: "Alt+Y",
                hint: "youtube",
            },
            Command {
                icon: "\u{1f3b5} Spotify",
                keys: "Alt+S",
                hint: "spotify",
            },
            Command {
                icon: "\u{1f4dd} Fetch Lyrics",
                keys: "l",
                hint: "fetch lyrics",
            },
            Command {
                icon: "\u{1f5d1} Clear Queue",
                keys: "D",
                hint: "clear queue",
            },
            Command {
                icon: "\u{2611}\u{fe0f} Multiselect",
                keys: "v",
                hint: "multiselect",
            },
            Command {
                icon: "\u{2611}\u{fe0f} Multiselect Up",
                keys: "Shift+Up",
                hint: "multiselect up",
            },
            Command {
                icon: "\u{2611}\u{fe0f} Multiselect Down",
                keys: "Shift+Down",
                hint: "multiselect down",
            },
            Command {
                icon: "\u{2795} Add to Queue",
                keys: "a",
                hint: "add to queue",
            },
            Command {
                icon: "\u{1f4dc} Add to Playlist",
                keys: "A",
                hint: "add to playlist",
            },
            Command {
                icon: "\u{274c} Delete from List",
                keys: "x",
                hint: "delete from list",
            },
            Command {
                icon: "\u{2b07}\u{fe0f} Jump to End",
                keys: "G",
                hint: "jump to end",
            },
            Command {
                icon: "\u{270f}\u{fe0f} Edit Metadata",
                keys: "e",
                hint: "edit metadata",
            },
            Command {
                icon: "\u{27a1}\u{fe0f} Tab Cycle",
                keys: "Tab",
                hint: "tab cycle",
            },
            Command {
                icon: "\u{2b05}\u{fe0f} Prev Tab",
                keys: "Shift+Tab",
                hint: "prev tab",
            },
            Command {
                icon: "\u{2699}\u{fe0f} Settings",
                keys: "Alt+,",
                hint: "settings",
            },
            Command {
                icon: "\u{1f39a} Equalizer",
                keys: "Alt+E",
                hint: "eq",
            },
            Command {
                icon: "\u{23f0}\u{fe0f} Sleep Timer",
                keys: "Alt+Z",
                hint: "sleeptimer",
            },
            Command {
                icon: "\u{1f3a8} Theme",
                keys: "Alt+C",
                hint: "themepicker",
            },
            Command {
                icon: "\u{2139}\u{fe0f} About",
                keys: "Alt+A",
                hint: "about",
            },
            Command {
                icon: "\u{1f514} Notifications",
                keys: "Alt+N",
                hint: "notifications",
            },
            Command {
                icon: "\u{1f3a8} Progress Style",
                keys: "Alt+P",
                hint: "progress style",
            },
            Command {
                icon: "\u{1f3b6} Visualizer: Toggle",
                keys: "Ctrl+V",
                hint: "visualizer",
            },
            Command {
                icon: "\u{1f3b6} Visualizer Preset",
                keys: "Alt+V",
                hint: "visualizer preset",
            },
            Command {
                icon: "\u{23f9}\u{fe0f} Quit",
                keys: "q",
                hint: "quit",
            },
            Command {
                icon: "\u{23f9}\u{fe0f} Quit Daemon",
                keys: "Q/Ctrl+Q",
                hint: "quit daemon",
            },
            Command {
                icon: "\u{2753} Toggle Help",
                keys: "?",
                hint: "toggle help",
            },
            Command {
                icon: "\u{1f6ab} Hide Help Bar",
                keys: "Ctrl+H",
                hint: "hide help bar",
            },
            Command {
                icon: "\u{1fa7a} Health Check",
                keys: "Alt+H",
                hint: "health check",
            },
            Command {
                icon: "\u{2699}\u{fe0f} Setup Services",
                keys: "Alt+X",
                hint: "setup",
            },
            Command {
                icon: "\u{1f3f7}\u{fe0f} Radio Browser",
                keys: "Alt+R",
                hint: "radio browse",
            },
        ]
    }
}

pub const COMMAND_GROUPS: &[(&str, usize)] = &[
    ("Playback", 12),
    ("Library & Queue", 15),
    ("View & Overlays", 11),
    ("System", 7),
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
    ("", "   > / <       Speed Up / Down (pitch-preserving)"),
    ("", "   z           Zen Mode (cover / lyrics / visualizer)"),
    ("", "   m           Mute Toggle"),
    ("", "   Alt+1       Mono Toggle"),
    ("", "   *           Love / Un-love on Last.fm"),
    ("", "   &           Toggle Last.fm Scrobbling"),
    ("", "   r           Repeat Mode"),
    ("", "   S           Shuffle Library"),
    ("", "   f           Toggle Favourite"),
    ("topic", "── Navigation ──"),
    ("", "   Tab         Switch Pane"),
    ("", "   Shift+Tab   Switch Pane (back)"),
    ("", "   /           Search (context-aware)"),
    ("", "   Alt+Q       Queue"),
    ("", "   Alt+/       Search Library"),
    ("", "   Alt+Y       YouTube Search"),
    ("", "   Alt+S       Spotify"),
    ("", "   Alt+P       Podcasts"),
    ("", "   Alt+R       Radio Browser"),
    ("", "   Alt+O       Play Stream URL"),
    ("topic", "── Playlists ──"),
    ("", "   e           Rename Playlist (overview)"),
    ("", "   d / Del     Delete Playlist (overview)"),
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
    ("", "   Shift+Up    Multiselect Up"),
    ("", "   Shift+Down  Multiselect Down"),
    ("", "   e           Edit Metadata"),
    ("topic", "── System ──"),
    ("", "   q           Quit"),
    ("", "   Q / Ctrl+Q  Quit Daemon"),
    ("", "   Alt+H       Health Check"),
    ("", "   Alt+X       Setup Services"),
    ("", "   Alt+,       Settings"),
];

pub const CROSSFADE_DURATIONS: [u8; 5] = [3, 5, 10, 15, 30];

impl Pickers {
    fn render_about(f: &mut ratatui::Frame, area: Rect, app: &App) {
        let block = Self::picker_panel(app, " About ", None);
        let inner = block.inner(area);
        f.render_widget(block, area);

        let version = option_env!("CARGO_PKG_VERSION").unwrap_or("0.1.0");
        let commit = option_env!("VERGEN_GIT_SHA").unwrap_or("unknown");
        // Nightly CI builds bake a `+nightly` suffix into `CARGO_PKG_VERSION`;
        // surface that so users can tell a nightly build from a release.
        let nightly = version.contains("+nightly");

        let mut lines = vec![
            Line::from(Span::styled(
                format!(
                    "gtm {version} ({:.7}{})",
                    commit,
                    if nightly { " nightly" } else { "" }
                ),
                Style::default()
                    .fg(app.theme.accent)
                    .add_modifier(Modifier::BOLD),
            )),
            Line::from(""),
            Line::from(Span::styled(
                "Copyright (C) 2026, prjctimg",
                Style::default().fg(app.theme.fg_dim),
            )),
            Line::from(Span::styled(
                "License GPL-3.0",
                Style::default().fg(app.theme.fg_dim),
            )),
            Line::from(""),
        ];

        // Decorative visualization under the text, cycling presets every 5s.
        let presets = VisualizerPreset::all();
        if !presets.is_empty() {
            let preset = presets[app.about_viz.preset % presets.len()];
            let width = inner.width.saturating_sub(2) as usize;
            if width > 0 && inner.height as usize > lines.len() + 4 {
                lines.extend(Self::visualizer_preview_lines(
                    preset,
                    &app.about_viz.bars,
                    width as u16,
                    app,
                ));
            }
        }

        let p = Paragraph::new(lines).alignment(Alignment::Center);
        f.render_widget(p, inner);
    }

    fn render_help(f: &mut ratatui::Frame, area: Rect, app: &mut App) {
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

    fn render_notifications(f: &mut ratatui::Frame, area: Rect, app: &mut App) {
        let block = Self::picker_panel(app, " Notifications ", None);
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
            app.mouse_map.register(row_rect, MouseZone::PickerItem(i));
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
                // Reserve one trailing row (when the preview pane is tall
                // enough) for the copy hint.
                let hint_h = u16::from(preview_area.height >= 4);
                let msg_area = Rect {
                    x: preview_area.x,
                    y: preview_area.y + header_h,
                    width: preview_area.width,
                    height: preview_area.height.saturating_sub(header_h + hint_h),
                };
                f.render_widget(preview_para, msg_area);
                if hint_h > 0 {
                    let hint = Paragraph::new(Line::from(Span::styled(
                        " y — copy notification to clipboard",
                        Style::default().fg(app.theme.fg_dim),
                    )));
                    f.render_widget(
                        hint,
                        Rect {
                            x: preview_area.x,
                            y: preview_area.y + preview_area.height - hint_h,
                            width: preview_area.width,
                            height: hint_h,
                        },
                    );
                }
            }
        }
    }

    fn render_notification_settings(f: &mut ratatui::Frame, area: Rect, app: &mut App) {
        let block = Self::picker_panel(app, " Notification Settings ", None);
        let inner = block.inner(area);
        f.render_widget(block, area);

        let sel = app
            .pickers
            .top()
            .map_or(0, |o| o.selected.min(NotifType::ALL.len() - 1));

        let mut lines: Vec<Line> = Vec::new();
        for (i, ntype) in NotifType::ALL.iter().enumerate() {
            let mode = app
                .notification_modes
                .get(ntype)
                .copied()
                .unwrap_or(NotifMode::Floating);
            let is_sel = i == sel;
            let style = if is_sel {
                Style::default()
                    .fg(app.theme.selection_fg_readable())
                    .bg(app.theme.selection_bg)
            } else {
                Style::default().fg(app.theme.fg)
            };
            let mode_style = if is_sel {
                style
            } else {
                Style::default().fg(app.theme.accent)
            };
            lines.push(Line::from(vec![
                Span::styled(format!(" {:<26}", ntype.label()), style),
                Span::styled(mode.label().to_string(), mode_style),
            ]));
        }
        let para = Paragraph::new(lines);
        f.render_widget(para, inner);
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

    fn render_sleep_timer(f: &mut ratatui::Frame, area: Rect, app: &App) {
        if app.sleep_timer.input_mode {
            let block = Self::picker_panel(app, " Sleep Timer: Manual Input ", None);
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
            "↑/↓: option   ←/→ or h/l: ±5   -/+: ±1   i: input   Enter: set   c: cancel"
        } else {
            "↑/↓: option   ←/→ or h/l: ±5   -/+: ±1   i: input   Enter: set"
        };
        let block = Self::picker_panel(app, " Sleep Timer ", Some(help));
        let inner = block.inner(area);
        f.render_widget(block, area);

        let mins = app.sleep_timer.minutes;
        let focus = app.sleep_timer.focus;

        let focus_style = |app: &App, active: bool| {
            if active {
                Style::default()
                    .fg(app.theme.selection_fg_readable())
                    .bg(app.theme.selection_bg)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(app.theme.fg)
            }
        };

        let mut lines: Vec<Line> = Vec::new();

        // Row 0: the time slider (focus 0). Left/Right and h/l adjust it;
        // Enter arms the timer with the current minutes.
        lines.push(Line::from(vec![Span::styled(
            format!("  Timer: {} minutes", mins),
            focus_style(app, focus == 0),
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

        // Rows 1..=7: quick presets. `focus - 1` indexes into this list, and
        // the focused chip is highlighted (the value matches `mins` whenever
        // the user adjusted the slider to a preset).
        let quick_opts = [5u32, 10, 15, 30, 60, 90, 120];
        let mut spans: Vec<Span> = vec![Span::styled("  ", Style::default())];
        for (i, &m) in quick_opts.iter().enumerate() {
            let focused = (1..=7).contains(&focus) && focus - 1 == i;
            let style = if focused {
                Style::default()
                    .fg(app.theme.selection_fg_readable())
                    .bg(app.theme.accent)
                    .add_modifier(Modifier::BOLD)
            } else if m == mins {
                Style::default()
                    .fg(app.theme.selection_fg_readable())
                    .bg(app.theme.selection_bg)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(app.theme.fg_dim)
            };
            spans.push(Span::styled(format!("[{}m] ", m), style));
        }
        lines.push(Line::from(spans));
        lines.push(Line::from(""));

        // Row 8: the "stop immediately" checkbox.
        let boxed = if app.sleep_timer.stop_immediately {
            "[x]"
        } else {
            "[ ]"
        };
        lines.push(Line::from(vec![Span::styled(
            format!("  {} End playback immediately when the time is up", boxed),
            focus_style(app, focus == 8),
        )]));
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

    fn command_palette(f: &mut ratatui::Frame, area: Rect, app: &mut App) {
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

    fn render_equalizer(f: &mut ratatui::Frame, area: Rect, app: &mut App) {
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

        let block = Self::picker_panel(app, " Equalizer ", None);
        let inner = block.inner(area);
        f.render_widget(block, area);

        let preview_height: u16 = 5;
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
        for (i, (name, _desc, _eq)) in presets
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
            } else if *name == app.state.audio.eq_preset.label() {
                Style::default().fg(app.theme.success)
            } else {
                Style::default()
            };
            let spans = vec![Span::styled(format!("{prefix}{}", name), Style::default())];
            list_items.push(ListItem::new(Line::from(spans)).style(style));
        }

        let list = List::new(list_items);
        f.render_widget(list, list_area);

        if preview_area.height >= 4 {
            let selected_preset = presets.get(sel);
            let selected_name = selected_preset.map(|p| p.0).unwrap_or("");
            let selected_desc = selected_preset.map(|p| p.1).unwrap_or("");
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

            let selected_eq = selected_preset.map(|p| p.2);

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

            // Render description below visualization
            if !selected_desc.is_empty() {
                f.render_widget(
                    Paragraph::new(Line::from(vec![Span::styled(
                        format!("  {selected_desc}"),
                        Style::default().fg(app.theme.fg_dim),
                    )])),
                    Rect {
                        x: preview_area.x,
                        y: preview_area.y + 2,
                        width: preview_area.width,
                        height: 1,
                    },
                );
            }
        }
    }

    fn eq_preset_preview(eq: EqPreset, app: &App) -> Vec<Span<'static>> {
        const BLOCKS: [char; 8] = ['▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];
        const BRAILLE: [char; 8] = ['⠁', '⠃', '⠇', '⡇', '⣇', '⣧', '⣷', '⣿'];
        let chars: &[char; 8] = match app.visualizer.preset {
            VisualizerPreset::Braille | VisualizerPreset::Gradient => &BRAILLE,
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
        preset: VisualizerPreset,
        bars: &[f32],
        width: u16,
        app: &App,
    ) -> Vec<Line<'static>> {
        let w = width as usize;
        let mut lines = Vec::new();

        match preset {
            VisualizerPreset::Braille => {
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
            VisualizerPreset::Blocks | VisualizerPreset::Mirror => {
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
            VisualizerPreset::Gradient => {
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
            VisualizerPreset::Spectrum => {
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
            VisualizerPreset::BarsDot => {
                for row in 0..2 {
                    let mut spans = Vec::with_capacity(w);
                    for &b in bars.iter().take(w) {
                        let level = (b * 7.0) as u32;
                        let ch = if row == 0 {
                            match level {
                                6.. => '⣿',
                                4.. => '⣷',
                                2.. => '⣧',
                                1.. => '⠇',
                                _ => '⠀',
                            }
                        } else {
                            match level {
                                4.. => '⣿',
                                2.. => '⡇',
                                1.. => '⠁',
                                _ => '⠀',
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
            VisualizerPreset::ClassicPeak => {
                for row in 0..2 {
                    let mut spans = Vec::with_capacity(w);
                    for &b in bars.iter().take(w) {
                        let (ch, color) = if row == 0 {
                            if b > 0.78 {
                                ('⎺', app.theme.accent)
                            } else {
                                (' ', app.theme.bg)
                            }
                        } else if b > 0.35 {
                            ('▏', app.theme.fg_bright)
                        } else {
                            (' ', app.theme.bg)
                        };
                        spans.push(Span::styled(ch.to_string(), Style::default().fg(color)));
                    }
                    lines.push(Line::from(spans));
                }
            }
            VisualizerPreset::Columns => {
                for row in 0..2 {
                    let mut spans = Vec::with_capacity(w);
                    for &b in bars.iter().take(w) {
                        let (ch, color) = if row == 0 {
                            if b > 0.55 {
                                ('█', app.theme.warning)
                            } else {
                                (' ', app.theme.bg)
                            }
                        } else if b > 0.05 {
                            ('█', app.theme.accent)
                        } else {
                            (' ', app.theme.bg)
                        };
                        spans.push(Span::styled(ch.to_string(), Style::default().fg(color)));
                    }
                    lines.push(Line::from(spans));
                }
            }
            VisualizerPreset::Wave => {
                // One-dot braille sweep: dot row follows the sine, column
                // alternates for sub-cell horizontal resolution.
                const ONE_DOT: [char; 8] = ['⠁', '⠂', '⠄', '⡀', '⠈', '⠐', '⠠', '⣀'];
                for _ in 0..2 {
                    let mut spans = Vec::with_capacity(w);
                    for i in 0..w {
                        let t = i as f64 / w.max(1) as f64;
                        let v = (t * std::f64::consts::TAU * 1.5).sin();
                        let y_dot = ((1.0 - v) * 1.5) as usize;
                        let ch = ONE_DOT[y_dot.min(3) + if i % 2 == 0 { 0 } else { 4 }];
                        spans.push(Span::styled(
                            ch.to_string(),
                            Style::default().fg(app.theme.accent),
                        ));
                    }
                    lines.push(Line::from(spans));
                }
            }
            VisualizerPreset::Stereo => {
                let meter = w.saturating_sub(3);
                for (lane, label) in [(0usize, "L "), (1usize, "R ")] {
                    let mut spans = vec![Span::styled(
                        label,
                        Style::default()
                            .fg(app.theme.fg_dim)
                            .add_modifier(ratatui::style::Modifier::BOLD),
                    )];
                    for (x, &b) in bars.iter().take(meter).enumerate() {
                        let lvl = if lane == 0 { b } else { b * 0.72 };
                        let filled = (lvl * meter as f32) as usize;
                        let (ch, color) = if x < filled {
                            ('█', app.theme.accent)
                        } else if x == filled && x < meter {
                            ('▌', app.theme.secondary_accent)
                        } else if x == meter.saturating_sub(1) && lvl > 0.8 {
                            ('·', app.theme.accent)
                        } else {
                            (' ', app.theme.bg)
                        };
                        spans.push(Span::styled(ch.to_string(), Style::default().fg(color)));
                    }
                    lines.push(Line::from(spans));
                }
            }
            VisualizerPreset::Retro => {
                for row in 0..3 {
                    let mut spans = Vec::with_capacity(w);
                    for i in 0..w {
                        let center = i as f64 / w.max(1) as f64;
                        let dist = (center - 0.5).abs() * 2.0;
                        let (ch, color) = match row {
                            0 if dist < 0.62 => ('⠛', app.theme.accent),
                            0 => (' ', app.theme.bg),
                            1 => ('▔', app.theme.fg_dim),
                            _ if dist < 0.4 => ('⣀', app.theme.fg_dim),
                            _ if dist < 0.75 => ('⣀', app.theme.secondary_accent),
                            _ => ('⣀', app.theme.tertiary_accent),
                        };
                        spans.push(Span::styled(ch.to_string(), Style::default().fg(color)));
                    }
                    lines.push(Line::from(spans));
                }
            }
            VisualizerPreset::Flame => {
                for row in 0..2 {
                    let mut spans = Vec::with_capacity(w);
                    for &b in bars.iter().take(w) {
                        let (ch, color) = if row == 0 {
                            if b > 0.6 {
                                ('⠛', app.theme.warning)
                            } else if b > 0.25 {
                                ('⠉', app.theme.secondary_accent)
                            } else {
                                (' ', app.theme.bg)
                            }
                        } else {
                            ('⣀', app.theme.accent)
                        };
                        spans.push(Span::styled(ch.to_string(), Style::default().fg(color)));
                    }
                    lines.push(Line::from(spans));
                }
            }
        }
        lines
    }

    fn render_theme(f: &mut ratatui::Frame, area: Rect, app: &mut App) {
        let query = app.pickers.top().map_or(String::new(), |o| o.query.clone());
        let filtered: Vec<_> = app
            .themes
            .iter()
            .enumerate()
            .filter(|(_, entry)| fuzzy_match(&query, &entry.name))
            .collect();

        let total = filtered.len();
        let sel = app
            .pickers
            .top()
            .map_or(0, |o| o.selected.min(total.saturating_sub(1)));

        let block = Self::picker_panel(app, " Theme ", None);
        let inner = block.inner(area);
        f.render_widget(block, area);

        let cursor_style = cursor_span_style(app);
        let search_line = Line::from(vec![
            Span::styled(" > ", Style::default().fg(app.theme.fg_dim)),
            Span::styled(query.as_str(), Style::default().fg(app.theme.fg)),
            Span::styled(" ", cursor_style.unwrap_or_default()),
        ]);

        // Each theme row is padded with a blank line so the swatch runs don't
        // visually merge into one continuous color band; the math must count
        // rows at 2 cells each (the search line stays 1 high).
        let item_h: u16 = 2;
        let visible = (inner.height.saturating_sub(1) / item_h) as usize;
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
        for (visible_idx, &(i, entry)) in filtered[scroll_start..scroll_end].iter().enumerate() {
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
            list_items.push(ListItem::new(vec![Line::from(spans), Line::from("")]).style(style));
            let row_rect = Rect {
                x: inner.x,
                y: inner.y + 1 + (visible_idx as u16) * item_h,
                width: inner.width,
                height: item_h,
            };
            app.mouse_map.register(row_rect, MouseZone::PickerItem(i));
        }

        let list = List::new(list_items);
        f.render_widget(list, inner);
    }

    fn render_crossfade(f: &mut ratatui::Frame, area: Rect, app: &App) {
        let block = Self::picker_panel(app, " Crossfade Options ", None);
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

    fn render_visualizer_preset(f: &mut ratatui::Frame, area: Rect, app: &mut App) {
        let block = Self::picker_panel(app, " Visualizer Preset ", None);
        let inner = block.inner(area);
        f.render_widget(block, area);

        let current = app.visualizer.preset;
        let presets = VisualizerPreset::all();

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

    fn render_progress_style(f: &mut ratatui::Frame, area: Rect, app: &mut App) {
        let block = Self::picker_panel(app, " Progress Style ", None);
        let inner = block.inner(area);
        f.render_widget(block, area);

        let current = app.progress_style;
        let styles = ProgressStyle::all();

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
            let spans = render_progress_styled(
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

    fn render_footer_preset(f: &mut ratatui::Frame, area: Rect, app: &mut App) {
        let block = Self::picker_panel(app, " Footer Preset ", None);
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
    fn render_playlist_select(f: &mut ratatui::Frame, area: Rect, app: &App) {
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

    /// Multi-select track picker shown right after a playlist is created:
    /// every track is listed and `Space` toggles a persistent highlight, with
    /// `Ctrl+Enter` committing the selection. Highlights survive scrolling.
    fn render_track_select(f: &mut ratatui::Frame, area: Rect, app: &mut App) {
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

    fn render_edit_metadata(f: &mut ratatui::Frame, area: Rect, app: &mut App) {
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

#[cfg(test)]
mod tests {
    use super::scroll_text;

    #[test]
    fn scroll_multibyte() {
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
    fn scroll_pads_fits() {
        assert_eq!(scroll_text("ab", 4, 0, true), "ab  ");
    }

    #[test]
    fn scroll_empty_panics() {
        let out = scroll_text("", 0, 1, true);
        assert_eq!(out, "");
    }
}
