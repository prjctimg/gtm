use super::*;

impl Pickers {
    /// Destinations for the track on air: Liked Songs at row 0, then the
    /// user's synced Spotify playlists, filtered by the picker's query. Marks
    /// mirror `render_track_select` so a tick reads the same everywhere.
    pub(crate) fn render_live_dest(f: &mut ratatui::Frame, area: Rect, app: &mut App) {
        let picked = app.live_dests.len();
        let hint = format!(
            "Space/Tab: toggle   \u{2191}/\u{2193}: navigate   Ctrl+Enter: add ({picked} selected)   Esc: cancel"
        );
        let block = Self::picker_panel(app, " Add to Spotify ", Some(&hint));
        let inner = block.inner(area);
        f.render_widget(block, area);

        let Some(query) = app.live_query.clone() else {
            let p = Paragraph::new("No live track resolved")
                .style(Style::default().fg(app.theme.fg_dim));
            f.render_widget(p, inner);
            return;
        };
        let names = app.live_dest_names(&query);
        if names.is_empty() {
            let p = Paragraph::new("No Spotify playlists — link an account first")
                .style(Style::default().fg(app.theme.fg_dim));
            f.render_widget(p, inner);
            return;
        }
        let total = names.len() + 1;
        let sel = app
            .pickers
            .top()
            .map_or(0, |o| o.selected.min(total.saturating_sub(1)));
        let visible = inner.height as usize;
        let (scroll_start, scroll_end) = if let Some(top) = app.pickers.top_mut() {
            let (s, e) = step_viewport(top.viewport_offset, sel, visible, total);
            top.viewport_offset = s;
            (s, e)
        } else {
            (0, total)
        };

        let mut items: Vec<ListItem> = Vec::new();
        for i in scroll_start..scroll_end {
            // Row 0 is Liked Songs, keyed by the empty string since it has no
            // playlist id; the rest are the filtered playlists.
            let (key, label) = if i == 0 {
                (String::new(), "\u{2665} Liked Songs".to_string())
            } else {
                let (id, name) = &names[i - 1];
                (id.clone(), name.clone())
            };
            let is_picked = app.live_dests.contains(&key);
            let is_sel = i == sel;
            let mark = if is_picked { " \u{2713} " } else { "   " };
            let content = format!("{mark}{label}");
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
            let pad = if is_sel {
                row_pad(&content, inner.width)
            } else {
                0
            };
            let content = format!("{content}{}", " ".repeat(pad));
            let row_rect = Rect {
                x: inner.x,
                y: inner.y + (i - scroll_start) as u16,
                width: inner.width,
                height: 1,
            };
            app.mouse_map.register(row_rect, MouseZone::PickerItem(i));
            items.push(ListItem::new(content).style(style));
        }
        f.render_widget(List::new(items), inner);
    }
}
