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
use tui_big_text::BigText;

/// Re-exported so the About window can ask for a size without naming the crate
/// the widget comes from.
pub(crate) use tui_big_text::PixelSize;

use crate::theme::{AppTheme, blend_colors};
use crate::ui::*;

/// The program's name, drawn oversized.
const WORDMARK: &str = "gtm";

/// What the program is, set beside the name. Read rather than recognised, which
/// is why it is body text and not more glyphs.
const TITLE: &str = "The missing terminal audio player.";

/// Where the name points.
pub(crate) const SITE: &str = "gtmd.dev";

/// What the hint row says, and the key it names. The call to action is the hint
/// rather than a line of its own: it is one key, and a welcome that spends a
/// whole row on "star us" next to a row saying "press any key" is two paragraphs
/// saying one thing.
const HINT: &str = "Press s to star on GitHub";

/// The way out of the pane for good, on the same row as the call to action.
///
/// Said here because `H` is otherwise invisible: it is not in the keymap, not
/// in `?`, and not on any strip. A binding nobody can discover is not a
/// binding.
const NEVER_AGAIN: &str = "H: never show again";

/// The mark at the end of the hint: GitHub's, when the font has it.
///
/// Font Awesome's mark rather than Material Design's, for no better reason than
/// that it is the one that sits consistently with the other `nf-fa-*` glyphs in
/// the icon table. There is no official GitHub mark in any terminal font — the
/// real one is a wordmark with a registered shape — so this is an Octocat in
/// one idiom or another, not the brand asset.
///
/// Two glyphs because the pane draws for both fonts: a private-use codepoint on
/// a terminal without the nerd font is a tofu box, and a pane that ends in a
/// tofu box is worse than one that ends in an arrow.
pub(crate) fn mark(icon_style: &str) -> &'static str {
    if icon_style == "mdi" && crate::ui::use_nerd_fonts() {
        "\u{f09b}" // nf-fa-github
    } else {
        "↗"
    }
}

/// Where the hint leads. `s` on the welcome pane and in About opens it.
pub(crate) const REPO: &str = "https://github.com/prjctimg/gtm";

/// Blank rows between the wordmark and the text beneath it, and between that
/// text and the provider row.
const GUTTER: u16 = 1;

/// Ticks of the scroll clock per cell of marquee travel.
///
/// One. The clock ticks once every third frame and the welcome now forces a
/// redraw on every frame, so this is one cell per three frames -- a steady
/// slide rather than the stall-then-jump it was tuned around.
const MARCH: usize = 1;

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

/// How tall `WORDMARK` is at `size`, in cells.
///
/// Exposed next to [`logo`] because the two travel together: a caller reserving
/// rows for the mark and a caller sizing the mark are asking the same question,
/// and a second answer to it would be a second place for them to disagree.
pub(crate) fn mark_height(size: PixelSize) -> u16 {
    glyph_box(size).1
}

/// The wordmark centred in `area`, at the hueshift `phase`.
///
/// Public so About draws the same mark as the welcome rather than a second
/// interpretation of it: one `Line` of glyphs at whatever size the caller has
/// room for, centred, hueshifting with the theme's own phase.
pub(crate) fn logo(f: &mut ratatui::Frame, area: Rect, app: &App, size: PixelSize) {
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
/// Cells every glyph occupies, so every item's name starts at the same column.
///
/// A missing glyph used to contribute zero cells, so `Local` -- and `Radio`
/// and `Podcast` in emoji mode, where the icon table has nothing for them --
/// read as the separator plus a space before the name, while every glyphed
/// item read as one glyph plus a space. Four extra columns of gape before
/// exactly the items the directory had no icon for, which is what made
/// `Local` look detached from its neighbours rather than evenly set with them.
///
/// Fixed rather than "pad only when missing" because a glyph's own width is not
/// something this code can know: a Private Use Area codepoint is East Asian
/// Ambiguous and renders one cell or two depending on the terminal's locale, so
/// a slot sized to whatever this build measures would drift on someone else's
/// machine. Two cells is the widest any of these render, so the slot never clips
/// one and never varies.
const GLYPH_SLOT: usize = 2;

/// The separator between items: a dot on two spaces each side.
const PROVIDER_GAP: &str = "  \u{b7}  ";

/// `slot + space + name` per item, with the separator between.
///
/// Split out of [`providers`] so the alignment it exists to guarantee can be
/// asserted directly, over both icon styles, instead of being a property
/// inferred from a screenshot.
fn provider_spans(
    all: &[(&'static str, &'static str)],
    glyph: Style,
    name: Style,
) -> Vec<Span<'static>> {
    let mut spans: Vec<Span<'static>> = Vec::with_capacity(all.len() * 3);
    for (i, (glyph_c, name_c)) in all.iter().enumerate() {
        if i > 0 {
            spans.push(Span::styled(PROVIDER_GAP.to_string(), glyph));
        }
        // Measured with ratatui's own `Line::width` rather than a second width
        // implementation, so the padding matches exactly what the renderer
        // will lay out. A hand-rolled `chars().count()` disagrees with ratatui
        // on every wide glyph and pads the wrong amount.
        let mut slot = (*glyph_c).to_string();
        while Line::from(slot.as_str()).width() < GLYPH_SLOT {
            slot.push(' ');
        }
        spans.push(Span::styled(slot, glyph));
        spans.push(Span::styled(format!(" {name_c}"), name));
    }
    spans
}

fn providers(app: &App, frame: usize, w: u16) -> Line<'static> {
    let all = crate::ui::providers(&app.icon_style);
    let (glyph, name) = (
        Style::default().fg(app.theme.accent),
        Style::default().fg(app.theme.fg_dim),
    );
    let spans = provider_spans(&all, glyph, name);
    let total = Line::from(spans.clone()).width() as u16;
    if total <= w {
        return Line::from(spans);
    }
    // Constant motion, one cell at a time, wrapping forever. It used to hold at
    // the start for a window's width before restarting, which read as a stall
    // followed by a jump — the jank. A jumbotron that pauses is not revolving;
    // the motion is the whole thing.
    //
    // One cell every `MARCH` ticks rather than every tick: `frame` already ticks
    // once every third rendered frame, so one per tick would carry the row past
    // a name before it could be read, which trades the old stall for a blur. A
    // single cell per step — never several — is what makes it read as sliding
    // rather than as a jump.
    Line::from(slice(&spans, march(frame, total as usize), w as usize))
}

/// Where in a row of `total` cells the marquee is at tick `frame`.
///
/// Split out from the renderer so the motion can be tested on its own: the bug
/// this replaced was not in the drawing but in the arithmetic deciding where to
/// draw, and arithmetic is cheap to assert on.
fn march(frame: usize, total: usize) -> usize {
    (frame / MARCH) % total
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
    // The pane is sized from the name and the longest line beneath it, plus a
    // floor. The provider row is deliberately not one of them: it scrolls, so a
    // list wider than the pane is not a reason to widen the pane, and letting it
    // do that is what made this welcome full-bleed on a wide terminal and a
    // sliver on a narrow one.
    let inner_w = mark_width(PixelSize::Full)
        .max(TITLE.chars().count() as u16)
        .max(HINT.chars().count() as u16 + 4)
        .max(52);
    let w = inner_w
        .saturating_add(CHROME_W)
        .min(area.width.saturating_sub(2));
    if w < CHROME_W + mark_width(PixelSize::Octant) {
        return;
    }
    let inner_w = w.saturating_sub(CHROME_W);
    // The tallest the name may be on a pane with room for the text under it.
    let size = fit(inner_w, area.height.saturating_sub(12));
    let (_, mark_h) = glyph_box(size);
    // Name, two gutters, four text rows, the border and the vertical padding.
    let h = mark_h
        .saturating_add(GUTTER * 2)
        .saturating_add(4 + 4)
        .min(area.height.saturating_sub(2));
    let pane = Rect {
        x: area.x + area.width.saturating_sub(w) / 2,
        y: area.y + area.height.saturating_sub(h) / 2,
        width: w,
        height: h,
    };

    f.render_widget(Clear, pane);
    // Same top-right Esc chip every picker carries, so the pane is dismissible
    // by the same key and says so the same way rather than being a modal the
    // user has to guess at.
    let block = Block::default()
        .title(Line::from(Span::styled(
            " Welcome ",
            Style::default()
                .fg(app.theme.accent)
                .add_modifier(Modifier::BOLD),
        )))
        .title(
            Line::from(Span::styled(
                "  Esc  ",
                Style::default().fg(app.theme.fg_dim),
            ))
            .right_aligned(),
        )
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

    // Name, gutters, the lines that say what this is, the hint carrying the call
    // to action, and the provider row pinned to the last row the panel covers.
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
        f.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled(
                    HINT.to_string(),
                    Style::default()
                        .fg(app.theme.accent)
                        .add_modifier(Modifier::BOLD),
                ),
                // The mark rides at the end of the sentence rather than on a row
                // of its own: it is the logo for the link the sentence names, so
                // it reads as part of it.
                Span::styled(
                    format!("  {}", mark(&app.icon_style)),
                    Style::default().fg(app.theme.fg_dim),
                ),
                Span::raw("   "),
                Span::styled(
                    NEVER_AGAIN.to_string(),
                    Style::default().fg(app.theme.fg_dim),
                ),
            ]))
            .alignment(Alignment::Center),
            rows[5],
        );
    }
    f.render_widget(
        Paragraph::new(providers(app, app.footer_title_scroll, rows[6].width))
            .alignment(Alignment::Center),
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

    /// The provider row keeps every cell, and never stops.
    ///
    /// Two bugs in one property, because they are the same row: it used to be cut
    /// to whatever fitted, so a narrow pane quietly pitched five of seven
    /// sources; and it used to hold still for a window's width before restarting,
    /// which read as a stall followed by a jump. A jumbotron that pauses is not
    /// revolving.
    #[test]
    fn the_provider_row_wraps_without_stalling() {
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

        // And it wraps: scrolled to the last character, the head returns on the
        // left rather than the row stopping. This is the whole behaviour — a
        // truncated row would end here, at `j` followed by five blanks.
        let tail: String = slice(&spans, all.len() - 1, w)
            .iter()
            .map(|s| s.content.as_ref())
            .collect();
        assert_eq!(tail, "jabcde", "the row did not wrap back to the head");

        // And it never holds: every tick moves the offset exactly one cell, all
        // the way round. The old code sat still for a whole window's width of
        // ticks and then jumped, which is what read as jank -- a hold is a tick
        // that leaves the offset alone, and there are none.
        //
        // `MARCH` is 1 because the welcome now forces a redraw every frame, so
        // one cell per tick is one cell per frame: a steady slide, and slow
        // enough to read. The assertion holds for any `MARCH`, so a future
        // change to the speed is caught here too.
        assert_eq!(MARCH, 1, "the marquee speed is tied to a per-frame redraw");
        let total = 37;
        for f in 0..(total * 4) {
            let at = march(f, total);
            assert!(
                at < total,
                "tick {f}: offset {at} ran off the end of a {total}-cell row"
            );
            if f > 0 {
                let prev = march(f - 1, total);
                assert_eq!(
                    at,
                    (prev + 1) % total,
                    "tick {f}: the row jumped {prev} -> {at} instead of one cell"
                );
            }
        }
        // And it comes back around to where it started rather than stopping at
        // the end of the row, which is the other half of "revolving".
        assert_eq!(march(total * 7, total), 0);
    }

    /// Every item in the provider row is the same shape: a full-width glyph
    /// slot, one space, the name.
    ///
    /// The bug this pins: a source the directory had no icon for contributed
    /// zero cells, so its name started four columns left of everyone else's --
    /// `Local`, plus `Radio` and `Podcast` in emoji mode. Both styles are
    /// covered because the gap only appears where a glyph is missing, and only
    /// the emoji table has missing glyphs.
    ///
    /// Asserted per span rather than by comparing item starts, because the
    /// names are different lengths and a uniform stride is therefore not the
    /// property. What has to hold is that every slot is the same width whatever
    /// glyph, or no glyph, is in it.
    #[test]
    fn every_provider_item_is_the_same_shape() {
        for style in ["mdi", "emoji"] {
            let all = crate::ui::providers(style);
            assert!(!all.is_empty(), "{style}: no providers");
            let spans = provider_spans(&all, Style::default(), Style::default());
            let sep = Line::from(PROVIDER_GAP).width();
            let w = |i: usize| Line::from(spans[i].content.as_ref()).width();
            for i in 0..all.len() {
                let (slot, name_i, sep_i) = (i * 3, i * 3 + 1, i * 3 + 2);
                assert_eq!(
                    w(slot),
                    GLYPH_SLOT,
                    "{style}: item {i} glyph slot is {} wide, want {GLYPH_SLOT} ({:?})",
                    w(slot),
                    spans[slot].content.as_ref()
                );
                assert_eq!(
                    spans[name_i].content.as_ref(),
                    format!(" {}", all[i].1),
                    "{style}: item {i} name span"
                );
                if i + 1 < all.len() {
                    assert_eq!(w(sep_i), sep, "{style}: item {i} separator width");
                }
            }
            // And the row is exactly the sum of its parts, so nothing hides
            // outside the spans.
            let sum: usize = spans
                .iter()
                .map(|s| Line::from(s.content.as_ref()).width())
                .sum();
            assert_eq!(Line::from(spans.clone()).width(), sum, "{style}");
        }
    }

    /// A missing glyph still gets a full slot -- which is the whole fix, and
    /// needs a fixture that actually has one.
    #[test]
    fn a_missing_glyph_still_gets_a_full_slot() {
        let all = crate::ui::providers("emoji");
        assert!(
            all.iter().any(|(g, _)| g.is_empty()),
            "the emoji table no longer has a missing glyph to pad"
        );
        let spans = provider_spans(&all, Style::default(), Style::default());
        for (i, name) in all.iter().enumerate() {
            if !name.0.is_empty() {
                continue;
            }
            let slot = &spans[i * 3].content;
            assert_eq!(
                Line::from(slot.as_ref()).width(),
                GLYPH_SLOT,
                "{:?} got a {}-cell slot",
                name.1,
                Line::from(slot.as_ref()).width()
            );
            assert!(
                slot.trim().is_empty(),
                "{:?} slot is not blank: {slot:?}",
                name.1
            );
        }
    }

    /// Items in the provider row are separated well enough to see a boundary.
    ///
    /// At the old three spaces, a name ran straight into the next item's glyph
    /// and the row read as one run-on word rather than as a list — the spacing
    /// was what made it legible as a list rather than as a smear.
    #[test]
    fn provider_items_are_visibly_separated() {
        let pairs = [("A", "Name"), ("B", "Other")];
        let (glyph, name) = (
            Style::default().fg(Color::Rgb(1, 2, 3)),
            Style::default().fg(Color::Rgb(4, 5, 6)),
        );
        let gap = "  ·  ";
        let mut spans = Vec::new();
        for (i, (g, n)) in pairs.iter().enumerate() {
            if i > 0 {
                spans.push(Span::styled(gap.to_string(), glyph));
            }
            spans.push(Span::styled((*g).to_string(), glyph));
            spans.push(Span::styled(format!(" {n}"), name));
        }
        let row: String = slice(&spans, 0, 40)
            .iter()
            .map(|s| s.content.as_ref())
            .collect();
        assert!(row.contains(gap), "no separator between items: {row}");
        // A name is followed by a gap before the next glyph, and never by one
        // directly — that adjacency is what made the row run on.
        assert!(
            !row.contains("NameB"),
            "a name butted straight into the next glyph: {row}"
        );
        assert!(row.contains("Name  ·  B"), "spacing changed shape: {row}");
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
        // Three spans, not four: `slice` merges neighbours that share a style,
        // which is what keeps a sixty-column row from being sixty spans a
        // frame. So `c` and `d` arrive as one plain span between two accent
        // ones, and the accent survives at both ends of the window.
        assert_eq!(row.len(), 3, "merging did not run: {text}");
        assert_eq!(row[0].style, accent, "the first cell lost its accent");
        assert_eq!(row[2].style, accent, "the last cell lost its accent");
        assert_ne!(
            row[0].style, row[1].style,
            "the middle kept the accent it should not have"
        );
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
