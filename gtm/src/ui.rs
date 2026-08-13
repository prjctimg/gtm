use std::path::PathBuf;

use crossterm::terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen};
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Clear, List, ListItem, Paragraph, Tabs};
use ratatui::Terminal;
use crate::app::{App, InputMode};
use crate::overlay::OverlayId;
use gtm_core::state::{EqPreset, PlaybackStatus, RepeatMode, Tab};

pub fn run_tui(socket: Option<String>) -> Result<(), Box<dyn std::error::Error>> {
    let socket_path = socket
        .map(PathBuf::from)
        .unwrap_or_else(default_socket);

    ensure_daemon_running(&socket_path)?;

    let rt = tokio::runtime::Runtime::new()?;
    rt.block_on(async {
        color_eyre::install()?;

        enable_raw_mode()?;
        let mut stdout = std::io::stdout();
        crossterm::execute!(stdout, EnterAlternateScreen)?;
        let mut terminal = Terminal::new(CrosstermBackend::new(stdout))?;
        terminal.clear()?;

        let panic_hook = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |panic| {
            let _ = disable_raw_mode();
            let mut stdout = std::io::stdout();
            let _ = crossterm::execute!(stdout, LeaveAlternateScreen);
            panic_hook(panic);
        }));

        let res = App::new(&socket_path).await?.run(&mut terminal).await;

        let _ = disable_raw_mode();
        let mut stdout = std::io::stdout();
        let _ = crossterm::execute!(stdout, LeaveAlternateScreen);

        res
    })
}

fn default_socket() -> PathBuf {
    let runtime =
        std::env::var("XDG_RUNTIME_DIR").unwrap_or_else(|_| "/run/user/1000".into());
    PathBuf::from(runtime).join("gtmd.socket")
}

fn ensure_daemon_running(socket_path: &std::path::Path) -> Result<(), Box<dyn std::error::Error>> {
    if socket_path.exists() {
        if let Ok(mut stream) = std::os::unix::net::UnixStream::connect(socket_path) {
            let ping = serde_json::to_string(&gtm_core::ipc::DaemonReq::Ping)? + "\n";
            use std::io::Write;
            if stream.write_all(ping.as_bytes()).is_ok() {
                return Ok(());
            }
        }
    }

    let gtmd_path = find_gtmd_binary()?;
    let socket_arg = format!("--socket={}", socket_path.display());

    let mut child = std::process::Command::new(&gtmd_path)
        .arg(&socket_arg)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|e| format!("Failed to start gtmd at {gtmd_path:?}: {e}"))?;

    std::thread::spawn(move || {
        let _ = child.wait();
    });

    Ok(())
}

fn find_gtmd_binary() -> Result<std::path::PathBuf, Box<dyn std::error::Error>> {
    if let Ok(exe) = std::env::current_exe() {
        if let Some(parent) = exe.parent() {
            let candidate = parent.join("gtmd");
            if candidate.exists() {
                return Ok(candidate);
            }
        }
    }

    if let Ok(paths) = std::env::var("PATH") {
        for dir in std::env::split_paths(&paths) {
            let candidate = dir.join("gtmd");
            if candidate.exists() {
                return Ok(candidate);
            }
        }
    }

    let candidate = std::path::PathBuf::from("/usr/bin/gtmd");
    if candidate.exists() {
        return Ok(candidate);
    }

    Err("gtmd binary not found".into())
}

// ─── Layout ───

pub fn render(f: &mut ratatui::Frame, app: &mut App) {
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

    // Render overlays on top of everything
    if app.overlays.is_open() {
        render_overlay(f, area, app);
    }
}

// ─── Tab Bar ───

fn render_tabs(f: &mut ratatui::Frame, area: Rect, app: &App) {
    let tab_names = vec![" NowPlaying ", " Library ", " Settings "];
    let titles: Vec<Line> = tab_names
        .iter()
        .enumerate()
        .map(|(i, name)| {
            let tab = match i {
                0 => Tab::NowPlaying,
                1 => Tab::Library,
                2 => Tab::Settings,
                _ => Tab::NowPlaying,
            };
            if tab == app.current_tab {
                Line::styled(*name, Style::default().fg(app.theme.tab_active_fg).bg(app.theme.tab_active_bg))
            } else {
                Line::styled(*name, Style::default().fg(app.theme.tab_inactive_fg))
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

    let overlay_hint = if app.overlays.is_open() {
        " [Esc]Close "
    } else {
        " Alt+Q Queue "
    };

    let status_line = format!(" {status_icon} Vol:{vol}{repeat_icon}{shuffle_icon}{overlay_hint} ");

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
                .border_type(BorderType::Rounded)
                .title(" GTM ")
                .title_alignment(Alignment::Center),
        )
        .highlight_style(Style::default().add_modifier(Modifier::BOLD));

    f.render_widget(tabs, title_chunks[0]);

    let status_para = Paragraph::new(status_line)
        .style(Style::default().fg(app.theme.success).add_modifier(Modifier::BOLD));
    f.render_widget(status_para, title_chunks[1]);
}

// ─── Content Area ───

fn render_content(f: &mut ratatui::Frame, area: Rect, app: &mut App) {
    match app.current_tab {
        Tab::NowPlaying => render_now_playing(f, area, app),
        Tab::Library => render_list(f, area, app, "Library", &app.tracks_cache.clone()),
        Tab::Settings => render_settings(f, area, app),
    }
}

fn render_now_playing(f: &mut ratatui::Frame, area: Rect, app: &App) {
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

    let track = match &app.state.current_track {
        Some(t) => t,
        None => {
            let p = Paragraph::new("No track playing")
                .alignment(Alignment::Center)
                .style(Style::default().fg(app.theme.fg_dim));
            f.render_widget(p, chunks[0]);
            return;
        }
    };

    let title = Line::from(vec![
        Span::styled("Title:  ", Style::default().fg(app.theme.fg_dim)),
        Span::styled(
            &track.title,
            Style::default()
                .fg(app.theme.fg_bright)
                .add_modifier(Modifier::BOLD),
        ),
    ]);
    let artist = Line::from(vec![
        Span::styled("Artist: ", Style::default().fg(app.theme.fg_dim)),
        Span::styled(&track.artist, Style::default().fg(app.theme.warning)),
    ]);
    let album = Line::from(vec![
        Span::styled("Album:  ", Style::default().fg(app.theme.fg_dim)),
        Span::styled(&track.album, Style::default().fg(app.theme.accent)),
    ]);

    let mut info_text = vec![title, artist, album];

    if !track.genre.is_empty() {
        info_text.push(Line::from(vec![
            Span::styled("Genre:  ", Style::default().fg(app.theme.fg_dim)),
            Span::styled(&track.genre, Style::default().fg(app.theme.success)),
        ]));
    }

    let info_block = Block::default()
        .borders(Borders::ALL)
        .title(" Now Playing ")
        .border_type(BorderType::Rounded);
    let info_para = Paragraph::new(info_text).block(info_block);
    f.render_widget(info_para, chunks[0]);

    let dur = track.duration;
    let pos = app.state.time_pos;
    let ratio = if dur > 0.0 { (pos / dur) as f64 } else { 0.0 };
    let pos_str = format_duration(pos as u64);
    let dur_str = format_duration(dur as u64);

    let progress_line = render_progress_line(ratio, chunks[1].width.saturating_sub(4) as usize);
    let gauge = Paragraph::new(progress_line)
        .block(
            Block::default()
                .title(format!(" {pos_str} / {dur_str} "))
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded),
        )
        .style(Style::default().fg(app.theme.accent));
    f.render_widget(gauge, chunks[1]);

    let vol_ratio = if app.state.mute {
        0.0
    } else {
        app.state.volume as f64 / 100.0
    };
    let vol_label: String = if app.state.mute {
        "\u{1F507} Volume: MUTED ".to_string()
    } else {
        format!(" {} Volume: {:3}% ", volume_icon(app.state.volume), app.state.volume)
    };
    let vol_bar = render_progress_line(vol_ratio, chunks[2].width.saturating_sub(4) as usize);
    let vol_gauge = Paragraph::new(vol_bar)
        .block(
            Block::default()
                .title(format!(" {vol_label} "))
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded),
        )
        .style(Style::default().fg(app.theme.volume_color(app.state.volume)));
    f.render_widget(vol_gauge, chunks[2]);

    let controls = Paragraph::new(
        " \u{23EF}P/P [n]\u{23ED}Next [p]\u{23EE}Prev [+/-]\u{1F50A}Vol [m]\u{1F507}Mute [r]\u{1F501}Repeat [h]\u{1F500}Shuffle [:]Cmd [q]\u{1F6AA}Quit ",
    )
    .alignment(Alignment::Center)
    .style(Style::default().fg(app.theme.fg_dim));
    f.render_widget(controls, chunks[3]);
}

fn render_list(
    f: &mut ratatui::Frame,
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
                Style::default().fg(app.theme.selection_fg).bg(app.theme.selection_bg)
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

fn render_settings(f: &mut ratatui::Frame, area: Rect, app: &App) {
    let crossfade_on = app.state
        .crossfade
        .as_ref()
        .map(|c| c.enabled)
        .unwrap_or(false);
    let crossfade_dur = app.state
        .crossfade
        .as_ref()
        .map(|c| c.duration_secs)
        .unwrap_or(0);
    let crossfade_label = if crossfade_on {
        format!("Crossfade: ON  [c]toggle [C]dur: {}s", crossfade_dur)
    } else {
        "Crossfade: OFF  [c]toggle".to_string()
    };

    let items = vec![
        format!(
            "Volume:    {}% {}",
            app.state.volume,
            if app.state.mute { "(MUTED)" } else { "" }
        ),
        format!("Repeat:    {:?}", app.state.repeat),
        format!("Shuffle:   {}", if app.state.shuffle { "ON" } else { "OFF" }),
        format!("Mute:      {}", if app.state.mute { "ON" } else { "OFF" }),
        crossfade_label,
        format!("Status:    {:?}", app.state.status),
        format!("Queue:     {} tracks", app.state.queue.len()),
    ];

    let settings_items: Vec<ListItem> = items.into_iter().enumerate()
        .map(|(i, s)| {
            let style = if i == 4 {
                if crossfade_on {
                    Style::default().fg(app.theme.success)
                } else {
                    Style::default().fg(app.theme.fg_dim)
                }
            } else {
                Style::default()
            };
            ListItem::new(s).style(style)
        })
        .collect();

    let list = List::new(settings_items).block(
        Block::default()
            .borders(Borders::ALL)
            .title(" Settings ")
            .border_type(BorderType::Rounded),
    );

    f.render_widget(list, area);
}

// ─── Overlay Rendering ───

fn render_overlay(f: &mut ratatui::Frame, area: Rect, app: &mut App) {
    let Some(top) = app.overlays.top() else {
        return;
    };

    // Dim the background
    let dim = Style::default().fg(app.theme.overlay_bg);
    let dim_block = Block::default().style(dim);
    f.render_widget(dim_block, area);

    // Overlay box: centered, 60% width, 70% height
    let overlay_width = (area.width as f64 * 0.6) as u16;
    let overlay_height = (area.height as f64 * 0.7) as u16;
    let overlay_x = (area.width - overlay_width) / 2;
    let overlay_y = (area.height - overlay_height) / 3;

    let overlay_area = Rect {
        x: overlay_x,
        y: overlay_y,
        width: overlay_width,
        height: overlay_height,
    };

    // Use a block with rounded borders
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .title(format!(" {} ", top.id.title()))
        .style(Style::default().bg(app.theme.overlay_bg));

    let inner = block.inner(overlay_area);
    f.render_widget(Clear, overlay_area);
    f.render_widget(block, overlay_area);

    match top.id {
        OverlayId::Queue => render_queue_overlay(f, inner, app),
        OverlayId::YTSearch => render_yt_search_overlay(f, inner, app),
        OverlayId::SearchLibrary => render_search_library_overlay(f, inner, app),
        OverlayId::VolumeConfirm => render_volume_confirm_overlay(f, inner, app),
        OverlayId::About => render_about_overlay(f, inner, app),
        OverlayId::SleepTimer => render_sleep_timer_overlay(f, inner, app),
        OverlayId::CommandPalette => render_command_palette_overlay(f, inner, app),
        OverlayId::Equalizer => render_equalizer_overlay(f, inner, app),
        OverlayId::SoundEffects => render_sound_effects_overlay(f, inner, app),
        _ => {
            let p = Paragraph::new(format!("{} overlay", top.id.title()))
                .alignment(Alignment::Center)
                .style(Style::default().fg(app.theme.fg_dim));
            f.render_widget(p, inner);
        }
    }
}

fn render_queue_overlay(f: &mut ratatui::Frame, area: Rect, app: &App) {
    let items: Vec<ListItem> = app
        .queue_cache
        .iter()
        .enumerate()
        .map(|(i, track)| {
            let is_current = i == app.queue_cursor;
            let prefix = if is_current { " \u{25B6} " } else { "   " };
            let dur = format_duration(track.duration as u64);
            let content = format!("{prefix}#{} {} - {} [{}]", i, track.artist, track.title, dur);
            let style = if i == app.overlays.top().map_or(0, |o| o.selected) {
                Style::default().fg(app.theme.selection_fg).bg(app.theme.selection_bg)
            } else if is_current {
                Style::default().fg(app.theme.warning)
            } else {
                Style::default()
            };
            ListItem::new(content).style(style)
        })
        .collect();

    let list = List::new(items).block(
        Block::default()
            .borders(Borders::ALL)
            .title(" Queue ")
            .border_type(BorderType::Rounded),
    );

    f.render_widget(list, area);
}

fn render_yt_search_overlay(f: &mut ratatui::Frame, area: Rect, app: &App) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(3), Constraint::Min(0)])
        .split(area);

    let query = app.overlays.top().map_or(String::new(), |o| o.query.clone());
    let search_input = Paragraph::new(format!(" Search: {}", query))
        .style(Style::default().fg(app.theme.fg).bg(app.theme.warning));
    f.render_widget(search_input, chunks[0]);

    let items: Vec<ListItem> = app
        .yt_results_cache
        .iter()
        .enumerate()
        .map(|(i, r)| {
            let dur = format_duration(r.duration as u64);
            let prefix = if i == app.overlays.top().map_or(0, |o| o.selected) {
                " \u{25B6} "
            } else {
                "   "
            };
            let content = format!("{prefix}{} - {} [{}]", r.channel, r.title, dur);
            let style = if i == app.overlays.top().map_or(0, |o| o.selected) {
                Style::default().fg(app.theme.selection_fg).bg(app.theme.selection_bg)
            } else {
                Style::default()
            };
            ListItem::new(content).style(style)
        })
        .collect();

    let list = List::new(items).block(
        Block::default()
            .borders(Borders::ALL)
            .title(" Results ")
            .border_type(BorderType::Rounded),
    );

    f.render_widget(list, chunks[1]);
}

fn render_search_library_overlay(f: &mut ratatui::Frame, area: Rect, app: &App) {
    let query = app.overlays.top().map_or(String::new(), |o| o.query.clone());
    let filtered: Vec<&gtm_core::track::TrackInfo> = if query.is_empty() {
        app.tracks_cache.iter().collect()
    } else {
        let q = query.to_lowercase();
        app.tracks_cache
            .iter()
            .filter(|t| {
                t.title.to_lowercase().contains(&q)
                    || t.artist.to_lowercase().contains(&q)
            })
            .collect()
    };

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(3), Constraint::Min(0)])
        .split(area);

    let search_input = Paragraph::new(format!(" Search: {}", query))
        .style(Style::default().fg(app.theme.fg).bg(app.theme.warning));
    f.render_widget(search_input, chunks[0]);

    let sel = app.overlays.top().map_or(0, |o| o.selected.min(filtered.len().saturating_sub(1)));
    let items: Vec<ListItem> = filtered
        .iter()
        .enumerate()
        .map(|(i, track)| {
            let prefix = if i == sel { " \u{25B6} " } else { "   " };
            let dur = format_duration(track.duration as u64);
            let content = format!("{prefix}{} - {} [{}]", track.artist, track.title, dur);
            let style = if i == sel {
                Style::default().fg(app.theme.selection_fg).bg(app.theme.selection_bg)
            } else {
                Style::default()
            };
            ListItem::new(content).style(style)
        })
        .collect();

    let list = List::new(items).block(
        Block::default()
            .borders(Borders::ALL)
            .title(" Tracks ")
            .border_type(BorderType::Rounded),
    );

    f.render_widget(list, chunks[1]);
}

fn render_volume_confirm_overlay(f: &mut ratatui::Frame, area: Rect, app: &App) {
    let vol = app.pending_volume.unwrap_or(app.state.volume);
    let lines = vec![
        Line::from(Span::styled(
            format!(" Setting volume to {}% may be unsafe for hearing.", vol),
            Style::default().fg(app.theme.error).add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
        Line::from(Span::styled(
            " Are you sure you want to continue?",
            Style::default().fg(app.theme.warning),
        )),
        Line::from(""),
        Line::from(Span::styled(
            " [Enter] Yes    [Esc] Cancel",
            Style::default().fg(app.theme.fg_dim),
        )),
    ];

    let p = Paragraph::new(lines)
        .alignment(Alignment::Center)
        .style(Style::default().bg(app.theme.overlay_bg));
    f.render_widget(p, area);
}

// ─── Footer ───

fn render_footer(f: &mut ratatui::Frame, area: Rect, app: &App) {
    match app.input_mode {
        InputMode::Normal => {
            let footer = Paragraph::new(
                " [1]NP [2]Lib [3]Set | Alt+Q:Queue Alt+Y:YT Alt+F:Library Alt+Z:Sleep Alt+E:EQ | Space:P n:Next p:Prev +/-:Vol :Cmd q:Quit ",
            )
            .style(
                Style::default()
                    .fg(app.theme.fg)
                    .bg(if app.state.status == PlaybackStatus::Playing {
                        app.theme.success
                    } else {
                        app.theme.fg_dim
                    }),
            );
            f.render_widget(footer, area);
        }
        InputMode::Searching => {
            let input = Paragraph::new(format!(" Search: {}", app.search_query))
                .style(Style::default().fg(app.theme.fg).bg(app.theme.warning));
            f.render_widget(input, area);
        }
        InputMode::Command => {
            let input = Paragraph::new(format!(" :{}", app.search_query))
                .style(Style::default().fg(app.theme.fg).bg(app.theme.accent));
            f.render_widget(input, area);
        }
    }

    if let Some(ref err) = app.error_message {
        let err_text = format!(" Error: {err} ");
        let err_area = Rect {
            x: area.x,
            y: area.y.saturating_sub(1),
            width: err_text.len() as u16,
            height: 1,
        };
        let err_para =
            Paragraph::new(err_text.clone()).style(Style::default().fg(app.theme.fg_bright).bg(app.theme.error));
        f.render_widget(Clear, err_area);
        f.render_widget(err_para, err_area);
    }
}

fn render_about_overlay(f: &mut ratatui::Frame, area: Rect, app: &App) {
    let version = option_env!("CARGO_PKG_VERSION").unwrap_or("0.1.0");
    let lines = vec![
        Line::from(Span::styled(
            format!(" gtm {version}"),
            Style::default().fg(app.theme.accent).add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
        Line::from(Span::styled(
            " Copyright (C) 2026, prjctimg <prjctimg@outlook.com>",
            Style::default().fg(app.theme.fg_dim),
        )),
        Line::from(Span::styled(
            " License GPL-3.0 — This is free software.",
            Style::default().fg(app.theme.fg_dim),
        )),
        Line::from(""),
        Line::from(Span::styled(
            format!(" Status:   {:?}", app.state.status),
            Style::default().fg(app.theme.warning),
        )),
        Line::from(Span::styled(
            format!(" Volume:   {}%", app.state.volume),
            Style::default().fg(app.theme.volume_color(app.state.volume)),
        )),
        Line::from(Span::styled(
            format!(" Queue:    {} tracks", app.state.queue.len()),
            Style::default().fg(app.theme.fg_bright),
        )),
        Line::from(Span::styled(
            format!(" Shuffle:  {}", if app.state.shuffle { "ON" } else { "OFF" }),
            Style::default().fg(app.theme.fg_bright),
        )),
        Line::from(Span::styled(
            format!(" Repeat:   {:?}", app.state.repeat),
            Style::default().fg(app.theme.fg_bright),
        )),
    ];

    let p = Paragraph::new(lines)
        .alignment(Alignment::Left)
        .style(Style::default().bg(app.theme.overlay_bg));
    f.render_widget(p, area);
}

fn render_sleep_timer_overlay(f: &mut ratatui::Frame, area: Rect, app: &App) {
    let presets = [5u64, 10, 15, 30, 60];
    let sel = app.overlays.top().map_or(0, |o| o.selected.min(presets.len() - 1));

    let mut items: Vec<ListItem> = presets
        .iter()
        .enumerate()
        .map(|(i, mins)| {
            let label = if *mins == 1 { "minute" } else { "minutes" };
            let prefix = if i == sel { " \u{25B6} " } else { "   " };
            let content = format!("{prefix}{} {}", mins, label);
            let style = if i == sel {
                Style::default().fg(app.theme.selection_fg).bg(app.theme.selection_bg)
            } else {
                Style::default()
            };
            ListItem::new(content).style(style)
        })
        .collect();

    if let Some(remaining) = app.sleep_timer_remaining {
        let status = format!(" Active: {} min remaining", remaining);
        items.push(ListItem::new(status).style(Style::default().fg(app.theme.success)));
        items.push(ListItem::new(" [Esc] Cancel timer").style(Style::default().fg(app.theme.fg_dim)));
    }

    let list = List::new(items).block(
        Block::default()
            .borders(Borders::ALL)
            .title(" Sleep Timer ")
            .border_type(BorderType::Rounded),
    );
    f.render_widget(list, area);
}

fn render_command_palette_overlay(f: &mut ratatui::Frame, area: Rect, app: &App) {
    let commands = [
        "Play/Pause   [Space]",
        "Next Track   [n]",
        "Prev Track   [p]",
        "Volume Up    [+]",
        "Volume Down  [-]",
        "Mute Toggle  [m]",
        "Repeat       [r]",
        "Shuffle      [h]",
        "Quit         [q]",
        "Tab Cycle    [Tab]",
        "NowPlaying   [1]",
        "Library      [2]",
        "Settings     [3]",
        "Search       [/]",
        "Command      [:]",
        "Queue O/L    [Alt+Q]",
        "YouTube O/L  [Alt+Y]",
        "Library O/L  [Alt+F]",
        "EQ O/L       [Alt+E]",
        "SleepTimer   [Alt+Z]",
        "ThemePicker  [Alt+T]",
        "Sound FX O/L [Alt+X]",
        "About O/L    [Alt+A]",
        "Spotify O/L  [Alt+S]",
        "Cmd Palette  [Alt+P]",
    ];

    let query = app.overlays.top().map_or(String::new(), |o| o.query.clone());
    let q = query.to_lowercase();
    let filtered: Vec<&&str> = if q.is_empty() {
        commands.iter().collect()
    } else {
        commands.iter().filter(|c| c.to_lowercase().contains(&q)).collect()
    };

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(3), Constraint::Min(0)])
        .split(area);

    let search_input = Paragraph::new(format!(" Filter: {}", query))
        .style(Style::default().fg(app.theme.fg).bg(app.theme.warning));
    f.render_widget(search_input, chunks[0]);

    let sel = app.overlays.top().map_or(0, |o| o.selected.min(filtered.len().saturating_sub(1)));
    let list_items: Vec<ListItem> = filtered
        .iter()
        .enumerate()
        .map(|(i, cmd)| {
            let prefix = if i == sel { " \u{25B6} " } else { "   " };
            let style = if i == sel {
                Style::default().fg(app.theme.selection_fg).bg(app.theme.selection_bg)
            } else {
                Style::default()
            };
            ListItem::new(format!("{prefix}{}", cmd)).style(style)
        })
        .collect();

    let list = List::new(list_items).block(
        Block::default()
            .borders(Borders::ALL)
            .title(" Commands ")
            .border_type(BorderType::Rounded),
    );
    f.render_widget(list, chunks[1]);
}

fn render_equalizer_overlay(f: &mut ratatui::Frame, area: Rect, app: &App) {
    let presets = [
        ("Flat",      EqPreset::Flat),
        ("Pop",       EqPreset::Pop),
        ("Rock",      EqPreset::Rock),
        ("Jazz",      EqPreset::Jazz),
        ("Classical", EqPreset::Classical),
        ("Bass",      EqPreset::Bass),
        ("Vocal",     EqPreset::Vocal),
    ];

    let sel = app.overlays.top().map_or(0, |o| o.selected.min(presets.len() - 1));

    let list_items: Vec<ListItem> = presets.iter().enumerate().map(|(i, (name, _))| {
        let prefix = if i == sel { " \u{25B6} " } else { "   " };
        let style = if i == sel {
            Style::default().fg(app.theme.selection_fg).bg(app.theme.selection_bg)
        } else if *name == app.state.eq_preset.label() {
            Style::default().fg(app.theme.success)
        } else {
            Style::default()
        };
        ListItem::new(format!("{prefix}{}", name)).style(style)
    }).collect();

    let graph = render_eq_graph();
    let all_items: Vec<ListItem> = list_items.into_iter().chain(
        std::iter::once(ListItem::new(""))
            .chain(std::iter::once(ListItem::new(graph).style(Style::default().fg(app.theme.accent))))
    ).collect();

    let list = List::new(all_items).block(
        Block::default()
            .borders(Borders::ALL)
            .title(" Equalizer ")
            .border_type(BorderType::Rounded),
    );
    f.render_widget(list, area);
}

fn render_eq_graph() -> String {
    let bands = ["31", "62", "125", "250", "500", "1k", "2k", "4k", "8k", "16k"];
    let bar_height = 5usize;
    let mut lines = Vec::new();
    for row in (0..=bar_height).rev() {
        let mut line = String::new();
        for _ in bands.iter() {
            if row == 0 {
                line.push_str(" ├─┤ ");
            } else if row == bar_height / 2 {
                line.push_str(" ─── ");
            } else {
                line.push_str("     ");
            }
        }
        lines.push(line);
    }
    lines.push(bands.join(" "));
    lines.join("\n")
}

fn render_sound_effects_overlay(f: &mut ratatui::Frame, area: Rect, app: &App) {
    let crossfade_on = app.state.crossfade.as_ref().map(|c| c.enabled).unwrap_or(false);
    let crossfade_dur = app.state.crossfade.as_ref().map(|c| c.duration_secs).unwrap_or(0);

    let items = vec![
        format!("Playback Speed:  {:.1}x", app.playback_speed),
        format!("Reverb:          Off"),
        format!("Crossfade:       {}", if crossfade_on { "ON" } else { "OFF" }),
        format!("Crossfade Dur:   {}s", crossfade_dur),
        format!("EQ Preset:       {}", app.state.eq_preset.label()),
    ];

    let sel = app.overlays.top().map_or(0, |o| o.selected.min(items.len() - 1));
    let list_items: Vec<ListItem> = items.iter().enumerate().map(|(i, s)| {
        let prefix = if i == sel { " \u{25B6} " } else { "   " };
        let style = if i == sel {
            Style::default().fg(app.theme.selection_fg).bg(app.theme.selection_bg)
        } else {
            Style::default()
        };
        ListItem::new(format!("{prefix}{}", s)).style(style)
    }).collect();

    let list = List::new(list_items).block(
        Block::default()
            .borders(Borders::ALL)
            .title(" Sound Effects ")
            .border_type(BorderType::Rounded),
    );
    f.render_widget(list, area);
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

// ─── Aesthetic Helpers ───

/// Braille spinner frames for loading states.
const SPINNER_FRAMES: &[char] = &['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏'];

/// Return a braille spinner character cycling by `frame` (incremented each tick).
#[allow(dead_code)]
pub fn braille_spinner(frame: usize) -> char {
    SPINNER_FRAMES[frame % SPINNER_FRAMES.len()]
}

/// Build a single-line progress bar string using unicode block characters
/// with an oscillating head at the fill position.
/// `ratio` in [0.0, 1.0]; `width` in terminal columns.
fn render_progress_line(ratio: f64, width: usize) -> String {
    let width = width.max(10); // never narrower than 10
    let filled = (ratio.clamp(0.0, 1.0) * width as f64).round() as usize;

    let head = if ratio > 0.0 && ratio < 1.0 { '●' } else { ' ' };

    let mut line = String::with_capacity(width);
    for i in 0..width {
        if i < filled.saturating_sub(1) {
            line.push('█');
        } else if i == filled.saturating_sub(1) && filled > 0 && filled < width {
            line.push(head);
        } else if i < width {
            line.push('░');
        }
    }
    line
}

/// Nerd-font volume icon with emoji fallback.
fn volume_icon(volume: u8) -> &'static str {
    match volume {
        0 => "\u{1F507}",   // muted
        1..=33 => "\u{1F509}", // low
        34..=66 => "\u{1F50A}", // medium
        _ => "\u{1F50A}",   // high
    }
}
