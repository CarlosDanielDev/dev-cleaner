//! The header is a composed band (#147): a braille icon at the left edge, the
//! status lines beside it and a rule that closes it (their rhythm is pinned in
//! `header_bar.rs`); and the wordmark, a gradient of the logo's two inks,
//! wherever the header is. The confirm and running screens draw the same band,
//! with the danger in a bar of its own.

pub mod common;

use std::time::Instant;

use common::Fixture;
use common::contrast::{contrast, rgb};
use dev_cleaner::config::Config;
use dev_cleaner::purge::Remover;
use dev_cleaner::tui::logo::{
    FALLBACK, FALLBACK_WIDTH, GAP, HEIGHT, ICON, MIN_COLS, MIN_ROWS, TOP, WIDTH,
};
use dev_cleaner::tui::{
    KeyPress, Screen, Tui, collect,
    palette::{GROUND_RGB, Theme},
};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier};

/// Glyphs a half-block logo is drawn in, and the lighter ones of the mono look.
const HALVES: [&str; 5] = ["▀", "▄", "█", "▒", "░"];

/// Whether `symbol` is a braille pattern with a dot in it.
fn is_braille(symbol: &str) -> bool {
    let mut chars = symbol.chars();
    matches!((chars.next(), chars.next()), (Some(c), None) if ('\u{2801}'..='\u{28ff}').contains(&c))
}

fn is_logo(symbol: &str) -> bool {
    is_braille(symbol) || HALVES.contains(&symbol)
}

/// The rows the body keeps on the smallest terminal there is, 80x24: all the
/// screens are laid out for that, so a taller header must not take them.
const BODY_AT_MINIMUM: u16 = 24 - 2 - 2;

/// The rows of the band: the title line with the wordmark, the stepper, the
/// hints, and the rule on the row under the icon.
const WORDMARK: u16 = 1;
const WAY_ROW: u16 = 3;
const RULE_ROW: u16 = HEIGHT;

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

/// Cells of the header rows that hold a piece of the logo, as `(x, y)`: the
/// columns left of the text, where the running screen's spinner is not.
fn logo_cells(buf: &Buffer) -> Vec<(u16, u16)> {
    let mut cells = Vec::new();
    for y in 0..buf.area.height.min(HEIGHT) {
        for x in 0..buf.area.width.min(TEXT_X) {
            if is_logo(buf[(x, y)].symbol()) {
                cells.push((x, y));
            }
        }
    }
    cells
}

/// The cells the whole braille icon holds a dot in, as the icon's own rows say.
fn whole_icon() -> Vec<(u16, u16)> {
    let mut cells = Vec::new();
    for (row, quad) in ICON.chunks(4).enumerate() {
        for col in 0..WIDTH as usize {
            if quad.iter().any(|r| r[2 * col..2 * col + 2] != *"..") {
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

/// The screens with a first section under the header, and the sizes at which
/// the issue wants the composition checked.
const TALL: [(u16, u16); 3] = [(90, 28), (100, 34), (120, 40)];

#[test]
fn the_thresholds_are_named_and_the_icon_keeps_its_pixels_square() {
    assert_eq!((MIN_COLS, MIN_ROWS), (90, 28), "the thresholds moved");
    // Braille: two dot columns and four dot rows to a cell, and a cell is half
    // as wide as it is tall.
    assert_eq!(ICON.len(), 4 * HEIGHT as usize);
    assert!(ICON.iter().all(|r| r.len() == 2 * WIDTH as usize));
    // Half blocks: two pixel rows to a cell, and one pixel to a column.
    assert_eq!(FALLBACK.len(), 2 * HEIGHT as usize);
    assert!(FALLBACK.iter().all(|r| r.len() == FALLBACK_WIDTH as usize));
    assert_eq!(FALLBACK_WIDTH as usize, FALLBACK.len());
    // Six rows above the body at the threshold leave it what every screen is
    // laid out for.
    assert_eq!(TOP, HEIGHT + 1);
    const { assert!(MIN_ROWS - TOP - 2 >= BODY_AT_MINIMUM) };
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
fn every_cell_of_the_icon_is_a_braille_pattern_with_a_dot_in_it() {
    let (fx, store) = fixture();
    let mut tui = driver(&fx, &store, Screen::Dashboard, Theme::neon());
    let buf = frame(&mut tui, 120, 40);
    for (x, y) in whole_icon() {
        let symbol = buf[(x, y)].symbol();
        assert!(is_braille(symbol), "({x},{y}) is {symbol:?}");
    }
    // Blank where the art has nothing: the ground shows through.
    for y in 0..HEIGHT {
        for x in ICON_X..ICON_X + WIDTH {
            if !whole_icon().contains(&(x, y)) {
                assert_eq!(buf[(x, y)].symbol(), " ", "({x},{y})");
            }
        }
    }
}

#[test]
fn where_the_braille_set_is_not_selected_the_half_block_icon_of_the_same_height_is_drawn() {
    let (fx, store) = fixture();
    for theme in [
        Theme::neon().ascii(),
        Theme::ansi().ascii(),
        Theme::mono().ascii(),
        Theme::neon().for_term(Some("linux")),
        Theme::neon().for_term(Some("dumb")),
        Theme::neon().icons_from(Some("ascii"), None),
    ] {
        let mut tui = driver(&fx, &store, Screen::Dashboard, theme);
        let buf = frame(&mut tui, 120, 40);
        let cells = logo_cells(&buf);
        assert!(cells.len() >= 25, "a can is many cells: {}", cells.len());
        for &(x, y) in &cells {
            let symbol = buf[(x, y)].symbol();
            assert!(HALVES.contains(&symbol), "({x},{y}) is {symbol:?}");
        }
        // The same five rows, and nothing wider than the fallback's columns.
        assert!(cells.iter().all(|&(x, y)| y < HEIGHT && x >= ICON_X));
        assert!(cells.iter().all(|&(x, _)| x < ICON_X + FALLBACK_WIDTH));
        assert!(cells.iter().any(|&(_, y)| y == 0) && cells.iter().any(|&(_, y)| y == HEIGHT - 1));
        // No cell of the whole header holds a placeholder: braille, a
        // replacement character or a private-use glyph.
        for y in 0..HEIGHT {
            for x in 0..120 {
                let symbol = buf[(x, y)].symbol();
                assert!(
                    !is_braille(symbol) && !symbol.contains('\u{fffd}') && !symbol.contains('?'),
                    "({x},{y}) is {symbol:?}"
                );
            }
        }
        // The text keeps the left edge the fallback's own width gives it.
        let text_x = ICON_X + FALLBACK_WIDTH + GAP;
        assert_eq!(row(&buf, WORDMARK, text_x, text_x + 11), LETTERS);
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
fn a_rule_closes_the_band_and_the_body_starts_under_it() {
    let (fx, store) = fixture();
    for screen in CHROME_SCREENS {
        for (cols, rows) in TALL {
            let mut tui = driver(&fx, &store, screen, Theme::neon());
            let buf = frame(&mut tui, cols, rows);
            let at = format!("{screen:?} {cols}x{rows}");
            assert_eq!(
                row(&buf, RULE_ROW, 0, cols),
                "─".repeat(cols as usize),
                "{at}"
            );
            assert_eq!(buf[(TEXT_X, RULE_ROW)].fg, Theme::neon().violet.fg.unwrap());
            assert!(!row(&buf, TOP, 0, cols).trim().is_empty(), "{at}: the body");
            assert!(rows - TOP - 2 >= BODY_AT_MINIMUM, "{at}");
        }
    }
}

#[test]
fn the_band_is_drawn_in_every_colour_mode_and_never_over_a_word() {
    let (fx, store) = fixture();
    for theme in [Theme::neon(), Theme::ansi(), Theme::mono()] {
        for screen in CHROME_SCREENS {
            let mut tui = driver(&fx, &store, screen, theme);
            let tall = frame(&mut tui, 120, 40);
            // The same text, wherever the icon is: the header costs the body
            // nothing but its own rows, so the rows under it are the rows a
            // taller terminal draws, and no row of the header holds two things.
            let compact = frame(&mut tui, 80, 24);
            assert!(!row(&compact, 1, 0, 80).trim().is_empty());
            let title = row(&tall, WORDMARK, TEXT_X, 120);
            assert!(
                title
                    .to_lowercase()
                    .contains(&format!("▸ {}", screen.name())),
                "{screen:?}: {title:?}"
            );
            let way = row(&tall, WAY_ROW, TEXT_X, 120);
            assert!(way.contains("Enter") || way.contains("Esc"), "{way:?}");
        }
    }
}

#[test]
fn below_the_thresholds_the_header_is_a_status_line_and_a_rule() {
    let (fx, store) = fixture();
    for screen in CHROME_SCREENS {
        for (cols, rows) in [(80, 24), (89, 40), (90, 27)] {
            let mut tui = driver(&fx, &store, screen, Theme::neon());
            let buf = frame(&mut tui, cols, rows);
            assert!(logo_cells(&buf).is_empty());
            assert_eq!(left_edge(&buf, 0, 0), 1);
            assert_eq!(row(&buf, 1, 0, cols), "─".repeat(cols as usize));
            assert!(
                !row(&buf, 2, 0, cols).trim().is_empty(),
                "{screen:?} {cols}x{rows}: the body starts on row 2"
            );
            let title = row(&buf, 0, 1, cols).to_lowercase();
            assert!(
                title.starts_with(&format!("dev-cleaner ▸ {}", screen.name())),
                "{title:?}"
            );
        }
    }
}

#[test]
fn a_hint_line_that_does_not_fit_beside_the_icon_loses_its_facts_before_the_icon() {
    let (fx, store) = fixture();
    let mut tui = driver(&fx, &store, Screen::Review, Theme::neon());
    // At 90 columns the plan's way, the longest there is, ends past the edge
    // where the icon's column would put it: its keys stay, the sentence goes.
    let narrow = frame(&mut tui, 90, 28);
    assert!(!logo_cells(&narrow).is_empty(), "the icon went first");
    let way = row(&narrow, WAY_ROW, TEXT_X, 90);
    assert!(
        way.contains("Esc ← candidates") && way.contains("Enter → confirm"),
        "{way:?}"
    );
    assert!(!way.contains("built from"), "{way:?}");
    let wide = frame(&mut tui, 100, 34);
    assert!(row(&wide, WAY_ROW, TEXT_X, 100).contains("built from the 1 you marked"));
}

#[test]
fn a_line_of_facts_with_no_key_in_it_drops_the_icon_and_keeps_every_word() {
    for (title, mut tui, _fx, _store) in danger_screens(Theme::neon()) {
        if title != "Purging" {
            continue;
        }
        let narrow = frame(&mut tui, 90, 28);
        assert!(
            logo_cells(&narrow).is_empty(),
            "the icon pushed the way off"
        );
        assert!(
            row(&narrow, 1, 0, 90).contains("items move"),
            "{:?}",
            row(&narrow, 1, 0, 90)
        );
    }
}

/// The foreground of the wordmark's letters as RGB, left to right.
fn gradient(buf: &Buffer, letters: &[(u16, u16)]) -> Vec<(u8, u8, u8)> {
    letters.iter().map(|&p| rgb(Some(buf[p].fg))).collect()
}

#[test]
fn the_wordmark_runs_cyan_through_violet_to_magenta_under_truecolor() {
    let (fx, store) = fixture();
    // At the icon's size and below it: the gradient needs no height.
    for (cols, rows, y, x) in [(120, 40, WORDMARK, TEXT_X), (80, 24, 0, 1)] {
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
    let colours: Vec<Color> = letters_at(&buf, WORDMARK, TEXT_X)
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
    for p in letters_at(&buf, WORDMARK, TEXT_X) {
        assert_eq!(buf[p].fg, Color::Reset, "{p:?}");
        assert_eq!(buf[p].bg, Color::Reset, "{p:?}");
        assert!(buf[p].modifier.contains(Modifier::BOLD), "{p:?}");
    }
}

#[test]
fn the_screen_name_is_bold_in_the_text_ink_after_the_wordmark() {
    let (fx, store) = fixture();
    for screen in CHROME_SCREENS {
        let mut tui = driver(&fx, &store, screen, Theme::neon());
        let buf = frame(&mut tui, 120, 40);
        let line = row(&buf, WORDMARK, TEXT_X, 120);
        let at = line.find('▸').unwrap();
        let col = TEXT_X + line[..at].chars().count() as u16 + 2;
        let name = &buf[(col, WORDMARK)];
        assert!(name.modifier.contains(Modifier::BOLD), "{screen:?}");
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

    // The same cells and glyphs in all three looks: the shape is the dots.
    assert_eq!(shape(&neon), shape(&ansi));
    assert_eq!(shape(&neon), shape(&mono));
    assert!(
        shape(&mono).len() >= 40,
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
    // No colour at all under NO_COLOR, and no background anywhere.
    assert!(colours(&mono).is_empty());
    for &p in &logo_cells(&mono) {
        assert_eq!(mono[p].fg, Color::Reset);
        assert_eq!(mono[p].bg, Color::Reset, "NO_COLOR painted a background");
    }
}

#[test]
fn the_fallback_still_draws_a_can_by_weight_under_no_color() {
    let (fx, store) = fixture();
    let mut tui = driver(&fx, &store, Screen::Dashboard, Theme::mono().ascii());
    let mono = frame(&mut tui, 120, 50);
    let glyphs: std::collections::HashSet<_> =
        shape(&mono).into_iter().map(|(_, _, g)| g).collect();
    assert!(glyphs.contains("█"), "{glyphs:?}");
    assert!(glyphs.contains("▒") || glyphs.contains("░"), "{glyphs:?}");
    for &p in &logo_cells(&mono) {
        assert_eq!(mono[p].fg, Color::Reset);
        assert_eq!(mono[p].bg, Color::Reset);
    }
}

#[test]
fn every_ink_of_the_icon_reads_against_the_ground() {
    let (fx, store) = fixture();
    for theme in [Theme::neon(), Theme::neon().ascii()] {
        let mut tui = driver(&fx, &store, Screen::Dashboard, theme);
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
}

/// A remover that removes nothing: the run is drawn, never carried out.
struct Nothing;

impl Remover for Nothing {
    fn remove(&self, path: &std::path::Path) -> std::io::Result<std::path::PathBuf> {
        Ok(path.to_path_buf())
    }
}

/// The confirm screen, and the running one a hold later, in `theme`.
fn danger_screens(theme: Theme) -> Vec<(&'static str, Tui, Fixture, Fixture)> {
    let mut screens = Vec::new();
    for running in [false, true] {
        let (fx, store) = fixture();
        let mut tui = driver(&fx, &store, Screen::Confirm, theme);
        if running {
            let records = Fixture::new();
            tui = tui.with_manifest_dir(records.root().to_path_buf());
            let now = Instant::now();
            for repeat in 0..80u32 {
                let at = now + std::time::Duration::from_millis(50 * u64::from(repeat));
                if tui.press(dev_cleaner::tui::PURGE, at) == dev_cleaner::tui::Step::Purge {
                    break;
                }
            }
            tui.purge(Box::new(Nothing));
            // The records are written under a directory of this test's own.
            std::mem::forget(records);
        }
        screens.push((if running { "Purging" } else { "Confirm" }, tui, fx, store));
    }
    screens
}

#[test]
fn the_danger_screens_draw_the_same_band_with_a_red_bar_under_it() {
    let band = Theme::neon().warning_band;
    for (cols, rows) in TALL {
        for (title, mut tui, _fx, _store) in danger_screens(Theme::neon()) {
            let buf = frame(&mut tui, cols, rows);
            let at = format!("{title} {cols}x{rows}");
            // The wordmark is on the ground, whole; the icon is there unless
            // the way is too long for it, and then the band is the compact one.
            let icon = !logo_cells(&buf).is_empty();
            let bar_row = if icon { RULE_ROW } else { 0 };
            if icon {
                assert_eq!(logo_cells(&buf), whole_icon(), "{at}");
                assert_eq!(row(&buf, WORDMARK, TEXT_X, TEXT_X + 11), LETTERS, "{at}");
                assert_eq!(left_edge(&buf, WAY_ROW, ICON_X + WIDTH), TEXT_X, "{at}");
                assert!(row(&buf, WORDMARK, TEXT_X, cols).contains(title), "{at}");
            } else {
                assert!(
                    cols < 100 && title == "Purging",
                    "{at}: the icon is missing"
                );
            }
            // The bar spans the whole width, in the band's fill, and the title
            // is read from the buffer on it.
            let bar = row(&buf, bar_row, 0, cols);
            assert!(bar.contains(title), "{at}: {bar:?}");
            for x in 0..cols {
                let cell = &buf[(x, bar_row)];
                assert_eq!(
                    cell.bg,
                    band.bg.unwrap(),
                    "{at}: ({x},{bar_row}) is not on the band"
                );
                if cell.symbol().trim().is_empty() {
                    continue;
                }
                // Every glyph on the red keeps 4.5:1 against it, and none is
                // drawn in a colour of the brand.
                let ratio = contrast(rgb(Some(cell.fg)), rgb(band.bg));
                assert!(
                    ratio >= 4.5,
                    "{at}: ({x},{bar_row}) {:?} is {ratio:.2}:1",
                    cell.symbol()
                );
                assert!(
                    cell.modifier.contains(Modifier::BOLD),
                    "{at}: ({x},{bar_row})"
                );
            }
            // The body starts where it does everywhere else.
            if icon {
                assert!(!row(&buf, TOP, 0, cols).trim().is_empty(), "{at}");
            }
        }
    }
}

#[test]
fn the_compact_danger_band_is_legible_in_every_colour_mode() {
    for theme in [Theme::neon(), Theme::ansi(), Theme::mono()] {
        for (title, mut tui, _fx, _store) in danger_screens(theme) {
            let buf = frame(&mut tui, 80, 24);
            assert!(logo_cells(&buf).is_empty(), "{title}: {:?}", shape(&buf));
            let line = row(&buf, 0, 0, 80);
            assert!(
                line.contains("dev-cleaner") && line.contains(title),
                "{line:?}"
            );
            for x in 0..80 {
                let cell = &buf[(x, 0)];
                assert!(
                    cell.modifier.contains(Modifier::REVERSED) || theme.warning_band.bg.is_some(),
                    "({x},0) is not on the band"
                );
                assert!(cell.modifier.contains(Modifier::BOLD), "({x},0)");
                if theme.warning_band.bg.is_some() && !cell.symbol().trim().is_empty() {
                    let ratio = contrast(rgb(Some(cell.fg)), rgb(Some(cell.bg)));
                    assert!(ratio >= 4.5, "({x},0) {:?} is {ratio:.2}:1", cell.symbol());
                }
            }
        }
    }
}

#[test]
fn the_confirm_gauge_says_what_it_is() {
    let (fx, store) = fixture();
    let mut tui = driver(&fx, &store, Screen::Confirm, Theme::neon());
    let buf = frame(&mut tui, 120, 40);
    let line = (0..40)
        .map(|y| row(&buf, y, 0, 120))
        .find(|l| l.contains('▱') || l.contains(" 0%"))
        .expect("a gauge");
    assert!(line.trim_start().starts_with("Held"), "{line:?}");
    assert!(
        line.contains("▱▱▱▱") && line.trim_end().ends_with("0%"),
        "{line:?}"
    );
}
