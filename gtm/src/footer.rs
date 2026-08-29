// Copyright (c) 2026
// Author: prjctimg <prjctimg@outlook.com>
// Footer bar: modular keybinding/system/playback status display.
//
// This is free software released under the GPL-3.0 license.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use std::borrow::Cow;

use gtm_core::state::PlaybackStatus;

use crate::app::App;

fn darken(c: Color, factor: f64) -> Color {
    match c {
        Color::Rgb(r, g, b) => Color::Rgb(
            (r as f64 * factor) as u8,
            (g as f64 * factor) as u8,
            (b as f64 * factor) as u8,
        ),
        _ => c,
    }
}

fn bg_luminance(c: Color) -> f64 {
    match c {
        Color::Rgb(r, g, b) => 0.299 * r as f64 + 0.587 * g as f64 + 0.114 * b as f64,
        _ => 128.0,
    }
}

/// A footer module. Each module renders a small run of text with its own
/// accent-coloured background (brand-badge style).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FooterModule {
    Playback,
    Title,
    Volume,
    Repeat,
    Shuffle,
    Progress,
    Queue,
    KeyAction,
    Backend,
    System,
    Device,
    EqPreset,
    SleepTimer,
    Notification,
}

impl FooterModule {
    /// Stable display name; also used to parse user presets from TOML.
    pub fn as_str(self) -> &'static str {
        match self {
            FooterModule::Playback => "Playback",
            FooterModule::Title => "Title",
            FooterModule::Volume => "Volume",
            FooterModule::Repeat => "Repeat",
            FooterModule::Shuffle => "Shuffle",
            FooterModule::Progress => "Progress",
            FooterModule::Queue => "Queue",
            FooterModule::KeyAction => "KeyAction",
            FooterModule::Backend => "Backend",
            FooterModule::System => "System",
            FooterModule::Device => "Device",
            FooterModule::EqPreset => "EqPreset",
            FooterModule::SleepTimer => "SleepTimer",
            FooterModule::Notification => "Notification",
        }
    }

    /// Parse a module name from a TOML footer preset file. Unknown names are
    /// dropped by the caller so typos don't break the whole preset.
    pub fn from_str_lossy(s: &str) -> Option<Self> {
        Some(match s.trim() {
            "Playback" => FooterModule::Playback,
            "Title" => FooterModule::Title,
            "Volume" => FooterModule::Volume,
            "Repeat" => FooterModule::Repeat,
            "Shuffle" => FooterModule::Shuffle,
            "Progress" => FooterModule::Progress,
            "Queue" => FooterModule::Queue,
            "KeyAction" => FooterModule::KeyAction,
            "Backend" => FooterModule::Backend,
            "System" => FooterModule::System,
            "Device" => FooterModule::Device,
            "EqPreset" => FooterModule::EqPreset,
            "SleepTimer" => FooterModule::SleepTimer,
            "Notification" => FooterModule::Notification,
            _ => return None,
        })
    }
}

/// A footer layout preset. `left` modules hug the left edge and `right`
/// modules hug the right edge (lualine-style `a,b,c` / `x,y,z` layout with no
/// centred middle group).
#[derive(Debug, Clone)]
pub struct FooterPreset {
    pub name: Cow<'static, str>,
    pub left: Vec<FooterModule>,
    pub right: Vec<FooterModule>,
}

/// Built-in presets, intended to mirror the left/right/middle layout
/// (historically mirrored the archived gtm.nim status-bar ordering where
/// playback/queue sit at an extreme end and system/device info at the other).
pub fn presets() -> Vec<FooterPreset> {
    vec![
        FooterPreset {
            name: Cow::Borrowed("Default"),
            left: vec![
                FooterModule::Playback,
                FooterModule::Queue,
                FooterModule::Repeat,
                FooterModule::Shuffle,
                FooterModule::Volume,
                FooterModule::EqPreset,
                FooterModule::KeyAction,
                FooterModule::Notification,
                FooterModule::SleepTimer,
            ],
            right: vec![
                FooterModule::Device,
                FooterModule::System,
                FooterModule::Backend,
            ],
        },
        // Bare minimum for termux or very small viewports.
        FooterPreset {
            name: Cow::Borrowed("Minimal"),
            left: vec![
                FooterModule::Playback,
                FooterModule::Volume,
                FooterModule::KeyAction,
                FooterModule::SleepTimer,
            ],
            right: vec![FooterModule::Backend],
        },
        FooterPreset {
            name: Cow::Borrowed("Full"),
            left: vec![
                FooterModule::Playback,
                FooterModule::Title,
                FooterModule::Volume,
                FooterModule::Repeat,
                FooterModule::Shuffle,
                FooterModule::EqPreset,
                FooterModule::Progress,
                FooterModule::KeyAction,
                FooterModule::SleepTimer,
            ],
            right: vec![
                FooterModule::Queue,
                FooterModule::Device,
                FooterModule::System,
                FooterModule::Backend,
            ],
        },
    ]
}

// ─── User presets (TOML) ──────────────────────────────────────────────

pub fn user_presets_path() -> std::path::PathBuf {
    let config = std::env::var("XDG_CONFIG_HOME")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| {
            let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".into());
            std::path::PathBuf::from(home).join(".config")
        });
    config.join("gtm").join("footer.toml")
}

#[derive(serde::Deserialize, Default)]
struct UserPresetsFile {
    #[serde(default)]
    preset: Vec<UserPreset>,
}

#[derive(serde::Deserialize, Default)]
struct UserPreset {
    name: String,
    #[serde(default)]
    left: Vec<String>,
    #[serde(default, alias = "middle")]
    _legacy_middle: Vec<String>,
    #[serde(default)]
    right: Vec<String>,
}

fn parse_module_list(names: &[String]) -> Vec<FooterModule> {
    names
        .iter()
        .filter_map(|s| FooterModule::from_str_lossy(s))
        .collect()
}

/// Load user-defined presets from `~/.config/gtm/footer.toml`. Unparseable
/// files are skipped so a malformed file never breaks the TUI.
pub fn load_user_presets() -> Vec<FooterPreset> {
    let Ok(text) = std::fs::read_to_string(user_presets_path()) else {
        return Vec::new();
    };
    let Ok(parsed) = toml::from_str::<UserPresetsFile>(&text) else {
        return Vec::new();
    };
    parsed
        .preset
        .into_iter()
        .map(|p| FooterPreset {
            name: Cow::Owned(p.name),
            left: parse_module_list(&p.left),
            right: parse_module_list(&p.right),
        })
        .collect()
}

/// Built-in presets followed by user presets; user presets replace built-ins
/// on name collision.
pub fn merged_presets() -> Vec<FooterPreset> {
    let mut v = presets();
    for up in load_user_presets() {
        if let Some(existing) = v.iter_mut().find(|p| p.name == up.name) {
            *existing = up;
        } else {
            v.push(up);
        }
    }
    v
}

// ─── Rendering ────────────────────────────────────────────────────────

/// One rendered footer group: a styled line, its background, and its width
/// in terminal cells.
#[derive(Clone)]
pub struct FooterGroup {
    pub line: Line<'static>,
    pub bg: Color,
    pub width: u16,
}

/// The full output of a footer render: left and right module groups plus the
/// background used for the unfilled trailing area on the right edge.
#[derive(Clone)]
pub struct FooterRenderOutput {
    pub left: Vec<FooterGroup>,
    pub right: Vec<FooterGroup>,
    pub right_bg: Color,
}

/// Cached footer render used to suppress refresh during tab transitions.
#[derive(Default)]
pub struct FooterCache {
    pub last: Option<FooterRenderOutput>,
    pub suppress_refresh: bool,
}

/// Render the current footer preset into left/right module groups plus the
/// trailing-area background. Returns `None` when every module would be empty
/// (e.g. no track loaded and no key action pending).
pub fn render(app: &App) -> Option<FooterRenderOutput> {
    let preset = app
        .footer_presets
        .get(app.footer_preset)
        .or_else(|| app.footer_presets.first())?;

    let mut out_left: Vec<FooterGroup> = Vec::new();
    let mut out_right: Vec<FooterGroup> = Vec::new();
    for (is_left, m) in preset
        .left
        .iter()
        .map(|m| (true, *m))
        .chain(preset.right.iter().map(|m| (false, *m)))
    {
        let Some(text) = module_text(m, app) else {
            continue;
        };
        if text.is_empty() {
            continue;
        }
        // Brand-badge styling: a solid per-module accent background with the
        // readable foreground and a bold weight, exactly like the "gtm" badge.
        let bg = module_color(m, &app.theme);
        let fg = crate::theme::readable_fg(app.theme.fg, bg);
        let span = Span::styled(
            format!(" {} ", text),
            Style::default().fg(fg).add_modifier(Modifier::BOLD),
        );
        let group = FooterGroup {
            width: span.width() as u16,
            line: Line::from(span),
            bg,
        };
        if is_left {
            out_left.push(group);
        } else {
            out_right.push(group);
        }
    }

    if out_left.is_empty() && out_right.is_empty() {
        return None;
    }
    Some(FooterRenderOutput {
        left: out_left,
        right: out_right,
        right_bg: if app.transparent_bg {
            Color::Reset
        } else if bg_luminance(app.theme.bg) > 180.0 {
            darken(app.theme.bg, 0.85)
        } else {
            app.theme.border
        },
    })
}

/// Draw a previously-computed [`FooterRenderOutput`] into `area`.
///
/// Left groups are laid out left-to-right hugging the left edge (`a,b,c`) and
/// right groups are laid out right-to-left hugging the right edge (`x,y,z`),
/// following the lualine approach with no centred middle group. The whole
/// strip is painted with the trailing background first so gaps between groups
/// stay transparent-aware.
pub fn draw(f: &mut Frame, area: Rect, out: &FooterRenderOutput) {
    if area.width == 0 {
        return;
    }
    let left_w: u16 = out.left.iter().map(|g| g.width).sum();
    let right_w: u16 = out.right.iter().map(|g| g.width).sum();
    if left_w + right_w == 0 {
        return;
    }

    f.render_widget(
        Paragraph::new("").style(Style::default().bg(out.right_bg)),
        area,
    );

    let render_at = |f: &mut Frame, group: &FooterGroup, x: u16, w: u16| {
        f.render_widget(
            Paragraph::new(group.line.clone()).style(Style::default().bg(group.bg)),
            Rect {
                x,
                y: area.y,
                width: w,
                height: area.height,
            },
        );
    };

    if left_w + right_w > area.width {
        // Not enough room: render left groups first, then as many right groups
        // as fit, truncating overflow at the edges.
        let mut x = area.x;
        for group in &out.left {
            if x >= area.x + area.width {
                break;
            }
            let avail = area.x + area.width - x;
            render_at(f, group, x, group.width.min(avail));
            x += group.width;
            if x >= area.x + area.width {
                break;
            }
        }
        let mut x = area.x + area.width;
        for group in out.right.iter().rev() {
            if x <= area.x {
                break;
            }
            let avail = x - area.x;
            let w = group.width.min(avail);
            render_at(f, group, x - w, w);
            x -= group.width;
            if x <= area.x {
                break;
            }
        }
        return;
    }

    // Fit: left groups hug the left edge, right groups hug the right edge.
    let mut x = area.x;
    for group in &out.left {
        render_at(f, group, x, group.width);
        x += group.width;
    }
    let mut x = area.x + area.width;
    for group in out.right.iter().rev() {
        let w = group.width;
        render_at(f, group, x - w, w);
        x -= w;
    }
}

// ─── Module dispatch ───────────────────────────────────────────────────

/// Per-module accent colour used as the group background (brand-badge style).
fn module_color(m: FooterModule, theme: &crate::theme::AppTheme) -> Color {
    match m {
        FooterModule::Playback => theme.accent,
        FooterModule::Title => theme.secondary_accent,
        FooterModule::Volume => theme.tertiary_accent,
        FooterModule::Repeat => theme.accent,
        FooterModule::Shuffle => theme.tertiary_accent,
        FooterModule::Progress => theme.secondary_accent,
        FooterModule::Queue => theme.accent,
        FooterModule::KeyAction => theme.tertiary_accent,
        FooterModule::Backend => theme.secondary_accent,
        FooterModule::System => theme.accent,
        FooterModule::Device => theme.tertiary_accent,
        FooterModule::EqPreset => theme.secondary_accent,
        FooterModule::SleepTimer => theme.accent,
        FooterModule::Notification => theme.fg_bright,
    }
}

fn module_text(m: FooterModule, app: &App) -> Option<String> {
    match m {
        FooterModule::Playback => Some(render_playback(app)),
        FooterModule::Title => render_title(app),
        FooterModule::Volume => Some(render_volume(app)),
        FooterModule::Repeat => render_repeat(app),
        FooterModule::Shuffle => render_shuffle(app),
        FooterModule::Progress => render_progress(app),
        FooterModule::Queue => render_queue(app),
        FooterModule::KeyAction => render_keyaction(app),
        FooterModule::Backend => Some(render_backend(app)),
        FooterModule::System => Some(render_system()),
        FooterModule::Device => render_device(app),
        FooterModule::EqPreset => render_eq_preset(app),
        FooterModule::SleepTimer => render_sleep_timer(app),
        FooterModule::Notification => render_footer_notification(app),
    }
}

fn render_footer_notification(app: &App) -> Option<String> {
    let (msg, expires) = app.footer_notification.as_ref()?;
    if std::time::Instant::now() >= *expires {
        return None;
    }
    Some(msg.clone())
}

fn render_playback(app: &App) -> String {
    match app.state.status {
        PlaybackStatus::Playing => "\u{25b6}".into(),
        PlaybackStatus::Paused => "\u{23f8}".into(),
        PlaybackStatus::Stopped => "\u{25a0}".into(),
    }
}

fn render_title(app: &App) -> Option<String> {
    let raw = app
        .state
        .current_track
        .as_ref()
        .map_or_else(String::new, |t| {
            if t.artist.is_empty() {
                t.title.clone()
            } else {
                format!("{} \u{2013} {}", t.artist, t.title)
            }
        });
    if raw.is_empty() {
        return None;
    }
    const MAX: usize = 30;
    let char_count = raw.chars().count();
    if char_count > MAX {
        // Char-based modulo so multibyte UTF-8 never splits mid-sequence.
        let chars: Vec<char> = raw.chars().collect();
        let offset = app.footer_title_scroll % char_count;
        let s: String = chars.iter().cycle().skip(offset).take(MAX).collect();
        Some(format!("{} \u{2026}", s))
    } else {
        Some(raw)
    }
}

fn render_volume(app: &App) -> String {
    if app.state.mute {
        "MUTE".into()
    } else {
        format!("{:>3}%", app.state.volume)
    }
}

fn render_repeat(app: &App) -> Option<String> {
    match app.state.repeat {
        gtm_core::state::RepeatMode::Off => None,
        gtm_core::state::RepeatMode::One => Some("1".into()),
        gtm_core::state::RepeatMode::All => Some("A".into()),
    }
}

fn render_shuffle(app: &App) -> Option<String> {
    if app.state.shuffle {
        Some("S".into())
    } else {
        None
    }
}

fn render_eq_preset(app: &App) -> Option<String> {
    if app.state.eq_enabled {
        Some(format!("EQ:{}", app.state.eq_preset.label()))
    } else {
        None
    }
}

fn render_sleep_timer(app: &App) -> Option<String> {
    if let Some(secs) = app.state.sleep_timer {
        let m = secs / 60;
        let s = secs % 60;
        Some(format!("zzz {}:{:02}", m, s))
    } else {
        None
    }
}

fn render_progress(app: &App) -> Option<String> {
    let track = app.state.current_track.as_ref()?;
    let pos = app.display_position as u64;
    let dur = if app.state.duration > 0.0 {
        app.state.duration as u64
    } else {
        track.duration as u64
    };
    if dur == 0 {
        return None;
    }
    let ratio = (pos as f64 / dur as f64).clamp(0.0, 1.0);
    let time_str = format!("{} / {}", format_duration(pos), format_duration(dur));
    let bar_w = ((app.terminal_cols as usize) / 4).clamp(10, 30);
    let progress = crate::ui::Render::progress_variant(ratio, bar_w, app);
    Some(format!("{} {}", progress, time_str))
}

pub fn format_duration(secs: u64) -> String {
    let h = secs / 3600;
    let m = (secs % 3600) / 60;
    let s = secs % 60;
    if h > 0 {
        format!("{}:{:02}:{:02}", h, m, s)
    } else {
        format!("{}:{:02}", m, s)
    }
}

pub fn format_uptime(secs: f64) -> String {
    let total = secs as u64;
    let d = total / 86400;
    let h = (total % 86400) / 3600;
    let m = (total % 3600) / 60;
    let s = total % 60;
    if d > 0 {
        format!("{}d {}h {}m {}s", d, h, m, s)
    } else if h > 0 {
        format!("{}h {}m {}s", h, m, s)
    } else if m > 0 {
        format!("{}m {}s", m, s)
    } else {
        format!("{}s", s)
    }
}

fn render_queue(app: &App) -> Option<String> {
    let len = app.queue_cache.len();
    if len == 0 {
        return None;
    }
    let cursor = app.queue_cursor;
    Some(format!("{}/{}", cursor + 1, len))
}

fn render_keyaction(app: &App) -> Option<String> {
    if let Some((ref action, expires)) = app.last_action_name
        && std::time::Instant::now() < expires
    {
        return Some(format!("[{}]", action));
    }
    None
}

fn render_backend(app: &App) -> String {
    let name = app
        .health_report
        .as_ref()
        .and_then(|h| {
            h.components
                .iter()
                .find(|c| c.name == "audio_backend")
                .and_then(|c| c.message.as_deref())
        })
        .unwrap_or("unknown");
    name.to_string()
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
            format!(
                "{} {} \u{2022} {}GB {}CPU",
                os,
                arch,
                kb / (1024 * 1024),
                cpus
            )
        } else {
            format!("{} {} \u{2022} {}MB {}CPU", os, arch, kb / 1024, cpus)
        }
    } else {
        format!("{} {} \u{2022} {}CPU", os, arch, cpus)
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

fn render_device(app: &App) -> Option<String> {
    let track_count = app.tracks_cache.len();
    if track_count == 0 {
        return None;
    }
    let total_dur: u64 = app.tracks_cache.iter().map(|t| t.duration as u64).sum();
    let hours = total_dur / 3600;
    let mins = (total_dur % 3600) / 60;
    Some(format!(
        "{} tracks \u{2022} {}h{}m",
        track_count, hours, mins
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_duration_hours() {
        assert_eq!(format_duration(3661), "1:01:01");
    }

    #[test]
    fn format_duration_minutes() {
        assert_eq!(format_duration(125), "2:05");
    }

    #[test]
    fn module_round_trip() {
        for m in [
            FooterModule::Playback,
            FooterModule::Title,
            FooterModule::Volume,
            FooterModule::Repeat,
            FooterModule::Shuffle,
            FooterModule::Progress,
            FooterModule::Queue,
            FooterModule::KeyAction,
            FooterModule::Backend,
            FooterModule::System,
            FooterModule::Device,
            FooterModule::EqPreset,
            FooterModule::SleepTimer,
            FooterModule::Notification,
        ] {
            let s = m.as_str();
            assert_eq!(FooterModule::from_str_lossy(s), Some(m));
        }
        assert!(FooterModule::from_str_lossy("NoSuchModule").is_none());
    }

    #[test]
    fn presets_have_unique_names() {
        crate::theme::assert_unique_names(
            presets().iter().map(|p| p.name.as_ref()),
            "preset",
        );
    }

    #[test]
    fn parse_module_list_drops_unknowns() {
        let names = vec!["Playback".into(), "Bogus".into(), "Volume".into()];
        let parsed = parse_module_list(&names);
        assert_eq!(parsed, vec![FooterModule::Playback, FooterModule::Volume]);
    }

    #[test]
    fn user_presets_round_trip() {
        let toml_text = r#"
            [[preset]]
            name = "Custom"
            left = ["Playback", "Queue"]
            middle = ["KeyAction"]
            right = ["Volume"]
        "#;
        let parsed: UserPresetsFile = toml::from_str(toml_text).unwrap();
        assert_eq!(parsed.preset.len(), 1);
        let preset = &parsed.preset[0];
        assert_eq!(preset.name, "Custom");
        assert_eq!(preset.left, vec!["Playback", "Queue"]);
        let built = parse_module_list(&preset.left);
        assert_eq!(built, vec![FooterModule::Playback, FooterModule::Queue]);
    }
}
