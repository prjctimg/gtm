// Copyright (c) 2025 - present
// Author: prjctimg <prjctimg@outlook.com>
// Footer bar: system info, keybindings, and playback status
//
// This is free software released under the GPL-3.0 license.

use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

use gtm_core::state::PlaybackStatus;

use crate::app::App;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FooterModule {
    Playback,
    Title,
    Volume,
    Repeat,
    Shuffle,
    Progress,
    Queue,
    Clock,
    KeyAction,
    Backend,
    System,
    Device,
}

#[derive(Debug, Clone)]
pub struct FooterPreset {
    pub name: &'static str,
    pub a: Vec<FooterModule>,
    pub b: Vec<FooterModule>,
    pub c: Vec<FooterModule>,
    pub x: Vec<FooterModule>,
    pub y: Vec<FooterModule>,
    pub z: Vec<FooterModule>,
}

pub fn presets() -> Vec<FooterPreset> {
    vec![
        FooterPreset {
            name: "Default",
            a: vec![FooterModule::Playback],
            b: vec![FooterModule::Title],
            c: vec![FooterModule::Volume, FooterModule::Queue],
            x: vec![],
            y: vec![FooterModule::KeyAction],
            z: vec![FooterModule::Clock, FooterModule::Progress],
        },
        FooterPreset {
            name: "Minimal",
            a: vec![FooterModule::Playback],
            b: vec![FooterModule::Title],
            c: vec![],
            x: vec![],
            y: vec![],
            z: vec![FooterModule::Clock],
        },
        FooterPreset {
            name: "Full",
            a: vec![FooterModule::Playback],
            b: vec![FooterModule::Title],
            c: vec![FooterModule::Volume, FooterModule::Repeat, FooterModule::Shuffle],
            x: vec![FooterModule::Backend, FooterModule::Device],
            y: vec![FooterModule::KeyAction],
            z: vec![FooterModule::System, FooterModule::Clock, FooterModule::Progress],
        },
    ]
}

pub fn num_presets() -> usize {
    3
}

/// Collect the rendered spans for a preset so they can be cached across frames.
pub fn collect_preset_spans(app: &App, preset: &FooterPreset) -> Option<(Vec<Span<'static>>, ratatui::style::Color, ratatui::style::Color)> {
    let left_modules: Vec<&FooterModule> = preset.a.iter()
        .chain(preset.b.iter())
        .chain(preset.c.iter())
        .collect();
    let left_parts = render_modules(&left_modules, app);
    let left_bg = if app.state.status == PlaybackStatus::Playing { app.theme.accent } else { app.theme.fg_dim };
    let right_bg = app.theme.border;
    if left_parts.is_empty() {
        return None;
    }
    let mut spans: Vec<Span<'static>> = Vec::new();
    spans.push(Span::raw(" "));
    for (i, (text, color)) in left_parts.iter().enumerate() {
        if i > 0 { spans.push(Span::raw(" ")); }
        let fg = crate::ui::readable_fg(left_bg, *color, app.theme.fg_bright);
        spans.push(Span::styled(text.clone(), Style::default().fg(fg)));
    }
    spans.push(Span::raw(" "));
    Some((spans, left_bg, right_bg))
}

pub fn render_preset(f: &mut Frame, area: Rect, app: &App, preset: &FooterPreset) {
    let left_modules: Vec<&FooterModule> = preset.a.iter()
        .chain(preset.b.iter())
        .chain(preset.c.iter())
        .collect();

    let right_modules: Vec<&FooterModule> = preset.x.iter()
        .chain(preset.y.iter())
        .chain(preset.z.iter())
        .collect();

    let left_parts = render_modules(&left_modules, app);
    let right_parts = render_modules(&right_modules, app);

    let left_str: String = left_parts.iter().map(|(s, _)| s.as_str()).collect::<Vec<_>>().join(" ");
    let right_str: String = right_parts.iter().map(|(s, _)| s.as_str()).collect::<Vec<_>>().join(" ");

    let left_w = if left_str.is_empty() { 0u16 } else { left_str.len() as u16 + 2 };
    let right_w = if right_str.is_empty() { 0u16 } else { right_str.len() as u16 + 2 };

    let total_needed = left_w + right_w;
    let is_playing = app.state.status == PlaybackStatus::Playing;

    let left_bg = if is_playing { app.theme.accent } else { app.theme.fg_dim };
    let right_bg = app.theme.border;

    if left_w == 0 && right_w == 0 {
        return;
    }

    if total_needed as u16 >= area.width {
        let mut spans = Vec::new();
        for (i, (text, color)) in left_parts.iter().enumerate() {
            if i > 0 { spans.push(Span::raw(" ")); }
            let fg = crate::ui::readable_fg(left_bg, *color, app.theme.fg_bright);
            spans.push(Span::styled(text.clone(), Style::default().fg(fg)));
        }
        f.render_widget(
            Paragraph::new(Line::from(spans))
                .style(Style::default().bg(left_bg)),
            area,
        );
        return;
    }

    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Length(left_w), Constraint::Min(0)])
        .split(area);

    if left_w > 0 {
        let mut spans = Vec::new();
        spans.push(Span::raw(" "));
        for (i, (text, color)) in left_parts.iter().enumerate() {
            if i > 0 { spans.push(Span::raw(" ")); }
            let fg = crate::ui::readable_fg(left_bg, *color, app.theme.fg_bright);
            spans.push(Span::styled(text.clone(), Style::default().fg(fg)));
        }
        spans.push(Span::raw(" "));
        f.render_widget(
            Paragraph::new(Line::from(spans))
                .style(Style::default().bg(left_bg)),
            chunks[0],
        );
    }

    if right_w > 0 {
        let mut spans = Vec::new();
        spans.push(Span::raw(" "));
        for (i, (text, color)) in right_parts.iter().enumerate() {
            if i > 0 { spans.push(Span::raw(" ")); }
            let fg = crate::ui::readable_fg(right_bg, *color, app.theme.fg_bright);
            spans.push(Span::styled(text.clone(), Style::default().fg(fg)));
        }
        spans.push(Span::raw(" "));
        f.render_widget(
            Paragraph::new(Line::from(spans))
                .style(Style::default().bg(right_bg)),
            chunks[1],
        );
    }
}

fn module_color(m: &FooterModule, theme: &crate::theme::AppTheme) -> ratatui::style::Color {
    match m {
        FooterModule::Playback => theme.syn_function,
        FooterModule::Title => theme.syn_string,
        FooterModule::Volume => theme.syn_constant,
        FooterModule::Repeat => theme.syn_keyword,
        FooterModule::Shuffle => theme.syn_type,
        FooterModule::Progress => theme.syn_variable,
        FooterModule::Queue => theme.syn_comment,
        FooterModule::Clock => theme.fg_dim,
        FooterModule::KeyAction => theme.warning,
        FooterModule::Backend => theme.fg_dim,
        FooterModule::System => theme.fg_dim,
        FooterModule::Device => theme.fg_dim,
    }
}

fn render_modules(modules: &[&FooterModule], app: &App) -> Vec<(String, ratatui::style::Color)> {
    let mut parts = Vec::new();
    for m in modules {
        let text = match m {
            FooterModule::Playback => render_playback(app),
            FooterModule::Title => render_title(app),
            FooterModule::Volume => render_volume(app),
            FooterModule::Repeat => render_repeat(app),
            FooterModule::Shuffle => render_shuffle(app),
            FooterModule::Progress => render_progress(app),
            FooterModule::Queue => render_queue(app),
            FooterModule::Clock => render_clock(),
            FooterModule::KeyAction => render_keyaction(app),
            FooterModule::Backend => render_backend(),
            FooterModule::System => render_system(),
            FooterModule::Device => render_device(app),
        };
        if !text.is_empty() {
            parts.push((text, module_color(m, &app.theme)));
        }
    }
    parts
}

fn render_playback(app: &App) -> String {
    match app.state.status {
        PlaybackStatus::Playing => "\u{25b6}".into(),
        PlaybackStatus::Paused => "\u{23f8}".into(),
        PlaybackStatus::Stopped => "\u{25a0}".into(),
    }
}

fn render_title(app: &App) -> String {
    let raw = app.state.current_track.as_ref().map_or_else(
        || String::new(),
        |t| {
            if t.artist.is_empty() {
                t.title.clone()
            } else {
                format!("{} \u{2013} {}", t.artist, t.title)
            }
        },
    );
    if raw.len() > 30 {
        let s = raw.chars().cycle().skip(app.footer_title_scroll % raw.len()).take(30).collect::<String>();
        format!("{} \u{2026}", s)
    } else {
        raw
    }
}

fn render_volume(app: &App) -> String {
    if app.state.mute {
        "MUTE".into()
    } else {
        format!("{:>3}%", app.state.volume)
    }
}

fn render_repeat(app: &App) -> String {
    match app.state.repeat {
        gtm_core::state::RepeatMode::Off => String::new(),
        gtm_core::state::RepeatMode::One => "1".into(),
        gtm_core::state::RepeatMode::All => "A".into(),
    }
}

fn render_shuffle(app: &App) -> String {
    if app.state.shuffle { "S".into() } else { String::new() }
}

fn render_progress(app: &App) -> String {
    let track = match app.state.current_track.as_ref() {
        Some(t) => t,
        None => return String::new(),
    };
    let pos = app.display_position as u64;
    let dur = track.duration as u64;
    if dur == 0 { return String::new(); }
    let ratio = (pos as f64 / dur as f64).clamp(0.0, 1.0);
    let time_str = format!("{} / {}", format_duration(pos), format_duration(dur));
    let bar_w = 14usize;
    let progress = crate::ui::render_progress_variant(ratio, bar_w, app);
    format!("{} {}", progress, time_str)
}

fn format_duration(secs: u64) -> String {
    let h = secs / 3600;
    let m = (secs % 3600) / 60;
    let s = secs % 60;
    if h > 0 {
        format!("{}:{:02}:{:02}", h, m, s)
    } else {
        format!("{}:{:02}", m, s)
    }
}

fn render_queue(app: &App) -> String {
    let len = app.queue_cache.len();
    if len == 0 { return String::new(); }
    let cursor = app.queue_cursor;
    let next = if cursor + 1 < len {
        format!(" \u{2192} {}", app.queue_cache[cursor + 1].title.chars().take(20).collect::<String>())
    } else {
        String::new()
    };
    format!("[{}/{}]{}", cursor + 1, len, next)
}

fn render_clock() -> String {
    let s = crate::ui::local_time_str();
    s.trim().to_string()
}

fn render_keyaction(app: &App) -> String {
    if let Some((ref action, expires)) = app.last_action_name {
        if std::time::Instant::now() < expires {
            return format!("[{}]", action);
        }
    }
    String::new()
}

fn render_backend() -> String {
    let rust_ver = option_env!("VERGEN_RUSTC_SEMVER").unwrap_or("?");
    let crate_ver = option_env!("CARGO_PKG_VERSION").unwrap_or("?");
    format!("rust {} • v{}", rust_ver, crate_ver)
}

fn render_system() -> String {
    let os = std::env::consts::OS;
    let arch = std::env::consts::ARCH;
    let cpus = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(1);
    let mem = read_process_memory_kb();
    if let Some(kb) = mem {
        if kb > 1024 * 1024 {
            format!("{} {} • {}GB {}CPU", os, arch, kb / (1024 * 1024), cpus)
        } else {
            format!("{} {} • {}MB {}CPU", os, arch, kb / 1024, cpus)
        }
    } else {
        format!("{} {} • {}CPU", os, arch, cpus)
    }
}

fn read_process_memory_kb() -> Option<u64> {
    let status = std::fs::read_to_string("/proc/self/status").ok()?;
    for line in status.lines() {
        if let Some(rest) = line.strip_prefix("VmRSS:") {
            let kb: u64 = rest.trim().split_whitespace().next()?.parse().ok()?;
            return Some(kb);
        }
    }
    None
}

fn render_device(app: &App) -> String {
    let track_count = app.tracks_cache.len();
    let total_dur: u64 = app.tracks_cache.iter().map(|t| t.duration as u64).sum();
    let hours = total_dur / 3600;
    let mins = (total_dur % 3600) / 60;
    format!("{} tracks • {}h{}m", track_count, hours, mins)
}
