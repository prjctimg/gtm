use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::Style;
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

pub fn render_preset(f: &mut Frame, area: Rect, app: &App, preset: &FooterPreset) {
    let left_modules: Vec<&FooterModule> = preset.a.iter()
        .chain(preset.b.iter())
        .chain(preset.c.iter())
        .collect();

    let right_modules: Vec<&FooterModule> = preset.x.iter()
        .chain(preset.y.iter())
        .chain(preset.z.iter())
        .collect();

    let left_str = render_modules(&left_modules, app);
    let right_str = render_modules(&right_modules, app);

    let left_w = if left_str.is_empty() { 0u16 } else { left_str.len() as u16 + 2 };
    let right_w = if right_str.is_empty() { 0u16 } else { right_str.len() as u16 + 2 };

    let total_needed = left_w + right_w;
    let is_playing = app.state.status == PlaybackStatus::Playing;

    let left_bg = if is_playing { app.theme.accent } else { app.theme.fg_dim };
    let left_fg = if is_playing {
        crate::ui::readable_fg(app.theme.accent, app.theme.overlay_bg, app.theme.fg_bright)
    } else {
        app.theme.overlay_bg
    };

    if left_w == 0 && right_w == 0 {
        return;
    }

    if total_needed as u16 >= area.width {
        // Only show left side when too narrow
        let left_text = format!(" {} ", left_str);
        f.render_widget(
            Paragraph::new(left_text)
                .style(Style::default().fg(left_fg).bg(left_bg)),
            area,
        );
        return;
    }

    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Length(left_w), Constraint::Min(0)])
        .split(area);

    if left_w > 0 {
        let left_text = format!(" {} ", left_str);
        f.render_widget(
            Paragraph::new(left_text)
                .style(Style::default().fg(left_fg).bg(left_bg)),
            chunks[0],
        );
    }

    if right_w > 0 {
        let right_text = format!(" {} ", right_str);
        f.render_widget(
            Paragraph::new(right_text)
                .style(Style::default().fg(app.theme.fg_bright).bg(app.theme.border)),
            chunks[1],
        );
    }
}

fn render_modules(modules: &[&FooterModule], app: &App) -> String {
    let mut parts = Vec::new();
    for m in modules {
        match m {
            FooterModule::Playback => parts.push(render_playback(app)),
            FooterModule::Title => parts.push(render_title(app)),
            FooterModule::Volume => parts.push(render_volume(app)),
            FooterModule::Repeat => parts.push(render_repeat(app)),
            FooterModule::Shuffle => parts.push(render_shuffle(app)),
            FooterModule::Progress => parts.push(render_progress(app)),
            FooterModule::Queue => parts.push(render_queue(app)),
            FooterModule::Clock => parts.push(render_clock()),
            FooterModule::KeyAction => parts.push(render_keyaction(app)),
            FooterModule::Backend => parts.push(render_backend()),
            FooterModule::System => parts.push(render_system()),
            FooterModule::Device => parts.push(render_device()),
        }
    }
    parts.iter().filter(|s| !s.is_empty()).fold(String::new(), |mut acc, s| {
        if !acc.is_empty() { acc.push(' '); }
        acc.push_str(s);
        acc
    })
}

fn render_playback(app: &App) -> String {
    match app.state.status {
        PlaybackStatus::Playing => "\u{25b6}".into(),
        PlaybackStatus::Paused => "\u{23f8}".into(),
        PlaybackStatus::Stopped => "\u{25a0}".into(),
    }
}

fn render_title(app: &App) -> String {
    app.state.current_track.as_ref().map_or_else(
        || String::new(),
        |t| {
            if t.artist.is_empty() {
                t.title.clone()
            } else {
                format!("{} \u{2013} {}", t.artist, t.title)
            }
        },
    )
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
    "rodio".into()
}

fn render_system() -> String {
    String::new()
}

fn render_device() -> String {
    String::new()
}
