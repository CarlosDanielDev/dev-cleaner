//! Marks follow the project (#144): the candidates screen shows the project that
//! was opened, the marks stay one set keyed by path, and the projects table
//! says what is marked where.

pub mod common;

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::Instant;

use common::Fixture;
use dev_cleaner::classify::Activity;
use dev_cleaner::config::Config;
use dev_cleaner::tui::{
    KeyPress, ProjectSummary, Projects, Screen, Screens, Tally, Tui, collect, palette::Theme,
};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;

const AREA: Rect = Rect::new(0, 0, 120, 40);

/// alpha: two entries. bravo: one. charlie: one, held back by uncommitted work.
fn fixture(fx: &Fixture) {
    fx.file("alpha/package.json", b"{}");
    fx.file("alpha/Cargo.toml", b"[package]\nname = \"alpha\"\n");
    fx.file("alpha/node_modules/dep/blob.bin", &vec![1u8; 200_000]);
    fx.file("alpha/target/debug/blob.bin", &vec![2u8; 100_000]);
    fx.file("bravo/package.json", b"{}");
    fx.file("bravo/node_modules/dep/blob.bin", &vec![3u8; 50_000]);
    fx.git_repo("charlie", 0);
    fx.file("charlie/package.json", b"{}");
    fx.file("charlie/node_modules/dep/blob.bin", &vec![4u8; 20_000]);
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

/// On the projects table, ordered by name: alpha, bravo, charlie.
fn on_projects(fx: &Fixture, store: &Fixture) -> Tui {
    let mut tui = Tui::new(screens(fx, store)).with_theme(Theme::ansi());
    let now = Instant::now();
    tui.press(KeyPress::Enter, now);
    assert_eq!(tui.app().screen(), Screen::Projects);
    tui.press(KeyPress::Char('1'), now);
    tui
}

fn key(tui: &mut Tui, k: KeyPress) {
    tui.press(k, Instant::now());
}

/// Put the table's cursor on the `n`th project, in name order.
fn at(tui: &mut Tui, n: usize) {
    key(tui, KeyPress::Char('g'));
    for _ in 0..n {
        key(tui, KeyPress::Down);
    }
}

fn open(tui: &mut Tui, n: usize) {
    at(tui, n);
    key(tui, KeyPress::Enter);
    assert_eq!(tui.app().screen(), Screen::Candidates);
}

fn frame(tui: &mut Tui) -> Vec<String> {
    let mut buf = Buffer::empty(AREA);
    tui.render(AREA, &mut buf);
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

fn notice(tui: &mut Tui) -> String {
    let rows = frame(tui);
    rows[rows.len() - 2].trim().to_string()
}

/// The table row of project `name`, mark column included.
fn row_of(tui: &mut Tui, name: &str) -> String {
    frame(tui)
        .into_iter()
        .find(|line| {
            let line = line.trim_start();
            let line = line.trim_start_matches(['●', '◐', '·']).trim_start();
            line.starts_with(&format!("{name} "))
        })
        .unwrap_or_else(|| panic!("no row for {name}"))
}

fn marked_in_total(tui: &mut Tui) -> String {
    let rows = frame(tui);
    rows[1].clone()
}

#[test]
fn a_project_opens_its_own_entries_and_its_marks_stay_on_them() {
    let (fx, store) = (Fixture::new(), Fixture::new());
    fixture(&fx);
    let mut tui = on_projects(&fx, &store);

    open(&mut tui, 0);
    let a = text(&mut tui);
    assert!(a.contains("alpha/node_modules") && a.contains("alpha/target"));
    assert!(!a.contains("bravo/node_modules"), "{a}");
    key(&mut tui, KeyPress::Space);
    key(&mut tui, KeyPress::Esc);

    open(&mut tui, 1);
    let b = text(&mut tui);
    assert!(b.contains("bravo/node_modules"), "{b}");
    assert!(
        !b.contains("alpha/"),
        "alpha's entries are not bravo's:\n{b}"
    );
    assert!(!b.contains("[x]"), "nothing of bravo is marked yet:\n{b}");
    key(&mut tui, KeyPress::Space);
    key(&mut tui, KeyPress::Esc);

    let (alpha, bravo, charlie) = (
        row_of(&mut tui, "alpha"),
        row_of(&mut tui, "bravo"),
        row_of(&mut tui, "charlie"),
    );
    assert!(alpha.contains('◐'), "some of alpha: {alpha}");
    assert!(alpha.contains(" of "), "marked of offered bytes: {alpha}");
    assert!(bravo.contains('●'), "all of bravo: {bravo}");
    assert!(bravo.contains(" / "), "{bravo}");
    assert!(
        !charlie.contains(['●', '◐', '·']),
        "nothing offerable is blank: {charlie}"
    );
    assert!(
        marked_in_total(&mut tui).contains("2 marked") && text(&mut tui).contains("in 2 projects"),
        "the table says the total"
    );

    open(&mut tui, 0);
    let again = text(&mut tui);
    assert_eq!(again.matches("[x]").count(), 1, "{again}");
    assert_eq!(again.matches("[ ]").count(), 1, "{again}");

    // The plan is built from every mark in every project.
    key(&mut tui, KeyPress::Enter);
    assert_eq!(tui.app().screen(), Screen::Review);
    assert!(
        marked_in_total(&mut tui).contains("built from the 2 you marked"),
        "{}",
        marked_in_total(&mut tui)
    );
}

#[test]
fn tab_widens_to_every_project_and_back_without_touching_the_marks() {
    let (fx, store) = (Fixture::new(), Fixture::new());
    fixture(&fx);
    let mut tui = on_projects(&fx, &store);
    open(&mut tui, 1);
    key(&mut tui, KeyPress::Space);
    let narrow = text(&mut tui);
    let way = marked_in_total(&mut tui);
    assert!(narrow.contains("bravo"), "the scope is named:\n{narrow}");

    key(&mut tui, KeyPress::Tab);
    let wide = text(&mut tui);
    assert!(wide.contains("All projects"), "{wide}");
    assert!(wide.contains("alpha/node_modules") && wide.contains("bravo/node_modules"));
    assert_eq!(marked_in_total(&mut tui), way, "same marks, widened");
    assert_eq!(wide.matches("[x]").count(), 1, "{wide}");

    key(&mut tui, KeyPress::Tab);
    let back = text(&mut tui);
    assert!(!back.contains("alpha/"), "{back}");
    assert!(!back.contains("All projects"), "{back}");
    assert_eq!(marked_in_total(&mut tui), way, "same marks, narrowed");
    assert_eq!(back.matches("[x]").count(), 1, "{back}");
}

#[test]
fn space_on_a_project_row_marks_all_of_its_own_entries_and_no_others() {
    let (fx, store) = (Fixture::new(), Fixture::new());
    fixture(&fx);
    let mut tui = on_projects(&fx, &store);

    at(&mut tui, 0);
    key(&mut tui, KeyPress::Space);
    let said = notice(&mut tui);
    assert!(
        said.starts_with("Marked 2 entries in alpha") && said.contains("2 marked in total"),
        "{said}"
    );
    assert!(row_of(&mut tui, "alpha").contains('●'));
    let bravo = row_of(&mut tui, "bravo");
    assert!(
        bravo.contains('·') && !bravo.contains(['●', '◐']),
        "bravo has none of its own marked: {bravo}"
    );

    key(&mut tui, KeyPress::Space);
    let said = notice(&mut tui);
    assert!(
        said.starts_with("Unmarked 2 entries in alpha") && said.contains("0 marked in total"),
        "{said}"
    );
    assert!(!row_of(&mut tui, "alpha").contains(['●', '◐']));

    // Some marked: the key completes the set rather than clearing it.
    open(&mut tui, 0);
    key(&mut tui, KeyPress::Space);
    key(&mut tui, KeyPress::Esc);
    at(&mut tui, 0);
    key(&mut tui, KeyPress::Space);
    assert!(row_of(&mut tui, "alpha").contains('●'));
}

#[test]
fn space_on_a_project_with_nothing_offerable_says_why_and_changes_nothing() {
    let (fx, store) = (Fixture::new(), Fixture::new());
    fixture(&fx);
    let mut tui = on_projects(&fx, &store);
    at(&mut tui, 2);
    key(&mut tui, KeyPress::Space);
    let said = notice(&mut tui);
    assert!(
        said.contains("charlie")
            && said.contains("nothing can be rebuilt")
            && said.contains("held back"),
        "{said}"
    );
    assert!(!text(&mut tui).contains("marked ("), "nothing was marked");
}

#[test]
fn a_and_c_act_on_the_visible_scope_and_name_what_stays_marked_elsewhere() {
    let (fx, store) = (Fixture::new(), Fixture::new());
    fixture(&fx);
    let mut tui = on_projects(&fx, &store);
    at(&mut tui, 0);
    key(&mut tui, KeyPress::Space); // alpha: both marked

    open(&mut tui, 1);
    key(&mut tui, KeyPress::Char('a'));
    let said = notice(&mut tui);
    assert!(
        said.contains("Marked all 1") && said.contains("bravo"),
        "{said}"
    );
    assert!(said.contains("3 marked in total"), "{said}");

    key(&mut tui, KeyPress::Char('c'));
    let said = notice(&mut tui);
    assert!(
        said.contains("Cleared 1 mark in bravo") && said.contains("2 more marked elsewhere"),
        "{said}"
    );

    // `c` again puts back what that `c` cleared, in the scope it was pressed in.
    key(&mut tui, KeyPress::Char('c'));
    let said = notice(&mut tui);
    assert!(
        said.contains("Restored 1 mark in bravo") && said.contains("2 more marked elsewhere"),
        "{said}"
    );
    key(&mut tui, KeyPress::Char('c'));
    let said = notice(&mut tui);
    assert!(said.contains("Cleared 1 mark in bravo"), "{said}");
    key(&mut tui, KeyPress::Esc);
    assert!(row_of(&mut tui, "alpha").contains('●'), "alpha untouched");
    open(&mut tui, 1);

    // Under Tab, `c` clears everything, as before.
    key(&mut tui, KeyPress::Tab);
    key(&mut tui, KeyPress::Char('c'));
    let said = notice(&mut tui);
    assert!(said.contains("Cleared 2 marks"), "{said}");
}

#[test]
fn a_project_with_nothing_offerable_shows_its_own_held_back_entries() {
    let (fx, store) = (Fixture::new(), Fixture::new());
    fixture(&fx);
    let mut tui = on_projects(&fx, &store);
    open(&mut tui, 2);
    let t = text(&mut tui);
    assert!(t.contains("charlie/node_modules"), "{t}");
    assert!(
        !t.contains("alpha/") && !t.contains("bravo/"),
        "the global list never shows:\n{t}"
    );
    assert!(t.contains("nothing can be rebuilt here"), "{t}");
    assert!(
        notice(&mut tui).contains("nothing can be rebuilt here"),
        "the notice and the screen agree"
    );
}

fn summary(name: &str) -> ProjectSummary {
    ProjectSummary {
        path: PathBuf::from(format!("/p/{name}")),
        bytes_apparent: 1000,
        bytes_unique: 1000,
        inodes: 3,
        activity: Activity::Active,
        reclaimable: 500,
    }
}

fn table_text(table: &Projects, theme: &Theme, width: u16) -> String {
    let area = Rect::new(0, 0, width, 6);
    let mut buf = Buffer::empty(area);
    table.render(theme, area, &mut buf);
    (0..area.height)
        .map(|y| {
            (0..area.width)
                .map(|x| buf[(x, y)].symbol())
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn the_mark_column_goes_whole_below_80_columns_and_the_glyph_carries_the_state() {
    let mut table = Projects::new(vec![summary("a")]);
    let mut marks = BTreeMap::new();
    marks.insert(
        PathBuf::from("/p/a"),
        Tally {
            offered: (2, 500),
            marked: (1, 200),
        },
    );
    table.set_marks(marks);

    for theme in [Theme::ansi(), Theme::mono(), Theme::neon()] {
        let wide = table_text(&table, &theme, 80);
        assert!(
            wide.contains('◐') && wide.contains("200 B of 500 B"),
            "{wide}"
        );
        assert!(
            wide.contains("some marked"),
            "the legend has words:\n{wide}"
        );
        let narrow = table_text(&table, &theme, 79);
        assert!(!narrow.contains(['●', '◐']), "{narrow}");
        assert!(
            !narrow.contains("200 B of"),
            "the byte cell is plain too:\n{narrow}"
        );
    }
}

#[test]
fn a_project_inside_another_keeps_its_own_entries() {
    let (fx, store) = (Fixture::new(), Fixture::new());
    fx.file("outer/package.json", b"{}");
    fx.file("outer/node_modules/dep/blob.bin", &vec![1u8; 30_000]);
    fx.file("outer/pkg/package.json", b"{}");
    fx.file("outer/pkg/node_modules/dep/blob.bin", &vec![2u8; 20_000]);
    let mut tui = on_projects(&fx, &store);

    open(&mut tui, 0);
    let outer = text(&mut tui);
    assert!(outer.contains("outer/node_modules"), "{outer}");
    assert!(
        !outer.contains("pkg/node_modules"),
        "the inner project's entry is the inner project's:\n{outer}"
    );
    key(&mut tui, KeyPress::Esc);

    at(&mut tui, 0);
    key(&mut tui, KeyPress::Space);
    assert!(row_of(&mut tui, "outer").contains('●'));
    let inner = row_of(&mut tui, "pkg");
    assert!(
        inner.contains('·'),
        "marking the outer project leaves the inner one alone: {inner}"
    );
}
