// Copyright (c) 2026
// Author: prjctimg <prjctimg@outlook.com>
// Shared layout, colour and scrolling primitives
//
//
// This is free software released under the GPL-3.0 license.

use crate::ui::*;

pub(crate) fn cubic_ease_out(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    1.0 - (1.0 - t).powi(3)
}

pub(crate) fn cubic_ease_in(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    t * t * t
}

pub(crate) fn dim_color(c: Color) -> Color {
    match c {
        Color::Rgb(r, g, b) => Color::Rgb(
            (r as f32 * 0.7) as u8,
            (g as f32 * 0.7) as u8,
            (b as f32 * 0.7) as u8,
        ),
        other => other,
    }
}

pub(crate) fn dim_background(f: &mut ratatui::Frame, area: Rect) {
    for y in area.top()..area.bottom() {
        for x in area.left()..area.right() {
            if let Some(cell) = f.buffer_mut().cell_mut((x, y)) {
                let fg = dim_color(cell.fg);
                let bg = dim_color(cell.bg);
                cell.set_fg(fg);
                cell.set_bg(bg);
            }
        }
    }
}

pub(crate) const NOTIFICATION_LIFETIME: std::time::Duration =
    std::time::Duration::from_millis(1500);

pub(crate) const NOTIFICATION_EXIT_DURATION: std::time::Duration =
    std::time::Duration::from_millis(300);

pub(crate) fn wrap_text(text: &str, max_chars: usize) -> Vec<String> {
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

pub const COVER_W: u16 = 24;

pub const COVER_H: u16 = 12;

pub(crate) fn row_pad(content: &str, width: u16) -> usize {
    (width as usize).saturating_sub(content.chars().count())
}

pub(crate) fn empty_hint_lines(app: &App, headline: &str, hint: &str) -> Vec<Line<'static>> {
    let dim = Style::default().fg(app.theme.fg_dim);
    vec![
        Line::from(Span::styled(format!(" {headline}"), dim)),
        Line::from(Span::styled(format!(" {hint}"), dim)),
    ]
}

pub(crate) fn cursor_span_style(app: &App) -> Option<Style> {
    let phase = (app.frame_count % 64) as f32 / 64.0;
    let t = (1.0 - (phase * std::f32::consts::TAU).cos()) * 0.5;
    let bg = blend_colors(app.theme.selection_bg, app.float_bg(), (t * 0.85) as f64);
    Some(
        Style::default()
            .fg(app.theme.selection_fg_readable())
            .bg(bg),
    )
}

/// The rendered rows of a main-pane list, mapped back to the indexes its
/// cursor uses.
///
/// A list can draw fixed rows above its scrollable items — the Spotify
/// playlist view puts Play All and Shuffle there — and those rows belong to
/// the same cursor space even though they never scroll. Deriving click
/// targets from `scroll + screen row` is wrong as soon as a prefix exists:
/// every click lands one or two tracks too far down, and the last rows have
/// no zone at all. Both the renderer and the mouse map therefore build this
/// one description of what is on screen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ListRows {
    /// Fixed rows above the scrolling items, counted in cursor space.
    pub(crate) action_rows: usize,
    /// First scrolling item's absolute index.
    pub(crate) scroll: usize,
    /// Number of scrolling items on screen.
    pub(crate) len: usize,
}

impl ListRows {
    pub(crate) fn new(action_rows: usize, scroll: usize, len: usize) -> Self {
        Self {
            action_rows,
            scroll,
            len,
        }
    }

    /// `(screen y, absolute item index)` for every row that fits.
    ///
    /// `first_y` is the first row below the list's leading blank line and
    /// `capacity` is how many rows that pane can draw.
    pub(crate) fn entries(self, first_y: u16, capacity: usize) -> Vec<(u16, usize)> {
        let actions = self.action_rows.min(capacity);
        let tracks = self.len.min(capacity.saturating_sub(actions));
        let mut rows = Vec::with_capacity(actions + tracks);
        for offset in 0..actions {
            rows.push((first_y.saturating_add(offset as u16), offset));
        }
        for offset in 0..tracks {
            rows.push((
                first_y.saturating_add((actions + offset) as u16),
                self.action_rows + self.scroll + offset,
            ));
        }
        rows
    }
}

pub(crate) fn step_viewport(
    offset: usize,
    sel: usize,
    visible: usize,
    total: usize,
) -> (usize, usize) {
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

pub(crate) fn fill_pane(f: &mut ratatui::Frame, area: Rect, app: &App) {
    f.render_widget(
        ratatui::widgets::Block::default()
            .style(ratatui::style::Style::default().bg(app.pane_surface_bg())),
        area,
    );
}

pub(crate) const LOADER_BRAILLE: [&str; 10] = [
    "\u{280b}", "\u{2819}", "\u{2839}", "\u{2838}", "\u{283c}", "\u{2834}", "\u{2826}", "\u{2827}",
    "\u{2807}", "\u{280f}",
];

pub(crate) fn opencode_spinner(frame: usize) -> &'static str {
    LOADER_BRAILLE[(frame / 2) % LOADER_BRAILLE.len()]
}

pub(crate) const SCROLL_HOLD_FRAMES: usize = 300;
/// Frames per scroll step; larger = slower animation.
pub(crate) const SCROLL_SPEED: usize = 6;

pub(crate) fn scroll_text(text: &str, max_width: usize, frame: usize, is_selected: bool) -> String {
    if text.chars().count() <= max_width {
        return format!("{:<width$}", text, width = max_width);
    }
    if !is_selected {
        let truncated: String = text.chars().take(max_width.saturating_sub(1)).collect();
        return format!("{}…", truncated);
    }
    // Cycle the marquee through n offsets, then hold still for
    // `SCROLL_HOLD_FRAMES / SCROLL_SPEED` steps before looping again.
    let chars: Vec<char> = text.chars().collect();
    let n = chars.len();
    let hold_steps = SCROLL_HOLD_FRAMES / SCROLL_SPEED;
    let step = frame / SCROLL_SPEED;
    let pos = step % (n + hold_steps).max(1);
    let scroll = if pos < n { pos } else { 0 };
    let scrolled: String = chars
        .iter()
        .skip(scroll)
        .chain(chars.iter().take(scroll))
        .collect();
    scrolled.chars().take(max_width).collect()
}

#[cfg(test)]
mod tests {
    use super::{ListRows, scroll_text};

    /// Fixed action rows map to their own indexes, and the scrolling rows
    /// start after them instead of being shifted down by the arithmetic.
    #[test]
    fn list_rows_map_actions_and_scrolled_tracks() {
        let rows = ListRows::new(2, 5, 3).entries(10, 20);
        assert_eq!(rows, vec![(10, 0), (11, 1), (12, 7), (13, 8), (14, 9)]);
    }

    /// A short pane clips tracks but never the actions, which are the only
    /// way to start a playlist that cannot fit its first page.
    #[test]
    fn list_rows_keep_actions_when_capacity_is_tight() {
        let rows = ListRows::new(2, 0, 5).entries(3, 3);
        assert_eq!(rows, vec![(3, 0), (4, 1), (5, 2)]);
    }

    /// Ordinary lists have no fixed prefix and still start at the scroll
    /// offset.
    #[test]
    fn list_rows_without_actions_start_at_scroll() {
        let rows = ListRows::new(0, 8, 2).entries(4, 10);
        assert_eq!(rows, vec![(4, 8), (5, 9)]);
    }

    #[test]
    fn scroll_multibyte() {
        let text = "Artist \u{2014} T\u{e9}t\u{e9} Song Title That Is Quite Long";
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
    fn scroll_pads_fits() {
        assert_eq!(scroll_text("ab", 4, 0, true), "ab  ");
    }

    #[test]
    fn scroll_empty_panics() {
        let out = scroll_text("", 0, 1, true);
        assert_eq!(out, "");
    }
}
