// Copyright (c) 2026 - present
// Author: prjctimg <prjctimg@outlook.com>
// Footer bar: system info, keybindings, and playback status.
//
// This is free software released under the GPL-3.0 license.

use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;
use std::borrow::Cow;

use gtm_core::state::PlaybackStatus;

use crate::app::App;

/// Darken an RGB color by the given factor (0.0 = black, 1.0 = unchanged).
/// Used to create readable background colors from bright accent colors.
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

/// A footer layout preset. Unlike the previous a/b/c/x/y/z slots (which were
/// silently re-flattened into 3 groups), `left`/`middle`/`right` map directly
/// to the on-screen groups.
#[derive(Debug, Clone)]
pub struct FooterPreset {
    pub name: Cow<'static, str>,
    pub left: Vec<FooterModule>,
    pub middle: Vec<FooterModule>,
    pub right: Vec<FooterModule>,
}

/// Built-in presets inspired by lualine.nvim groups.
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
            ],
            middle: vec![
                FooterModule::KeyAction,
                FooterModule::Notification,
                FooterModule::SleepTimer,
            ],
            right: vec![],
        },
        // Bare minimum for termux or very small viewports.
        FooterPreset {
            name: Cow::Borrowed("Minimal"),
            left: vec![FooterModule::Playback, FooterModule::EqPreset],
            middle: vec![FooterModule::KeyAction, FooterModule::SleepTimer],
            right: vec![],
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
            ],
            middle: vec![FooterModule::KeyAction, FooterModule::SleepTimer],
            right: vec![],
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
    #[serde(default)]
    middle: Vec<String>,
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
            middle: parse_module_list(&p.middle),
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

/// The full output of a footer render: per-group lines plus the background
/// used for the unfilled trailing area on the right edge.
#[derive(Clone)]
pub struct FooterRenderOutput {
    pub groups: Vec<FooterGroup>,
    pub right_bg: Color,
}

/// Cached footer render used to suppress refresh during tab transitions.
#[derive(Default)]
pub struct FooterCache {
    pub last: Option<FooterRenderOutput>,
    pub suppress_refresh: bool,
}

/// Render the current footer preset into a list of group widgets plus the
/// trailing-area background. Returns `None` when every group would be empty
/// (e.g. no track loaded and no key action pending).
pub fn render(app: &App) -> Option<FooterRenderOutput> {
    let preset = app
        .footer_presets
        .get(app.footer_preset)
        .or_else(|| app.footer_presets.first())?;

    let is_playing = app.state.status == PlaybackStatus::Playing;

    // Each footer section gets a distinct background from the theme's accent
    // colors, darkened for readability.  Left = accent (playing) or
    // secondary_accent (paused); Middle = secondary_accent; Right = tertiary_accent.
    let left_bg = if is_playing {
        darken(app.theme.accent, 0.25)
    } else {
        darken(app.theme.secondary_accent, 0.25)
    };
    let middle_bg = darken(app.theme.secondary_accent, 0.20);
    let right_bg = darken(app.theme.accent, 0.20);

    let slots: [(&[FooterModule], Color); 3] = [
        (&preset.left, left_bg),
        (&preset.middle, middle_bg),
        (&preset.right, right_bg),
    ];

    let mut out_groups: Vec<FooterGroup> = Vec::new();
    for (modules, bg) in &slots {
        let parts = render_modules(modules, app);
        if parts.is_empty() {
            continue;
        }
        let mut spans: Vec<Span<'static>> = Vec::with_capacity(parts.len() * 2 + 2);
        spans.push(Span::raw(" "));
        let mut width: u32 = 1;
        for (j, (text, color)) in parts.iter().enumerate() {
            if j > 0 {
                spans.push(Span::raw(" "));
                width += 1;
            }
            let fg = crate::ui::readable_fg(*color, *bg);
            let span = Span::styled(text.clone(), Style::default().fg(fg));
            width += span.width() as u32;
            spans.push(span);
        }
        spans.push(Span::raw(" "));
        width += 1;
        out_groups.push(FooterGroup {
            line: Line::from(spans),
            bg: *bg,
            width: width as u16,
        });
    }

    if out_groups.is_empty() {
        return None;
    }
    Some(FooterRenderOutput {
        groups: out_groups,
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
/// Groups keep their preset order (left, middle, right): the first group hugs
/// the left edge, the last hugs the right edge, and any middle groups sit
/// centred in the remaining space.  The whole strip is painted with the
/// trailing background first so gaps between groups stay transparent-aware.
pub fn draw(f: &mut Frame, area: Rect, out: &FooterRenderOutput) {
    if out.groups.is_empty() || area.width == 0 {
        return;
    }
    let widths: Vec<u16> = out.groups.iter().map(|g| g.width).collect();
    let total: u16 = widths.iter().sum();
    if total == 0 {
        return;
    }

    f.render_widget(
        Paragraph::new("").style(Style::default().bg(out.right_bg)),
        area,
    );

    if total > area.width {
        let mut x = area.x;
        for (group, &w) in out.groups.iter().zip(&widths) {
            if x >= area.x + area.width {
                break;
            }
            let avail = area.x + area.width - x;
            f.render_widget(
                Paragraph::new(group.line.clone()).style(Style::default().bg(group.bg)),
                Rect {
                    x,
                    y: area.y,
                    width: w.min(avail),
                    height: area.height,
                },
            );
            x += w;
        }
        return;
    }

    let n = out.groups.len();
    let mut xs: Vec<u16> = Vec::with_capacity(n);
    let mut used = 0u16;
    for (i, group) in out.groups.iter().enumerate() {
        let x = if i == 0 {
            0
        } else if i == n - 1 {
            area.width - group.width
        } else {
            (area.width - group.width) / 2
        };
        let x = x.max(used);
        xs.push(x);
        used = x.saturating_add(group.width);
    }

    for (i, group) in out.groups.iter().enumerate() {
        f.render_widget(
            Paragraph::new(group.line.clone()).style(Style::default().bg(group.bg)),
            Rect {
                x: area.x + xs[i],
                y: area.y,
                width: group.width,
                height: area.height,
            },
        );
    }
}

// ─── Module dispatch ───────────────────────────────────────────────────

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

fn render_modules(modules: &[FooterModule], app: &App) -> Vec<(String, Color)> {
    let mut parts = Vec::new();
    for m in modules {
        if let Some(text) = module_text(*m, app) {
            if !text.is_empty() {
                parts.push((text, module_color(*m, &app.theme)));
            }
        }
    }
    parts
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
        FooterModule::Backend => Some(render_backend()),
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
    let bar_w = 14usize;
    let progress = crate::ui::render_progress_variant(ratio, bar_w, app);
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
    if let Some((ref action, expires)) = app.last_action_name {
        if std::time::Instant::now() < expires {
            return Some(format!("[{}]", action));
        }
    }
    None
}

fn render_backend() -> String {
    let rust_ver = option_env!("VERGEN_RUSTC_SEMVER").unwrap_or("?");
    let crate_ver = option_env!("CARGO_PKG_VERSION").unwrap_or("?");
    format!("rust {} \u{2022} v{}", rust_ver, crate_ver)
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
        ] {
            let s = m.as_str();
            assert_eq!(FooterModule::from_str_lossy(s), Some(m));
        }
        assert!(FooterModule::from_str_lossy("NoSuchModule").is_none());
    }

    #[test]
    fn presets_have_unique_names() {
        let all = presets();
        let names: Vec<&str> = all.iter().map(|p| p.name.as_ref()).collect();
        let mut sorted = names.clone();
        sorted.sort();
        sorted.dedup();
        assert_eq!(sorted.len(), names.len());
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

    #[test]
    fn render_title_handles_multibyte_without_panic() {
        // Construct an App-like minimal context isn't trivial, so just verify
        // the char-based scroll math: a 40-char title scrolls at offsets that
        // are valid character boundaries.
        let raw = "ＡＢＣＤＥＦＧＨＩＪＫＬＭＮＯＰＱＲＳＴＵＶＷＸＹＺ"; // 26 fullwidth chars
        let chars: Vec<char> = raw.chars().collect();
        let n = chars.len();
        let max = 10;
        let mut produced = Vec::new();
        for offset in 0..n {
            let s: String = chars.iter().cycle().skip(offset).take(max).collect();
            produced.push(s);
        }
        assert_eq!(produced.len(), n);
        // Every produced string is exactly `max` chars wide (fullwidth chars
        // advance one usize each in `chars().take`).
        for s in &produced {
            assert_eq!(s.chars().count(), max);
        }
    }
}
