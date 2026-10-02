//! The dashboard's Enter hints keep their promise (#149): each hint is read off
//! the screen, its keys are pressed, and the screen they reach shows what the
//! sentence beside the hint is about.

pub mod common;

use std::path::Path;
use std::time::{Instant, SystemTime};

use common::Fixture;
use dev_cleaner::config::Config;
use dev_cleaner::tui::{KeyPress, Screen, Tui, collect, palette::Theme};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Modifier;

const AREA: Rect = Rect::new(0, 0, 160, 40);
const MB: usize = 1_000_000;

fn backdate(dir: &Path, when: SystemTime) {
    for entry in std::fs::read_dir(dir).expect("read_dir") {
        let path = entry.expect("entry").path();
        if path.is_dir() {
            backdate(&path, when);
        } else {
            std::fs::File::open(&path)
                .expect("open")
                .set_modified(when)
                .expect("set mtime");
        }
    }
}

/// The biggest project by bytes has nothing to rebuild; the one under it does.
fn win_fixture(fx: &Fixture) {
    fx.file("aaa-clean/package.json", b"{}");
    fx.file("aaa-clean/data.bin", &vec![1u8; 3 * MB]);
    fx.file("zzz-big/package.json", b"{}");
    fx.file("zzz-big/node_modules/dep/blob.bin", &vec![2u8; MB]);
}

/// A dead project with no build output, behind a bigger live one: nothing is
/// offerable, so the dead project is the dashboard's lead.
fn quiet_fixture(fx: &Fixture) {
    fx.file("aaa-clean/package.json", b"{}");
    fx.file("aaa-clean/data.bin", &vec![1u8; 3 * MB]);
    fx.file("old/package.json", b"{}");
    fx.file("old/src/index.js", b"console.log(1)");
    fx.git_repo("old", 200);
    fx.mark_pushed("old");
    let long_ago = SystemTime::now() - std::time::Duration::from_secs(400 * 86_400);
    backdate(&fx.root().join("old"), long_ago);
}

/// Nothing offerable and nothing dead; a project under a bigger one holds
/// something back.
fn held_fixture(fx: &Fixture) {
    fx.file("aaa-clean/package.json", b"{}");
    fx.file("aaa-clean/data.bin", &vec![1u8; 3 * MB]);
    fx.git_repo("held", 10);
    fx.file("held/package.json", b"{}");
    fx.file("held/node_modules/react/index.js", b"x");
}

fn tui(fx: &Fixture, store: &Fixture) -> Tui {
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
    Tui::new(screens).with_theme(Theme::ansi())
}

fn buffer(tui: &mut Tui) -> Buffer {
    let mut buf = Buffer::empty(AREA);
    tui.render(AREA, &mut buf);
    buf
}

fn frame(tui: &mut Tui) -> Vec<String> {
    let buf = buffer(tui);
    (0..AREA.height)
        .map(|y| {
            (0..AREA.width)
                .map(|x| buf[(x, y)].symbol())
                .collect::<String>()
                .trim_end()
                .to_string()
        })
        .collect()
}

fn text(tui: &mut Tui) -> String {
    frame(tui).join("\n")
}

/// The insight lines that carry an `Enter ... → screen` hint: the line, and the
/// hint split into how many times Enter is pressed and where it says it goes.
fn hints(tui: &mut Tui) -> Vec<(String, usize, String)> {
    frame(tui)
        .into_iter()
        .skip_while(|line| !line.trim_start().starts_with("Insights"))
        .filter_map(|line| {
            let hint = line.rsplit("  ").next()?.trim().to_string();
            let (keys, to) = hint.split_once(" → ")?;
            let mut words = keys.split_whitespace();
            assert_eq!(words.next(), Some("Enter"), "the hint names Enter: {line}");
            let times = match words.next() {
                None => 1,
                Some("twice") => 2,
                Some(n) => n.parse().expect("N times"),
            };
            Some((line.clone(), times, to.to_string()))
        })
        .collect()
}

fn press(tui: &mut Tui, key: KeyPress, times: usize) {
    for _ in 0..times {
        tui.press(key, Instant::now());
    }
}

/// Whether the row of project `name` is the one under the cursor.
fn cursor_is_on(tui: &mut Tui, name: &str) -> bool {
    let buf = buffer(tui);
    (0..AREA.height).any(|y| {
        let row: String = (0..AREA.width).map(|x| buf[(x, y)].symbol()).collect();
        row.contains(&format!("{name} "))
            && buf[(AREA.width / 2, y)]
                .modifier
                .contains(Modifier::REVERSED)
    })
}

#[test]
fn enter_twice_on_the_biggest_win_opens_the_project_that_holds_it() {
    let (fx, store) = (Fixture::new(), Fixture::new());
    win_fixture(&fx);
    let mut tui = tui(&fx, &store);
    let hints = hints(&mut tui);
    let (line, times, to) = hints.first().expect("the biggest win has a hint").clone();
    assert!(line.contains("Biggest win"), "{line}");
    assert_eq!((times, to.as_str()), (2, "candidates"), "{line}");

    press(&mut tui, KeyPress::Enter, times);
    assert_eq!(tui.app().screen(), Screen::Candidates);
    let t = text(&mut tui);
    assert!(t.contains("zzz-big/node_modules"), "{t}");
    assert!(!t.contains("nothing can be rebuilt"), "{t}");
}

#[test]
fn the_projects_table_keeps_its_order_and_only_the_cursor_moves() {
    let (fx, store) = (Fixture::new(), Fixture::new());
    win_fixture(&fx);
    let mut tui = tui(&fx, &store);
    press(&mut tui, KeyPress::Enter, 1);
    assert_eq!(tui.app().screen(), Screen::Projects);
    let rows = text(&mut tui);
    let (aaa, zzz) = (rows.find("aaa-clean"), rows.find("zzz-big"));
    assert!(
        aaa < zzz,
        "the table is still ordered by unique size:\n{rows}"
    );
    assert!(cursor_is_on(&mut tui, "zzz-big"), "{rows}");
    assert!(!cursor_is_on(&mut tui, "aaa-clean"), "{rows}");
}

#[test]
fn every_hint_the_dashboard_draws_lands_on_what_its_sentence_names() {
    type Case = (&'static str, fn(&Fixture), &'static str, fn(&mut Tui));
    let cases: [Case; 3] = [
        ("Biggest win", win_fixture, "candidates", |t| {
            assert!(text(t).contains("zzz-big/node_modules"), "{}", text(t));
        }),
        ("Gone quiet", quiet_fixture, "projects", |t| {
            assert!(cursor_is_on(t, "old"), "{}", text(t));
        }),
        ("Held back", held_fixture, "candidates", |t| {
            let shown = text(t);
            assert!(shown.contains("held/node_modules"), "{shown}");
            assert!(!shown.contains("aaa-clean"), "{shown}");
        }),
    ];
    for (headline, fixture, goes, lands_on_subject) in cases {
        let (fx, store) = (Fixture::new(), Fixture::new());
        fixture(&fx);
        let mut tui = tui(&fx, &store);
        let hints = hints(&mut tui);
        assert_eq!(
            hints.len(),
            1,
            "{headline}: one Enter, one promise: {hints:?}\n{}",
            text(&mut tui)
        );
        let (line, times, to) = &hints[0];
        assert!(line.contains(headline), "{headline}: {line}");
        assert_eq!(to, goes, "{line}");

        press(&mut tui, KeyPress::Enter, *times);
        assert_eq!(tui.app().screen().name(), goes, "{line}");
        lands_on_subject(&mut tui);
    }
}

#[test]
fn an_insight_with_no_path_carries_no_hint() {
    let (fx, store) = (Fixture::new(), Fixture::new());
    // Nothing rebuildable, nothing dead, nothing held back: no insight leads.
    fx.file("only/package.json", b"{}");
    fx.file("only/data.bin", &vec![1u8; MB]);
    let mut tui = tui(&fx, &store);
    assert!(hints(&mut tui).is_empty(), "{}", text(&mut tui));
}

#[test]
fn a_project_with_nothing_offerable_says_where_something_is() {
    let (fx, store) = (Fixture::new(), Fixture::new());
    win_fixture(&fx);
    let mut tui = tui(&fx, &store);
    // Straight from the table's top row, which is the empty one.
    press(&mut tui, KeyPress::Enter, 1);
    press(&mut tui, KeyPress::Char('g'), 1);
    press(&mut tui, KeyPress::Enter, 1);
    let shown = text(&mut tui);
    assert!(shown.contains("aaa-clean"), "{shown}");
    let notice = frame(&mut tui)[AREA.height as usize - 2].trim().to_string();
    assert!(
        notice.contains("1 project has something: zzz-big, 980.00 KB") && notice.contains("Tab"),
        "{notice}"
    );
}

#[test]
fn when_nothing_is_offerable_anywhere_the_notice_says_so() {
    let (fx, store) = (Fixture::new(), Fixture::new());
    held_fixture(&fx);
    let mut tui = tui(&fx, &store);
    press(&mut tui, KeyPress::Enter, 1);
    press(&mut tui, KeyPress::Char('g'), 1);
    press(&mut tui, KeyPress::Enter, 1);
    let notice = frame(&mut tui)[AREA.height as usize - 2].trim().to_string();
    assert!(
        notice.contains("Nothing is offered anywhere else"),
        "{notice}"
    );
}
