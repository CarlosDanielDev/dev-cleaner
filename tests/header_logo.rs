//! The header carries the logo (#138, #141): the owner's pixel art drawn in
//! half blocks, two inks to a cell, on every screen tall enough to give it its
//! rows and never over a word of text.

pub mod common;

use std::time::Instant;

use common::Fixture;
use common::contrast::{contrast, rgb};
use dev_cleaner::config::Config;
use dev_cleaner::tui::logo::{HEIGHT, MIN_COLS, MIN_ROWS, WIDTH};
use dev_cleaner::tui::{
    KeyPress, Screen, Tui, collect,
    palette::{GROUND_RGB, Theme},
};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Color;

/// Glyphs a half-block logo is drawn in, and the lighter ones of the mono look.
const HALVES: [&str; 5] = ["▀", "▄", "█", "▒", "░"];

/// The rows the body keeps on the smallest terminal there is, 80x24: all the
/// screens are laid out for that, so a taller header must not take them.
const BODY_AT_MINIMUM: u16 = 24 - 2 - 2;

/// A tree with one project that has something to remove.
fn fixture() -> (Fixture, Fixture) {
    let fx = Fixture::new();
    let store = Fixture::new();
    fx.file("app/package.json", b"{}");
    fx.file("app/src/index.js", b"console.log(1)");
    fx.file("app/node_modules/dep/blob.bin", &vec![0xABu8; 4096]);
    (fx, store)
}

/// A driver on `screen`, in `theme`, with everything marked.
fn driver(fx: &Fixture, store: &Fixture, screen: Screen, theme: Theme) -> Tui {
    let cfg = Config {
        roots: vec![fx.root().to_path_buf()],
        caches: Vec::new(),
        denylist: Vec::new(),
    };
    let screens = collect(
        &[fx.root().to_path_buf()],
        &cfg,
        fx.root(),
        &store.root().join("history.sqlite3"),
    );
    let mut tui = Tui::new(screens).with_theme(theme);
    let now = Instant::now();
    while tui.app().screen() != screen {
        if tui.app().screen() == Screen::Candidates {
            tui.press(KeyPress::Char('a'), now);
        }
        tui.press(KeyPress::Enter, now);
    }
    tui
}

fn frame(tui: &mut Tui, cols: u16, rows: u16) -> Buffer {
    let area = Rect::new(0, 0, cols, rows);
    let mut buf = Buffer::empty(area);
    tui.render(area, &mut buf);
    buf
}

/// The column the logo starts at when it is drawn: against the right edge,
/// with one column of margin.
fn logo_x(cols: u16) -> u16 {
    cols - WIDTH - 1
}

/// Cells of the header rows that hold a half block, as `(x, y)`.
fn logo_cells(buf: &Buffer) -> Vec<(u16, u16)> {
    let mut cells = Vec::new();
    for y in 0..buf.area.height.min(HEIGHT) {
        for x in 0..buf.area.width {
            if HALVES.contains(&buf[(x, y)].symbol()) {
                cells.push((x, y));
            }
        }
    }
    cells
}

fn row(buf: &Buffer, y: u16, from: u16, to: u16) -> String {
    (from..to).map(|x| buf[(x, y)].symbol()).collect()
}

const CHROME_SCREENS: [Screen; 4] = [
    Screen::Dashboard,
    Screen::Projects,
    Screen::Candidates,
    Screen::Review,
];

#[test]
fn the_logo_is_drawn_only_where_the_header_can_grow_to_it() {
    let (fx, store) = fixture();
    assert_eq!((MIN_COLS, MIN_ROWS), (100, 34), "the thresholds moved");
    for screen in [Screen::Dashboard, Screen::Projects, Screen::Candidates] {
        for (cols, rows, shown) in [
            (80, 24, false),
            (90, 30, false),
            (99, 40, false),
            (100, 33, false),
            (100, 34, true),
            (100, 40, true),
            (120, 50, true),
        ] {
            let mut tui = driver(&fx, &store, screen, Theme::neon());
            let buf = frame(&mut tui, cols, rows);
            let cells = logo_cells(&buf);
            assert_eq!(
                !cells.is_empty(),
                shown,
                "{screen:?} at {cols}x{rows}: logo shown is {shown}"
            );
            for (x, y) in &cells {
                assert!(*x >= logo_x(cols), "{screen:?} at {cols}x{rows}: x={x}");
                assert!(*y < HEIGHT, "{screen:?} at {cols}x{rows}: y={y}");
            }
        }
    }
}

#[test]
fn below_the_thresholds_the_header_is_the_two_rows_it_was_before_the_logo() {
    let (fx, store) = fixture();
    for screen in CHROME_SCREENS {
        let mut tui = driver(&fx, &store, screen, Theme::neon());
        let buf = frame(&mut tui, 90, 30);
        assert!(!row(&buf, 1, 0, 90).trim().is_empty(), "{screen:?}: way");
        assert!(
            !row(&buf, 2, 0, 90).trim().is_empty(),
            "{screen:?}: the body starts on row 2"
        );
    }
}

#[test]
fn the_header_grows_to_the_logo_and_the_body_keeps_what_it_had_at_the_minimum() {
    let (fx, store) = fixture();
    for screen in CHROME_SCREENS {
        for (cols, rows) in [(100, 40), (120, 50)] {
            let mut tui = driver(&fx, &store, screen, Theme::neon());
            let buf = frame(&mut tui, cols, rows);
            // Rows under the title and the way, left of the logo, are spare.
            for y in 2..HEIGHT {
                assert!(
                    row(&buf, y, 0, logo_x(cols)).trim().is_empty(),
                    "{screen:?} at {cols}x{rows}: row {y} of the header is spare"
                );
            }
            assert!(
                !row(&buf, HEIGHT, 0, cols).trim().is_empty(),
                "{screen:?} at {cols}x{rows}: the body starts on row {HEIGHT}"
            );
            let body = rows - HEIGHT - 2;
            assert!(body >= BODY_AT_MINIMUM, "{cols}x{rows}: {body} body rows");
        }
    }
    // The tightest terminal that gets the logo is the one that matters most.
    const { assert!(MIN_ROWS - HEIGHT - 2 >= BODY_AT_MINIMUM) };
}

#[test]
fn the_logo_never_overwrites_the_title_or_the_way() {
    let (fx, store) = fixture();
    for screen in CHROME_SCREENS {
        for (cols, rows) in [(100, 40), (120, 50)] {
            let mut tall = driver(&fx, &store, screen, Theme::neon());
            let with = frame(&mut tall, cols, rows);
            let mut short = driver(&fx, &store, screen, Theme::neon());
            let without = frame(&mut short, cols, 29);
            assert!(
                !logo_cells(&with).is_empty() || screen == Screen::Review,
                "{screen:?} at {cols}x{rows}: no logo"
            );
            assert!(logo_cells(&without).is_empty());
            // Every cell of the two header rows is as it is without the logo, up
            // to the title's rule, which stops short of the logo.
            for y in 0..2 {
                for x in 0..logo_x(cols) - 3 {
                    assert_eq!(
                        with[(x, y)].symbol(),
                        without[(x, y)].symbol(),
                        "{screen:?} at {cols}x{rows}: ({x}, {y}) changed"
                    );
                }
            }
        }
    }
}

#[test]
fn a_breadcrumb_that_reaches_the_logo_drops_it_and_keeps_every_word() {
    let (fx, store) = fixture();
    let mut tui = driver(&fx, &store, Screen::Review, Theme::neon());
    // The plan's way row is the longest there is: at 100 columns it runs under
    // where the logo would stand.
    let narrow = frame(&mut tui, 100, 40);
    assert!(
        logo_cells(&narrow).is_empty(),
        "the logo was drawn over the plan's breadcrumb"
    );
    assert!(
        row(&narrow, 1, 0, 100).contains("Enter → confirm"),
        "the breadcrumb lost its end: {:?}",
        row(&narrow, 1, 0, 100)
    );
    // The header keeps its height whether or not the logo is in it, so the
    // body does not jump when the way gets longer.
    assert!(row(&narrow, 2, 0, 100).trim().is_empty());
    assert!(!row(&narrow, HEIGHT, 0, 100).trim().is_empty());
    let wide = frame(&mut tui, 140, 40);
    assert!(!logo_cells(&wide).is_empty(), "room enough, and no logo");
}

#[test]
fn the_screen_that_removes_things_keeps_its_band_whole() {
    let (fx, store) = fixture();
    let mut tui = driver(&fx, &store, Screen::Confirm, Theme::ansi());
    let buf = frame(&mut tui, 120, 50);
    assert!(
        logo_cells(&buf).is_empty(),
        "the logo is on the warning band"
    );
    assert!(row(&buf, 0, 0, 120).starts_with(' '));
}

/// The logo's cells, as `(x, y, symbol)`, to compare shapes across looks.
fn shape(buf: &Buffer) -> Vec<(u16, u16, String)> {
    logo_cells(buf)
        .into_iter()
        .map(|(x, y)| (x, y, buf[(x, y)].symbol().to_string()))
        .collect()
}

#[test]
fn every_colour_mode_draws_a_can_by_shape_and_not_by_colour_alone() {
    let (fx, store) = fixture();
    let mut neon = driver(&fx, &store, Screen::Dashboard, Theme::neon());
    let mut ansi = driver(&fx, &store, Screen::Dashboard, Theme::ansi());
    let mut mono = driver(&fx, &store, Screen::Dashboard, Theme::mono());
    let neon = frame(&mut neon, 120, 50);
    let ansi = frame(&mut ansi, 120, 50);
    let mono = frame(&mut mono, 120, 50);

    // The same cells and glyphs in both colour looks; the same cells in mono.
    assert_eq!(shape(&neon), shape(&ansi));
    assert_eq!(logo_cells(&neon), logo_cells(&mono));
    assert!(
        shape(&mono).len() >= 60,
        "a can is many cells: {}",
        shape(&mono).len()
    );

    let colours = |buf: &Buffer| -> Vec<Color> {
        let mut inks: Vec<Color> = logo_cells(buf)
            .iter()
            .flat_map(|&p| [buf[p].fg, buf[p].bg])
            .filter(|c| {
                *c != Color::Reset && *c != Color::Rgb(GROUND_RGB.0, GROUND_RGB.1, GROUND_RGB.2)
            })
            .collect();
        inks.sort_by_key(|c| format!("{c:?}"));
        inks.dedup();
        inks
    };
    assert_eq!(colours(&ansi), vec![Color::Cyan, Color::Magenta]);
    assert_eq!(colours(&neon).len(), 2, "{:?}", colours(&neon));
    // No colour at all under NO_COLOR.
    assert!(colours(&mono).is_empty());
    for &p in &logo_cells(&mono) {
        assert_eq!(mono[p].fg, Color::Reset);
        assert_eq!(mono[p].bg, Color::Reset, "NO_COLOR painted a background");
    }
    // The can is the full block and the mark a lighter glyph, so one weight
    // still shows a can with something in it.
    let glyphs: std::collections::HashSet<_> =
        shape(&mono).into_iter().map(|(_, _, g)| g).collect();
    assert!(glyphs.contains("█"), "{glyphs:?}");
    assert!(glyphs.contains("▒") || glyphs.contains("░"), "{glyphs:?}");
}

#[test]
fn a_cell_carries_two_inks_and_every_ink_reads_against_the_ground() {
    let (fx, store) = fixture();
    let mut tui = driver(&fx, &store, Screen::Dashboard, Theme::neon());
    let buf = frame(&mut tui, 120, 50);
    let cells = logo_cells(&buf);
    assert!(!cells.is_empty(), "no logo to measure");
    let mut two_inks = 0;
    for p in cells {
        for colour in [buf[p].fg, buf[p].bg] {
            if rgb(Some(colour)) == GROUND_RGB {
                continue;
            }
            let ratio = contrast(rgb(Some(colour)), GROUND_RGB);
            assert!(ratio >= 3.0, "{p:?}: {ratio:.2}:1");
        }
        if rgb(Some(buf[p].bg)) != GROUND_RGB && buf[p].fg != buf[p].bg {
            two_inks += 1;
        }
    }
    assert!(two_inks > 0, "no cell holds two inks");
}
