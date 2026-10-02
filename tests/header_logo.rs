//! The header carries the logo (#138): the owner's pixel art, shrunk to half
//! blocks, on every screen big enough for a third header row and never over a
//! word of text.

pub mod common;

use std::time::Instant;

use common::Fixture;
use common::contrast::{contrast, rgb};
use dev_cleaner::config::Config;
use dev_cleaner::tui::logo::{HEIGHT, WIDTH};
use dev_cleaner::tui::{
    KeyPress, Screen, Tui, collect,
    palette::{GROUND_RGB, Theme},
};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Color;

/// Glyphs a half-block logo is drawn in.
const HALVES: [&str; 4] = ["▀", "▄", "█", "▒"];

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
fn the_logo_is_drawn_from_ninety_by_thirty_and_not_below() {
    let (fx, store) = fixture();
    // Dashboard, projects and candidates all have a short enough way row for
    // the logo to fit beside it at the minimum width.
    for screen in [Screen::Dashboard, Screen::Projects, Screen::Candidates] {
        for (cols, rows, shown) in [
            (80, 24, false),
            (89, 30, false),
            (90, 29, false),
            (90, 30, true),
            (120, 40, true),
        ] {
            let mut tui = driver(&fx, &store, screen, Theme::neon());
            let buf = frame(&mut tui, cols, rows);
            let cells = logo_cells(&buf);
            assert_eq!(
                !cells.is_empty(),
                shown,
                "{screen:?} at {cols}x{rows}: logo shown is {shown}"
            );
            for (x, _) in &cells {
                assert!(*x >= logo_x(cols), "{screen:?} at {cols}x{rows}: x={x}");
            }
        }
    }
}

#[test]
fn the_header_is_three_rows_when_the_logo_has_room() {
    let (fx, store) = fixture();
    for screen in CHROME_SCREENS {
        let mut tui = driver(&fx, &store, screen, Theme::neon());
        let buf = frame(&mut tui, 120, 40);
        // The spare row: nothing of the title or the way sits on it, and the
        // body starts under it.
        assert!(
            row(&buf, 2, 0, logo_x(120)).trim().is_empty(),
            "{screen:?}: the third header row is spare"
        );
        assert!(
            !row(&buf, 3, 0, 120).trim().is_empty(),
            "{screen:?}: the body starts on row 3"
        );
    }
}

#[test]
fn the_logo_never_overwrites_the_title_or_the_way() {
    let (fx, store) = fixture();
    for screen in CHROME_SCREENS {
        let mut tall = driver(&fx, &store, screen, Theme::neon());
        let with = frame(&mut tall, 120, 40);
        let mut short = driver(&fx, &store, screen, Theme::neon());
        let without = frame(&mut short, 120, 29);
        assert!(!logo_cells(&with).is_empty(), "{screen:?}: no logo");
        assert!(
            logo_cells(&without).is_empty(),
            "{screen:?}: logo at 120x29"
        );
        let stop = logo_x(120) - 2;
        for y in 0..2 {
            assert_eq!(
                row(&with, y, 0, stop),
                row(&without, y, 0, stop),
                "{screen:?} row {y}: text left of the logo changed"
            );
        }
    }
}

#[test]
fn a_breadcrumb_that_reaches_the_logo_drops_it_and_keeps_every_word() {
    let (fx, store) = fixture();
    let mut tui = driver(&fx, &store, Screen::Review, Theme::neon());
    // The plan's way row is the longest there is: at the minimum width it
    // runs under where the logo would stand.
    let narrow = frame(&mut tui, 90, 30);
    assert!(
        logo_cells(&narrow).is_empty(),
        "the logo was drawn over the plan's breadcrumb"
    );
    assert!(
        row(&narrow, 1, 0, 90).contains("Enter → confirm"),
        "the breadcrumb lost its end: {:?}",
        row(&narrow, 1, 0, 90)
    );
    // The header keeps its height whether or not the logo is in it, so the
    // body does not jump when the way gets longer.
    assert!(row(&narrow, 2, 0, 90).trim().is_empty());
    let wide = frame(&mut tui, 120, 40);
    assert!(!logo_cells(&wide).is_empty(), "room enough, and no logo");
}

#[test]
fn the_screen_that_removes_things_keeps_its_band_whole() {
    let (fx, store) = fixture();
    let mut tui = driver(&fx, &store, Screen::Confirm, Theme::ansi());
    let buf = frame(&mut tui, 120, 40);
    assert!(
        logo_cells(&buf).is_empty(),
        "the logo is on the warning band"
    );
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
    let neon = frame(&mut neon, 120, 40);
    let ansi = frame(&mut ansi, 120, 40);
    let mono = frame(&mut mono, 120, 40);

    // The same cells, whatever the colour, and the same glyph in both of the
    // colour looks.
    assert_eq!(shape(&neon), shape(&ansi));
    let cells = |buf: &Buffer| logo_cells(buf);
    assert_eq!(cells(&neon), cells(&mono));
    assert!(
        shape(&mono).len() >= 18,
        "a can is more than a few cells: {}",
        shape(&mono).len()
    );

    let fgs = |buf: &Buffer| -> Vec<Color> {
        let mut inks: Vec<Color> = logo_cells(buf).iter().map(|&p| buf[p].fg).collect();
        inks.sort_by_key(|c| format!("{c:?}"));
        inks.dedup();
        inks
    };
    // Two inks in colour: the can and the mark inside it.
    assert_eq!(fgs(&ansi), vec![Color::Cyan, Color::Magenta]);
    let neon_inks = fgs(&neon);
    assert_eq!(neon_inks.len(), 2, "{neon_inks:?}");
    // No colour at all under NO_COLOR.
    assert_eq!(fgs(&mono), vec![Color::Reset]);
    for &p in &logo_cells(&mono) {
        assert_eq!(mono[p].bg, Color::Reset, "NO_COLOR painted a background");
    }
    // The mark inside the can is told apart from the can by a glyph, so one
    // weight still shows a can with something in it.
    let glyphs: std::collections::HashSet<_> =
        shape(&mono).into_iter().map(|(_, _, g)| g).collect();
    assert!(glyphs.len() >= 2, "one glyph: {glyphs:?}");
}

#[test]
fn the_neon_logo_reads_against_the_ground() {
    let (fx, store) = fixture();
    let mut tui = driver(&fx, &store, Screen::Dashboard, Theme::neon());
    let buf = frame(&mut tui, 120, 40);
    assert!(!logo_cells(&buf).is_empty(), "no logo to measure");
    for p in logo_cells(&buf) {
        let ratio = contrast(rgb(Some(buf[p].fg)), GROUND_RGB);
        assert!(ratio >= 4.5, "{p:?}: {ratio:.2}:1");
        assert_eq!(rgb(Some(buf[p].bg)), GROUND_RGB, "{p:?}: not the ground");
    }
}
