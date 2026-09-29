// Copyright (c) 2026
// Author: prjctimg <prjctimg@outlook.com>
// Settings pane
//
// This is free software released under the GPL-3.0 license.

use crate::providers::spotify::SpotifyStatus;
use crate::ui::pickers::settings_rows::{RowKind, row_help, rows_for};
use crate::ui::*;

impl Pickers {
    pub(crate) fn render_settings(f: &mut ratatui::Frame, area: Rect, app: &App) {
        let block = Self::picker_panel(app, " Settings ", Some("Tab: switch pane   Enter: act"));
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

        // The row list is declared once, in `settings_rows`; only the values are
        // computed here.
        let decl = rows_for(app.settings_category);
        let values = Self::settings_values(app);

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
        for (i, (label, kind)) in decl.iter().enumerate() {
            let is_sel = i == sel && !settings_focus;
            let style = if is_sel {
                Style::default()
                    .fg(app.theme.selection_fg_readable())
                    .bg(app.theme.selection_bg)
            } else {
                Style::default().fg(app.theme.fg)
            };
            let mut row = format!(
                "{:<16}{}",
                label,
                values.get(i).cloned().unwrap_or_default()
            );
            // `▶` marks a row that has more than one value to move through, so
            // the cue is on the row's own kind rather than on each hand-written
            // label — it used to be missing from several rows that had one.
            if matches!(kind, RowKind::Cycle | RowKind::Chooser | RowKind::Action)
                && *kind != RowKind::Action
                && !row.contains("▶")
            {
                row.push_str("  ▶");
            }
            if *kind == RowKind::Action && !row.contains("Enter") {
                row.push_str("  Enter");
            }
            lines.push(Line::from(Span::styled(row, style)));
        }
        lines.push(Line::from(""));
        let help = row_help(app.settings_category, sel);
        if !help.is_empty() {
            lines.push(Line::from(Span::styled(
                help,
                Style::default().fg(app.theme.fg_dim),
            )));
        }

        let right_para = Paragraph::new(lines);
        f.render_widget(right_para, right_inner);
    }

    /// The value column of every settings row, index-aligned with
    /// [`rows_for`]. Returned as strings so the renderer stays a single `match`
    /// over values instead of one over labels and kinds.
    fn settings_values(app: &App) -> Vec<String> {
        match app.settings_category {
            0 => {
                let crossfade = app.state.crossfade.as_ref();
                vec![
                    format!("{:?}", app.state.repeat),
                    on_off(app.state.shuffle),
                    match crossfade {
                        Some(c) if c.enabled => format!("On  {}s", c.duration_secs),
                        _ => "Off".to_string(),
                    },
                    on_off(app.state.audio.eq_enabled),
                    on_off(app.state.audio.reverb.enabled),
                    cover_provider_label(&app.cover_provider).to_string(),
                ]
            }
            1 => {
                let st = app.state.audio.audio_device.clone();
                vec![
                    app.themes
                        .get(app.theme_index)
                        .map(|t| t.name.as_ref())
                        .unwrap_or("Chadrula")
                        .to_string(),
                    theme_mode_label(&app.theme_mode).to_string(),
                    st.unwrap_or_else(|| "System default".into()),
                    on_off(app.transparent_bg),
                    on_off(app.transparent_pickers),
                    on_off(app.hide_footer),
                    on_off(app.reactive_theme),
                    format!("{:.0}%", app.reactive_theme_intensity * 100.0),
                    app.visualizer.preset.name().to_string(),
                    app.footer_presets
                        .get(app.footer_preset)
                        .map(|p| p.name.as_ref())
                        .unwrap_or("Default")
                        .to_string(),
                    String::new(),
                    String::new(),
                    String::new(),
                    String::new(),
                    String::new(),
                    String::new(),
                ]
            }
            2 => {
                let st = app.spotify.status.clone().unwrap_or_default();
                vec![
                    Self::spotify_status_line(&st),
                    String::new(),
                    String::new(),
                    String::new(),
                    String::new(),
                ]
            }
            _ => Vec::new(),
        }
    }

    /// One merged line: connection, account, playlists and the Connect device.
    ///
    /// It is a single row because it is a single fact — whether the account can
    /// play audio. Split across four rows, a user had to read all four to find
    /// out the one thing they came for.
    fn spotify_status_line(st: &SpotifyStatus) -> String {
        let mut status = if !st.linked {
            "Disconnected".to_string()
        } else if st.needs_relink {
            "Relink required".to_string()
        } else if st.needs_play_link {
            // Everything but audio works. Re-linking cannot fix this one: the
            // Web API token belongs to a different app than the one Connect
            // accepts, so the account needs its own playback authorization.
            "Playback not linked".to_string()
        } else if let Some(err) = st.error.as_deref() {
            let mut e: String = err.chars().take(20).collect();
            if err.chars().count() > 20 {
                e.push('…');
            }
            format!("Connected: {e}")
        } else if st.premium {
            if st.playing {
                "Playing ▶".to_string()
            } else {
                "Paused  ❚❚".to_string()
            }
        } else {
            "Unavailable (Premium)".to_string()
        };
        if st.linked {
            if let Some(user) = st.user.as_deref().filter(|u| !u.is_empty()) {
                status.push_str(&format!(" · {user}"));
            }
            status.push_str(&format!(" · {} playlists", st.playlists));
            if let Some(device) = st.device.as_deref().filter(|d| !d.is_empty()) {
                status.push_str(&format!(" · {device}"));
            }
        }
        let mut c: String = status.chars().take(52).collect();
        if status.chars().count() > 52 {
            c.push('…');
        }
        c
    }
}

fn on_off(v: bool) -> String {
    if v {
        "On".to_string()
    } else {
        "Off".to_string()
    }
}
