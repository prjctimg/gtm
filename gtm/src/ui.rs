// Copyright (c) 2026 - present
// Author: prjctimg <prjctimg@outlook.com>
// TUI rendering: tab layout, pickers, library, now-playing, settings
//
// This is free software released under the GPL-3.0 license.

use std::path::PathBuf;

use crate::app::{
    no_image_protocol, App, InputMode, LibraryPick, TrackInfoKind, LIBRARY_CATEGORIES,
};
use crate::footer::format_duration;
use crate::picker::{Picker, PickerId, PickerSource};
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use gtm_core::state::{EqPreset, Tab};
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Alignment, Constraint, Direction, Layout, Margin, Rect};
use ratatui::style::Color;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, Paragraph, Wrap};
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
        // Socket exists but daemon is dead/stale: remove it
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
            // Socket exists: try a ping to confirm daemon is responsive
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

    // Daemon didn't become ready in time: let App::new handle the connection
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
    // Explicit background fill: the TUI defines its own background
    f.render_widget(
        ratatui::widgets::Block::default().style(ratatui::style::Style::default().bg(app.theme.bg)),
        area,
    );
    // help bar shows on Library tab, hidden during pickers
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Min(0),
            Constraint::Length(1),
        ])
        .split(area);

    render_tabs(f, chunks[0], app);
    // gtm brand divider pinned to the top-right of the tab bar (T28):
    // accent background with two spaces of padding on each side.
    let brand = Paragraph::new(Span::styled(
        "  gtm  ",
        Style::default()
            .fg(crate::theme::readable_fg(app.theme.fg, app.theme.accent))
            .bg(app.theme.accent)
            .add_modifier(Modifier::BOLD),
    ))
    .alignment(Alignment::Right);
    f.render_widget(brand, chunks[0]);
    render_content(f, chunks[1], app);
    render_footer(f, chunks[2], app);

    // Render pickers on top of everything
    if app.pickers.is_open() {
        render_picker(f, area, app);
    }

    // Floating notification overlay (rendered last, on top of everything)
    render_notification_overlay(f, area, app);

    // Health check panel overlay
    if app.show_health_panel {
        render_health_panel(f, area, app);
    }

    // The animation trigger fires once per track change: keep it set for this
    // frame only, then let the EffectManager carry the effect to completion.
    app.track_anim_trigger = false;
}

fn render_tabs(f: &mut ratatui::Frame, area: Rect, app: &App) {
    let tabs: [(u16, Tab, &str); 2] =
        [(1, Tab::Library, "Library"), (2, Tab::Settings, "Settings")];
    let mut spans: Vec<Span> = Vec::new();
    for (num, tab, label) in tabs {
        let active = app.current_tab == tab;
        let text = format!(" [{num}] {label} ");
        let style = if active {
            Style::default()
                .fg(app.theme.selection_fg_readable())
                .bg(app.theme.selection_bg)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(app.theme.fg_dim)
        };
        spans.push(Span::styled(text, style));
    }
    let para = Paragraph::new(Line::from(spans));
    f.render_widget(para, area);
}

/// Easing: cubic ease-out for smooth deceleration.
fn cubic_ease_out(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    1.0 - (1.0 - t).powi(3)
}

/// Easing: cubic ease-in for the leave animation.
fn cubic_ease_in(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    t * t * t
}

/// How long a notification stays on screen before being removed.
const NOTIFICATION_LIFETIME: std::time::Duration = std::time::Duration::from_millis(1500);

/// How long the slide-out (leave) animation takes once a notification
/// expires.  During this window the card is still rendered while it drifts
/// back off the right edge of the screen.
const NOTIFICATION_EXIT_DURATION: std::time::Duration = std::time::Duration::from_millis(300);

/// "Up Next" card shown 5s before a crossfade begins (T10).  A bordered
/// panel with the next track's cover, title/artist/album, and an animated
/// top progress bar counting down to the crossfade start.
fn render_upnext_card(f: &mut ratatui::Frame, area: Rect, app: &mut App, remaining: f64) {
    let (display_title, artist, album, has_album, has_cover, source_label) = {
        let u = match app.upnext.as_ref() {
            Some(u) => u,
            None => return,
        };
        let display_title = if u.track.title.is_empty() {
            std::path::Path::new(&u.track.path)
                .file_stem()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_default()
        } else {
            u.track.title.clone()
        };
        let artist = if u.track.artist.is_empty() {
            "Unknown".to_string()
        } else {
            u.track.artist.clone()
        };
        let album = u.track.album.clone();
        let has_album = !album.is_empty();
        let has_cover = u.cover.is_some();
        let source = if u.track.path.contains("/audio/spotify")
            || u.track.path.starts_with("spotify:")
        {
            "Spotify"
        } else if u.track.path.contains("/audio/youtube") || u.track.path.starts_with("youtube:") {
            "YouTube"
        } else {
            "Local"
        };
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
        (
            display_title,
            artist,
            album,
            has_album,
            has_cover,
            source_label.to_string(),
        )
    };

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(app.theme.notification_border))
        .style(Style::default().bg(app.theme.elevated_bg));
    f.render_widget(block, area);
    let inner = area.inner(Margin {
        horizontal: 1,
        vertical: 1,
    });

    // Animated top progress bar counting down to the crossfade.
    let filled_w = (inner.width as f64 * remaining).round() as u16;
    let empty_w = inner.width.saturating_sub(filled_w);
    if filled_w > 0 {
        f.render_widget(
            Block::default().style(Style::default().bg(app.theme.accent)),
            Rect {
                x: inner.x,
                y: inner.y,
                width: filled_w,
                height: 1,
            },
        );
    }
    if empty_w > 0 {
        f.render_widget(
            Block::default().style(Style::default().bg(app.theme.muted_border)),
            Rect {
                x: inner.x + filled_w,
                y: inner.y,
                width: empty_w,
                height: 1,
            },
        );
    }

    let body = Rect {
        x: inner.x,
        y: inner.y + 1,
        width: inner.width,
        height: inner.height.saturating_sub(1),
    };

    // Cover on the left (clamped to the body).
    let cover_w = COVER_W.min(body.width.saturating_sub(2));
    let cover_h = COVER_H.min(body.height);
    if has_cover && cover_w > 0 {
        let cover_area = Rect {
            x: body.x,
            y: body.y,
            width: cover_w,
            height: cover_h,
        };
        if let Some(protocol) = app.upnext.as_mut().and_then(|u| u.cover_stateful.as_mut()) {
            let image = StatefulImage::new();
            f.render_stateful_widget(image, cover_area, protocol);
        } else if let Some(bytes) = app.upnext.as_ref().and_then(|u| u.cover.as_ref()) {
            render_cover_block(f, cover_area, bytes);
        }
    }

    // Text right of the cover.
    let text_area = Rect {
        x: body.x + cover_w + 1,
        y: body.y,
        width: body.width.saturating_sub(cover_w + 1),
        height: body.height,
    };
    let text_w = text_area.width.saturating_sub(1) as usize;
    let animated = scroll_text(&display_title, text_w.max(4), app.np_title_scroll, false);
    let mut lines: Vec<Line> = vec![
        Line::from(Span::styled(
            "UP NEXT",
            Style::default()
                .fg(app.theme.accent)
                .add_modifier(Modifier::BOLD),
        )),
        Line::from(Span::styled(
            format!(" {}", animated),
            Style::default()
                .fg(app.theme.fg_bright)
                .add_modifier(Modifier::BOLD),
        )),
        Line::from(Span::styled(
            format!(" {}", artist),
            Style::default().fg(app.theme.fg),
        )),
    ];
    if has_album {
        lines.push(Line::from(Span::styled(
            format!(" {}", album),
            Style::default().fg(app.theme.fg),
        )));
    }
    lines.push(Line::from(Span::styled(
        format!(" {}", source_label.trim_start()),
        Style::default().fg(app.theme.fg_dim),
    )));
    f.render_widget(Paragraph::new(lines), text_area);
}

/// Floating notification overlay rendered as cards stacked from the top-right.
/// Each card has a thin solid left border coloured by the theme, wrapped
/// text on an opaque fill, and no outer border.  Cards slide in from the
/// right on appearance and slide back out again as they leave. The stack is
/// capped at 3/4 of the screen height.
fn render_notification_overlay(f: &mut ratatui::Frame, area: Rect, app: &mut App) {
    // Don't show notifications when a picker is open (pickers have their own panels)
    if app.pickers.is_open() {
        return;
    }

    let now = std::time::Instant::now();

    // Advance animation progress for each notification.
    let slide_duration_ms: f32 = 300.0;
    for n in &mut app.notifications {
        if n.animation_progress < 1.0 {
            let elapsed = now
                .duration_since(n.expires_at - NOTIFICATION_LIFETIME)
                .as_millis() as f32;
            n.animation_progress = (elapsed / slide_duration_ms).min(1.0);
        }
    }

    // Keep notifications alive through their slide-out animation; fully
    // departed ones are dropped.
    app.notifications
        .retain(|n| now < n.expires_at + NOTIFICATION_EXIT_DURATION);

    let max_notif_width = 55u16;
    let padding = 2u16;
    let gap = 1u16;

    // Notifications live in the top-right corner and stack downward. The
    // overlay is capped at 3/4 of the screen height so it never hides the
    // bottom of the UI (progress, footer) or the Up Next card.
    let mut y_top = area.y + padding;
    let max_y = area.y + (area.height * 3) / 4;

    // Up Next crossfade-countdown card (T10): pinned to the bottom-right,
    // regular notifications stack above it.  The card is removed here once
    // its countdown reaches zero (the crossfade has begun).
    if let Some(remaining) = app.upnext.as_ref().and_then(|u| {
        let remaining = 1.0 - u.started_at.elapsed().as_secs_f64() / u.total_secs;
        (remaining > 0.0).then_some(remaining)
    }) {
        let card_w = 55u16;
        let card_h = 10u16;
        let card_x = area.x + area.width.saturating_sub(card_w + padding);
        let card_y = area
            .y
            .saturating_add(area.height)
            .saturating_sub(card_h + padding);
        if card_y >= area.y {
            let card_area = Rect {
                x: card_x,
                y: card_y,
                width: card_w,
                height: card_h,
            };
            render_upnext_card(f, card_area, app, remaining);
        }
    } else if app.upnext.is_some() {
        app.upnext = None;
    }

    // Separate volume and regular notifications.
    let mut regular: Vec<_> = app.notifications.iter().filter(|n| !n.is_volume).collect();
    let volume: Vec<_> = app.notifications.iter().filter(|n| n.is_volume).collect();

    // Stack regular notifications downward from the top, max 5 visible.
    regular.truncate(5);

    for n in regular.iter() {
        // Wrap text to fit within max width minus border (both sides) and padding.
        let text_area_w = max_notif_width.saturating_sub(2 + padding * 2);
        let wrapped = wrap_text(&n.message, text_area_w as usize);
        let line_count = wrapped.len() as u16;
        // A titled card spends one row on the heading (accent, bold) plus a
        // blank separator row before the message.
        let has_title = !n.title.is_empty();
        let title_rows = if has_title { 2 } else { 0 };
        let card_h = line_count + padding * 2 + title_rows;

        // Check if card fits in remaining area (3/4-height cap).
        let card_y = y_top;
        if card_y + card_h > max_y {
            break;
        }

        // Slide-in from right: start off-screen, animate to final position.
        // Once expired the card slides back out to the right.
        let final_x = area.x + area.width.saturating_sub(max_notif_width + padding);
        let start_x = area.x + area.width;
        let leaving = now.saturating_duration_since(n.expires_at);
        let x = if leaving > std::time::Duration::ZERO {
            let exit_progress = cubic_ease_in(
                (leaving.as_millis() as f32 / NOTIFICATION_EXIT_DURATION.as_millis() as f32)
                    .min(1.0),
            );
            (final_x as f32 + (start_x as f32 - final_x as f32) * exit_progress) as u16
        } else {
            let progress = cubic_ease_out(n.animation_progress);
            (start_x as f32 + (final_x as f32 - start_x as f32) * progress) as u16
        };

        let card_area = Rect {
            x,
            y: card_y,
            width: max_notif_width,
            height: card_h,
        };

        // Rounded border with a solid full-area background (PROMPT #2).
        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(app.theme.notification_border))
            .style(Style::default().bg(app.theme.elevated_bg));
        f.render_widget(block, card_area);

        // Text inside the border, padded.
        let inner = card_area.inner(Margin {
            horizontal: padding,
            vertical: padding,
        });
        let mut lines: Vec<Line> = Vec::with_capacity(1 + line_count as usize);
        if has_title {
            lines.push(Line::from(Span::styled(
                n.title.clone(),
                Style::default()
                    .fg(app.theme.accent)
                    .add_modifier(Modifier::BOLD),
            )));
            lines.push(Line::raw(""));
        }
        for l in wrapped.iter() {
            lines.push(Line::raw(l));
        }
        let para = Paragraph::new(lines).style(Style::default().fg(app.theme.fg_bright));
        f.render_widget(para, inner);

        y_top = card_y + card_h + gap;
    }

    // Volume notifications: vertical bar on the far right edge (stacked
    // below the regular cards).
    for n in volume.iter() {
        let bar_h = 10u16;
        let bar_w = 5u16;

        let final_x = area.x + area.width.saturating_sub(bar_w + padding);
        let start_x = area.x + area.width;
        let leaving = now.saturating_duration_since(n.expires_at);
        let x = if leaving > std::time::Duration::ZERO {
            let exit_progress = cubic_ease_in(
                (leaving.as_millis() as f32 / NOTIFICATION_EXIT_DURATION.as_millis() as f32)
                    .min(1.0),
            );
            (final_x as f32 + (start_x as f32 - final_x as f32) * exit_progress) as u16
        } else {
            let progress = cubic_ease_out(n.animation_progress);
            (start_x as f32 + (final_x as f32 - start_x as f32) * progress) as u16
        };

        let bar_y = y_top;
        if bar_y + bar_h + 2 > max_y {
            break;
        }
        let bar_area = Rect {
            x,
            y: bar_y,
            width: bar_w,
            height: bar_h + 2,
        };

        // Background
        f.render_widget(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(app.theme.border))
                .style(Style::default().bg(app.theme.elevated_bg)),
            bar_area,
        );

        // Fill bar (bottom-up, respecting theme volume colors)
        let fill_h = ((n.volume_value as f64 / 100.0) * bar_h as f64) as u16;
        if fill_h > 0 {
            let fill_area = Rect {
                x: bar_area.x + 1,
                y: bar_area.y + 1 + (bar_h - fill_h),
                width: bar_w.saturating_sub(2),
                height: fill_h,
            };
            let vol_color = app.theme.volume_color(n.volume_value);
            f.render_widget(
                Block::default().style(Style::default().bg(vol_color)),
                fill_area,
            );
        }

        // Percentage label below bar
        let label_area = Rect {
            x: bar_area.x,
            y: bar_area.y + bar_h + 1,
            width: bar_w,
            height: 1,
        };
        let label = Paragraph::new(format!("{:>3}%", n.volume_value))
            .style(Style::default().fg(app.theme.fg_bright));
        f.render_widget(label, label_area);

        y_top = bar_y + bar_h + 2 + gap;
    }
}

/// Wrap text to fit within `max_chars` per line, breaking at spaces when possible.
fn wrap_text(text: &str, max_chars: usize) -> Vec<String> {
    if max_chars == 0 {
        return vec![text.to_string()];
    }
    let mut lines = Vec::new();
    let mut current = String::new();
    for word in text.split(' ') {
        if current.len() + word.len() + 1 > max_chars && !current.is_empty() {
            lines.push(std::mem::take(&mut current));
        }
        if !current.is_empty() {
            current.push(' ');
        }
        current.push_str(word);
    }
    if !current.is_empty() {
        lines.push(current);
    }
    if lines.is_empty() {
        lines.push(String::new());
    }
    lines
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

/// Render the help keybinding hints right-aligned in the footer row so the
/// TUI keeps a single status line instead of a dedicated help row.
fn render_footer_help(f: &mut ratatui::Frame, area: Rect, app: &App) {
    if app.current_tab != Tab::Library || app.pickers.is_open() || app.hide_help_bar {
        return;
    }
    let text = " [?] Help  [:] Command palette  [q] Quit ";
    let para = Paragraph::new(text)
        .alignment(Alignment::Right)
        .style(Style::default().fg(app.theme.fg_dim).bg(app.theme.border));
    f.render_widget(para, area);
}

fn render_cover(
    f: &mut ratatui::Frame,
    area: Rect,
    cover_stateful: Option<&mut StatefulProtocol>,
    current_cover: Option<&[u8]>,
    placeholder_fg: Color,
) {
    // Skip image rendering in terminals without protocol passthrough
    if std::env::var("NVIM").is_ok() || std::env::var("ZELLIJ").is_ok() {
        let placeholder = Paragraph::new(Span::styled(
            " \u{266b} Cover art unavailable in this terminal ",
            Style::default().fg(placeholder_fg),
        ));
        f.render_widget(placeholder, area);
        return;
    }
    if let Some(protocol) = cover_stateful {
        let image = StatefulImage::new();
        f.render_stateful_widget(image, area, protocol);
    } else if let Some(cover_bytes) = current_cover {
        render_cover_block(f, area, cover_bytes);
    } else {
        let placeholder = Paragraph::new(Span::styled(
            " \u{266b} ",
            Style::default().fg(placeholder_fg),
        ));
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
    "\u{f001}", "\u{f004}", "\u{f025}", "\u{f007}", "\u{f03a}", "\u{f1bc}",
];

const LIBRARY_ICONS_ASCII: &[&str] = &["♫", "♥", "▤", "♪", "≡", "☊"];

fn use_nerd_fonts() -> bool {
    !matches!(std::env::var("GTM_NERD_FONTS"), Ok(v) if v == "0" || v == "false" || v == "no")
}

/// Canonical cover-art size across the TUI.  A terminal cell is roughly
/// half as tall as it is wide, so a 14×7 block reads as square artwork.
/// Every cover renderer clamps to its available space but shares these base
/// dimensions so the layout never shifts when a cover loads or unloads (T13).
pub const COVER_W: u16 = 14;
pub const COVER_H: u16 = 7;

/// 1-row-at-a-time viewport (PROMPT #10): the stored `offset` only changes by
/// ±1 when the selection leaves the visible window, so the list never
/// recenters or jumps.  Returns the new `(start, end)` range.
fn step_viewport(offset: usize, sel: usize, visible: usize, total: usize) -> (usize, usize) {
    if total <= visible || visible == 0 {
        return (0, total);
    }
    let max_start = total - visible;
    let mut o = offset.min(max_start);
    if sel < o {
        o = sel;
    } else if sel >= o + visible {
        o = (sel + 1).saturating_sub(visible).min(max_start);
    }
    (o, (o + visible).min(total))
}

/// Render `widget` into `area`, applying a tachyonfx evolve animation so the
/// text resolves from placeholder glyphs (which read as stray "escape
/// characters") into the real content.  The effect (re)starts whenever
/// `app.track_anim_trigger` is set and either this pane may animate on track
/// change (`animate_on_track_change`) or this is the very first frame
/// (`app.frame_count == 0`, i.e. startup).  While it is running, subsequent
/// refresh frames keep advancing it (keyed uniquely) until it completes, at
/// which point rendering returns to the plain `render_widget` path.  The
/// scratch buffer isolates the effect to `area` so it never leaks into
/// neighbours, and the widget is re-rendered underneath every frame so the
/// effect always resolves to clean text.
fn render_evolving<W: ratatui::widgets::Widget>(
    f: &mut ratatui::Frame,
    area: Rect,
    widget: W,
    key: &'static str,
    app: &mut App,
    animate_on_track_change: bool,
) {
    let start = app.track_anim_trigger && (animate_on_track_change || app.frame_count == 0);
    if !start && !app.anim_fx.is_running() {
        f.render_widget(widget, area);
        return;
    }
    let mut buf = ratatui::buffer::Buffer::empty(area);
    widget.render(area, &mut buf);
    if start {
        app.anim_fx.add_unique_effect(
            key,
            tachyonfx::fx::evolve_into(
                tachyonfx::fx::EvolveSymbolSet::Circles,
                (350, tachyonfx::Interpolation::QuadInOut),
            )
            .with_area(area)
            .with_filter(tachyonfx::CellFilter::All),
        );
    }
    app.anim_fx
        .process_effects(tachyonfx::Duration::from_millis(16), &mut buf, area);
    f.buffer_mut().merge(&buf);
}

/// Fill a pane body rectangle with the theme's `pane_bg` (the configurable
/// "fill area" background) before the pane content renders on top.
fn fill_pane(f: &mut ratatui::Frame, area: Rect, app: &App) {
    f.render_widget(
        ratatui::widgets::Block::default()
            .style(ratatui::style::Style::default().bg(app.theme.pane_bg)),
        area,
    );
}

/// Borderless pane header (C1/C2): a 1-row BOLD label: `accent` + a `▎`
/// edge bar when `focused`, `fg_bright` otherwise: with an optional muted
/// separator beneath.  When `left_rule` is set a thin `muted_border` vertical
/// rule is drawn along the pane's left edge and content is indented past it.
/// Returns the content rect (1-cell horizontal padding) below the header.
fn render_pane_header(
    f: &mut ratatui::Frame,
    area: Rect,
    app: &App,
    label: &str,
    focused: bool,
    sep: bool,
    left_rule: bool,
) -> Rect {
    if left_rule {
        let rule = Block::default()
            .borders(Borders::LEFT)
            .border_style(Style::default().fg(app.theme.muted_border));
        f.render_widget(rule, area);
    }
    let inset: u16 = if left_rule { 1 } else { 0 };
    let text_x = area.x + inset;
    let text_w = area.width.saturating_sub(inset);
    if focused {
        let bar = Paragraph::new(Span::styled(
            "\u{258e}",
            Style::default().fg(app.theme.accent),
        ));
        f.render_widget(
            bar,
            Rect {
                x: area.x,
                y: area.y,
                width: 1,
                height: 1,
            },
        );
    }
    let label_style = if focused {
        Style::default()
            .fg(app.theme.accent)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default()
            .fg(app.theme.fg_bright)
            .add_modifier(Modifier::BOLD)
    };
    let header = Paragraph::new(Line::from(Span::styled(format!(" {label} "), label_style)));
    f.render_widget(
        header,
        Rect {
            x: text_x,
            y: area.y,
            width: text_w,
            height: 1,
        },
    );
    let mut content = Rect {
        x: text_x + 1,
        y: area.y.saturating_add(1),
        width: text_w.saturating_sub(2),
        height: area.height.saturating_sub(1),
    };
    if sep && content.height > 0 {
        let rule = Line::from(Span::styled(
            "\u{2500}".repeat(content.width as usize),
            Style::default().fg(app.theme.muted_border),
        ));
        f.render_widget(
            Paragraph::new(rule),
            Rect {
                x: content.x,
                y: content.y,
                width: content.width,
                height: 1,
            },
        );
        content = Rect {
            x: content.x,
            y: content.y.saturating_add(1),
            width: content.width,
            height: content.height.saturating_sub(1),
        };
    }
    content
}

fn render_library(f: &mut ratatui::Frame, area: Rect, app: &mut App) {
    let is_narrow = app.terminal_cols < 60;
    let show_vis = app.visualizer.is_enabled() && app.terminal_cols >= 80;
    let np_height: u16 = if is_narrow { 5 } else { 8 };

    let lib_width: u16 = if is_narrow {
        (app.terminal_cols / 3)
            .max(12)
            .min(area.width.saturating_sub(2))
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
        .constraints([
            Constraint::Length(np_height),
            Constraint::Length(1),
            Constraint::Min(1),
        ])
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
                .split(chunks[2])
                .to_vec()
        } else {
            Layout::default()
                .direction(Direction::Horizontal)
                .constraints([Constraint::Length(0), Constraint::Min(0)])
                .split(chunks[2])
                .to_vec()
        }
    } else {
        Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Length(lib_width), Constraint::Min(0)])
            .split(chunks[2])
            .to_vec()
    };

    let left_focus = app.library_pane_focus;

    // ── Now Playing section ──
    {
        // Borderless pane: bold header + muted separator, content below.
        let np_inner = render_pane_header(f, np_area, app, "", false, true, false);
        fill_pane(f, np_inner, app);

        // Clone the current track out of the app state so the render calls
        // below (which borrow `app` mutably for the evolve animation) can
        // coexist with the track lookup.
        if let Some(track) = app.state.current_track.clone() {
            let inner = np_inner;

            if inner.width >= COVER_W + 6 {
                // Cover on the LEFT, info on the RIGHT
                let hchunks = Layout::default()
                    .direction(Direction::Horizontal)
                    .constraints([
                        Constraint::Length(COVER_W),
                        Constraint::Length(2),
                        Constraint::Min(0),
                    ])
                    .split(inner);

                // Inset the cover 1 cell from the block's left/top so it never
                // butts against the pane edge or the divider column.
                let cover_area = Rect {
                    x: hchunks[0].x + 1,
                    y: hchunks[0].y + 1,
                    width: hchunks[0].width.saturating_sub(1),
                    height: COVER_H.min(hchunks[0].height).saturating_sub(1),
                };
                render_cover(
                    f,
                    cover_area,
                    app.cover_stateful.as_mut(),
                    app.current_cover.as_deref(),
                    app.theme.fg_dim,
                );

                let info_area = hchunks[2];

                let display_title = if track.title.is_empty() {
                    std::path::Path::new(&track.path)
                        .file_stem()
                        .map(|s| s.to_string_lossy().to_string())
                        .unwrap_or_default()
                } else {
                    track.title.clone()
                };
                let display_artist = if track.artist.is_empty() {
                    " "
                } else {
                    &track.artist
                };

                // Vertically center the details block against the cover:
                // title, artist, album (when present), a spacer, progress,
                // another spacer and the timestamps row (T22).  The blank
                // rows give the progress indicator and the timestamps some
                // breathing room instead of abutting the rows above.
                let has_album = !track.album.is_empty();
                let content_h: u16 = 7;
                let offset = COVER_H.saturating_sub(content_h) / 2;
                let vchunks = Layout::default()
                    .direction(Direction::Vertical)
                    .constraints([
                        Constraint::Length(offset),
                        Constraint::Length(content_h),
                        Constraint::Min(0),
                    ])
                    .split(info_area);
                let content_area = vchunks[1];

                let info_chunks = Layout::default()
                    .direction(Direction::Vertical)
                    .constraints([
                        Constraint::Length(1),
                        Constraint::Length(1),
                        Constraint::Length(1),
                        Constraint::Length(1),
                        Constraint::Length(1),
                        Constraint::Length(1),
                        Constraint::Length(1),
                    ])
                    .split(content_area);

                // Row 0: animated title (fav icon + title)
                let fav_prefix = if track.favourite { "\u{2665} " } else { "" };
                let title_text = format!("{}{}", fav_prefix, display_title);
                let title_avail = info_chunks[0].width as usize;
                let animated_title =
                    scroll_text(&title_text, title_avail, app.np_title_scroll, true);
                let title_para = Paragraph::new(Line::from(vec![Span::styled(
                    &animated_title,
                    Style::default()
                        .fg(app.theme.fg_bright)
                        .add_modifier(Modifier::BOLD),
                )]));
                render_evolving(f, info_chunks[0], title_para, "np", app, true);

                // Row 1: Artist (icon instead of a text label)
                let artist_para = Paragraph::new(Line::from(vec![
                    Span::styled("\u{1f3a4} ", Style::default().fg(app.theme.fg)),
                    Span::styled(display_artist, Style::default().fg(app.theme.fg_bright)),
                ]));
                f.render_widget(artist_para, info_chunks[1]);

                // Row 2: Album when present (icon instead of a text label),
                // otherwise a blank spacer row.
                if has_album {
                    let album_para = Paragraph::new(Line::from(vec![
                        Span::styled("\u{1f4bf} ", Style::default().fg(app.theme.fg)),
                        Span::styled(&track.album, Style::default().fg(app.theme.fg_bright)),
                    ]));
                    f.render_widget(album_para, info_chunks[2]);
                }

                // Row 3: blank spacer above the progress indicator (T22).
                // Row 4: Progress bar.  Row 5: blank spacer above the
                // timestamps.  Row 6: elapsed/duration timestamps.  Prefer
                // the daemon-reported duration: queued/foreign tracks carry
                // duration 0 in their TrackInfo until played, so the panel
                // must not show "0:00" when the real duration is known.
                let dur = if app.state.duration > 0.0 {
                    app.state.duration
                } else {
                    track.duration
                };
                let pos = app.display_position;
                let pos_str = format_duration(pos as u64);
                let dur_str = format_duration(dur as u64);
                let ratio = if dur > 0.0 { pos / dur } else { 0.0 };
                let ts_str = format!("{} / {}", pos_str, dur_str);
                let bar_row = info_chunks[4];
                let ts_row = info_chunks[6];
                let bar_width = (bar_row.width.saturating_sub(1) as usize).min(40);
                let bar_spans = render_progress_variant_styled(ratio, bar_width, app);
                let bar_para = Paragraph::new(Line::from(bar_spans));
                f.render_widget(bar_para, bar_row);
                let ts_para = Paragraph::new(Line::from(vec![Span::styled(
                    ts_str,
                    Style::default().fg(app.theme.fg_dim),
                )]));
                f.render_widget(ts_para, ts_row);
            } else {
                // Terminal too narrow for cover: just show info text
                let display_title = if track.title.is_empty() {
                    std::path::Path::new(&track.path)
                        .file_stem()
                        .map(|s| s.to_string_lossy().to_string())
                        .unwrap_or_default()
                } else {
                    track.title.clone()
                };
                let fav_prefix = if track.favourite { "\u{2665} " } else { "" };
                let title_text = format!("{}{}", fav_prefix, display_title);
                let title_avail = inner.width as usize;
                let animated_title =
                    scroll_text(&title_text, title_avail, app.np_title_scroll, true);
                let title_para = Paragraph::new(Line::from(vec![Span::styled(
                    &animated_title,
                    Style::default()
                        .fg(app.theme.fg_bright)
                        .add_modifier(Modifier::BOLD),
                )]));
                let title_area = Rect {
                    x: inner.x,
                    y: inner.y,
                    width: inner.width,
                    height: 1,
                };
                render_evolving(f, title_area, title_para, "np", app, true);

                let mut row_offset = 1u16;
                if !track.album.is_empty() {
                    let album_para = Paragraph::new(Line::from(vec![
                        Span::styled("\u{1f4bf} ", Style::default().fg(app.theme.fg)),
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
                // Blank spacer above the progress indicator.
                row_offset += 1;

                let dur = if app.state.duration > 0.0 {
                    app.state.duration
                } else {
                    track.duration
                };
                let pos = app.display_position;
                let pos_str = format_duration(pos as u64);
                let dur_str = format_duration(dur as u64);
                let ratio = if dur > 0.0 { pos / dur } else { 0.0 };
                let ts_str = format!("{} / {}", pos_str, dur_str);
                let bar_width = (inner.width.saturating_sub(1) as usize).min(40);
                let bar_spans = render_progress_variant_styled(ratio, bar_width, app);
                let bar_para = Paragraph::new(Line::from(bar_spans));
                let bar_area = Rect {
                    x: inner.x,
                    y: inner.y + row_offset,
                    width: inner.width,
                    height: 1,
                };
                f.render_widget(bar_para, bar_area);
                // Blank spacer above the timestamps, then the timestamps on
                // their own row below the progress bar (T22).
                let ts_area = Rect {
                    x: inner.x,
                    y: inner.y + row_offset + 2,
                    width: inner.width,
                    height: 1,
                };
                let ts_para = Paragraph::new(Line::from(vec![Span::styled(
                    ts_str,
                    Style::default().fg(app.theme.fg_dim),
                )]));
                f.render_widget(ts_para, ts_area);
            }
        } else {
            let inner = np_inner;
            // Idle state (T27): point the user at the quick-start actions
            // instead of a single dead-end line.
            let lines = vec![
                Line::from(Span::styled(
                    "It's awfully quiet here…",
                    Style::default()
                        .fg(app.theme.fg_bright)
                        .add_modifier(Modifier::BOLD),
                )),
                Line::from(""),
                Line::from(Span::styled(
                    " Alt+Y  search YouTube",
                    Style::default().fg(app.theme.fg),
                )),
                Line::from(Span::styled(
                    " Alt+F  search library",
                    Style::default().fg(app.theme.fg),
                )),
                Line::from(Span::styled(
                    " Alt+S  link Spotify",
                    Style::default().fg(app.theme.fg),
                )),
                Line::from(Span::styled(
                    " Alt+C  change theme",
                    Style::default().fg(app.theme.fg),
                )),
            ];
            let msg = Paragraph::new(lines);
            // Animate the idle message once when the TUI boots (frame_count 0)
            // instead of leaving it static; golden strings keep the same text.
            render_evolving(f, inner, msg, "idle", app, false);
        }
    }

    // ── Visualizer (right of Now Playing) ──
    if let Some(vis_a) = vis_area {
        if vis_a.width >= 4 && vis_a.height >= 3 {
            app.visualizer.tick(
                app.state.status == gtm_core::state::PlaybackStatus::Playing,
                vis_a.width,
            );
            let vis_header = Paragraph::new(Line::from(Span::styled(
                " ",
                Style::default()
                    .fg(app.theme.fg_dim)
                    .add_modifier(Modifier::BOLD),
            )));
            f.render_widget(
                vis_header,
                Rect {
                    x: vis_a.x,
                    y: vis_a.y,
                    width: vis_a.width,
                    height: 1,
                },
            );
            let vis_inner = Rect {
                x: vis_a.x + 1,
                y: vis_a.y + 1,
                width: vis_a.width.saturating_sub(2),
                height: vis_a.height.saturating_sub(1),
            };
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
            let label = if count > 0 {
                format!(" {icon}  {:<14} {:>4}", cat, count)
            } else {
                format!(" {icon}  {}", cat)
            };
            let is_active = i == app.library_category;
            let style = if is_active && left_focus {
                Style::default()
                    .fg(app.theme.selection_fg_readable())
                    .bg(app.theme.selection_bg)
            } else if is_active {
                Style::default().fg(app.theme.accent)
            } else {
                Style::default().fg(app.theme.fg)
            };
            ListItem::new(label).style(style)
        })
        .collect();

    let left_inner = render_pane_header(f, panes[0], app, " ", left_focus, false, false);
    fill_pane(f, left_inner, app);

    // Reserve the bottom rows of the left pane for the embedded track-info
    // block (replaces the deprecated floating popup).  Layout from the top:
    // category list, then the track-info separator + content (cover, meta and
    // the library stats row) while a track is highlighted.
    let track_info_h: u16 = if app.track_popup_visible {
        track_info_block_height()
    } else {
        0
    };
    let left_vchunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(1),
            Constraint::Length(if app.track_popup_visible { 1 } else { 0 }),
            Constraint::Length(track_info_h),
        ])
        .split(left_inner);
    let left_list_area = left_vchunks[0];
    let left_track_info_sep_area = left_vchunks[1];
    let left_track_info_area = left_vchunks[2];

    f.render_widget(List::new(left_items), left_list_area);

    // Active category left-border indicator picker
    if app.library_category < LIBRARY_CATEGORIES.len() {
        let indicator_y = left_list_area.y + app.library_category as u16;
        if indicator_y < left_list_area.y + left_list_area.height {
            let indicator_area = Rect {
                x: left_list_area.x + 1,
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

    let (right_lines, _stats_line) = if app.browse_detail.is_some() && app.library_category == 5 {
        // Spotify playlist detail: cached track list, resolved via the daemon.
        let tracks = &app.spotify_playlist_tracks_cache;
        let total_len = tracks.len();
        let st_line = format!(" {} tracks ", total_len);
        let reserve = 3usize;
        let available = panes[1].height.saturating_sub(reserve as u16) as usize;
        app.viewport_items = available;
        let sel = app.list_pos().min(total_len.saturating_sub(1));
        let (list_scroll, end) = step_viewport(app.list_scroll, sel, available, total_len);
        app.list_scroll = list_scroll;

        let pane_w = panes[1].width as usize;
        let mut lines = vec![Line::from("")];
        if tracks.is_empty() {
            lines.push(Line::from(Span::styled(
                " No tracks: run Settings > Spotify > Sync Now, then press Enter again",
                Style::default().fg(app.theme.fg_dim),
            )));
            (lines, st_line)
        } else {
            for (i, tr) in tracks[app.list_scroll..end].iter().enumerate() {
                let real_i = app.list_scroll + i;
                let is_sel = real_i == sel && !left_focus;
                let label = tr.name.clone();
                let avail = pane_w.saturating_sub(2);
                let display_label = scroll_text(&label, avail, app.footer_title_scroll, is_sel);
                let dur = tr
                    .duration_ms
                    .map(|d| format_duration_short(d / 1000))
                    .unwrap_or_default();
                let prefix = if is_sel { " >" } else { "  " };
                let name_pad = avail.saturating_sub(9);
                let style = if is_sel {
                    Style::default()
                        .fg(app.theme.selection_fg_readable())
                        .bg(app.theme.selection_bg)
                } else {
                    Style::default().fg(app.theme.fg)
                };
                let dur_style = if is_sel {
                    Style::default()
                        .fg(app.theme.selection_fg_readable())
                        .bg(app.theme.selection_bg)
                } else {
                    Style::default().fg(app.theme.fg_dim)
                };
                lines.push(Line::from(vec![
                    Span::styled(
                        format!("{prefix}{:<width$}", display_label, width = name_pad),
                        style,
                    ),
                    Span::styled(format!("  {:>6}", dur), dur_style),
                ]));
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
        let sel = app.list_pos().min(total_len.saturating_sub(1));
        let (list_scroll, end) = step_viewport(app.list_scroll, sel, available, total_len);
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
                let label = track.title.clone();
                let avail = pane_w.saturating_sub(2);
                let display_label = scroll_text(&label, avail, app.footer_title_scroll, is_sel);
                let prefix = if is_current { "\u{25b6} " } else { "  " };
                let row = format!("{}{}", prefix, display_label);
                let style = if is_current {
                    Style::default()
                        .fg(app.theme.accent)
                        .add_modifier(Modifier::BOLD)
                } else if is_sel {
                    Style::default()
                        .fg(app.theme.selection_fg_readable())
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
        let sel = app.list_pos().min(total_len.saturating_sub(1));
        let st_line = format!(" {} albums ", total_len);
        let reserve = 3usize;
        let available = panes[1].height.saturating_sub(reserve as u16) as usize;
        app.viewport_items = available;
        let (list_scroll, end) = step_viewport(app.list_scroll, sel, available, total_len);
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
                    .fg(app.theme.selection_fg_readable())
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
        let sel = app.list_pos().min(total_len.saturating_sub(1));
        let st_line = format!(" {} artists ", total_len);
        let reserve = 3usize;
        let available = panes[1].height.saturating_sub(reserve as u16) as usize;
        app.viewport_items = available;
        let (list_scroll, end) = step_viewport(app.list_scroll, sel, available, total_len);
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
                    .fg(app.theme.selection_fg_readable())
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
        let sel = app.list_pos().min(total_len.saturating_sub(1));
        let st_line = format!(" {} playlists ", total_len);
        let reserve = 3usize;
        let available = panes[1].height.saturating_sub(reserve as u16) as usize;
        app.viewport_items = available;
        let (list_scroll, end) = step_viewport(app.list_scroll, sel, available, total_len);
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
                    .fg(app.theme.selection_fg_readable())
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
        let sel = app.list_pos().min(total_len.saturating_sub(1));
        let st_line = format!(" {} playlists ", total_len);
        let reserve = 3usize;
        let available = panes[1].height.saturating_sub(reserve as u16) as usize;
        app.viewport_items = available;
        let (list_scroll, end) = step_viewport(app.list_scroll, sel, available, total_len);
        app.list_scroll = list_scroll;
        let mut lines = vec![Line::from("")];
        if playlists.is_empty() {
            lines.push(Line::from(Span::styled(
                " No synced playlists: link an account in Settings > Spotify",
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
                        .fg(app.theme.selection_fg_readable())
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
        let sel = app.list_pos().min(total_len.saturating_sub(1));
        let (list_scroll, end) = step_viewport(app.list_scroll, sel, available, total_len);
        app.list_scroll = list_scroll;

        let filtered = app.filtered_tracks();
        let pane_w = panes[1].width as usize;

        let mut lines = vec![Line::from("")];
        for (i, track) in filtered[app.list_scroll..end].iter().enumerate() {
            let real_i = app.list_scroll + i;
            let is_current = app.state.current_track.as_ref().map(|t| t.id) == Some(track.id);
            let is_sel = real_i == sel && !left_focus;
            let label = track.title.clone();
            let avail = pane_w.saturating_sub(2);
            let display_label = scroll_text(&label, avail, app.footer_title_scroll, is_sel);
            let prefix = if is_current { "\u{25b6} " } else { "  " };
            let row = format!("{}{}", prefix, display_label);
            let style = if is_current {
                Style::default()
                    .fg(app.theme.accent)
                    .add_modifier(Modifier::BOLD)
            } else if is_sel {
                Style::default()
                    .fg(app.theme.selection_fg_readable())
                    .bg(app.theme.selection_bg)
            } else {
                Style::default()
            };
            lines.push(Line::from(Span::styled(row, style)));
        }
        (lines, st_line)
    };

    // Render the embedded track-info block (see above): the stats row now
    // lives inside the block, below the track meta.
    if app.track_popup_visible
        && (left_track_info_sep_area.height > 0 || left_track_info_area.height > 0)
    {
        render_track_info_in_pane(f, left_track_info_sep_area, left_track_info_area, app);
    }

    let right_para = Paragraph::new(right_lines);
    let header_label = if let Some(detail) = app.browse_detail.as_deref() {
        format!("▶ {detail}")
    } else {
        category_label.to_string()
    };
    let right_inner = render_pane_header(f, panes[1], app, &header_label, !left_focus, false, true);
    fill_pane(f, right_inner, app);
    render_evolving(f, right_inner, right_para, "lib", app, false);
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
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Min(0)])
        .split(area);
    let panes = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Length(22), Constraint::Min(0)])
        .split(chunks[1]);

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

    let left_inner = render_pane_header(f, panes[0], app, " ", settings_focus, false, false);
    fill_pane(f, left_inner, app);
    f.render_widget(List::new(left_items), left_inner);

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
            format!("Cookie Source   [ chromium ]"),
            format!(
                "Cookie File     [ {} ]",
                app.cookie_file.as_deref().unwrap_or("(none)")
            ),
            format!("JS Runtime      [ deno ]"),
            "Auto Download   [ read-only ]".to_string(),
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
            let reverb_on = app.state.reverb.enabled;
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
                format!(
                    "Reverb          [ {} ]",
                    if reverb_on {
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
                "Sync Metadata   [ Enter  ▶ ]".to_string(),
                format!("Footer Preset   [ {:>8} ▶ ]", preset_name),
                format!("Visualizer      [ {:>8} ▶ ]", app.visualizer.preset.name()),
                format!("Design          [ {:>7} ▶ ]", app.design.name()),
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
            let status_label = if !st.linked {
                connected.to_string()
            } else if let Some(err) = st.error.as_deref() {
                let mut e = err.chars().take(14).collect::<String>();
                if err.chars().count() > 14 {
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
                "Unavailable (needs Premium)".to_string()
            };
            let device_label = st
                .device
                .clone()
                .filter(|d| !d.is_empty())
                .unwrap_or_else(|| "(none)".to_string());
            let soloist = app.soloist_status.clone().unwrap_or_default();
            let soloist_state = if soloist.running {
                if soloist.connected {
                    if soloist.logged_in {
                        "Running ✓"
                    } else {
                        "Auth needed"
                    }
                } else {
                    "Starting…"
                }
            } else {
                "Stopped"
            };
            let auto_start = app.state.soloist_auto_start;
            let lyrics_provider = if app.state.lyrics_provider.is_empty() {
                "lrclib".to_string()
            } else {
                app.state.lyrics_provider.clone()
            };
            vec![
                if st.linked && st.premium {
                    format!("Status          [ {status_label}  Enter ]")
                } else {
                    format!("Status          [ {status_label} ]")
                },
                format!("Account         [ {user:<10} ]"),
                format!("Playlists       [ {:>3} ]", st.playlists),
                format!("Link Account    [ Enter ]"),
                format!("Sync Now        [ Enter ]"),
                format!("Unlink          [ Enter ]"),
                format!("Soloist         [ {soloist_state} ]"),
                format!("Link Soloist    [ Enter ]"),
                format!("Start Soloist   [ Enter ]"),
                format!("Stop Soloist    [ Enter ]"),
                format!("Activate Device [ Enter ]"),
                format!("Device          [ {device_label:<14} ]"),
                format!(
                    "Soloist: Auto-Start [ {} ]",
                    if auto_start { "●" } else { "○" }
                ),
                format!("Lyrics Provider [ {lyrics_provider} ]"),
            ]
        }
        _ => vec![],
    };

    let category_label = SETTINGS_CATEGORIES
        .get(app.settings_category)
        .unwrap_or(&"");
    let right_inner = render_pane_header(
        f,
        panes[1],
        app,
        category_label,
        !settings_focus,
        false,
        true,
    );
    fill_pane(f, right_inner, app);

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
        (0, 0) => lines.push(Line::from(Span::styled(" Master Volume: Press Enter to cycle (caps maximum loudness).", Style::default().fg(app.theme.fg_dim)))),
        (0, 1) => lines.push(Line::from(Span::styled(" Mute: Press Enter to toggle mute on/off.", Style::default().fg(app.theme.fg_dim)))),
        (1, 0) => lines.push(Line::from(Span::styled(" Cookie Source: Read-only. Configured via the config file.", Style::default().fg(app.theme.fg_dim)))),
        (1, 1) => lines.push(Line::from(Span::styled(" Cookie File: Press Enter to toggle the YouTube cookie path (~/.cookies/youtube.txt).", Style::default().fg(app.theme.fg_dim)))),
        (1, 2) => lines.push(Line::from(Span::styled(" JS Runtime: Read-only. Configured via the config file.", Style::default().fg(app.theme.fg_dim)))),
        (1, 3) => lines.push(Line::from(Span::styled(" Auto Download: Read-only. Download behaviour is configured via the config file.", Style::default().fg(app.theme.fg_dim)))),
        (2, 0) => lines.push(Line::from(Span::styled(format!(" Repeat: Press Enter to cycle (current: {:?}).", app.state.repeat), Style::default().fg(app.theme.fg_dim)))),
        (2, 1) => lines.push(Line::from(Span::styled(" Shuffle: Press Enter to toggle shuffle on/off.", Style::default().fg(app.theme.fg_dim)))),
        (2, 2) => {
            let cf_on = app.state.crossfade.as_ref().map(|c| c.enabled).unwrap_or(false);
            lines.push(Line::from(Span::styled(if cf_on { " Crossfade: On. Press Enter to open the crossfade picker (duration + easing)." } else { " Crossfade: Off. Press Enter to open the crossfade picker (duration + easing)." }, Style::default().fg(app.theme.fg_dim))));
        }
        (2, 3) => {
            lines.push(Line::from(Span::styled(" Easing: Press Enter to open the crossfade picker and choose the volume curve.", Style::default().fg(app.theme.fg_dim))));
        }
        (2, 4) => {
            let eq_on = app.state.eq_enabled;
            lines.push(Line::from(Span::styled(if eq_on { " EQ: On. Press Enter to disable the equalizer." } else { " EQ: Off. Press Enter to enable the equalizer." }, Style::default().fg(app.theme.fg_dim))));
        }
        (2, 5) => {
            let rev_on = app.state.reverb.enabled;
            lines.push(Line::from(Span::styled(if rev_on { " Reverb: On. Press Enter to disable the reverb effect." } else { " Reverb: Off. Press Enter to enable the reverb effect." }, Style::default().fg(app.theme.fg_dim))));
        }
        (3, 0) => lines.push(Line::from(Span::styled(" Theme: Press Enter to open the Theme Picker (Alt+C). Drop custom themes in ~/.config/gtm/themes/*.toml.", Style::default().fg(app.theme.fg_dim)))),
        (3, 1) => lines.push(Line::from(Span::styled(" Transparent BG: Press Enter to toggle. When on, picker backgrounds become transparent.", Style::default().fg(app.theme.fg_dim)))),
        (3, 2) => lines.push(Line::from(Span::styled(" Sync Covers: Download missing cover art from Deezer for all library tracks.", Style::default().fg(app.theme.fg_dim)))),
        (3, 3) => lines.push(Line::from(Span::styled(" Sync Lyrics: Fetch and save lyrics files alongside all library tracks.", Style::default().fg(app.theme.fg_dim)))),
        (3, 4) => lines.push(Line::from(Span::styled(" Sync Metadata: Resolve unreliable track metadata via Deezer and embed clean tags (title, artist, album, genre, year, track, cover) into the files.", Style::default().fg(app.theme.fg_dim)))),
        (3, 5) => lines.push(Line::from(Span::styled(" Footer Preset: Press Enter to cycle. Also available via the Command Palette. Add or override presets in ~/.config/gtm/footer.toml.", Style::default().fg(app.theme.fg_dim)))),
        (3, 6) => lines.push(Line::from(Span::styled(" Visualizer Preset: Press Enter to cycle (Braille, Blocks, Mirror, Gradient, Spectrum). Also toggled via Alt+V.", Style::default().fg(app.theme.fg_dim)))),
        (3, 7) => lines.push(Line::from(Span::styled(" Design: Press Enter to cycle between Modern and Classic layouts.", Style::default().fg(app.theme.fg_dim)))),
        (4, 0) => lines.push(Line::from(Span::styled(" Spotify: Integration status for the linked account.", Style::default().fg(app.theme.fg_dim)))),
        (4, 1) => lines.push(Line::from(Span::styled(" Account: Display name of the linked Spotify user.", Style::default().fg(app.theme.fg_dim)))),
        (4, 2) => lines.push(Line::from(Span::styled(" Playlists: Number of playlists synced by the daemon.", Style::default().fg(app.theme.fg_dim)))),
        (4, 3) => lines.push(Line::from(Span::styled(" Link Account: Press Enter to paste a Spotify access token (OAuth). Token is stored at ~/.config/gtm/spotify.json.", Style::default().fg(app.theme.fg_dim)))),
        (4, 4) => lines.push(Line::from(Span::styled(" Sync Now: Re-fetch playlists from the Spotify Web API.", Style::default().fg(app.theme.fg_dim)))),
        (4, 5) => lines.push(Line::from(Span::styled(" Unlink: Remove the token and disconnect the account.", Style::default().fg(app.theme.fg_dim)))),
        (4, 6) => lines.push(Line::from(Span::styled(" Soloist: Status of the local soloist daemon (running/connected/auth).", Style::default().fg(app.theme.fg_dim)))),
        (4, 7) => lines.push(Line::from(Span::styled(" Link Soloist: Press Enter to paste a Soloist API key (from Spotify developer dashboard). Key stored at ~/.config/gtm/soloist.key.", Style::default().fg(app.theme.fg_dim)))),
        (4, 8) => lines.push(Line::from(Span::styled(" Start Soloist: Launch the soloist daemon with the saved key.", Style::default().fg(app.theme.fg_dim)))),
        (4, 9) => lines.push(Line::from(Span::styled(" Stop Soloist: Terminate the soloist daemon (key is retained).", Style::default().fg(app.theme.fg_dim)))),
        (4, 10) => lines.push(Line::from(Span::styled(" Activate Device: Ask Soloist to become the active Spotify Connect device.", Style::default().fg(app.theme.fg_dim)))),
        _ => {}
    }

    let right_para = Paragraph::new(lines);
    f.render_widget(right_para, right_inner);
}

// ─── Overlay Rendering ───

/// Content-driven sizing hint for a floating picker: `(width, height)` the
/// picker's content needs so the panel is never wider (or taller) than
/// necessary.  Row counts are clamped so very large lists scroll instead of
/// covering the whole screen.
fn picker_content_hint(top: &Picker, app: &App) -> (u16, u16) {
    match top.id {
        PickerId::Queue => {
            let w = app
                .queue_cache
                .iter()
                .map(|t| t.artist.len() as u16 + t.title.len() as u16 + 14)
                .max()
                .unwrap_or(46)
                .clamp(44, 72);
            let h = (app.queue_cache.len() as u16 + 6).clamp(18, 30);
            (w, h)
        }
        PickerId::YTSearch => {
            let w = app
                .yt_results_cache
                .iter()
                .map(|r| r.channel.len() as u16 + r.title.len() as u16 + 22)
                .max()
                .unwrap_or(52)
                .clamp(48, 84);
            let h = (app.yt_results_cache.len() as u16 + 6).clamp(20, 32);
            (w, h)
        }
        PickerId::SearchLibrary => {
            let n = app.search_library_picks().len();
            let w = app
                .tracks_cache
                .iter()
                .map(|t| t.artist.len() as u16 + t.title.len() as u16 + 14)
                .max()
                .unwrap_or(46)
                .clamp(44, 72);
            // Reserve extra rows for the cover-art preview window.
            let h = (n as u16 + 12).clamp(24, 34);
            (w, h)
        }
        PickerId::ThemePicker => (58, 24),
        PickerId::CommandPalette => (46, 18),
        PickerId::PlaylistSelect => (48, 20),
        PickerId::SpotifySearch => (60, 12),
        PickerId::Crossfade => (58, 20),
        // Equalizer / SleepTimer / About / EditMetadata
        _ => (56, 22),
    }
}

fn render_picker(f: &mut ratatui::Frame, area: Rect, app: &mut App) {
    // Keep the SearchLibrary preview cover in sync with the highlight (the
    // fetch is guarded, so this only does work when the row changes).
    app.update_picker_preview();
    let Some(top) = app.pickers.top() else {
        return;
    };
    let top_id = top.id;
    let top_help = top.id == PickerId::Help;

    let picker_area = if top_help {
        area
    } else {
        // Content-driven size, centered on both axes. Scrolling list pickers
        // are capped at half the terminal so they never dominate the screen;
        // the old 40-row square floor made even short lists balloon. Their
        // internal renderers use `step_viewport`, so the highlighted row
        // stays in view as the viewport shrinks. Fixed-layout pickers (EQ,
        // About, EditMetadata, …) keep their content height.
        let square_min = 40
            .min(area.width.saturating_sub(2))
            .min(area.height.saturating_sub(2));
        let (content_w, content_h) = picker_content_hint(top, app);
        let picker_width = content_w.max(square_min).min(area.width.saturating_sub(2));
        let scrolling = matches!(
            top.id,
            PickerId::Queue
                | PickerId::YTSearch
                | PickerId::SearchLibrary
                | PickerId::CommandPalette
                | PickerId::ThemePicker
                | PickerId::PlaylistSelect
        );
        let picker_height = if scrolling {
            let height_cap = (area.height.saturating_sub(2) / 2).max(10);
            content_h.max((square_min / 2).max(10)).min(height_cap)
        } else {
            content_h.max(10).min(area.height.saturating_sub(2))
        };
        let picker_x = area.width.saturating_sub(picker_width) / 2;
        let picker_y = area.height.saturating_sub(picker_height) / 2;

        Rect {
            x: picker_x,
            y: picker_y,
            width: picker_width,
            height: picker_height,
        }
    };

    f.render_widget(Clear, picker_area);

    match top_id {
        PickerId::Queue => render_queue_picker(f, picker_area, app),
        PickerId::YTSearch => render_yt_search_picker(f, picker_area, app),
        PickerId::SearchLibrary => render_search_library_picker(f, picker_area, app),
        PickerId::About => render_about_picker(f, picker_area, app),
        PickerId::SleepTimer => render_sleep_timer_picker(f, picker_area, app),
        PickerId::CommandPalette => render_command_palette_picker(f, picker_area, app),
        PickerId::Equalizer => render_equalizer_picker(f, picker_area, app),
        PickerId::ThemePicker => render_theme_picker_picker(f, picker_area, app),
        PickerId::Help => render_help_picker(f, picker_area, app),
        PickerId::PlaylistSelect => render_playlist_select_picker(f, picker_area, app),
        PickerId::EditMetadata => render_edit_metadata_picker(f, picker_area, app),
        PickerId::Crossfade => render_crossfade_picker(f, picker_area, app),
        PickerId::SpotifySearch => {
            let block = picker_panel(app, " Spotify Link Token ", Some(" [Esc] Close"));
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
    let help = Paragraph::new(Span::styled(text, Style::default().fg(app.theme.fg_dim))).style(
        Style::default().bg(if app.transparent_bg {
            ratatui::style::Color::Reset
        } else {
            app.theme.elevated_bg
        }),
    );
    let help_area = Rect {
        x: area.x,
        y: area.y + area.height - 1,
        width: area.width,
        height: 1,
    };
    f.render_widget(help, help_area);
}

/// Elevated floating panel (C4): `elevated_bg` fill (Reset when transparent)
/// framed by a `muted_border` box instead of a loud full-border picker.
/// The title is rendered in the accent colour (bold) so it stays legible
/// against dark picker backgrounds (T9).
fn picker_panel<'a>(app: &App, title: &'a str, help: Option<&'a str>) -> Block<'a> {
    let mut block = Block::default()
        .borders(Borders::ALL)
        .title(Line::from(Span::styled(
            title,
            Style::default()
                .fg(app.theme.accent)
                .add_modifier(Modifier::BOLD),
        )))
        .border_style(Style::default().fg(app.theme.muted_border))
        .style(Style::default().bg(if app.transparent_bg {
            ratatui::style::Color::Reset
        } else {
            app.theme.elevated_bg
        }));
    if let Some(h) = help {
        block = block.title_bottom(Line::from(Span::styled(
            h,
            Style::default().fg(app.theme.fg_dim),
        )));
    }
    block
}

fn render_queue_picker(f: &mut ratatui::Frame, area: Rect, app: &mut App) {
    let sel = app.pickers.top().map_or(0, |o| o.selected);

    let block = picker_panel(
        app,
        " Queue ",
        Some(" [Enter] Play  [d] Remove from Queue  [Esc] Close  j/k Navigate"),
    );
    let inner = block.inner(area);
    f.render_widget(block, area);

    let total = app.queue_cache.len();
    if total == 0 {
        let p = Paragraph::new("Queue is empty").style(Style::default().fg(app.theme.fg_dim));
        f.render_widget(p, inner);
        return;
    }

    // Reserve the bottom rows for the static Up Next preview (T11): it shows
    // the track after the one currently playing and never tracks the
    // highlighter.
    let preview_h: u16 = if app.terminal_cols >= 80 { 5 } else { 0 };
    let list_area = Rect {
        x: inner.x,
        y: inner.y,
        width: inner.width,
        height: inner.height.saturating_sub(preview_h),
    };

    let visible = list_area.height as usize;
    let (scroll_start, scroll_end) = if let Some(top) = app.pickers.top_mut() {
        let (s, e) = step_viewport(top.viewport_offset, sel, visible, total);
        top.viewport_offset = s;
        (s, e)
    } else {
        (0, total)
    };

    let mut lines = Vec::new();

    for i in scroll_start..scroll_end {
        let track = &app.queue_cache[i];
        let is_current = i == app.queue_cursor;
        let is_sel = i == sel;
        let prefix = if is_sel { " > " } else { "   " };
        let icon = if is_current { "\u{25b6} " } else { "\u{266b} " };
        let label = if track.title.is_empty() {
            std::path::Path::new(&track.path)
                .file_stem()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_else(|| track.path.clone())
        } else {
            track.title.clone()
        };
        let artist = if track.artist.is_empty() {
            String::new()
        } else {
            format!(" - {}", track.artist)
        };
        let dur = format_duration_short(track.duration as u64);
        let row = format!("{prefix}{icon}{label}{artist} [{}]", dur);

        let style = if is_sel {
            Style::default()
                .fg(app.theme.selection_fg_readable())
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
    f.render_widget(para, list_area);

    if preview_h > 0 {
        let preview_area = Rect {
            x: inner.x,
            y: inner.y + inner.height - preview_h,
            width: inner.width,
            height: preview_h,
        };
        render_queue_upnext_preview(f, preview_area, app, app.queue_cursor + 1);
    }
}

/// Static "Up Next" strip at the bottom of the queue picker (T11).  Shows the
/// track after the one currently playing (with cover art when available); the
/// highlighter never changes it.
fn render_queue_upnext_preview(f: &mut ratatui::Frame, area: Rect, app: &mut App, next_idx: usize) {
    app.update_queue_preview_cover();
    let block = Block::default()
        .borders(Borders::TOP)
        .title(" Up Next ")
        .border_style(Style::default().fg(app.theme.accent))
        .style(Style::default().bg(if app.transparent_bg {
            Color::Reset
        } else {
            app.theme.elevated_bg
        }));
    f.render_widget(block, area);
    let inner = Rect {
        x: area.x,
        y: area.y + 1,
        width: area.width,
        height: area.height.saturating_sub(1),
    };
    match app.queue_cache.get(next_idx) {
        Some(track) => {
            let label = if track.title.is_empty() {
                std::path::Path::new(&track.path)
                    .file_stem()
                    .map(|s| s.to_string_lossy().to_string())
                    .unwrap_or_else(|| track.path.clone())
            } else {
                track.title.clone()
            };
            let artist = if track.artist.is_empty() {
                "Unknown artist".to_string()
            } else {
                track.artist.clone()
            };
            let album = if track.album.is_empty() {
                None
            } else {
                Some(track.album.clone())
            };
            // Cover on the left (clamped to the strip), with a glyph
            // fallback while none is available yet.
            let cover_w = COVER_W.min(inner.width.saturating_sub(2));
            let cover_h = COVER_H.min(inner.height);
            let has_cover = app.queue_preview_cover.is_some();
            if cover_w > 0 && cover_h > 0 {
                let cover_area = Rect {
                    x: inner.x + 1,
                    y: inner.y,
                    width: cover_w,
                    height: cover_h,
                };
                if has_cover {
                    if let Some(ref mut protocol) = app.queue_preview_cover_stateful {
                        let image = StatefulImage::new();
                        f.render_stateful_widget(image, cover_area, protocol);
                    } else if let Some(ref bytes) = app.queue_preview_cover {
                        render_cover_block(f, cover_area, bytes);
                    }
                } else {
                    let glyph = Paragraph::new(Line::from(Span::styled(
                        "\u{266b}",
                        Style::default().fg(app.theme.fg_dim),
                    )))
                    .alignment(Alignment::Center);
                    f.render_widget(glyph, cover_area);
                }
            }
            let text_area = Rect {
                x: inner.x + 1 + cover_w + 1,
                y: inner.y,
                width: inner.width.saturating_sub(cover_w + 2),
                height: inner.height,
            };
            let mut lines: Vec<Line> = vec![
                Line::from(Span::styled(
                    format!("  {}", label),
                    Style::default()
                        .fg(app.theme.fg_bright)
                        .add_modifier(Modifier::BOLD),
                )),
                Line::from(Span::styled(
                    format!("  {}", artist),
                    Style::default().fg(app.theme.fg),
                )),
            ];
            if let Some(album) = album {
                lines.push(Line::from(Span::styled(
                    format!("  {}", album),
                    Style::default().fg(app.theme.fg_dim),
                )));
            }
            f.render_widget(Paragraph::new(lines), text_area);
        }
        None => {
            let p = Paragraph::new("Nothing queued after this track")
                .style(Style::default().fg(app.theme.fg_dim));
            f.render_widget(p, inner);
        }
    }
}

fn render_yt_search_picker(f: &mut ratatui::Frame, area: Rect, app: &mut App) {
    let block = picker_panel(app, " YouTube Search ", Some(" [Enter] Play / Drill-down  [Ctrl+d] Download  [Ctrl+a] Add to Queue  [Esc] Close  Type to search (auto, 500ms)"));
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
        let spinner = braille_spinner(app.frame_count as usize);
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
    let (scroll_start, scroll_end) = if total > 0 {
        if let Some(top) = app.pickers.top_mut() {
            let (s, e) = step_viewport(top.viewport_offset, sel, visible, total);
            top.viewport_offset = s;
            (s, e)
        } else {
            (0, total)
        }
    } else {
        (0, 0)
    };

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
                .fg(app.theme.selection_fg_readable())
                .bg(app.theme.selection_bg)
        } else {
            Style::default()
        };
        lines.push(Line::from(Span::styled(content, style)));
    }

    let para = Paragraph::new(lines);
    f.render_widget(para, inner);
}

fn render_search_library_picker(f: &mut ratatui::Frame, area: Rect, app: &mut App) {
    let source = app.pickers.top().map_or(PickerSource::All, |o| o.source);
    let title = format!(" Search: {} ", source.label());
    let help_text = format!(
        " [Enter] Open  [Tab] Source: {}  [Esc] Close  j/k Navigate",
        source.label()
    );
    let block = picker_panel(app, &title, Some(&help_text));
    let inner = block.inner(area);
    f.render_widget(block, area);

    let picks = app.search_library_picks();

    let search_line = Line::from(Span::styled(
        format!(
            " > {}_",
            app.pickers.top().map_or(String::new(), |o| o.query.clone())
        ),
        Style::default().fg(app.theme.fg),
    ));

    // Reserve the bottom rows for the preview window (ASCII cover + meta).
    let preview_h: u16 = if app.terminal_cols >= 80 { 7 } else { 0 };
    let results_area = Rect {
        x: inner.x,
        y: inner.y,
        width: inner.width,
        height: inner.height.saturating_sub(preview_h),
    };

    let sel = app
        .pickers
        .top()
        .map_or(0, |o| o.selected.min(picks.len().saturating_sub(1)));
    let total = picks.len();
    let visible = results_area.height.saturating_sub(1) as usize;
    let (scroll_start, scroll_end) = if total > 0 {
        if let Some(top) = app.pickers.top_mut() {
            let (s, e) = step_viewport(top.viewport_offset, sel, visible, total);
            top.viewport_offset = s;
            (s, e)
        } else {
            (0, total)
        }
    } else {
        (0, 0)
    };

    let mut lines: Vec<Line> = vec![search_line];
    for (i, pick) in picks.iter().enumerate().take(scroll_end).skip(scroll_start) {
        let prefix = if i == sel { " > " } else { "   " };
        let style = if i == sel {
            Style::default()
                .fg(app.theme.selection_fg_readable())
                .bg(app.theme.selection_bg)
        } else {
            Style::default()
        };
        let text = match pick {
            LibraryPick::Track(idx) => {
                let t = &app.tracks_cache[*idx];
                let artist = if t.artist.is_empty() {
                    String::new()
                } else {
                    format!("{} - ", t.artist)
                };
                format!(
                    "{}\u{266b} {}{} [{}]",
                    prefix,
                    artist,
                    t.title,
                    format_duration(t.duration as u64)
                )
            }
            LibraryPick::Artist(name) => format!("{}\u{1f465} {}", prefix, name),
            LibraryPick::Album(album) => format!("{}\u{1f4bf} {}", prefix, album),
            LibraryPick::Playlist(i) => {
                format!("{}\u{1f4dc} {}", prefix, app.playlist_cache[*i].name)
            }
        };
        lines.push(Line::from(Span::styled(text, style)));
    }

    let para = Paragraph::new(lines);
    f.render_widget(para, results_area);

    // Preview window: actual cover art as ASCII art next to the track meta.
    if preview_h > 0 {
        let preview_area = Rect {
            x: inner.x,
            y: inner.y + inner.height - preview_h,
            width: inner.width,
            height: preview_h,
        };
        render_search_preview(f, preview_area, app, &picks, sel);
    }
}

/// Bottom preview pane of the SearchLibrary picker: ASCII cover art on the
/// left, track metadata on the right.
fn render_search_preview(
    f: &mut ratatui::Frame,
    area: Rect,
    app: &mut App,
    picks: &[crate::app::LibraryPick],
    sel: usize,
) {
    let rule = Line::from(Span::styled(
        "\u{2500}".repeat(area.width as usize),
        Style::default().fg(app.theme.muted_border),
    ));
    f.render_widget(Paragraph::new(rule), area);

    let body = Rect {
        x: area.x,
        y: area.y.saturating_add(1),
        width: area.width,
        height: area.height.saturating_sub(1),
    };
    let cover_w = 20u16.min(body.width.saturating_sub(24).max(8));
    let hchunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Length(cover_w), Constraint::Min(0)])
        .split(body);

    let cover_area = Rect {
        x: hchunks[0].x + 1,
        y: hchunks[0].y,
        width: hchunks[0].width.saturating_sub(1),
        height: hchunks[0].height,
    };
    if let Some(protocol) = app.picker_preview_stateful.as_mut() {
        let image = StatefulImage::new();
        f.render_stateful_widget(image, cover_area, protocol);
    } else if let Some(bytes) = app.picker_preview_cover.as_deref() {
        render_cover_block(f, cover_area, bytes);
    } else {
        let placeholder = Paragraph::new(Line::from(Span::styled(
            format!("{:^width$}", "\u{266b}", width = cover_w as usize),
            Style::default().fg(app.theme.fg_dim),
        )));
        f.render_widget(placeholder, cover_area);
    }

    let meta_area = hchunks[1];
    let mut meta_lines = Vec::new();
    let mut push = |key: &str, value: &str| {
        meta_lines.push(Line::from(vec![
            Span::styled(format!("{key:>9} "), Style::default().fg(app.theme.fg_dim)),
            Span::styled(value.to_string(), Style::default().fg(app.theme.fg_bright)),
        ]));
    };
    match picks.get(sel) {
        Some(LibraryPick::Track(i)) => {
            let t = &app.tracks_cache[*i];
            let display_title = if t.title.is_empty() {
                std::path::Path::new(&t.path)
                    .file_stem()
                    .map(|s| s.to_string_lossy().to_string())
                    .unwrap_or_default()
            } else {
                t.title.clone()
            };
            push("Title", &display_title);
            push(
                "Artist",
                if t.artist.is_empty() {
                    "Unknown"
                } else {
                    &t.artist
                },
            );
            push(
                "Album",
                if t.album.is_empty() {
                    "Unknown"
                } else {
                    &t.album
                },
            );
            push("Length", &format_duration(t.duration as u64));
        }
        Some(LibraryPick::Artist(name)) => {
            let count = app
                .tracks_cache
                .iter()
                .filter(|t| t.artist.eq_ignore_ascii_case(name))
                .count();
            push("Artist", name);
            push("Tracks", &count.to_string());
        }
        Some(LibraryPick::Album(album)) => {
            let count = app
                .tracks_cache
                .iter()
                .filter(|t| t.album.eq_ignore_ascii_case(album))
                .count();
            push("Album", album);
            push("Tracks", &count.to_string());
        }
        Some(LibraryPick::Playlist(i)) => {
            let p = &app.playlist_cache[*i];
            push("Playlist", &p.name);
            push("Tracks", &p.track_count.to_string());
        }
        None => {
            push("", "No results");
        }
    }
    f.render_widget(Paragraph::new(meta_lines), meta_area);
}

// ─── Footer ───

fn render_footer(f: &mut ratatui::Frame, area: Rect, app: &mut App) {
    match app.input_mode {
        InputMode::Normal => {
            // A committed library filter (from `/`) stays active; keep the
            // query visible so the user knows the list is filtered. Esc clears.
            if !app.search_query.is_empty() {
                f.render_widget(
                    Paragraph::new(format!(" > {}  [Esc] clear filter", app.search_query)).style(
                        Style::default()
                            .fg(app.theme.fg_bright)
                            .bg(app.theme.border),
                    ),
                    area,
                );
                return;
            }
            // During tab transitions, preserve the last footer render to avoid
            // visual jumps from stale state becoming momentarily visible.
            if app.footer_cache.suppress_refresh {
                if let Some(ref cached) = app.footer_cache.last {
                    crate::footer::draw(f, area, cached);
                    render_footer_help(f, area, app);
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
            render_footer_help(f, area, app);
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

/// Render the progress bar as styled spans (supports per-character coloring
/// for the `TrueGradient` style).
pub fn render_progress_variant_styled<'a>(
    ratio: f64,
    width: usize,
    app: &App,
) -> Vec<ratatui::text::Span<'a>> {
    crate::progress::render_progress_styled(
        ratio,
        width,
        app.progress_style,
        app.theme.accent,
        app.theme.secondary_accent,
        app.theme.tertiary_accent,
    )
}

/// Time-synced lyrics pane on the right side of the library view.
fn render_lyrics_pane(f: &mut ratatui::Frame, area: Rect, app: &mut App) {
    let inner = render_pane_header(f, area, app, "LYRICS", app.lyrics_pane_focus, false, true);
    fill_pane(f, inner, app);

    let Some(ref lyrics) = app.current_lyrics else {
        let msg_text = if app.lyrics_fetching {
            let spinner = braille_spinner(app.frame_count as usize);
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
    let width = inner.width.max(1) as usize;
    // A lyric line renders as "<marker> <text>" (" > " for the active line).
    // B7 wraps long lines, so compute per-line display-row heights and keep
    // the scroll window in display-row space (not logical LRC lines).
    let mut row_offsets = Vec::with_capacity(total);
    let mut text = Vec::with_capacity(total);
    let mut cumulative = 0usize;
    for (i, line) in lyrics.lines.iter().enumerate() {
        let is_current = i == app.lyrics_scroll;
        // Active-verse left marker is a bright accent so the playing line
        // stands out against the muted gutter of the other rows.
        let marker_ch = if is_current { ">" } else { " " };
        let marker_fg = if is_current {
            app.theme.secondary_accent
        } else {
            app.theme.fg_dim
        };
        let text_style = if is_current {
            Style::default()
                .fg(app.theme.fg_bright)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(app.theme.fg_dim)
        };
        row_offsets.push(cumulative);
        let rendered = format!("{marker_ch} {}", line.text);
        cumulative += (rendered.chars().count().max(1)).div_ceil(width);
        text.push(Line::from(vec![
            Span::styled(
                marker_ch,
                Style::default().fg(marker_fg).add_modifier(Modifier::BOLD),
            ),
            Span::styled(format!(" {}", line.text), text_style),
        ]));
    }
    let total_rows = cumulative;
    let visible = inner.height as usize;
    let anchor = app.lyrics_scroll.min(total - 1);
    let bottom = total_rows.saturating_sub(visible);
    let scroll_display = if total_rows <= visible {
        0
    } else if app.lyrics_manual_scroll {
        if anchor == total - 1 {
            // Last line reached: snap to the bottom so a wrapped tail is
            // never clipped by centering.
            bottom
        } else {
            // Free manual scroll keeps the active line roughly centered.
            row_offsets[anchor].saturating_sub(visible / 2).min(bottom)
        }
    } else {
        // Auto-follow: active line pinned to the top of the pane.
        row_offsets[anchor].min(bottom)
    };

    let para = Paragraph::new(text)
        .wrap(Wrap { trim: false })
        .scroll((scroll_display as u16, 0));
    f.render_widget(para, inner);
}

/// Height of the track-info card embedded at the bottom of the left Library
/// pane: one border row plus the five content rows (title, artist, album,
/// spacer, duration/source).  The cover art is vertically centred to match.
/// Height of the track-info block content reserved at the bottom of the left
/// Library pane when an image protocol is available: cover art (7 rows) plus
/// title, artist, album, duration, a spacer and the library stats row.  The
/// separator line above is sized separately in the pane layout.
const TRACK_INFO_CARD_H: u16 = 14;

/// Height of the track-info block in cover-less terminals (no image
/// protocol): title, artist, album, duration and the stats row, one row each.
const TRACK_INFO_TEXT_H: u16 = 6;

/// Height of the track-info block for the current terminal.  Image-protocol
/// detection is session-constant, so the pane height never shifts mid-render.
fn track_info_block_height() -> u16 {
    if no_image_protocol() {
        TRACK_INFO_TEXT_H
    } else {
        TRACK_INFO_CARD_H
    }
}

/// Library stats label (" X tracks | Xh Xm " / " X albums " / ...) for the
/// list rendered in the right pane, mirroring the per-category renderer
/// branches so the block below the track meta stays consistent.
fn library_stats_line(app: &App) -> String {
    if app.browse_detail.is_some() {
        if app.library_category == 5 {
            return format!(" {} tracks ", app.spotify_playlist_tracks_cache.len());
        }
        let f = app.filtered_tracks();
        let total_dur: u64 = f.iter().map(|t| t.duration as u64).sum();
        return format!(
            " {} tracks | {}h {}m ",
            f.len(),
            total_dur / 3600,
            (total_dur % 3600) / 60
        );
    }
    match app.library_category {
        2 => format!(" {} albums ", app.unique_albums().len()),
        3 => format!(" {} artists ", app.unique_artists().len()),
        4 => format!(" {} playlists ", app.playlist_cache.len()),
        5 => format!(" {} playlists ", app.spotify_playlists.len()),
        _ => {
            let f = app.filtered_tracks();
            let total_dur: u64 = f.iter().map(|t| t.duration as u64).sum();
            format!(
                " {} tracks | {}h {}m ",
                f.len(),
                total_dur / 3600,
                (total_dur % 3600) / 60
            )
        }
    }
}

/// Source label for the meta line (nerd-font aware, leading space trimmed by
/// the caller through `meta_line`).
fn source_label(use_nerd: bool, source: &str) -> String {
    if use_nerd {
        match source {
            "Spotify" => " \u{f1bc} Spotify".to_string(),
            "YouTube" => " \u{f167} YouTube".to_string(),
            _ => " \u{f3b5} Local".to_string(),
        }
    } else {
        match source {
            "Spotify" => " ♫ Spotify".to_string(),
            "YouTube" => " ▶ YouTube".to_string(),
            _ => " ♪ Local".to_string(),
        }
    }
}

/// Display fields for the library track-info block, context aware of the
/// active list type (PROMPT #20).  Returns `None` when the selected row is
/// out of range, which lets the caller fall back to the stats-only block.
struct TrackInfoFields {
    title: String,
    artist: String,
    album: Option<String>,
    /// Right meta line content: `[3:45] | Local` or `[12 tracks] | Spotify`.
    meta: String,
    /// Favourite marker appended to the separator label.
    fav: String,
    /// Whether cover art is currently loaded for the block.
    has_cover: bool,
}

fn track_info_fields(app: &App) -> Option<TrackInfoFields> {
    let use_nerd = use_nerd_fonts();
    match app.track_info_kind() {
        TrackInfoKind::Track => {
            let track = app
                .track_popup_track_id
                .and_then(|id| app.tracks_cache.iter().find(|t| t.id == id))?;
            let title = if track.title.is_empty() {
                std::path::Path::new(&track.path)
                    .file_stem()
                    .map(|s| s.to_string_lossy().to_string())
                    .unwrap_or_default()
            } else {
                track.title.clone()
            };
            let artist = if track.artist.is_empty() {
                "Unknown".to_string()
            } else {
                track.artist.clone()
            };
            let album = if track.album.is_empty() {
                None
            } else {
                Some(track.album.clone())
            };
            let source = if track.path.contains("/audio/spotify") || track.path.starts_with("spotify:")
            {
                "Spotify"
            } else if track.path.contains("/audio/youtube") || track.path.starts_with("youtube:") {
                "YouTube"
            } else {
                "Local"
            };
            let meta = format!(
                " [{}] | {}",
                format_duration(track.duration as u64),
                source_label(use_nerd, source).trim_start()
            );
            let fav = if track.favourite { " \u{2665}" } else { "" };
            Some(TrackInfoFields {
                title,
                artist,
                album,
                meta,
                fav: fav.to_string(),
                has_cover: app.track_popup_cover.is_some(),
            })
        }
        TrackInfoKind::Album => {
            let albums = app.unique_albums();
            let pos = app.list_pos();
            let (name, count) = albums.get(pos)?;
            let artist = app
                .tracks_cache
                .iter()
                .find(|t| {
                    let album: &str = if t.album.is_empty() { "Unknown Album" } else { &t.album };
                    album == name
                })
                .map(|t| {
                    if t.artist.is_empty() {
                        "Unknown".to_string()
                    } else {
                        t.artist.clone()
                    }
                })
                .unwrap_or_default();
            Some(TrackInfoFields {
                title: name.clone(),
                artist,
                album: None,
                meta: format!(
                    " [{} tracks] | {}",
                    count,
                    source_label(use_nerd, "Local").trim_start()
                ),
                fav: String::new(),
                has_cover: app.track_popup_cover.is_some(),
            })
        }
        TrackInfoKind::Artist => {
            let artists = app.unique_artists();
            let pos = app.list_pos();
            let (name, count) = artists.get(pos)?;
            Some(TrackInfoFields {
                title: name.clone(),
                artist: String::new(),
                album: None,
                meta: format!(
                    " [{} tracks] | {}",
                    count,
                    source_label(use_nerd, "Local").trim_start()
                ),
                fav: String::new(),
                has_cover: app.track_popup_cover.is_some(),
            })
        }
        TrackInfoKind::Playlist => {
            let playlists = &app.playlist_cache;
            let pos = app.list_pos();
            let pl = playlists.get(pos)?;
            Some(TrackInfoFields {
                title: pl.name.clone(),
                artist: String::new(),
                album: None,
                meta: format!(
                    " [{} tracks] | {}",
                    pl.track_count,
                    source_label(use_nerd, "Local").trim_start()
                ),
                fav: String::new(),
                has_cover: false,
            })
        }
        TrackInfoKind::SpotifyPlaylist => {
            let playlists = &app.spotify_playlists;
            let pos = app.list_pos();
            let pl = playlists.get(pos)?;
            Some(TrackInfoFields {
                title: pl.name.clone(),
                artist: pl.owner.clone(),
                album: None,
                meta: format!(
                    " [{} tracks] | {}",
                    pl.tracks.len(),
                    source_label(use_nerd, "Spotify").trim_start()
                ),
                fav: String::new(),
                has_cover: false,
            })
        }
        TrackInfoKind::SpotifyTrack => {
            let tracks = &app.spotify_playlist_tracks_cache;
            let pos = app.list_pos();
            let st = tracks.get(pos)?;
            let dur = st
                .duration_ms
                .map(|ms| format!(" [{}]", format_duration(ms / 1000)))
                .unwrap_or_default();
            Some(TrackInfoFields {
                title: st.name.clone(),
                artist: st.artists.clone(),
                album: None,
                meta: format!(
                    "{} | {}",
                    dur,
                    source_label(use_nerd, "Spotify").trim_start()
                ),
                fav: String::new(),
                has_cover: false,
            })
        }
    }
}

/// Track info block rendered at the bottom of the left Library pane, directly
/// on the pane background (no container with its own background).  This
/// replaces the deprecated floating popup so the lyrics pane and
/// notifications are never covered.  Layout: a separator line, cover art on
/// top, then the meta rows (title / artist / album / [duration | source]) and
/// finally the library stats row.
fn render_track_info_in_pane(
    f: &mut ratatui::Frame,
    sep_area: Rect,
    area: Rect,
    app: &mut App,
) {
    let fields = match track_info_fields(app) {
        Some(fields) => fields,
        None => return,
    };
    let has_cover = fields.has_cover;
    let can_cover = !no_image_protocol() && area.width > COVER_W + 1;

    // Separator line on the pane background, replacing the old bordered card.
    let sep_w = sep_area.width.saturating_sub(1) as usize;
    let sep_label = format!(" Track Info{} ", fields.fav);
    let sep_style = Style::default().fg(app.theme.fg_dim);
    let sep_line = if sep_w >= sep_label.len() {
        let side = (sep_w - sep_label.len()) / 2;
        let extra = (sep_w - sep_label.len()) % 2;
        format!("{}{}{}", "─".repeat(side), sep_label, "─".repeat(side + extra))
    } else {
        sep_label
    };
    f.render_widget(
        Paragraph::new(Line::from(Span::styled(sep_line, sep_style))),
        sep_area,
    );

    let stats_line = library_stats_line(app);

    if can_cover {
        // Cover on top, meta + stats below (no layout shift when a cover
        // loads, since the block height is fixed for this session).
        let split = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(COVER_H + 1), Constraint::Min(0)])
            .split(area);

        let cover_area = Rect {
            x: split[0].x,
            y: split[0].y,
            width: COVER_W.min(split[0].width),
            height: COVER_H.min(split[0].height),
        };
        if has_cover {
            if let Some(ref mut protocol) = app.popup_cover_stateful {
                let image = StatefulImage::new();
                f.render_stateful_widget(image, cover_area, protocol);
            } else if let Some(ref cover_bytes) = app.track_popup_cover {
                render_cover_block(f, cover_area, cover_bytes);
            }
        }

        let text_area = split[1];
        let title_avail = text_area.width.saturating_sub(1) as usize;
        let animated_title = scroll_text(&fields.title, title_avail, app.np_title_scroll, true);
        let lines = vec![
            Line::from(Span::styled(
                format!(" {}", animated_title),
                Style::default()
                    .fg(app.theme.fg_bright)
                    .add_modifier(Modifier::BOLD),
            )),
            Line::from(Span::styled(
                format!(" {}", fields.artist),
                Style::default().fg(app.theme.fg),
            )),
            Line::from(if let Some(album) = &fields.album {
                Span::styled(format!(" {}", album), Style::default().fg(app.theme.fg))
            } else {
                Span::raw("")
            }),
            Line::from(Span::styled(fields.meta.clone(), Style::default().fg(app.theme.fg_dim))),
            Line::from(""),
            Line::from(Span::styled(
                stats_line.trim(),
                Style::default().fg(app.theme.fg_dim),
            )),
        ];
        let para = Paragraph::new(lines);
        f.render_widget(para, text_area);
    } else {
        // Text only (no cover or too narrow): meta then stats.
        let title_avail = area.width.saturating_sub(2) as usize;
        let animated_title = scroll_text(&fields.title, title_avail, app.np_title_scroll, true);
        let lines = vec![
            Line::from(Span::styled(
                format!("  {}", animated_title),
                Style::default()
                    .fg(app.theme.fg_bright)
                    .add_modifier(Modifier::BOLD),
            )),
            Line::from(Span::styled(
                format!("  {}", fields.artist),
                Style::default().fg(app.theme.fg),
            )),
            Line::from(if let Some(album) = &fields.album {
                Span::styled(format!("  {}", album), Style::default().fg(app.theme.fg))
            } else {
                Span::raw("")
            }),
            Line::from(""),
            Line::from(Span::styled(fields.meta.clone(), Style::default().fg(app.theme.fg_dim))),
            Line::from(Span::styled(
                stats_line.trim(),
                Style::default().fg(app.theme.fg_dim),
            )),
        ];
        let para = Paragraph::new(lines);
        f.render_widget(para, area);
    }
}

fn render_about_picker(f: &mut ratatui::Frame, area: Rect, app: &App) {
    let block = picker_panel(app, " About ", Some(" [Esc] Close"));
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
            " Copyright (C) 2026 - present, prjctimg",
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
}

fn render_help_picker(f: &mut ratatui::Frame, area: Rect, app: &mut App) {
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
        ("key", "   Alt+S        Spotify Search"),
        ("", ""),
        ("topic", "Other"),
        ("key", "   q            Quit"),
        ("key", "   Q            Quit & stop daemon"),
        ("key", "   S            Toggle shuffle"),
        ("key", "   r / R        Cycle repeat"),
        ("key", "   :            Command palette"),
        ("", ""),
        ("topic", "Help"),
        ("key", "   ?            Toggle this help"),
        ("key", "   gg / G       Jump to top / bottom"),
        ("key", "   0 / $        Jump to first / last line"),
        ("key", "   /            Search"),
        ("key", "   n / N        Next / previous match"),
        ("key", "   Esc / q      Close"),
    ];

    let filtered: Vec<(&str, &str)> = if query.is_empty() {
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
    let (scroll_start, scroll_end) = if total > 0 {
        if let Some(top) = app.pickers.top_mut() {
            let (s, e) = step_viewport(top.viewport_offset, sel, visible, total);
            top.viewport_offset = s;
            (s, e)
        } else {
            (0, total)
        }
    } else {
        (0, 0)
    };

    let mut lines: Vec<Line> = Vec::new();

    let title = Line::from(Span::styled(
        " KEYBINDINGS ",
        Style::default()
            .fg(app.theme.accent)
            .add_modifier(Modifier::BOLD),
    ));
    lines.push(title);

    for (i, (kind, line)) in filtered
        .iter()
        .enumerate()
        .take(scroll_end)
        .skip(scroll_start)
    {
        let is_sel = i == sel;
        let style = if is_sel {
            Style::default()
                .fg(app.theme.selection_fg_readable())
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
        format!(
            " /{}  [Esc] Close  ? Toggle  gg/G Top/Bottom  0/$ First/Last  n/N Next/Prev",
            query
        )
    } else {
        "[Esc] Close  ? Toggle  gg/G Top/Bottom  0/$ First/Last  / Search  n/N Next/Prev"
            .to_string()
    };
    picker_help(f, area, &footer, app);
}

fn render_sleep_timer_picker(f: &mut ratatui::Frame, area: Rect, app: &App) {
    if app.sleep_timer_input_mode {
        let block = picker_panel(
            app,
            " Sleep Timer: Manual Input ",
            Some(" Enter minutes  [Enter] Set  [Esc] Cancel"),
        );
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

    let is_active = app.sleep_timer_remaining.is_some();
    let help_text = if is_active {
        " h/- Decrease  l/+ Increase  i: Input  Enter: Set  c: Cancel Timer  [Esc] Close"
    } else {
        " h/- Decrease  l/+ Increase  i: Input  Enter: Set  [Esc] Close"
    };
    let block = picker_panel(app, " Sleep Timer ", Some(help_text));
    let inner = block.inner(area);
    f.render_widget(block, area);

    let mins = app.sleep_timer_minutes;

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
                .fg(app.theme.selection_fg_readable())
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

    // Controls (kept for visual reference, same as border help)
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

/// (emoji label, keybinding, action id). The action id is the stable handle
/// the app dispatches on, so label text or emoji glyph changes can never
/// break command execution.
pub const COMMAND_PALETTE_COMMANDS: &[(&str, &str, &str)] = &[
    ("\u{25b6}\u{fe0f} Play/Pause", "Space", "play/pause"),
    ("\u{23ed}\u{fe0f} Next Track", "n", "next track"),
    ("\u{23ee}\u{fe0f} Prev Track", "p", "prev track"),
    ("\u{1f50a} Volume Up", "+", "volume up"),
    ("\u{1f509} Volume Down", "-", "volume down"),
    ("\u{1f507} Mute: Toggle", "m", "mute"),
    ("\u{1f501} Repeat Mode", "r", "repeat"),
    ("\u{1f500} Shuffle Library", "S", "shuffle"),
    ("\u{23f9}\u{fe0f} Quit", "q", "quit"),
    ("\u{23f9}\u{fe0f} Quit Daemon", "Q/Ctrl+Q", "quit daemon"),
    ("\u{27a1}\u{fe0f} Tab Cycle", "Tab", "tab cycle"),
    ("\u{1f3b5} Library", "1", "library"),
    ("\u{2699}\u{fe0f} Settings", "2", "settings"),
    ("\u{1f50d} Search Track", "/", "search"),
    ("\u{1f4cb} Queue", "Alt+Q", "queue"),
    ("\u{25b6}\u{fe0f} YouTube Search", "Alt+Y", "youtube"),
    ("\u{1f50e} Search Library", "Alt+F", "search lib"),
    ("\u{1f39a} Equalizer", "Alt+E", "eq"),
    ("\u{23f0}\u{fe0f} Sleep Timer", "Alt+Z", "sleeptimer"),
    ("\u{1f3a8} Theme", "Alt+C", "themepicker"),
    ("\u{2139}\u{fe0f} About", "Alt+A", "about"),
    ("\u{1f3b5} Spotify", "Alt+S", "spotify"),
    ("\u{1f4dd} Fetch Lyrics", "l", "fetch lyrics"),
    ("\u{1f3a8} Progress Style", "P", "progress style"),
    ("\u{1f3b6} Visualizer: Toggle", "Ctrl+V", "visualizer"),
    ("\u{1f3b6} Visualizer Preset", "Alt+V", "visualizer preset"),
    ("\u{23f9}\u{fe0f} Stop", "s", "stop"),
    ("\u{23e9}\u{fe0f} Seek Forward", ".", "seek forward"),
    ("\u{23ea}\u{fe0f} Seek Backward", ",", "seek backward"),
    ("\u{2764}\u{fe0f} Toggle Favourite", "f", "toggle favourite"),
    ("\u{1f5d1} Clear Queue", "D", "clear queue"),
    ("\u{2b05}\u{fe0f} Prev Tab", "Shift+Tab", "prev tab"),
    ("\u{2611}\u{fe0f} Multiselect", "v", "multiselect"),
    ("\u{2795} Add to Queue", "a", "add to queue"),
    ("\u{1f4dc} Add to Playlist", "A", "add to playlist"),
    ("\u{274c} Delete from List", "x", "delete from list"),
    ("\u{2b07}\u{fe0f} Jump to End", "G", "jump to end"),
    ("\u{270f}\u{fe0f} Edit Metadata", "e", "edit metadata"),
    ("\u{2753} Toggle Help", "?", "toggle help"),
    ("\u{1f6ab} Hide Help Bar", "Ctrl+H", "hide help bar"),
    ("\u{1f4cf} Footer Preset", "\u{2014}", "footer preset"),
    ("\u{25c9} Cycle Design", "\u{2014}", "cycle design"),
    ("\u{1fa7a} Health Check", ":", "health check"),
];

fn render_command_palette_picker(f: &mut ratatui::Frame, area: Rect, app: &mut App) {
    let commands = COMMAND_PALETTE_COMMANDS;

    let query = app.pickers.top().map_or(String::new(), |o| o.query.clone());
    let q = query.to_lowercase();
    let mut filtered: Vec<(&(&str, &str, &str), usize)> = if q.is_empty() {
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

    let block = picker_panel(
        app,
        " Commands ",
        Some(" Type to filter  [Enter] Execute  [Esc] Close  \u{2191}/\u{2193} Navigate"),
    );
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
    let (scroll_start, scroll_end) = if total > 0 {
        if let Some(top) = app.pickers.top_mut() {
            let (s, e) = step_viewport(top.viewport_offset, sel, visible, total);
            top.viewport_offset = s;
            (s, e)
        } else {
            (0, total)
        }
    } else {
        (0, 0)
    };

    // Pad names so keybindings line up in a column.
    let name_w = filtered
        .iter()
        .map(|((name, _, _), _)| name.chars().count())
        .max()
        .unwrap_or(0)
        .min(inner.width.saturating_sub(8) as usize);

    let mut lines: Vec<Line> = vec![search_line];
    let row_w = inner.width as usize;
    for (i, ((name, key, _), _score)) in filtered
        .iter()
        .enumerate()
        .take(scroll_end)
        .skip(scroll_start)
    {
        let is_sel = i == sel;
        let prefix = if is_sel { " > " } else { "   " };
        let style = if is_sel {
            Style::default()
                .fg(app.theme.selection_fg_readable())
                .bg(app.theme.selection_bg)
        } else {
            Style::default()
        };
        let key_style = if is_sel {
            Style::default()
                .fg(app.theme.selection_fg_readable())
                .bg(app.theme.selection_bg)
        } else {
            Style::default().fg(app.theme.fg_dim)
        };
        // Selected row highlights edge-to-edge: pad the name to fill the width.
        let pad = if is_sel {
            row_w.saturating_sub(prefix.len() + key.len() + 4)
        } else {
            name_w
        };
        lines.push(Line::from(vec![
            Span::styled(format!("{prefix}{:<pad$}", name, pad = pad), style),
            Span::styled(format!("  [{key}]", key = key), key_style),
        ]));
    }

    let para = Paragraph::new(lines);
    f.render_widget(para, inner);
}

fn render_equalizer_picker(f: &mut ratatui::Frame, area: Rect, app: &mut App) {
    let presets = [
        ("Flat", "Neutral, uncoloured response", EqPreset::Flat),
        ("Normal", "Balanced all-rounder", EqPreset::Normal),
        ("Pop", "Vocal-forward with a lively top end", EqPreset::Pop),
        (
            "Rock",
            "Aggressive mids for punch and drive",
            EqPreset::Rock,
        ),
        ("Jazz", "Smooth mids and sparkly highs", EqPreset::Jazz),
        ("Classical", "Wide, airy and natural", EqPreset::Classical),
        ("Bass", "Deep low-end emphasis", EqPreset::Bass),
        ("Vocal", "Brings voices to the front", EqPreset::Vocal),
        (
            "Electronic",
            "Tight, modern club sound",
            EqPreset::Electronic,
        ),
        ("Hip-Hop", "Heavy bass and crisp highs", EqPreset::HipHop),
        ("Latin", "Warm and rhythmic", EqPreset::Latin),
        ("Acoustic", "Clean and intimate", EqPreset::Acoustic),
        ("Podcast", "Speech clarity over music", EqPreset::Podcast),
        ("Dance", "Pumping lows for the floor", EqPreset::Dance),
        (
            "Headphones",
            "Close-up stereo imaging",
            EqPreset::Headphones,
        ),
        ("Speaker", "Room-filling broad response", EqPreset::Speaker),
    ];

    let sel = app
        .pickers
        .top()
        .map_or(0, |o| o.selected.min(presets.len() - 1));

    let block = picker_panel(
        app,
        " Equalizer ",
        Some(" [Enter] Apply  [Esc] Close  j/k Navigate"),
    );
    let inner = block.inner(area);
    f.render_widget(block, area);

    // Live EQ preview panel below the list (PROMPT #3): a separator, the
    // active/highlighted preset label, then the gain curve with a dB scale.
    let preview_h = 4u16;
    let sep_y = inner.y + inner.height.saturating_sub(preview_h + 1);
    let list_area = Rect {
        x: inner.x,
        y: inner.y,
        width: inner.width,
        height: inner.height.saturating_sub(preview_h + 1),
    };
    let preview_area = Rect {
        x: inner.x,
        y: sep_y + 1,
        width: inner.width,
        height: preview_h,
    };

    let sep_w = inner.width.saturating_sub(2) as usize;
    let sep_label = " EQ Preview ";
    let sep = if sep_w >= sep_label.len() {
        let side = (sep_w - sep_label.len()) / 2;
        format!(
            "{}{}{}",
            "─".repeat(side),
            sep_label,
            "─".repeat(sep_w - sep_label.len() - side)
        )
    } else {
        sep_label.to_string()
    };
    f.render_widget(
        Paragraph::new(Line::from(Span::styled(
            sep,
            Style::default().fg(app.theme.fg_dim),
        ))),
        Rect {
            x: inner.x,
            y: sep_y,
            width: inner.width,
            height: 1,
        },
    );

    // Preset list (1-row-at-a-time scroll, PROMPT #10).
    let visible = list_area.height as usize;
    let total = presets.len();
    let (scroll_start, scroll_end) = if let Some(top) = app.pickers.top_mut() {
        let (s, e) = step_viewport(top.viewport_offset, sel, visible, total);
        top.viewport_offset = s;
        (s, e)
    } else {
        (0, total)
    };

    let mut list_items: Vec<ListItem> = Vec::new();
    for (i, (name, desc, eq)) in presets
        .iter()
        .enumerate()
        .skip(scroll_start)
        .take(scroll_end - scroll_start)
    {
        let is_sel = i == sel;
        let prefix = if is_sel { " > " } else { "   " };
        let style = if is_sel {
            Style::default()
                .fg(app.theme.selection_fg_readable())
                .bg(app.theme.selection_bg)
        } else if *name == app.state.eq_preset.label() {
            Style::default().fg(app.theme.success)
        } else {
            Style::default()
        };
        let mut spans = vec![Span::styled(format!("{prefix}{}", name), Style::default())];
        // Static bar-chart preview of the gain curve (T24).
        spans.extend(eq_preset_preview(*eq, app));
        if !is_sel {
            spans.push(Span::styled(
                format!("  \u{2014} {desc}"),
                Style::default().fg(app.theme.fg_dim),
            ));
        }
        list_items.push(ListItem::new(Line::from(spans)).style(style));
    }

    let list = List::new(list_items);
    f.render_widget(list, list_area);

    // Preview: which preset is active (and which is highlighted when they
    // differ), then the live gain curve of the highlighted preset.
    let highlighted = presets[sel];
    let active_label = app.state.eq_preset.label();
    let current_line = if app.state.eq_preset == highlighted.2 {
        Line::from(Span::styled(
            format!(" Current: {}", active_label),
            Style::default().fg(app.theme.success),
        ))
    } else {
        Line::from(vec![
            Span::styled(
                format!(" Current: {}", active_label),
                Style::default().fg(app.theme.success),
            ),
            Span::styled(
                format!("  \u{2192} {}", highlighted.0),
                Style::default().fg(app.theme.accent),
            ),
        ])
    };
    f.render_widget(
        current_line,
        Rect {
            x: inner.x,
            y: preview_area.y,
            width: inner.width,
            height: 1,
        },
    );
    let curve_area = Rect {
        x: inner.x,
        y: preview_area.y + 1,
        width: inner.width,
        height: preview_h - 1,
    };
    render_eq_curve(f, curve_area, highlighted.2.to_gains(), app);
}

/// Draw an EQ gain curve (15 ISO 1/3-octave band gains in dB) into `area`
/// with a vertical dB scale and gridlines (PROMPT #3).  The zero line is a
/// row of `─`; the curve is traced with `█` and the edges are marked `▔`/`▁`.
fn render_eq_curve(f: &mut ratatui::Frame, area: Rect, gains: [f32; 15], app: &App) {
    if area.width < 8 || area.height < 2 {
        return;
    }
    let rows = area.height as usize;
    let cols = area.width as usize;
    let zero_row = rows.saturating_sub(1) / 2;
    let to_row = |g: f32| {
        let n = ((g.clamp(-6.0, 6.0) + 6.0) / 12.0) as f64;
        ((1.0 - n) * (rows as f64 - 1.0)).round() as usize
    };

    let mut grid: Vec<Vec<char>> = vec![vec![' '; cols]; rows];
    for x in 0..cols {
        let frac = if cols > 1 {
            x as f64 / (cols - 1) as f64
        } else {
            0.0
        };
        let pos = frac * 14.0;
        let lo = pos.floor() as usize;
        let hi = (pos.ceil() as usize).min(14);
        let t = pos - lo as f64;
        let g = gains[lo] as f64 * (1.0 - t) + gains[hi] as f64 * t;
        let row = to_row(g as f32);
        if let Some(cell) = grid.get_mut(row).and_then(|r| r.get_mut(x)) {
            *cell = '█';
        }
    }
    for (r, row) in grid.iter_mut().enumerate() {
        for cell in row.iter_mut() {
            if *cell == ' ' {
                *cell = if r == zero_row {
                    '─'
                } else if r == rows - 1 {
                    '▁'
                } else if r == 0 {
                    '▔'
                } else {
                    '·'
                };
            }
        }
    }

    let mut lines: Vec<Line> = Vec::with_capacity(rows);
    for (r, row) in grid.iter().enumerate() {
        let label = if r == 0 {
            "+6 "
        } else if r == zero_row {
            " 0 "
        } else if r == rows - 1 {
            "-6 "
        } else {
            "   "
        };
        let mut spans = vec![Span::styled(label, Style::default().fg(app.theme.fg_dim))];
        for &ch in row {
            let style = if ch == '█' {
                Style::default()
                    .fg(app.theme.accent)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(app.theme.fg_dim)
            };
            spans.push(Span::styled(ch.to_string(), style));
        }
        lines.push(Line::from(spans));
    }
    f.render_widget(Paragraph::new(lines), area);
}

/// Static bar-chart preview of an EQ preset's gain curve (T24).  The fifteen
/// ISO 1/3-octave gains (±4 dB) are mapped onto the visualizer's block glyphs
/// — the active visualizer preset picks the glyph set — and coloured by the
/// theme: boosts are `success`, cuts are `warning`, and neutral bands stay
/// muted so the curve is readable at a glance.
fn eq_preset_preview(eq: EqPreset, app: &App) -> Vec<Span<'static>> {
    const BLOCKS: [char; 8] = ['▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];
    const BRAILLE: [char; 8] = ['⠁', '⠃', '⠇', '⡇', '⣇', '⣧', '⣷', '⣿'];
    let chars: &[char; 8] = match app.visualizer.preset {
        crate::visualizer::VisualizerPreset::Braille
        | crate::visualizer::VisualizerPreset::Gradient => &BRAILLE,
        _ => &BLOCKS,
    };
    let mut spans = vec![Span::raw("  ")];
    for g in eq.to_gains() {
        let norm = (((g.clamp(-4.0, 4.0) + 4.0) / 8.0) * 7.0).round() as usize;
        let color = if g > 0.75 {
            app.theme.success
        } else if g < -0.75 {
            app.theme.warning
        } else {
            app.theme.fg_dim
        };
        spans.push(Span::styled(
            chars[norm.min(7)].to_string(),
            Style::default().fg(color),
        ));
    }
    spans
}

fn render_theme_picker_picker(f: &mut ratatui::Frame, area: Rect, app: &mut App) {
    let block = picker_panel(app, " Theme ", Some(" [Enter] Select  [Esc] Close"));
    let _inner = block.inner(area);
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

    let block = picker_panel(app, " Theme ", Some(" [Enter] Select  [Esc] Close"));
    let inner = block.inner(area);
    f.render_widget(block, area);

    let search_line = Line::from(Span::styled(
        format!(" > {}_", query),
        Style::default().fg(app.theme.fg),
    ));

    let visible = inner.height.saturating_sub(1) as usize;
    let (scroll_start, scroll_end) = if total > 0 {
        if let Some(top) = app.pickers.top_mut() {
            let (s, e) = step_viewport(top.viewport_offset, sel, visible, total);
            top.viewport_offset = s;
            (s, e)
        } else {
            (0, total)
        }
    } else {
        (0, 0)
    };

    let mut list_items: Vec<ListItem> = vec![ListItem::new(search_line)];
    let row_w = inner.width as usize;
    let mut first_row = true;
    for &(i, entry) in &filtered[scroll_start..scroll_end] {
        // Faint horizontal rule between items (not above the first row).
        if !first_row {
            let rule = Span::styled(
                "  ".to_string() + &"\u{2500}".repeat(row_w.saturating_sub(6)),
                Style::default().fg(app.theme.fg_dim),
            );
            list_items.push(ListItem::new(Line::from(rule)));
        }
        first_row = false;

        let is_active = i == app.theme_index;
        let prefix = if i == sel { " > " } else { "   " };
        let check = if is_active { " \u{2713}" } else { "" };
        // Badge light themes so users can spot them at a glance.
        let light_badge = if entry.light { " \u{2600}" } else { "" };
        let name_part = format!("{}{}{}", prefix, entry.name, light_badge);

        // Swatch run of the theme's core colours, right-aligned with a
        // 2-cell margin so the palette reads as a consistent column.
        let colors = [
            entry.theme.bg,
            entry.theme.fg,
            entry.theme.accent,
            entry.theme.secondary_accent,
            entry.theme.tertiary_accent,
            entry.theme.border,
        ];
        let swatch_run_w = colors.len() * 2 + colors.len() + 1;
        let left_w = name_part.chars().count() + check.chars().count() + 1;
        let pad = row_w.saturating_sub(left_w + swatch_run_w + 2);

        // The highlighter covers only the name (and the padding that pushes
        // the palette into its right-aligned column).  It stops where the
        // swatches begin so the theme colours stay unobscured (T6).
        let name_style = if i == sel {
            Style::default()
                .fg(app.theme.selection_fg_readable())
                .bg(app.theme.selection_bg)
        } else {
            Style::default()
        };
        let mut spans: Vec<Span> = vec![Span::styled(
            format!("{name_part}{:pad$}", "", pad = pad),
            name_style,
        )];
        for c in colors {
            spans.push(Span::styled("  ", Style::default().fg(c).bg(c)));
            spans.push(Span::raw(" "));
        }
        spans.push(Span::styled(check, Style::default()));
        let style = if i == sel {
            Style::default()
        } else if is_active {
            Style::default()
                .fg(app.theme.accent)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default()
        };
        list_items.push(ListItem::new(Line::from(spans)).style(style));
    }

    let list = List::new(list_items);
    f.render_widget(list, inner);
}

/// Crossfade duration options (seconds) shown in the crossfade picker.
pub const CROSSFADE_DURATIONS: [u8; 5] = [3, 5, 10, 15, 30];

/// Easing options shown in the crossfade picker, in display order.
pub const CROSSFADE_EASINGS: [gtm_core::state::Easing; 7] = [
    gtm_core::state::Easing::Linear,
    gtm_core::state::Easing::SlowFadeInFastFadeOut,
    gtm_core::state::Easing::FastFadeInSlowFadeOut,
    gtm_core::state::Easing::Logarithmic,
    gtm_core::state::Easing::Smoothstep,
    gtm_core::state::Easing::EqualPower,
    gtm_core::state::Easing::Exponential,
];

fn easing_description(e: gtm_core::state::Easing) -> &'static str {
    match e {
        gtm_core::state::Easing::Linear => {
            "Linear: constant gain ramp; abrupt but predictable."
        }
        gtm_core::state::Easing::Smoothstep => {
            "Smoothstep: smooth start and end, no clicks."
        }
        gtm_core::state::Easing::EqualPower => {
            "Equal Power: constant perceived loudness, no mid-fade dip."
        }
        gtm_core::state::Easing::Logarithmic => {
            "Logarithmic: fast attack, fast release profile."
        }
        gtm_core::state::Easing::Exponential => {
            "Exponential: fast attack, fast release profile."
        }
        gtm_core::state::Easing::SlowFadeInFastFadeOut => {
            "Slow In, Fast Out: asymmetric curve."
        }
        gtm_core::state::Easing::FastFadeInSlowFadeOut => {
            "Fast In, Slow Out: asymmetric curve."
        }
    }
}

fn render_crossfade_picker(f: &mut ratatui::Frame, area: Rect, app: &App) {
    let block = picker_panel(
        app,
        " Crossfade Options ",
        Some(" j/k Navigate  [Enter] Select  [Esc] Close"),
    );
    let inner = block.inner(area);
    f.render_widget(block, area);

    let dur = app
        .state
        .crossfade
        .as_ref()
        .map(|c| c.duration_secs)
        .unwrap_or(0);
    let easing = app
        .state
        .crossfade
        .as_ref()
        .map(|c| c.easing)
        .unwrap_or_default();

    // Rows: [0] "Duration" header, [1..=5] durations, [6] "Easing" header,
    // [7..=13] easings.
    let mut rows: Vec<String> = Vec::new();
    rows.push(" Duration ".to_string());
    for d in CROSSFADE_DURATIONS {
        let cur = if d == dur { "   (current)" } else { "" };
        rows.push(format!("   {d}s{cur}"));
    }
    rows.push(" Easing ".to_string());
    for e in CROSSFADE_EASINGS {
        let cur = if e == easing { "   (current)" } else { "" };
        rows.push(format!("   {}{cur}", e.name()));
    }

    let sel = app
        .pickers
        .top()
        .map_or(0, |o| o.selected.min(rows.len() - 1));
    let mut lines = Vec::new();
    for (i, row) in rows.iter().enumerate() {
        let is_header = i == 0 || i == 6;
        let is_sel = i == sel;
        let line = if is_header {
            Line::from(Span::styled(
                row.clone(),
                Style::default().fg(app.theme.accent).add_modifier(Modifier::BOLD),
            ))
        } else {
            let prefix = if is_sel { " > " } else { "   " };
            let style = if is_sel {
                Style::default()
                    .fg(app.theme.selection_fg_readable())
                    .bg(app.theme.selection_bg)
            } else {
                Style::default()
            };
            Line::from(Span::styled(format!("{prefix}{row}"), style))
        };
        lines.push(line);
    }

    // Bottom panel: describe the highlighted easing.
    lines.push(Line::from(""));
    if sel >= 7 && sel < 7 + CROSSFADE_EASINGS.len() {
        let e = CROSSFADE_EASINGS[sel - 7];
        let desc = easing_description(e);
        lines.push(Line::from(Span::styled(
            format!(" {desc}"),
            Style::default().fg(app.theme.fg_dim),
        )));
    } else if sel >= 1 && sel < 1 + CROSSFADE_DURATIONS.len() {
        lines.push(Line::from(Span::styled(
            " Select a crossfade duration. 3s is subtle, 30s is ambient.",
            Style::default().fg(app.theme.fg_dim),
        )));
    } else {
        lines.push(Line::from(Span::styled(
            " Choose a crossfade duration or an easing curve.",
            Style::default().fg(app.theme.fg_dim),
        )));
    }

    let para = Paragraph::new(lines);
    f.render_widget(para, inner);
}

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

/// Re-export so footer.rs and other modules can call `crate::ui::readable_fg`.
pub use crate::theme::readable_fg;

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
    let block = picker_panel(
        app,
        " Select Playlist ",
        Some(" [Enter] Select  [Esc] Cancel"),
    );
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
                .fg(app.theme.selection_fg_readable())
                .bg(app.theme.selection_bg)
        } else {
            Style::default().fg(app.theme.fg)
        };
        items.push(ListItem::new(content).style(style));
    }

    let list = List::new(items);
    f.render_widget(list, inner);
}

fn render_edit_metadata_picker(f: &mut ratatui::Frame, area: Rect, app: &mut App) {
    let help_text = " [j/k] Fields  [Ctrl+S] Sync Cover  [Tab] Next  [Enter] Save  [Esc] Cancel";
    let block = picker_panel(app, " Edit Metadata ", Some(help_text));
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

    // Reserve a right column for a larger cover preview (bottom-right corner
    // feel).  The width is the canonical COVER_W scaled up ~1.7× so the 'e'
    // window shows the artwork at a readable size (T12).
    const COVER_W_EDIT: u16 = 24;
    let vchunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(0)])
        .split(inner);
    let content = vchunks[0];
    let cover_col_w = if content.width > COVER_W_EDIT + 2 {
        COVER_W_EDIT
    } else {
        0
    };
    let hchunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Min(0), Constraint::Length(cover_col_w)])
        .split(content);
    let list_area = hchunks[0];
    let cover_area = hchunks[1];

    let mut lines: Vec<Line> = Vec::new();
    for (i, name) in field_names.iter().enumerate() {
        let value = app.metadata_fields.get(i).map(|s| s.as_str()).unwrap_or("");
        let is_active = i == app.metadata_field_idx;
        let prefix = if is_active { " > " } else { "   " };
        let style = if is_active {
            Style::default()
                .fg(app.theme.selection_fg_readable())
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
    f.render_widget(para, list_area);

    // Cover preview pinned to the bottom of the right column; taller than the
    // canonical size so the artwork fills the reserved column.
    if cover_area.width > 0 {
        let cover_h = 12u16.min(cover_area.height);
        let c_area = Rect {
            x: cover_area.x,
            y: cover_area.y + cover_area.height.saturating_sub(cover_h),
            width: cover_area.width,
            height: cover_h,
        };
        if let Some(ref mut protocol) = app.metadata_cover_stateful {
            let image = StatefulImage::new();
            f.render_stateful_widget(image, c_area, protocol);
        } else if let Some(ref cover_bytes) = app.metadata_cover {
            render_cover_block(f, c_area, cover_bytes);
        } else {
            let placeholder = Paragraph::new(Line::from(Span::styled(
                " \u{266b} no cover ",
                Style::default().fg(app.theme.fg_dim),
            )))
            .alignment(Alignment::Center);
            f.render_widget(placeholder, c_area);
        }
    }
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
        .title(Line::from(Span::styled(
            " Health Check ",
            Style::default()
                .fg(app.theme.accent)
                .add_modifier(Modifier::BOLD),
        )))
        .borders(Borders::ALL)
        .style(Style::default().fg(app.theme.fg).bg(if app.transparent_bg {
            ratatui::style::Color::Reset
        } else {
            app.theme.elevated_bg
        }));

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
                    format!(": {msg}"),
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
