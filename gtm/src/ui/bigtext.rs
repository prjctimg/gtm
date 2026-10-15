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
const TITLE: &str = "The missing terminal audio player.";

/// Where the name points.
const SITE: &str = "gtmd.dev";

/// The call to action, and where it goes. A welcome that only describes what the
/// program is leaves the reader with nothing to do about it.
const CTA: &str = "★  Star gtm on GitHub";

/// Where [`CTA`] leads. `s` on the welcome pane opens it.
pub(crate) const REPO: &str = "https://github.com/prjctimg/gtm";

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

/// The supported providers as one jumbotron row.
///
/// The whole list, always — glyphs, names and all. When the pane is narrower than
/// the list, the row scrolls instead of being cut: the tail runs off the right
/// edge and comes back in on the left, so every provider is read. Dropping the
/// overflow was the alternative and it was the wrong one here: the list is the
/// pitch, and a pane quietly showing five of seven sources is not a shorter
/// pitch, it is a different one.
///
/// Scrolled over the spans rather than over a joined string, because the glyphs
/// are accent-coloured and the names are not — a plain `String` would have
/// flattened both into one colour to buy a helper that already existed.
fn providers(app: &App, frame: usize, w: u16) -> Line<'static> {
    let all = crate::ui::providers(&app.icon_style);
    let mut spans: Vec<Span<'static>> = Vec::with_capacity(all.len() * 2);
    for (i, (glyph, name)) in all.iter().enumerate() {
        if i > 0 {
            spans.push(Span::raw("   "));
        }
        spans.push(Span::styled(
            (*glyph).to_string(),
            Style::default().fg(app.theme.accent),
        ));
        spans.push(Span::styled(
            (*name).to_string(),
            Style::default().fg(app.theme.fg_dim),
        ));
    }
    let total = Line::from(spans.clone()).width() as u16;
    if total <= w {
        return Line::from(spans);
    }
    // One column per step, for `total + w` steps so the whole row passes through
    // the window once — the tail leaving on the right as the head arrives on the
    // left, which is the jumbotron — then held at the start long enough to be
    // read before it runs again. Holding at the end instead would park the last
    // provider on screen alone and leave the first one unread.
    let cycle = total as usize + w as usize + w as usize;
    let pos = (frame / SCROLL_SPEED) % cycle;
    let skip = if pos < total as usize + w as usize {
        pos % total as usize
    } else {
        0
    };
    Line::from(slice(&spans, skip, w as usize))
}

/// The `w` cells of `spans` starting `skip` cells in, wrapping at the end.
///
/// The wrap is what makes it a jumbotron rather than a truncated line: the tail
/// returns on the left instead of stopping.
fn slice(spans: &[Span<'static>], skip: usize, w: usize) -> Vec<Span<'static>> {
    let chars: Vec<(char, Style)> = spans
        .iter()
        .flat_map(|s| {
            let style = s.style;
            s.content.chars().map(move |c| (c, style))
        })
        .collect();
    let n = chars.len();
    let out: Vec<Span<'static>> = (0..w)
        .map(|i| {
            let (c, style) = chars[(skip + i) % n];
            Span::styled(c.to_string(), style)
        })
        .collect();
    // Adjacent cells of one span are re-joined so the row is a handful of spans
    // rather than `w` of them, which matters at sixty frames a second.
    let mut merged: Vec<Span<'static>> = Vec::new();
    for span in out {
        match merged.last_mut() {
            Some(last) if last.style == span.style => {
                last.content.to_mut().push_str(&span.content);
            }
            _ => merged.push(span),
        }
    }
    merged
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
    // The pane is sized from the name and the longest line beneath it. The
    // provider row is deliberately not one of them: it scrolls, so a list wider
    // than the pane is not a reason to widen the pane, and letting it do that is
    // what made this welcome full-bleed on a wide terminal and a sliver on a
    // narrow one.
    let inner_w = mark_width(PixelSize::Full)
        .max(TITLE.chars().count() as u16)
        .max(CTA.chars().count() as u16 + 6)
        .max(38);
    let w = inner_w
        .saturating_add(CHROME_W)
        .min(area.width.saturating_sub(2));
    if w < CHROME_W + mark_width(PixelSize::Octant) {
        return;
    }
    let inner_w = w.saturating_sub(CHROME_W);
    // The tallest the name may be on a pane with room for the text under it.
    let size = fit(inner_w, area.height.saturating_sub(14));
    let (_, mark_h) = glyph_box(size);
    // Name, three gutters, five text rows, the border and the vertical padding.
    let h = mark_h
        .saturating_add(GUTTER * 3)
        .saturating_add(5 + 4)
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

    // Name, gutters, the lines that say what this is, the call to action, the
    // hint, and the provider row pinned to the last row the panel covers.
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(mark_h),
            Constraint::Length(GUTTER),
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(GUTTER),
            Constraint::Length(1),
            Constraint::Min(0),
            Constraint::Length(1),
        ])
        .split(inner);

    logo(f, rows[0], app, size);
    caption(f, rows[2], TITLE, Style::default().fg(app.theme.fg));
    caption(f, rows[3], SITE, Style::default().fg(app.theme.accent));
    caption(
        f,
        rows[5],
        CTA,
        Style::default()
            .fg(app.theme.accent)
            .add_modifier(Modifier::BOLD),
    );
    // The key that opens it, on the same row as the site rather than as a
    // seventh line: the welcome has one action and it should not cost a line of
    // its own to say which key.
    // In the slack, so it sits between the identity block and the provider row
    // rather than crowding either of them, and disappears with the slack when
    // the pane is too short to hold it.
    if rows[6].height > 0 {
        caption(
            f,
            rows[6],
            "press any key to continue",
            Style::default()
                .fg(app.theme.fg_dim)
                .add_modifier(Modifier::DIM),
        );
    }
    f.render_widget(
        Paragraph::new(providers(app, app.footer_title_scroll, rows[7].width))
            .alignment(Alignment::Center),
        rows[7],
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

    /// The provider row wraps rather than dropping its tail.
    ///
    /// The bug it guards was silent truncation — the list was cut to whatever
    /// fitted, so a narrow pane quietly pitched five of seven sources, which is
    /// not a shorter pitch but a different one.
    #[test]
    fn the_provider_row_wraps_instead_of_dropping() {
        let all: Vec<char> = "abcdefghij".chars().collect();
        let spans = vec![Span::raw(all.iter().collect::<String>())];
        let w = 6;

        // Nothing is lost: every character reaches the window, and the window is
        // always exactly full — a jumbotron, not a shrinking line.
        let mut seen = std::collections::HashSet::new();
        for skip in 0..all.len() {
            let row: String = slice(&spans, skip, w)
                .iter()
                .map(|s| s.content.as_ref())
                .collect();
            assert_eq!(row.chars().count(), w, "skip {skip}: window not full");
            seen.extend(row.chars());
        }
        assert_eq!(
            seen.len(),
            all.len(),
            "a character never reached the window"
        );

        // And it wraps: past the end the head returns on the left, so the last
        // character is followed by the first rather than by nothing.
        let tail = slice(&spans, all.len() - 1, w);
        let head = slice(&spans, 0, w);
        assert_eq!(tail.last().unwrap().content.as_ref(), "j");
        assert_eq!(head.first().unwrap().content.as_ref(), "a");
    }

    /// Styles survive the scroll, or the accent on the glyphs is lost.
    #[test]
    fn the_provider_row_keeps_its_colours() {
        let accent = Style::default().fg(Color::Rgb(1, 2, 3));
        let spans = vec![
            Span::styled("ab", accent),
            Span::raw("cd"),
            Span::styled("ef", accent),
        ];
        let row = slice(&spans, 1, 4);
        let text: String = row.iter().map(|s| s.content.as_ref()).collect();
        assert_eq!(text, "bcde");
        assert_eq!(row[0].style, accent);
        assert_eq!(row[3].style, accent);
        assert_ne!(row[1].style, row[2].style, "the middle kept its own style");
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
