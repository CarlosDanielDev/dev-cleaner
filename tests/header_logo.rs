//! The header is an icon beside a styled name (#143): a small mark, drawn
//! whole or not at all, at the left edge, with the wordmark on its centre line
//! and the way under it; and the wordmark, a gradient of the logo's two inks,
//! wherever the header is.

pub mod common;

use std::time::Instant;

use common::Fixture;
use common::contrast::{contrast, rgb};
use dev_cleaner::config::Config;
use dev_cleaner::tui::logo::{GAP, HEIGHT, ICON, MIN_COLS, MIN_ROWS, WIDTH};
use dev_cleaner::tui::{
    KeyPress, Screen, Tui, collect,
    palette::{GROUND_RGB, Theme},
};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier};

/// Glyphs a half-block logo is drawn in, and the lighter ones of the mono look.
const HALVES: [&str; 5] = ["▀", "▄", "█", "▒", "░"];

/// The rows the body keeps on the smallest terminal there is, 80x24: all the
/// screens are laid out for that, so a taller header must not take them.
const BODY_AT_MINIMUM: u16 = 24 - 2 - 2;

/// Where the icon starts, and where the wordmark and the way start.
const ICON_X: u16 = 1;
const TEXT_X: u16 = ICON_X + WIDTH + GAP;

/// The sizes the issue names, and the wordmark's letters.
const SIZES: [(u16, u16); 4] = [(80, 24), (90, 28), (100, 34), (120, 40)];
const LETTERS: &str = "dev-cleaner";

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

/// The cells the whole icon holds ink in, as the icon's own rows say.
fn whole_icon() -> Vec<(u16, u16)> {
    let mut cells = Vec::new();
    for (row, pair) in ICON.chunks(2).enumerate() {
        for col in 0..pair[0].len() {
            if pair.iter().any(|r| r.as_bytes()[col] != b'.') {
                cells.push((ICON_X + col as u16, row as u16));
            }
        }
    }
    cells
}

fn row(buf: &Buffer, y: u16, from: u16, to: u16) -> String {
    (from..to).map(|x| buf[(x, y)].symbol()).collect()
}

/// The column the first non-blank cell of row `y` is in, past `from`.
fn left_edge(buf: &Buffer, y: u16, from: u16) -> u16 {
    (from..buf.area.width)
        .find(|&x| buf[(x, y)].symbol() != " ")
        .unwrap_or(u16::MAX)
}

/// The columns of the wordmark's letters on row `y`, which starts at `from`.
fn letters_at(buf: &Buffer, y: u16, from: u16) -> Vec<(u16, u16)> {
    assert_eq!(
        row(buf, y, from, from + 11),
        LETTERS,
        "no wordmark on row {y}"
    );
    (from..from + 11)
        .enumerate()
        .filter(|&(i, _)| LETTERS.as_bytes()[i] != b'-')
        .map(|(_, x)| (x, y))
        .collect()
}

const CHROME_SCREENS: [Screen; 4] = [
    Screen::Dashboard,
    Screen::Projects,
    Screen::Candidates,
    Screen::Review,
];

#[test]
fn the_thresholds_are_named_and_the_icon_is_square() {
    assert_eq!((MIN_COLS, MIN_ROWS), (90, 28), "the thresholds moved");
    // A text cell is two pixel rows high, so the columns the icon takes are
    // the pixels it is tall, and the rows are half of them.
    assert_eq!(ICON.len(), 2 * HEIGHT as usize);
    assert!(ICON.iter().all(|r| r.len() == WIDTH as usize));
    assert_eq!(WIDTH as usize, ICON.len());
    // The header at the threshold is the icon's height and leaves the body what
    // every screen is laid out for.
    const { assert!(MIN_ROWS - HEIGHT - 2 >= BODY_AT_MINIMUM) };
}

#[test]
fn the_icon_is_drawn_whole_and_only_where_the_header_can_grow_to_it() {
    let (fx, store) = fixture();
    for screen in [Screen::Dashboard, Screen::Projects, Screen::Candidates] {
        for (cols, rows, shown) in [
            (80, 24, false),
            (89, 40, false),
            (90, 27, false),
            (90, 28, true),
            (100, 34, true),
            (120, 40, true),
            (200, 60, true),
        ] {
            let mut tui = driver(&fx, &store, screen, Theme::neon());
            let buf = frame(&mut tui, cols, rows);
            let cells = logo_cells(&buf);
            if shown {
                assert_eq!(cells, whole_icon(), "{screen:?} at {cols}x{rows}");
            } else {
                assert!(cells.is_empty(), "{screen:?} at {cols}x{rows}: {cells:?}");
            }
        }
    }
}

#[test]
fn every_size_the_issue_names_draws_its_icon_by_the_size() {
    let (fx, store) = fixture();
    for (cols, rows) in SIZES {
        let mut tui = driver(&fx, &store, Screen::Dashboard, Theme::neon());
        let buf = frame(&mut tui, cols, rows);
        let wanted = cols >= MIN_COLS && rows >= MIN_ROWS;
        assert_eq!(!logo_cells(&buf).is_empty(), wanted, "{cols}x{rows}");
    }
}

#[test]
fn the_wordmark_sits_on_the_icons_centre_line_and_the_way_starts_where_it_does() {
    let (fx, store) = fixture();
    for screen in CHROME_SCREENS {
        for (cols, rows) in [(90, 28), (100, 34), (120, 40)] {
            let mut tui = driver(&fx, &store, screen, Theme::neon());
            let buf = frame(&mut tui, cols, rows);
            // The plan's way is the longest there is; see the test below.
            if screen == Screen::Review && cols == 90 {
                continue;
            }
            assert!(!logo_cells(&buf).is_empty(), "{screen:?} {cols}x{rows}");
            let centre = HEIGHT / 2;
            assert_eq!(row(&buf, centre, TEXT_X, TEXT_X + 11), LETTERS);
            assert_eq!(
                left_edge(&buf, centre + 1, ICON_X + WIDTH),
                TEXT_X,
                "{screen:?}: the way"
            );
            // Nothing but the icon, and the gap, left of the text on the rows
            // the icon is on.
            for y in 0..HEIGHT {
                let wrong: Vec<u16> = (0..TEXT_X)
                    .filter(|&x| {
                        let s = buf[(x, y)].symbol();
                        s != " " && !HALVES.contains(&s)
                    })
                    .collect();
                assert!(
                    wrong.is_empty(),
                    "{screen:?} {cols}x{rows} row {y}: {wrong:?}"
                );
                assert_eq!(row(&buf, y, ICON_X + WIDTH, TEXT_X).trim(), "");
            }
        }
    }
}

#[test]
fn the_header_is_as_tall_as_the_icon_and_the_body_keeps_what_it_had_at_the_minimum() {
    let (fx, store) = fixture();
    for screen in CHROME_SCREENS {
        for (cols, rows) in [(90, 28), (100, 34), (120, 40)] {
            let mut tui = driver(&fx, &store, screen, Theme::neon());
            let buf = frame(&mut tui, cols, rows);
            // Under the title and the way there is nothing but the icon.
            for y in HEIGHT / 2 + 2..HEIGHT {
                assert!(
                    row(&buf, y, TEXT_X, cols).trim().is_empty(),
                    "{screen:?} at {cols}x{rows}: row {y} of the header is spare"
                );
            }
            assert!(
                !row(&buf, HEIGHT, 0, cols).trim().is_empty(),
                "{screen:?} at {cols}x{rows}: the body starts on row {HEIGHT}"
            );
            assert!(rows - HEIGHT - 2 >= BODY_AT_MINIMUM, "{cols}x{rows}");
        }
    }
}

#[test]
fn below_the_thresholds_the_header_is_the_two_rows_it_was_before_the_logo() {
    let (fx, store) = fixture();
    for screen in CHROME_SCREENS {
        for (cols, rows) in [(80, 24), (89, 40), (90, 27)] {
            let mut tui = driver(&fx, &store, screen, Theme::neon());
            let buf = frame(&mut tui, cols, rows);
            assert!(logo_cells(&buf).is_empty());
            assert_eq!(left_edge(&buf, 0, 0), 1);
            assert_eq!(left_edge(&buf, 1, 0), 1, "{screen:?}: the way");
            assert!(
                !row(&buf, 2, 0, cols).trim().is_empty(),
                "{screen:?} {cols}x{rows}: the body starts on row 2"
            );
        }
    }
}

#[test]
fn a_breadcrumb_that_does_not_fit_beside_the_icon_drops_it_and_keeps_every_word() {
    let (fx, store) = fixture();
    let mut tui = driver(&fx, &store, Screen::Review, Theme::neon());
    // At 90 columns the plan's way, the longest row there is, ends under where
    // the icon's column would put it past the edge.
    let narrow = frame(&mut tui, 90, 28);
    assert!(
        logo_cells(&narrow).is_empty(),
        "the icon pushed the way off"
    );
    assert!(
        row(&narrow, 1, 0, 90).contains("Enter → confirm"),
        "the breadcrumb lost its end: {:?}",
        row(&narrow, 1, 0, 90)
    );
    assert_eq!(left_edge(&narrow, 0, 0), 1);
    // The header keeps its height whether or not the icon is in it, so the
    // body does not jump when the way gets longer.
    for y in 2..HEIGHT {
        assert!(row(&narrow, y, 0, 90).trim().is_empty(), "row {y}");
    }
    assert!(!row(&narrow, HEIGHT, 0, 90).trim().is_empty());
    let wide = frame(&mut tui, 140, 40);
    assert!(!logo_cells(&wide).is_empty(), "room enough, and no icon");
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

/// The foreground of the wordmark's letters as RGB, left to right.
fn gradient(buf: &Buffer, letters: &[(u16, u16)]) -> Vec<(u8, u8, u8)> {
    letters.iter().map(|&p| rgb(Some(buf[p].fg))).collect()
}

#[test]
fn the_wordmark_runs_cyan_through_violet_to_magenta_under_truecolor() {
    let (fx, store) = fixture();
    // At the icon's size and below it: the gradient needs no height.
    for (cols, rows, y, x) in [(120, 40, 2, TEXT_X), (80, 24, 0, 1)] {
        let mut tui = driver(&fx, &store, Screen::Dashboard, Theme::neon());
        let buf = frame(&mut tui, cols, rows);
        let steps = gradient(&buf, &letters_at(&buf, y, x));
        assert_eq!(steps.len(), 10);
        assert_eq!(steps[0], (0x00, 0xe5, 0xff), "starts in the icon's cyan");
        assert_eq!(steps[9], (0xff, 0x2e, 0x97), "ends in the icon's magenta");
        for pair in steps.windows(2) {
            assert!(pair[1].0 >= pair[0].0, "red steps back: {steps:?}");
            assert!(pair[1].1 <= pair[0].1, "green steps back: {steps:?}");
        }
        assert!(
            steps.iter().collect::<std::collections::HashSet<_>>().len() >= 8,
            "a gradient has steps: {steps:?}"
        );
        for step in &steps {
            let ratio = contrast(*step, GROUND_RGB);
            assert!(ratio >= 3.0, "{step:?} is {ratio:.2}:1");
        }
        // The hyphen is the quieter one, and the letters are bold.
        let hyphen = buf[(x + 3, y)].clone();
        assert!(!hyphen.modifier.contains(Modifier::BOLD));
        assert_ne!(rgb(Some(hyphen.fg)), steps[3]);
        assert!(buf[(x, y)].modifier.contains(Modifier::BOLD));
    }
}

#[test]
fn the_wordmark_is_two_colours_under_256_colours() {
    let (fx, store) = fixture();
    let mut tui = driver(&fx, &store, Screen::Dashboard, Theme::ansi());
    let buf = frame(&mut tui, 120, 40);
    let colours: Vec<Color> = letters_at(&buf, 2, TEXT_X)
        .iter()
        .map(|&p| buf[p].fg)
        .collect();
    let (dev, cleaner) = colours.split_at(3);
    assert!(dev.iter().all(|&c| c == Color::Cyan), "{colours:?}");
    assert!(cleaner.iter().all(|&c| c == Color::Magenta), "{colours:?}");
}

#[test]
fn under_no_color_the_weight_carries_the_wordmark() {
    let (fx, store) = fixture();
    let mut tui = driver(&fx, &store, Screen::Dashboard, Theme::mono());
    let buf = frame(&mut tui, 120, 40);
    for p in letters_at(&buf, 2, TEXT_X) {
        assert_eq!(buf[p].fg, Color::Reset, "{p:?}");
        assert_eq!(buf[p].bg, Color::Reset, "{p:?}");
        assert!(buf[p].modifier.contains(Modifier::BOLD), "{p:?}");
    }
}

#[test]
fn the_screen_name_is_its_own_quieter_label_after_the_wordmark() {
    let (fx, store) = fixture();
    for screen in CHROME_SCREENS {
        let mut tui = driver(&fx, &store, screen, Theme::neon());
        let buf = frame(&mut tui, 120, 40);
        let line = row(&buf, 2, TEXT_X, 120);
        let wanted = format!("dev-cleaner  ·  {}", screen.name());
        assert!(line.starts_with(&wanted), "{line:?}");
        let name_at = TEXT_X + "dev-cleaner  ·  ".chars().count() as u16;
        let name = &buf[(name_at, 2)];
        assert!(!name.modifier.contains(Modifier::BOLD), "{screen:?}");
        assert_eq!(
            rgb(Some(name.fg)),
            (0xc8, 0xd3, 0xf5),
            "{screen:?}: text ink"
        );
    }
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
        shape(&mono).len() >= 25,
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
fn every_ink_of_the_icon_reads_against_the_ground() {
    let (fx, store) = fixture();
    let mut tui = driver(&fx, &store, Screen::Dashboard, Theme::neon());
    let buf = frame(&mut tui, 120, 50);
    let cells = logo_cells(&buf);
    assert!(!cells.is_empty(), "no logo to measure");
    for p in cells {
        for colour in [buf[p].fg, buf[p].bg] {
            if rgb(Some(colour)) == GROUND_RGB {
                continue;
            }
            let ratio = contrast(rgb(Some(colour)), GROUND_RGB);
            assert!(ratio >= 3.0, "{p:?}: {ratio:.2}:1");
        }
    }
}
