//! The one progress bar: discrete squared cells, filled ones bright and
//! unfilled ones dim, with the percentage right after them.

pub mod common;

use dev_cleaner::tui::bar::{self, Bar};
use dev_cleaner::tui::palette::{Ramp, Theme};
use ratatui::style::Style;

/// The cells of `parts`: every character that is a bar glyph, in order.
fn glyphs(parts: &[(String, Style)], theme: &Theme) -> Vec<char> {
    let c = theme.cells();
    parts
        .iter()
        .flat_map(|(text, _)| text.chars())
        .filter(|ch| [c.full, c.mark, c.empty].contains(ch))
        .collect()
}

fn text(parts: &[(String, Style)]) -> String {
    parts.iter().map(|(t, _)| t.as_str()).collect()
}

#[test]
fn the_split_between_filled_and_unfilled_cells_follows_the_percentage() {
    // 20 cells: 0 is empty, 1 shows at least one cell, 50 is half, 99 is never
    // full, 100 is full. A full bar that says 99 % would be a bar that lies.
    for (done, total, filled, percent) in [
        (0, 100, 0, 0),
        (1, 100, 1, 1),
        (50, 100, 10, 50),
        (99, 100, 19, 99),
        (100, 100, 20, 100),
    ] {
        let b = Bar::of(done, total, 20);
        assert_eq!(b.cells, 20, "the cell count is the one asked for");
        assert_eq!(b.filled, filled, "{percent} % fills {filled} of 20 cells");
        assert_eq!(b.percent, percent);
    }
}

#[test]
fn the_percentage_printed_is_the_filled_share() {
    let theme = Theme::ansi();
    for total in [1u64, 7, 100, 258] {
        for done in 0..=total {
            let parts = bar::line(&theme, Ramp::Measure, done, total, 60);
            let shown = text(&parts);
            let cells = glyphs(&parts, &theme);
            let filled = cells.iter().filter(|c| **c == theme.cells().full).count();
            let percent: u64 = shown
                .trim()
                .rsplit(' ')
                .next()
                .and_then(|p| p.strip_suffix('%'))
                .and_then(|p| p.parse().ok())
                .unwrap_or_else(|| panic!("no percentage in {shown:?}"));
            assert_eq!(percent, done * 100 / total, "{done}/{total}: {shown:?}");
            // The filled share, to within the cell it is rounded up to.
            let share = filled as u64 * 100 / cells.len() as u64;
            assert!(
                share.abs_diff(percent) <= 100 / cells.len() as u64 + 1,
                "{done}/{total}: {filled} of {} cells is {share} %, printed {percent} %",
                cells.len()
            );
        }
    }
}

#[test]
fn filled_cells_come_first_and_unfilled_ones_last() {
    let theme = Theme::ansi();
    let parts = bar::line(&theme, Ramp::Measure, 3, 10, 40);
    let c = theme.cells();
    let cells = glyphs(&parts, &theme);
    let first_empty = cells
        .iter()
        .position(|g| *g == c.empty)
        .expect("some empty");
    assert!(cells[..first_empty].iter().all(|g| *g == c.full));
    assert!(cells[first_empty..].iter().all(|g| *g == c.empty));
}

#[test]
fn the_bar_is_never_cut_mid_cell_and_never_wider_than_the_room() {
    let theme = Theme::ansi();
    let mut previous = 0;
    for width in 0..=200 {
        let parts = bar::line(&theme, Ramp::Measure, 1, 2, width);
        let drawn: usize = text(&parts).chars().count();
        assert!(drawn <= width.max(5), "{drawn} columns in {width}");
        let n = glyphs(&parts, &theme).len();
        assert!(n >= previous || n == 0, "a wider room drew fewer cells");
        previous = n;
        // Either whole cells and the percentage, or the percentage alone.
        assert!(n == 0 || n >= 4, "{n} cells is a stump, not a bar");
        assert!(n <= bar::MAX_CELLS);
        assert!(text(&parts).trim_end().ends_with('%'));
    }
}

#[test]
fn eighty_and_two_hundred_columns_both_get_a_bar() {
    let theme = Theme::ansi();
    for width in [80, 200] {
        let parts = bar::line(&theme, Ramp::Measure, 1, 2, width - 4);
        assert!(glyphs(&parts, &theme).len() >= 20, "at {width} columns");
    }
}

#[test]
fn the_filled_cells_run_through_the_ramp_and_the_unfilled_are_muted() {
    for theme in [Theme::neon(), Theme::ansi()] {
        let parts = bar::line(&theme, Ramp::Measure, 1, 2, 40);
        let c = theme.cells();
        let styles = |glyph: char| -> Vec<Style> {
            parts
                .iter()
                .filter(|(t, _)| t.chars().all(|ch| ch == glyph) && !t.is_empty())
                .map(|(_, s)| *s)
                .collect()
        };
        let filled = styles(c.full);
        assert!(filled.len() >= 2, "filled cells are drawn one by one");
        assert_ne!(
            filled.first(),
            filled.last(),
            "the ramp starts and ends on different colours"
        );
        assert!(
            styles(c.empty).iter().all(|s| *s == theme.muted),
            "unfilled cells are the muted role"
        );
    }
}

#[test]
fn the_danger_ramp_is_not_the_measuring_one() {
    let theme = Theme::neon();
    assert_ne!(
        theme.ramp(Ramp::Danger, 0, 10),
        theme.ramp(Ramp::Measure, 0, 10)
    );
    assert_ne!(
        theme.ramp(Ramp::Danger, 9, 10),
        theme.ramp(Ramp::Measure, 9, 10)
    );
}

#[test]
fn the_three_glyphs_differ_so_no_meaning_rests_on_colour() {
    let theme = Theme::mono();
    let c = theme.cells();
    assert_ne!(c.full, c.empty);
    assert_ne!(c.mark, c.empty);
    assert_ne!(
        c.mark, c.full,
        "the part a gauge is about needs its own glyph"
    );
}

#[test]
fn a_console_that_cannot_draw_the_cells_gets_brackets_and_hashes() {
    let theme = Theme::ansi().for_term(Some("linux"));
    let parts = bar::line(&theme, Ramp::Measure, 1, 2, 30);
    let shown = text(&parts);
    assert!(shown.starts_with('['), "{shown:?}");
    assert!(shown.contains("]"), "{shown:?}");
    assert!(shown.contains('#') && shown.contains('-'), "{shown:?}");
    assert!(shown.is_ascii(), "{shown:?}");
    assert_eq!(
        Theme::ansi().for_term(Some("xterm-256color")).cells(),
        Theme::ansi().cells()
    );
}

mod roles {
    //! #133's contrast test, extended to the roles the bar adds.

    use super::*;

    use super::common;
    use common::contrast::{contrast, rgb};
    use dev_cleaner::tui::palette::{self, Mode};
    use ratatui::buffer::Buffer;
    use ratatui::layout::Rect;
    use ratatui::style::Color;

    fn drawn(theme: &Theme, ramp: Ramp) -> Buffer {
        let area = Rect::new(0, 0, 60, 3);
        let mut buf = Buffer::empty(area);
        buf.set_style(area, theme.ground);
        let mut x = 0;
        for (text, style) in bar::line(theme, ramp, 1, 2, 50) {
            buf.set_string(x, 1, &text, style);
            x += text.chars().count() as u16;
        }
        buf
    }

    #[test]
    fn every_cell_of_both_ramps_reads_on_the_neon_ground() {
        let theme = Theme::neon();
        for ramp in [Ramp::Measure, Ramp::Danger] {
            for i in 0..bar::MAX_CELLS {
                let ratio = contrast(
                    rgb(theme.ramp(ramp, i, bar::MAX_CELLS).fg),
                    palette::GROUND_RGB,
                );
                assert!(
                    ratio >= 4.5,
                    "{ramp:?} cell {i} is {ratio:.2}:1 on the ground"
                );
            }
        }
    }

    #[test]
    fn a_whole_bar_in_neon_has_a_reading_colour_on_every_cell() {
        let theme = Theme::neon();
        for ramp in [Ramp::Measure, Ramp::Danger] {
            common::contrast::assert_readable(&drawn(&theme, ramp), &format!("{ramp:?} bar"));
        }
    }

    #[test]
    fn on_a_256_colour_terminal_a_bar_is_drawn_in_named_colours() {
        let theme = Theme::choose(None, Some("256color"));
        assert_eq!(theme.mode(), Mode::Ansi);
        for ramp in [Ramp::Measure, Ramp::Danger] {
            let buf = drawn(&theme, ramp);
            assert!(
                buf.content
                    .iter()
                    .all(|c| !matches!(c.fg, Color::Rgb(..) | Color::Indexed(_))),
                "{ramp:?}: a 256-colour terminal is handed only the named colours"
            );
        }
    }

    #[test]
    fn under_no_colour_a_bar_sets_no_colour_and_still_shows_how_far_it_is() {
        let theme = Theme::choose(Some("1"), Some("truecolor"));
        assert_eq!(theme.mode(), Mode::Mono);
        for ramp in [Ramp::Measure, Ramp::Danger] {
            let buf = drawn(&theme, ramp);
            assert!(
                buf.content
                    .iter()
                    .all(|c| c.fg == Color::Reset && c.bg == Color::Reset)
            );
            let row: String = (0..buf.area.width).map(|x| buf[(x, 1)].symbol()).collect();
            // The cells say how far, and so does the number.
            assert!(
                row.contains('▰') && row.contains('▱') && row.contains("50%"),
                "{row}"
            );
        }
    }
}
