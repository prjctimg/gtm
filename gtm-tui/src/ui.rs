use gtm_core::state::{PlaybackStatus, RepeatMode, Tab};
use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Clear, Gauge, List, ListItem, Paragraph, Tabs};
use ratatui::Frame;

use crate::app::{App, InputMode};

pub fn render(f: &mut Frame, app: &mut App) {
    let area = f.area();
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(0),
            Constraint::Length(3),
        ])
        .split(area);

    render_tabs(f, chunks[0], app);
    render_content(f, chunks[1], app);
    render_footer(f, chunks[2], app);
}

fn render_tabs(f: &mut Frame, area: Rect, app: &App) {
    let tab_names = vec![
        " NowPlaying ",
        " Library ",
        " Queue ",
        " YouTube ",
        " Settings ",
        " Help ",
    ];
    let titles: Vec<Line> = tab_names
        .iter()
        .enumerate()
        .map(|(i, name)| {
            let tab = match i {
                0 => Tab::NowPlaying,
                1 => Tab::Library,
                2 => Tab::Queue,
                3 => Tab::YouTube,
                4 => Tab::Settings,
                5 => Tab::Help,
                _ => Tab::NowPlaying,
            };
            if tab == app.current_tab {
                Line::styled(*name, Style::default().fg(Color::Black).bg(Color::Cyan))
            } else {
                Line::styled(*name, Style::default().fg(Color::Cyan))
            }
        })
        .collect();

    let status_icon = match app.state.status {
        PlaybackStatus::Playing => "\u{25B6}",
        PlaybackStatus::Paused => "\u{23F8}",
        PlaybackStatus::Stopped => "\u{25A0}",
    };

    let vol = if app.state.mute {
        "MUT".to_string()
    } else {
        format!("{:3}%", app.state.volume)
    };

    let repeat_icon = match app.state.repeat {
        RepeatMode::Off => "",
        RepeatMode::One => " \u{1F501}",
        RepeatMode::All => " \u{1F500}",
    };

    let shuffle_icon = if app.state.shuffle { " \u{1F500}" } else { "" };

    let status_line = format!(" {status_icon} Vol:{vol}{repeat_icon}{shuffle_icon} ");

    let title_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Min(0),
            Constraint::Length(status_line.len() as u16),
        ])
        .split(area);

    let tabs = Tabs::new(titles)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_type(BorderType::Plain)
                .title(" GTM ")
                .title_alignment(Alignment::Center),
        )
        .highlight_style(Style::default().add_modifier(Modifier::BOLD));

    f.render_widget(tabs, title_chunks[0]);

    let status_para = Paragraph::new(status_line).style(
        Style::default()
            .fg(Color::Green)
            .add_modifier(Modifier::BOLD),
    );
    f.render_widget(status_para, title_chunks[1]);
}

fn render_content(f: &mut Frame, area: Rect, app: &mut App) {
    match app.current_tab {
        Tab::NowPlaying => render_now_playing(f, area, app),
        Tab::Library => render_list(f, area, app, "Library", &app.tracks_cache),
        Tab::Queue => render_queue(f, area, app),
        Tab::YouTube => render_yt_results(f, area, app),
        Tab::Settings => render_settings(f, area, app),
        Tab::Help => render_help(f, area),
    }
}

fn render_now_playing(f: &mut Frame, area: Rect, app: &App) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(4),
            Constraint::Length(3),
            Constraint::Length(3),
            Constraint::Min(0),
        ])
        .margin(2)
        .split(area);

    // Track info
    let track = match &app.state.current_track {
        Some(t) => t,
        None => {
            let p = Paragraph::new("No track playing")
                .alignment(Alignment::Center)
                .style(Style::default().fg(Color::Gray));
            f.render_widget(p, chunks[0]);
            return;
        }
    };

    let title = Line::from(vec![
        Span::styled("Title:  ", Style::default().fg(Color::Gray)),
        Span::styled(
            &track.title,
            Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD),
        ),
    ]);
    let artist = Line::from(vec![
        Span::styled("Artist: ", Style::default().fg(Color::Gray)),
        Span::styled(&track.artist, Style::default().fg(Color::Yellow)),
    ]);
    let album = Line::from(vec![
        Span::styled("Album:  ", Style::default().fg(Color::Gray)),
        Span::styled(&track.album, Style::default().fg(Color::Cyan)),
    ]);

    let mut info_text = vec![title, artist, album];

    if !track.genre.is_empty() {
        info_text.push(Line::from(vec![
            Span::styled("Genre:  ", Style::default().fg(Color::Gray)),
            Span::styled(&track.genre, Style::default().fg(Color::Green)),
        ]));
    }

    let info_block = Block::default()
        .borders(Borders::ALL)
        .title(" Now Playing ")
        .border_type(BorderType::Rounded);
    let info_para = Paragraph::new(info_text).block(info_block);
    f.render_widget(info_para, chunks[0]);

    // Progress bar
    let dur = track.duration;
    let pos = app.state.time_pos;
    let ratio = if dur > 0.0 { (pos / dur) as f64 } else { 0.0 };
    let pos_str = format_duration(pos as u64);
    let dur_str = format_duration(dur as u64);

    let gauge = Gauge::default()
        .block(
            Block::default()
                .title(format!(" {pos_str} / {dur_str} "))
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded),
        )
        .gauge_style(
            Style::default()
                .fg(Color::Cyan)
                .bg(Color::DarkGray)
                .add_modifier(Modifier::BOLD),
        )
        .ratio(ratio);
    f.render_widget(gauge, chunks[1]);

    // Volume bar
    let vol_ratio = if app.state.mute {
        0.0
    } else {
        app.state.volume as f64 / 100.0
    };
    let vol_label = if app.state.mute {
        " Volume: MUTED ".to_string()
    } else {
        format!(" Volume: {:3}% ", app.state.volume)
    };
    let vol_gauge = Gauge::default()
        .block(
            Block::default()
                .title(format!(" {vol_label} "))
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded),
        )
        .gauge_style(
            Style::default()
                .fg(Color::Green)
                .bg(Color::DarkGray)
                .add_modifier(Modifier::BOLD),
        )
        .ratio(vol_ratio);
    f.render_widget(vol_gauge, chunks[2]);

    // Controls hint
    let controls = Paragraph::new(
        "[Space] Play/Pause  [n] Next  [b] Prev  [+/-] Volume  [m] Mute  [r] Repeat  [h] Shuffle  [:] Cmd",
    )
    .alignment(Alignment::Center)
    .style(Style::default().fg(Color::DarkGray));
    f.render_widget(controls, chunks[3]);
}

fn render_list(
    f: &mut Frame,
    area: Rect,
    app: &App,
    title: &str,
    items: &[gtm_core::track::TrackInfo],
) {
    let filtered: Vec<&gtm_core::track::TrackInfo> = if app.search_query.is_empty() {
        items.iter().collect()
    } else {
        let q = app.search_query.to_lowercase();
        items
            .iter()
            .filter(|t| {
                t.title.to_lowercase().contains(&q)
                    || t.artist.to_lowercase().contains(&q)
                    || t.album.to_lowercase().contains(&q)
            })
            .collect()
    };

    let sel = app.scroll_offset.min(filtered.len().saturating_sub(1));

    let list_items: Vec<ListItem> = filtered
        .iter()
        .enumerate()
        .map(|(i, track)| {
            let prefix = if i == sel { " \u{25B6} " } else { "   " };
            let dur = format_duration(track.duration as u64);
            let content = format!("{prefix}{} - {} [{}]", track.artist, track.title, dur);
            let style = if i == sel {
                Style::default().fg(Color::Black).bg(Color::Cyan)
            } else {
                Style::default()
            };
            ListItem::new(content).style(style)
        })
        .collect();

    let list = List::new(list_items).block(
        Block::default()
            .borders(Borders::ALL)
            .title(format!(" {title} "))
            .border_type(BorderType::Rounded),
    );

    f.render_widget(list, area);
}

fn render_queue(f: &mut Frame, area: Rect, app: &mut App) {
    let filtered: Vec<(usize, &gtm_core::track::TrackInfo)> = if app.search_query.is_empty() {
        app.queue_cache.iter().enumerate().collect()
    } else {
        let q = app.search_query.to_lowercase();
        app.queue_cache
            .iter()
            .enumerate()
            .filter(|(_, t)| {
                t.title.to_lowercase().contains(&q) || t.artist.to_lowercase().contains(&q)
            })
            .collect()
    };

    let list_items: Vec<ListItem> = filtered
        .iter()
        .map(|(idx, track)| {
            let is_current = *idx == app.queue_cursor;
            let prefix = if is_current { " \u{25B6} " } else { "   " };
            let dur = format_duration(track.duration as u64);
            let content = format!(
                "{prefix}#{} {} - {} [{}]",
                idx, track.artist, track.title, dur
            );
            let style = if is_current {
                Style::default().fg(Color::Black).bg(Color::Cyan)
            } else {
                Style::default()
            };
            ListItem::new(content).style(style)
        })
        .collect();

    let list = List::new(list_items).block(
        Block::default()
            .borders(Borders::ALL)
            .title(" Queue ")
            .border_type(BorderType::Rounded),
    );

    f.render_widget(list, area);
}

fn render_yt_results(f: &mut Frame, area: Rect, app: &App) {
    let len = app.yt_results_cache.len();
    let sel = if len > 0 {
        app.scroll_offset.min(len - 1)
    } else {
        0
    };
    let items: Vec<ListItem> = app
        .yt_results_cache
        .iter()
        .enumerate()
        .map(|(i, r)| {
            let dur = format_duration(r.duration as u64);
            let prefix = if i == sel { " \u{25B6} " } else { "   " };
            let content = format!("{prefix}{} - {} [{}]", r.channel, r.title, dur);
            let style = if i == sel {
                Style::default().fg(Color::Black).bg(Color::Cyan)
            } else {
                Style::default()
            };
            ListItem::new(content).style(style)
        })
        .collect();

    let list = List::new(items).block(
        Block::default()
            .borders(Borders::ALL)
            .title(" YouTube Search Results ")
            .border_type(BorderType::Rounded),
    );

    f.render_widget(list, area);
}

fn render_settings(f: &mut Frame, area: Rect, app: &App) {
    let items = vec![
        format!(
            "Volume:    {}% {}",
            app.state.volume,
            if app.state.mute { "(MUTED)" } else { "" }
        ),
        format!("Repeat:    {:?}", app.state.repeat),
        format!(
            "Shuffle:   {}",
            if app.state.shuffle { "ON" } else { "OFF" }
        ),
        format!("Mute:      {}", if app.state.mute { "ON" } else { "OFF" }),
        format!(
            "Crossfade: {} ({}s)",
            if app
                .state
                .crossfade
                .as_ref()
                .map(|c| c.enabled)
                .unwrap_or(false)
            {
                "ON"
            } else {
                "OFF"
            },
            app.state
                .crossfade
                .as_ref()
                .map(|c| c.duration_secs)
                .unwrap_or(0)
        ),
        format!("Status:    {:?}", app.state.status),
        format!("Queue:     {} tracks", app.state.queue.len()),
    ];

    let settings_items: Vec<ListItem> = items.into_iter().map(|s| ListItem::new(s)).collect();

    let list = List::new(settings_items).block(
        Block::default()
            .borders(Borders::ALL)
            .title(" Settings ")
            .border_type(BorderType::Rounded),
    );

    f.render_widget(list, area);
}

fn render_help(f: &mut Frame, area: Rect) {
    let help_text = vec![
        Line::from(Span::styled(
            "Keyboard Shortcuts",
            Style::default().add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
        Line::from(vec![
            Span::styled("1-6    ", Style::default().fg(Color::Cyan)),
            Span::from("Switch tabs"),
        ]),
        Line::from(vec![
            Span::styled("Space/p", Style::default().fg(Color::Cyan)),
            Span::from("Play / Pause"),
        ]),
        Line::from(vec![
            Span::styled("s      ", Style::default().fg(Color::Cyan)),
            Span::from("Search/filter current tab"),
        ]),
        Line::from(vec![
            Span::styled("n/b    ", Style::default().fg(Color::Cyan)),
            Span::from("Next / Previous track"),
        ]),
        Line::from(vec![
            Span::styled("+/-    ", Style::default().fg(Color::Cyan)),
            Span::from("Volume up/down"),
        ]),
        Line::from(vec![
            Span::styled("m      ", Style::default().fg(Color::Cyan)),
            Span::from("Toggle mute"),
        ]),
        Line::from(vec![
            Span::styled("r      ", Style::default().fg(Color::Cyan)),
            Span::from("Cycle repeat mode"),
        ]),
        Line::from(vec![
            Span::styled("h      ", Style::default().fg(Color::Cyan)),
            Span::from("Toggle shuffle"),
        ]),
        Line::from(vec![
            Span::styled("d/Del  ", Style::default().fg(Color::Cyan)),
            Span::from("Remove from queue"),
        ]),
        Line::from(vec![
            Span::styled("Enter  ", Style::default().fg(Color::Cyan)),
            Span::from("Play selected item"),
        ]),
        Line::from(vec![
            Span::styled(":      ", Style::default().fg(Color::Cyan)),
            Span::from("Command mode (e.g. :100 to set volume)"),
        ]),
        Line::from(vec![
            Span::styled("q/Esc  ", Style::default().fg(Color::Cyan)),
            Span::from("Quit"),
        ]),
    ];

    let p = Paragraph::new(help_text)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(" Help ")
                .border_type(BorderType::Rounded),
        )
        .style(Style::default());
    f.render_widget(p, area);
}

fn render_footer(f: &mut Frame, area: Rect, app: &App) {
    match app.input_mode {
        InputMode::Normal => {
            let footer = Paragraph::new(
                " [1]NowPlaying [2]Library [3]Queue [4]YouTube [5]Settings [6]Help | Space:Pause n:Next b:Prev +/-:Vol :Cmd q:Quit ",
            )
            .style(
                Style::default()
                    .fg(Color::Black)
                    .bg(if app.state.status == PlaybackStatus::Playing {
                        Color::Green
                    } else {
                        Color::DarkGray
                    }),
            );
            f.render_widget(footer, area);
        }
        InputMode::Searching => {
            let input = Paragraph::new(format!(" Search: {}", app.search_query))
                .style(Style::default().fg(Color::Black).bg(Color::Yellow));
            f.render_widget(input, area);
        }
        InputMode::Command => {
            let input = Paragraph::new(format!(" :{}", app.search_query))
                .style(Style::default().fg(Color::Black).bg(Color::Cyan));
            f.render_widget(input, area);
        }
    }

    // Error overlay
    if let Some(ref err) = app.error_message {
        let err_text = format!(" Error: {err} ");
        let err_area = Rect {
            x: area.x,
            y: area.y.saturating_sub(1),
            width: err_text.len() as u16,
            height: 1,
        };
        let err_para = Paragraph::new(err_text.clone())
            .style(Style::default().fg(Color::White).bg(Color::Red));
        f.render_widget(Clear, err_area);
        f.render_widget(err_para, err_area);
    }
}

fn format_duration(secs: u64) -> String {
    let h = secs / 3600;
    let m = (secs % 3600) / 60;
    let s = secs % 60;
    if h > 0 {
        format!("{h}:{m:02}:{s:02}")
    } else {
        format!("{m}:{s:02}")
    }
}
