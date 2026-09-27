// Copyright (c) 2026
// Author: prjctimg <prjctimg@outlook.com>
// Spotify search picker
//
//
// This is free software released under the GPL-3.0 license.

use crate::ui::*;

pub(crate) fn spotify_waiting_lines(app: &App) -> Vec<Line<'static>> {
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
