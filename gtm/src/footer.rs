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
#[allow(dead_code)]
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
    EqPreset,
    SleepTimer,
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

// The different presets available for the footer, got inspiration from lualine.nvim
// I wanted to be able to have that very informative env where I can see the internals of my player,
// like the next track (really useful when you have shuffle on)
// TODO: What other internals would I like to know ?
pub fn presets() -> Vec<FooterPreset> {
    vec![
        FooterPreset {
            name: "Default",
            a: vec![FooterModule::Playback],
            b: vec![FooterModule::KeyAction],
            c: vec![FooterModule::Queue],
            x: vec![FooterModule::Repeat, FooterModule::Shuffle],
            y: vec![FooterModule::Volume, FooterModule::EqPreset],
            z: vec![FooterModule::SleepTimer],
        },
        // The bare minimum especically, when I'm on termux or running on a device with small
        // viewport
        FooterPreset {
            name: "Minimal",
            a: vec![FooterModule::KeyAction, FooterModule::Playback],
            b: vec![],
            c: vec![FooterModule::EqPreset],
            x: vec![],
            y: vec![],
            z: vec![FooterModule::SleepTimer],
        },
        FooterPreset {
            name: "Full",
            a: vec![FooterModule::KeyAction, FooterModule::Playback],
            b: vec![FooterModule::Title],
            c: vec![
                FooterModule::Volume,
                FooterModule::Repeat,
                FooterModule::Shuffle,
                FooterModule::EqPreset,
            ],
            x: vec![],
            y: vec![],
            z: vec![FooterModule::SleepTimer, FooterModule::Progress],
        },
    ]
}

pub fn num_presets() -> usize {
    3
}

/// Collect the rendered spans for a preset so they can be cached across frames.
pub fn collect_preset_spans(
    app: &App,
    preset: &FooterPreset,
) -> Option<(
    Vec<Span<'static>>,
    ratatui::style::Color,
    ratatui::style::Color,
)> {
    let groups = build_groups(preset, app);
    let mut all_spans: Vec<Span<'static>> = Vec::new();
    for (i, group) in groups.iter().enumerate() {
        let parts = render_modules(&group.modules, app);
        if parts.is_empty() {
            continue;
        }
        if i > 0 {
            all_spans.push(Span::raw(" "));
        }
        all_spans.push(Span::raw(" "));
        for (j, (text, color)) in parts.iter().enumerate() {
            if j > 0 {
                all_spans.push(Span::raw(" "));
            }
            let fg = crate::ui::readable_fg(group.bg, *color, app.theme.fg_bright);
            all_spans.push(Span::styled(text.clone(), Style::default().fg(fg)));
        }
        all_spans.push(Span::raw(" "));
    }
    let left_bg = if all_spans.is_empty() {
        app.theme.fg_dim
    } else {
        groups[0].bg
    };
    let right_bg = app.theme.border;
    Some((all_spans, left_bg, right_bg))
}

struct FooterGroup<'a> {
    modules: Vec<&'a FooterModule>,
    bg: ratatui::style::Color,
}

fn build_groups<'a>(preset: &'a FooterPreset, app: &App) -> Vec<FooterGroup<'a>> {
    let is_playing = app.state.status == PlaybackStatus::Playing;
    let status_bg = if is_playing {
        app.theme.accent
    } else {
        app.theme.fg_dim
    };
    let mut groups = Vec::new();

    // Collect all modules from all preset fields
    let all: Vec<&'a FooterModule> = preset
        .a
        .iter()
        .chain(preset.b.iter())
        .chain(preset.c.iter())
        .chain(preset.x.iter())
        .chain(preset.y.iter())
        .chain(preset.z.iter())
        .collect();

    // Group 1 (left): Status/info modules
    let mut left = Vec::new();
    for m in &all {
        if matches!(
            m,
            FooterModule::Playback
                | FooterModule::Volume
                | FooterModule::EqPreset
                | FooterModule::Queue
                | FooterModule::Repeat
                | FooterModule::Shuffle
                | FooterModule::Progress
                | FooterModule::Title
                | FooterModule::Backend
                | FooterModule::Device
                | FooterModule::System
        ) {
            left.push(*m);
        }
    }
    if !left.is_empty() {
        groups.push(FooterGroup {
            modules: left,
            bg: status_bg,
        });
    }

    // Group 2 (middle): KeyAction + SleepTimer
    let mut middle = Vec::new();
    for m in &all {
        if matches!(m, FooterModule::KeyAction | FooterModule::SleepTimer) {
            middle.push(*m);
        }
    }
    if !middle.is_empty() {
        groups.push(FooterGroup {
            modules: middle,
            bg: app.theme.fg_dim,
        });
    }

    // Group 3 (right): Clock
    groups.push(FooterGroup {
        modules: vec![&FooterModule::Clock],
        bg: app.theme.fg_dim,
    });

    groups
}

pub fn render_preset(f: &mut Frame, area: Rect, app: &App, preset: &FooterPreset) {
    let groups = build_groups(preset, app);

    // Calculate width for each group
    let mut group_widths: Vec<u16> = Vec::new();
    for group in &groups {
        let parts = render_modules(&group.modules, app);
        let s: String = parts
            .iter()
            .map(|(t, _)| t.as_str())
            .collect::<Vec<_>>()
            .join(" ");
        let w = if s.is_empty() { 0 } else { s.len() as u16 + 2 };
        group_widths.push(w);
    }

    let total: u16 = group_widths.iter().sum();
    if total == 0 {
        return;
    }

    // If terminal too narrow, show only as much as fits
    let mut constraints: Vec<Constraint> = group_widths
        .iter()
        .map(|w| {
            if *w > 0 {
                Constraint::Length(*w)
            } else {
                Constraint::Length(0)
            }
        })
        .collect();
    // Add fill for remaining space
    constraints.push(Constraint::Min(0));

    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints(constraints)
        .split(area);

    for (i, group) in groups.iter().enumerate() {
        let parts = render_modules(&group.modules, app);
        if parts.is_empty() || group_widths[i] == 0 {
            continue;
        }
        if chunks[i].x >= area.x + area.width {
            break;
        }

        let mut spans = Vec::new();
        spans.push(Span::raw(" "));
        for (j, (text, color)) in parts.iter().enumerate() {
            if j > 0 {
                spans.push(Span::raw(" "));
            }
            let fg = crate::ui::readable_fg(group.bg, *color, app.theme.fg_bright);
            spans.push(Span::styled(text.clone(), Style::default().fg(fg)));
        }
        spans.push(Span::raw(" "));

        f.render_widget(
            Paragraph::new(Line::from(spans)).style(Style::default().bg(group.bg)),
            chunks[i],
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
        FooterModule::EqPreset => theme.syn_type,
        FooterModule::SleepTimer => theme.syn_keyword,
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
            FooterModule::EqPreset => render_eq_preset(app),
            FooterModule::SleepTimer => render_sleep_timer(app),
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
        String::new,
        |t| {
            if t.artist.is_empty() {
                t.title.clone()
            } else {
                format!("{} \u{2013} {}", t.artist, t.title)
            }
        },
    );
    if raw.len() > 30 {
        let s = raw
            .chars()
            .cycle()
            .skip(app.footer_title_scroll % raw.len())
            .take(30)
            .collect::<String>();
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
    if app.state.shuffle {
        "S".into()
    } else {
        String::new()
    }
}

fn render_eq_preset(app: &App) -> String {
    if app.state.eq_enabled {
        format!("EQ:{}", app.state.eq_preset.label())
    } else {
        String::new()
    }
}

fn render_sleep_timer(app: &App) -> String {
    if let Some(secs) = app.state.sleep_timer {
        let m = secs / 60;
        let s = secs % 60;
        format!("zzz {}:{:02}", m, s)
    } else {
        String::new()
    }
}

fn render_progress(app: &App) -> String {
    let track = match app.state.current_track.as_ref() {
        Some(t) => t,
        None => return String::new(),
    };
    let pos = app.display_position as u64;
    let dur = track.duration as u64;
    if dur == 0 {
        return String::new();
    }
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
    if len == 0 {
        return String::new();
    }
    let cursor = app.queue_cursor;
    let next = if cursor + 1 < len {
        format!(
            " \u{2192} {}",
            app.queue_cache[cursor + 1]
                .title
                .chars()
                .take(20)
                .collect::<String>()
        )
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
    let cpus = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1);
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
            let kb: u64 = rest.split_whitespace().next()?.parse().ok()?;
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
