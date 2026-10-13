// Copyright (c) 2026
// Author: prjctimg <prjctimg@outlook.com>
// Oversized type: the idle logo and the first-run onboarding view
//
// This is free software released under the GPL-3.0 license.

//! The `gtm` wordmark, and the one view that is about the program rather than
//! the music.
//!
//! Both surfaces here are one idea at two scales: a name drawn in glyphs large
//! enough to be the subject of the screen. The idle logo is the app waiting;
//! the onboarding view is the app introducing itself. Neither is a status
//! message, so neither is typeset like one.

use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Widget};
use tui_big_text::{BigText, PixelSize};

use crate::theme::{AppTheme, blend_colors};

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

/// The largest size whose three-glyph wordmark fits `area`.
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
        let (gw, gh) = glyph_box(size);
        if u32::from(gw) * 3 <= u32::from(area.width) && gh <= area.height {
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
    "gtm"
        .chars()
        .enumerate()
        .map(|(i, c)| {
            let t = (i as f64 / 3.0 + phase).rem_euclid(1.0);
            let color = blend_colors(ramp[i % 3], ramp[(i + 1) % 3], t);
            Line::from(Span::styled(c.to_string(), Style::default().fg(color)))
        })
        .collect()
}

/// The wordmark centred in `area`.
fn logo(f: &mut ratatui::Frame, area: Rect, app: &AppTheme, phase: f64, size: PixelSize) {
    let (gw, gh) = glyph_box(size);
    let slot = Rect {
        x: area.x + area.width.saturating_sub(gw.saturating_mul(3)) / 2,
        y: area.y + area.height.saturating_sub(gh) / 2,
        width: gw.saturating_mul(3).min(area.width),
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

/// The idle screen: the wordmark, and the site it came from underneath.
///
/// Drawn when nothing is playing and no track is on air, so the pane is the
/// program's name rather than a track list that looks broken.
pub(crate) fn idle(f: &mut ratatui::Frame, area: Rect, app: &AppTheme, phase: f64) {
    if area.width < 8 || area.height < 4 {
        return;
    }
    let (_, gh) = glyph_box(fit(area));
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(gh.saturating_add(1)),
            Constraint::Length(gh),
            Constraint::Length(1),
        ])
        .split(area);
    logo(f, rows[1], app, phase, fit(area));
    caption(f, rows[2], "gtmd.dev", Style::default().fg(app.fg_dim));
}

/// The first-run onboarding view.
///
/// Three things in order of size: the wordmark, the site, and the providers the
/// program can talk to. The provider line is the only part that has to be read
/// rather than recognised, so it keeps body text styling.
pub(crate) fn onboard(
    f: &mut ratatui::Frame,
    area: Rect,
    app: &AppTheme,
    icon_style: &str,
    phase: f64,
) {
    if area.width < 24 || area.height < 10 {
        return;
    }
    // Half-height rather than the largest that fits: onboarding has to leave
    // room for the two captions and the provider line, and a wordmark that eats
    // the pane makes them the smaller thing instead.
    let size = PixelSize::HalfHeight;
    let (_, gh) = glyph_box(size);
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(1),
            Constraint::Length(gh),
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Min(0),
        ])
        .split(area);
    logo(f, rows[1], app, phase, size);
    caption(f, rows[2], "gtmd.dev", Style::default().fg(app.accent));
    caption(
        f,
        rows[3],
        "Supported providers",
        Style::default().fg(app.fg).add_modifier(Modifier::BOLD),
    );
    let providers: Vec<String> = crate::ui::providers(icon_style)
        .iter()
        .map(|(glyph, name)| format!("{glyph} {name}"))
        .collect();
    caption(
        f,
        rows[4],
        &providers.join("   "),
        Style::default().fg(app.fg_dim),
    );
    caption(
        f,
        rows[5],
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
