// Copyright (c) 2026
// Author: prjctimg <prjctimg@outlook.com>
// Music browse picker: search, album tracklists, artist pages
//
//
// This is free software released under the GPL-3.0 license.

use crate::app::BrowseLevel;
use crate::ui::pickers::queue::ScrollList;
use crate::ui::*;

impl Pickers {
    /// The Browse overlay: one list for all three levels.
    ///
    /// A song plays, a release or a person opens. The level only changes what a
    /// row means, so there is no reason for three overlays — and three would
    /// mean three cursor and scroller states to keep coherent for one question.
    pub(crate) fn render_browse(f: &mut ratatui::Frame, area: Rect, app: &mut App) {
        let rows = app.browse_len();
        let mut out = Vec::new();
        for i in 0..rows {
            let text = Self::browse_row(app, i);
            out.push(text);
        }
        let title = match app.browse.level {
            BrowseLevel::Results => " Browse ".to_string(),
            BrowseLevel::Album => format!(" {} ", app.browse.heading),
            BrowseLevel::Artist => format!(" {} ", app.browse.heading),
        };
        let mut prepend = Vec::new();
        let query = app
            .pickers
            .top()
            .map(|p| p.query.clone())
            .unwrap_or_default();
        if app.browse.level == BrowseLevel::Results {
            prepend.push(Line::from(Span::styled(
                format!(" search: {query}"),
                Style::default().fg(app.theme.fg_dim),
            )));
        } else {
            prepend.push(Line::from(Span::styled(
                " backspace: back to results",
                Style::default().fg(app.theme.fg_dim),
            )));
        }
        Self::render_scroll_rows(
            f,
            area,
            app,
            ScrollList {
                title: &title,
                hint: "enter open \u{b7} esc close",
                empty_msg: if app.browse.pending {
                    " searching\u{2026}"
                } else if query.is_empty() {
                    " type to search for a song, album or artist"
                } else {
                    " nothing found"
                },
            },
            prepend,
            out,
        );
    }

    /// One row of the Browse list, in the same shape every picker row uses.
    fn browse_row(app: &App, i: usize) -> String {
        let tracks = &app.browse.rows;
        let releases = &app.browse.releases;
        let with_releases = !releases.is_empty();
        if i < tracks.len() {
            let t = &tracks[i];
            let dur = t
                .duration_secs
                .map(format_duration_short)
                .unwrap_or_else(|| "--:--".to_string());
            let who = if t.artist.is_empty() {
                String::new()
            } else {
                format!(" \u{2014} {}", t.artist)
            };
            return format!("\u{266b} [{dur}] {}{who}", t.title);
        }
        if with_releases && i == tracks.len() {
            return "  \u{2500}\u{2500} releases \u{2500}\u{2500}".to_string();
        }
        let a = &releases[i - tracks.len() - usize::from(with_releases)];
        let n = a
            .track_count
            .map(|c| format!("\u{2003}[{c}]"))
            .unwrap_or_default();
        format!("\u{1f4bc} {}{n}", a.title)
    }
}
