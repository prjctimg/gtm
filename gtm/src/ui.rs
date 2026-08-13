use std::path::PathBuf;

use crossterm::terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen};
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Clear, List, ListItem, Paragraph};
use ratatui::Terminal;
use crate::app::{App, InputMode, LIBRARY_CATEGORIES};
use crate::overlay::OverlayId;
use crate::theme::THEMES;
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
            Constraint::Length(2),
            Constraint::Length(1),
            Constraint::Min(0),
            Constraint::Length(3),
        ])
        .split(area);

    render_tabs(f, chunks[0], app);
    render_notifications(f, chunks[1], app);
    render_content(f, chunks[2], app);
    render_footer(f, chunks[3], app);

    // Render overlays on top of everything
    if app.overlays.is_open() {
        render_overlay(f, area, app);
    }
}

// ─── Tab Bar ───

fn render_tabs(f: &mut ratatui::Frame, area: Rect, app: &App) {
    let version = option_env!("CARGO_PKG_VERSION").unwrap_or("0.1.0");
    let tab_items = [("1", "Now Playing", Tab::NowPlaying), ("2", "Library", Tab::Library), ("3", "Settings", Tab::Settings)];

    let mut span_data: Vec<(bool, String)> = Vec::new();
    for (num, name, tab) in &tab_items {
        let label = format!("[{}] {}  ", num, name);
        span_data.push((*tab == app.current_tab, label));
    }

    let mut spans: Vec<Span> = vec![Span::raw(" ")];
    for (is_active, label) in &span_data {
        if *is_active {
            spans.push(Span::styled(
                label.clone(),
                Style::default()
                    .fg(app.theme.tab_active_fg)
                    .bg(app.theme.tab_active_bg)
                    .add_modifier(Modifier::BOLD),
            ));
        } else {
            spans.push(Span::styled(label.clone(), Style::default().fg(app.theme.tab_inactive_fg)));
        }
    }

    let version_str = format!("gtm {version}");
    let remaining = area.width.saturating_sub(3);
    let pad = remaining.saturating_sub(spans.iter().map(|s| s.width() as u16).sum::<u16>());
    if pad > version_str.len() as u16 + 1 {
        spans.push(Span::raw(" ".repeat((pad - version_str.len() as u16) as usize)));
        spans.push(Span::styled(version_str, Style::default().fg(app.theme.fg_dim)));
    }

    let tab_line = Paragraph::new(Line::from(spans))
        .style(Style::default().bg(app.theme.tab_bar_bg));
    f.render_widget(tab_line, Rect { x: area.x, y: area.y, width: area.width, height: 1 });

    let sep = Paragraph::new("─".repeat(area.width as usize))
        .style(Style::default().fg(app.theme.border));
    f.render_widget(sep, Rect { x: area.x, y: area.y + 1, width: area.width, height: 1 });
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
    match app.current_tab {
        Tab::NowPlaying => render_now_playing(f, area, app),
        Tab::Library => render_library(f, area, app),
        Tab::Settings => render_settings(f, area, app),
    }
}

fn render_now_playing(f: &mut ratatui::Frame, area: Rect, app: &App) {
    if app.show_tag_popup {
        return render_track_info_popup(f, area, app);
    }
    let track = match &app.state.current_track {
        Some(t) => t,
        None => {
            let p = Paragraph::new("No track playing")
                .alignment(Alignment::Center)
                .style(Style::default().fg(app.theme.fg_dim));
            f.render_widget(p, area);
            return;
        }
    };

    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Length(10), Constraint::Min(0)])
        .margin(1)
        .split(area);

    // ── Cover Art ──
    let cover_area = chunks[0];
    if let Some(ref cover_bytes) = app.current_cover {
        render_cover_block(f, cover_area, cover_bytes);
    } else {
        let placeholder = Block::default()
            .borders(Borders::ALL)
            .title(" Cover ")
            .border_type(BorderType::Plain)
            .style(Style::default().fg(app.theme.fg_dim));
        f.render_widget(placeholder, cover_area);
    }

    // ── Info + Progress + Controls ──
    let right = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),  // NOW PLAYING header
            Constraint::Length(1),  // separator
            Constraint::Length(1),  // title
            Constraint::Length(1),  // artist
            Constraint::Length(1),  // format chip
            Constraint::Length(1),  // album
            Constraint::Length(1),  // hashtag progress bar
            Constraint::Min(0),     // controls
        ])
        .split(chunks[1]);

    // NOW PLAYING header
    let header = Paragraph::new("NOW PLAYING")
        .style(Style::default().fg(app.theme.accent).add_modifier(Modifier::BOLD));
    f.render_widget(header, right[0]);

    // Separator
    let sep = Paragraph::new("─".repeat(right[1].width as usize))
        .style(Style::default().fg(app.theme.fg_dim));
    f.render_widget(sep, right[1]);

    // Track title
    let fav_prefix = if track.favourite { "\u{2665} " } else { "" };
    let title = Paragraph::new(Line::from(vec![
        Span::styled(fav_prefix, Style::default().fg(app.theme.error)),
        Span::styled(&track.title, Style::default().fg(app.theme.fg_bright).add_modifier(Modifier::BOLD)),
    ]));
    f.render_widget(title, right[2]);

    // Artist
    let artist = Paragraph::new(Line::from(vec![
        Span::styled("Artist: ", Style::default().fg(app.theme.fg_dim)),
        Span::styled(&track.artist, Style::default().fg(app.theme.fg)),
    ]));
    f.render_widget(artist, right[3]);

    // Format chip
    let fmt_parts: Vec<String> = {
        let mut p = Vec::new();
        if let Some(b) = track.bitrate { p.push(format!("{}kbps", b)); }
        if let Some(s) = track.samplerate { p.push(format!("{}kHz", (s as f64 / 1000.0).round() as u32)); }
        p
    };
    let format_chip = if !fmt_parts.is_empty() {
        format!("Format: [ {} ]", fmt_parts.join(" | "))
    } else {
        String::new()
    };
    if !format_chip.is_empty() {
        let fmt = Paragraph::new(Line::from(vec![
            Span::styled("Format: ", Style::default().fg(app.theme.fg_dim)),
            Span::styled(fmt_parts.join(" | "), Style::default().fg(app.theme.accent)),
        ]));
        f.render_widget(fmt, right[4]);
    }

    // Album
    let album = Paragraph::new(Line::from(vec![
        Span::styled("Album:  ", Style::default().fg(app.theme.fg_dim)),
        Span::styled(&track.album, Style::default().fg(app.theme.fg)),
    ]));
    f.render_widget(album, right[5]);

    // Hashtag progress bar with timestamps inline
    let dur = track.duration;
    let pos = app.display_position;
    let pos_str = format_duration(pos as u64);
    let dur_str = format_duration(dur as u64);
    let ratio = if dur > 0.0 { (pos / dur) as f64 } else { 0.0 };
    let ts_str = format!("{} / {}", pos_str, dur_str);
    let bar_width = right[6].width.saturating_sub(2) as usize;
    let bar = render_progress_line(ratio, bar_width);
    let bar_with_ts = format!("{} {}", bar, ts_str);
    let bar_para = Paragraph::new(bar_with_ts)
        .style(Style::default().fg(app.theme.accent));
    f.render_widget(bar_para, right[6]);

    // Volume bar + controls on same bottom row
    let bottom_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Length(10), Constraint::Min(0)])
        .split(right[7]);
    let vol_ratio = if app.state.mute { 0.0 } else { app.state.volume as f64 / 100.0 };
    let vol_bar = render_progress_line(vol_ratio, 8);
    let vol_label: String = if app.state.mute { "MUTED".into() } else { format!("{:3}%", app.state.volume) };
    let vol_text = Paragraph::new(format!("{vol_bar} {vol_label}"))
        .style(Style::default().fg(app.theme.volume_color(app.state.volume)));
    f.render_widget(vol_text, bottom_chunks[0]);

    let controls = Paragraph::new(
        " [Space]P/P  [n/N]Next  [p/P]Prev  [s]Stop  [+/-]Vol  [m]Mute  [r/R]Repeat  [S]Shuffle  [t]Info  [:]Cmd  [q]Quit ",
    )
    .style(Style::default().fg(app.theme.fg_dim));
    f.render_widget(controls, bottom_chunks[1]);
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
    "\u{f042a}", "\u{f00b6}", "\u{f0883}", "\u{f1ae7}",
    "\u{f0510}", "\u{f010d}", "\u{f010e}", "\u{f04c7}", "\u{f39d8}",
];

const LIBRARY_ICONS_ASCII: &[&str] = &["♪", "▤", "♪", "≡", "⏱", "★", "☆", "☊", "▼"];

fn use_nerd_fonts() -> bool {
    match std::env::var("GTM_NERD_FONTS") {
        Ok(v) if v == "0" || v == "false" || v == "no" => false,
        _ => true,
    }
}

fn render_library(f: &mut ratatui::Frame, area: Rect, app: &App) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(1), Constraint::Length(1)])
        .split(area);

    let panes = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Length(28), Constraint::Min(0)])
        .split(chunks[0]);

    let stats_area = chunks[1];
    let left_focus = app.library_pane_focus;

    // ── Left pane: categories with icons ──
    let lib_icons = if use_nerd_fonts() { LIBRARY_ICONS_NERD } else { LIBRARY_ICONS_ASCII };
    let left_items: Vec<ListItem> = LIBRARY_CATEGORIES
        .iter()
        .enumerate()
        .map(|(i, cat)| {
            let icon = lib_icons.get(i).unwrap_or(&" ");
            let count = match *cat {
                "All Tracks" => app.tracks_cache.len(),
                _ => 0,
            };
            let num = i + 1;
            let label = if count > 0 {
                format!(" {icon} {num:>2}. {:<14} {:>4}", cat, count)
            } else {
                format!(" {icon} {num:>2}. {}", cat)
            };
            let is_active = i == app.library_category;
            let style = if is_active && left_focus {
                Style::default().fg(app.theme.selection_fg).bg(app.theme.selection_bg)
            } else if is_active {
                Style::default().fg(app.theme.accent)
            } else {
                Style::default().fg(app.theme.fg_dim)
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

    // ── Right pane: track table ──
    let filtered: Vec<&gtm_core::track::TrackInfo> = if app.search_query.is_empty() {
        app.tracks_cache.iter().collect()
    } else {
        let q = app.search_query.to_lowercase();
        app.tracks_cache
            .iter()
            .filter(|t| {
                t.title.to_lowercase().contains(&q)
                    || t.artist.to_lowercase().contains(&q)
                    || t.album.to_lowercase().contains(&q)
            })
            .collect()
    };

    let category_label = LIBRARY_CATEGORIES.get(app.library_category).unwrap_or(&"All Tracks");
    let total_dur: u64 = filtered.iter().map(|t| t.duration as u64).sum();
    let hours = total_dur / 3600;
    let mins = (total_dur % 3600) / 60;
    let stats_line = format!(
        " {} tracks | {}h {}m ",
        filtered.len(),
        hours,
        mins,
    );

    let sel = app.scroll_offset.min(filtered.len().saturating_sub(1));

    // Up Next header
    let current_id = app.state.current_track.as_ref().map(|t| t.id);
    let up_next = if let (Some(t), Some(_)) = (app.state.current_track.as_ref(), current_id.and_then(|id| filtered.iter().position(|ft| ft.id == id))) {
        format!(" Up Next: {} - {}    {}", t.artist, t.title, stats_line)
    } else if !filtered.is_empty() {
        format!(" {} · {} tracks", category_label, filtered.len())
    } else {
        format!(" {} · 0 tracks", category_label)
    };

    // Column widths
    let pane_w = panes[1].width as usize;
    let num_w = 4;   // "#  "
    let dur_w = 9;   // " 00:00:00" or " 99:99"
    let br_w = 8;    // " 320kbps"
    let title_w = pane_w.saturating_sub(num_w + dur_w + br_w + 4).max(10);

    let header_fmt = format!(
        "{:>w1$}│ {:<w2$} │ {:>w3$} │ {:>w4$}",
        "#", "Title / Artist / Album", "Duration", "Bitrate",
        w1 = num_w - 1, w2 = title_w, w3 = dur_w - 1, w4 = br_w - 1
    );
    let sep_line = format!(
        "{:─>w1$}┼{:─>w2$}┼{:─>w3$}┼{:─>w4$}",
        "", "", "", "",
        w1 = num_w + 1, w2 = title_w + 2, w3 = dur_w + 1, w4 = br_w
    );

    let mut right_lines = vec![
        Line::from(Span::styled(up_next, Style::default().fg(app.theme.fg_dim))),
        Line::from(""),
        Line::from(Span::styled(header_fmt, Style::default().fg(app.theme.fg_dim))),
        Line::from(Span::styled(sep_line, Style::default().fg(app.theme.fg_dim))),
    ];

    for (i, track) in filtered.iter().enumerate() {
        let is_current = app.state.current_track.as_ref().map(|t| t.id) == Some(track.id);
        let is_sel = i == sel && !left_focus;
        let prefix = if is_current { ">" } else if is_sel { " " } else { " " };
        let num_str = format!("{}{:02}", prefix, i + 1);
        let dur = format_duration_short(track.duration as u64);
        let br = track.bitrate.map(|b| format!("{}kbps", b)).unwrap_or_default();
        let title_artist = format!("{} - {}", track.artist, track.title);

        let row = format!(
            "{:<w1$}│ {:<w2$} │ {:>w3$} │ {:>w4$}",
            num_str, title_artist, dur, br,
            w1 = num_w, w2 = title_w, w3 = dur_w, w4 = br_w
        );

        let style = if is_current {
            Style::default().fg(app.theme.accent).add_modifier(Modifier::BOLD)
        } else if is_sel {
            Style::default().fg(app.theme.selection_fg).bg(app.theme.selection_bg)
        } else {
            Style::default()
        };

        right_lines.push(Line::from(Span::styled(row, style)));
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

    // ── Stats bar ──
    let stats = Paragraph::new(Line::from(vec![
        Span::raw("  "),
        Span::styled(stats_line, Style::default().fg(app.theme.fg_dim)),
    ]));
    f.render_widget(stats, stats_area);
}

const SETTINGS_ICONS: &[&str] = &["♫", "▶", "✧", "⚙", "☊"];
const SETTINGS_CATEGORIES: &[&str] = &["Audio", "YouTube", "Appearance", "System", "Spotify"];

fn render_settings(f: &mut ratatui::Frame, area: Rect, app: &App) {
    let panes = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Length(22), Constraint::Min(0)])
        .split(area);

    // ── Left pane: categories with icons ──
    let settings_focus = app.settings_pane_focus;
    let left_items: Vec<ListItem> = SETTINGS_CATEGORIES
        .iter()
        .enumerate()
        .map(|(i, cat)| {
            let icon = SETTINGS_ICONS.get(i).unwrap_or(&" ");
            let is_active = i == app.settings_category;
            let style = if is_active && settings_focus {
                Style::default().fg(app.theme.selection_fg).bg(app.theme.selection_bg)
            } else if is_active {
                Style::default().fg(app.theme.accent)
            } else {
                Style::default().fg(app.theme.fg_dim)
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
            vec![
                format!("Repeat          [ {:?}       ▶ ]", app.state.repeat),
                format!("Shuffle         [ {} ]", if app.state.shuffle { "●   On " } else { "○   Off" }),
                if crossfade_on {
                    format!("Crossfade       [ ● ]  On  {}s", crossfade_dur)
                } else {
                    "Crossfade       [ ○ ]  Off".to_string()
                },
            ]
        }
        3 => vec![
            "Theme           [ Cyberdeck  ▶ ]".to_string(),
            "Notifications   [ ● ]  On".to_string(),
        ],
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
    for (i, item) in items.iter().enumerate() {
        if i == 1 && app.settings_category == 1 {
            lines.push(Line::from(Span::styled(
                " ──────────────── YouTube ─────────────────",
                Style::default().fg(app.theme.accent),
            )));
        }
        lines.push(Line::from(Span::styled(item, Style::default().fg(app.theme.fg))));
    }
    lines.push(Line::from(""));
    match app.settings_category {
        0 => lines.push(Line::from(Span::styled(" Volume: Adjust playback volume. Mute: Toggle mute.", Style::default().fg(app.theme.fg_dim)))),
        1 => lines.push(Line::from(Span::styled(" YouTube integration: JS runtime, download limits, search prefs.", Style::default().fg(app.theme.fg_dim)))),
        2 => lines.push(Line::from(Span::styled(" Repeat/Shuffle/Crossfade playback settings.", Style::default().fg(app.theme.fg_dim)))),
        3 => lines.push(Line::from(Span::styled(" Theme and notification display settings.", Style::default().fg(app.theme.fg_dim)))),
        4 => lines.push(Line::from(Span::styled(" Spotify integration status.", Style::default().fg(app.theme.fg_dim)))),
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
        .border_type(BorderType::Plain)
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
        OverlayId::ThemePicker => render_theme_picker_overlay(f, inner, app),
        _ => {
            let p = Paragraph::new(format!("{} overlay", top.id.title()))
                .alignment(Alignment::Center)
                .style(Style::default().fg(app.theme.fg_dim));
            f.render_widget(p, inner);
        }
    }
}

fn render_queue_overlay(f: &mut ratatui::Frame, area: Rect, app: &App) {
    let sel = app.overlays.top().map_or(0, |o| o.selected);

    let pane_w = area.width as usize;
    let num_w = 4;
    let dur_w = 9;
    let br_w = 8;
    let title_w = pane_w.saturating_sub(num_w + dur_w + br_w + 4).max(10);

    let header_fmt = format!(
        "{:>w1$}│ {:<w2$} │ {:>w3$} │ {:>w4$}",
        "#", "Title / Artist / Album", "Duration", "Bitrate",
        w1 = num_w - 1, w2 = title_w, w3 = dur_w - 1, w4 = br_w - 1
    );
    let sep_line = format!(
        "{:─>w1$}┼{:─>w2$}┼{:─>w3$}┼{:─>w4$}",
        "", "", "", "",
        w1 = num_w + 1, w2 = title_w + 2, w3 = dur_w + 1, w4 = br_w
    );

    let mut lines = vec![
        Line::from(Span::styled(header_fmt, Style::default().fg(app.theme.fg_dim))),
        Line::from(Span::styled(sep_line, Style::default().fg(app.theme.fg_dim))),
    ];

    for (i, track) in app.queue_cache.iter().enumerate() {
        let is_current = i == app.queue_cursor;
        let is_sel = i == sel;
        let prefix = if is_current { ">" } else if is_sel { " " } else { " " };
        let num_str = format!("{}{:02}", prefix, i + 1);
        let dur = format_duration_short(track.duration as u64);
        let br = track.bitrate.map(|b| format!("{}kbps", b)).unwrap_or_default();
        let title_artist = format!("{} - {}", track.artist, track.title);

        let row = format!(
            "{:<w1$}│ {:<w2$} │ {:>w3$} │ {:>w4$}",
            num_str, title_artist, dur, br,
            w1 = num_w, w2 = title_w, w3 = dur_w, w4 = br_w
        );

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
    let block = Block::default()
        .borders(Borders::ALL)
        .title(" Queue ")
        .border_type(BorderType::Plain);

    let inner = block.inner(area);
    f.render_widget(block, area);
    f.render_widget(para, inner);
}

fn render_yt_search_overlay(f: &mut ratatui::Frame, area: Rect, app: &App) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(3), Constraint::Min(0)])
        .split(area);

    let query = app.overlays.top().map_or(String::new(), |o| o.query.clone());
    let cursor = if app.overlays.top().map_or(false, |o| o.id == OverlayId::YTSearch) { "_" } else { "" };
    let search_input = Paragraph::new(format!(" > {}{}", query, cursor))
        .style(Style::default().fg(app.theme.fg));
    f.render_widget(search_input, chunks[0]);

    let items: Vec<ListItem> = app
        .yt_results_cache
        .iter()
        .enumerate()
        .map(|(i, r)| {
            let dur = format_duration(r.duration as u64);
            let prefix = if i == app.overlays.top().map_or(0, |o| o.selected) { " > " } else { "   " };
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
            .border_type(BorderType::Plain),
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

    let search_input = Paragraph::new(format!(" > {}_", query))
        .style(Style::default().fg(app.theme.fg));
    f.render_widget(search_input, chunks[0]);

    let sel = app.overlays.top().map_or(0, |o| o.selected.min(filtered.len().saturating_sub(1)));
    let items: Vec<ListItem> = filtered
        .iter()
        .enumerate()
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
            let is_playing = app.state.status == PlaybackStatus::Playing;
            let status_icon = match app.state.status {
                PlaybackStatus::Playing => "\u{25b6}",
                PlaybackStatus::Paused => "\u{23f8}",
                PlaybackStatus::Stopped => "\u{25a0}",
            };
            let vol_str = if app.state.mute { "MUTE".into() } else { format!("{:>3}%", app.state.volume) };
            let repeat_str = match app.state.repeat {
                RepeatMode::Off => "",
                RepeatMode::One => " [1]",
                RepeatMode::All => " [A]",
            };
            let shuffle_str = if app.state.shuffle { " [S]" } else { "" };
            let vol_ratio = if app.state.mute { 0.0 } else { app.state.volume as f64 / 100.0 };
            let vol_bar = render_progress_line(vol_ratio, 8);
            let status_text = format!(" {} {} {} {}{}", status_icon, vol_bar, vol_str, repeat_str, shuffle_str);

            // Left segment: fixed width for compact status bar
            let left_w = (status_text.len() as u16 + 2).min(area.width.saturating_sub(20));
            let chunks = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([Constraint::Length(left_w), Constraint::Min(0)])
                .split(area);

            // Left: status
            let status_bg = if is_playing { app.theme.accent } else { app.theme.fg_dim };
            let status_fg = if is_playing { app.theme.fg_bright } else { app.theme.overlay_bg };
            let status_para = Paragraph::new(status_text)
                .style(Style::default().fg(status_fg).bg(status_bg));
            f.render_widget(status_para, chunks[0]);

            // Right: progress + clock
            let right_area = chunks[1];
            let clock_str = local_time_str();
            if let Some(ref track) = app.state.current_track {
                let pos = app.display_position as u64;
                let dur = track.duration as u64;
                let ratio = if dur > 0 { pos as f64 / dur as f64 } else { 0.0 };
                let time_str = format!(" {} / {} ", format_duration(pos), format_duration(dur));
                let clock_w = clock_str.len() as u16;
                let bar_w = right_area.width.saturating_sub(time_str.len() as u16 + clock_w + 2).max(4) as usize;
                let bar = render_progress_line(ratio, bar_w);
                let progress_text = format!(" {} {} {}", bar, time_str, clock_str);
                let progress_para = Paragraph::new(progress_text)
                    .style(Style::default().fg(app.theme.fg).bg(app.theme.border));
                f.render_widget(progress_para, right_area);
            } else {
                let progress_text = format!(" \u{25a0} stopped  {}", clock_str);
                let progress_para = Paragraph::new(progress_text)
                    .style(Style::default().fg(app.theme.fg_dim).bg(app.theme.border));
                f.render_widget(progress_para, right_area);
            }
        }
        InputMode::Searching => {
            let input = Paragraph::new(format!(" > {}_", app.search_query))
                .style(Style::default().fg(app.theme.fg));
            f.render_widget(input, area);
        }
        InputMode::Command => {
            let input = Paragraph::new(format!(" :{}_", app.search_query))
                .style(Style::default().fg(app.theme.fg));
            f.render_widget(input, area);
        }
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

    let search_input = Paragraph::new(format!(" > {}_", query))
        .style(Style::default().fg(app.theme.fg));
    f.render_widget(search_input, chunks[0]);

    let sel = app.overlays.top().map_or(0, |o| o.selected.min(filtered.len().saturating_sub(1)));
    let list_items: Vec<ListItem> = filtered
        .iter()
        .enumerate()
        .map(|(i, cmd)| {
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

    let graph = render_eq_graph();
    let all_items: Vec<ListItem> = list_items.into_iter().chain(
        std::iter::once(ListItem::new(""))
            .chain(std::iter::once(ListItem::new(graph).style(Style::default().fg(app.theme.accent))))
    ).collect();

    let list = List::new(all_items).block(
        Block::default()
            .borders(Borders::ALL)
            .title(" Equalizer ")
            .border_type(BorderType::Plain),
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
    let list_items: Vec<ListItem> = THEMES.iter().enumerate().map(|(i, entry)| {
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

fn render_track_info_popup(f: &mut ratatui::Frame, area: Rect, app: &App) {
    let Some(ref track) = app.state.current_track else {
        let p = Paragraph::new("No track playing")
            .alignment(Alignment::Center)
            .style(Style::default().fg(app.theme.fg_dim));
        f.render_widget(p, area);
        return;
    };

    let fav_symbol = if track.favourite { "\u{2665}" } else { "\u{2661}" };
    let fmt_parts: Vec<String> = {
        let mut p = Vec::new();
        if let Some(b) = track.bitrate { p.push(format!("{}kbps", b)); }
        if let Some(s) = track.samplerate { p.push(format!("{}kHz", (s as f64 / 1000.0).round() as u32)); }
        p
    };
    let format_str = if fmt_parts.is_empty() { "Unknown".into() } else { fmt_parts.join(" | ") };
    let genre_str = if track.genre.is_empty() { "Unknown".into() } else { track.genre.clone() };
    let year_str = track.year.map(|y| y.to_string()).unwrap_or_else(|| "Unknown".into());
    let track_num = track.track_number.map(|n| n.to_string()).unwrap_or_else(|| "-".into());
    let pos = format_duration(app.display_position as u64);
    let dur = format_duration(track.duration as u64);

    let lines = vec![
        Line::from(Span::styled(" Track Info", Style::default().fg(app.theme.accent).add_modifier(Modifier::BOLD))),
        Line::from(""),
        Line::from(vec![
            Span::styled(" Title:   ", Style::default().fg(app.theme.fg_dim)),
            Span::styled(&track.title, Style::default().fg(app.theme.fg_bright).add_modifier(Modifier::BOLD)),
        ]),
        Line::from(vec![
            Span::styled(" Artist:  ", Style::default().fg(app.theme.fg_dim)),
            Span::styled(&track.artist, Style::default().fg(app.theme.fg)),
        ]),
        Line::from(vec![
            Span::styled(" Album:   ", Style::default().fg(app.theme.fg_dim)),
            Span::styled(&track.album, Style::default().fg(app.theme.fg)),
        ]),
        Line::from(vec![
            Span::styled(" Genre:   ", Style::default().fg(app.theme.fg_dim)),
            Span::styled(genre_str, Style::default().fg(app.theme.fg)),
        ]),
        Line::from(vec![
            Span::styled(" Year:    ", Style::default().fg(app.theme.fg_dim)),
            Span::styled(year_str, Style::default().fg(app.theme.fg)),
        ]),
        Line::from(vec![
            Span::styled(" Track #: ", Style::default().fg(app.theme.fg_dim)),
            Span::styled(track_num, Style::default().fg(app.theme.fg)),
        ]),
        Line::from(vec![
            Span::styled(" Format:  ", Style::default().fg(app.theme.fg_dim)),
            Span::styled(format_str, Style::default().fg(app.theme.accent)),
        ]),
        Line::from(vec![
            Span::styled(" Favourite:", Style::default().fg(app.theme.fg_dim)),
            Span::styled(format!(" {}", fav_symbol), Style::default().fg(if track.favourite { app.theme.error } else { app.theme.fg_dim })),
        ]),
        Line::from(vec![
            Span::styled(" Progress:", Style::default().fg(app.theme.fg_dim)),
            Span::styled(format!(" {} / {}", pos, dur), Style::default().fg(app.theme.fg)),
        ]),
        Line::from(""),
        Line::from(Span::styled(" [t] Close  ", Style::default().fg(app.theme.fg_dim))),
    ];

    let p = Paragraph::new(lines)
        .alignment(Alignment::Left)
        .style(Style::default().bg(app.theme.overlay_bg));
    let block = Block::default()
        .borders(Borders::ALL)
        .title(" Track Info ")
        .border_type(BorderType::Plain)
        .style(Style::default().bg(app.theme.overlay_bg));
    let inner = block.inner(area);
    f.render_widget(block, area);
    f.render_widget(p, inner);
}

/// Return the local time as " HH:MM " using the system clock.
fn local_time_str() -> String {
    let dur = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    let secs = dur.as_secs();
    let total_min = (secs / 60) as i64;
    let h = (total_min / 60) % 24;
    let m = total_min % 60;
    format!(" {:02}:{:02} ", h, m)
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
