// Copyright (c) 2026
// Author: prjctimg <prjctimg@outlook.com>
// Podcast feed and episode pickers
//
//
// This is free software released under the GPL-3.0 license.

use crate::ui::*;

impl Pickers {
    pub(crate) fn render_podcast_feeds(f: &mut ratatui::Frame, area: Rect, app: &mut App) {
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

    pub(crate) fn render_podcast_episodes(f: &mut ratatui::Frame, area: Rect, app: &mut App) {
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

    pub(crate) fn render_podcast_subscribe(f: &mut ratatui::Frame, area: Rect, app: &mut App) {
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
}
