// Copyright (c) 2026
// Author: prjctimg <prjctimg@outlook.com>
// Main TUI chrome: layout, panes and the render loop
//
//
// This is free software released under the GPL-3.0 license.

use crate::app::{
    GRID_CELL_H, GRID_CELL_W, LIB_ADDED, LIB_ALL, LIB_CATEGORIES, LIB_CHARTS, LIB_FOLDERS,
    LIB_LIKED, LIB_PLAYED, LIB_PLAYLISTS, LIB_PODCASTS, LIB_RADIO, LIB_RECENT, LIB_SPOTIFY,
    LibraryFilter,
};
use crate::ui::*;

/// Rows of cover art in the one-column bottom-pane track card.
///
/// A preview, not a second docked card: the pane exists to describe the list
/// above it, and six rows of half-block art is 12 columns of recognisable
/// album while still leaving the list most of the screen.
const DOCK_ART_H: u16 = 6;

/// Which background the lyric lines are drawn on.
///
/// The two callers share every line of the layout and differ only here: the
/// pane's foregrounds come from the theme, which was authored against the app
/// surface, while the Zen surface's are re-derived from its own background
/// because that background is artwork.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LyricSurface {
    Pane,
    Zen,
}

/// Whether an album line is worth a row next to a track's title and artist.
///
/// A release with no artist is a single, and on a single the album field is
/// either empty or a compilation the track does not belong to — "Greatest
/// Hits", a VA compilation, the label's own imprint. Neither tells the listener
/// anything about the track they are looking at, and the row was spent on it
/// anyway, pushing the progress bar down.
pub(crate) fn wants_album_line(artist: &str, album: &str) -> bool {
    !album.trim().is_empty() && !artist.trim().is_empty()
}

impl Render {
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
            let final_x = center_x(max_notif_width);
            let leaving = now.saturating_duration_since(n.expires_at);
            // The card enters from off the right edge and leaves the same way.
            // It used to drop in from above the top of the screen, which read as
            // a glitch: the card was cut in half by the terminal edge and
            // appeared without its background for the frames it spent partly
            // off-screen. Coming from the side keeps the whole card visible for
            // the whole animation and leaves the top edge alone.
            let travel = (area.right().saturating_sub(final_x)) as f32;
            let (y, x) = if leaving > std::time::Duration::ZERO {
                let p = cubic_ease_in(
                    (leaving.as_millis() as f32 / NOTIFICATION_EXIT_DURATION.as_millis() as f32)
                        .min(1.0),
                );
                (final_y, final_x as f32 + travel * p)
            } else {
                let p = cubic_ease_out(n.animation_progress);
                (final_y, final_x as f32 + travel * (1.0 - p))
            };

            let card_area = Rect {
                x: (x.round() as u16).min(area.right().saturating_sub(max_notif_width)),
                y,
                width: max_notif_width,
                height: card_h,
            };

            // Opaque, and filled before the accent bar so no row of the card is
            // ever left showing the surface underneath.
            let bg = Block::default().style(Style::default().bg(app.notification_bg()));
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
    /// now-playing surface, the lyrics, or the visualizer.
    ///
    /// Daydreaming borrows this path and always lands on the visualizer, which
    /// is the surface it exists to show: nobody is watching a track list during
    /// a preview, and showing the now-playing artwork would be indistinguishable
    /// from Zen having opened by accident.
    pub(crate) fn zen(f: &mut ratatui::Frame, area: Rect, app: &mut App) {
        let surface = if app.daydreaming {
            ZenSurface::Visualizer
        } else {
            app.zen_surface
        };
        match surface {
            ZenSurface::NowPlaying => Render::zen_now_playing(f, area, app),
            ZenSurface::Lyrics => Render::zen_lyrics(f, area, app),
            ZenSurface::Visualizer => Render::zen_visualizer(f, area, app),
        }
    }

    /// Centered title on the first row, artist on the second.
    ///
    /// They were one line joined by an em dash, which is fine until a title or
    /// an artist is long: the pair then truncates at the right edge and the
    /// half that got cut is the part that identifies the track.
    pub(crate) fn zen_track_header(
        f: &mut ratatui::Frame,
        app: &App,
        track: &TrackInfo,
        area: Rect,
    ) {
        let title = Paragraph::new(Line::from(Span::styled(
            track.display_title(),
            Style::default()
                .fg(app.theme.secondary_accent)
                .add_modifier(Modifier::BOLD),
        )))
        .alignment(Alignment::Center);
        f.render_widget(title, area);
        if area.height < 2 {
            return;
        }
        if track.artist.is_empty() {
            return;
        }
        let artist = Paragraph::new(Line::from(Span::styled(
            track.artist.clone(),
            Style::default().fg(app.theme.fg),
        )))
        .alignment(Alignment::Center);
        f.render_widget(
            artist,
            Rect {
                x: area.x,
                y: area.y + 1,
                width: area.width,
                height: 1,
            },
        );
    }

    /// Zen surface 1: enlarged cover art centered on screen, the title and
    /// artist above it on their own lines, the current lyric line directly
    /// under the art, and the progress bar / elapsed time below that.
    pub(crate) fn zen_now_playing(f: &mut ratatui::Frame, area: Rect, app: &mut App) {
        let track = app.state.current_track.clone();
        // One row is always reserved for the lyric line, whether or not there
        // is a lyric to show, so toggling `l` does not resize the artwork.
        //
        // The progress band used to be read out of `vchunks[2]` — the artwork's
        // own band — which drew the bar over the top of the cover and the
        // elapsed time two rows into it, and left the 3 rows reserved below
        // never rendered at all. That read as a cover floating too high with a
        // band of dead space under it.
        //
        // The fifth band is surface: three rows reserved under the progress
        // indicator, so the lyric line and the bar sit three rows higher than
        // they did pinned to the bottom edge. It comes out of the artwork band
        // rather than being added to the layout, which is what keeps the
        // composition the same overall height.
        let vchunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(2),
                Constraint::Min(0),
                Constraint::Length(1),
                Constraint::Length(3),
                Constraint::Length(3),
            ])
            .split(area);

        if let Some(t) = &track {
            Render::zen_track_header(f, app, t, vchunks[0]);
        }

        // Everything the cover is not: the header above it, and the lyric line
        // and progress below it. The artwork centres in the band that is left,
        // so the composition as a whole sits in the middle of the screen rather
        // than the art alone.
        let lyric_rect = vchunks[2];
        // Enlarged cover centred in what is left. Half-block art keeps the
        // image square at a 1:2 cell aspect, so width = height * 2.
        let art_band = vchunks[1];
        let max_w = art_band.width.saturating_sub(4);
        let max_h = art_band.height.saturating_sub(2);
        let mut w = max_w.min(max_h.saturating_mul(2));
        let mut h = w / 2;
        if h > max_h {
            h = max_h;
            w = h.saturating_mul(2);
        }
        h = h.max(1);
        w = w.max(2);
        let cover_area = Rect {
            x: art_band.x + art_band.width.saturating_sub(w) / 2,
            y: art_band.y + art_band.height.saturating_sub(h) / 2,
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
        Render::zen_lyric_line(f, lyric_rect, app);

        // Centered progress bar with elapsed / total underneath; hidden for
        // live streams (mirrors the now-playing pane).
        let prog = vchunks[3];
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
                Rect {
                    x: prog.x,
                    y: prog.y + 1,
                    width: prog.width,
                    height: 1,
                },
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
                    y: prog.y + 3,
                    width: prog.width,
                    height: 1,
                },
            );
        }
    }

    /// The one lyric line that belongs under the Zen artwork.
    ///
    /// Only the line being sung: a full-screen lyrics view is what Zen already
    /// was, and the complaint was that it was a separate surface to Tab into
    /// rather than something you could see while looking at the cover.
    fn zen_lyric_line(f: &mut ratatui::Frame, area: Rect, app: &App) {
        if area.height == 0 {
            return;
        }
        let line = if app.lyrics.show {
            match app.lyrics.current.as_ref() {
                Some(lyrics) if !lyrics.lines.is_empty() => {
                    let idx = app.current_lyric_index();
                    Some(lyrics.lines.get(idx).map(|l| l.text.as_str())).flatten()
                }
                Some(_) => Some("No lyrics found"),
                None => Some(if app.lyrics.fetching {
                    "Fetching lyrics..."
                } else {
                    "No lyrics"
                }),
            }
        } else {
            None
        };
        let Some(text) = line.filter(|t| !t.trim().is_empty()) else {
            return;
        };
        f.render_widget(
            Paragraph::new(Line::from(Span::styled(
                text.to_string(),
                Style::default().fg(app.theme.fg),
            )))
            .alignment(Alignment::Center),
            area,
        );
    }

    /// Zen surface 2: full-screen lyrics for the track on air, sharing the
    /// exact same body rendering as the normal lyrics pane.
    ///
    /// The now-playing surface already carries the line being sung, one row
    /// under the artwork. This is the other half of that: the whole song, for
    /// reading along to, on the surface Zen exists to give a track.
    pub(crate) fn zen_lyrics(f: &mut ratatui::Frame, area: Rect, app: &mut App) {
        let header = Rect {
            x: area.x,
            y: area.y,
            width: area.width,
            height: 2,
        };
        if let Some(t) = app.state.current_track.clone() {
            Render::zen_track_header(f, app, &t, header);
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
            width: area.width.saturating_sub(4).max(16),
            height: area.height.saturating_sub(4),
        };
        Render::lyrics_body(f, body, app, lyrics, LyricSurface::Zen);
    }

    /// Zen surface 3: the audio visualizer stretched across the full screen.
    ///
    /// Reached only by cycling Zen's surfaces, so there is no toggle to consult
    /// here: asking for the surface is the whole of the request. The
    /// `[extensions] visualizer` kill switch is honoured by the caller, which
    /// declines to pay for the frames when it is off.
    pub(crate) fn zen_visualizer(f: &mut ratatui::Frame, area: Rect, app: &mut App) {
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

    /// The rect the artwork will actually occupy inside `area`, centred.
    ///
    /// A protocol fits the image and never upscales it past its own pixel size,
    /// so on a small cover the rendered block is smaller than the box it was
    /// handed — and the protocol draws from the top-left of whatever rect it
    /// gets. Handing it the full box therefore pinned the art to the corner:
    /// the Zen cover sat high and left of centre, and the floating card's art
    /// sat in the corner of a box sized for a much larger one. Asking the
    /// protocol for the size it will use and centring that is the same
    /// geometry for every pane.
    pub(crate) fn cover_fit(area: Rect, protocol: &StatefulProtocol) -> Rect {
        let fit = protocol.size_for(Resize::Fit(None), Size::new(area.width, area.height));
        let w = fit.width.min(area.width).max(1);
        let h = fit.height.min(area.height).max(1);
        Rect {
            x: area.x + area.width.saturating_sub(w) / 2,
            y: area.y + area.height.saturating_sub(h) / 2,
            width: w,
            height: h,
        }
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
            // Centred, and clipped to the area. It was neither: the message
            // hugged the top-left of whatever box it was handed, which in Zen
            // is the middle of the whole screen.
            let msg = " \u{266b} Cover art unavailable in this terminal ";
            let shown: String = msg.chars().take(area.width as usize).collect();
            let line = format!("{:^width$}", shown, width = area.width as usize);
            f.render_widget(
                Paragraph::new(line)
                    .alignment(Alignment::Center)
                    .style(Style::default().fg(placeholder_fg)),
                Rect {
                    y: area.y + area.height.saturating_sub(1) / 2,
                    height: 1,
                    ..area
                },
            );
            return;
        }
        if area.width == 0 || area.height == 0 {
            return;
        }
        if let Some(protocol) = cover_stateful {
            let image = StatefulImage::new();
            f.render_stateful_widget(image, Render::cover_fit(area, protocol), protocol);
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

        // Rows the now-playing cover may occupy. Also the row the left pane's
        // category list aligns to, so the two blocks read as one unit: the
        // cover is what the list is browsing.
        let cover_band: u16 = if is_small_height {
            (np_height.saturating_sub(3)).clamp(2, 7)
        } else {
            np_height.saturating_sub(3).min(12)
        };

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

        // The library column runs the full height with the now-playing band
        // stacked over the results to its right, so the band starts exactly
        // where the results start: the cover, the title and the progress used
        // to sit over the library column, half a screen away from the list
        // they belonged to.
        //
        // Narrow keeps the old order — the band across the full width, the two
        // panes below it. Only one of those panes has width at a time, so a
        // band placed to their right would be gone whenever the library held
        // the cursor, and a now-playing pane that comes and goes with the Tab
        // key is not a now-playing pane.
        let (np_area, lib_area, results_area) = if is_narrow {
            let v = Layout::default()
                .direction(Direction::Vertical)
                .constraints([Constraint::Length(np_height), Constraint::Min(1)])
                .split(left_area);
            let h = Layout::default()
                .direction(Direction::Horizontal)
                .constraints(if app.library_pane_focus {
                    [Constraint::Min(0), Constraint::Length(0)]
                } else {
                    [Constraint::Length(0), Constraint::Min(0)]
                })
                .split(v[1]);
            (v[0], h[0], h[1])
        } else {
            let h = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([Constraint::Length(lib_width), Constraint::Min(0)])
                .split(left_area);
            let v = Layout::default()
                .direction(Direction::Vertical)
                .constraints([Constraint::Length(np_height), Constraint::Min(1)])
                .split(h[1]);
            (v[0], h[0], v[1])
        };

        let left_focus = app.library_pane_focus;

        {
            // No rule under the label. The now-playing label is empty, so the
            // only thing the rule ever separated was the cover art from the
            // pane's own edge — a line across the top of the artwork, which is
            // the one place a border reads as damage.
            let np_inner = Render::pane_header(f, np_area, app, "", false, false, false);
            fill_pane(f, np_inner, app);

            if let Some(track) = app.state.current_track.clone() {
                let inner = np_inner;
                // Narrow panes have no row to spare: the 2-row padding meant
                // for breathing room around the cover cost half the art, and on
                // a 5-row pane left it 4x2 cells. The cover fills the pane
                // there instead, which is wider than the padding it displaces.
                let avail_h = if is_narrow {
                    inner.height
                } else {
                    inner.height.saturating_sub(2)
                };
                // `cover_band` is computed once in `library` so the left pane's
                // list can align to it; here it is only bounded by what the
                // band actually has room for. One column has no left pane to
                // align to, and its band is four rows, so the artwork takes
                // all of them: the band cap was holding a single-column cover
                // to two rows beside two rows of labels, which is the one
                // layout where the artwork is the smaller half.
                let cover_h = if is_narrow {
                    avail_h
                } else if is_small_height {
                    avail_h.clamp(2, 7)
                } else {
                    avail_h.min(cover_band)
                };
                // Half-block art is square at a 1:2 cell aspect, so the width
                // follows the height. The band lost the library column's width
                // when it moved to the results column, and a cover that kept its
                // full 24 columns on a 60-column terminal would leave the title,
                // artist, album and progress eight columns between them — so it
                // gives up the columns the text needs first, down to a 3-row
                // thumbnail.
                //
                // The reserve is 16 columns, not 20. Two thirds of the band's
                // height on a narrow terminal was going to the title block rather
                // than the artwork: 20 columns is enough for the longest detail
                // line at the sizes this renders, and the two columns it gave
                // back are a whole extra row of art at a 1:2 aspect.
                let cover_h = cover_h.min(inner.width.saturating_sub(16) / 2).max(3);
                let cover_w = cover_h * 2;

                // A live stream reports the track on air over ICY (or through
                // the station's tracklist), and the daemon's synthesised track
                // carries only the station name. Prefer the live title and the
                // artist half the tracklist supplies, falling back to the
                // station's own naming.
                let (display_title, display_artist, is_live) = match app.live_track() {
                    Some((title, artist)) => (title, artist, true),
                    None => {
                        let title = track.display_title();
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
                let has_album = !is_live && wants_album_line(&display_artist, &track.album);

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
                if inner.width >= cover_w + 16 && (is_narrow || avail_h >= 5 || is_small_height) {
                    let hchunks = Layout::default()
                        .direction(Direction::Horizontal)
                        .constraints([
                            Constraint::Length(cover_w),
                            Constraint::Length(2),
                            Constraint::Min(0),
                        ])
                        .split(inner);

                    // Match the left pane's card: centre the artwork in its
                    // column rather than pinning it to the pane's left edge, and
                    // drop the extra `y + 1`, which pushed the image a row below
                    // the left pane's cover so the two never lined up. Centred
                    // vertically as well -- pinned to the top of a tall pane it
                    // left a band of dead space under it that read as a
                    // rendering fault rather than as layout.
                    let col = hchunks[0];
                    let ch = cover_h.min(col.height);
                    let cw = cover_w.min(col.width);
                    let cover_area = Rect {
                        x: col.x + col.width.saturating_sub(cw) / 2,
                        y: col.y + col.height.saturating_sub(ch) / 2,
                        width: cw,
                        height: ch,
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

                    // The label block takes the rows it can: the title and the artist
                    // always, then the album, the bar and the elapsed time, and
                    // whatever does not fit is dropped from the bottom. One
                    // column has four rows beside a four-row cover, so it keeps
                    // the album and the bar and loses the elapsed time — which
                    // the footer's `Time` module carries there.
                    let mut info_constraints = vec![Constraint::Length(1), Constraint::Length(1)];
                    if has_album {
                        info_constraints.push(Constraint::Length(1));
                    }
                    if has_progress {
                        info_constraints.push(Constraint::Length(1));
                        info_constraints.push(Constraint::Length(1));
                    }
                    info_constraints.truncate(avail_h.max(2) as usize);
                    let content_h = info_constraints.len() as u16;
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

                    // Every label scrolls on the same clock: they share one
                    // offset, so a long artist does not start moving a moment
                    // after the long title did. `scroll_text` pads a short
                    // string to the full width, which keeps the block from
                    // flickering as the track changes.
                    let avail = info_chunks[0].width as usize;
                    let artist_para = Paragraph::new(Line::from(vec![Span::styled(
                        scroll_text(&display_artist, avail, app.np_title_scroll, true),
                        Style::default().fg(app.theme.fg_bright),
                    )]));
                    f.render_widget(artist_para, info_chunks[1]);

                    let mut info_row = 2;
                    if has_album && let Some(area) = info_chunks.get(info_row) {
                        let album_para = Paragraph::new(Line::from(vec![Span::styled(
                            scroll_text(&track.album, avail, app.np_title_scroll, true),
                            Style::default().fg(app.theme.fg_bright),
                        )]));
                        f.render_widget(album_para, *area);
                        info_row += 1;
                    }
                    if has_progress && let Some(area) = info_chunks.get(info_row) {
                        let pos = app.display_position as u64;
                        let ratio = (pos as f64 / dur as f64).clamp(0.0, 1.0);
                        let bar_w = (area.width / 3).saturating_sub(2).max(4) as usize;
                        let progress_str = Render::progress_variant(ratio, bar_w, app);
                        let time_str =
                            format!(" {} / {}", format_duration(pos), format_duration(dur));
                        // Progress bar on first line
                        let prog_para = Paragraph::new(Line::from(vec![Span::styled(
                            progress_str,
                            Style::default().fg(app.theme.secondary_accent),
                        )]));
                        f.render_widget(prog_para, *area);
                        // Elapsed time on second line
                        if let Some(area) = info_chunks.get(info_row + 1) {
                            let time_para = Paragraph::new(Line::from(vec![Span::styled(
                                time_str,
                                Style::default().fg(app.theme.fg_dim),
                            )]));
                            f.render_widget(time_para, *area);
                        }
                    }
                } else if inner.width >= 12 {
                    // Compact layout still keeps the cover left with the
                    // details to its right (scaled to a slim column) whenever
                    // there is any horizontal room at all.
                    let slim_cover_h = inner.height.saturating_sub(2).clamp(2, 6);
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

        // The left pane carries the category list again, alongside the Alt+.
        // picker, so the highlight is visible without opening anything. The
        // list is capped rather than grown: an uncapped one would take every
        // row the pane has and squeeze the cover art to nothing, so the number
        // of visible items is derived from what is left after reserving the
        // card and one padding row above it.
        let left_inner = Render::pane_header(f, lib_area, app, " ", left_focus, false, false);
        fill_pane(f, left_inner, app);

        // The left pane's info slot shows the track card, or the highlighted
        // Spotify playlist's cover while that list is open. Either one is
        // enough to reserve the slot, so the two conditions are OR'd rather
        // than nested — gating the playlist case on the track popup would leave
        // it at zero height for most of a browsing session.
        let want_track_card = app.show_preview && app.track_popup_visible && !is_small_height;
        let want_playlist_card = app.in_spotify_playlists() && app.spotify.list_cover.is_some();
        // The Spotify drill-down shows its own highlighted track here rather
        // than inline with the list rows, so the list panes stay pure text.
        let want_row_card = app.in_spotify_playlist() && app.spotify.row_cover.is_some();
        // A chart row renders through the same track card, so it is governed by
        // `want_track_card` and needs no condition of its own.
        let has_card = (want_track_card || want_playlist_card || want_row_card) && !is_small_height;
        // Clearance between the list and the card: the padding row plus the
        // gap that keeps the artwork from reading as a clipped list row.
        let card_gap = if has_card {
            LEFT_LIST_PADDING + INFO_CARD_GAP
        } else {
            0
        };
        // Sit the first category level with the cover image beside it.
        let list_top = left_list_top(cover_band);
        // Rows the category list takes. The list is served first and the card
        // takes what is left: the card's artwork is already sized from the box
        // it is handed, so it gives up rows gracefully, whereas a list cut to
        // zero does not come back until the cursor leaves the category. That is
        // the bug this fixes — four categories always have a card, so those are
        // the four whose list vanished on a pane that could have held both.
        let list_rows: u16 = if has_card {
            // A card needs its field block and its separator to be legible;
            // anything beyond that is artwork, and artwork is what shrinks.
            let avail = left_inner
                .height
                .saturating_sub(list_top)
                .saturating_sub(card_gap)
                .saturating_sub(1);
            let card_floor = INFO_TEXT_H + 1;
            let for_list = avail.saturating_sub(card_floor).min(LEFT_LIST_MAX_ROWS);
            for_list.max(LEFT_LIST_MIN_ROWS.min(avail))
        } else {
            left_inner.height.saturating_sub(list_top)
        };
        let track_info_h: u16 = if has_card {
            let left = left_inner
                .height
                .saturating_sub(list_rows)
                .saturating_sub(list_top)
                .saturating_sub(card_gap)
                .saturating_sub(1);
            info_block_h().min(left)
        } else {
            0
        };
        let left_vchunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                // Clearance above the list, so its first row sits level with
                // the now-playing cover rather than under the pane header.
                Constraint::Length(list_top),
                Constraint::Length(list_rows),
                Constraint::Length(card_gap),
                Constraint::Length(
                    if (want_track_card || want_playlist_card) && !is_small_height {
                        1
                    } else {
                        0
                    },
                ),
                Constraint::Length(track_info_h),
            ])
            .split(left_inner);
        let left_list_area = left_vchunks[1];
        let left_pad_area = left_vchunks[2];
        let info_sep_area = left_vchunks[3];
        let left_info_area = left_vchunks[4];

        if list_rows > 0 {
            let lib_icons = if use_nerd_fonts() {
                LIBRARY_ICONS_NERD
            } else {
                LIBRARY_ICONS_ASCII
            };
            let visible_cats = app.visible_library_indices();
            let total = visible_cats.len();
            let sel = visible_cats
                .iter()
                .position(|&i| i == app.library_category)
                .unwrap_or(0);
            // Fed a real offset rather than a hardcoded 0. The category list is
            // every category, while the results pane is one category's rows, so
            // a pane shorter than the list has to scroll -- and with the offset
            // pinned at 0 the window was always rows 0..list_rows: everything
            // past the pane height was never drawn, so on a short terminal the
            // categories after "Recently Played" simply were not there, while
            // j/k still moved the selection into them and the highlight
            // vanished off-screen. Persisted back each frame, as the results
            // pane does with `list_scroll`, so the window settles instead of
            // resetting every redraw.
            let (scroll_start, scroll_end) =
                step_viewport(app.left_list_scroll, sel, list_rows as usize, total);
            app.left_list_scroll = scroll_start;
            // The icon is its own span so the selected row can invert it.
            //
            // It used to be part of the label string, which meant the
            // selection style applied to the glyph exactly like the text: the
            // indicator bar then landed on the icon cell wearing the selection
            // background as its foreground colour, and in every built-in theme
            // those two are the same value — the selected category had no icon
            // at all. Inverting is the same trick the row itself uses, so the
            // glyph stays legible against either half of the swap.
            let left_items: Vec<ListItem> = visible_cats[scroll_start..scroll_end]
                .iter()
                .map(|&i| {
                    let cat = LIB_CATEGORIES[i];
                    let icon = lib_icons.get(i).copied().unwrap_or(" ");
                    let count = app.library_count(cat);
                    let text = if count > 0 {
                        format!("{:<14} {:>4}", cat, count)
                    } else {
                        cat.to_string()
                    };
                    let is_active = i == app.library_category;
                    let picked = is_active && left_focus;
                    let style = if picked {
                        Style::default()
                            .fg(app.theme.selection_fg_readable())
                            .bg(app.theme.selection_bg)
                    } else if is_active {
                        Style::default().fg(app.theme.accent)
                    } else {
                        Style::default().fg(app.theme.fg)
                    };
                    let glyph = if picked {
                        Style::default()
                            .fg(app.theme.selection_bg)
                            .bg(app.theme.selection_fg_readable())
                    } else {
                        style
                    };
                    ListItem::new(Line::from(vec![
                        Span::styled(" ", style),
                        Span::styled(icon, glyph),
                        Span::styled(format!("  {text}"), style),
                    ]))
                    .style(style)
                })
                .collect();
            // The list gives the scrollbar its own column rather than letting
            // it draw over the count on the right of each row.
            let (list_area, bar_area) = if total > list_rows as usize {
                let bar = Rect {
                    x: left_list_area.x + left_list_area.width - 1,
                    width: 1,
                    ..left_list_area
                };
                (
                    Rect {
                        width: left_list_area.width - 1,
                        ..left_list_area
                    },
                    Some(bar),
                )
            } else {
                (left_list_area, None)
            };
            f.render_widget(List::new(left_items), list_area);

            if let Some(area) = bar_area {
                let mut sb = ratatui::widgets::ScrollbarState::new(total)
                    .position(scroll_start)
                    .viewport_content_length(list_rows as usize);
                f.render_stateful_widget(
                    ratatui::widgets::Scrollbar::new(
                        ratatui::widgets::ScrollbarOrientation::VerticalRight,
                    ),
                    area,
                    &mut sb,
                );
            }

            // No indicator block on the active row.
            //
            // There was a left-quarter block drawn over the first column in
            // `sidebar_active_border`, a third colour against a row that was
            // already painted with the selection background. On the focused row
            // it was drawn on top of the highlight, so it read as a stray block
            // inside the selection rather than as a marker — the artefact. It
            // was also redundant in both states: focused, the background already
            // says which row this is; unfocused, the active row is already in
            // the accent colour. The leading space the glyph occupied stays, so
            // the icon keeps its column and the rows do not shift.
            let _ = left_pad_area;
        }

        let category_label = LIB_CATEGORIES
            .get(app.library_category)
            .copied()
            .unwrap_or("");
        // The Library view has four groupings behind one row, so its header
        // names the one on screen: a pane headed "Library" over a list of
        // albums says nothing about which list the user is looking at.
        let category_label: &str = if app.library_category == LIB_ALL {
            app.library_filter.label()
        } else {
            category_label
        };

        // Total rows in the active right-pane list, threaded out of the category
        // branches so mouse hit zones only cover real rows.
        let mut lib_total_rows: usize = 0;

        // On narrow/medium screens the lyrics take over the results pane, so
        // skip rendering the list underneath and registering hit zones for rows
        // that are not visible.
        //
        // Only while the lyrics hold focus. This condition never consulted
        // focus, so with `l` on the list was gone for good: under 100 columns
        // the lyrics were drawn over the pane permanently and Tab moved a
        // one-column focus bar between two panes that both stayed put. On
        // narrow it was worse than that — the list renders into `results_area`,
        // which is `Length(0)` whenever the left pane has focus, so it was
        // being drawn into a zero-width rect and was not on screen at all.
        // Gating on `pane_focus` makes the existing `cycle_library_focus` states
        // swap the two views instead of just moving the highlight.
        let lyrics_results_pane = app.lyrics.show && lyrics_area.is_none() && app.lyrics.pane_focus;

        // With one pane the track card is docked as a bottom pane, and the list
        // is shortened to sit above it. It used to float over the list instead:
        // a card anchored to the bottom-right corner of a one-column terminal is
        // narrower than the pane but taller than the space it left, so it
        // covered rows that were neither reachable nor clickable, and the
        // height it reserved was a second, disagreeing estimate of its own size.
        //
        // Decided before the list is built because the list is sized around it.
        let dock_card =
            is_narrow && !lyrics_results_pane && app.show_preview && app.track_popup_visible;
        let dock_h = if dock_card {
            Render::dock_size(results_area).1
        } else {
            0
        };
        // Every category branch sizes its window the same way: the pane less the
        // leading blank and the stats row, less the docked card when it is on
        // screen. Not named for the library column's own `list_rows` above.
        let window_rows = || results_area.height.saturating_sub(3 + dock_h) as usize;

        let (right_lines, _stats_line) = if app.grid_active() {
            // Drawn as cells below, not as rows: the renderer needs the pane, so
            // it runs after the header. Two consequences of being empty:
            // `lib_total_rows` stays zero, so the row hit zones are not
            // registered over the grid's own, and the stats line below — which
            // is computed from the category, not from here — keeps counting
            // albums while there are no rows to count.
            (Vec::new(), String::new())
        } else if app.browse_detail.is_some() && app.library_category == LIB_SPOTIFY {
            let tracks = &app.spotify.playlist_tracks_cache;
            let total_len = app.spotify_playlist_rows();
            let st_line = library_stats_line(app);
            let available = window_rows();
            app.viewport_items = available;
            let sel = app.list_pos().min(total_len.saturating_sub(1));

            let pane_w = results_area.width as usize;
            let mut lines = vec![Line::from("")];
            const ACTION_ROWS: usize = App::SPOTIFY_PLAYLIST_ROWS;
            // The spacer line and the two action rows are always emitted, so the
            // track window gets whatever is left. Sizing it from `available`
            // instead drew two rows more than the pane could hold, which pushed
            // the bottom of the list off screen and — because the window then
            // shrank as `end` clamped to the last row — made the final track
            // impossible to scroll into view.
            let budget = available.saturating_sub(1 + ACTION_ROWS).max(1);
            // The scroll is kept in row space like the selection, but the
            // viewport is stepped in track space, where the two action rows do
            // not shift the arithmetic.
            let sel_track = sel.saturating_sub(ACTION_ROWS);
            let (start, stop) = step_viewport(
                app.list_scroll.saturating_sub(ACTION_ROWS),
                sel_track,
                budget,
                tracks.len(),
            );
            app.list_scroll = start + ACTION_ROWS;

            // Rows 0/1: virtual actions (Play All / Shuffle), then the tracks.
            let action_help = [("▶  Play All", "  Enter"), ("🔀  Shuffle", "  Enter / S")];
            for (ai, (action, key_hint)) in action_help.iter().enumerate() {
                let real_i = ai;
                let is_sel = real_i == sel && !left_focus;
                // `fg`, not `fg_bright`: this is the only list in the pane whose
                // unselected rows used the brighter accent, so All Tracks read as
                // a different kind of row from every category beside it. The
                // Spotify list sets no colour at all on an unselected row and
                // lets the pane's own foreground through, which is the same
                // value `fg` names.
                let style = if is_sel {
                    Style::default()
                        .fg(app.theme.selection_fg_readable())
                        .bg(app.theme.selection_bg)
                } else {
                    Style::default().fg(app.theme.fg)
                };
                let prefix = "   ";
                let content = format!("{prefix}{action}");
                let pad = row_pad(&content, results_area.width);
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
                    let prefix = "   ";
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
                    let pad = row_pad(&format!("{head}{tail}"), results_area.width);
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

            let available = window_rows();
            app.viewport_items = available;
            let sel = app.list_pos().min(total_len.saturating_sub(1));
            let (list_scroll, end) = step_viewport(app.list_scroll, sel, available, total_len);
            app.list_scroll = list_scroll;

            let filtered = app.filtered_tracks();

            let pane_w = results_area.width as usize;
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
                    let is_sel = real_i == sel && !left_focus;
                    let is_multiselected = app.multiselect_mode && app.row_is_selected(&track.path);
                    let label = track.title.clone();
                    let avail = pane_w.saturating_sub(2);
                    let display_label = scroll_text(&label, avail, app.footer_title_scroll, is_sel);
                    let checkbox = if is_multiselected { "☑ " } else { "" };
                    let row = format!("{}{}{}", "   ", checkbox, display_label);
                    let style = if is_sel {
                        Style::default()
                            .fg(app.theme.selection_fg_readable())
                            .bg(app.theme.selection_bg)
                    } else if is_multiselected {
                        Style::default().bg(app.theme.warning).fg(app.theme.bg)
                    } else {
                        Style::default().fg(app.theme.fg)
                    };
                    let row = if is_sel {
                        let pad = row_pad(&row, results_area.width);
                        format!("{row}{}", " ".repeat(pad))
                    } else {
                        row
                    };
                    lines.push(Line::from(Span::styled(row, style)));
                }
                lib_total_rows = total_len;
                (lines, st_line)
            }
        } else if app.library_category == LIB_ALL
            && !matches!(app.library_filter, LibraryFilter::Tracks)
        {
            // One arm for the three grouped lists. They differ only in the row
            // set and the noun under the count, both of which the filter names,
            // and they render identically otherwise.
            let groups = app.library_groups();
            let total_len = groups.len();
            let sel = app.list_pos().min(total_len.saturating_sub(1));
            let st_line = format!(
                " {} {} ",
                total_len,
                plural(
                    total_len,
                    app.library_filter.one(),
                    app.library_filter.many()
                )
            );
            let available = window_rows();
            app.viewport_items = available;
            let (list_scroll, end) = step_viewport(app.list_scroll, sel, available, total_len);
            app.list_scroll = list_scroll;
            let mut lines = vec![Line::from("")];
            if groups.is_empty() {
                lines.extend(empty_hint_lines(
                    app,
                    &format!("No {} yet", app.library_filter.many()),
                    "Hint: import tagged audio files, then browse them here",
                ));
            }
            for (i, (name, _count)) in groups[app.list_scroll..end].iter().enumerate() {
                let real_i = app.list_scroll + i;
                let is_sel = real_i == sel && !left_focus;
                let style = if is_sel {
                    Style::default()
                        .fg(app.theme.selection_fg_readable())
                        .bg(app.theme.selection_bg)
                } else {
                    Style::default().fg(app.theme.fg)
                };
                let row = format!("   {name}");
                let row = if is_sel {
                    let pad = row_pad(&row, results_area.width);
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
        } else if app.library_category == LIB_PLAYLISTS {
            let playlists = &app.playlist_cache;
            let total_len = playlists.len();
            let sel = app.list_pos().min(total_len.saturating_sub(1));
            let st_line = format!(
                " {} {} ",
                total_len,
                plural(total_len, "playlist", "playlists")
            );
            let available = window_rows();
            app.viewport_items = available;
            let (list_scroll, end) = step_viewport(app.list_scroll, sel, available, total_len);
            app.list_scroll = list_scroll;
            let mut lines = vec![Line::from("")];
            for (i, pl) in playlists[app.list_scroll..end].iter().enumerate() {
                let real_i = app.list_scroll + i;
                let is_sel = real_i == sel && !left_focus;
                let prefix = "   ";
                let style = if is_sel {
                    Style::default()
                        .fg(app.theme.selection_fg_readable())
                        .bg(app.theme.selection_bg)
                } else {
                    Style::default().fg(app.theme.fg)
                };
                let row = format!("{}{}", prefix, pl.name);
                let row = if is_sel {
                    let pad = row_pad(&row, results_area.width);
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
        } else if app.library_category == LIB_SPOTIFY {
            let playlists = &app.spotify.playlists;
            let total_len = playlists.len();
            let sel = app.list_pos().min(total_len.saturating_sub(1));
            let st_line = format!(
                " {} {} ",
                total_len,
                plural(total_len, "playlist", "playlists")
            );
            let available = window_rows();
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
                    let prefix = "   ";
                    let style = if is_sel {
                        Style::default()
                            .fg(app.theme.selection_fg_readable())
                            .bg(app.theme.selection_bg)
                    } else {
                        Style::default().fg(app.theme.fg)
                    };
                    let row = format!("{}{}", prefix, pl.name);
                    let row = if is_sel {
                        let pad = row_pad(&row, results_area.width);
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
        } else if app.library_category == LIB_RADIO {
            let stations = &app.radio.custom;
            let total_len = stations.len();
            let sel = app.list_pos().min(total_len.saturating_sub(1));
            let st_line = format!(
                " {} {} ",
                total_len,
                plural(total_len, "station", "stations")
            );
            let available = window_rows();
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
                    let prefix = "   ";
                    let style = if is_sel {
                        Style::default()
                            .fg(app.theme.selection_fg_readable())
                            .bg(app.theme.selection_bg)
                    } else {
                        Style::default().fg(app.theme.fg)
                    };
                    let row = format!("{}{}", prefix, s.name);
                    let row = if is_sel {
                        let pad = row_pad(&row, results_area.width);
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
        } else if app.library_category == LIB_FOLDERS {
            let folders = app.unique_folders();
            let total_len = folders.len();
            let sel = app.list_pos().min(total_len.saturating_sub(1));
            let st_line = format!(" {} {} ", total_len, plural(total_len, "folder", "folders"));
            let available = window_rows();
            app.viewport_items = available;
            let (list_scroll, end) = step_viewport(app.list_scroll, sel, available, total_len);
            app.list_scroll = list_scroll;
            let mut lines = vec![Line::from("")];
            for (i, (dir, _count)) in folders[app.list_scroll..end].iter().enumerate() {
                let real_i = app.list_scroll + i;
                let is_sel = real_i == sel && !left_focus;
                let prefix = "   ";
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
                    let pad = row_pad(&row, results_area.width);
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
        } else if app.library_category == LIB_CHARTS {
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
                let available = window_rows();
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
                        let prefix = "   ";
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
                            let pad = row_pad(&row, results_area.width);
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
                let available = window_rows();
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
                        let prefix = "   ";
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
                            let pad = row_pad(&row, results_area.width);
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
                let available = window_rows();
                app.viewport_items = available;
                let (list_scroll, end) = step_viewport(app.list_scroll, sel, available, total_len);
                app.list_scroll = list_scroll;
                let pane_w = results_area.width as usize;
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
                        let prefix = "   ";
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
                            let pad = row_pad(&row, results_area.width);
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
        } else if app.library_category == LIB_PODCASTS {
            // Podcasts: level 0 lists the subscribed feeds, level 1 the
            // episodes of the feed drilled into. Mirrors the chart's two-level
            // shape so Backspace/Enter behave the same in both.
            if app.podcast.episodes_feed_id.is_some() {
                let episodes = &app.podcast.episodes;
                let total_len = episodes.len();
                let sel = app.list_pos().min(total_len.saturating_sub(1));
                let st_line = format!(
                    " {} {} ",
                    total_len,
                    plural(total_len, "episode", "episodes")
                );
                let available = window_rows();
                app.viewport_items = available;
                let (list_scroll, end) = step_viewport(app.list_scroll, sel, available, total_len);
                app.list_scroll = list_scroll;
                let mut lines = vec![Line::from("")];
                if total_len == 0 {
                    lines.extend(empty_hint_lines(
                        app,
                        "No episodes in this feed",
                        "Hint: press Backspace to go back",
                    ));
                } else {
                    for (i, ep) in episodes[list_scroll..end].iter().enumerate() {
                        let real_i = list_scroll + i;
                        let is_sel = real_i == sel && !left_focus;
                        let style = if is_sel {
                            Style::default()
                                .fg(app.theme.selection_fg_readable())
                                .bg(app.theme.selection_bg)
                        } else {
                            Style::default().fg(app.theme.fg)
                        };
                        let prefix = "   ";
                        let dur = ep
                            .duration_secs
                            .map(|s| format!("  [{}:{:02}]", s / 60, s % 60))
                            .unwrap_or_default();
                        let row = format!("{}{}{}", prefix, ep.title, dur);
                        let row = if is_sel {
                            let pad = row_pad(&row, results_area.width);
                            format!("{row}{}", " ".repeat(pad))
                        } else {
                            row
                        };
                        lines.push(Line::from(Span::styled(row, style)));
                    }
                }
                lib_total_rows = total_len;
                (lines, st_line)
            } else {
                let feeds = &app.podcast.feeds;
                let total_len = feeds.len();
                let sel = app.list_pos().min(total_len.saturating_sub(1));
                let st_line = format!(" {} {} ", total_len, plural(total_len, "feed", "feeds"));
                let available = window_rows();
                app.viewport_items = available;
                let (list_scroll, end) = step_viewport(app.list_scroll, sel, available, total_len);
                app.list_scroll = list_scroll;
                let mut lines = vec![Line::from("")];
                if total_len == 0 {
                    lines.extend(empty_hint_lines(
                        app,
                        "No podcast feeds",
                        "Hint: press a to add a feed by URL",
                    ));
                } else {
                    for (i, feed) in feeds[list_scroll..end].iter().enumerate() {
                        let real_i = list_scroll + i;
                        let is_sel = real_i == sel && !left_focus;
                        let style = if is_sel {
                            Style::default()
                                .fg(app.theme.selection_fg_readable())
                                .bg(app.theme.selection_bg)
                        } else {
                            Style::default().fg(app.theme.fg)
                        };
                        let prefix = "   ";
                        let count = if feed.episodes > 0 {
                            format!("  [{}]", feed.episodes)
                        } else {
                            String::new()
                        };
                        let row = format!("{}{}{}", prefix, feed.title, count);
                        let row = if is_sel {
                            let pad = row_pad(&row, results_area.width);
                            format!("{row}{}", " ".repeat(pad))
                        } else {
                            row
                        };
                        lines.push(Line::from(Span::styled(row, style)));
                    }
                }
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

            let available = window_rows();
            app.viewport_items = available;
            let sel = app.list_pos().min(total_len.saturating_sub(1));
            let (list_scroll, end) = step_viewport(app.list_scroll, sel, available, total_len);
            app.list_scroll = list_scroll;

            let filtered = app.filtered_tracks();
            let pane_w = results_area.width as usize;

            let mut lines = vec![Line::from("")];
            if filtered.is_empty() {
                let (headline, hint) = match app.library_category {
                    LIB_PLAYED => (
                        "No most-played tracks yet",
                        "Hint: play counts build up as you listen",
                    ),
                    LIB_RECENT => (
                        "Nothing played recently",
                        "Hint: play any track and it will show up here",
                    ),
                    LIB_ADDED => (
                        "No recent additions",
                        "Hint: add music to your library to see it here",
                    ),
                    _ => (
                        "Nothing in the library yet",
                        "Hint: add music to your library, or link a Spotify account",
                    ),
                };
                lines.extend(empty_hint_lines(app, headline, hint));
            } else {
                for (i, track) in filtered[app.list_scroll..end].iter().enumerate() {
                    let real_i = app.list_scroll + i;
                    let is_sel = real_i == sel && !left_focus;
                    let is_multiselected = app.multiselect_mode && app.row_is_selected(&track.path);
                    let label = track.title.clone();
                    let avail = pane_w.saturating_sub(2);
                    let display_label = scroll_text(&label, avail, app.footer_title_scroll, is_sel);
                    let checkbox = if is_multiselected { "☑ " } else { "" };
                    let row = format!("{}{}{}", "   ", checkbox, display_label);
                    let style = if is_sel {
                        Style::default()
                            .fg(app.theme.selection_fg_readable())
                            .bg(app.theme.selection_bg)
                    } else if is_multiselected {
                        Style::default().bg(app.theme.warning).fg(app.theme.bg)
                    } else {
                        Style::default().fg(app.theme.fg)
                    };
                    let row = if is_sel {
                        let pad = row_pad(&row, results_area.width);
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

        // On narrow/medium screens the lyrics take over the results pane, so
        // skip rendering the list underneath and registering hit zones for rows
        // that are not visible.
        //
        // Only while the lyrics hold focus. This condition never consulted
        // focus, so with `l` on the list was gone for good: under 100 columns
        // the lyrics were drawn over the pane permanently and Tab moved a
        // one-column focus bar between two panes that both stayed put. On
        // narrow it was worse than that — the list renders into `results_area`,
        // which is `Length(0)` whenever the left pane has focus, so it was
        // being drawn into a zero-width rect and was not on screen at all.
        // Gating on `pane_focus` makes the existing `cycle_library_focus` states
        // swap the two views instead of just moving the highlight.
        if (want_playlist_card || want_row_card)
            && left_info_area.height > 0
            && (info_sep_area.height > 0 || left_info_area.height > 0)
        {
            Render::spot_card(f, left_info_area, app);
        } else if app.show_preview
            && app.track_popup_visible
            && left_info_area.height >= info_block_h()
            && (info_sep_area.height > 0 || left_info_area.height > 0)
        {
            // Narrow + lyrics: the results pane is given over to the lyrics, so
            // the block is repurposed to show the currently-highlighted list
            // contents (the selected row and its neighbours) instead of the
            // now-playing track card. Only while the lyrics are actually on
            // screen there; when the left pane holds focus the pane is the
            // library again and the list preview is the sensible thing to keep.
            if is_narrow && lyrics_results_pane {
                Render::list_in_info(f, left_info_area, app);
            } else if !is_narrow {
                Render::info_in_pane(f, info_sep_area, left_info_area, app);
            }
        }

        if !lyrics_results_pane {
            let right_para = Paragraph::new(right_lines);
            // The detail is the title for every category except Spotify, where
            // it is a playlist id.
            let header_label = match (app.browse_detail.as_deref(), app.browse_title.as_deref()) {
                (Some(_), Some(title)) => format!("▶ {title}"),
                (Some(detail), None) => format!("▶ {detail}"),
                (None, _) => category_label.to_string(),
            };
            let right_inner = Render::pane_header(
                f,
                results_area,
                app,
                &header_label,
                !left_focus,
                false,
                true,
            );
            fill_pane(f, right_inner, app);
            if app.grid_active() {
                // The grid draws its own cells and registers its own hit zones,
                // so the row list is neither drawn nor counted: `lib_total_rows`
                // is left at zero above precisely so the row hit zones below are
                // not registered over the top of them.
                Render::grid(f, right_inner, app);
            } else {
                Render::evolving(f, right_inner, right_para, "lib", app, false);
            }

            // Mouse hit zones for the visible library rows: rows start
            // below one leading blank line.
            //
            // The budget subtracts the docked card as well. It used to stop at
            // the pane's own rows, so every hit zone below the card's top edge
            // belonged to a row the card was covering — the list could be
            // scrolled to a row and then clicked only where the card was not.
            if lib_total_rows > 0 {
                let avail = right_inner.height.saturating_sub(2).saturating_sub(dock_h) as usize;
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
        } else if lyrics_results_pane {
            // Medium-width screens (60-99 cols): show lyrics in the results pane
            // instead of a separate third pane.
            //
            // The fallback is the narrow case: the results column is collapsed
            // to nothing while the library holds the cursor, and both the `l`
            // key and the palette action set the lyrics focus without
            // releasing it, so the lyrics would land in a zero-width rect and
            // be on screen nowhere.
            let base = if results_area.width > 1 {
                results_area
            } else {
                lib_area
            };
            let lyrics = Rect {
                height: base.height.saturating_sub(1),
                ..base
            };
            Render::lyrics_pane(f, lyrics, app);
        }

        if dock_card {
            Render::docked_card(f, results_area, app);
        }
    }

    /// Height of the bottom-pane card on a one-column layout.
    ///
    /// Derived from the same artwork the card draws, so the box and its contents
    /// cannot disagree the way the old float's two independent estimates did.
    /// Capped at a quarter of the pane: the card describes the list above it, so
    /// it may not take the rows it is describing. The floor of 2 rows for the art
    /// is what keeps a release recognisable.
    pub(crate) fn dock_size(area: Rect) -> (u16, u16) {
        let art = area
            .height
            .saturating_sub(INFO_FIELDS_H + 4)
            .min(DOCK_ART_H)
            .min(area.height / 4)
            .max(2);
        (
            area.width,
            (art + INFO_FIELDS_H + 1).min(area.height.saturating_sub(1)),
        )
    }

    /// The track-info card docked under the list on narrow screens.
    ///
    /// Docked rather than floating: on one column there is no second pane to
    /// hold it, and a float anchored to the bottom-right corner covered rows of
    /// the list underneath it that could be neither read nor clicked. Docking
    /// spends rows the list is sized around (`window_rows`) instead, which is
    /// the same arithmetic the library column's own card already used.
    pub(crate) fn docked_card(f: &mut ratatui::Frame, area: Rect, app: &mut App) {
        let (_, h) = Render::dock_size(area);
        if area.width < 8 || h < INFO_FIELDS_H + 2 {
            return;
        }
        let rect = Rect {
            x: area.x,
            y: area.y + area.height.saturating_sub(h),
            width: area.width,
            height: h,
        };
        fill_pane(f, rect, app);
        Render::info_in_pane(f, Rect::new(0, 0, 0, 0), rect, app);
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

    pub(crate) fn lyrics_pane(f: &mut ratatui::Frame, area: Rect, app: &mut App) {
        // No heading, and no left rule, in any layout. "LYRICS" named a panel
        // whose neighbours already say what is playing, and the rule drew a
        // border that had nothing to separate — the lyrics were either the
        // third pane in a row of three or the only thing on screen, and in
        // neither case was there anything for a divider to divide.
        let inner = area;
        fill_pane(f, inner, app);

        // Focus, without a header. Every other pane says which one has it with
        // an accent bar on its header row; this one has no header row, so the
        // bar runs the full height of the pane's edge instead and the body
        // gives up the column so nothing is drawn under it.
        let inner = if app.lyrics.pane_focus && inner.width > 1 {
            f.render_widget(
                Block::default()
                    .borders(Borders::LEFT)
                    .border_style(Style::default().fg(app.theme.accent)),
                Rect { width: 1, ..inner },
            );
            Rect {
                x: inner.x + 1,
                width: inner.width - 1,
                ..inner
            }
        } else {
            inner
        };

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

        // The whole pane is lyrics: no title, no artist, no artwork. Every
        // layout already names the track beside these lines — the now-playing
        // band, or the third pane's neighbour — and the artwork was the one
        // thing repeated twice: the same bytes, decoded and uploaded a second
        // time, for a pane whose entire job is to be read.
        let lyrics_inner = Rect {
            x: inner.x,
            y: inner.y,
            width: inner.width,
            height: inner.height,
        };

        Render::lyrics_body(f, lyrics_inner, app, lyrics, LyricSurface::Pane);
    }

    /// Body of the lyrics pane: wrap the lyric lines, emphasize the active
    /// line (with karaoke word timing when available) and scroll to the
    /// anchor. Shared by the normal lyrics pane and the Zen-mode fullscreen
    /// lyrics surface.
    ///
    /// The surfaces differ only in what they sit on. The pane's background is
    /// the app surface, and the theme's own `fg`/`fg_dim` are picked against
    /// it. The Zen surface is a wash of the reactive palette, whose luminance
    /// is whatever the artwork happened to be: a pale cover washed the lines to
    /// near-invisible while a dark one left them fine, and neither is a theme
    /// problem to fix. So the Zen surface re-derives its foregrounds from its own
    /// background rather than inheriting them.
    pub(crate) fn lyrics_body(
        f: &mut ratatui::Frame,
        lyrics_inner: Rect,
        app: &App,
        lyrics: &LrcData,
        surface: LyricSurface,
    ) {
        // Read once: the Zen background is a blend, and blending it per line
        // would paint the same two values sixteen times.
        let bg = match surface {
            LyricSurface::Pane => app.surface_bg(),
            LyricSurface::Zen => app.zen_bg(),
        };
        let lit = match surface {
            LyricSurface::Pane => app.theme.accent,
            LyricSurface::Zen => readable_fg(app.theme.accent, bg),
        };
        let past = match surface {
            LyricSurface::Pane => app.theme.fg,
            LyricSurface::Zen => readable_fg(app.theme.fg, bg),
        };
        let ahead = match surface {
            LyricSurface::Pane => app.theme.fg_dim,
            LyricSurface::Zen => readable_fg(app.theme.fg_dim, bg),
        };
        let total = lyrics.lines.len();
        let width = lyrics_inner.width.max(1) as usize;
        let synced = lyrics_are_synced(&lyrics.lines);
        let anchor = app.lyrics.scroll.min(total.saturating_sub(1));
        let mut row_offsets = Vec::with_capacity(total);
        let mut text = Vec::with_capacity(total);
        let mut cumulative = 0usize;
        for (i, line) in lyrics.lines.iter().enumerate() {
            let text_style = if !synced {
                Style::default().fg(past)
            } else {
                // Past lines stay readable, the active (current) line matching
                // the playback timestamp is emphasized, future lines fade out.
                let d = i as isize - anchor as isize;
                if d == 0 {
                    Style::default().fg(lit).add_modifier(Modifier::BOLD)
                } else if d < 0 {
                    Style::default().fg(past)
                } else {
                    // The pane dims what is ahead of the playhead; Zen does not.
                    // `DIM` halves whatever the terminal already resolved, and a
                    // foreground chosen for contrast still lands under the bar
                    // once it is halved — which is how the future lines came to
                    // be the unreadable ones rather than the faint ones.
                    let s = Style::default().fg(ahead);
                    match surface {
                        LyricSurface::Pane => s.add_modifier(Modifier::DIM),
                        LyricSurface::Zen => s,
                    }
                }
            };
            // No timestamp gutter. Every synced line carried a `[0:19-0:24]`
            // range, which cost two to nine columns on *every* row and told the
            // reader nothing they had not just watched the highlight move
            // through. The line being sung is already marked; the timing is in
            // the source.
            // Karaoke: the active line lights up word-by-word when the source
            // carries per-word timings (enhanced LRC). Words not yet sung sit
            // at the future-lines colour.
            let rendered = if i == anchor && synced && !line.words.is_empty() {
                let pos = app.raw_position;
                let mut spans: Vec<Span> = Vec::with_capacity(line.words.len() + 1);
                for w in &line.words {
                    let sung = pos >= w.time;
                    spans.push(Span::styled(
                        w.text.clone(),
                        if sung {
                            Style::default().fg(lit).add_modifier(Modifier::BOLD)
                        } else {
                            Style::default().fg(ahead)
                        },
                    ));
                }
                Line::from(spans)
            } else {
                Line::from(Span::styled(line.text.clone(), text_style))
            };
            // Measured from the line that is actually rendered, by ratatui's own
            // wrapper. A hand estimate (`chars().div_ceil(width)`) drifts from
            // it — display width is not character count, and a word longer than
            // the pane wraps differently than a `div_ceil` assumes — and every
            // drifted row scrolls the highlight off the line it belongs to.
            row_offsets.push(cumulative);
            cumulative += Paragraph::new(rendered.clone())
                .wrap(Wrap { trim: false })
                .line_count(width as u16);
            text.push(rendered);
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
                // Near the top: cur - 2
                row_offsets[anchor].saturating_sub(2).min(bottom)
            }
        } else {
            row_offsets[anchor].saturating_sub(2).min(bottom)
        };

        // Centred rather than left-aligned. Lyrics are read as a block, and a
        // ragged left edge makes a chorus look like a list; centring also means
        // the active line sits over the one it replaces instead of snapping to
        // a margin as the line length changes.
        //
        // Set here rather than at the call sites because `lyrics_body` is the
        // one renderer both the zen lyrics view and the single-column lyrics
        // pane go through -- aligning per call site would have left the two
        // disagreeing about the same text.
        let para = Paragraph::new(text)
            .wrap(Wrap { trim: false })
            .alignment(Alignment::Center)
            .scroll((scroll_display as u16, 0));
        f.render_widget(para, scroll_view);
        if let Some(h) = hint_area {
            f.render_widget(
                Paragraph::new(Line::from(Span::styled(
                    "not time synced",
                    Style::default().fg(ahead),
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
                let prefix = "   ";
                let label = track.display_title();
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

    /// The left pane's info slot while browsing Spotify: the highlighted
    /// playlist's cover when the playlist list is open, or the highlighted
    /// track's when the drill-down is. One renderer for both so the slot has a
    /// single shape.
    ///
    /// The cover is never drawn inline with the list rows — the list panes stay
    /// pure text, and the row budget keeps its whole height.
    pub(crate) fn spot_card(f: &mut ratatui::Frame, area: Rect, app: &mut App) {
        if app.in_spotify_playlist() {
            return Self::spot_row(f, area, app);
        }
        Self::playlist_in_pane(f, area, app);
    }

    /// The highlighted playlist's name, owner and track count under its cover.
    fn spot_row(f: &mut ratatui::Frame, area: Rect, app: &mut App) {
        // Copied out before the mutable borrow of the decoder state below, so
        // the text does not have to be read across it.
        let Some(track) = app.selected_spotify_track().cloned() else {
            return;
        };
        let text_need = 2u16;
        let cover_h = COVER_H.min(area.height.saturating_sub(text_need).saturating_sub(1));
        let cover_w = (cover_h * 2).min(area.width.saturating_sub(2));
        let mut y = area.y;
        if cover_h > 0 && cover_w > 0 {
            let cover_area = Rect {
                x: area.x + area.width.saturating_sub(cover_w) / 2,
                y,
                width: cover_w,
                height: cover_h,
            };
            Render::cover(
                f,
                cover_area,
                app.spotify.row_cover_stateful.as_mut(),
                app.spotify.row_cover.as_deref(),
                app.theme.fg_dim,
                None,
            );
            y += cover_h;
        }
        let clip = |t: &str| t.chars().take(area.width as usize).collect::<String>();
        for (text, style) in [
            (clip(&track.name), Style::default().fg(app.theme.fg_bright)),
            (clip(&track.artists), Style::default().fg(app.theme.fg_dim)),
        ] {
            if y >= area.y + area.height {
                break;
            }
            f.render_widget(
                Paragraph::new(Line::from(Span::styled(text, style))).alignment(Alignment::Center),
                Rect {
                    x: area.x,
                    y,
                    width: area.width,
                    height: 1,
                },
            );
            y += 1;
        }
    }

    /// The highlighted Spotify playlist, in the left pane's info slot: its cover
    /// above its name, owner and track count.
    ///
    /// Shares the slot with the track card rather than taking a new one, so it
    /// only renders when there is art to show — an empty slot would push the
    /// category list up for nothing.
    pub(crate) fn playlist_in_pane(f: &mut ratatui::Frame, area: Rect, app: &mut App) {
        let Some(playlist) = app.highlighted_playlist() else {
            return;
        };
        let text_need = 3u16;
        let cover_h_avail = area.height.saturating_sub(text_need).saturating_sub(1);
        let cover_h = COVER_H.min(cover_h_avail);
        let cover_w = (cover_h * 2).min(area.width.saturating_sub(2));
        let mut y = area.y;
        if cover_h > 0 && cover_w > 0 {
            let cover_area = Rect {
                x: area.x + area.width.saturating_sub(cover_w) / 2,
                y,
                width: cover_w,
                height: cover_h,
            };
            Render::cover(
                f,
                cover_area,
                app.spotify.list_cover_stateful.as_mut(),
                app.spotify.list_cover.as_deref(),
                app.theme.fg_dim,
                None,
            );
            y += cover_h;
        }
        // The pane is one line per field, so clip on width rather than let the
        // paragraph wrap and push the owner line off the bottom.
        let clip = |t: &str| t.chars().take(area.width as usize).collect::<String>();
        let name = clip(&playlist.name);
        let owner = clip(&playlist.owner);
        let count = format!(
            " {} {} ",
            playlist.track_count(),
            plural(playlist.track_count(), "track", "tracks")
        );
        for line in [
            Line::from(Span::styled(name, Style::default().fg(app.theme.fg_bright))),
            Line::from(Span::styled(count, Style::default().fg(app.theme.fg_dim))),
            Line::from(Span::styled(owner, Style::default().fg(app.theme.fg_dim))),
        ] {
            if y >= area.y + area.height {
                break;
            }
            f.render_widget(
                Paragraph::new(line).alignment(Alignment::Center),
                Rect {
                    x: area.x,
                    y,
                    width: area.width,
                    height: 1,
                },
            );
            y += 1;
        }
    }

    /// The album, artist and genre lists as a grid of covers.
    ///
    /// A mode of the same list rather than a second view: the cursor, the
    /// drill-down, the mouse zones, the selection and the stats line are the
    /// ones the row view already has, and only the drawing differs. Rows of
    /// names are how a thousand albums are *read*; a grid of sleeves is how
    /// they are *recognised*, and there was no second screen to recognise them
    /// on.
    ///
    /// Covers are resolved through the same representative track the info card
    /// uses, so a cell and the card below it show the same artwork.
    pub(crate) fn grid(f: &mut ratatui::Frame, area: Rect, app: &mut App) {
        // A pane too small for one cell draws nothing and fetches nothing. On a
        // narrow layout the results column is `Length(0)` while the left pane
        // holds the cursor, so this is the common case, not a corner one.
        if area.width < GRID_CELL_W || area.height < GRID_CELL_H {
            return;
        }
        let labels = app.grid_labels();
        let total = labels.len();
        if total == 0 {
            f.render_widget(
                Paragraph::new(empty_hint_lines(
                    app,
                    "Nothing to show here yet",
                    "Hint: import tagged audio files, then browse them here",
                )),
                area,
            );
            return;
        }
        let plan = app.grid_plan(area, total);
        // Before the cells are read: this is what fills `grid.ids` with the
        // window's representative tracks and queues the missing covers.
        app.grid_fetch(&plan);
        let sel = app.list_pos().min(total - 1);
        let cols = plan.cols;
        let cell_h = GRID_CELL_H;
        let cover_w = GRID_CELL_W - 1;
        let cover_h = GRID_CELL_H - 2;

        for (cell, id) in app.grid.ids.iter().enumerate() {
            let pos = plan.first + cell;
            let Some(name) = labels.get(pos) else {
                continue;
            };
            let x = area.x + (cell % cols) as u16 * GRID_CELL_W;
            let y = area.y + (cell / cols) as u16 * cell_h;
            if y + cell_h > area.y + area.height {
                break;
            }
            let is_sel = pos == sel && !app.library_pane_focus;
            let cell_area = Rect {
                x,
                y,
                width: cover_w,
                height: cell_h,
            };
            // Selection goes under the artwork rather than over it: painting a
            // highlight rect on top of an image protocol's cells blanks the
            // image. Drawn with a `Block` and not a `Paragraph` — a paragraph
            // with no lines styles nothing at all, which would have left the
            // selected cell's only highlight invisible.
            if is_sel {
                f.render_widget(
                    Block::default().style(
                        Style::default()
                            .fg(app.theme.selection_fg_readable())
                            .bg(app.theme.selection_bg),
                    ),
                    cell_area,
                );
            }
            let cover_area = Rect {
                x,
                y,
                width: cover_w,
                height: cover_h,
            };
            match id.and_then(|id| app.grid.covers.get_mut(&id)) {
                Some(cell_art) => Render::cover(
                    f,
                    cover_area,
                    cell_art.proto.as_mut(),
                    Some(&cell_art.bytes),
                    app.theme.fg_dim,
                    None,
                ),
                None => Render::cover(
                    f,
                    cover_area,
                    None,
                    None,
                    app.theme.fg_dim,
                    Some(" \u{25a1} "),
                ),
            }
            let label_area = Rect {
                x,
                y: y + cover_h,
                width: cover_w,
                height: 1,
            };
            let style = if is_sel {
                Style::default()
                    .fg(app.theme.selection_fg_readable())
                    .bg(app.theme.selection_bg)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(app.theme.fg)
            };
            let label = scroll_text(name, cover_w as usize, app.footer_title_scroll, is_sel);
            f.render_widget(
                Paragraph::new(Line::from(Span::styled(label, style))),
                label_area,
            );
            app.mouse_map.register(cell_area, MouseZone::ListItem(pos));
        }
    }

    pub(crate) fn info_in_pane(f: &mut ratatui::Frame, sep_area: Rect, area: Rect, app: &mut App) {
        let fields = match track_info_fields(app) {
            Some(fields) => fields,
            None => return,
        };
        let has_cover = fields.has_cover;
        // The artwork is sized from the box the card was handed, not from the
        // docked card's 24 columns: the float sizes its box around the art it
        // can show, and a gate on `COVER_W` threw that art away and left four
        // lines of text marooned in a box built for a cover.
        let can_cover = !no_image_protocol() && area.width >= 6 && area.height >= 8;

        let sep_style = Style::default().fg(app.theme.fg_dim);
        let sep_label = String::new();
        f.render_widget(
            Paragraph::new(Line::from(Span::styled(sep_label, sep_style))),
            sep_area,
        );

        if can_cover {
            // Adaptive cover: use as much of the card height for cover as
            // fits while keeping the field block whole below it.
            let card_h = area.height;
            let text_need = INFO_FIELDS_H;
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
    // Zen mode: exactly one fullscreen surface at a time — no chrome, no
    // footer. A picker still draws over it: zen is a surface, not a modal, and
    // a picker opened by an event (a link flow finishing, a track landing) was
    // otherwise invisible while still swallowing every keystroke.
    // Daydreaming shares Zen's route — the visualizer, full screen — because
    // that is what it is: a preview shown when nobody is driving. It is not
    // Zen, so it never changes `app.zen` and so cannot be cycled away from; the
    // next keypress ends it. Both yield to an open picker, which is a view the
    // user asked for.
    let zen = (app.zen || app.daydreaming) && !app.pickers.is_open();
    // The surface fill comes first and covers every mode. It used to sit below
    // the Zen branch, which returned before reaching it — and ratatui does not
    // clear between frames, it diffs. So Zen painted on top of whatever the
    // last non-Zen frame left behind: the library text, the footer and the
    // brand badge stayed on screen for as long as Zen was open, and because
    // the background was never repainted, a reactive theme change could not
    // reach the screen at all. The text and accents did update, so Zen was
    // half-reactive — new colours on the previous track's background.
    //
    // Zen gets its own fill, lifted off the app surface out of the reactive
    // palette: see `App::zen_bg`. On the app surface, a fullscreen surface with
    // no panes around it read as the library it covers.
    let bg = if zen { app.zen_bg() } else { app.surface_bg() };
    f.render_widget(
        ratatui::widgets::Block::default().style(ratatui::style::Style::default().bg(bg)),
        area,
    );

    // Two rows are reserved at the top of every view and left as surface.
    //
    // The brand badge used to be pinned to row 0 and the panes started
    // immediately under it, so the badge sat against both the top edge and the
    // content it labels. Reserving the rows puts the whole composition -- zen
    // included, since it is the same "no chrome" surface with the same badge --
    // one step down from the terminal edge.
    //
    // The badge is drawn inside the reserved band rather than above it, so the
    // reservation is what separates it from the content instead of being
    // overwritten by it. Overlays still draw over the whole area: they are
    // centred modals, and centring one inside a band would look off-centre.
    let top_rows = 2u16.min(area.height.saturating_sub(3));
    let vchunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(top_rows), Constraint::Min(0)])
        .split(area);
    let top_band = vchunks[0];
    let body = vchunks[1];

    if zen {
        Render::zen(f, body, app);
        Render::brand_badge(f, top_band, app);
        app.track_anim_trigger = false;
        return;
    }
    let footer_height = if app.hide_footer { 0 } else { 1 };
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(0), Constraint::Length(footer_height)])
        .split(body);

    Render::content(f, chunks[0], app);
    if !app.hide_footer {
        Render::footer(f, chunks[1], app);
    }

    Render::brand_badge(f, top_band, app);

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
    /// The "gtm" brand badge, pinned to the right of the reserved top band with
    /// the themed accent background.
    fn brand_badge(f: &mut ratatui::Frame, top_band: Rect, app: &App) {
        if top_band.height == 0 {
            return;
        }
        let brand_w: u16 = 7.min(top_band.width);
        if brand_w == 0 {
            return;
        }
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
                x: top_band.right().saturating_sub(brand_w),
                y: top_band.y,
                width: brand_w,
                height: 1,
            },
        );
    }

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
