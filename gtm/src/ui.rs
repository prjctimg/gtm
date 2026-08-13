// Copyright (c) 2025 - present
// Author: prjctimg <prjctimg@outlook.com>
// TUI rendering: tab layout, overlays, library, now-playing, settings
//
// This is free software released under the GPL-3.0 license.

use std::path::PathBuf;

use chrono::Timelike;
use crossterm::terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen};
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Clear, List, ListItem, Paragraph};
use ratatui::Terminal;
use ratatui::style::Color;
use ratatui_image::StatefulImage;
use ratatui_image::protocol::StatefulProtocol;
use crate::app::{App, InputMode, LIBRARY_CATEGORIES};
use crate::overlay::OverlayId;
use crate::theme::THEMES;
use gtm_core::state::{EqPreset, Tab};

pub fn run_tui(socket: Option<String>) -> Result<(), Box<dyn std::error::Error>> {
    let socket_path = socket
        .map(PathBuf::from)
        .unwrap_or_else(gtm_core::default_socket_path);

    ensure_daemon_running(&socket_path)?;

    // Redirect stderr to log file so diagnostic messages don't break the TUI
    let _original_stderr = gtm_core::log::redirect_stderr_to_log();

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

    if let Some(parent) = socket_path.parent() {
        let _ = std::fs::create_dir_all(parent);
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
    // Explicit background fill — the TUI defines its own background
    f.render_widget(
        ratatui::widgets::Block::default()
            .style(ratatui::style::Style::default().bg(app.theme.bg)),
        area,
    );
    // help bar shows on Library tab, hidden during overlays
    let show_help = app.current_tab == Tab::Library && !app.overlays.is_open();
    let help_height: u16 = if show_help { 1 } else { 0 };
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Min(0),
            Constraint::Length(help_height),
            Constraint::Length(1),
        ])
        .split(area);

    render_notifications(f, chunks[0], app);
    render_content(f, chunks[1], app);
    if show_help {
        render_help_bar(f, chunks[2], app);
    }
    render_footer(f, chunks[3], app);

    // Track info popup on Library tab
    if app.current_tab == Tab::Library && app.track_popup_visible && !app.overlays.is_open() {
        render_track_popup(f, chunks[1], app);
    }

    // Render overlays on top of everything
    if app.overlays.is_open() {
        render_overlay(f, area, app);
    }


}

fn render_notifications(f: &mut ratatui::Frame, area: Rect, app: &App) {
    if let Some(n) = app.notifications.last() {
        let color = match n.kind {
            crate::app::NotificationKind::Info => app.theme.accent,
            crate::app::NotificationKind::Success => app.theme.success,
            crate::app::NotificationKind::Warning => app.theme.warning,
            crate::app::NotificationKind::Error => app.theme.error,
        };
        let text = format!(" {} ", n.message);
        let para = Paragraph::new(text)
            .style(Style::default().fg(app.theme.fg_bright).bg(color));
        f.render_widget(para, area);
    }
}

// ─── Content Area ───

fn render_content(f: &mut ratatui::Frame, area: Rect, app: &mut App) {
    if !app.is_ready {
        let loading = Paragraph::new(" Loading library…")
            .alignment(Alignment::Center)
            .style(Style::default().fg(app.theme.fg_dim));
        f.render_widget(loading, area);
        return;
    }
    match app.current_tab {
        Tab::Library => render_library(f, area, app),
        Tab::Settings => render_settings(f, area, app),
    }
}

fn render_help_bar(f: &mut ratatui::Frame, area: Rect, app: &App) {
    let text = " [Space] Play/Pause  [n/N] Next  [p/P] Prev  [+/-] Volume  [s] Stop  [:] Command Palette  [q] Quit ";
    let para = Paragraph::new(text)
        .style(Style::default().fg(app.theme.fg_dim));
    f.render_widget(para, area);
}

fn render_cover(
    f: &mut ratatui::Frame,
    area: Rect,
    cover_stateful: Option<&mut StatefulProtocol>,
    current_cover: Option<&[u8]>,
    fg: Color,
) {
    // Skip image rendering in terminals without protocol passthrough
    if std::env::var("NVIM").is_ok() || std::env::var("ZELLIJ").is_ok() {
        let terminal_name = if std::env::var("ZELLIJ").is_ok() { "Zellij" } else { "Neovim" };
        let placeholder = Block::default()
            .borders(Borders::ALL)
            .title(format!(" Cover art unavailable in {terminal_name} "))
            .style(Style::default().fg(fg));
        f.render_widget(placeholder, area);
        return;
    }
    if let Some(protocol) = cover_stateful {
        let image = StatefulImage::new();
        f.render_stateful_widget(image, area, protocol);
    } else if let Some(cover_bytes) = current_cover {
        render_cover_block(f, area, cover_bytes);
    } else {
        let placeholder = Block::default()
            .borders(Borders::ALL)
            .style(Style::default().fg(fg));
        f.render_widget(placeholder, area);
    }
}

fn render_cover_block(f: &mut ratatui::Frame, area: Rect, cover_bytes: &[u8]) {
    let img = match image::load_from_memory(cover_bytes) {
        Ok(img) => img.into_rgba8(),
        Err(_) => return,
    };
    let disp_w = (area.width as u32).max(1);
    let disp_h = (area.height as u32 * 2).max(1);
    let thumb = image::imageops::resize(&img, disp_w, disp_h, image::imageops::FilterType::CatmullRom);
    for y in 0..area.height as u32 {
        let mut spans = Vec::with_capacity(disp_w as usize);
        for x in 0..disp_w {
            let top = thumb.get_pixel(x, y * 2);
            let bot = if y * 2 + 1 < disp_h {
                *thumb.get_pixel(x, y * 2 + 1)
            } else {
                image::Rgba([0, 0, 0, 255])
            };
            let fg = ratatui::style::Color::Rgb(top[0], top[1], top[2]);
            let bg = ratatui::style::Color::Rgb(bot[0], bot[1], bot[2]);
            spans.push(Span::styled("\u{2580}", Style::default().fg(fg).bg(bg)));
        }
        let row = Rect {
            x: area.x,
            y: area.y + y as u16,
            width: area.width,
            height: 1,
        };
        f.render_widget(Paragraph::new(Line::from(spans)), row);
    }
}

const LIBRARY_ICONS_NERD: &[&str] = &[
    "\u{f001}", "\u{f004}", "\u{f025}", "\u{f007}",
    "\u{f03a}", "\u{f1bc}", "\u{f019}",
];

const LIBRARY_ICONS_ASCII: &[&str] = &["♫", "♥", "▤", "♪", "≡", "☊", "↓"];

fn use_nerd_fonts() -> bool {
    match std::env::var("GTM_NERD_FONTS") {
        Ok(v) if v == "0" || v == "false" || v == "no" => false,
        _ => true,
    }
}

/// Scroll helper that keeps the selected item centered in the viewport.
fn centered_scroll(sel: usize, available: usize, total: usize) -> (usize, usize) {
    if total <= available {
        return (0, total);
    }
    let half = available / 2;
    let scroll = if sel <= half {
        0
    } else if sel >= total.saturating_sub(available - half) {
        total.saturating_sub(available)
    } else {
        sel.saturating_sub(half)
    };
    let end = (scroll + available).min(total);
    (scroll, end)
}

fn render_library(f: &mut ratatui::Frame, area: Rect, app: &mut App) {
    let version = option_env!("CARGO_PKG_VERSION").unwrap_or("0.1.0");
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(8), Constraint::Min(1)])
        .split(area);

    let np_area = chunks[0];

    let panes = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Length(28), Constraint::Min(0)])
        .split(chunks[1]);

    let left_focus = app.library_pane_focus;

    // ── Now Playing section ──
    {
        let np_block = Block::default()
            .borders(Borders::ALL)
            .title(format!(" Now Playing  ·  gtm {} ", version))
            .border_type(BorderType::Plain)
            .border_style(Style::default().fg(app.theme.border));

        if app.state.current_track.is_none() {
            let inner = np_block.inner(np_area);
            f.render_widget(np_block, np_area);
            let msg = Paragraph::new("No track playing")
                .alignment(Alignment::Center)
                .style(Style::default().fg(app.theme.fg_dim));
            f.render_widget(msg, inner);
        } else {
            let inner = np_block.inner(np_area);
            f.render_widget(np_block, np_area);

            const COVER_W: u16 = 12;
            const COVER_H: u16 = 6;

            if inner.width >= COVER_W + 4 {
                // Cover on the LEFT, info on the RIGHT
                let hchunks = Layout::default()
                    .direction(Direction::Horizontal)
                    .constraints([Constraint::Length(COVER_W), Constraint::Min(0)])
                    .split(inner);

                let cover_area = Rect {
                    x: hchunks[0].x,
                    y: hchunks[0].y,
                    width: hchunks[0].width,
                    height: COVER_H.min(hchunks[0].height),
                };
                render_cover(
                    f,
                    cover_area,
                    app.cover_stateful.as_mut(),
                    app.current_cover.as_deref(),
                    app.theme.fg,
                );

                let info_area = hchunks[0];
                let track = app.state.current_track.as_ref().unwrap();

                let info_chunks = Layout::default()
                    .direction(Direction::Vertical)
                    .constraints([
                        Constraint::Length(1),
                        Constraint::Length(1),
                        Constraint::Length(1),
                        Constraint::Length(1),
                        Constraint::Min(0),
                    ])
                    .split(info_area);

                // Row 0: animated title (fav icon + artist — title)
                let display_title = if track.title.is_empty() {
                    std::path::Path::new(&track.path)
                        .file_stem()
                        .map(|s| s.to_string_lossy().to_string())
                        .unwrap_or_default()
                } else {
                    track.title.clone()
                };
                let display_artist = if track.artist.is_empty() { "Unknown" } else { &track.artist };
                let fav_prefix = if track.favourite { "\u{2665} " } else { "" };
                let title_text = format!("{}{} \u{2014} {}", fav_prefix, display_artist, display_title);
                let title_avail = info_chunks[0].width as usize;
                let animated_title = scroll_text(&title_text, title_avail, app.np_title_scroll, true);
                let title_para = Paragraph::new(Line::from(vec![
                    Span::styled(&animated_title, Style::default().fg(app.theme.fg_bright).add_modifier(Modifier::BOLD)),
                ]));
                f.render_widget(title_para, info_chunks[0]);

                // Row 1: Artist
                let artist_para = Paragraph::new(Line::from(vec![
                    Span::styled("Artist: ", Style::default().fg(app.theme.fg)),
                    Span::styled(display_artist, Style::default().fg(app.theme.fg_bright)),
                ]));
                f.render_widget(artist_para, info_chunks[1]);

                // Row 2: Album
                let display_album = if track.album.is_empty() { "Unknown" } else { &track.album };
                let album_para = Paragraph::new(Line::from(vec![
                    Span::styled("Album: ", Style::default().fg(app.theme.fg)),
                    Span::styled(display_album, Style::default().fg(app.theme.fg_bright)),
                ]));
                f.render_widget(album_para, info_chunks[2]);

                // Row 3: Progress bar + timestamps
                let dur = track.duration;
                let pos = app.display_position;
                let pos_str = format_duration(pos as u64);
                let dur_str = format_duration(dur as u64);
                let ratio = if dur > 0.0 { (pos / dur) as f64 } else { 0.0 };
                let ts_str = format!("{} / {}", pos_str, dur_str);
                let bar_width = (info_chunks[3].width.saturating_sub(ts_str.len() as u16 + 2) as usize).min(40);
                let bar = render_progress_variant(ratio, bar_width, app);
                let bar_with_ts = format!("{} {}", bar, ts_str);
                let bar_para = Paragraph::new(bar_with_ts)
                    .style(Style::default().fg(app.theme.accent));
                f.render_widget(bar_para, info_chunks[3]);
            } else {
                // Terminal too narrow for cover — just show info text
                let track = app.state.current_track.as_ref().unwrap();
                let display_title = if track.title.is_empty() {
                    std::path::Path::new(&track.path)
                        .file_stem()
                        .map(|s| s.to_string_lossy().to_string())
                        .unwrap_or_default()
                } else {
                    track.title.clone()
                };
                let display_artist = if track.artist.is_empty() { "Unknown" } else { &track.artist };
                let fav_prefix = if track.favourite { "\u{2665} " } else { "" };
                let title_text = format!("{}{} \u{2014} {}", fav_prefix, display_artist, display_title);
                let title_avail = inner.width as usize;
                let animated_title = scroll_text(&title_text, title_avail, app.np_title_scroll, true);
                let title_para = Paragraph::new(Line::from(vec![
                    Span::styled(&animated_title, Style::default().fg(app.theme.fg_bright).add_modifier(Modifier::BOLD)),
                ]));
                f.render_widget(title_para, Rect { x: inner.x, y: inner.y, width: inner.width, height: 1 });

                let display_album = if track.album.is_empty() { "Unknown" } else { &track.album };
                let album_para = Paragraph::new(Line::from(vec![
                    Span::styled("Album: ", Style::default().fg(app.theme.fg)),
                    Span::styled(display_album, Style::default().fg(app.theme.fg_bright)),
                ]));
                let album_area = Rect { x: inner.x, y: inner.y + 1, width: inner.width, height: 1 };
                f.render_widget(album_para, album_area);

                let dur = track.duration;
                let pos = app.display_position;
                let pos_str = format_duration(pos as u64);
                let dur_str = format_duration(dur as u64);
                let ratio = if dur > 0.0 { (pos / dur) as f64 } else { 0.0 };
                let ts_str = format!("{} / {}", pos_str, dur_str);
                let bar_width = (inner.width.saturating_sub(ts_str.len() as u16 + 2) as usize).min(40);
                let bar = render_progress_variant(ratio, bar_width, app);
                let bar_with_ts = format!("{} {}", bar, ts_str);
                let bar_para = Paragraph::new(bar_with_ts)
                    .style(Style::default().fg(app.theme.accent));
                let bar_area = Rect { x: inner.x, y: inner.y + 2, width: inner.width, height: 1 };
                f.render_widget(bar_para, bar_area);
            }
        }
    }

    // ── Left pane: categories with icons ──
    let lib_icons = if use_nerd_fonts() { LIBRARY_ICONS_NERD } else { LIBRARY_ICONS_ASCII };
    let left_items: Vec<ListItem> = LIBRARY_CATEGORIES
        .iter()
        .enumerate()
        .map(|(i, cat)| {
            let icon = lib_icons.get(i).unwrap_or(&" ");
            let count = match *cat {
                "All Tracks" => app.tracks_cache.len(),
                "Albums" => app.unique_albums().len(),
                "Artists" => app.unique_artists().len(),
                "Playlists" => app.playlist_cache.len(),
                _ => 0,
            };
            // Skip icon for "Liked" — the heart is redundant with the name
            let display_icon = if *cat == "Liked" { " " } else { *icon };
            let label = if count > 0 {
                format!(" {display_icon}  {:<14} {:>4}", cat, count)
            } else {
                format!(" {display_icon}  {}", cat)
            };
            let is_active = i == app.library_category;
            let style = if is_active && left_focus {
                Style::default().fg(app.theme.selection_fg).bg(app.theme.selection_bg)
            } else if is_active {
                Style::default().fg(app.theme.accent)
            } else {
                Style::default().fg(app.theme.fg)
            };
            ListItem::new(label).style(style)
        })
        .collect();

    let left_block = Block::default()
        .borders(Borders::ALL)
        .title(" Library ")
        .border_type(BorderType::Plain)
        .border_style(if left_focus {
            Style::default().fg(app.theme.border_active)
        } else {
            Style::default().fg(app.theme.border)
        });

    let left_inner = left_block.inner(panes[0]);
    f.render_widget(List::new(left_items).block(left_block), panes[0]);

    // Active category left-border indicator overlay
    if app.library_category < LIBRARY_CATEGORIES.len() {
        let indicator_y = left_inner.y + app.library_category as u16;
        if indicator_y < left_inner.y + left_inner.height {
            let indicator_area = Rect { x: left_inner.x + 1, y: indicator_y, width: 1, height: 1 };
            let indicator = Paragraph::new("▎")
                .style(Style::default().fg(app.theme.sidebar_active_border));
            f.render_widget(indicator, indicator_area);
        }
    }

    // ── Stats at bottom of left pane ──
    let category_label = LIBRARY_CATEGORIES.get(app.library_category).unwrap_or(&"All Tracks");

    let (right_lines, stats_line) = if app.browse_detail.is_some() {
        // Detail view: show tracks filtered by album/artist/playlist
        let (total_len, hours, mins) = {
            let f = app.filtered_tracks();
            let total_dur: u64 = f.iter().map(|t| t.duration as u64).sum();
            (f.len(), total_dur / 3600, (total_dur % 3600) / 60)
        };
        let st_line = format!(" {} tracks | {}h {}m ", total_len, hours, mins);

        let reserve = 3usize;
        let available = panes[1].height.saturating_sub(reserve as u16) as usize;
        app.viewport_items = available;
        let sel = app.scroll_offset.min(total_len.saturating_sub(1));
        let (list_scroll, end) = centered_scroll(sel, available, total_len);
        app.list_scroll = list_scroll;

        let filtered = app.filtered_tracks();

        let pane_w = panes[1].width as usize;
        let mut lines = vec![
            Line::from(""),
        ];
        for (i, track) in filtered[app.list_scroll..end].iter().enumerate() {
            let real_i = app.list_scroll + i;
            let is_current = app.state.current_track.as_ref().map(|t| t.id) == Some(track.id);
            let is_sel = real_i == sel && !left_focus;
            let label = if track.artist.is_empty() { track.title.clone() } else { format!("{}  {}", track.artist, track.title) };
            let avail = pane_w.saturating_sub(2);
            let display_label = scroll_text(&label, avail, app.footer_title_scroll, is_sel);
            let prefix = if is_current { "> " } else if is_sel { "  " } else { "  " };
            let row = format!("{}{}", prefix, display_label);
            let style = if is_current {
                Style::default().fg(app.theme.accent).add_modifier(Modifier::BOLD)
            } else if is_sel {
                Style::default().fg(app.theme.selection_fg).bg(app.theme.selection_bg)
            } else {
                Style::default()
            };
            lines.push(Line::from(Span::styled(row, style)));
        }
        (lines, st_line)
    } else if app.library_category == 2 {
        // Albums browse
        let albums = app.unique_albums();
        let total_len = albums.len();
        let sel = app.scroll_offset.min(total_len.saturating_sub(1));
        let st_line = format!(" {} albums ", total_len);
        let reserve = 3usize;
        let available = panes[1].height.saturating_sub(reserve as u16) as usize;
        app.viewport_items = available;
        let (list_scroll, end) = centered_scroll(sel, available, total_len);
        app.list_scroll = list_scroll;
        let mut lines = vec![
            Line::from(""),
        ];
        for (i, (name, count)) in albums[app.list_scroll..end].iter().enumerate() {
            let real_i = app.list_scroll + i;
            let prefix = if real_i == sel && !left_focus { " >" } else { "  " };
            let style = if real_i == sel && !left_focus {
                Style::default().fg(app.theme.selection_fg).bg(app.theme.selection_bg)
            } else {
                Style::default().fg(app.theme.fg)
            };
            lines.push(Line::from(Span::styled(format!("{}{:<40} {:>4} tracks", prefix, name, count), style)));
        }
        (lines, st_line)
    } else if app.library_category == 3 {
        // Artists browse
        let artists = app.unique_artists();
        let total_len = artists.len();
        let sel = app.scroll_offset.min(total_len.saturating_sub(1));
        let st_line = format!(" {} artists ", total_len);
        let reserve = 3usize;
        let available = panes[1].height.saturating_sub(reserve as u16) as usize;
        app.viewport_items = available;
        let (list_scroll, end) = centered_scroll(sel, available, total_len);
        app.list_scroll = list_scroll;
        let mut lines = vec![
            Line::from(""),
        ];
        for (i, (name, count)) in artists[app.list_scroll..end].iter().enumerate() {
            let real_i = app.list_scroll + i;
            let prefix = if real_i == sel && !left_focus { " >" } else { "  " };
            let style = if real_i == sel && !left_focus {
                Style::default().fg(app.theme.selection_fg).bg(app.theme.selection_bg)
            } else {
                Style::default().fg(app.theme.fg)
            };
            lines.push(Line::from(Span::styled(format!("{}{:<40} {:>4} tracks", prefix, name, count), style)));
        }
        (lines, st_line)
    } else if app.library_category == 4 {
        // Playlists browse
        let playlists = &app.playlist_cache;
        let total_len = playlists.len();
        let sel = app.scroll_offset.min(total_len.saturating_sub(1));
        let st_line = format!(" {} playlists ", total_len);
        let reserve = 3usize;
        let available = panes[1].height.saturating_sub(reserve as u16) as usize;
        app.viewport_items = available;
        let (list_scroll, end) = centered_scroll(sel, available, total_len);
        app.list_scroll = list_scroll;
        let mut lines = vec![
            Line::from(""),
        ];
        for (i, pl) in playlists[app.list_scroll..end].iter().enumerate() {
            let real_i = app.list_scroll + i;
            let prefix = if real_i == sel && !left_focus { " >" } else { "  " };
            let style = if real_i == sel && !left_focus {
                Style::default().fg(app.theme.selection_fg).bg(app.theme.selection_bg)
            } else {
                Style::default().fg(app.theme.fg)
            };
            lines.push(Line::from(Span::styled(format!("{}{:<40} {:>4} tracks", prefix, pl.name, pl.track_count), style)));
        }
        (lines, st_line)
    } else {
        // All Tracks or other: flat track table
        let (total_len, total_dur) = {
            let f = app.filtered_tracks();
            let dur: u64 = f.iter().map(|t| t.duration as u64).sum();
            (f.len(), dur)
        };
        let hours = total_dur / 3600;
        let mins = (total_dur % 3600) / 60;
        let st_line = format!(" {} tracks | {}h {}m ", total_len, hours, mins);

        let reserve = 3usize;
        let available = panes[1].height.saturating_sub(reserve as u16) as usize;
        app.viewport_items = available;
        let sel = app.scroll_offset.min(total_len.saturating_sub(1));
        let (list_scroll, end) = centered_scroll(sel, available, total_len);
        app.list_scroll = list_scroll;

        let filtered = app.filtered_tracks();
        let pane_w = panes[1].width as usize;

        let mut lines = vec![
            Line::from(""),
        ];
        for (i, track) in filtered[app.list_scroll..end].iter().enumerate() {
            let real_i = app.list_scroll + i;
            let is_current = app.state.current_track.as_ref().map(|t| t.id) == Some(track.id);
            let is_sel = real_i == sel && !left_focus;
            let label = if track.artist.is_empty() { track.title.clone() } else { format!("{}  {}", track.artist, track.title) };
            let avail = pane_w.saturating_sub(2);
            let display_label = scroll_text(&label, avail, app.footer_title_scroll, is_sel);
            let prefix = if is_current { "> " } else if is_sel { "  " } else { "  " };
            let row = format!("{}{}", prefix, display_label);
            let style = if is_current {
                Style::default().fg(app.theme.accent).add_modifier(Modifier::BOLD)
            } else if is_sel {
                Style::default().fg(app.theme.selection_fg).bg(app.theme.selection_bg)
            } else {
                Style::default()
            };
            lines.push(Line::from(Span::styled(row, style)));
        }
        (lines, st_line)
    };

    // Render stats at the bottom of the left pane
    if left_inner.height > 0 {
        let stats_y = left_inner.y + left_inner.height - 1;
        let stats_area = Rect {
            x: left_inner.x,
            y: stats_y,
            width: left_inner.width,
            height: 1,
        };
        let stats = Paragraph::new(Line::from(vec![
            Span::styled(stats_line.trim(), Style::default().fg(app.theme.fg_dim)),
        ]));
        f.render_widget(stats, stats_area);
    }

    let right_para = Paragraph::new(right_lines);
    let right_block = Block::default()
        .borders(Borders::ALL)
        .title(format!(" {} ", category_label))
        .border_type(BorderType::Plain)
        .border_style(if !left_focus {
            Style::default().fg(app.theme.border_active)
        } else {
            Style::default().fg(app.theme.border)
        });

    let inner = right_block.inner(panes[1]);
    f.render_widget(right_block, panes[1]);
    f.render_widget(right_para, inner);
}

const SETTINGS_ICONS_NERD: &[&str] = &["\u{f028}", "\u{f16a}", "\u{f04b}", "\u{f013}", "\u{f1bc}"];
const SETTINGS_ICONS_ASCII: &[&str] = &["♪", "YT", "▶", "⚙", "★"];
const SETTINGS_CATEGORIES: &[&str] = &["Audio", "YouTube", "Playback", "System", "Spotify"];

fn render_settings(f: &mut ratatui::Frame, area: Rect, app: &App) {
    let panes = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Length(22), Constraint::Min(0)])
        .split(area);

    // ── Left pane: categories with icons ──
    let settings_icons = if use_nerd_fonts() { SETTINGS_ICONS_NERD } else { SETTINGS_ICONS_ASCII };
    let settings_focus = app.settings_pane_focus;
    let left_items: Vec<ListItem> = SETTINGS_CATEGORIES
        .iter()
        .enumerate()
        .map(|(i, cat)| {
            let icon = settings_icons.get(i).unwrap_or(&" ");
            let is_active = i == app.settings_category;
            let style = if is_active && settings_focus {
                Style::default().fg(app.theme.selection_fg).bg(app.theme.selection_bg)
            } else if is_active {
                Style::default().fg(app.theme.accent)
            } else {
                Style::default().fg(app.theme.fg)
            };
            ListItem::new(format!(" {} {}", icon, cat)).style(style)
        })
        .collect();

    let left_block = Block::default()
        .borders(Borders::ALL)
        .title(" Settings ")
        .border_type(BorderType::Plain);

    let left_inner = left_block.inner(panes[0]);
    f.render_widget(List::new(left_items).block(left_block), panes[0]);

    // Active category left-border indicator
    if app.settings_category < SETTINGS_CATEGORIES.len() {
        let indicator_y = left_inner.y + app.settings_category as u16;
        if indicator_y < left_inner.y + left_inner.height {
            let indicator_area = Rect { x: left_inner.x + 1, y: indicator_y, width: 1, height: 1 };
            let indicator = Paragraph::new("▎")
                .style(Style::default().fg(app.theme.sidebar_active_border));
            f.render_widget(indicator, indicator_area);
        }
    }

    // ── Right pane: options for selected category ──
    let items: Vec<String> = match app.settings_category {
        0 => vec![
            format!("Volume          [ {:>3}%  ]", app.state.volume),
            format!("Mute            [ {} ]", if app.state.mute { "●   On " } else { "○   Off" }),
        ],
        1 => vec![
            format!("Cookie Source   [ chromium   ▶ ]"),
            format!("Cookie File     [ (none)     ▶ ]"),
            format!("JS Runtime      [ deno       ▶ ]"),
            format!("Max Downloads   [{:-<13}]  3", "█".repeat(9)),
            format!("Results/Page    10"),
            format!("Search History  [ 0 entries ▶ ]"),
            format!("Auto Download   [ ● ]  On"),
            format!("Clear History   [Clear]"),
        ],
         2 => {
            let crossfade_on = app.state.crossfade.as_ref().map(|c| c.enabled).unwrap_or(false);
            let crossfade_dur = app.state.crossfade.as_ref().map(|c| c.duration_secs).unwrap_or(0);
            let easing = app.state.crossfade.as_ref().map(|c| format!("{:?}", c.easing)).unwrap_or_else(|| "N/A".into());
            vec![
                format!("Repeat          [ {:?}       ▶ ]", app.state.repeat),
                format!("Shuffle         [ {} ]", if app.state.shuffle { "●   On " } else { "○   Off" }),
                if crossfade_on {
                    format!("Crossfade       [ ● ]  On  {}s", crossfade_dur)
                } else {
                    "Crossfade       [ ○ ]  Off".to_string()
                },
                format!("Easing          [ {}   ▶ ]", easing),
                format!("EQ Enabled      [ {} ]", if app.state.eq_enabled { "●   On " } else { "○   Off" }),
            ]
        }
        3 => {
            let preset_name = crate::footer::presets().get(app.footer_preset).map(|p| p.name).unwrap_or("Default");
            vec![
                "Theme           [ Cyberdeck  ▶ ]".to_string(),
                format!("Transparent BG  [ {} ]", if app.transparent_bg { "●" } else { "○" }),
                "Sync Covers     [ Enter  ▶ ]".to_string(),
                format!("Footer Preset   [ {:>8} ▶ ]", preset_name),
            ]
        },
        4 => vec!["Spotify Status  [ Disconnected ▶ ]".to_string()],
        _ => vec![],
    };

    let category_label = SETTINGS_CATEGORIES.get(app.settings_category).unwrap_or(&"");
    let right_block = Block::default()
        .borders(Borders::ALL)
        .title(format!(" {} ", category_label))
        .border_type(BorderType::Plain);

    let inner = right_block.inner(panes[1]);
    f.render_widget(right_block, panes[1]);

    let mut lines = Vec::new();
    let sel = app.settings_option;
    for (i, item) in items.iter().enumerate() {
        if i == 1 && app.settings_category == 1 {
            lines.push(Line::from(Span::styled(
                " ──────────────── YouTube ─────────────────",
                Style::default().fg(app.theme.accent),
            )));
        }
        let is_sel = i == sel && !settings_focus;
        let style = if is_sel {
            Style::default().fg(app.theme.selection_fg).bg(app.theme.selection_bg)
        } else {
            Style::default().fg(app.theme.fg)
        };
        lines.push(Line::from(Span::styled(item, style)));
    }
    lines.push(Line::from(""));
    match (app.settings_category, sel) {
        (0, 0) => lines.push(Line::from(Span::styled(" Volume: Use +/- keys to adjust playback volume.", Style::default().fg(app.theme.fg)))),
        (0, 1) => lines.push(Line::from(Span::styled(" Mute: Press Enter to toggle mute on/off.", Style::default().fg(app.theme.fg)))),
        (1, _) => lines.push(Line::from(Span::styled(" YouTube: Configure JS runtime, download limits & search preferences.", Style::default().fg(app.theme.fg)))),
        (2, 0) => lines.push(Line::from(Span::styled(format!(" Repeat: Press Enter to cycle (current: {:?}).", app.state.repeat), Style::default().fg(app.theme.fg)))),
        (2, 1) => lines.push(Line::from(Span::styled(" Shuffle: Press Enter to toggle shuffle on/off.", Style::default().fg(app.theme.fg)))),
        (2, 2) => {
            let cf_on = app.state.crossfade.as_ref().map(|c| c.enabled).unwrap_or(false);
            lines.push(Line::from(Span::styled(if cf_on { " Crossfade: On. Press Enter to toggle off or use C to change duration." } else { " Crossfade: Off. Press Enter to toggle on." }, Style::default().fg(app.theme.fg))));
        }
        (2, 3) => {
            let easing = app.state.crossfade.as_ref().map(|c| format!("{:?}", c.easing)).unwrap_or_else(|| "N/A".into());
            lines.push(Line::from(Span::styled(format!(" Easing: Press Enter to cycle (current: {}). Controls crossfade volume curve.", easing), Style::default().fg(app.theme.fg))));
        }
        (2, 4) => {
            let eq_on = app.state.eq_enabled;
            lines.push(Line::from(Span::styled(if eq_on { " EQ: On. Press Enter to disable the equalizer." } else { " EQ: Off. Press Enter to enable the equalizer." }, Style::default().fg(app.theme.fg))));
        }
        (3, 0) => lines.push(Line::from(Span::styled(" Theme: Press Enter to open the Theme Picker overlay (Alt+C).", Style::default().fg(app.theme.fg)))),
        (3, 1) => lines.push(Line::from(Span::styled(" Transparent BG: Press Enter to toggle. When on, overlay backgrounds become transparent.", Style::default().fg(app.theme.fg)))),
        (3, 2) => lines.push(Line::from(Span::styled(" Sync Covers: Download missing cover art from Deezer for all library tracks.", Style::default().fg(app.theme.fg)))),
        (3, 3) => lines.push(Line::from(Span::styled(" Footer Preset: Press Enter to cycle (Default, Minimal, Full). Also toggled via Alt+F.", Style::default().fg(app.theme.fg)))),
        (4, 0) => lines.push(Line::from(Span::styled(" Spotify: Integration status — requires daemon restart.", Style::default().fg(app.theme.fg)))),
        _ => {}
    }

    let right_para = Paragraph::new(lines);
    f.render_widget(right_para, inner);
}

// ─── Overlay Rendering ───

fn render_overlay(f: &mut ratatui::Frame, area: Rect, app: &mut App) {
    let Some(top) = app.overlays.top() else {
        return;
    };

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
    let overlay_box_bg = if app.transparent_bg { ratatui::style::Color::Reset } else { app.theme.overlay_bg };
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Plain)
        .title(format!(" {} ", top.id.title()))
        .style(Style::default().bg(overlay_box_bg));

    let inner = block.inner(overlay_area);
    f.render_widget(Clear, overlay_area);
    f.render_widget(block, overlay_area);

    match top.id {
        OverlayId::Queue => render_queue_overlay(f, inner, app),
        OverlayId::YTSearch => render_yt_search_overlay(f, inner, app),
        OverlayId::SearchLibrary => render_search_library_overlay(f, inner, app),
        OverlayId::About => render_about_overlay(f, inner, app),
        OverlayId::SleepTimer => render_sleep_timer_overlay(f, inner, app),
        OverlayId::CommandPalette => render_command_palette_overlay(f, inner, app),
        OverlayId::Equalizer => render_equalizer_overlay(f, inner, app),
        OverlayId::SoundEffects => render_sound_effects_overlay(f, inner, app),
        OverlayId::ThemePicker => render_theme_picker_overlay(f, inner, app),
        OverlayId::Help => render_help_overlay(f, inner, app),
        OverlayId::PlaylistSelect => render_playlist_select_overlay(f, inner, app),
        OverlayId::EditMetadata => render_edit_metadata_overlay(f, inner, app),
        OverlayId::SpotifySearch => {
            let p = Paragraph::new("Spotify search not yet implemented")
                .alignment(Alignment::Center)
                .style(Style::default().fg(app.theme.fg_dim));
            f.render_widget(p, inner);
        }
    }
}

fn overlay_help(f: &mut ratatui::Frame, area: Rect, text: &str, app: &App) {
    let help = Paragraph::new(Span::styled(text, Style::default().fg(app.theme.fg_dim)))
        .style(Style::default().bg(app.theme.overlay_bg));
    let help_area = Rect { x: area.x, y: area.y + area.height - 1, width: area.width, height: 1 };
    f.render_widget(help, help_area);
}

fn render_queue_overlay(f: &mut ratatui::Frame, area: Rect, app: &App) {
    let sel = app.overlays.top().map_or(0, |o| o.selected);

    let pane_w = area.width as usize;
    let total = app.queue_cache.len();
    if total == 0 {
        let p = Paragraph::new("Queue is empty")
            .style(Style::default().fg(app.theme.fg_dim));
        f.render_widget(p, area);
        overlay_help(f, area, " [Esc] Close", app);
        return;
    }

    let visible = area.height as usize;
    let (scroll_start, scroll_end) = centered_scroll(sel, visible, total);

    let mut lines = Vec::new();

    for i in scroll_start..scroll_end {
        let track = &app.queue_cache[i];
        let is_current = i == app.queue_cursor;
        let is_sel = i == sel;
        let prefix = if is_current { ">" } else if is_sel { " " } else { " " };
        let num_str = format!("{}{:02}", prefix, i + 1);
        let dur = format_duration_short(track.duration as u64);
        let label = if track.artist.is_empty() { track.title.clone() } else { format!("{}  {}", track.artist, track.title) };

        let row = format!("{:>5}  {:<w$}  {:>6}", num_str, label, dur, w = pane_w.saturating_sub(16));

        let style = if is_sel {
            Style::default().fg(app.theme.selection_fg).bg(app.theme.selection_bg)
        } else if is_current {
            Style::default().fg(app.theme.accent).add_modifier(Modifier::BOLD)
        } else {
            Style::default()
        };

        lines.push(Line::from(Span::styled(row, style)));
    }

    let para = Paragraph::new(lines);
    f.render_widget(para, area);
    overlay_help(f, area, " [Enter] Play  [d] Remove from Queue  [Esc] Close  j/k Navigate", app);
}

fn render_yt_search_overlay(f: &mut ratatui::Frame, area: Rect, app: &App) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(3), Constraint::Min(0)])
        .split(area);

    let query = app.overlays.top().map_or(String::new(), |o| o.query.clone());
    let cursor = if app.overlays.top().map_or(false, |o| o.id == OverlayId::YTSearch) { "_" } else { "" };
    let loading_indicator = if app.yt_search_loading {
        let spinner = braille_spinner(app.scroll_offset);
        format!(" {} ", spinner)
    } else {
        String::new()
    };
    let search_input = Paragraph::new(format!(" > {}{}{}", query, cursor, loading_indicator))
        .style(Style::default().fg(app.theme.fg));
    f.render_widget(search_input, chunks[0]);

    let results_area = chunks[1];
    let sel = app.overlays.top().map_or(0, |o| o.selected);
    let total = app.yt_results_cache.len();
    let visible = results_area.height as usize;
    let (scroll_start, _) = if total > 0 { centered_scroll(sel, visible, total) } else { (0, 0) };
    let scroll_end = (scroll_start + visible).min(total);

    let items: Vec<ListItem> = app
        .yt_results_cache
        .iter()
        .enumerate()
        .skip(scroll_start)
        .take(scroll_end - scroll_start)
        .map(|(i, r)| {
            let dur = format_duration(r.duration as u64);
            let icon = if r.is_playlist { "\u{f01db} " } else { "\u{f008} " };
            let prefix = if i == sel { " > " } else { "   " };
            let content = format!("{prefix}{}{} - {} [{}]", icon, r.channel, r.title, dur);
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
            .title(" Results ")
            .border_type(BorderType::Plain),
    );
    f.render_widget(list, results_area);

    // Help footer
    let help_text = " [Enter] Play / Drill-down  [Ctrl+d] Download  [Ctrl+a] Add to Queue  [Esc] Close  Type to search (auto, 500ms)";
    let help = Paragraph::new(Span::styled(help_text, Style::default().fg(app.theme.fg_dim)))
        .style(Style::default().bg(app.theme.overlay_bg));
    let help_area = Rect { x: area.x, y: area.y + area.height - 1, width: area.width, height: 1 };
    f.render_widget(help, help_area);
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

    let search_input = Paragraph::new(format!(" > {}_", query))
        .style(Style::default().fg(app.theme.fg));
    f.render_widget(search_input, chunks[0]);

    let sel = app.overlays.top().map_or(0, |o| o.selected.min(filtered.len().saturating_sub(1)));
    let results_area = chunks[1];
    let total = filtered.len();
    let visible = results_area.height as usize;
    let (scroll_start, _) = if total > 0 { centered_scroll(sel, visible, total) } else { (0, 0) };
    let scroll_end = (scroll_start + visible).min(total);

    let items: Vec<ListItem> = filtered
        .iter()
        .enumerate()
        .skip(scroll_start)
        .take(scroll_end - scroll_start)
        .map(|(i, track)| {
            let prefix = if i == sel { " > " } else { "   " };
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
            .border_type(BorderType::Plain),
    );

    f.render_widget(list, results_area);
    overlay_help(f, area, " [Enter] Play  [Esc] Close  Type to search  j/k Navigate", app);
}

// ─── Footer ───

fn render_footer(f: &mut ratatui::Frame, area: Rect, app: &mut App) {
    // Volume safety prompt: shown inline in the footer
    if app.pending_volume.is_some() {
        let vol = app.pending_volume.unwrap_or(app.state.volume);
        let prompt = format!("Volume >{}%? [Enter] Yes [Esc] No", vol);
        f.render_widget(
            Paragraph::new(prompt)
                .style(Style::default().fg(app.theme.fg_bright).bg(app.theme.error)),
            area,
        );
        return;
    }
    match app.input_mode {
        InputMode::Normal => {
            // During tab transitions, preserve the last footer render to avoid
            // visual jumps from stale state becoming momentarily visible.
            if app.suppress_footer_refresh {
                if let Some((ref spans, left_bg, right_bg)) = app.cached_footer_spans {
                    let left_w: u16 = spans.iter().map(|s| s.width() as u16).sum::<u16>() + 4;
                    let right_w = area.width.saturating_sub(left_w);
                    let chunks = Layout::default()
                        .direction(Direction::Horizontal)
                        .constraints([Constraint::Length(left_w), Constraint::Min(right_w)])
                        .split(area);
                    f.render_widget(
                        Paragraph::new(Line::from(spans.clone())).style(Style::default().bg(left_bg)),
                        chunks[0],
                    );
                    if right_w > 0 {
                        f.render_widget(
                            Paragraph::new(Line::from(""))
                                .style(Style::default().bg(right_bg)),
                            chunks[1],
                        );
                    }
                    return;
                }
            }
            let presets = crate::footer::presets();
            let idx = app.footer_preset.min(presets.len().saturating_sub(1));
            crate::footer::render_preset(f, area, app, &presets[idx]);
            // Cache the rendered footer spans for the next frame
            app.cached_footer_spans = crate::footer::collect_preset_spans(app, &presets[idx]);
        }
        InputMode::Searching => {
            f.render_widget(
                Paragraph::new(format!(" > {}_", app.search_query))
                    .style(Style::default().fg(app.theme.fg_bright).bg(app.theme.border)),
                area,
            );
        }
        InputMode::Command => {
            f.render_widget(
                Paragraph::new(format!(" :{}_", app.search_query))
                    .style(Style::default().fg(app.theme.fg_bright).bg(app.theme.border)),
                area,
            );
        }
    }
}

pub fn render_progress_variant(ratio: f64, width: usize, app: &App) -> String {
    let inner_w = width.saturating_sub(2).max(4);
    let filled = (ratio.clamp(0.0, 1.0) * inner_w as f64).round() as usize;
    let mut line = String::with_capacity(width);
    match app.theme_index % 3 {
        0 => {
            // Braille dots variant
            line.push('⡀');
            for i in 0..inner_w {
                if i < filled {
                    line.push('⣿');
                } else {
                    line.push('⣀');
                }
            }
            line.push('⠤');
        }
        1 => {
            // Seek-head line variant
            line.push('─');
            for i in 0..inner_w {
                if i == filled {
                    line.push('●');
                } else {
                    line.push('─');
                }
            }
            line.push('─');
        }
        _ => {
            // Classic bracket variant
            line.push('[');
            for i in 0..inner_w {
                line.push(if i < filled { '█' } else { '░' });
            }
            line.push(']');
        }
    }
    line
}

/// Floating track info popup shown when scrolling in the Library list.
fn render_track_popup(f: &mut ratatui::Frame, content_area: Rect, app: &App) {
    let track_id = match app.track_popup_track_id {
        Some(id) => id,
        None => return,
    };
    let track = match app.tracks_cache.iter().find(|t| t.id == track_id) {
        Some(t) => t,
        None => return,
    };

    let has_cover = app.track_popup_cover.is_some();
    const COVER_W: u16 = 6;
    const COVER_H: u16 = 6;
    let text_margin = 2u16;

    let popup_w = if has_cover {
        (COVER_W + 1 + 38 + text_margin).min(content_area.width.saturating_sub(2))
    } else {
        48u16.min(content_area.width.saturating_sub(4))
    };
    let popup_h = if has_cover { COVER_H + 2 } else { 7u16 };
    if popup_w < 20 || popup_h > content_area.height {
        return;
    }

    // Position at bottom-right of content area
    let popup_x = content_area.x + content_area.width.saturating_sub(popup_w + 1);
    let popup_y = content_area.y + content_area.height.saturating_sub(popup_h + 1);
    let popup_area = Rect { x: popup_x, y: popup_y, width: popup_w, height: popup_h };

    let display_title = if track.title.is_empty() {
        std::path::Path::new(&track.path)
            .file_stem()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_default()
    } else {
        track.title.clone()
    };
    let display_artist = if track.artist.is_empty() { "Unknown" } else { &track.artist };
    let display_album = if track.album.is_empty() { "Unknown" } else { &track.album };
    let dur = format_duration(track.duration as u64);
    let fav = if track.favourite { " \u{2665}" } else { "" };

    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Plain)
        .title(format!(" Track Info{} ", fav))
        .border_style(Style::default().fg(app.theme.accent))
        .style(Style::default().bg(app.theme.overlay_bg));

    let inner = block.inner(popup_area);
    f.render_widget(Clear, popup_area);
    f.render_widget(block, popup_area);

    if has_cover && inner.width > COVER_W + 1 {
        // Side-by-side: cover on left, text on right
        let split = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Length(COVER_W + 1), Constraint::Min(0)])
            .split(inner);

        let cover_area = Rect {
            x: split[0].x,
            y: split[0].y,
            width: COVER_W,
            height: COVER_H.min(split[0].height),
        };
        if let Some(ref cover_bytes) = app.track_popup_cover {
            render_cover_block(f, cover_area, cover_bytes);
        }

        let text_area = split[1];
        let lines = vec![
            Line::from(Span::styled(
                format!(" {}", display_title),
                Style::default().fg(app.theme.fg_bright).add_modifier(Modifier::BOLD),
            )),
            Line::from(Span::styled(
                format!(" {}", display_artist),
                Style::default().fg(app.theme.fg),
            )),
            Line::from(Span::styled(
                format!(" {}", display_album),
                Style::default().fg(app.theme.fg),
            )),
            Line::from(""),
            Line::from(Span::styled(
                format!(" Duration: {}", dur),
                Style::default().fg(app.theme.fg_dim),
            )),
            Line::from(Span::styled(
                format!(" Path: {}", track.path),
                Style::default().fg(app.theme.fg_dim),
            )),
        ];
        let para = Paragraph::new(lines);
        f.render_widget(para, text_area);
    } else {
        // Text only (no cover or too narrow)
        let lines = vec![
            Line::from(Span::styled(
                format!("  {}", display_title),
                Style::default().fg(app.theme.fg_bright).add_modifier(Modifier::BOLD),
            )),
            Line::from(Span::styled(
                format!("  {}", display_artist),
                Style::default().fg(app.theme.fg),
            )),
            Line::from(Span::styled(
                format!("  {}", display_album),
                Style::default().fg(app.theme.fg),
            )),
            Line::from(""),
            Line::from(Span::styled(
                format!("  Duration: {}", dur),
                Style::default().fg(app.theme.fg_dim),
            )),
            Line::from(Span::styled(
                format!("  Path: {}", track.path),
                Style::default().fg(app.theme.fg_dim),
            )),
        ];
        let para = Paragraph::new(lines);
        f.render_widget(para, inner);
    }
}

fn render_about_overlay(f: &mut ratatui::Frame, area: Rect, app: &App) {
    let version = option_env!("CARGO_PKG_VERSION").unwrap_or("0.1.0");
    let commit = option_env!("VERGEN_GIT_SHA").unwrap_or("unknown");
    let build_date = option_env!("VERGEN_BUILD_DATE").unwrap_or("unknown");
    let rust_ver = option_env!("VERGEN_RUSTC_SEMVER").unwrap_or("unknown");
    let lib_count = app.tracks_cache.len();
    let queue_count = app.queue_cache.len();

    let lines = vec![
        Line::from(Span::styled(
            format!(" gtm {version}"),
            Style::default().fg(app.theme.accent).add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
        Line::from(Span::styled(
            " Copyright (C) 2026, prjctimg",
            Style::default().fg(app.theme.fg_dim),
        )),
        Line::from(Span::styled(
            " License GPL-3.0",
            Style::default().fg(app.theme.fg_dim),
        )),
        Line::from(""),
        Line::from(Span::styled(" Build", Style::default().fg(app.theme.fg_dim))),
        Line::from(Span::styled(
            format!("   Commit: {:.7}", commit),
            Style::default().fg(app.theme.fg),
        )),
        Line::from(Span::styled(
            format!("   Date:   {}", build_date),
            Style::default().fg(app.theme.fg),
        )),
        Line::from(Span::styled(
            format!("   Rust:   {}", rust_ver),
            Style::default().fg(app.theme.fg),
        )),
        Line::from(""),
        Line::from(Span::styled(" Status", Style::default().fg(app.theme.fg_dim))),
        Line::from(Span::styled(
            format!("   Playing:  {:?}", app.state.status),
            Style::default().fg(app.theme.warning),
        )),
        Line::from(Span::styled(
            format!("   Volume:   {}%", app.state.volume),
            Style::default().fg(app.theme.volume_color(app.state.volume)),
        )),
        Line::from(Span::styled(
            format!("   Queue:    {} tracks", queue_count),
            Style::default().fg(app.theme.fg_bright),
        )),
        Line::from(Span::styled(
            format!("   Library:  {} tracks", lib_count),
            Style::default().fg(app.theme.fg_bright),
        )),
        Line::from(Span::styled(
            format!("   Shuffle:  {}", if app.state.shuffle { "ON" } else { "OFF" }),
            Style::default().fg(app.theme.fg_bright),
        )),
        Line::from(Span::styled(
            format!("   Repeat:   {:?}", app.state.repeat),
            Style::default().fg(app.theme.fg_bright),
        )),
    ];

    let p = Paragraph::new(lines)
        .alignment(Alignment::Left)
        .style(Style::default().bg(app.theme.overlay_bg));
    f.render_widget(p, area);
    overlay_help(f, area, " [Esc] Close  [q] Quit gtm", app);
}

fn render_help_overlay(f: &mut ratatui::Frame, area: Rect, app: &App) {
    let query = app.overlays.top().map_or(String::new(), |o| o.query.clone());
    let help_lines = vec![
        " Playback",
        "   Space        Play / Pause",
        "   n / Ctrl+N   Next track",
        "   p / Ctrl+P   Previous track",
        "   s            Stop",
        "   . / ,        Seek forward / back",
        "",
        " Volume",
        "   + / =        Volume up",
        "   -            Volume down",
        "   m            Toggle mute",
        "",
        " Queue & Library",
        "   Enter        Play selected / drill-down",
        "   d / Del      Remove item",
        "   F            Toggle favourite",
        "   D            Clear queue",
        "   /            Filter mode",
        "",
        " Navigation",
        "   Tab          Toggle left/right pane focus",
        "   1 / 2        Switch to Library / Settings",
        "   j/k / arrows Move up/down",
        "   h/l          Focus left/right pane",
        "   ?            Toggle this help",
        "",
        " Overlays (Alt+key)",
        "   Alt+Q        Queue",
        "   Alt+Y        YouTube Search",
        "   Alt+F        Search Library",
        "   Alt+A        About",
        "   Alt+C        Theme Picker",
        "   Alt+E        Equalizer",
        "   Alt+P        Command Palette",
        "   Alt+Z        Sleep Timer",
        "   Alt+X        Sound Effects",
        "   Alt+S        Spotify Search",
        "",
        " Other",
        "   q            Quit",
        "   Q            Quit & stop daemon",
        "   S            Toggle shuffle",
        "   r / R        Cycle repeat",
        "   :            Command palette",
        "   Alt+F        Cycle footer preset",
    ];

    let filtered: Vec<&str> = if query.is_empty() {
        help_lines.iter().copied().collect()
    } else {
        let q = query.to_lowercase();
        help_lines.iter().filter(|l| l.to_lowercase().contains(&q)).copied().collect()
    };

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(3), Constraint::Min(0)])
        .split(area);

    let search_input = Paragraph::new(format!(" > {}_", query))
        .style(Style::default().fg(app.theme.fg));
    f.render_widget(search_input, chunks[0]);

    let items: Vec<ListItem> = filtered
        .iter()
        .enumerate()
        .map(|(i, line)| {
            let is_header = line.starts_with(|c: char| c.is_uppercase()) && !line.starts_with("   ");
            let is_sel = i == app.overlays.top().map_or(0, |o| o.selected);
            let style = if is_sel {
                Style::default().fg(app.theme.selection_fg).bg(app.theme.selection_bg)
            } else if is_header {
                Style::default().fg(app.theme.accent).add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(app.theme.fg)
            };
            ListItem::new(*line).style(style)
        })
        .collect();

    let list = List::new(items).block(
        Block::default()
            .borders(Borders::ALL)
            .title(" Keybindings ")
            .border_type(BorderType::Plain),
    );
    f.render_widget(list, chunks[1]);
    overlay_help(f, area, " [Esc] Close  Type to search  j/k Navigate", app);
}

fn render_sleep_timer_overlay(f: &mut ratatui::Frame, area: Rect, app: &App) {
    let presets = [5u64, 10, 15, 30, 60];
    let sel = app.overlays.top().map_or(0, |o| o.selected.min(presets.len() - 1));

    let mut items: Vec<ListItem> = presets
        .iter()
        .enumerate()
        .map(|(i, mins)| {
            let label = if *mins == 1 { "minute" } else { "minutes" };
            let prefix = if i == sel { " > " } else { "   " };
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
            .border_type(BorderType::Plain),
    );
    f.render_widget(list, area);
}

pub const COMMAND_PALETTE_COMMANDS: &[&str] = &[
    "Play/Pause   [Space]",
    "Next Track   [n]",
    "Prev Track   [p]",
    "Volume Up    [+]",
    "Volume Down  [-]",
    "Mute Toggle  [m]",
    "Repeat       [r]",
    "Shuffle      [S]",
    "Quit         [Q]",
    "Tab Cycle    [Tab]",
    "Library      [1]",
    "Settings     [2]",
    "Search       [/]",
    "Queue O/L    [Alt+Q]",
    "YouTube O/L  [Alt+Y]",
    "Search Lib   [Alt+F]",
    "EQ O/L       [Alt+E]",
    "SleepTimer   [Alt+Z]",
    "ThemePicker  [Alt+C]",
    "Sound FX O/L [Alt+X]",
    "About O/L    [Alt+A]",
    "Spotify O/L  [Alt+S]",
    "Cmd Palette  [Alt+P]",
];

fn render_command_palette_overlay(f: &mut ratatui::Frame, area: Rect, app: &App) {
    let commands = COMMAND_PALETTE_COMMANDS;

    let query = app.overlays.top().map_or(String::new(), |o| o.query.clone());
    let q = query.to_lowercase();
    let mut filtered: Vec<(&&str, usize)> = if q.is_empty() {
        commands.iter().map(|c| (c, 0)).collect()
    } else {
        commands.iter().filter_map(|c| {
            let lower = c.to_lowercase();
            // Fuzzy subsequence match: each char in q must appear in order
            let mut qi = 0usize;
            for ch in lower.chars() {
                if qi < q.len() && ch == q.as_bytes()[qi] as char {
                    qi += 1;
                }
            }
            if qi == q.len() {
                Some((c, qi))
            } else {
                None
            }
        }).collect()
    };
    // Sort by score descending (longer match = better)
    filtered.sort_by(|a, b| b.1.cmp(&a.1));

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(3), Constraint::Min(0)])
        .split(area);

    let search_input = Paragraph::new(format!(" > {}_", query))
        .style(Style::default().fg(app.theme.fg));
    f.render_widget(search_input, chunks[0]);

    let sel = app.overlays.top().map_or(0, |o| o.selected.min(filtered.len().saturating_sub(1)));
    let results_area = chunks[1];
    let total = filtered.len();
    let visible = results_area.height as usize;
    let (scroll_start, _) = if total > 0 { centered_scroll(sel, visible, total) } else { (0, 0) };
    let scroll_end = (scroll_start + visible).min(total);

    let list_items: Vec<ListItem> = filtered
        .iter()
        .enumerate()
        .skip(scroll_start)
        .take(scroll_end - scroll_start)
        .map(|(i, (cmd, _score))| {
            let prefix = if i == sel { " > " } else { "   " };
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
            .border_type(BorderType::Plain),
    );
    f.render_widget(list, results_area);
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
        ("Electronic",EqPreset::Electronic),
        ("Hip-Hop",   EqPreset::HipHop),
        ("Latin",     EqPreset::Latin),
        ("Acoustic",  EqPreset::Acoustic),
        ("Podcast",   EqPreset::Podcast),
        ("Dance",     EqPreset::Dance),
        ("Headphones",EqPreset::Headphones),
        ("Speaker",   EqPreset::Speaker),
    ];

    let sel = app.overlays.top().map_or(0, |o| o.selected.min(presets.len() - 1));

    let visible = area.height as usize;
    let total = presets.len();
    let (scroll_start, _) = centered_scroll(sel, visible, total);
    let scroll_end = (scroll_start + visible).min(total);

    let list_items: Vec<ListItem> = presets.iter().enumerate()
        .skip(scroll_start)
        .take(scroll_end - scroll_start)
        .map(|(i, (name, _))| {
            let prefix = if i == sel { " > " } else { "   " };
            let style = if i == sel {
                Style::default().fg(app.theme.selection_fg).bg(app.theme.selection_bg)
            } else if *name == app.state.eq_preset.label() {
                Style::default().fg(app.theme.success)
            } else {
                Style::default()
            };
            ListItem::new(format!("{prefix}{}", name)).style(style)
        }).collect();

    let list = List::new(list_items).block(
        Block::default()
            .borders(Borders::ALL)
            .title(" Equalizer ")
            .border_type(BorderType::Plain),
    );
    f.render_widget(list, area);
}

fn render_sound_effects_overlay(f: &mut ratatui::Frame, area: Rect, app: &App) {
    let crossfade_on = app.state.crossfade.as_ref().map(|c| c.enabled).unwrap_or(false);
    let crossfade_dur = app.state.crossfade.as_ref().map(|c| c.duration_secs).unwrap_or(0);

    let reverb_on = app.state.reverb.enabled;

    let items = vec![
        format!("Playback Speed:  {:.1}x", app.playback_speed),
        format!("Reverb:          {}", if reverb_on { "ON" } else { "OFF" }),
        format!("Crossfade:       {}", if crossfade_on { "ON" } else { "OFF" }),
        format!("Crossfade Dur:   {}s", crossfade_dur),
        format!("EQ Preset:       {}", app.state.eq_preset.label()),
    ];

    let sel = app.overlays.top().map_or(0, |o| o.selected.min(items.len() - 1));
    let visible = area.height as usize;
    let total = items.len();
    let (scroll_start, _) = centered_scroll(sel, visible, total);
    let scroll_end = (scroll_start + visible).min(total);

    let list_items: Vec<ListItem> = items.iter().enumerate()
        .skip(scroll_start)
        .take(scroll_end - scroll_start)
        .map(|(i, s)| {
            let prefix = if i == sel { " > " } else { "   " };
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
            .border_type(BorderType::Plain),
    );
    f.render_widget(list, area);
}

fn render_theme_picker_overlay(f: &mut ratatui::Frame, area: Rect, app: &App) {
    let sel = app.overlays.top().map_or(0, |o| o.selected.min(THEMES.len().saturating_sub(1)));
    let visible = area.height as usize;
    let total = THEMES.len();
    let (scroll_start, _) = centered_scroll(sel, visible, total);
    let scroll_end = (scroll_start + visible).min(total);

    let list_items: Vec<ListItem> = THEMES.iter().enumerate()
        .skip(scroll_start)
        .take(scroll_end - scroll_start)
        .map(|(i, entry)| {
            let is_active = i == app.theme_index;
            let prefix = if i == sel { " > " } else { "   " };
            let check = if is_active { " \u{2713}" } else { "" };
            let content = format!("{}{}{}", prefix, entry.name, check);
            let style = if i == sel {
                Style::default().fg(app.theme.selection_fg).bg(app.theme.selection_bg)
            } else if is_active {
                Style::default().fg(app.theme.accent).add_modifier(Modifier::BOLD)
            } else {
                Style::default()
            };
            ListItem::new(content).style(style)
        }).collect();

    let list = List::new(list_items).block(
        Block::default()
            .borders(Borders::ALL)
            .title(" Theme ")
            .border_type(BorderType::Plain),
    );
    f.render_widget(list, area);
}

/// Return the local time as " HH:MM " using the system clock.
pub fn local_time_str() -> String {
    let now = chrono::Local::now();
    format!(" {:02}:{:02} ", now.hour(), now.minute())
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

/// Shorter duration format for table columns (no zero-padding on hours).
fn format_duration_short(secs: u64) -> String {
    let h = secs / 3600;
    let m = (secs % 3600) / 60;
    let s = secs % 60;
    if h > 0 {
        format!("{h}:{m:02}:{s:02}")
    } else {
        format!("{m:02}:{s:02}")
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

/// Build a single-line progress bar string in bracket style:
/// `[###########---------]` — `#` fill, `-` empty.
/// `ratio` in [0.0, 1.0]; `width` in terminal columns (includes brackets).
#[allow(dead_code)]
fn render_progress_line(ratio: f64, width: usize) -> String {
    let width = width.max(6);
    let inner_w = width.saturating_sub(2);
    let filled = (ratio.clamp(0.0, 1.0) * inner_w as f64).round() as usize;

    let mut line = String::with_capacity(width);
    line.push('[');
    for i in 0..inner_w {
        line.push(if i < filled { '#' } else { '-' });
    }
    line.push(']');
    line
}

/// Volume label in bracket style.
#[allow(dead_code)]
fn volume_icon(volume: u8) -> &'static str {
    match volume {
        0 => "[MUTED]",
        _ => "[VOL]",
    }
}

/// Pick the foreground colour that has enough contrast against `bg`.
/// Uses simple luminance formula (BT.601) to decide between `dark` and `light`.
pub fn readable_fg(bg: ratatui::style::Color, _dark: ratatui::style::Color, _light: ratatui::style::Color) -> ratatui::style::Color {
    fn luminance(c: &ratatui::style::Color) -> f64 {
        match c {
            ratatui::style::Color::Rgb(r, g, b) => {
                0.299 * *r as f64 + 0.587 * *g as f64 + 0.114 * *b as f64
            }
            _ => 128.0,
        }
    }
    if luminance(&bg) > 128.0 {
        ratatui::style::Color::Rgb(20, 20, 20)
    } else {
        ratatui::style::Color::Rgb(240, 240, 240)
    }
}

/// Scroll text horizontally if it exceeds max_width, using a frame-based offset.
/// Only the selected item scrolls; others are truncated with "…".
fn scroll_text(text: &str, max_width: usize, frame: usize, is_selected: bool) -> String {
    if text.len() <= max_width {
        return format!("{:<width$}", text, width = max_width);
    }
    if !is_selected {
        let truncated: String = text.chars().take(max_width.saturating_sub(1)).collect();
        return format!("{}…", truncated);
    }
    // Animated scroll: shift by (frame / 3) characters, wrap around
    let scroll = (frame / 3) % text.len();
    let scrolled = format!("{}{}", &text[scroll..], &text[..scroll]);
    scrolled.chars().take(max_width).collect()
}

// ─── Library Motion Overlays ───

fn render_playlist_select_overlay(f: &mut ratatui::Frame, area: Rect, app: &App) {
    let sel = app.overlays.top().map_or(0, |o| o.selected);
    let mut items: Vec<ListItem> = Vec::new();

    // "Create New" option at the top
    items.push(ListItem::new("  + Create New Playlist").style(
        Style::default().fg(app.theme.accent),
    ));

    for (i, pl) in app.playlist_cache.iter().enumerate() {
        let prefix = if i + 1 == sel { " > " } else { "   " };
        let content = format!("{}{} ({} tracks)", prefix, pl.name, pl.track_count);
        let style = if i + 1 == sel {
            Style::default().fg(app.theme.selection_fg).bg(app.theme.selection_bg)
        } else {
            Style::default().fg(app.theme.fg)
        };
        items.push(ListItem::new(content).style(style));
    }

    let list = List::new(items).block(
        Block::default()
            .borders(Borders::ALL)
            .title(" Select Playlist ")
            .border_type(BorderType::Plain),
    );
    f.render_widget(list, area);

    let help_text = " [Enter] Select  [Esc] Cancel";
    overlay_help(f, area, help_text, app);
}

fn render_edit_metadata_overlay(f: &mut ratatui::Frame, area: Rect, app: &App) {
    let field_names = ["Title", "Artist", "Album", "Album Artist", "Genre", "Year", "Track #"];
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(field_names.len() as u16 * 2 + 1),
            Constraint::Min(0),
        ])
        .split(area);

    let mut lines: Vec<Line> = Vec::new();
    for (i, name) in field_names.iter().enumerate() {
        let value = app.metadata_fields.get(i).map(|s| s.as_str()).unwrap_or("");
        let is_active = i == app.metadata_field_idx;
        let prefix = if is_active { " > " } else { "   " };
        let style = if is_active {
            Style::default().fg(app.theme.selection_fg).bg(app.theme.selection_bg)
        } else {
            Style::default().fg(app.theme.fg)
        };
        lines.push(Line::from(vec![
            Span::styled(format!("{}{}: ", prefix, name), style),
            Span::styled(value.to_string(), style),
        ]));
    }

    let para = Paragraph::new(lines);
    f.render_widget(para, chunks[0]);

    let help_text = " [j/k] Navigate fields  [Tab] Next  [Enter] Next/Save  [Ctrl+Enter] Save  [Esc] Cancel";
    overlay_help(f, chunks[1], help_text, app);
}
