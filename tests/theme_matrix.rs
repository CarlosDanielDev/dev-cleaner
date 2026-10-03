//! The matrix theme on every screen (#166): MS-DOS meets the Matrix, in every
//! colour mode, with the same data under it as neon has.

pub mod common;

use std::sync::Arc;
use std::time::{Duration, Instant};

use common::Fixture;
use dev_cleaner::config::Config;
use dev_cleaner::scan::Progress;
use dev_cleaner::tui::palette::{Mode, Theme, ThemeName};
use dev_cleaner::tui::{KeyPress, Screen, Screens, Tui, collect};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Color;

fn fixture() -> (Fixture, Fixture) {
    let fx = Fixture::new();
    let store = Fixture::new();
    for (name, bytes) in [("app", 4096usize), ("web", 9000)] {
        fx.file(&format!("{name}/package.json"), b"{}");
        fx.file(&format!("{name}/src/index.js"), b"console.log(1)");
        fx.file(
            &format!("{name}/node_modules/dep/blob.bin"),
            &vec![0xABu8; bytes],
        );
    }
    (fx, store)
}

fn screens(fx: &Fixture, store: &Fixture) -> Screens {
    let cfg = Config {
        roots: vec![fx.root().to_path_buf()],
        caches: Vec::new(),
        denylist: Vec::new(),
    };
    collect(
        &[fx.root().to_path_buf()],
        &cfg,
        fx.root(),
        &store.root().join("history.sqlite3"),
    )
}

fn driver(fx: &Fixture, store: &Fixture, screen: Screen, theme: Theme) -> Tui {
    let mut tui = Tui::new(screens(fx, store)).with_theme(theme);
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

fn row(buf: &Buffer, y: u16) -> String {
    (0..buf.area.width).map(|x| buf[(x, y)].symbol()).collect()
}

fn text(buf: &Buffer) -> String {
    (0..buf.area.height)
        .map(|y| row(buf, y))
        .collect::<Vec<_>>()
        .join("\n")
}

const SCREENS: [Screen; 5] = [
    Screen::Dashboard,
    Screen::Projects,
    Screen::Candidates,
    Screen::Review,
    Screen::Confirm,
];

const MODES: [Mode; 3] = [Mode::Truecolor, Mode::Ansi, Mode::Mono];

fn matrix(mode: Mode) -> Theme {
    Theme::named(ThemeName::Matrix, mode)
}

#[test]
fn every_screen_draws_in_matrix_in_every_colour_mode_at_every_size() {
    let (fx, store) = fixture();
    for mode in MODES {
        for screen in SCREENS {
            for (cols, rows) in [(80, 24), (90, 28), (100, 34), (160, 40), (79, 23)] {
                let mut tui = driver(&fx, &store, screen, matrix(mode));
                let buf = frame(&mut tui, cols, rows);
                assert!(
                    !text(&buf).trim().is_empty(),
                    "{screen:?} {mode:?} {cols}x{rows} drew nothing"
                );
                // Neon's single-line cells and rule are not in this look.
                let drawn = text(&buf);
                for glyph in ['▰', '▱', '▮'] {
                    assert!(
                        !drawn.contains(glyph),
                        "{screen:?} {mode:?} {cols}x{rows} has {glyph}:\n{drawn}"
                    );
                }
            }
        }
    }
}

#[test]
fn the_key_list_the_scan_block_and_the_too_small_paragraph_draw_in_matrix_too() {
    let (fx, store) = fixture();
    for mode in MODES {
        let mut tui = driver(&fx, &store, Screen::Dashboard, matrix(mode));
        tui.press(KeyPress::Char('?'), Instant::now());
        let keys = text(&frame(&mut tui, 100, 34));
        assert!(keys.contains("Keys"), "{keys}");
        assert!(
            keys.contains("[Enter]"),
            "the key list has bracketed caps:\n{keys}"
        );

        let mut small = driver(&fx, &store, Screen::Dashboard, matrix(mode));
        let small = text(&frame(&mut small, 60, 20));
        assert!(small.contains("needs 80×24"), "{small}");

        let now = Instant::now();
        let mut scanning = Tui::starting(
            Screens::pending(vec![fx.root().to_path_buf()], fx.root().join("n.sqlite3")),
            Arc::new(Progress::default()),
            now,
        )
        .with_theme(matrix(mode));
        scanning.tick(now + Duration::from_secs(2));
        let scan = text(&frame(&mut scanning, 100, 34));
        assert!(scan.contains("Scanning"), "{scan}");
    }
}

#[test]
fn the_header_is_a_dos_prompt_with_the_screen_name_in_capitals() {
    let (fx, store) = fixture();
    for screen in SCREENS {
        for (cols, rows, title_row) in [(100, 34, 1), (80, 24, 0)] {
            let mut tui = driver(&fx, &store, screen, matrix(Mode::Truecolor));
            let buf = frame(&mut tui, cols, rows);
            let line = row(&buf, title_row);
            let want = format!("C:\\DEV-CLEANER\\{}>", screen.name().to_uppercase());
            assert!(line.contains(&want), "{screen:?} {cols}x{rows}: {line}");
        }
    }
}

#[test]
fn the_prompt_title_stays_readable_in_the_buffer_without_colour() {
    let (fx, store) = fixture();
    let mut tui = driver(&fx, &store, Screen::Candidates, matrix(Mode::Mono));
    let line = row(&frame(&mut tui, 100, 34), 1);
    assert!(line.contains("C:\\DEV-CLEANER\\CANDIDATES>"), "{line}");
}

#[test]
fn the_block_cursor_blinks_on_the_tick() {
    let (fx, store) = fixture();
    let mut tui = driver(&fx, &store, Screen::Dashboard, matrix(Mode::Truecolor));
    let t0 = Instant::now();
    let title = |tui: &mut Tui| row(&frame(tui, 100, 34), 1);
    let on = "DASHBOARD>█";
    let off = "DASHBOARD> ";
    tui.tick(t0);
    assert!(title(&mut tui).contains(on), "{}", title(&mut tui));
    tui.tick(t0 + Duration::from_millis(400));
    assert!(title(&mut tui).contains(on), "still lit at 400 ms");
    tui.tick(t0 + Duration::from_millis(600));
    assert!(title(&mut tui).contains(off), "dark at 600 ms");
    tui.tick(t0 + Duration::from_millis(1100));
    assert!(title(&mut tui).contains(on), "lit again at 1100 ms");
}

#[test]
fn the_cursor_does_not_move_the_text_when_it_blinks() {
    let (fx, store) = fixture();
    let mut tui = driver(&fx, &store, Screen::Dashboard, matrix(Mode::Truecolor));
    let t0 = Instant::now();
    tui.tick(t0);
    let lit = frame(&mut tui, 100, 34);
    tui.tick(t0 + Duration::from_millis(600));
    let dark = frame(&mut tui, 100, 34);
    let strip = |b: &Buffer| row(b, 1).replace('█', " ");
    assert_eq!(strip(&lit), strip(&dark));
}

#[test]
fn reduced_motion_keeps_the_cursor_solid() {
    let (fx, store) = fixture();
    let theme = matrix(Mode::Truecolor).with_motion(false);
    let mut tui = driver(&fx, &store, Screen::Dashboard, theme);
    let t0 = Instant::now();
    for ms in [0, 600, 1100, 1700] {
        tui.tick(t0 + Duration::from_millis(ms));
        let line = row(&frame(&mut tui, 100, 34), 1);
        assert!(line.contains("DASHBOARD>█"), "at {ms} ms: {line}");
    }
}

#[test]
fn neon_has_no_cursor_and_its_title_is_what_it_was() {
    let (fx, store) = fixture();
    let mut tui = driver(&fx, &store, Screen::Dashboard, Theme::neon());
    let t0 = Instant::now();
    tui.tick(t0 + Duration::from_millis(600));
    let line = row(&frame(&mut tui, 100, 34), 1);
    assert!(line.contains("dev-cleaner ▸ Dashboard"), "{line}");
    assert!(!line.contains('█'), "{line}");
}

#[test]
fn the_stepper_is_bracketed_and_the_band_is_closed_by_a_double_rule() {
    let (fx, store) = fixture();
    let mut tui = driver(&fx, &store, Screen::Candidates, matrix(Mode::Truecolor));
    let buf = frame(&mut tui, 120, 40);
    let stepper = row(&buf, 2);
    assert!(
        stepper.contains("[✓] Dashboard ══ [✓] Projects ══ [●] Candidates ══ [ ] Plan"),
        "{stepper}"
    );
    let rule = row(&buf, 5);
    assert!(rule.starts_with('╞') && rule.ends_with('╡'), "{rule}");
    assert!(rule.chars().filter(|c| *c == '═').count() >= 100, "{rule}");
    // Compact: the glyph a step, and the same rule under it.
    let mut small = driver(&fx, &store, Screen::Candidates, matrix(Mode::Truecolor));
    let buf = frame(&mut small, 80, 24);
    assert!(
        row(&buf, 0).contains("[✓] [✓] [●] [ ] [ ] [ ] 3/6"),
        "{}",
        row(&buf, 0)
    );
    assert!(row(&buf, 1).starts_with('╞'), "{}", row(&buf, 1));
}

#[test]
fn matrix_headings_run_into_double_rules_and_nothing_draws_a_single_one() {
    let (fx, store) = fixture();
    for screen in [Screen::Dashboard, Screen::Review, Screen::Confirm] {
        for mode in MODES {
            let mut tui = driver(&fx, &store, screen, matrix(mode));
            let drawn = text(&frame(&mut tui, 100, 34));
            assert!(drawn.contains("══"), "{screen:?} {mode:?}");
            assert!(
                !drawn.contains('─'),
                "{screen:?} {mode:?} draws a single rule:\n{drawn}"
            );
        }
    }
}

#[test]
fn the_disk_gauge_and_the_bars_are_shaded_blocks() {
    let (fx, store) = fixture();
    let mut tui = driver(&fx, &store, Screen::Dashboard, matrix(Mode::Truecolor));
    let drawn = text(&frame(&mut tui, 100, 34));
    assert!(drawn.contains('█') && drawn.contains('░'), "{drawn}");
}

#[test]
fn under_no_color_matrix_has_no_colour_and_keeps_its_shapes() {
    let (fx, store) = fixture();
    for screen in SCREENS {
        let mut tui = driver(&fx, &store, screen, matrix(Mode::Mono));
        let buf = frame(&mut tui, 100, 34);
        for y in 0..34 {
            for x in 0..100 {
                let cell = &buf[(x, y)];
                assert_eq!(cell.fg, Color::Reset, "{screen:?} ({x},{y})");
                assert_eq!(cell.bg, Color::Reset, "{screen:?} ({x},{y})");
            }
        }
        assert!(text(&buf).contains("C:\\DEV-CLEANER\\"), "{screen:?}");
    }
}

#[test]
fn matrix_paints_black_under_the_whole_frame_in_truecolor_only() {
    let (fx, store) = fixture();
    let mut tui = driver(&fx, &store, Screen::Dashboard, matrix(Mode::Truecolor));
    let buf = frame(&mut tui, 100, 34);
    assert_eq!(buf[(0, 20)].bg, Color::Rgb(0, 0, 0));
    assert_eq!(buf[(99, 33)].bg, Color::Rgb(0, 0, 0));
    let mut tui = driver(&fx, &store, Screen::Dashboard, matrix(Mode::Ansi));
    assert_eq!(frame(&mut tui, 100, 34)[(0, 20)].bg, Color::Reset);
}

#[test]
fn the_selected_row_is_the_dos_highlight_bar_black_on_bright_green() {
    let (fx, store) = fixture();
    let mut tui = driver(&fx, &store, Screen::Projects, matrix(Mode::Truecolor));
    let buf = frame(&mut tui, 100, 34);
    let bar = (0..34).find(|&y| buf[(5, y)].bg == Color::Rgb(0x00, 0xff, 0x41));
    let y = bar.expect("a row is under the cursor");
    assert_eq!(buf[(5, y)].fg, Color::Rgb(0, 0, 0));
}

#[test]
fn the_logo_is_drawn_in_two_greens() {
    let (fx, store) = fixture();
    let mut tui = driver(&fx, &store, Screen::Dashboard, matrix(Mode::Truecolor));
    let buf = frame(&mut tui, 100, 34);
    let mut inks = std::collections::BTreeSet::new();
    for y in 0..5 {
        for x in 1..15 {
            let cell = &buf[(x, y)];
            if ('\u{2801}'..='\u{28ff}').contains(&cell.symbol().chars().next().unwrap()) {
                inks.insert(format!("{:?}", cell.fg));
            }
        }
    }
    assert_eq!(inks.len(), 2, "the can and the mark, in two inks: {inks:?}");
}

#[test]
fn the_same_numbers_and_the_same_candidates_stand_under_both_themes() {
    let (fx, store) = fixture();
    // The body and the header: the key bar is left out, as brackets round the
    // caps leave it room for one entry fewer, which is its own business.
    let numbers = |buf: &Buffer| -> Vec<String> {
        let body: Vec<String> = (0..buf.area.height - 1).map(|y| row(buf, y)).collect();
        let mut tokens: Vec<String> = body
            .join("\n")
            .split_whitespace()
            .map(|t| t.trim_matches(['[', ']']))
            .filter(|t| t.chars().any(|c| c.is_ascii_digit()))
            .filter(|t| !t.contains(".tmp"))
            .map(str::to_string)
            .collect();
        tokens.sort();
        tokens
    };
    for screen in SCREENS {
        let mut neon = driver(&fx, &store, screen, Theme::neon());
        let mut green = driver(&fx, &store, screen, matrix(Mode::Truecolor));
        let (a, b) = (frame(&mut neon, 100, 34), frame(&mut green, 100, 34));
        assert_eq!(numbers(&a), numbers(&b), "{screen:?}");
        let offered = |t: &Tui| -> Vec<(String, u64)> {
            let mut out = Vec::new();
            if let Some(plan) = t.app().reviewing() {
                for c in plan.items() {
                    out.push((c.path.display().to_string(), c.bytes));
                }
            }
            out
        };
        assert_eq!(offered(&neon), offered(&green), "{screen:?}");
    }
    // What is offerable is the screens', and a theme never touched them.
    let a = screens(&fx, &store);
    let b = screens(&fx, &store);
    let bytes =
        |s: &Screens| -> Vec<u64> { s.candidates.selectable().iter().map(|c| c.bytes).collect() };
    assert_eq!(bytes(&a), bytes(&b));
    assert!(!bytes(&a).is_empty());
}

// --- the digital rain ---------------------------------------------------

fn katakana(c: char) -> bool {
    ('\u{ff66}'..='\u{ff9d}').contains(&c)
}

fn rain_cells(buf: &Buffer) -> usize {
    text(buf).chars().filter(|c| katakana(*c)).count()
}

/// A scan that has been running for `secs`, in `theme`, drawn at 100x34.
fn scanning(theme: Theme, secs: u64) -> Buffer {
    let now = Instant::now();
    let mut tui = Tui::starting(
        Screens::pending(vec!["/scan-root".into()], "/scan-root/n.sqlite3".into()),
        Arc::new(Progress::default()),
        now,
    )
    .with_theme(theme);
    tui.tick(now + Duration::from_secs(secs));
    frame(&mut tui, 100, 34)
}

/// The cells of the body that are blank when nothing falls: the room there is.
fn free_cells(buf: &Buffer) -> usize {
    (6..32)
        .flat_map(|y| (0..100).map(move |x| (x, y)))
        .filter(|&(x, y)| buf[(x, y)].symbol() == " ")
        .count()
}

#[test]
fn rain_falls_beside_a_matrix_scan_and_stays_under_a_quarter_of_the_free_cells() {
    for secs in [2, 3, 5, 9, 31] {
        let base = scanning(matrix(Mode::Truecolor).with_motion(false), secs);
        let rain = scanning(matrix(Mode::Truecolor), secs);
        let n = rain_cells(&rain);
        assert!(n > 0, "no rain at {secs} s:\n{}", text(&rain));
        assert!(
            n * 4 <= free_cells(&base),
            "{n} rain cells of {} free at {secs} s",
            free_cells(&base)
        );
    }
}

#[test]
fn rain_never_overwrites_a_cell_that_holds_anything() {
    for secs in [2, 4, 7, 20] {
        let base = scanning(matrix(Mode::Truecolor).with_motion(false), secs);
        let rain = scanning(matrix(Mode::Truecolor), secs);
        for y in 0..34 {
            for x in 0..100 {
                if rain[(x, y)] != base[(x, y)] {
                    assert_eq!(
                        base[(x, y)].symbol(),
                        " ",
                        "rain over {:?} at ({x},{y}) at {secs} s",
                        base[(x, y)].symbol()
                    );
                }
            }
        }
    }
}

#[test]
fn rain_only_falls_to_the_right_of_a_line_and_never_between_its_words() {
    for secs in [2, 4, 7, 20] {
        let rain = scanning(matrix(Mode::Truecolor), secs);
        for y in 6..32 {
            let cells: Vec<char> = (0..100)
                .map(|x| rain[(x, y)].symbol().chars().next().unwrap_or(' '))
                .collect();
            let Some(first_rain) = cells.iter().position(|c| katakana(*c)) else {
                continue;
            };
            // Everything that is not rain lies left of the first rain cell,
            // with the gutter between.
            let last_text = cells.iter().rposition(|c| *c != ' ' && !katakana(*c));
            if let Some(last) = last_text {
                assert!(
                    last + 2 < first_rain,
                    "row {y} at {secs} s: text ends at {last}, rain starts at {first_rain}: {}",
                    cells.iter().collect::<String>()
                );
            }
        }
    }
}

#[test]
fn rain_moves_with_the_tick() {
    let a = scanning(matrix(Mode::Truecolor), 3);
    let b = scanning(matrix(Mode::Truecolor), 4);
    let spots = |buf: &Buffer| -> Vec<(u16, u16)> {
        (0..34)
            .flat_map(|y| (0..100).map(move |x| (x, y)))
            .filter(|&(x, y)| katakana(buf[(x, y)].symbol().chars().next().unwrap_or(' ')))
            .collect()
    };
    assert_ne!(spots(&a), spots(&b), "the rain stood still for a second");
}

#[test]
fn there_is_no_rain_in_neon() {
    for mode in [Mode::Truecolor, Mode::Ansi] {
        let buf = scanning(Theme::named(ThemeName::Neon, mode), 5);
        assert_eq!(rain_cells(&buf), 0, "{mode:?}");
    }
}

#[test]
fn there_is_no_rain_under_no_color() {
    let buf = scanning(matrix(Mode::Mono), 5);
    assert_eq!(rain_cells(&buf), 0, "{}", text(&buf));
}

#[test]
fn there_is_no_rain_under_reduced_motion() {
    let theme = matrix(Mode::Truecolor).motion_from(Some("1"));
    let buf = scanning(theme, 5);
    assert_eq!(rain_cells(&buf), 0, "{}", text(&buf));
}

#[test]
fn the_rain_has_a_white_green_head_and_a_dim_green_tail() {
    let buf = scanning(matrix(Mode::Truecolor), 5);
    let mut inks = std::collections::BTreeSet::new();
    for y in 0..34 {
        for x in 0..100 {
            if katakana(buf[(x, y)].symbol().chars().next().unwrap_or(' ')) {
                inks.insert(format!("{:?}", buf[(x, y)].fg));
            }
        }
    }
    assert!(
        inks.contains(&format!("{:?}", Color::Rgb(0xcc, 0xff, 0xdd)))
            && inks.contains(&format!("{:?}", Color::Rgb(0x1f, 0x9a, 0x3f))),
        "{inks:?}"
    );
}

#[test]
fn rain_costs_a_frame_nothing_worth_counting_and_adds_no_redraw() {
    // The loop draws on every 100 ms tick whatever is on the screen, so the
    // rain adds no redraw: it is a function of the clock the scan view already
    // reads. What it can add is the cost of a frame, and that is measured.
    let fx = Fixture::new();
    let time = |theme: Theme| {
        let now = Instant::now();
        let mut tui = Tui::starting(
            Screens::pending(vec![fx.root().to_path_buf()], fx.root().join("n.sqlite3")),
            Arc::new(Progress::default()),
            now,
        )
        .with_theme(theme);
        tui.tick(now + Duration::from_secs(5));
        let start = Instant::now();
        for _ in 0..200 {
            frame(&mut tui, 100, 34);
        }
        start.elapsed() / 200
    };
    let off = time(matrix(Mode::Truecolor).with_motion(false));
    let on = time(matrix(Mode::Truecolor));
    assert!(
        on < Duration::from_millis(5),
        "a frame with rain took {on:?}"
    );
    assert!(
        on < off * 4 + Duration::from_millis(1),
        "rain costs {on:?} a frame against {off:?} without"
    );
}
