// Copyright (c) 2026
// Author: prjctimg <prjctimg@outlook.com>
// Footer bar: keybinding hint display.
//
// This is free software released under the GPL-3.0 license.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

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

#[derive(Clone)]
pub struct FooterGroup {
    pub line: Line<'static>,
    pub bg: Color,
    pub width: u16,
}

#[derive(Clone)]
pub struct FooterRenderOutput {
    pub groups: Vec<FooterGroup>,
    pub right_bg: Color,
}

#[derive(Default)]
pub struct FooterCache {
    pub last: Option<FooterRenderOutput>,
    pub suppress_refresh: bool,
}

pub fn render(app: &App) -> Option<FooterRenderOutput> {
    let right_bg = if app.transparent_bg {
        Color::Reset
    } else if bg_luminance(app.theme.bg) > 180.0 {
        darken(app.theme.bg, 0.85)
    } else {
        app.theme.border
    };

    let mut out_groups: Vec<FooterGroup> = Vec::new();

    if let Some(text) = render_keyaction(app) {
        let bg = darken(app.theme.tertiary_accent, 0.20);
        let fg = crate::ui::readable_fg(app.theme.tertiary_accent, bg);
        let span = Span::styled(format!(" {} ", text), Style::default().fg(fg));
        let width = span.width() as u16 + 2;
        out_groups.push(FooterGroup {
            line: Line::from(span),
            bg,
            width,
        });
    }

    if out_groups.is_empty() {
        return None;
    }
    Some(FooterRenderOutput {
        groups: out_groups,
        right_bg,
    })
}

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

fn render_keyaction(app: &App) -> Option<String> {
    if let Some((ref action, expires)) = app.last_action_name
        && std::time::Instant::now() < expires
    {
        return Some(format!("[{}]", action));
    }
    None
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
}
