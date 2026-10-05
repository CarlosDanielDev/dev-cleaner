//! The header is a status bar (#155): on the icon's five rows the title line,
//! the stepper and the hints share one left edge, the rule closes the band
//! from column 0, and the context of the scan sits at the right.

pub mod common;

use std::time::{Duration, Instant};

use common::Fixture;
use dev_cleaner::config::Config;
use dev_cleaner::purge::Remover;
use dev_cleaner::tui::logo::{GAP, HEIGHT, WIDTH};
use dev_cleaner::tui::{KeyPress, PURGE, Screen, Step, Tui, collect, palette::Theme};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Modifier;

const TEXT_X: u16 = 1 + WIDTH + GAP;
const TITLE: u16 = 1;
const STEPPER: u16 = 2;
const HINTS: u16 = 3;
const RULE_ROW: u16 = HEIGHT;

fn fixture() -> (Fixture, Fixture) {
    let fx = Fixture::new();
    let store = Fixture::new();
    fx.file("app/package.json", b"{}");
    fx.file("app/src/index.js", b"console.log(1)");
    fx.file("app/node_modules/dep/blob.bin", &vec![0xABu8; 4096]);
    (fx, store)
}

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
    while tui.app().screen() != screen && tui.app().screen() != Screen::Confirm {
        if tui.app().screen() == Screen::Candidates {
            tui.press(KeyPress::Char('a'), now);
        }
        tui.press(KeyPress::Enter, now);
    }
    if screen == Screen::Result {
        // Reached only through a purge: hold, run it against nothing, wait for it.
        let records = Fixture::new();
        tui = tui.with_manifest_dir(records.root().to_path_buf());
        for repeat in 0..80u32 {
            let at = now + Duration::from_millis(50 * u64::from(repeat));
            if tui.press(PURGE, at) == Step::Purge {
                break;
            }
        }
        tui.purge(Box::new(Nothing));
        for _ in 0..500 {
            tui.tick(Instant::now());
            if tui.app().screen() == Screen::Result {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(tui.app().screen(), Screen::Result);
        std::mem::forget(records);
    }
    tui
}

struct Nothing;

impl Remover for Nothing {
    fn remove(&self, path: &std::path::Path) -> std::io::Result<std::path::PathBuf> {
        Ok(path.to_path_buf())
    }
}

fn frame(tui: &mut Tui, cols: u16, rows: u16) -> Buffer {
    let area = Rect::new(0, 0, cols, rows);
    let mut buf = Buffer::empty(area);
    tui.render(area, &mut buf);
    buf
}

fn line(buf: &Buffer, y: u16) -> String {
    (0..buf.area.width).map(|x| buf[(x, y)].symbol()).collect()
}

fn left_edge(buf: &Buffer, y: u16, from: u16) -> Option<u16> {
    (from..buf.area.width).find(|&x| buf[(x, y)].symbol() != " ")
}

const FLOW: [Screen; 6] = [
    Screen::Dashboard,
    Screen::Projects,
    Screen::Candidates,
    Screen::Review,
    Screen::Confirm,
    Screen::Result,
];

#[test]
fn the_band_has_one_rhythm_and_the_rule_closes_it_from_column_zero() {
    let (fx, store) = fixture();
    for (cols, rows) in [(90, 28), (100, 34), (120, 40)] {
        let mut tui = driver(&fx, &store, Screen::Dashboard, Theme::neon());
        let buf = frame(&mut tui, cols, rows);
        let at = format!("{cols}x{rows}");
        // Rows 0 and 4 beside the icon are the margins of the three text rows.
        assert_eq!(left_edge(&buf, 0, 1 + WIDTH), None, "{at}: row 0");
        assert_eq!(left_edge(&buf, 4, 1 + WIDTH), None, "{at}: row 4");
        for y in [TITLE, STEPPER, HINTS] {
            assert_eq!(left_edge(&buf, y, 1 + WIDTH), Some(TEXT_X), "{at}: row {y}");
        }
        assert!(
            line(&buf, TITLE).contains("dev-cleaner ▸ Dashboard"),
            "{at}"
        );
        for x in 0..cols {
            assert_eq!(buf[(x, RULE_ROW)].symbol(), "─", "{at}: rule at {x}");
        }
    }
}

#[test]
fn the_context_sits_at_the_right_of_the_title_line_and_only_says_what_is_known() {
    let (fx, store) = fixture();
    let mut tui = driver(&fx, &store, Screen::Dashboard, Theme::neon());
    let buf = frame(&mut tui, 120, 40);
    let title = line(&buf, TITLE);
    assert!(title.trim_end().ends_with("theme: neon"), "{title}");
    assert!(title.contains("1 project"), "{title}");
    assert!(title.contains("scanned"), "{title}");
    assert_eq!(title.chars().count(), 120);
    assert!(
        !title.contains("0 projects") && !title.contains('?'),
        "{title}"
    );
    // The name is the primary fact: bold text, not muted.
    let at = title.find("Dashboard").unwrap();
    let col = title[..at].chars().count() as u16;
    assert!(buf[(col, TITLE)].modifier.contains(Modifier::BOLD));
}

#[test]
fn the_stepper_walks_the_flow_with_a_marker_for_each_step() {
    let (fx, store) = fixture();
    let labels = [
        "Dashboard",
        "Projects",
        "Candidates",
        "Plan",
        "Confirm",
        "Result",
    ];
    for (i, screen) in FLOW.into_iter().enumerate() {
        let mut tui = driver(&fx, &store, screen, Theme::neon());
        let buf = frame(&mut tui, 120, 40);
        let stepper = line(&buf, STEPPER);
        for (j, label) in labels.iter().enumerate() {
            let marker = match j.cmp(&i) {
                std::cmp::Ordering::Less => '✓',
                std::cmp::Ordering::Equal => '●',
                std::cmp::Ordering::Greater => '○',
            };
            assert!(
                stepper.contains(&format!("{marker} {label}")),
                "{screen:?}: `{marker} {label}` in `{stepper}`"
            );
        }
    }
}

#[test]
fn the_stepper_collapses_whole_steps_and_never_cuts_a_label() {
    let (fx, store) = fixture();
    let mut tui = driver(&fx, &store, Screen::Candidates, Theme::neon());
    let wide = line(&frame(&mut tui, 100, 34), STEPPER);
    assert!(
        wide.contains("Dashboard") && wide.contains("Result"),
        "{wide}"
    );
    let middle = line(&frame(&mut tui, 90, 28), STEPPER);
    assert!(middle.contains("‹") && middle.contains("›"), "{middle}");
    assert!(
        middle.contains("✓ Projects") && middle.contains("● Candidates"),
        "{middle}"
    );
    assert!(middle.contains("○ Plan"), "{middle}");
    assert!(
        !middle.contains("Dashboard") && !middle.contains("Result"),
        "{middle}"
    );
}

#[test]
fn the_compact_header_is_one_status_line_and_the_rule() {
    let (fx, store) = fixture();
    for (cols, rows) in [(80, 24), (60, 20)] {
        let mut tui = driver(&fx, &store, Screen::Candidates, Theme::neon());
        let buf = frame(&mut tui, cols, rows);
        let status = line(&buf, 0);
        assert!(
            status.contains("dev-cleaner ▸ Candidates"),
            "{cols}: {status}"
        );
        assert!(status.contains("✓✓●○○○ 3/6"), "{cols}: {status}");
        for x in 0..cols {
            assert_eq!(buf[(x, 1)].symbol(), "─", "{cols}: rule at {x}");
        }
    }
}

#[test]
fn the_markers_and_the_hints_are_glyphs_and_words_without_colour() {
    let (fx, store) = fixture();
    let mut tui = driver(&fx, &store, Screen::Projects, Theme::mono());
    let buf = frame(&mut tui, 100, 34);
    assert!(line(&buf, STEPPER).contains("✓ Dashboard"));
    assert!(line(&buf, STEPPER).contains("● Projects"));
    assert!(line(&buf, HINTS).contains("Esc ← dashboard"));
    assert!(line(&buf, HINTS).contains("Enter → candidates"));
}

#[test]
fn the_header_no_longer_describes_the_first_screen() {
    let (fx, store) = fixture();
    let mut tui = driver(&fx, &store, Screen::Dashboard, Theme::neon());
    let buf = frame(&mut tui, 100, 34);
    for y in 0..6 {
        assert!(!line(&buf, y).contains("the first screen"));
    }
    assert!(line(&buf, HINTS).contains("Enter → projects"));
}

#[test]
fn the_icon_is_drawn_in_bold() {
    let (fx, store) = fixture();
    let mut tui = driver(&fx, &store, Screen::Dashboard, Theme::neon());
    let buf = frame(&mut tui, 100, 34);
    let dots: Vec<_> = (1..1 + WIDTH)
        .flat_map(|x| (0..HEIGHT).map(move |y| (x, y)))
        .filter(|&(x, y)| {
            ('\u{2801}'..='\u{28ff}').contains(&buf[(x, y)].symbol().chars().next().unwrap())
        })
        .collect();
    assert!(!dots.is_empty());
    for (x, y) in dots {
        assert!(buf[(x, y)].modifier.contains(Modifier::BOLD), "({x},{y})");
    }
}
