// Copyright (c) 2026
// Author: prjctimg <prjctimg@outlook.com>
// Picker dispatch and shared panel chrome
//
//
// This is free software released under the GPL-3.0 license.

use crate::ui::*;

pub mod forms;
pub mod library;
pub mod palette;
pub mod podcast;
pub mod presets;
pub mod queue;
pub mod radio;
pub mod search;
pub mod settings;
pub mod spotify;
pub mod system;

impl Pickers {
    pub(crate) fn picker_content_hint(top: &Picker, app: &App) -> (u16, u16) {
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
            PickerId::AudioDevice => (60, 14),
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

    pub(crate) fn render_picker(f: &mut ratatui::Frame, area: Rect, app: &mut App) {
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
            PickerId::AudioDevice => Self::render_audio_device(f, picker_area, app),
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
                        for (row, &i) in
                            picks.iter().enumerate().take(scroll_end).skip(scroll_start)
                        {
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

    pub(crate) fn picker_panel<'a>(
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

    pub(crate) fn picker_query_line(app: &App) -> Line<'static> {
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
}
