//! The loop back from a result, and the keys that move through a list.
//!
//! The first: scan, look, purge, result, scan again, with nothing left over from
//! the run before. The second: `g` and `G` either move or say why they did not,
//! on every screen the keymap binds them on.

pub mod common;

use std::io;
use std::path::{Path, PathBuf};
use std::thread::sleep;
use std::time::{Duration, Instant};

use common::Fixture;
use dev_cleaner::config::Config;
use dev_cleaner::purge::Remover;
use dev_cleaner::tui::{
    KeyPress, PURGE, Screen, Screens, Step, Tui, bindings, collect, palette::Theme,
};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;

const AREA: Rect = Rect::new(0, 0, 120, 40);

/// Moves a directory out of the way the way the Trash does, inside the
/// fixture, so a rescan sees the disk as the purge left it.
struct Away(PathBuf);

impl Remover for Away {
    fn remove(&self, path: &Path) -> io::Result<PathBuf> {
        let to = self.0.join(path.to_string_lossy().replace('/', "_"));
        std::fs::rename(path, &to)?;
        Ok(to)
    }
}

/// Fails every item, so the result has a long list of what was not moved.
struct Refuses;

impl Remover for Refuses {
    fn remove(&self, _: &Path) -> io::Result<PathBuf> {
        Err(io::Error::other("the Trash said no"))
    }
}

/// Projects `app00`.. with a build directory each, larger the earlier it is.
fn projects(fx: &Fixture, n: usize) {
    for i in 0..n {
        fx.file(&format!("app{i:02}/package.json"), b"{}");
        fx.file(&format!("app{i:02}/src/index.js"), b"1");
        fx.file(
            &format!("app{i:02}/node_modules/dep/blob.bin"),
            &vec![7u8; 4096 * (n - i + 1)],
        );
    }
}

fn scan(fx: &Fixture, store: &Fixture) -> Screens {
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

fn frame(tui: &mut Tui) -> Buffer {
    let mut buf = Buffer::empty(AREA);
    tui.render(AREA, &mut buf);
    buf
}

fn rows(buf: &Buffer) -> Vec<String> {
    (0..buf.area.height)
        .map(|y| {
            (0..buf.area.width)
                .map(|x| buf[(x, y)].symbol())
                .collect::<String>()
                .trim_end()
                .to_string()
        })
        .collect()
}

fn text(tui: &mut Tui) -> String {
    rows(&frame(tui)).join("\n")
}

/// The notice row, which is the second from the bottom.
fn notice(tui: &mut Tui) -> String {
    let rows = rows(&frame(tui));
    rows[rows.len() - 2].trim().to_string()
}

fn enter_until(tui: &mut Tui, screen: Screen, now: Instant) {
    while tui.app().screen() != screen {
        tui.press(KeyPress::Enter, now);
    }
}

/// Hold the purge key on the confirm screen until it arms, and run the purge on
/// `remover` to its result.
fn purge_to_result(tui: &mut Tui, remover: Box<dyn Remover + Send>, now: Instant) {
    let mut step = Step::Stay;
    for repeat in 0..80u32 {
        step = tui.press(PURGE, now + Duration::from_millis(50 * u64::from(repeat)));
        if step == Step::Purge {
            break;
        }
    }
    assert_eq!(step, Step::Purge, "the hold never armed");
    tui.purge(remover);
    for _ in 0..1000 {
        tui.tick(Instant::now());
        if tui.app().screen() == Screen::Result {
            return;
        }
        sleep(Duration::from_millis(10));
    }
    panic!("the purge never reached its result");
}

/// One pass of the loop: mark the top entry, go on to the confirm screen, purge
/// it, and be on the result.
fn one_run(tui: &mut Tui, away: &Fixture, now: Instant) {
    assert_eq!(tui.app().screen(), Screen::Dashboard);
    enter_until(tui, Screen::Candidates, now);
    tui.press(KeyPress::Space, now);
    enter_until(tui, Screen::Confirm, now);
    purge_to_result(tui, Box::new(Away(away.root().to_path_buf())), now);
}

fn rebuildable(tui: &mut Tui) -> String {
    text(tui)
        .lines()
        .find(|l| l.contains("Biggest win"))
        .unwrap_or("")
        .trim()
        .to_string()
}

#[test]
fn leaving_a_result_gives_a_dashboard_of_a_fresh_scan_and_the_loop_runs_twice() {
    let fx = Fixture::new();
    let store = Fixture::new();
    let away = Fixture::new();
    let records = Fixture::new();
    projects(&fx, 3);
    let mut tui = Tui::new(scan(&fx, &store)).with_manifest_dir(records.root().to_path_buf());
    let now = Instant::now();
    assert!(rebuildable(&mut tui).contains("in 3 directories"));

    for (round, left) in [(1, "in 2 directories"), (2, "in 1 directory")] {
        one_run(&mut tui, &away, now);
        assert_eq!(
            tui.press(KeyPress::Enter, now),
            Step::Rescan,
            "round {round}: Enter on a result asks for a fresh scan"
        );
        tui.resume(scan(&fx, &store), now);

        assert_eq!(tui.app().screen(), Screen::Dashboard, "round {round}");
        assert_eq!(
            notice(&mut tui),
            "Back at the dashboard. Rescanned 3 projects.",
            "round {round}"
        );
        let line = rebuildable(&mut tui);
        assert!(
            line.contains(left),
            "round {round}: the dashboard still shows the numbers from before the purge: {line:?}"
        );
    }
}

#[test]
fn escape_leaves_a_result_the_same_way_enter_does() {
    let fx = Fixture::new();
    let store = Fixture::new();
    let away = Fixture::new();
    let records = Fixture::new();
    projects(&fx, 2);
    let mut tui = Tui::new(scan(&fx, &store)).with_manifest_dir(records.root().to_path_buf());
    let now = Instant::now();
    one_run(&mut tui, &away, now);

    assert_eq!(tui.press(KeyPress::Esc, now), Step::Rescan);
}

#[test]
fn the_key_bar_of_a_result_says_where_enter_and_escape_lead() {
    let fx = Fixture::new();
    let store = Fixture::new();
    let away = Fixture::new();
    let records = Fixture::new();
    projects(&fx, 2);
    let mut tui = Tui::new(scan(&fx, &store)).with_manifest_dir(records.root().to_path_buf());
    let now = Instant::now();
    one_run(&mut tui, &away, now);

    let rows = rows(&frame(&mut tui));
    let bar = rows.last().expect("a key bar");
    assert!(bar.contains("Enter/Esc dashboard"), "{bar:?}");
    assert!(bar.contains("quit"), "{bar:?}");
}

#[test]
fn a_stray_key_on_a_result_does_not_leave_it_and_q_still_quits() {
    let fx = Fixture::new();
    let store = Fixture::new();
    let away = Fixture::new();
    let records = Fixture::new();
    projects(&fx, 2);
    let mut tui = Tui::new(scan(&fx, &store)).with_manifest_dir(records.root().to_path_buf());
    let now = Instant::now();
    one_run(&mut tui, &away, now);

    for key in [
        KeyPress::Char('a'),
        KeyPress::Char('x'),
        KeyPress::Char('c'),
        KeyPress::Space,
        KeyPress::Tab,
        KeyPress::Delete,
    ] {
        assert_eq!(tui.press(key, now), Step::Stay, "{key} on a result");
        assert_eq!(tui.app().screen(), Screen::Result, "{key} left the result");
    }
    assert_eq!(tui.press(KeyPress::Char('q'), now), Step::Quit);
}

#[test]
fn nothing_of_the_run_survives_into_the_next_dashboard() {
    let fx = Fixture::new();
    let store = Fixture::new();
    let away = Fixture::new();
    let records = Fixture::new();
    projects(&fx, 3);
    let mut tui = Tui::new(scan(&fx, &store)).with_manifest_dir(records.root().to_path_buf());
    let now = Instant::now();
    one_run(&mut tui, &away, now);
    assert!(tui.record().is_some(), "the run wrote its record");
    tui.press(KeyPress::Enter, now);
    tui.resume(scan(&fx, &store), now);

    // Marks are gone: the candidates screen starts unmarked, and `q` has
    // nothing to ask about.
    enter_until(&mut tui, Screen::Candidates, now);
    let screen = text(&mut tui);
    assert!(
        !screen.contains("[x]"),
        "a mark outlived the run:\n{screen}"
    );
    assert_eq!(tui.press(KeyPress::Char('q'), now), Step::Quit);
}

// -- g and G ------------------------------------------------------------

/// How big the list a screen is looked at with is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Size {
    LongerThanTheWindow,
    ShorterThanTheWindow,
    Empty,
}

const LONG: usize = 60;
const SHORT: usize = 3;

/// A driver on `screen` with a list of `size`, and what keeps its fixtures
/// alive. `None` where the screen cannot have a list of that size.
fn on_screen(screen: Screen, size: Size) -> Option<(Tui, Vec<Fixture>)> {
    let n = match size {
        Size::LongerThanTheWindow => LONG,
        Size::ShorterThanTheWindow => SHORT,
        Size::Empty => 0,
    };
    let fx = Fixture::new();
    let store = Fixture::new();
    let away = Fixture::new();
    let records = Fixture::new();
    projects(&fx, n);
    let mut tui = Tui::new(scan(&fx, &store))
        .with_manifest_dir(records.root().to_path_buf())
        .with_theme(Theme::ansi());
    let now = Instant::now();
    match screen {
        Screen::Projects => tui.press(KeyPress::Enter, now),
        Screen::Candidates => {
            enter_until(&mut tui, Screen::Candidates, now);
            Step::Stay
        }
        Screen::Review => {
            enter_until(&mut tui, Screen::Candidates, now);
            tui.press(KeyPress::Char('a'), now);
            enter_until(&mut tui, Screen::Review, now);
            Step::Stay
        }
        Screen::Result => {
            // A result is as long as what it could not move. An empty plan has
            // nothing to purge, and a result of one with no failures is never
            // longer than its window.
            if size == Size::Empty {
                return None;
            }
            enter_until(&mut tui, Screen::Candidates, now);
            tui.press(KeyPress::Char('a'), now);
            enter_until(&mut tui, Screen::Confirm, now);
            let remover: Box<dyn Remover + Send> = if size == Size::LongerThanTheWindow {
                Box::new(Refuses)
            } else {
                Box::new(Away(away.root().to_path_buf()))
            };
            purge_to_result(&mut tui, remover, now);
            Step::Stay
        }
        other => panic!("{other:?} binds g, and this test does not know how to open it"),
    };
    frame(&mut tui);
    Some((tui, vec![fx, store, away, records]))
}

/// What the body shows, cursor band and all, with the notice row and the rows
/// of title left out: a key moved something if this changed.
fn body(tui: &mut Tui) -> Vec<String> {
    let buf = frame(tui);
    (2..buf.area.height - 2)
        .map(|y| {
            (0..buf.area.width)
                .map(|x| {
                    let cell = &buf[(x, y)];
                    format!(
                        "{}{:?}{:?}{:?}",
                        cell.symbol(),
                        cell.fg,
                        cell.bg,
                        cell.modifier
                    )
                })
                .collect::<String>()
        })
        .collect()
}

/// Press `key`, and say what came of it: the body moved, or the notice did.
///
/// The notice left by the key before is let go first, so what is read is the
/// answer to this key and not what was still on the row from the last.
fn pressed(tui: &mut Tui, key: char) -> (bool, String) {
    tui.tick(Instant::now() + Duration::from_secs(60));
    let before = body(tui);
    assert_eq!(tui.press(KeyPress::Char(key), Instant::now()), Step::Stay);
    let after = body(tui);
    (before != after, notice(tui))
}

const WORDS: [&str; 4] = [
    "Already at the top.",
    "Already at the bottom.",
    "Everything fits; nothing to scroll.",
    "The list is empty; nothing to scroll.",
];

/// The screens `g` or `G` is bound on, read from the keymap.
fn screens_binding_g() -> Vec<Screen> {
    let mut screens: Vec<Screen> = Vec::new();
    for b in bindings() {
        if matches!(b.key, KeyPress::Char('g' | 'G'))
            && let Some(screen) = b.screen
            && !screens.contains(&screen)
        {
            screens.push(screen);
        }
    }
    screens
}

#[test]
fn g_and_g_move_or_say_why_on_every_screen_that_binds_them() {
    let screens = screens_binding_g();
    assert!(
        screens.len() >= 4,
        "the keymap binds g on the projects, candidates, review and result: {screens:?}"
    );
    for screen in screens {
        for size in [
            Size::LongerThanTheWindow,
            Size::ShorterThanTheWindow,
            Size::Empty,
        ] {
            let Some((mut tui, _keep)) = on_screen(screen, size) else {
                continue;
            };
            // From the top, and from the bottom: each key at each edge.
            for (first, then) in [('g', 'G'), ('G', 'g')] {
                for key in [first, first, then, then] {
                    let (moved, said) = pressed(&mut tui, key);
                    assert!(
                        moved || WORDS.contains(&said.as_str()),
                        "{key} on {screen:?} with a list that is {size:?} moved nothing and said \
                         {said:?}"
                    );
                }
            }
        }
    }
}

#[test]
fn the_words_say_which_edge_or_why_there_is_no_edge() {
    // The same walk, held to what is said and not only to there being something.
    for screen in screens_binding_g() {
        let scrolls = matches!(screen, Screen::Review | Screen::Result);

        if let Some((mut tui, _keep)) = on_screen(screen, Size::LongerThanTheWindow) {
            assert_eq!(
                pressed(&mut tui, 'g').1,
                "Already at the top.",
                "{screen:?}"
            );
            assert!(pressed(&mut tui, 'G').0, "{screen:?}: G from the top moves");
            assert_eq!(
                pressed(&mut tui, 'G').1,
                "Already at the bottom.",
                "{screen:?}"
            );
            assert!(
                pressed(&mut tui, 'g').0,
                "{screen:?}: g from the bottom moves"
            );
        }
        if let Some((mut tui, _keep)) = on_screen(screen, Size::ShorterThanTheWindow) {
            let said = pressed(&mut tui, 'g').1;
            if scrolls {
                assert_eq!(said, "Everything fits; nothing to scroll.", "{screen:?}");
                let said = pressed(&mut tui, 'G').1;
                assert_eq!(said, "Everything fits; nothing to scroll.", "{screen:?}");
            } else {
                // A cursor has somewhere to go however short the list is.
                assert_eq!(said, "Already at the top.", "{screen:?}");
                assert!(pressed(&mut tui, 'G').0, "{screen:?}: the cursor moves");
                assert_eq!(
                    pressed(&mut tui, 'G').1,
                    "Already at the bottom.",
                    "{screen:?}"
                );
            }
        }
        if let Some((mut tui, _keep)) = on_screen(screen, Size::Empty) {
            for key in ['g', 'G'] {
                assert_eq!(
                    pressed(&mut tui, key).1,
                    "The list is empty; nothing to scroll.",
                    "{screen:?}"
                );
            }
        }
    }
}
