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

use chrono::Local;
use gtm_core::state::{PlaybackStatus, RepeatMode};

use crate::app::App;
use crate::theme::{AppTheme, readable_fg};
use crate::ui::{Render, use_nerd_fonts};

/// Namespace for per-module footer rendering.
pub struct Footer;

/// Scrolling marquee for footer text: if it fits in `MAX` chars, return it
/// verbatim; otherwise cycle one full loop then hold still before repeating.
fn scroll_text(raw: String, scroll: usize) -> String {
    const MAX: usize = 30;
    const SPEED: usize = 6;
    const HOLD_STEPS: usize = 50; // ~5s hold at 60fps / SPEED
    let char_count = raw.chars().count();
    if char_count > MAX {
        // Char-based modulo so multibyte UTF-8 never splits mid-sequence.
        let chars: Vec<char> = raw.chars().collect();
        let step = scroll / SPEED;
        let pos = step % (char_count + HOLD_STEPS);
        let offset = if pos < char_count { pos } else { 0 };
        let s: String = chars.iter().cycle().skip(offset).take(MAX).collect();
        format!("{s} \u{2026}")
    } else {
        raw
    }
}

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
    Speed,
    Repeat,
    Shuffle,
    Progress,
    Queue,
    KeyAction,
    Backend,
    System,
    EqPreset,
    SleepTimer,
    LowPower,
    Device,
    Notification,
    Time,
    Multiselect,
    Download,
}

impl FooterModule {
    /// Stable display name; also used to parse user presets from TOML.
    pub fn as_str(self) -> &'static str {
        match self {
            FooterModule::Playback => "Playback",
            FooterModule::Title => "Title",
            FooterModule::Volume => "Volume",
            FooterModule::Speed => "Speed",
            FooterModule::Repeat => "Repeat",
            FooterModule::Shuffle => "Shuffle",
            FooterModule::Progress => "Progress",
            FooterModule::Queue => "Queue",
            FooterModule::KeyAction => "KeyAction",
            FooterModule::Backend => "Backend",
            FooterModule::System => "System",
            FooterModule::EqPreset => "EqPreset",
            FooterModule::SleepTimer => "SleepTimer",
            FooterModule::LowPower => "LowPower",
            FooterModule::Device => "Device",
            FooterModule::Notification => "Notification",
            FooterModule::Time => "Time",
            FooterModule::Multiselect => "Multiselect",
            FooterModule::Download => "Download",
        }
    }

    /// Parse a module name from a TOML footer preset file. Unknown names are
    /// dropped by the caller so typos don't break the whole preset.
    pub fn from_str_lossy(s: &str) -> Option<Self> {
        Some(match s.trim() {
            "Playback" => FooterModule::Playback,
            "Title" => FooterModule::Title,
            "Volume" => FooterModule::Volume,
            "Speed" => FooterModule::Speed,
            "Repeat" => FooterModule::Repeat,
            "Shuffle" => FooterModule::Shuffle,
            "Progress" => FooterModule::Progress,
            "Queue" => FooterModule::Queue,
            "KeyAction" => FooterModule::KeyAction,
            "Backend" => FooterModule::Backend,
            "System" => FooterModule::System,
            "EqPreset" => FooterModule::EqPreset,
            "SleepTimer" => FooterModule::SleepTimer,
            "LowPower" => FooterModule::LowPower,
            "Device" => FooterModule::Device,
            "Notification" => FooterModule::Notification,
            "Time" => FooterModule::Time,
            "Multiselect" => FooterModule::Multiselect,
            "Download" => FooterModule::Download,
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
                FooterModule::Repeat,
                FooterModule::Shuffle,
                FooterModule::Volume,
                FooterModule::Speed,
                FooterModule::LowPower,
                FooterModule::Device,
                FooterModule::EqPreset,
                FooterModule::KeyAction,
                FooterModule::Notification,
                FooterModule::SleepTimer,
                FooterModule::Download,
            ],
            right: vec![
                FooterModule::Queue,
                FooterModule::Time,
                FooterModule::System,
                FooterModule::Multiselect,
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
            right: vec![FooterModule::Time, FooterModule::System],
        },
        FooterPreset {
            name: Cow::Borrowed("Full"),
            left: vec![
                FooterModule::Playback,
                FooterModule::Title,
                FooterModule::Repeat,
                FooterModule::Shuffle,
                FooterModule::Volume,
                FooterModule::EqPreset,
                FooterModule::Progress,
                FooterModule::KeyAction,
                FooterModule::SleepTimer,
            ],
            right: vec![
                FooterModule::Queue,
                FooterModule::Time,
                FooterModule::System,
                FooterModule::Multiselect,
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
        let fg = readable_fg(app.theme.fg, bg);
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

    // Multiselect mode is on: pin a "SEL" marker at the very far-left so the
    // active selection mode is always visible, independent of the preset.
    if app.multiselect_mode {
        let bg = app.theme.accent;
        let span = Span::styled(
            " SEL ",
            Style::default()
                .fg(readable_fg(app.theme.fg, bg))
                .bg(bg)
                .add_modifier(Modifier::BOLD),
        );
        out_left.insert(
            0,
            FooterGroup {
                width: span.width() as u16,
                line: Line::from(span),
                bg,
            },
        );
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

impl Footer {
    fn multiselect(app: &App) -> Option<String> {
        if app.multiselect_mode && !app.selected_indices.is_empty() {
            Some(format!("[{} selected]", app.selected_indices.len()))
        } else {
            None
        }
    }

    fn footer_notification(app: &App) -> Option<String> {
        let (msg, expires) = app.footer_notification.as_ref()?;
        if std::time::Instant::now() >= *expires {
            return None;
        }
        Some(msg.clone())
    }

    fn download(app: &App) -> Option<String> {
        // Prefer the most recently updated in-flight download.
        let dl = app.downloads.values().max_by_key(|d| d.updated_at)?;
        if !matches!(dl.status.as_str(), "downloading" | "pending") {
            return None;
        }
        let icon = if use_nerd_fonts() {
            "\u{f019} " // nf-fa-download
        } else {
            "\u{2193} " // ↓
        };
        let mut out = format!("{icon}{:.0}%", dl.percent.clamp(0.0, 100.0));
        if let Some(rate) = dl.rate_bytes_per_sec {
            out.push_str(&format!(" {}", Footer::format_rate(rate)));
        }
        if let Some(eta) = dl.eta_secs {
            out.push_str(&format!(" ETA {}", format_duration(eta)));
        }
        Some(out)
    }

    fn format_rate(bytes_per_sec: f64) -> String {
        const KB: f64 = 1024.0;
        const MB: f64 = KB * 1024.0;
        const GB: f64 = MB * 1024.0;
        if bytes_per_sec >= GB {
            format!("{:.1}GiB/s", bytes_per_sec / GB)
        } else if bytes_per_sec >= MB {
            format!("{:.1}MiB/s", bytes_per_sec / MB)
        } else if bytes_per_sec >= KB {
            format!("{:.0}KiB/s", bytes_per_sec / KB)
        } else {
            format!("{:.0}B/s", bytes_per_sec)
        }
    }

    fn playback(app: &App) -> String {
        match app.state.status {
            PlaybackStatus::Playing => {
                if use_nerd_fonts() {
                    "\u{f040a}".into()
                } else {
                    "\u{25b6}".into()
                }
            }
            PlaybackStatus::Paused => {
                if use_nerd_fonts() {
                    "\u{f03e4}".into()
                } else {
                    "\u{23f8}".into()
                }
            }
            PlaybackStatus::Stopped => {
                if use_nerd_fonts() {
                    "\u{f04db}".into()
                } else {
                    "\u{25a0}".into()
                }
            }
        }
    }

    fn title(app: &App) -> Option<String> {
        // A live stream's ICY `StreamTitle` (when present) overrides the
        // track title, which for radio is just the station name.
        if let Some(live) = &app.state.radio_title
            && !live.is_empty()
        {
            return Some(scroll_text(live.clone(), app.footer_title_scroll));
        }
        let raw = app
            .state
            .current_track
            .as_ref()
            .map_or_else(String::new, |t| {
                // Show the track title with the artist concatenated after it.
                if t.artist.is_empty() {
                    t.title.clone()
                } else {
                    format!("{} \u{2013} {}", t.title, t.artist)
                }
            });
        if raw.is_empty() {
            return None;
        }
        Some(scroll_text(raw, app.footer_title_scroll))
    }

    fn volume(app: &App) -> String {
        if app.state.mute {
            "MUTE".into()
        } else {
            format!("{:>3}%", app.state.volume)
        }
    }

    fn speed(app: &App) -> Option<String> {
        let s = app.state.audio.speed;
        if (s - 1.0).abs() < f32::EPSILON {
            None
        } else {
            Some(format!("{s:.2}x"))
        }
    }

    fn low_power(app: &App) -> Option<String> {
        if app.state.low_power {
            Some("LowPower".into())
        } else {
            None
        }
    }

    fn device(app: &App) -> Option<String> {
        Some(
            app.state
                .audio
                .audio_device
                .clone()
                .unwrap_or_else(|| "Default".into()),
        )
    }

    fn repeat(app: &App) -> Option<String> {
        match app.state.repeat {
            RepeatMode::Off => None,
            RepeatMode::One => Some(if use_nerd_fonts() {
                "\u{f0458}".into()
            } else {
                "1".into()
            }),
            RepeatMode::All => Some(if use_nerd_fonts() {
                "\u{f0456}".into()
            } else {
                "A".into()
            }),
        }
    }

    fn shuffle(app: &App) -> Option<String> {
        if app.state.shuffle {
            Some(if use_nerd_fonts() {
                "\u{f049d}".into()
            } else {
                "S".into()
            })
        } else {
            None
        }
    }

    fn eq_preset(app: &App) -> Option<String> {
        if app.state.audio.eq_enabled {
            let icon = if use_nerd_fonts() {
                "\u{f062e} "
            } else {
                "EQ:"
            };
            Some(format!("{icon}{}", app.state.audio.eq_preset.label()))
        } else {
            None
        }
    }

    fn sleep_timer(app: &App) -> Option<String> {
        if let Some(secs) = app.state.sleep_timer {
            let m = secs / 60;
            let s = secs % 60;
            let icon = if use_nerd_fonts() {
                "\u{f04b2} "
            } else {
                "zzz "
            };
            Some(format!("{icon}{}:{:02}", m, s))
        } else {
            None
        }
    }

    fn progress(app: &App) -> Option<String> {
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
        let bar_w = 12;
        let progress = Render::progress_variant(ratio, bar_w, app);
        Some(format!("{} {}", progress, time_str))
    }

    fn queue(app: &App) -> Option<String> {
        let len = app.queue.cache.len();
        if len == 0 {
            return None;
        }
        let cursor = app.queue.cursor;
        Some(format!("{}/{}", cursor + 1, len))
    }

    fn keyaction(app: &App) -> Option<String> {
        if let Some((ref action, expires)) = app.last_action_name
            && std::time::Instant::now() < expires
        {
            return Some(format!("[{}]", action));
        }
        None
    }

    fn backend(app: &App) -> String {
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

    fn system(app: &App) -> String {
        let backend = Footer::backend(app);
        format!("{} {}", platform_icon(), backend)
    }
    /// Render the current wall-clock time using the user's strftime-style format.
    fn time(app: &App) -> Option<String> {
        if app.footer_time_format.is_empty() {
            return None;
        }
        Some(Local::now().format(&app.footer_time_format).to_string())
    }
}

// ─── Module dispatch ───────────────────────────────────────────────────

/// Per-module accent colour used as the group background (brand-badge style).
fn module_color(m: FooterModule, theme: &AppTheme) -> Color {
    match m {
        FooterModule::Playback => theme.accent,
        FooterModule::Title => theme.secondary_accent,
        FooterModule::Volume => theme.tertiary_accent,
        FooterModule::Speed => theme.secondary_accent,
        FooterModule::Repeat => theme.accent,
        FooterModule::Shuffle => theme.tertiary_accent,
        FooterModule::Progress => theme.secondary_accent,
        FooterModule::Queue => theme.accent,
        FooterModule::KeyAction => theme.tertiary_accent,
        FooterModule::Backend => theme.secondary_accent,
        FooterModule::System => theme.accent,
        FooterModule::EqPreset => theme.secondary_accent,
        FooterModule::SleepTimer => theme.accent,
        FooterModule::LowPower => theme.warning,
        FooterModule::Device => theme.secondary_accent,
        FooterModule::Notification => theme.fg_bright,
        FooterModule::Time => theme.tertiary_accent,
        FooterModule::Multiselect => theme.warning,
        FooterModule::Download => theme.secondary_accent,
    }
}

fn module_text(m: FooterModule, app: &App) -> Option<String> {
    match m {
        FooterModule::Playback => Some(Footer::playback(app)),
        FooterModule::Title => Footer::title(app),
        FooterModule::Volume => Some(Footer::volume(app)),
        FooterModule::Speed => Footer::speed(app),
        FooterModule::Repeat => Footer::repeat(app),
        FooterModule::Shuffle => Footer::shuffle(app),
        FooterModule::Progress => Footer::progress(app),
        FooterModule::Queue => Footer::queue(app),
        FooterModule::KeyAction => Footer::keyaction(app),
        FooterModule::Backend => Some(Footer::backend(app)),
        FooterModule::System => Some(Footer::system(app)),
        FooterModule::EqPreset => Footer::eq_preset(app),
        FooterModule::SleepTimer => Footer::sleep_timer(app),
        FooterModule::LowPower => Footer::low_power(app),
        FooterModule::Device => Footer::device(app),
        FooterModule::Notification => Footer::footer_notification(app),
        FooterModule::Time => Footer::time(app),
        FooterModule::Multiselect => Footer::multiselect(app),
        FooterModule::Download => Footer::download(app),
    }
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

/// Get platform mascot/icon for the current OS.
fn platform_icon() -> &'static str {
    // Use nerd font icons when available, fallback to emoji
    if use_nerd_fonts() {
        match std::env::consts::OS {
            "linux" => "\u{f17c}",   // Linux (Tux)
            "macos" => "\u{f302}",   // Apple
            "windows" => "\u{f87a}", // Windows
            _ => "?",
        }
    } else {
        match std::env::consts::OS {
            "linux" => "\u{1f427}",   // Penguin emoji
            "macos" => "\u{1f34e}",   // Apple emoji
            "windows" => "\u{1f5a5}", // Computer emoji
            _ => "?",
        }
    }
}

pub(crate) fn read_process_memory_kb() -> Option<u64> {
    let status = std::fs::read_to_string("/proc/self/status").ok()?;
    for line in status.lines() {
        if let Some(rest) = line.strip_prefix("VmRSS:") {
            let kb: u64 = rest.split_whitespace().next()?.parse().ok()?;
            return Some(kb);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::theme::assert_unique_names;

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
            FooterModule::EqPreset,
            FooterModule::SleepTimer,
            FooterModule::Notification,
            FooterModule::Download,
        ] {
            let s = m.as_str();
            assert_eq!(FooterModule::from_str_lossy(s), Some(m));
        }
        assert!(FooterModule::from_str_lossy("NoSuchModule").is_none());
    }

    #[test]
    fn presets_have_unique_names() {
        assert_unique_names(presets().iter().map(|p| p.name.as_ref()), "preset");
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
