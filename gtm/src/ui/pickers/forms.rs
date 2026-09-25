// Copyright (c) 2026
// Author: prjctimg <prjctimg@outlook.com>
// Setup and URL entry forms
//
//
// This is free software released under the GPL-3.0 license.

use crate::ui::*;

impl Pickers {
    pub(crate) fn render_setup(f: &mut ratatui::Frame, area: Rect, app: &mut App) {
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

    pub(crate) fn render_lastfm_setup(f: &mut ratatui::Frame, area: Rect, app: &mut App) {
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

    pub(crate) fn render_load_stream(f: &mut ratatui::Frame, area: Rect, app: &mut App) {
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

    pub(crate) fn render_youtube_setup(f: &mut ratatui::Frame, area: Rect, app: &mut App) {
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
}
