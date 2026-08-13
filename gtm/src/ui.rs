// Copyright (c) 2025 - present
// Author: prjctimg <prjctimg@outlook.com>
// TUI rendering: tab layout, pickers, library, now-playing, settings
//
// This is free software released under the GPL-3.0 license.

use std::path::PathBuf;

use crate::app::{App, InputMode, LIBRARY_CATEGORIES};
use crate::picker::PickerId;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use gtm_core::state::{EqPreset, Tab};
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::Color;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Clear, List, ListItem, Paragraph};
use ratatui::Terminal;
use ratatui_image::protocol::StatefulProtocol;
use ratatui_image::StatefulImage;

pub fn run_tui(socket: Option<String>) -> Result<(), Box<dyn std::error::Error>> {
    let socket_path = socket
        .map(PathBuf::from)
        .unwrap_or_else(gtm_core::resolve_command_socket);

    // Redirect stderr to log file so diagnostic messages don't break the TUI
    let _original_stderr = gtm_core::log::redirect_stderr_to_log();

    let rt = tokio::runtime::Runtime::new()?;
    rt.block_on(async {
        color_eyre::install()?;

        ensure_daemon_running(&socket_path).await?;

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

        let res = async {
            let app = App::new(&socket_path).await?;
            app.run(&mut terminal).await
        }
        .await;

        let _ = disable_raw_mode();
        let mut stdout = std::io::stdout();
        let _ = crossterm::execute!(stdout, LeaveAlternateScreen);

        res
    })
}

async fn ensure_daemon_running(
    socket_path: &std::path::Path,
) -> Result<(), Box<dyn std::error::Error>> {
    // If socket exists, try a quick ping to check if daemon is alive
    if socket_path.exists() {
        if let Ok(mut stream) = tokio::net::UnixStream::connect(socket_path).await {
            use tokio::io::{AsyncReadExt, AsyncWriteExt};
            let ping = serde_json::to_string(&gtm_core::ipc::WireReq {
                id: 0,
                cmd: "ping".to_string(),
                params: serde_json::to_value(gtm_core::ipc::DaemonReq::Ping).unwrap(),
            })? + "\n";
            let _ = stream.write_all(ping.as_bytes()).await;
            let mut buf = [0u8; 256];
            if let Ok(Ok(n)) =
                tokio::time::timeout(std::time::Duration::from_millis(100), stream.read(&mut buf))
                    .await
            {
                if n > 0 {
                    return Ok(());
                }
            }
        }
        // Socket exists but daemon is dead/stale — remove it
        let _ = std::fs::remove_file(socket_path);
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

    // Wait for the daemon socket to appear (cold start can take a while
    // for audio backend init + library scan).
    for _ in 0..120 {
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        if socket_path.exists() {
            // Socket exists — try a ping to confirm daemon is responsive
            if let Ok(mut stream) = tokio::net::UnixStream::connect(socket_path).await {
                use tokio::io::{AsyncReadExt, AsyncWriteExt};
                let ping = serde_json::to_string(&gtm_core::ipc::WireReq {
                    id: 0,
                    cmd: "ping".to_string(),
                    params: serde_json::to_value(gtm_core::ipc::DaemonReq::Ping).unwrap(),
                })? + "\n";
                let _ = stream.write_all(ping.as_bytes()).await;
                let mut buf = [0u8; 256];
                if let Ok(Ok(n)) = tokio::time::timeout(
                    std::time::Duration::from_millis(500),
                    stream.read(&mut buf),
                )
                .await
                {
                    if n > 0 {
                        return Ok(());
                    }
                }
            }
        }
    }

    // Daemon didn't become ready in time — let App::new handle the connection
    // error with its own retries. The TUI will show an empty state and the
    // daemon may still come up.
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
    if area.width < 20 || area.height < 6 {
        let msg = Paragraph::new("Terminal too small (min 20x6)")
            .alignment(Alignment::Center)
            .style(Style::default().fg(app.theme.fg_dim));
        f.render_widget(msg, area);
        return;
    }
    // Explicit background fill — the TUI defines its own background
    f.render_widget(
        ratatui::widgets::Block::default().style(ratatui::style::Style::default().bg(app.theme.bg)),
        area,
    );
    // help bar shows on Library tab, hidden during pickers
    let show_help = app.current_tab == Tab::Library && !app.pickers.is_open() && !app.hide_help_bar;
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
    if app.current_tab == Tab::Library && app.track_popup_visible && !app.pickers.is_open() {
        render_track_popup(f, chunks[1], app);
    }

    // Render pickers on top of everything
    if app.pickers.is_open() {
        render_picker(f, area, app);
    }

    // Health check panel overlay
    if app.show_health_panel {
        render_health_panel(f, area, app);
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
        let para = Paragraph::new(text).style(Style::default().fg(app.theme.fg_bright).bg(color));
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
    let text = if app.terminal_cols < 60 {
        " [Space] Play  [n] Next  [p] Prev  [+/-] Vol  [:] Cmd  [q] Quit "
    } else {
        " [Space] Play/Pause  [n/N] Next  [p/P] Prev  [+/-] Volume  [s] Stop  [:] Command Palette  [q] Quit "
    };
    let para = Paragraph::new(text).style(Style::default().fg(app.theme.fg_dim));
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
        let terminal_name = if std::env::var("ZELLIJ").is_ok() {
            "Zellij"
        } else {
            "Neovim"
        };
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

    // Crop-then-resize: CSS background-size: cover behavior
    let src_w = img.width() as f64;
    let src_h = img.height() as f64;
    let target_ratio = disp_w as f64 / disp_h as f64;
    let source_ratio = src_w / src_h;

    let cropped = if (source_ratio - target_ratio).abs() < 0.01 {
        img
    } else if source_ratio > target_ratio {
        // Source is wider: crop sides
        let new_w = (src_h * target_ratio) as u32;
        let offset = ((src_w as u32 - new_w) / 2).min(img.width() - 1);
        image::imageops::crop_imm(&img, offset, 0, new_w, img.height()).to_image()
    } else {
        // Source is taller: crop top/bottom
        let new_h = (src_w / target_ratio) as u32;
        let offset = ((img.height() - new_h) / 2).min(img.height() - 1);
        image::imageops::crop_imm(&img, 0, offset, img.width(), new_h).to_image()
    };

    let thumb = image::imageops::resize(
        &cropped,
        disp_w,
        disp_h,
        image::imageops::FilterType::CatmullRom,
    );
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
    "\u{f001}", "\u{f004}", "\u{f025}", "\u{f007}", "\u{f03a}", "\u{f1bc}", "\u{f167}",
];

const LIBRARY_ICONS_ASCII: &[&str] = &["♫", "♥", "▤", "♪", "≡", "☊", "▶"];

fn use_nerd_fonts() -> bool {
    !matches!(std::env::var("GTM_NERD_FONTS"), Ok(v) if v == "0" || v == "false" || v == "no")
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
    let is_narrow = app.terminal_cols < 60;
    let show_vis = app.visualizer.is_enabled() && app.terminal_cols >= 80;
    let np_height: u16 = if is_narrow { 5 } else { 8 };

    let lib_width: u16 = if is_narrow {
        (app.terminal_cols / 3).max(12).min(area.width.saturating_sub(2))
    } else {
        28u16.min(area.width.saturating_sub(2))
    };

    // Lyrics always get a right column spanning the full height.
    // Left side has Now Playing on top + Library/Content below.
    let lyrics_takes_full_height = app.show_lyrics && !is_narrow;

    let (left_area, lyrics_area) = if lyrics_takes_full_height {
        let lyrics_w = area.width / 3;
        let left_w = area.width - lyrics_w;
        let h = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Length(left_w), Constraint::Length(lyrics_w)])
            .split(area);
        (h[0], Some(h[1]))
    } else {
        (area, None)
    };

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(np_height), Constraint::Min(1)])
        .split(left_area);

    let (np_area, vis_area) = if show_vis {
        let vis_w = app.terminal_cols / 3;
        let h = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Min(0), Constraint::Length(vis_w)])
            .split(chunks[0]);
        (h[0], Some(h[1]))
    } else {
        (chunks[0], None)
    };

    let panes = if is_narrow {
        if app.library_pane_focus {
            Layout::default()
                .direction(Direction::Horizontal)
                .constraints([Constraint::Min(0), Constraint::Length(0)])
                .split(chunks[1])
                .to_vec()
        } else {
            Layout::default()
                .direction(Direction::Horizontal)
                .constraints([Constraint::Length(0), Constraint::Min(0)])
                .split(chunks[1])
                .to_vec()
        }
    } else {
        Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Length(lib_width), Constraint::Min(0)])
            .split(chunks[1])
            .to_vec()
    };

    let left_focus = app.library_pane_focus;

    // ── Now Playing section ──
    {
        let np_block = Block::default()
            .borders(Borders::ALL)
            .title(" Now Playing ")
            .border_type(BorderType::Plain)
            .border_style(Style::default().fg(app.theme.fg));

        if let Some(track) = app.state.current_track.as_ref() {
            let inner = np_block.inner(np_area);
            f.render_widget(np_block, np_area);

            const COVER_W: u16 = 12;
            const COVER_H: u16 = 6;

            if inner.width >= COVER_W + 4 {
                // Cover on the LEFT, info on the RIGHT
                let hchunks = Layout::default()
                    .direction(Direction::Horizontal)
                    .constraints([
                        Constraint::Length(COVER_W),
                        Constraint::Length(1),
                        Constraint::Min(0),
                    ])
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

                let info_area = hchunks[2];

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
                let display_artist = if track.artist.is_empty() {
                    "Unknown"
                } else {
                    &track.artist
                };
                let fav_prefix = if track.favourite { "\u{2665} " } else { "" };
                let title_text = format!(
                    "{}{} \u{2014} {}",
                    fav_prefix, display_artist, display_title
                );
                let title_avail = info_chunks[0].width as usize;
                let animated_title =
                    scroll_text(&title_text, title_avail, app.np_title_scroll, true);
                let title_para = Paragraph::new(Line::from(vec![Span::styled(
                    &animated_title,
                    Style::default()
                        .fg(app.theme.fg_bright)
                        .add_modifier(Modifier::BOLD),
                )]));
                f.render_widget(title_para, info_chunks[0]);

                // Row 1: Artist
                let artist_para = Paragraph::new(Line::from(vec![
                    Span::styled("Artist: ", Style::default().fg(app.theme.fg)),
                    Span::styled(display_artist, Style::default().fg(app.theme.fg_bright)),
                ]));
                f.render_widget(artist_para, info_chunks[1]);

                // Row 2: Album (hide when empty)
                if !track.album.is_empty() {
                    let album_para = Paragraph::new(Line::from(vec![
                        Span::styled("Album: ", Style::default().fg(app.theme.fg)),
                        Span::styled(&track.album, Style::default().fg(app.theme.fg_bright)),
                    ]));
                    f.render_widget(album_para, info_chunks[2]);
                }

                // Row 3: Progress bar + timestamps
                let dur = track.duration;
                let pos = app.display_position;
                let pos_str = format_duration(pos as u64);
                let dur_str = format_duration(dur as u64);
                let ratio = if dur > 0.0 { pos / dur } else { 0.0 };
                let ts_str = format!("{} / {}", pos_str, dur_str);
                let bar_width =
                    (info_chunks[3].width.saturating_sub(ts_str.len() as u16 + 2) as usize).min(40);
                let bar = render_progress_variant(ratio, bar_width, app);
                let bar_with_ts = format!("{} {}", bar, ts_str);
                let bar_para =
                    Paragraph::new(bar_with_ts).style(Style::default().fg(app.theme.accent));
                f.render_widget(bar_para, info_chunks[3]);
            } else {
                // Terminal too narrow for cover — just show info text
                let display_title = if track.title.is_empty() {
                    std::path::Path::new(&track.path)
                        .file_stem()
                        .map(|s| s.to_string_lossy().to_string())
                        .unwrap_or_default()
                } else {
                    track.title.clone()
                };
                let display_artist = if track.artist.is_empty() {
                    "Unknown"
                } else {
                    &track.artist
                };
                let fav_prefix = if track.favourite { "\u{2665} " } else { "" };
                let title_text = format!(
                    "{}{} \u{2014} {}",
                    fav_prefix, display_artist, display_title
                );
                let title_avail = inner.width as usize;
                let animated_title =
                    scroll_text(&title_text, title_avail, app.np_title_scroll, true);
                let title_para = Paragraph::new(Line::from(vec![Span::styled(
                    &animated_title,
                    Style::default()
                        .fg(app.theme.fg_bright)
                        .add_modifier(Modifier::BOLD),
                )]));
                f.render_widget(
                    title_para,
                    Rect {
                        x: inner.x,
                        y: inner.y,
                        width: inner.width,
                        height: 1,
                    },
                );

                let mut row_offset = 1u16;
                if !track.album.is_empty() {
                    let album_para = Paragraph::new(Line::from(vec![
                        Span::styled("Album: ", Style::default().fg(app.theme.fg)),
                        Span::styled(&track.album, Style::default().fg(app.theme.fg_bright)),
                    ]));
                    let album_area = Rect {
                        x: inner.x,
                        y: inner.y + row_offset,
                        width: inner.width,
                        height: 1,
                    };
                    f.render_widget(album_para, album_area);
                    row_offset += 1;
                }

                let dur = track.duration;
                let pos = app.display_position;
                let pos_str = format_duration(pos as u64);
                let dur_str = format_duration(dur as u64);
                let ratio = if dur > 0.0 { pos / dur } else { 0.0 };
                let ts_str = format!("{} / {}", pos_str, dur_str);
                let bar_width =
                    (inner.width.saturating_sub(ts_str.len() as u16 + 2) as usize).min(40);
                let bar = render_progress_variant(ratio, bar_width, app);
                let bar_with_ts = format!("{} {}", bar, ts_str);
                let bar_para =
                    Paragraph::new(bar_with_ts).style(Style::default().fg(app.theme.accent));
                let bar_area = Rect {
                    x: inner.x,
                    y: inner.y + row_offset,
                    width: inner.width,
                    height: 1,
                };
                f.render_widget(bar_para, bar_area);
            }
        } else {
            let inner = np_block.inner(np_area);
            f.render_widget(np_block, np_area);
            let msg = Paragraph::new("It's awfully quiet here... ")
                .alignment(Alignment::Center)
                .style(Style::default().fg(app.theme.fg));
            f.render_widget(msg, inner);
        }
    }

    // ── Visualizer (right of Now Playing) ──
    if let Some(vis_a) = vis_area {
        if vis_a.width >= 4 && vis_a.height >= 3 {
            app.visualizer.tick(
                app.state.status == gtm_core::state::PlaybackStatus::Playing,
                vis_a.width,
            );
            let vis_block = Block::default()
                .borders(Borders::ALL)
                .title(" Visualizer ")
                .border_type(BorderType::Plain)
                .border_style(Style::default().fg(app.theme.fg_dim));
            let vis_inner = vis_block.inner(vis_a);
            f.render_widget(vis_block, vis_a);
            if let Some(lines) = app.visualizer.render(vis_inner, &app.theme) {
                f.render_widget(lines, vis_inner);
            }
        }
    }

    // ── Left pane: categories with icons ──
    let lib_icons = if use_nerd_fonts() {
        LIBRARY_ICONS_NERD
    } else {
        LIBRARY_ICONS_ASCII
    };
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
                "Spotify" => app.spotify_playlists.len(),
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
                Style::default()
                    .fg(app.theme.selection_fg)
                    .bg(app.theme.selection_bg)
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

    // Active category left-border indicator picker
    if app.library_category < LIBRARY_CATEGORIES.len() {
        let indicator_y = left_inner.y + app.library_category as u16;
        if indicator_y < left_inner.y + left_inner.height {
            let indicator_area = Rect {
                x: left_inner.x + 1,
                y: indicator_y,
                width: 1,
                height: 1,
            };
            let indicator =
                Paragraph::new("▎").style(Style::default().fg(app.theme.sidebar_active_border));
            f.render_widget(indicator, indicator_area);
        }
    }
    // end library rendering

    // ── Stats at bottom of left pane ──
    let category_label = LIBRARY_CATEGORIES
        .get(app.library_category)
        .unwrap_or(&"All Tracks");

    let (right_lines, stats_line) = if app.browse_detail.is_some() && app.library_category == 5 {
        // Spotify playlist detail: cached track list, resolved via the daemon.
        let tracks = &app.spotify_playlist_tracks_cache;
        let total_len = tracks.len();
        let st_line = format!(" {} tracks ", total_len);
        let reserve = 3usize;
        let available = panes[1].height.saturating_sub(reserve as u16) as usize;
        app.viewport_items = available;
        let sel = app.scroll_offset.min(total_len.saturating_sub(1));
        let (list_scroll, end) = centered_scroll(sel, available, total_len);
        app.list_scroll = list_scroll;

        let pane_w = panes[1].width as usize;
        let mut lines = vec![Line::from("")];
        if tracks.is_empty() {
            lines.push(Line::from(Span::styled(
                " No tracks — run Settings > Spotify > Sync Now, then press Enter again",
                Style::default().fg(app.theme.fg_dim),
            )));
            (lines, st_line)
        } else {
            for (i, tr) in tracks[app.list_scroll..end].iter().enumerate() {
                let real_i = app.list_scroll + i;
                let is_sel = real_i == sel && !left_focus;
                let label = if tr.artists.is_empty() {
                    tr.name.clone()
                } else {
                    format!("{} \u{2014} {}", tr.artists, tr.name)
                };
                let avail = pane_w.saturating_sub(2);
                let display_label = scroll_text(&label, avail, app.footer_title_scroll, is_sel);
                let dur = tr
                    .duration_ms
                    .map(|d| format_duration_short(d / 1000))
                    .unwrap_or_default();
                let prefix = if is_sel { " >" } else { "  " };
                let row = format!(
                    "{}{:<width$}  {:>6}",
                    prefix,
                    display_label,
                    dur,
                    width = avail.saturating_sub(9)
                );
                let style = if is_sel {
                    Style::default()
                        .fg(app.theme.selection_fg)
                        .bg(app.theme.selection_bg)
                } else {
                    Style::default().fg(app.theme.fg)
                };
                lines.push(Line::from(Span::styled(row, style)));
            }
            (lines, st_line)
        }
    } else if app.browse_detail.is_some() {
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
        let mut lines = vec![Line::from("")];
        if filtered.is_empty() {
            lines.push(Line::from(Span::styled(
                " No tracks found for this selection",
                Style::default().fg(app.theme.fg_dim),
            )));
            (lines, " 0 tracks | 0h 0m ".to_string())
        } else {
        for (i, track) in filtered[app.list_scroll..end].iter().enumerate() {
            let real_i = app.list_scroll + i;
            let is_current = app.state.current_track.as_ref().map(|t| t.id) == Some(track.id);
            let is_sel = real_i == sel && !left_focus;
            let label = if track.artist.is_empty() {
                track.title.clone()
            } else {
                format!("{}  {}", track.artist, track.title)
            };
            let avail = pane_w.saturating_sub(2);
            let display_label = scroll_text(&label, avail, app.footer_title_scroll, is_sel);
            let prefix = if is_current {
                "> "
            } else {
                "  "
            };
            let row = format!("{}{}", prefix, display_label);
            let style = if is_current {
                Style::default()
                    .fg(app.theme.accent)
                    .add_modifier(Modifier::BOLD)
            } else if is_sel {
                Style::default()
                    .fg(app.theme.selection_fg)
                    .bg(app.theme.selection_bg)
            } else {
                Style::default()
            };
            lines.push(Line::from(Span::styled(row, style)));
        }
        (lines, st_line)
        }
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
        let mut lines = vec![Line::from("")];
        for (i, (name, count)) in albums[app.list_scroll..end].iter().enumerate() {
            let real_i = app.list_scroll + i;
            let prefix = if real_i == sel && !left_focus {
                " >"
            } else {
                "  "
            };
            let style = if real_i == sel && !left_focus {
                Style::default()
                    .fg(app.theme.selection_fg)
                    .bg(app.theme.selection_bg)
            } else {
                Style::default().fg(app.theme.fg)
            };
            lines.push(Line::from(Span::styled(
                format!("{}{:<40} {:>4} tracks", prefix, name, count),
                style,
            )));
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
        let mut lines = vec![Line::from("")];
        for (i, (name, count)) in artists[app.list_scroll..end].iter().enumerate() {
            let real_i = app.list_scroll + i;
            let prefix = if real_i == sel && !left_focus {
                " >"
            } else {
                "  "
            };
            let style = if real_i == sel && !left_focus {
                Style::default()
                    .fg(app.theme.selection_fg)
                    .bg(app.theme.selection_bg)
            } else {
                Style::default().fg(app.theme.fg)
            };
            lines.push(Line::from(Span::styled(
                format!("{}{:<40} {:>4} tracks", prefix, name, count),
                style,
            )));
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
        let mut lines = vec![Line::from("")];
        for (i, pl) in playlists[app.list_scroll..end].iter().enumerate() {
            let real_i = app.list_scroll + i;
            let prefix = if real_i == sel && !left_focus {
                " >"
            } else {
                "  "
            };
            let style = if real_i == sel && !left_focus {
                Style::default()
                    .fg(app.theme.selection_fg)
                    .bg(app.theme.selection_bg)
            } else {
                Style::default().fg(app.theme.fg)
            };
            lines.push(Line::from(Span::styled(
                format!("{}{:<40} {:>4} tracks", prefix, pl.name, pl.track_count),
                style,
            )));
        }
        (lines, st_line)
    } else if app.library_category == 5 {
        // Spotify playlists browse
        let playlists = &app.spotify_playlists;
        let total_len = playlists.len();
        let sel = app.scroll_offset.min(total_len.saturating_sub(1));
        let st_line = format!(" {} playlists ", total_len);
        let reserve = 3usize;
        let available = panes[1].height.saturating_sub(reserve as u16) as usize;
        app.viewport_items = available;
        let (list_scroll, end) = centered_scroll(sel, available, total_len);
        app.list_scroll = list_scroll;
        let mut lines = vec![Line::from("")];
        if playlists.is_empty() {
            lines.push(Line::from(Span::styled(
                " No synced playlists — link an account in Settings > Spotify",
                Style::default().fg(app.theme.fg_dim),
            )));
        } else {
            for (i, pl) in playlists[app.list_scroll..end].iter().enumerate() {
                let real_i = app.list_scroll + i;
                let prefix = if real_i == sel && !left_focus {
                    " >"
                } else {
                    "  "
                };
                let style = if real_i == sel && !left_focus {
                    Style::default()
                        .fg(app.theme.selection_fg)
                        .bg(app.theme.selection_bg)
                } else {
                    Style::default().fg(app.theme.fg)
                };
                lines.push(Line::from(Span::styled(
                    format!("{}{:<40} {:>4} tracks", prefix, pl.name, pl.track_count()),
                    style,
                )));
            }
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

        let mut lines = vec![Line::from("")];
        for (i, track) in filtered[app.list_scroll..end].iter().enumerate() {
            let real_i = app.list_scroll + i;
            let is_current = app.state.current_track.as_ref().map(|t| t.id) == Some(track.id);
            let is_sel = real_i == sel && !left_focus;
            let label = if track.artist.is_empty() {
                track.title.clone()
            } else {
                format!("{}  {}", track.artist, track.title)
            };
            let avail = pane_w.saturating_sub(2);
            let display_label = scroll_text(&label, avail, app.footer_title_scroll, is_sel);
            let prefix = if is_current {
                "> "
            } else {
                "  "
            };
            let row = format!("{}{}", prefix, display_label);
            let style = if is_current {
                Style::default()
                    .fg(app.theme.accent)
                    .add_modifier(Modifier::BOLD)
            } else if is_sel {
                Style::default()
                    .fg(app.theme.selection_fg)
                    .bg(app.theme.selection_bg)
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
        let stats = Paragraph::new(Line::from(vec![Span::styled(
            stats_line.trim(),
            Style::default().fg(app.theme.fg_dim),
        )]));
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
    // end content rendering

    // ── Right lyrics pane (full height) ──
    if let Some(lyrics_area) = lyrics_area {
        render_lyrics_pane(f, lyrics_area, app);
    } else if app.show_lyrics && panes.len() == 1 {
        render_lyrics_pane(f, panes[0], app);
    }
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
                    .fg(app.theme.selection_fg)
                    .bg(app.theme.selection_bg)
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
            let indicator_area = Rect {
                x: left_inner.x + 1,
                y: indicator_y,
                width: 1,
                height: 1,
            };
            let indicator =
                Paragraph::new("▎").style(Style::default().fg(app.theme.sidebar_active_border));
            f.render_widget(indicator, indicator_area);
        }
    }

    // ── Right pane: options for selected category ──
    let items: Vec<String> = match app.settings_category {
        0 => vec![
            format!("Master Volume   [ {:>3}%  ]", app.state.master_volume),
            format!("Volume          [ {:>3}%  ]", app.state.volume),
            format!(
                "Mute            [ {} ]",
                if app.state.mute {
                    "●   On "
                } else {
                    "○   Off"
                }
            ),
        ],
        1 => vec![
            format!("Cookie Source   [ chromium   ▶ ]"),
            format!(
                "Cookie File     [ {} ]",
                app.cookie_file.as_deref().unwrap_or("(none)")
            ),
            format!("JS Runtime      [ deno       ▶ ]"),
            format!("Max Downloads   [{:-<13}]  3", "█".repeat(9)),
            format!("Results/Page    10"),
            format!("Search History  [ 0 entries ▶ ]"),
            format!("Auto Download   [ ● ]  On"),
            format!("Clear History   [Clear]"),
        ],
        2 => {
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
            let easing = app
                .state
                .crossfade
                .as_ref()
                .map(|c| c.easing.name())
                .unwrap_or("N/A");
            vec![
                format!("Repeat          [ {:?}       ▶ ]", app.state.repeat),
                format!(
                    "Shuffle         [ {} ]",
                    if app.state.shuffle {
                        "●   On "
                    } else {
                        "○   Off"
                    }
                ),
                if crossfade_on {
                    format!("Crossfade       [ ● ]  On  {}s", crossfade_dur)
                } else {
                    "Crossfade       [ ○ ]  Off".to_string()
                },
                format!("Easing          [ {}   ▶ ]", easing),
                format!(
                    "EQ Enabled      [ {} ]",
                    if app.state.eq_enabled {
                        "●   On "
                    } else {
                        "○   Off"
                    }
                ),
            ]
        }
        3 => {
            let preset_name = app
                .footer_presets
                .get(app.footer_preset)
                .map(|p| p.name.as_ref())
                .unwrap_or("Default");
            let theme_name = app
                .themes
                .get(app.theme_index)
                .map(|t| t.name.as_ref())
                .unwrap_or("Chadrula");
            vec![
                format!("Theme           [ {:>8} ▶ ]", theme_name),
                format!(
                    "Transparent BG  [ {} ]",
                    if app.transparent_bg { "●" } else { "○" }
                ),
                "Sync Covers     [ Enter  ▶ ]".to_string(),
                "Sync Lyrics     [ Enter  ▶ ]".to_string(),
                format!("Footer Preset   [ {:>8} ▶ ]", preset_name),
            ]
        }
        4 => {
            let st = app.spotify_status.clone().unwrap_or_default();
            let connected = if st.linked {
                "Connected"
            } else {
                "Disconnected"
            };
            let user = st.user.as_deref().unwrap_or("(none)");
            let status_label = if let Some(err) = st.error.as_deref() {
                let mut e = err.chars().take(14).collect::<String>();
                if err.chars().count() > 14 {
                    e.push('…');
                }
                format!("{connected}: {e}")
            } else {
                connected.to_string()
            };
            vec![
                format!("Status          [ {status_label} ]"),
                format!("Account         [ {user:<10} ]"),
                format!("Playlists       [ {:>3} ]", st.playlists),
                format!("Link Account    [ Enter ]"),
                format!("Sync Now        [ Enter ]"),
                format!("Unlink          [ Enter ]"),
            ]
        }
        _ => vec![],
    };

    let category_label = SETTINGS_CATEGORIES
        .get(app.settings_category)
        .unwrap_or(&"");
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
            Style::default()
                .fg(app.theme.selection_fg)
                .bg(app.theme.selection_bg)
        } else {
            Style::default().fg(app.theme.fg)
        };
        lines.push(Line::from(Span::styled(item, style)));
    }
    lines.push(Line::from(""));
    match (app.settings_category, sel) {
        (0, 0) => lines.push(Line::from(Span::styled(" Master Volume: Press Enter to cycle (caps maximum loudness).", Style::default().fg(app.theme.fg)))),
        (0, 1) => lines.push(Line::from(Span::styled(" Volume: Use +/- keys to adjust playback volume.", Style::default().fg(app.theme.fg)))),
        (0, 2) => lines.push(Line::from(Span::styled(" Mute: Press Enter to toggle mute on/off.", Style::default().fg(app.theme.fg)))),
        (1, _) => lines.push(Line::from(Span::styled(" YouTube: Configure JS runtime, download limits & search preferences.", Style::default().fg(app.theme.fg)))),
        (2, 0) => lines.push(Line::from(Span::styled(format!(" Repeat: Press Enter to cycle (current: {:?}).", app.state.repeat), Style::default().fg(app.theme.fg)))),
        (2, 1) => lines.push(Line::from(Span::styled(" Shuffle: Press Enter to toggle shuffle on/off.", Style::default().fg(app.theme.fg)))),
        (2, 2) => {
            let cf_on = app.state.crossfade.as_ref().map(|c| c.enabled).unwrap_or(false);
            lines.push(Line::from(Span::styled(if cf_on { " Crossfade: On. Press Enter to toggle off or use C to change duration." } else { " Crossfade: Off. Press Enter to toggle on." }, Style::default().fg(app.theme.fg))));
        }
        (2, 3) => {
            let easing = app.state.crossfade.as_ref().map(|c| c.easing.name()).unwrap_or("N/A");
            lines.push(Line::from(Span::styled(format!(" Easing: Press Enter to cycle (current: {}). Controls crossfade volume curve.", easing), Style::default().fg(app.theme.fg))));
        }
        (2, 4) => {
            let eq_on = app.state.eq_enabled;
            lines.push(Line::from(Span::styled(if eq_on { " EQ: On. Press Enter to disable the equalizer." } else { " EQ: Off. Press Enter to enable the equalizer." }, Style::default().fg(app.theme.fg))));
        }
        (3, 0) => lines.push(Line::from(Span::styled(" Theme: Press Enter to open the Theme Picker (Alt+C). Drop custom themes in ~/.config/gtm/themes/*.toml.", Style::default().fg(app.theme.fg)))),
        (3, 1) => lines.push(Line::from(Span::styled(" Transparent BG: Press Enter to toggle. When on, picker backgrounds become transparent.", Style::default().fg(app.theme.fg)))),
        (3, 2) => lines.push(Line::from(Span::styled(" Sync Covers: Download missing cover art from Deezer for all library tracks.", Style::default().fg(app.theme.fg)))),
        (3, 3) => lines.push(Line::from(Span::styled(" Sync Lyrics: Fetch and save lyrics files alongside all library tracks.", Style::default().fg(app.theme.fg)))),
        (3, 4) => lines.push(Line::from(Span::styled(" Footer Preset: Press Enter to cycle. Also toggled via Alt+F. Add or override presets in ~/.config/gtm/footer.toml.", Style::default().fg(app.theme.fg)))),
        (4, 0) => lines.push(Line::from(Span::styled(" Spotify: Integration status for the linked account.", Style::default().fg(app.theme.fg)))),
        (4, 1) => lines.push(Line::from(Span::styled(" Account: Display name of the linked Spotify user.", Style::default().fg(app.theme.fg)))),
        (4, 2) => lines.push(Line::from(Span::styled(" Playlists: Number of playlists synced by the daemon.", Style::default().fg(app.theme.fg)))),
        (4, 3) => lines.push(Line::from(Span::styled(" Link Account: Press Enter to paste a Spotify access token (OAuth). Token is stored at ~/.config/gtm/spotify.json.", Style::default().fg(app.theme.fg)))),
        (4, 4) => lines.push(Line::from(Span::styled(" Sync Now: Re-fetch playlists from the Spotify Web API.", Style::default().fg(app.theme.fg)))),
        (4, 5) => lines.push(Line::from(Span::styled(" Unlink: Remove the token and disconnect the account.", Style::default().fg(app.theme.fg)))),
        _ => {}
    }

    let right_para = Paragraph::new(lines);
    f.render_widget(right_para, inner);
}

// ─── Overlay Rendering ───

fn render_picker(f: &mut ratatui::Frame, area: Rect, app: &mut App) {
    let Some(top) = app.pickers.top() else {
        return;
    };

    let picker_area = if top.id == PickerId::Help {
        area
    } else {
        // Overlay box: centered, 60% width, 70% height, with minimum size
        let picker_width = ((area.width as f64 * 0.6) as u16).max(50).min(area.width);
        let picker_height = ((area.height as f64 * 0.7) as u16).max(15).min(area.height);
        let picker_x = (area.width.saturating_sub(picker_width)) / 2;
        let picker_y = (area.height.saturating_sub(picker_height)) / 3;

        Rect {
            x: picker_x,
            y: picker_y,
            width: picker_width,
            height: picker_height,
        }
    };

    let picker_box_bg = if app.transparent_bg {
        ratatui::style::Color::Reset
    } else {
        app.theme.picker_bg
    };
    f.render_widget(Clear, picker_area);

    match top.id {
        PickerId::Queue => render_queue_picker(f, picker_area, app),
        PickerId::YTSearch => render_yt_search_picker(f, picker_area, app),
        PickerId::SearchLibrary => render_search_library_picker(f, picker_area, app),
        PickerId::About => render_about_picker(f, picker_area, app),
        PickerId::SleepTimer => render_sleep_timer_picker(f, picker_area, app),
        PickerId::CommandPalette => render_command_palette_picker(f, picker_area, app),
        PickerId::Equalizer => render_equalizer_picker(f, picker_area, app),
        PickerId::SoundEffects => render_sound_effects_picker(f, picker_area, app),
        PickerId::ThemePicker => render_theme_picker_picker(f, picker_area, app),
        PickerId::Help => render_help_picker(f, picker_area, app),
        PickerId::PlaylistSelect => render_playlist_select_picker(f, picker_area, app),
        PickerId::EditMetadata => render_edit_metadata_picker(f, picker_area, app),
        PickerId::SpotifySearch => {
            let block = Block::default()
                .borders(Borders::ALL)
                .border_type(BorderType::Plain)
                .title(" Spotify Link Token ")
                .style(Style::default().bg(picker_box_bg));
            let inner = block.inner(picker_area);
            f.render_widget(block, picker_area);

            let mut lines = Vec::new();
            lines.push(Line::from(Span::styled(
                "Paste a Spotify access token below, then press Enter.",
                Style::default().fg(app.theme.fg),
            )));
            lines.push(Line::from(Span::styled(
                "Get one from the Spotify developer dashboard (OAuth access token).",
                Style::default().fg(app.theme.fg_dim),
            )));
            lines.push(Line::from(""));
            let masked: String = "*".repeat(app.spotify_token_input.chars().count());
            if app.spotify_token_input.is_empty() {
                lines.push(Line::from(Span::styled(
                    " [ token ]",
                    Style::default().fg(app.theme.fg_dim),
                )));
            } else {
                lines.push(Line::from(Span::styled(
                    format!("  {masked}"),
                    Style::default().fg(app.theme.accent),
                )));
                lines.push(Line::from(Span::styled(
                    format!(
                        "  ({} chars entered)",
                        app.spotify_token_input.chars().count()
                    ),
                    Style::default().fg(app.theme.fg_dim),
                )));
            }
            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled(
                " Enter: link account   Esc: cancel",
                Style::default().fg(app.theme.fg_dim),
            )));
            let p = Paragraph::new(lines);
            f.render_widget(p, inner);
        }
    }
}

fn picker_help(f: &mut ratatui::Frame, area: Rect, text: &str, app: &App) {
    let help = Paragraph::new(Span::styled(text, Style::default().fg(app.theme.fg_dim)))
        .style(Style::default().bg(app.theme.picker_bg));
    let help_area = Rect {
        x: area.x,
        y: area.y + area.height - 1,
        width: area.width,
        height: 1,
    };
    f.render_widget(help, help_area);
}

fn render_queue_picker(f: &mut ratatui::Frame, area: Rect, app: &App) {
    let sel = app.pickers.top().map_or(0, |o| o.selected);

    let block = Block::default()
        .borders(Borders::ALL)
        .title(" Queue ")
        .border_type(BorderType::Plain)
        .style(Style::default().bg(if app.transparent_bg {
            ratatui::style::Color::Reset
        } else {
            app.theme.picker_bg
        }));
    let inner = block.inner(area);
    f.render_widget(block, area);

    let pane_w = inner.width as usize;
    let total = app.queue_cache.len();
    if total == 0 {
        let p = Paragraph::new("Queue is empty").style(Style::default().fg(app.theme.fg_dim));
        f.render_widget(p, inner);
        picker_help(f, inner, " [Esc] Close", app);
        return;
    }

    let visible = inner.height as usize;
    let (scroll_start, scroll_end) = centered_scroll(sel, visible, total);

    let mut lines = Vec::new();

    for i in scroll_start..scroll_end {
        let track = &app.queue_cache[i];
        let is_current = i == app.queue_cursor;
        let is_sel = i == sel;
        let prefix = if is_current {
            ">"
        } else {
            " "
        };
        let num_str = format!("{}{:02}", prefix, i + 1);
        let dur = format_duration_short(track.duration as u64);
        let label = if track.artist.is_empty() {
            track.title.clone()
        } else {
            format!("{}  {}", track.artist, track.title)
        };

        let row = format!(
            "{:>5}  {:<w$}  {:>6}",
            num_str,
            label,
            dur,
            w = pane_w.saturating_sub(16)
        );

        let style = if is_sel {
            Style::default()
                .fg(app.theme.selection_fg)
                .bg(app.theme.selection_bg)
        } else if is_current {
            Style::default()
                .fg(app.theme.accent)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default()
        };

        lines.push(Line::from(Span::styled(row, style)));
    }

    let para = Paragraph::new(lines);
    f.render_widget(para, inner);
    picker_help(
        f,
        inner,
        " [Enter] Play  [d] Remove from Queue  [Esc] Close  j/k Navigate",
        app,
    );
}

fn render_yt_search_picker(f: &mut ratatui::Frame, area: Rect, app: &App) {
    let block = Block::default()
        .borders(Borders::ALL)
        .title(" YouTube Search ")
        .border_type(BorderType::Plain)
        .style(Style::default().bg(if app.transparent_bg {
            ratatui::style::Color::Reset
        } else {
            app.theme.picker_bg
        }));
    let inner = block.inner(area);
    f.render_widget(block, area);

    let query = app.pickers.top().map_or(String::new(), |o| o.query.clone());
    let cursor = if app
        .pickers
        .top()
        .is_some_and(|o| o.id == PickerId::YTSearch)
    {
        "_"
    } else {
        ""
    };
    let loading_indicator = if app.yt_search_loading {
        let spinner = braille_spinner(app.scroll_offset);
        format!(" {} ", spinner)
    } else {
        String::new()
    };
    let search_line = Line::from(Span::styled(
        format!(" > {}{}{}", query, cursor, loading_indicator),
        Style::default().fg(app.theme.fg),
    ));

    let sel = app.pickers.top().map_or(0, |o| o.selected);
    let total = app.yt_results_cache.len();
    let visible = inner.height.saturating_sub(1) as usize; // reserve 1 line for search
    let (scroll_start, _) = if total > 0 {
        centered_scroll(sel, visible, total)
    } else {
        (0, 0)
    };
    let scroll_end = (scroll_start + visible).min(total);

    let mut lines: Vec<Line> = vec![search_line];
    for i in scroll_start..scroll_end {
        let r = &app.yt_results_cache[i];
        let dur = format_duration(r.duration as u64);
        let icon = if r.is_playlist {
            "\u{f01db} "
        } else {
            "\u{f008} "
        };
        let prefix = if i == sel { " > " } else { "   " };
        let content = format!("{prefix}{}{} - {} [{}]", icon, r.channel, r.title, dur);
        let style = if i == sel {
            Style::default()
                .fg(app.theme.selection_fg)
                .bg(app.theme.selection_bg)
        } else {
            Style::default()
        };
        lines.push(Line::from(Span::styled(content, style)));
    }

    let para = Paragraph::new(lines);
    f.render_widget(para, inner);

    let help_text = " [Enter] Play / Drill-down  [Ctrl+d] Download  [Ctrl+a] Add to Queue  [Esc] Close  Type to search (auto, 500ms)";
    let help = Paragraph::new(Span::styled(
        help_text,
        Style::default().fg(app.theme.fg_dim),
    ))
    .style(Style::default().bg(if app.transparent_bg {
        ratatui::style::Color::Reset
    } else {
        app.theme.picker_bg
    }));
    let help_area = Rect {
        x: inner.x,
        y: inner.y + inner.height.saturating_sub(1),
        width: inner.width,
        height: 1,
    };
    f.render_widget(help, help_area);
}

fn render_search_library_picker(f: &mut ratatui::Frame, area: Rect, app: &App) {
    let block = Block::default()
        .borders(Borders::ALL)
        .title(" Tracks ")
        .border_type(BorderType::Plain)
        .style(Style::default().bg(if app.transparent_bg {
            ratatui::style::Color::Reset
        } else {
            app.theme.picker_bg
        }));
    let inner = block.inner(area);
    f.render_widget(block, area);

    let query = app.pickers.top().map_or(String::new(), |o| o.query.clone());
    let filtered: Vec<&gtm_core::track::TrackInfo> = if query.is_empty() {
        app.tracks_cache.iter().collect()
    } else {
        let q = query.to_lowercase();
        app.tracks_cache
            .iter()
            .filter(|t| t.title.to_lowercase().contains(&q) || t.artist.to_lowercase().contains(&q))
            .collect()
    };

    let search_line = Line::from(Span::styled(
        format!(" > {}_", query),
        Style::default().fg(app.theme.fg),
    ));

    let sel = app
        .pickers
        .top()
        .map_or(0, |o| o.selected.min(filtered.len().saturating_sub(1)));
    let total = filtered.len();
    let visible = inner.height.saturating_sub(1) as usize;
    let (scroll_start, _) = if total > 0 {
        centered_scroll(sel, visible, total)
    } else {
        (0, 0)
    };
    let scroll_end = (scroll_start + visible).min(total);

    let mut lines: Vec<Line> = vec![search_line];
    for (i, track) in filtered.iter().enumerate().take(scroll_end).skip(scroll_start) {
        let prefix = if i == sel { " > " } else { "   " };
        let dur = format_duration(track.duration as u64);
        let content = format!("{prefix}{} - {} [{}]", track.artist, track.title, dur);
        let style = if i == sel {
            Style::default()
                .fg(app.theme.selection_fg)
                .bg(app.theme.selection_bg)
        } else {
            Style::default()
        };
        lines.push(Line::from(Span::styled(content, style)));
    }

    let para = Paragraph::new(lines);
    f.render_widget(para, inner);
    picker_help(
        f,
        inner,
        " [Enter] Play  [Esc] Close  Type to search  j/k Navigate",
        app,
    );
}

// ─── Footer ───

fn render_footer(f: &mut ratatui::Frame, area: Rect, app: &mut App) {
    match app.input_mode {
        InputMode::Normal => {
            // During tab transitions, preserve the last footer render to avoid
            // visual jumps from stale state becoming momentarily visible.
            if app.footer_cache.suppress_refresh {
                if let Some(ref cached) = app.footer_cache.last {
                    crate::footer::draw(f, area, cached);
                    return;
                }
            }
            let rendered = crate::footer::render(app);
            if let Some(ref out) = rendered {
                crate::footer::draw(f, area, out);
            } else {
                f.render_widget(
                    Paragraph::new("").style(Style::default().bg(app.theme.border)),
                    area,
                );
            }
            app.footer_cache.last = rendered;
        }
        InputMode::Searching => {
            f.render_widget(
                Paragraph::new(format!(" > {}_", app.search_query)).style(
                    Style::default()
                        .fg(app.theme.fg_bright)
                        .bg(app.theme.border),
                ),
                area,
            );
        }
    }
}

pub fn render_progress_variant(ratio: f64, width: usize, app: &App) -> String {
    crate::progress::render_progress(ratio, width, app.progress_style)
}

/// Time-synced lyrics pane on the right side of the library view.
fn render_lyrics_pane(f: &mut ratatui::Frame, area: Rect, app: &mut App) {
    let block = Block::default()
        .borders(Borders::ALL)
        .title(" Lyrics ")
        .border_type(BorderType::Plain)
        .border_style(Style::default().fg(app.theme.border));

    let inner = block.inner(area);
    f.render_widget(block, area);

    let Some(ref lyrics) = app.current_lyrics else {
        let msg_text = if app.lyrics_fetching {
            let spinner = braille_spinner(app.scroll_offset);
            format!("Fetching lyrics... {}", spinner)
        } else {
            "Press [l] to search".to_string()
        };
        let msg = Paragraph::new(msg_text)
            .alignment(Alignment::Center)
            .style(Style::default().fg(if app.lyrics_fetching {
                app.theme.accent
            } else {
                app.theme.fg_dim
            }));
        f.render_widget(msg, inner);
        return;
    };

    if lyrics.lines.is_empty() {
        let msg = Paragraph::new("No lyrics found")
            .alignment(Alignment::Center)
            .style(Style::default().fg(app.theme.fg_dim));
        f.render_widget(msg, inner);
        return;
    }

    let total = lyrics.lines.len();
    let visible = inner.height as usize;
    let current = app.lyrics_scroll;
    let scroll_start = if total <= visible {
        0
    } else if current >= visible / 2 {
        (current - visible / 2).min(total - visible)
    } else {
        0
    };
    let scroll_end = (scroll_start + visible).min(total);

    let mut lines = Vec::new();
    for i in scroll_start..scroll_end {
        let line_text = &lyrics.lines[i].text;
        let is_current = i == current;
        let style = if is_current {
            Style::default()
                .fg(app.theme.fg_bright)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(app.theme.fg_dim)
        };
        lines.push(Line::from(Span::styled(format!(" {} ", line_text), style)));
    }
    let para = Paragraph::new(lines);
    f.render_widget(para, inner);
}

/// Floating track info popup shown when scrolling in the Library list.
fn render_track_popup(f: &mut ratatui::Frame, content_area: Rect, app: &mut App) {
    let track_id = match app.track_popup_track_id {
        Some(id) => id,
        None => return,
    };
    let track = match app.tracks_cache.iter().find(|t| t.id == track_id) {
        Some(t) => t,
        None => return,
    };

    let has_cover = app.track_popup_cover.is_some();
    const COVER_W: u16 = 10;
    const COVER_H: u16 = 5;
    let text_margin = 2u16;

    // Fixed dimensions to prevent layout shift when cover loads/unloads.
    let popup_w = (COVER_W + 1 + 38 + text_margin).min(content_area.width.saturating_sub(2));
    let popup_h = COVER_H + 2;
    if popup_w < 20 || popup_h > content_area.height {
        return;
    }

    // Position at bottom-right of content area
    let popup_x = content_area.x + content_area.width.saturating_sub(popup_w + 1);
    let popup_y = content_area.y + content_area.height.saturating_sub(popup_h + 1);
    let popup_area = Rect {
        x: popup_x,
        y: popup_y,
        width: popup_w,
        height: popup_h,
    };

    let display_title = if track.title.is_empty() {
        std::path::Path::new(&track.path)
            .file_stem()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_default()
    } else {
        track.title.clone()
    };
    let display_artist = if track.artist.is_empty() {
        "Unknown"
    } else {
        &track.artist
    };
    let has_album = !track.album.is_empty();
    let dur = format_duration(track.duration as u64);
    let fav = if track.favourite { " \u{2665}" } else { "" };

    let source = if track.path.contains("/audio/spotify") {
        "Spotify"
    } else if track.path.contains("/audio/youtube") {
        "YouTube"
    } else {
        "Local"
    };

    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Plain)
        .title(format!(" Track Info{} ", fav))
        .border_style(Style::default().fg(app.theme.accent))
        .style(Style::default().bg(app.theme.picker_bg));

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
        if let Some(ref mut protocol) = app.popup_cover_stateful {
            let image = StatefulImage::new();
            f.render_stateful_widget(image, cover_area, protocol);
        } else if let Some(ref cover_bytes) = app.track_popup_cover {
            render_cover_block(f, cover_area, cover_bytes);
        }

        let text_area = split[1];
        let title_avail = text_area.width.saturating_sub(1) as usize;
        let animated_title = scroll_text(&display_title, title_avail, app.np_title_scroll, true);
        let source_label = if use_nerd_fonts() {
            match source {
                "Spotify" => " \u{f1bc} Spotify",
                "YouTube" => " \u{f167} YouTube",
                _ => " \u{f3b5} Local",
            }
        } else {
            match source {
                "Spotify" => " ♫ Spotify",
                "YouTube" => " ▶ YouTube",
                _ => " ♪ Local",
            }
        };
        let lines = vec![
            Line::from(Span::styled(
                format!(" {}", animated_title),
                Style::default()
                    .fg(app.theme.fg_bright)
                    .add_modifier(Modifier::BOLD),
            )),
            Line::from(Span::styled(
                format!(" {}", display_artist),
                Style::default().fg(app.theme.fg),
            )),
            Line::from(if has_album {
                Span::styled(
                    format!(" {}", track.album),
                    Style::default().fg(app.theme.fg),
                )
            } else {
                Span::raw("")
            }),
            Line::from(""),
            Line::from(Span::styled(
                format!(" Duration: {}", dur),
                Style::default().fg(app.theme.fg_dim),
            )),
            Line::from(Span::styled(
                format!(" Source: {}", source_label),
                Style::default().fg(app.theme.fg_dim),
            )),
        ];
        let para = Paragraph::new(lines);
        f.render_widget(para, text_area);
    } else {
        // Text only (no cover or too narrow)
        let title_avail = inner.width.saturating_sub(2) as usize;
        let animated_title = scroll_text(&display_title, title_avail, app.np_title_scroll, true);
        let source_label = if use_nerd_fonts() {
            match source {
                "Spotify" => " \u{f1bc} Spotify",
                "YouTube" => " \u{f167} YouTube",
                _ => " \u{f3b5} Local",
            }
        } else {
            match source {
                "Spotify" => " ♫ Spotify",
                "YouTube" => " ▶ YouTube",
                _ => " ♪ Local",
            }
        };
        let lines = vec![
            Line::from(Span::styled(
                format!("  {}", animated_title),
                Style::default()
                    .fg(app.theme.fg_bright)
                    .add_modifier(Modifier::BOLD),
            )),
            Line::from(Span::styled(
                format!("  {}", display_artist),
                Style::default().fg(app.theme.fg),
            )),
            Line::from(if has_album {
                Span::styled(
                    format!("  {}", track.album),
                    Style::default().fg(app.theme.fg),
                )
            } else {
                Span::raw("")
            }),
            Line::from(""),
            Line::from(Span::styled(
                format!("  Duration: {}", dur),
                Style::default().fg(app.theme.fg_dim),
            )),
            Line::from(Span::styled(
                format!("  Source: {}", source_label),
                Style::default().fg(app.theme.fg_dim),
            )),
        ];
        let para = Paragraph::new(lines);
        f.render_widget(para, inner);
    }
}

fn render_about_picker(f: &mut ratatui::Frame, area: Rect, app: &App) {
    let block = Block::default()
        .borders(Borders::ALL)
        .title(" About ")
        .border_type(BorderType::Plain)
        .style(Style::default().bg(if app.transparent_bg {
            ratatui::style::Color::Reset
        } else {
            app.theme.picker_bg
        }));
    let inner = block.inner(area);
    f.render_widget(block, area);

    let version = option_env!("CARGO_PKG_VERSION").unwrap_or("0.1.0");
    let commit = option_env!("VERGEN_GIT_SHA").unwrap_or("unknown");
    let build_date = option_env!("VERGEN_BUILD_DATE").unwrap_or("unknown");
    let rust_ver = option_env!("VERGEN_RUSTC_SEMVER").unwrap_or("unknown");
    let lib_count = app.tracks_cache.len();
    let queue_count = app.queue_cache.len();

    let lines = vec![
        Line::from(Span::styled(
            format!(" gtm {version}"),
            Style::default()
                .fg(app.theme.accent)
                .add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
        Line::from(Span::styled(
            " Copyright (C) 2025 - present, prjctimg",
            Style::default().fg(app.theme.fg_dim),
        )),
        Line::from(Span::styled(
            " License GPL-3.0",
            Style::default().fg(app.theme.fg_dim),
        )),
        Line::from(""),
        Line::from(Span::styled(
            " Build",
            Style::default().fg(app.theme.fg_dim),
        )),
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
        Line::from(Span::styled(
            " Status",
            Style::default().fg(app.theme.fg_dim),
        )),
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
            format!(
                "   Shuffle:  {}",
                if app.state.shuffle { "ON" } else { "OFF" }
            ),
            Style::default().fg(app.theme.fg_bright),
        )),
        Line::from(Span::styled(
            format!("   Repeat:   {:?}", app.state.repeat),
            Style::default().fg(app.theme.fg_bright),
        )),
    ];

    let p = Paragraph::new(lines).alignment(Alignment::Left);
    f.render_widget(p, inner);
    picker_help(f, inner, " [Esc] Close", app);
}

fn render_help_picker(f: &mut ratatui::Frame, area: Rect, app: &App) {
    let query = app.pickers.top().map_or(String::new(), |o| o.query.clone());
    let help_lines = vec![
        ("topic", "Playback"),
        ("key", "   Space        Play / Pause"),
        ("key", "   n / Ctrl+N   Next track"),
        ("key", "   p / Ctrl+P   Previous track"),
        ("key", "   s            Stop"),
        ("key", "   . / ,        Seek forward / back"),
        ("", ""),
        ("topic", "Volume"),
        ("key", "   + / =        Volume up"),
        ("key", "   -            Volume down"),
        ("key", "   m            Toggle mute"),
        ("", ""),
        ("topic", "Queue & Library"),
        ("key", "   Enter        Play selected / drill-down"),
        ("key", "   d / Del      Remove item"),
        ("key", "   F            Toggle favourite"),
        ("key", "   D            Clear queue"),
        ("key", "   /            Filter mode"),
        ("", ""),
        ("topic", "Navigation"),
        ("key", "   Tab          Toggle left/right pane focus"),
        ("key", "   j/k / arrows Move up/down"),
        ("key", "   h/l          Focus left/right pane"),
        ("key", "   ?            Toggle this help"),
        ("", ""),
        ("topic", "Overlays (Alt+key)"),
        ("key", "   Alt+Q        Queue"),
        ("key", "   Alt+Y        YouTube Search"),
        ("key", "   Alt+F        Search Library"),
        ("key", "   Alt+A        About"),
        ("key", "   Alt+C        Theme Picker"),
        ("key", "   Alt+E        Equalizer"),
        ("key", "   Alt+P        Command Palette"),
        ("key", "   Alt+Z        Sleep Timer"),
        ("key", "   Alt+X        Sound Effects"),
        ("key", "   Alt+S        Spotify Search"),
        ("", ""),
        ("topic", "Other"),
        ("key", "   q            Quit"),
        ("key", "   Q            Quit & stop daemon"),
        ("key", "   S            Toggle shuffle"),
        ("key", "   r / R        Cycle repeat"),
        ("key", "   :            Command palette"),
        ("key", "   Alt+F        Cycle footer preset"),
        ("", ""),
        ("topic", "Help"),
        ("key", "   ?            Toggle this help"),
        ("key", "   gg / G       Jump to top / bottom"),
        ("key", "   0 / $        Jump to first / last line"),
        ("key", "   /            Search"),
        ("key", "   n / N        Next / previous match"),
        ("key", "   Esc / q      Close"),
    ];

    let filtered: Vec<( &str, &str)> = if query.is_empty() {
        help_lines.iter().map(|(t, l)| (*t, *l)).collect()
    } else {
        let q = query.to_lowercase();
        help_lines
            .iter()
            .filter(|(_, l)| l.to_lowercase().contains(&q))
            .map(|(t, l)| (*t, *l))
            .collect()
    };

    let total = filtered.len();
    let sel = app.pickers.top().map_or(0, |o| o.selected);
    let visible = area.height.saturating_sub(1) as usize;
    let (scroll_start, _) = if total > 0 {
        centered_scroll(sel, visible, total)
    } else {
        (0, 0)
    };
    let scroll_end = (scroll_start + visible).min(total);

    let mut lines: Vec<Line> = Vec::new();

    let title = Line::from(Span::styled(
        " KEYBINDINGS ",
        Style::default()
            .fg(app.theme.accent)
            .add_modifier(Modifier::BOLD),
    ));
    lines.push(title);

    for (i, (kind, line)) in filtered.iter().enumerate().take(scroll_end).skip(scroll_start) {
        let is_sel = i == sel;
        let style = if is_sel {
            Style::default()
                .fg(app.theme.selection_fg)
                .bg(app.theme.selection_bg)
        } else if *kind == "topic" {
            Style::default()
                .fg(app.theme.accent)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(app.theme.fg)
        };
        lines.push(Line::from(Span::styled(*line, style)));
    }

    let para = Paragraph::new(lines);
    f.render_widget(para, area);

    let footer = if !query.is_empty() {
        format!(" /{}  [Esc] Close  ? Toggle  gg/G Top/Bottom  0/$ First/Last  n/N Next/Prev", query)
    } else {
        "[Esc] Close  ? Toggle  gg/G Top/Bottom  0/$ First/Last  / Search  n/N Next/Prev".to_string()
    };
    picker_help(f, area, &footer, app);
}

fn render_sleep_timer_picker(f: &mut ratatui::Frame, area: Rect, app: &App) {
    let picker_bg = if app.transparent_bg {
        ratatui::style::Color::Reset
    } else {
        app.theme.picker_bg
    };

    if app.sleep_timer_input_mode {
        let block = Block::default()
            .borders(Borders::ALL)
            .title(" Sleep Timer — Manual Input ")
            .border_type(BorderType::Plain)
            .border_style(Style::default().fg(app.theme.border))
            .style(Style::default().bg(picker_bg));
        let inner = block.inner(area);
        f.render_widget(block, area);
        let label = Paragraph::new(Line::from(vec![
            Span::styled(" Enter minutes: ", Style::default().fg(app.theme.fg_dim)),
            Span::styled(
                &app.sleep_timer_input_buf,
                Style::default()
                    .fg(app.theme.accent)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw("_"),
        ]));
        f.render_widget(label, inner);
        return;
    }

    let block = Block::default()
        .borders(Borders::ALL)
        .title(" Sleep Timer ")
        .border_type(BorderType::Plain)
        .border_style(Style::default().fg(app.theme.border))
        .style(Style::default().bg(picker_bg));
    let inner = block.inner(area);
    f.render_widget(block, area);

    let mins = app.sleep_timer_minutes;
    let is_active = app.sleep_timer_remaining.is_some();

    let mut lines: Vec<Line> = Vec::new();

    // Current value
    lines.push(Line::from(vec![Span::styled(
        format!("  Timer: {} minutes", mins),
        Style::default()
            .fg(app.theme.fg)
            .add_modifier(Modifier::BOLD),
    )]));
    lines.push(Line::from(""));

    // Slider
    let slider_w = inner.width.saturating_sub(4) as u32;
    let pos = if slider_w > 0 {
        ((mins as f32 / 180.0) * slider_w as f32) as u32
    } else {
        0
    };
    let filled: String = "─".repeat(pos as usize);
    let empty: String = "─".repeat((slider_w.saturating_sub(pos + 1)) as usize);
    lines.push(Line::from(vec![
        Span::styled("  ", Style::default()),
        Span::styled(filled, Style::default().fg(app.theme.success)),
        Span::styled(
            "●",
            Style::default()
                .fg(app.theme.accent)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(empty, Style::default().fg(app.theme.fg_dim)),
    ]));
    lines.push(Line::from(vec![
        Span::styled("  0m", Style::default().fg(app.theme.fg_dim)),
        Span::styled(
            format!("{:.0}m", 180.0),
            Style::default().fg(app.theme.fg_dim),
        ),
    ]));
    lines.push(Line::from(""));

    // Quick options
    let quick_opts = [5u32, 10, 15, 30, 60, 90, 120];
    let sel = app
        .pickers
        .top()
        .map_or(0, |o| o.selected.min(quick_opts.len() - 1));
    let mut spans: Vec<Span> = vec![Span::styled("  ", Style::default())];
    for (i, &m) in quick_opts.iter().enumerate() {
        let style = if i == sel {
            Style::default()
                .fg(app.theme.selection_fg)
                .bg(app.theme.selection_bg)
                .add_modifier(Modifier::BOLD)
        } else if m == mins {
            Style::default().fg(app.theme.accent)
        } else {
            Style::default().fg(app.theme.fg_dim)
        };
        spans.push(Span::styled(format!("[{}m] ", m), style));
    }
    lines.push(Line::from(spans));
    lines.push(Line::from(""));

    // Active status
    if is_active {
        if let Some(remaining) = app.sleep_timer_remaining {
            let r_mins = remaining / 60;
            let r_secs = remaining % 60;
            lines.push(Line::from(Span::styled(
                format!("  Active: {:02}:{:02} remaining", r_mins, r_secs),
                Style::default()
                    .fg(app.theme.success)
                    .add_modifier(Modifier::BOLD),
            )));
        }
    }
    lines.push(Line::from(""));

    // Controls
    lines.push(Line::from(Span::styled(
        "  h/- Decrease  l/+ Increase  i: Input  Enter: Set",
        Style::default().fg(app.theme.fg_dim),
    )));
    if is_active {
        lines.push(Line::from(Span::styled(
            "  c: Cancel Timer  Esc: Close",
            Style::default().fg(app.theme.fg_dim),
        )));
    } else {
        lines.push(Line::from(Span::styled(
            "  Esc: Close",
            Style::default().fg(app.theme.fg_dim),
        )));
    }

    let paragraph = Paragraph::new(lines);
    f.render_widget(paragraph, inner);
}

pub const COMMAND_PALETTE_COMMANDS: &[(&str, &str)] = &[
    ("\u{25b6} Play/Pause", "Space"),
    ("\u{23ed} Next Track", "n"),
    ("\u{23ee} Prev Track", "p"),
    ("\u{1f50a} Volume Up", "+"),
    ("\u{1f509} Volume Down", "-"),
    ("\u{1f507} Mute Toggle", "m"),
    ("\u{1f501} Repeat", "r"),
    ("\u{1f500} Shuffle", "S"),
    ("\u{23f9} Quit", "Q"),
    ("\u{2192} Tab Cycle", "Tab"),
    ("\u{1f3b5} Library", "1"),
    ("\u{2699} Settings", "2"),
    ("\u{1f50d} Search", "/"),
    ("\u{1f4cb} Queue O/L", "Alt+Q"),
    ("\u{25b6} YouTube O/L", "Alt+Y"),
    ("\u{1f50e} Search Lib", "Alt+F"),
    ("\u{1f39a} EQ O/L", "Alt+E"),
    ("\u{23f0} SleepTimer", "Alt+Z"),
    ("\u{1f3a8} ThemePicker", "Alt+C"),
    ("\u{1f508} Sound FX O/L", "Alt+X"),
    ("\u{2139} About O/L", "Alt+A"),
    ("\u{266b} Spotify O/L", "Alt+S"),
    ("\u{1f4dd} Fetch Lyrics", "l"),
    ("\u{2576} Progress Style", "P"),
    ("\u{1f3b6} Visualizer", "Ctrl+V"),
];

fn render_command_palette_picker(f: &mut ratatui::Frame, area: Rect, app: &App) {
    let commands = COMMAND_PALETTE_COMMANDS;

    let query = app.pickers.top().map_or(String::new(), |o| o.query.clone());
    let q = query.to_lowercase();
    let mut filtered: Vec<(&(&str, &str), usize)> = if q.is_empty() {
        commands.iter().map(|c| (c, 0)).collect()
    } else {
        commands
            .iter()
            .filter_map(|c| {
                let lower = c.0.to_lowercase();
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
            })
            .collect()
    };
    // Sort by score descending (longer match = better)
    filtered.sort_by_key(|b| std::cmp::Reverse(b.1));

    let block = Block::default()
        .borders(Borders::ALL)
        .title(" Commands ")
        .border_type(BorderType::Plain)
        .style(Style::default().bg(if app.transparent_bg {
            ratatui::style::Color::Reset
        } else {
            app.theme.picker_bg
        }));
    let inner = block.inner(area);
    f.render_widget(block, area);

    let search_line = Line::from(Span::styled(
        format!(" > {}_", query),
        Style::default().fg(app.theme.fg),
    ));

    let sel = app
        .pickers
        .top()
        .map_or(0, |o| o.selected.min(filtered.len().saturating_sub(1)));
    let total = filtered.len();
    let visible = inner.height.saturating_sub(1) as usize;
    let (scroll_start, _) = if total > 0 {
        centered_scroll(sel, visible, total)
    } else {
        (0, 0)
    };
    let scroll_end = (scroll_start + visible).min(total);

    // Pad names so keybindings line up in a column.
    let name_w = filtered
        .iter()
        .map(|((name, _), _)| name.chars().count())
        .max()
        .unwrap_or(0)
        .min(inner.width.saturating_sub(8) as usize);

    let mut lines: Vec<Line> = vec![search_line];
    for (i, ((name, key), _score)) in filtered.iter().enumerate().take(scroll_end).skip(scroll_start) {
        let prefix = if i == sel { " > " } else { "   " };
        let style = if i == sel {
            Style::default()
                .fg(app.theme.selection_fg)
                .bg(app.theme.selection_bg)
        } else {
            Style::default()
        };
        lines.push(Line::from(vec![
            Span::styled(
                format!("{prefix}{name:<width$}", name = name, width = name_w),
                style,
            ),
            Span::styled(
                format!("  [{key}]", key = key),
                Style::default().fg(app.theme.fg_dim),
            ),
        ]));
    }

    let para = Paragraph::new(lines);
    f.render_widget(para, inner);
}

fn render_equalizer_picker(f: &mut ratatui::Frame, area: Rect, app: &App) {
    let presets = [
        ("Flat", EqPreset::Flat),
        ("Pop", EqPreset::Pop),
        ("Rock", EqPreset::Rock),
        ("Jazz", EqPreset::Jazz),
        ("Classical", EqPreset::Classical),
        ("Bass", EqPreset::Bass),
        ("Vocal", EqPreset::Vocal),
        ("Electronic", EqPreset::Electronic),
        ("Hip-Hop", EqPreset::HipHop),
        ("Latin", EqPreset::Latin),
        ("Acoustic", EqPreset::Acoustic),
        ("Podcast", EqPreset::Podcast),
        ("Dance", EqPreset::Dance),
        ("Headphones", EqPreset::Headphones),
        ("Speaker", EqPreset::Speaker),
    ];

    let sel = app
        .pickers
        .top()
        .map_or(0, |o| o.selected.min(presets.len() - 1));

    let block = Block::default()
        .borders(Borders::ALL)
        .title(" Equalizer ")
        .border_type(BorderType::Plain)
        .style(Style::default().bg(if app.transparent_bg {
            ratatui::style::Color::Reset
        } else {
            app.theme.picker_bg
        }));
    let inner = block.inner(area);
    f.render_widget(block, area);

    let visible = inner.height as usize;
    let total = presets.len();
    let (scroll_start, _) = centered_scroll(sel, visible, total);
    let scroll_end = (scroll_start + visible).min(total);

    let block = Block::default()
        .borders(Borders::ALL)
        .title(" Equalizer ")
        .border_type(BorderType::Plain)
        .style(Style::default().bg(if app.transparent_bg {
            ratatui::style::Color::Reset
        } else {
            app.theme.picker_bg
        }));
    let inner = block.inner(area);
    f.render_widget(block, area);

    let list_items: Vec<ListItem> = presets
        .iter()
        .enumerate()
        .skip(scroll_start)
        .take(scroll_end - scroll_start)
        .map(|(i, (name, _))| {
            let prefix = if i == sel { " > " } else { "   " };
            let style = if i == sel {
                Style::default()
                    .fg(app.theme.selection_fg)
                    .bg(app.theme.selection_bg)
            } else if *name == app.state.eq_preset.label() {
                Style::default().fg(app.theme.success)
            } else {
                Style::default()
            };
            ListItem::new(format!("{prefix}{}", name)).style(style)
        })
        .collect();

    let list = List::new(list_items);
    f.render_widget(list, inner);
}

fn render_sound_effects_picker(f: &mut ratatui::Frame, area: Rect, app: &App) {
    let block = Block::default()
        .borders(Borders::ALL)
        .title(" Sound Effects ")
        .border_type(BorderType::Plain)
        .style(Style::default().bg(if app.transparent_bg {
            ratatui::style::Color::Reset
        } else {
            app.theme.picker_bg
        }));
    let inner = block.inner(area);
    f.render_widget(block, area);

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

    let reverb_on = app.state.reverb.enabled;

    let items = [format!("Playback Speed:  {:.1}x", app.playback_speed),
        format!("Reverb:          {}", if reverb_on { "ON" } else { "OFF" }),
        format!(
            "Crossfade:       {}",
            if crossfade_on { "ON" } else { "OFF" }
        ),
        format!("Crossfade Dur:   {}s", crossfade_dur),
        format!("EQ Preset:       {}", app.state.eq_preset.label())];

    let sel = app
        .pickers
        .top()
        .map_or(0, |o| o.selected.min(items.len() - 1));
    let visible = inner.height as usize;
    let total = items.len();
    let (scroll_start, _) = centered_scroll(sel, visible, total);
    let scroll_end = (scroll_start + visible).min(total);

    let list_items: Vec<ListItem> = items
        .iter()
        .enumerate()
        .skip(scroll_start)
        .take(scroll_end - scroll_start)
        .map(|(i, s)| {
            let prefix = if i == sel { " > " } else { "   " };
            let style = if i == sel {
                Style::default()
                    .fg(app.theme.selection_fg)
                    .bg(app.theme.selection_bg)
            } else {
                Style::default()
            };
            ListItem::new(format!("{prefix}{}", s)).style(style)
        })
        .collect();

    let list = List::new(list_items);
    f.render_widget(list, inner);
}

fn render_theme_picker_picker(f: &mut ratatui::Frame, area: Rect, app: &App) {
    let block = Block::default()
        .borders(Borders::ALL)
        .title(" Theme ")
        .border_type(BorderType::Plain)
        .style(Style::default().bg(if app.transparent_bg {
            ratatui::style::Color::Reset
        } else {
            app.theme.picker_bg
        }));
    let inner = block.inner(area);
    f.render_widget(block, area);

    let query = app.pickers.top().map_or(String::new(), |o| o.query.clone());
    let q = query.to_lowercase();

    let filtered: Vec<_> = if q.is_empty() {
        app.themes.iter().enumerate().collect()
    } else {
        app.themes
            .iter()
            .enumerate()
            .filter(|(_, entry)| {
                let lower = entry.name.to_lowercase();
                let mut qi = 0usize;
                for ch in lower.chars() {
                    if qi < q.len() && ch == q.as_bytes()[qi] as char {
                        qi += 1;
                    }
                }
                qi == q.len()
            })
            .collect()
    };

    let total = filtered.len();
    let sel = app
        .pickers
        .top()
        .map_or(0, |o| o.selected.min(total.saturating_sub(1)));

    let search_line = Line::from(Span::styled(
        format!(" > {}_", query),
        Style::default().fg(app.theme.fg),
    ));

    let visible = inner.height.saturating_sub(1) as usize;
    let (scroll_start, _) = if total > 0 {
        centered_scroll(sel, visible, total)
    } else {
        (0, 0)
    };
    let scroll_end = (scroll_start + visible).min(total);

    let mut list_items: Vec<ListItem> = vec![ListItem::new(search_line)];
    for &(i, entry) in &filtered[scroll_start..scroll_end] {
        let is_active = i == app.theme_index;
        let prefix = if i == sel { " > " } else { "   " };
        let check = if is_active { " \u{2713}" } else { "" };
        // Badge light themes so users can spot them at a glance.
        let light_badge = if entry.light { " \u{2600}" } else { "" };
        let content = format!("{}{}{}{}", prefix, entry.name, light_badge, check);
        let style = if i == sel {
            Style::default()
                .fg(app.theme.selection_fg)
                .bg(app.theme.selection_bg)
        } else if is_active {
            Style::default()
                .fg(app.theme.accent)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default()
        };
        list_items.push(ListItem::new(content).style(style));
    }

    let list = List::new(list_items);
    f.render_widget(list, inner);
}

/// Return the local time using the system clock and timezone.
pub fn local_time_str() -> String {
    let now = chrono::Local::now();
    format!(" {} | {} ", now.format("%H:%M"), now.format("%Z"))
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
pub fn braille_spinner(frame: usize) -> char {
    SPINNER_FRAMES[frame % SPINNER_FRAMES.len()]
}

/// Pick the foreground colour with enough contrast against `bg`. If the
/// requested `fg` already has sufficient contrast it is preserved — this
/// lets the footer's per-module colour mapping do something rather than
/// being thrown away in favour of a monochrome fallback.
pub fn readable_fg(fg: ratatui::style::Color, bg: ratatui::style::Color) -> ratatui::style::Color {
    fn luminance(c: &ratatui::style::Color) -> f64 {
        match c {
            ratatui::style::Color::Rgb(r, g, b) => {
                0.299 * *r as f64 + 0.587 * *g as f64 + 0.114 * *b as f64
            }
            _ => 128.0,
        }
    }
    let fg_l = luminance(&fg);
    let bg_l = luminance(&bg);
    const CONTRAST_THRESHOLD: f64 = 90.0;
    if (fg_l - bg_l).abs() >= CONTRAST_THRESHOLD {
        fg
    } else if bg_l > 128.0 {
        ratatui::style::Color::Rgb(20, 20, 20)
    } else {
        ratatui::style::Color::Rgb(240, 240, 240)
    }
}

/// Scroll text horizontally if it exceeds max_width, using a frame-based offset.
/// Only the selected item scrolls; others are truncated with "…".
fn scroll_text(text: &str, max_width: usize, frame: usize, is_selected: bool) -> String {
    if text.chars().count() <= max_width {
        return format!("{:<width$}", text, width = max_width);
    }
    if !is_selected {
        let truncated: String = text.chars().take(max_width.saturating_sub(1)).collect();
        return format!("{}…", truncated);
    }
    // Animated scroll: shift by (frame / 3) characters, wrap around.
    // Rotate on character boundaries to avoid slicing mid-UTF-8-sequence.
    let chars: Vec<char> = text.chars().collect();
    let n = chars.len();
    let scroll = (frame / 3) % n.max(1);
    let scrolled: String = chars
        .iter()
        .skip(scroll)
        .chain(chars.iter().take(scroll))
        .collect();
    scrolled.chars().take(max_width).collect()
}

// ─── Library Motion Overlays ───

fn render_playlist_select_picker(f: &mut ratatui::Frame, area: Rect, app: &App) {
    let block = Block::default()
        .borders(Borders::ALL)
        .title(" Select Playlist ")
        .border_type(BorderType::Plain)
        .style(Style::default().bg(if app.transparent_bg {
            ratatui::style::Color::Reset
        } else {
            app.theme.picker_bg
        }));
    let inner = block.inner(area);
    f.render_widget(block, area);

    let sel = app.pickers.top().map_or(0, |o| o.selected);
    let mut items: Vec<ListItem> = Vec::new();

    // "Create New" option at the top
    items.push(
        ListItem::new("  + Create New Playlist").style(Style::default().fg(app.theme.accent)),
    );

    for (i, pl) in app.playlist_cache.iter().enumerate() {
        let prefix = if i + 1 == sel { " > " } else { "   " };
        let content = format!("{}{} ({} tracks)", prefix, pl.name, pl.track_count);
        let style = if i + 1 == sel {
            Style::default()
                .fg(app.theme.selection_fg)
                .bg(app.theme.selection_bg)
        } else {
            Style::default().fg(app.theme.fg)
        };
        items.push(ListItem::new(content).style(style));
    }

    let list = List::new(items);
    f.render_widget(list, inner);

    let help_text = " [Enter] Select  [Esc] Cancel";
    picker_help(f, inner, help_text, app);
}

fn render_edit_metadata_picker(f: &mut ratatui::Frame, area: Rect, app: &App) {
    let block = Block::default()
        .borders(Borders::ALL)
        .title(" Edit Metadata ")
        .border_type(BorderType::Plain)
        .style(Style::default().bg(if app.transparent_bg {
            ratatui::style::Color::Reset
        } else {
            app.theme.picker_bg
        }));
    let inner = block.inner(area);
    f.render_widget(block, area);

    let field_names = [
        "Title",
        "Artist",
        "Album",
        "Album Artist",
        "Genre",
        "Year",
        "Track #",
    ];
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(field_names.len() as u16 * 2 + 1),
            Constraint::Min(0),
        ])
        .split(inner);

    let mut lines: Vec<Line> = Vec::new();
    for (i, name) in field_names.iter().enumerate() {
        let value = app.metadata_fields.get(i).map(|s| s.as_str()).unwrap_or("");
        let is_active = i == app.metadata_field_idx;
        let prefix = if is_active { " > " } else { "   " };
        let style = if is_active {
            Style::default()
                .fg(app.theme.selection_fg)
                .bg(app.theme.selection_bg)
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

    let help_text =
        " [j/k] Navigate fields  [Tab] Next  [Enter] Next/Save  [Ctrl+Enter] Save  [Esc] Cancel";
    picker_help(f, chunks[1], help_text, app);
}

fn render_health_panel(f: &mut ratatui::Frame, area: Rect, app: &App) {
    use ratatui::widgets::{Block, Borders, Clear};

    let panel_width = area.width.min(60);
    let panel_height = area.height.min(20);
    let x = (area.width.saturating_sub(panel_width)) / 2;
    let y = (area.height.saturating_sub(panel_height)) / 2;
    let rect = Rect::new(area.x + x, area.y + y, panel_width, panel_height);

    f.render_widget(Clear, rect);

    let block = Block::default()
        .title(" Health Check ")
        .borders(Borders::ALL)
        .style(Style::default().fg(app.theme.fg).bg(app.theme.bg));

    let inner = block.inner(rect);
    f.render_widget(block, rect);

    if let Some(ref report) = app.health_report {
        let mut lines = Vec::new();
        lines.push(Line::from(vec![
            Span::styled(
                format!("Daemon v{}", report.version),
                Style::default()
                    .fg(app.theme.accent)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!("  uptime {:.0}s", report.daemon_uptime_secs),
                Style::default().fg(app.theme.fg_dim),
            ),
        ]));
        lines.push(Line::from(""));

        for c in &report.components {
            let (icon, color) = match c.status {
                gtm_core::ipc::HealthStatus::Ok => ("✓", app.theme.success),
                gtm_core::ipc::HealthStatus::Degraded => ("⚠", app.theme.warning),
                gtm_core::ipc::HealthStatus::Error => ("✗", app.theme.error),
            };
            let mut spans = vec![
                Span::styled(format!(" {icon} "), Style::default().fg(color)),
                Span::styled(
                    c.name.clone(),
                    Style::default()
                        .fg(app.theme.fg_bright)
                        .add_modifier(Modifier::BOLD),
                ),
            ];
            if let Some(ref msg) = c.message {
                spans.push(Span::styled(
                    format!(" — {msg}"),
                    Style::default().fg(app.theme.fg_dim),
                ));
            }
            lines.push(Line::from(spans));
        }

        let para = Paragraph::new(lines).scroll((0, 0));
        f.render_widget(para, inner);
    } else {
        let loading = Paragraph::new(" Loading...").style(Style::default().fg(app.theme.fg_dim));
        f.render_widget(loading, inner);
    }

    let help = Paragraph::new(" [Esc] Close").style(Style::default().fg(app.theme.fg_dim));
    let help_area = Rect::new(
        rect.x,
        rect.y + rect.height.saturating_sub(1),
        rect.width,
        1,
    );
    f.render_widget(help, help_area);
}

#[cfg(test)]
mod tests {
    use super::scroll_text;

    #[test]
    fn scroll_text_handles_multibyte_utf8() {
        let text = "Artist \u{2014} T\u{e9}t\u{e9} Song Title That Is Quite Long";
        // Exercise a range of frame offsets so byte/char boundaries vary.
        for frame in 0..600 {
            for width in [8usize, 16, 24] {
                let out = scroll_text(text, width, frame, true);
                assert!(out.chars().count() <= width, "frame {frame} width {width}");
            }
            let out = scroll_text(text, 16, frame, false);
            assert!(out.chars().count() <= 16);
        }
    }

    #[test]
    fn scroll_text_pads_when_fits() {
        assert_eq!(scroll_text("ab", 4, 0, true), "ab  ");
    }

    #[test]
    fn scroll_text_empty_never_panics() {
        let out = scroll_text("", 0, 1, true);
        assert_eq!(out, "");
    }
}
