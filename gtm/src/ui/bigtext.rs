// Copyright (c) 2026
// Author: prjctimg <prjctimg@outlook.com>
// Oversized type: the first-run onboarding view
//
// This is free software released under the GPL-3.0 license.

//! The one view that is about the program rather than the music.
//!
//! The name is drawn in glyphs large enough to be the subject of the screen,
//! with the full title set beside it in ordinary type so the wordmark is never
//! asked to carry the meaning on its own.

use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Widget};
use tui_big_text::{BigText, PixelSize};

use crate::theme::{AppTheme, blend_colors};

/// The program's name, drawn oversized.
const WORDMARK: &str = "gtm";

/// What the program is, set beside the name. Read rather than recognised, which
/// is why it is body text and not more glyphs.
const TITLE: &str = "terminal audio player";

/// Where the name points.
const SITE: &str = "gtmd.dev";

/// Cells one glyph occupies at `size`, as (width, height).
///
/// `Full` gives every font pixel a whole cell, so an 8x8 glyph is 8 columns by
/// 8 rows. A cell is about twice as tall as it is wide, which is what makes
/// `Full` read as an oversized wordmark rather than as a stretched bitmap.
fn glyph_box(size: PixelSize) -> (u16, u16) {
    match size {
        PixelSize::Full => (8, 8),
        PixelSize::HalfHeight => (8, 4),
        PixelSize::HalfWidth => (4, 8),
        PixelSize::Quadrant => (4, 4),
        PixelSize::ThirdHeight => (8, 3),
        PixelSize::Sextant => (8, 2),
        PixelSize::QuarterHeight => (8, 2),
        PixelSize::Octant => (8, 1),
    }
}

/// How wide `WORDMARK` is at `size`, in cells.
fn mark_width(size: PixelSize) -> u16 {
    glyph_box(size)
        .0
        .saturating_mul(WORDMARK.chars().count() as u16)
}

/// The largest size at which the wordmark still fits `area`.
///
/// Clipped, a wordmark stops being a wordmark, so the sizes are walked in order
/// and the smallest is used when nothing fits — a 20-column terminal still gets
/// a readable name rather than two and a half letters.
fn fit(area: Rect) -> PixelSize {
    for size in [
        PixelSize::Full,
        PixelSize::HalfHeight,
        PixelSize::HalfWidth,
        PixelSize::Quadrant,
        PixelSize::ThirdHeight,
        PixelSize::QuarterHeight,
        PixelSize::Octant,
    ] {
        let (_, gh) = glyph_box(size);
        if u32::from(mark_width(size)) <= u32::from(area.width) && gh <= area.height {
            return size;
        }
    }
    PixelSize::Octant
}

/// The wordmark as one line per character, each on its own point of the theme's
/// accent ramp.
///
/// A `BigText` line carries a single style, so the hueshift is expressed by
/// splitting the word per character. `phase` rotates the whole ramp, which is
/// what animates it.
fn wordmark(app: &AppTheme, phase: f64) -> Vec<Line<'static>> {
    // The theme's own accents, cycled: a theme that ships one accent still gets
    // a gradient because the ramp repeats.
    let ramp = [app.accent, app.secondary_accent, app.tertiary_accent];
    let n = ramp.len() as f64;
    WORDMARK
        .chars()
        .enumerate()
        .map(|(i, c)| {
            let t = (i as f64 / n + phase).rem_euclid(1.0);
            let color = blend_colors(ramp[i % 3], ramp[(i + 1) % 3], t);
            Line::from(Span::styled(c.to_string(), Style::default().fg(color)))
        })
        .collect()
}

/// The wordmark centred in `area`.
fn logo(f: &mut ratatui::Frame, area: Rect, app: &AppTheme, phase: f64, size: PixelSize) {
    let (_, gh) = glyph_box(size);
    let w = mark_width(size);
    let slot = Rect {
        x: area.x + area.width.saturating_sub(w) / 2,
        y: area.y + area.height.saturating_sub(gh) / 2,
        width: w.min(area.width),
        height: gh.min(area.height),
    };
    f.render_widget(
        BigText::builder()
            .pixel_size(size)
            .style(Style::default())
            .lines(wordmark(app, phase))
            .alignment(Alignment::Center)
            .build(),
        slot,
    );
}

/// A centred single-line caption.
fn caption(f: &mut ratatui::Frame, area: Rect, text: &str, style: Style) {
    f.render_widget(
        Paragraph::new(Line::from(Span::styled(text.to_string(), style)))
            .alignment(Alignment::Center),
        area,
    );
}

/// The first-run onboarding view.
///
/// Laid out around one rule: the name is the biggest thing on the screen and
/// everything else is support, so the wordmark takes the largest size that
/// leaves room for the text beneath it and the slack is split above and below
/// rather than pooled at the top. A view pinned to the top edge with an empty
/// lower two thirds reads as a rendering fault, not as a welcome.
pub(crate) fn onboard(
    f: &mut ratatui::Frame,
    area: Rect,
    app: &AppTheme,
    icon_style: &str,
    phase: f64,
) {
    // One blank row of breathing room either side of the text block, and the
    // rows the block itself needs. The wordmark is fitted into what is left.
    let text_rows = 5u16;
    let gutter = 1u16;
    let (_, gh) = glyph_box(PixelSize::Full);
    let room_h = area.height.saturating_sub(text_rows + gutter * 2);
    let room_w = area.width;
    let size = if room_h >= gh && u32::from(mark_width(PixelSize::Full)) <= u32::from(room_w) {
        PixelSize::Full
    } else {
        fit(area)
    };
    let (_, mark_h) = glyph_box(size);

    // Slack is divided evenly: half opens the composition, half closes it.
    let slack = area.height.saturating_sub(mark_h + text_rows + gutter * 2) / 2;
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(slack),
            Constraint::Length(gutter),
            Constraint::Length(mark_h),
            Constraint::Length(gutter),
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(1),
        ])
        .split(area);

    logo(f, rows[2], app, phase, size);
    caption(f, rows[4], TITLE, Style::default().fg(app.fg));
    caption(f, rows[5], SITE, Style::default().fg(app.accent));
    caption(
        f,
        rows[6],
        "Supported providers",
        Style::default().fg(app.fg_dim).add_modifier(Modifier::BOLD),
    );
    let providers: Vec<String> = crate::ui::providers(icon_style)
        .iter()
        .map(|(glyph, name)| format!("{glyph} {name}"))
        .collect();
    caption(
        f,
        rows[7],
        &providers.join("   "),
        Style::default().fg(app.fg_dim),
    );
    caption(
        f,
        rows[8],
        "press any key to continue",
        Style::default().fg(app.fg_dim).add_modifier(Modifier::DIM),
    );
}

/// The `Widget` bound `BigText` needs, spelled out so the import list stays
/// honest when the feature set changes.
const _: fn() = || {
    fn assert_widget<W: Widget>() {}
    assert_widget::<BigText>();
};
