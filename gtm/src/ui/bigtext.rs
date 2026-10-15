// Copyright (c) 2026
// Author: prjctimg <prjctimg@outlook.com>
// Oversized type: the first-run onboarding view
//
// This is free software released under the GPL-3.0 license.

//! The one view that is about the program rather than the music.
//!
//! The name is drawn in glyphs large enough to be the subject of the screen,
//! with the full title set beside it in ordinary type so the wordmark is never
//! asked to carry the meaning on its own. It is a floating pane rather than a
//! full view: the library is already loaded behind it, and a welcome that hides
//! the program the user just installed reads as a splash screen for an app that
//! has nothing to show yet.

use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use tui_big_text::{BigText, PixelSize};

use crate::theme::{AppTheme, blend_colors};
use crate::ui::*;

/// The program's name, drawn oversized.
const WORDMARK: &str = "gtm";

/// What the program is, set beside the name. Read rather than recognised, which
/// is why it is body text and not more glyphs.
const TITLE: &str = "terminal audio player";

/// Where the name points.
const SITE: &str = "gtmd.dev";

/// Blank rows between the wordmark and the text beneath it, and between that
/// text and the provider row.
const GUTTER: u16 = 1;

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

/// The largest size at which the wordmark fits `w` by `h`.
///
/// Clipped, a wordmark stops being a wordmark, so the sizes are walked in order
/// and the smallest is used when nothing fits — a 20-column terminal still gets
/// a readable name rather than two and a half letters.
fn fit(w: u16, h: u16) -> PixelSize {
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
        if u32::from(mark_width(size)) <= u32::from(w) && gh <= h {
            return size;
        }
    }
    PixelSize::Octant
}

/// The wordmark as a single line, each letter on its own point of the theme's
/// accent ramp.
///
/// One `Line`, not one line per letter: `BigText` gives every line its own band
/// of `pixel_size` rows, so three lines stack the letters instead of setting
/// them side by side and only the first one is ever on screen. A `Line` carries
/// many styles, so the ramp is expressed by one span per character instead.
/// `phase` rotates the whole ramp, which is what animates it.
fn wordmark(app: &AppTheme, phase: f64) -> Line<'static> {
    // The theme's own accents, cycled: a theme that ships one accent still gets
    // a gradient because the ramp repeats.
    let ramp = [app.accent, app.secondary_accent, app.tertiary_accent];
    let n = ramp.len() as f64;
    let spans = WORDMARK
        .chars()
        .enumerate()
        .map(|(i, c)| {
            let t = (i as f64 / n + phase).rem_euclid(1.0);
            let color = blend_colors(ramp[i % 3], ramp[(i + 1) % 3], t);
            Span::styled(c.to_string(), Style::default().fg(color))
        })
        .collect::<Vec<_>>();
    Line::from(spans)
}

/// The wordmark centred in `area`, at the hueshift `phase`.
fn logo(f: &mut ratatui::Frame, area: Rect, app: &App, size: PixelSize) {
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
            .lines(vec![wordmark(&app.theme, app.logo_phase())])
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

/// The supported providers as one row, narrowed to `w` cells.
///
/// Glyphs first, then the names alone, then fewer names: a clipped row reads as
/// a rendering fault, and this is the one line whose width the terminal decides
/// rather than the layout.
fn providers(app: &App, w: u16) -> Line<'static> {
    let all = crate::ui::providers(&app.icon_style);
    let (glyph, name) = (
        Style::default().fg(app.theme.accent),
        Style::default().fg(app.theme.fg_dim),
    );
    let row = |parts: &[Vec<Span<'static>>], gap: &str| {
        Line::from(
            parts
                .iter()
                .enumerate()
                .flat_map(|(i, p)| {
                    let sep = if i > 0 {
                        Some(Span::raw(gap.to_string()))
                    } else {
                        None
                    };
                    sep.into_iter().chain(p.iter().cloned())
                })
                .collect::<Vec<_>>(),
        )
    };
    let full: Vec<Vec<Span<'static>>> = all
        .iter()
        .map(|(g, n)| vec![Span::styled(*g, glyph), Span::styled(*n, name)])
        .collect();
    let line = row(&full, "   ");
    if line.width() as u16 <= w {
        return line;
    }
    let plain: Vec<Vec<Span<'static>>> = all
        .iter()
        .map(|(_, n)| vec![Span::styled(*n, name)])
        .collect();
    let line = row(&plain, " ");
    if line.width() as u16 <= w {
        return line;
    }
    (0..plain.len())
        .rev()
        .find_map(|i| {
            let l = row(&plain[..=i], " ");
            (l.width() as u16 <= w).then_some(l)
        })
        .unwrap_or_default()
}

/// The first-run onboarding view: a floating pane over the library, drawn with
/// the same chrome as every picker.
///
/// Laid out around one rule: the name is the biggest thing on the pane and
/// everything else is support, so the wordmark takes the top band and the
/// providers row is pinned to the last row the panel's background covers. The
/// slack between them is what separates the identity block from the list, which
/// is also what puts the wordmark above the pane's own centre.
pub(crate) fn onboard(f: &mut ratatui::Frame, area: Rect, app: &App) {
    // Border and horizontal padding either side, so the width below is what the
    // content itself is measured against.
    const CHROME_W: u16 = 4;
    let w = providers(app, area.width.saturating_sub(CHROME_W + 4)).width() as u16;
    let w = w.saturating_add(CHROME_W).min(area.width.saturating_sub(2));
    if w < CHROME_W + mark_width(PixelSize::Octant) {
        return;
    }
    let inner_w = w.saturating_sub(CHROME_W);
    // The tallest the name may be on a pane with room for the text under it.
    let size = fit(inner_w, area.height.saturating_sub(10));
    let (_, mark_h) = glyph_box(size);
    let h = mark_h
        .saturating_add(GUTTER * 3)
        // Border, vertical padding and the four text rows.
        .saturating_add(6)
        .min(area.height.saturating_sub(2));
    let pane = Rect {
        x: area.x + area.width.saturating_sub(w) / 2,
        y: area.y + area.height.saturating_sub(h) / 2,
        width: w,
        height: h,
    };

    f.render_widget(Clear, pane);
    let block = Block::default()
        .title(Line::from(Span::styled(
            " Welcome ",
            Style::default()
                .fg(app.theme.accent)
                .add_modifier(Modifier::BOLD),
        )))
        .padding(Padding {
            left: 1,
            right: 1,
            top: 1,
            bottom: 1,
        })
        .style(Style::default().bg(if app.transparent_pickers {
            Color::Reset
        } else {
            app.float_bg()
        }));
    let inner = block.inner(pane);
    f.render_widget(block, pane);

    // Name, a gutter, the two lines that say what this is, a gutter, the hint,
    // and the provider row pinned to the last row the panel covers.
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(mark_h),
            Constraint::Length(GUTTER),
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(GUTTER),
            Constraint::Min(0),
            Constraint::Length(1),
        ])
        .split(inner);

    logo(f, rows[0], app, size);
    caption(f, rows[2], TITLE, Style::default().fg(app.theme.fg));
    caption(f, rows[3], SITE, Style::default().fg(app.theme.accent));
    // In the slack, so it sits between the identity block and the provider row
    // rather than crowding either of them, and disappears with the slack when
    // the pane is too short to hold it.
    if rows[5].height > 0 {
        caption(
            f,
            rows[5],
            "press any key to continue",
            Style::default()
                .fg(app.theme.fg_dim)
                .add_modifier(Modifier::DIM),
        );
    }
    f.render_widget(
        Paragraph::new(providers(app, inner.width)).alignment(Alignment::Center),
        rows[6],
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    /// The wordmark has to be one line, and it has to fit its own box.
    ///
    /// `BigText` gives every line its own band of `pixel_size` rows, so three
    /// lines stack the letters instead of setting them side by side and only the
    /// first lands inside the area — which rendered the name as a lone `g`.
    #[test]
    fn the_wordmark_is_one_line_that_fits() {
        let theme = crate::theme::builtin_themes()[0].theme;
        let line = wordmark(&theme, 0.0);
        assert_eq!(
            line.spans.len(),
            WORDMARK.chars().count(),
            "one span per letter, so the ramp survives"
        );
        assert_eq!(line.to_string(), WORDMARK);

        // Rendered: every letter's columns are non-blank, side by side, within
        // the slot the size was fitted for.
        let size = fit(64, 8);
        assert_eq!(size, PixelSize::Full);
        let w = mark_width(size);
        let (_, gh) = glyph_box(size);
        let mut term = Terminal::new(TestBackend::new(w, gh)).unwrap();
        term.draw(|f| {
            f.render_widget(
                BigText::builder()
                    .pixel_size(size)
                    .lines(vec![line])
                    .build(),
                Rect::new(0, 0, w, gh),
            );
        })
        .unwrap();
        let buf = term.backend().buffer();
        let ink = |x: u16, y: u16| buf[(x, y)].symbol() != " ";
        // Each letter owns 8 columns, and each must have ink in them. A name set
        // as three lines puts each letter in its own row band instead, so the
        // second and third bands would be blank — which is how `gtm` rendered
        // as a lone `g`. Ink per band, not per column: an 8x8 glyph has blank
        // columns of its own (`m` ends on its second-from-last), so demanding
        // all eight would be asserting something about the font.
        let gw = glyph_box(size).0;
        for letter in 0..WORDMARK.chars().count() {
            let at = letter as u16 * gw;
            let painted = (at..at + gw)
                .flat_map(|x| (0..gh).map(move |y| (x, y)))
                .filter(|&(x, y)| ink(x, y))
                .count();
            assert!(
                painted > 0,
                "letter {letter} painted no cells in columns {}..{}: \
                 the name is not set side by side",
                at,
                at + gw
            );
        }
    }

    /// The name is fitted to the pane, and only the floor may still overflow.
    ///
    /// `fit` walks the sizes and takes the largest that fits, falling back to the
    /// smallest when none does — a pane too narrow for even that clips rather
    /// than drawing nothing, which is the documented trade.
    #[test]
    fn fit_takes_the_largest_size_that_fits() {
        for w in [24u16, 40, 200] {
            for h in [2u16, 4, 8, 20] {
                let size = fit(w, h);
                let (_, gh) = glyph_box(size);
                assert!(
                    u32::from(mark_width(size)) <= u32::from(w),
                    "{size:?} needs {} columns in {w}",
                    mark_width(size)
                );
                assert!(gh <= h.max(1), "{size:?} needs {gh} rows in {h}");
                // And nothing larger would have fit either, which is what makes
                // this the *largest* rather than merely a size that fits.
                for bigger in [PixelSize::Full, PixelSize::HalfHeight, PixelSize::HalfWidth] {
                    if bigger == size {
                        break;
                    }
                    let (_, bh) = glyph_box(bigger);
                    assert!(
                        u32::from(mark_width(bigger)) > u32::from(w) || bh > h,
                        "{bigger:?} also fits {w}x{h}, so {size:?} was not the largest"
                    );
                }
            }
        }
        // Below the narrowest name there is no size to fall back to, and the
        // floor clips rather than vanishing.
        assert_eq!(fit(8, 8), PixelSize::Octant);
    }
}
