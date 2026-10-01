// Copyright (c) 2026
// Author: prjctimg <prjctimg@outlook.com>
// Podcast feed and episode pickers
//
//
// This is free software released under the GPL-3.0 license.

use crate::ui::pickers::queue::ScrollList;
use crate::ui::*;

impl Pickers {
    /// The subscribed feed list, and — when one is showing — the directory
    /// search that found it.
    ///
    /// The two are the same list rather than two pickers: they answer the same
    /// question ("what can I listen to"), a search result is one Enter away from
    /// a subscription, and stacking them behind each other meant the empty state
    /// of a fresh install was a dead end with a URL box as its only exit.
    pub(crate) fn render_podcast_feeds(f: &mut ratatui::Frame, area: Rect, app: &mut App) {
        // A search replaces the subscriptions for as long as it has results:
        // showing both at once under one cursor would mean one set of keys doing
        // two different things depending on what scrolled into view.
        let searching = !app.podcast.results.is_empty() || app.podcast.searching;
        let mut rows = Vec::new();
        if searching {
            for r in &app.podcast.results {
                let subscribed = app.podcast.feeds.iter().any(|f| f.url == r.url);
                let mark = if subscribed { "\u{2005}" } else { "+" };
                let eps = if r.episodes > 0 {
                    format!("\u{2003}[{} episodes]", r.episodes)
                } else {
                    String::new()
                };
                let who = if r.author.is_empty() {
                    String::new()
                } else {
                    format!(" \u{2014} {}", r.author)
                };
                rows.push(format!("\u{1f4e1} {mark} {}{who}{eps}", r.title));
            }
        } else {
            for feed in &app.podcast.feeds {
                rows.push(format!(
                    "\u{1f4e1} {} \u{2003}[{} episodes]",
                    feed.title, feed.episodes
                ));
            }
        }
        let mut prepend = Vec::new();
        if searching {
            if let Some(top) = app.pickers.top() {
                prepend.push(Line::from(Span::styled(
                    format!(" search: {}", top.query),
                    Style::default().fg(app.theme.fg_dim),
                )));
            }
        } else if let Some(st) = app.podcast.status.as_ref() {
            prepend.push(Line::from(Span::styled(
                format!(" {} feeds, {} episodes", st.feeds, st.episodes),
                Style::default().fg(app.theme.fg_dim),
            )));
        }
        Self::render_scroll_rows(
            f,
            area,
            app,
            ScrollList {
                title: if searching {
                    " Podcasts \u{2014} search "
                } else {
                    " Podcasts "
                },
                hint: if searching {
                    "enter subscribe \u{b7} esc back to subscriptions"
                } else {
                    "/ search \u{b7} a add by URL"
                },
                empty_msg: if searching {
                    " no results \u{2014} esc to go back"
                } else if app.podcast.feeds_pending {
                    " loading feeds\u{2026}"
                } else {
                    "no subscriptions \u{2014} press / to search, a to add by URL"
                },
            },
            prepend,
            rows,
        );
    }

    pub(crate) fn render_podcast_episodes(f: &mut ratatui::Frame, area: Rect, app: &mut App) {
        let selected = app.pickers.top().map_or(0, |o| o.selected);
        let mut rows = Vec::new();
        for ep in &app.podcast.episodes {
            let dur = ep
                .duration_secs
                .map(format_duration_short)
                .unwrap_or_else(|| "--:--".to_string());
            // Two marks that used to be missing and that change what a row can
            // do: an episode with no enclosure in the feed cannot be played at
            // all, and only some feeds publish a transcript.
            let mut marks = String::new();
            if ep.url.trim().is_empty() {
                marks.push_str("  [no audio]");
            }
            if !ep.transcripts.is_empty() {
                marks.push_str("  [transcript]");
            }
            rows.push(format!("\u{266b} [{dur}] {}{marks}", ep.title));
        }
        let title = app
            .podcast
            .episodes
            .first()
            .map(|e| format!(" {} ", e.feed_title))
            .unwrap_or_else(|| " Episodes ".into());
        let has_transcript = app
            .podcast
            .episodes
            .get(selected)
            .is_some_and(|e| !e.transcripts.is_empty());
        let hint = if has_transcript {
            "enter play \u{b7} t transcript"
        } else {
            "enter play"
        };
        Self::render_scroll_rows(
            f,
            area,
            app,
            ScrollList {
                title: &title,
                hint,
                empty_msg: "no episodes \u{2014} press r in the feed list to refresh",
            },
            Vec::new(),
            rows,
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
