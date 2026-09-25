// Copyright (c) 2026
// Author: prjctimg <prjctimg@outlook.com>
// Settings pane
//
//
// This is free software released under the GPL-3.0 license.

use crate::ui::*;

impl Pickers {
    pub(crate) fn render_settings(f: &mut ratatui::Frame, area: Rect, app: &App) {
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
