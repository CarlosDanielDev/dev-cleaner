//! The view says what is on (#152): a view bar over the projects table and the
//! candidates list, a filter with four fixed states, one key that resets, and no
//! body left empty.

pub mod common;

use std::path::PathBuf;
use std::time::Instant;

use common::Fixture;
use dev_cleaner::classify::Activity;
use dev_cleaner::config::Config;
use dev_cleaner::tui::{
    Column, Filter, KeyPress, ProjectSummary, Projects, Screen, Tally, Tui, collect, palette::Theme,
};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use std::collections::BTreeMap;

const MB: u64 = 1024 * 1024;

fn row(name: &str, unique: u64, activity: Activity, reclaimable: u64) -> ProjectSummary {
    ProjectSummary {
        path: PathBuf::from("/p").join(name),
        bytes_apparent: unique,
        bytes_unique: unique,
        inodes: 10,
        activity,
        reclaimable,
        checkout: Default::default(),
    }
}

/// Sixty projects, exactly one of which has something to remove: the shape of
/// the owner's real scan.
fn sixty() -> Projects {
    let mut rows: Vec<ProjectSummary> = (0..60)
        .map(|i| {
            let activity = if i % 3 == 0 {
                Activity::Dormant
            } else {
                Activity::Active
            };
            row(&format!("proj-{i:02}"), (i + 1) * MB, activity, 0)
        })
        .collect();
    rows[7].reclaimable = 44 * 1024;
    let mut table = Projects::new(rows);
    table.reset_view();
    table
}

fn lines(buf: &Buffer, area: Rect) -> Vec<String> {
    (0..area.height)
        .map(|y| {
            (0..area.width)
                .map(|x| buf[(x, y)].symbol())
                .collect::<String>()
                .trim_end()
                .to_string()
        })
        .collect()
}

fn table_lines(t: &Projects, w: u16, h: u16) -> Vec<String> {
    let area = Rect::new(0, 0, w, h);
    let mut buf = Buffer::empty(area);
    t.render(&Theme::ansi(), area, &mut buf);
    lines(&buf, area)
}

const COLUMNS: [Column; 6] = [
    Column::Name,
    Column::Unique,
    Column::Apparent,
    Column::Inodes,
    Column::Reclaimable,
    Column::Activity,
];

#[test]
fn the_default_view_bar_says_what_is_on_and_how_to_reset() {
    let t = sixty();
    let bar = &table_lines(&t, 160, 30)[0];
    assert_eq!(
        bar.trim(),
        "view  sort reclaimable ▼ largest first · show all projects · \
         1 of 60 have something to remove · f filter · r reset"
    );
}

#[test]
fn the_view_bar_names_every_filter_and_every_sort() {
    for (filter, words) in [
        (Filter::All, "show all projects"),
        (Filter::Removable, "show removable only (1 of 60)"),
        (Filter::Marked, "show marked only (0 of 60)"),
        (Filter::Quiet, "show quiet only (20 of 60)"),
    ] {
        for column in COLUMNS {
            let mut t = sixty();
            while t.filter() != filter {
                t.cycle_filter();
            }
            // Pressed twice where the column is already the one in use, so
            // the direction under test is a real choice and not a leftover.
            for presses in [1, 2] {
                if column != Column::Reclaimable || presses == 2 {
                    t.sort_by(column);
                }
                for (w, h) in [(80, 24), (100, 34), (160, 40)] {
                    let bar = table_lines(&t, w, h)[0].clone();
                    assert!(bar.starts_with(" view"), "{bar}");
                    let sort = t.ordering();
                    let (name, way) = sort.split_once(", ").expect("column, way");
                    assert!(
                        bar.contains(&format!("sort {name}")),
                        "{filter:?} {sort}: {bar}"
                    );
                    let arrow = if way.starts_with("largest") || way.starts_with("most first") {
                        "▼"
                    } else {
                        "▲"
                    };
                    assert!(
                        bar.contains(arrow) || column == Column::Name || column == Column::Activity,
                        "{sort}: {bar}"
                    );
                    assert!(bar.contains("r reset"), "{w}x{h} {filter:?} {sort}: {bar}");
                    assert!(bar.contains(way), "{w}x{h} {way}: {bar}");
                    if w >= 160 {
                        assert!(bar.contains(words), "{words}: {bar}");
                    }
                }
            }
        }
    }
}

#[test]
fn repeating_a_sort_key_flips_the_arrow_and_the_words_in_the_bar() {
    let mut t = sixty();
    t.sort_by(Column::Unique);
    let down = table_lines(&t, 160, 30)[0].clone();
    assert!(down.contains("sort unique ▼ largest first"), "{down}");
    t.sort_by(Column::Unique);
    let up = table_lines(&t, 160, 30)[0].clone();
    assert!(up.contains("sort unique ▲ smallest first"), "{up}");
}

#[test]
fn f_cycles_all_removable_marked_quiet_and_the_counts_match_the_rows() {
    let mut t = sixty();
    let mut seen = Vec::new();
    for _ in 0..4 {
        seen.push(t.filter());
        let drawn = t.shown().len();
        let total = t.rows().len();
        let line = table_lines(&t, 160, 80)
            .into_iter()
            .find(|l| l.contains("showing"))
            .expect("the position line");
        if t.filter() == Filter::All {
            assert!(line.contains("of 60") && !line.contains("total"), "{line}");
        } else {
            assert!(
                line.contains(&format!("of {drawn} ({total} total)")),
                "{:?}: {line}",
                t.filter()
            );
        }
        t.cycle_filter();
    }
    assert_eq!(
        seen,
        [
            Filter::All,
            Filter::Removable,
            Filter::Marked,
            Filter::Quiet
        ]
    );
    assert_eq!(t.filter(), Filter::All, "and round again");
    t.cycle_filter();
    assert_eq!(t.shown().len(), 1, "one project has something");
    t.cycle_filter();
    assert_eq!(t.shown().len(), 0, "nothing is marked");
    t.cycle_filter();
    assert_eq!(t.shown().len(), 20, "dormant or dead: the quiet ones");
}

#[test]
fn a_filtered_table_names_the_total_it_was_cut_from() {
    let mut t = sixty();
    t.cycle_filter();
    let shown = table_lines(&t, 160, 30).join("\n");
    assert!(shown.contains("showing 1-1 of 1 (60 total)"), "{shown}");
    assert!(shown.contains("proj-07"), "{shown}");
    assert!(!shown.contains("proj-08"), "{shown}");
}

#[test]
fn the_cursor_keeps_its_project_through_a_filter() {
    let mut t = sixty();
    t.focus(&PathBuf::from("/p/proj-07"));
    t.cycle_filter();
    assert_eq!(t.selected().expect("a row").name(), "proj-07");
    t.sort_by(Column::Name);
    assert_eq!(t.selected().expect("a row").name(), "proj-07");
    // A filter that shows nothing has nothing to select.
    t.cycle_filter();
    assert_eq!(t.filter(), Filter::Marked);
    assert!(t.selected().is_none(), "nothing is marked");
}

#[test]
fn the_marked_filter_shows_the_projects_that_have_marks() {
    let mut t = sixty();
    let mut marks = BTreeMap::new();
    marks.insert(
        PathBuf::from("/p/proj-30"),
        Tally {
            offered: (2, 10),
            marked: (1, 5),
        },
    );
    t.set_marks(marks);
    t.cycle_filter();
    t.cycle_filter();
    assert_eq!(t.filter(), Filter::Marked);
    let names: Vec<&str> = t.shown().iter().map(|r| r.name()).collect();
    assert_eq!(names, ["proj-30"]);
    assert!(table_lines(&t, 160, 30)[0].contains("show marked only (1 of 60)"));
}

#[test]
fn reset_returns_sort_and_filter_to_the_arrival_view() {
    let fresh = sixty();
    let mut t = sixty();
    t.sort_by(Column::Name);
    t.sort_by(Column::Name);
    t.cycle_filter();
    t.cycle_filter();
    t.cycle_filter();
    assert_ne!(t.ordering(), fresh.ordering());
    assert_ne!(t.filter(), fresh.filter());
    t.reset_view();
    assert_eq!(t.ordering(), "reclaimable, largest first");
    assert_eq!(t.ordering(), fresh.ordering());
    assert_eq!(t.filter(), fresh.filter());
    let names =
        |t: &Projects| -> Vec<String> { t.shown().iter().map(|r| r.name().to_string()).collect() };
    assert_eq!(names(&t), names(&fresh));
    assert_eq!(names(&t)[0], "proj-07", "what can be removed comes first");
    assert_eq!(t.cursor(), 0, "and the cursor is on it");
}

#[test]
fn a_view_that_is_not_the_default_is_lit_and_the_default_is_quiet() {
    let theme = Theme::ansi();
    // The bar's ink, and the position line's, which is plain text.
    let inks = |t: &Projects| {
        let area = Rect::new(0, 0, 160, 30);
        let mut buf = Buffer::empty(area);
        t.render(&theme, area, &mut buf);
        let ink = |x, y| (buf[(x, y)].fg, buf[(x, y)].modifier);
        (ink(2, 0), ink(1, 29))
    };
    let mut t = sixty();
    let (bar, plain) = inks(&t);
    assert_eq!(bar, plain, "the default view is quiet");
    t.cycle_filter();
    let (bar, plain) = inks(&t);
    assert_ne!(bar, plain, "a filter lights the bar");
    t.reset_view();
    t.sort_by(Column::Name);
    let (bar, plain) = inks(&t);
    assert_ne!(bar, plain, "a sort lights the bar");
    t.reset_view();
    let (bar, plain) = inks(&t);
    assert_eq!(bar, plain, "and r puts the quiet back");
}

#[test]
fn rows_with_nothing_to_remove_are_muted_and_the_one_with_bytes_is_not() {
    for theme in [Theme::ansi(), Theme::neon(), Theme::mono()] {
        let t = sixty();
        let area = Rect::new(0, 0, 160, 30);
        let mut buf = Buffer::empty(area);
        t.render(&theme, area, &mut buf);
        let rows = lines(&buf, area);
        // The last character of the reclaimable cell, which is a `B` either way.
        let cell = |name: &str| {
            let y = rows
                .iter()
                .position(|l| l.contains(name))
                .unwrap_or_else(|| panic!("no row {name}:\n{}", rows.join("\n")));
            let end = 1 + 26 + 12 + 12 + 11 + 13 - 1;
            assert_eq!(buf[(end, y as u16)].symbol(), "B", "{}", rows[y]);
            (buf[(end, y as u16)].fg, buf[(end, y as u16)].modifier)
        };
        let muted = (theme.muted.fg.unwrap_or_default(), theme.muted.add_modifier);
        assert_eq!(cell("proj-08"), muted, "0 B is muted");
        assert_ne!(cell("proj-07"), muted, "44 KB is not");
    }
}

#[test]
fn the_count_in_the_bar_carries_what_the_colour_carries() {
    // Under NO_COLOR the muted split may be invisible; the words are not.
    let t = sixty();
    let bar = &table_lines(&t, 100, 34)[0];
    assert!(bar.contains("1 of 60"), "{bar}");
}

#[test]
fn an_empty_filter_draws_a_body_that_says_why_and_what_to_press() {
    let mut t = sixty();
    t.cycle_filter();
    t.cycle_filter();
    let shown = table_lines(&t, 100, 34).join("\n");
    assert!(shown.contains("No project is marked."), "{shown}");
    assert!(
        shown.contains("proj-07"),
        "names where something is:\n{shown}"
    );
    assert!(shown.contains("r shows every project"), "{shown}");
}

#[test]
fn a_table_with_no_projects_still_says_so() {
    let t = Projects::new(Vec::new());
    let shown = table_lines(&t, 100, 34).join("\n");
    assert!(shown.contains("No project was found"), "{shown}");
}

// ---- through the interface -------------------------------------------------

const AREA: Rect = Rect::new(0, 0, 100, 34);
const MBYTES: usize = 1_000_000;

/// Sixty-one projects, one of them with something to rebuild: the owner's scan
/// in miniature.
fn world(fx: &Fixture) {
    for i in 0..60 {
        fx.file(&format!("p{i:02}/package.json"), b"{}");
        fx.file(&format!("p{i:02}/data.bin"), &vec![1u8; 1000 + i]);
    }
    fx.file("zz-one/package.json", b"{}");
    fx.file("zz-one/node_modules/dep/blob.bin", &vec![2u8; MBYTES / 20]);
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

fn frame_at(tui: &mut Tui, area: Rect) -> Vec<String> {
    let mut buf = Buffer::empty(area);
    tui.render(area, &mut buf);
    lines(&buf, area)
}

fn frame(tui: &mut Tui) -> Vec<String> {
    frame_at(tui, AREA)
}

fn text(tui: &mut Tui) -> String {
    frame(tui).join("\n")
}

fn press(tui: &mut Tui, keys: &[KeyPress]) {
    for key in keys {
        tui.press(*key, Instant::now());
    }
}

fn ch(c: char) -> KeyPress {
    KeyPress::Char(c)
}

fn bar(tui: &mut Tui) -> String {
    frame(tui)
        .into_iter()
        .find(|l| l.trim_start().starts_with("view"))
        .expect("a view bar")
}

fn notice(tui: &mut Tui) -> String {
    frame(tui)[AREA.height as usize - 2].trim().to_string()
}

#[test]
fn arriving_from_the_dashboard_puts_what_can_be_removed_first() {
    let (fx, store) = (Fixture::new(), Fixture::new());
    world(&fx);
    let mut tui = tui(&fx, &store);
    press(&mut tui, &[KeyPress::Enter]);
    assert_eq!(tui.app().screen(), Screen::Projects);
    let b = bar(&mut tui);
    assert!(b.contains("sort reclaimable ▼ largest first"), "{b}");
    assert!(b.contains("1 of 61"), "{b}");
    let rows = frame(&mut tui);
    let first = rows
        .iter()
        .position(|l| l.contains("zz-one"))
        .expect("zz-one is drawn");
    let other = rows.iter().position(|l| l.contains("p00")).expect("p00");
    assert!(first < other, "{}", rows.join("\n"));
}

#[test]
fn f_and_r_work_through_the_interface_and_say_so() {
    let (fx, store) = (Fixture::new(), Fixture::new());
    world(&fx);
    let mut tui = tui(&fx, &store);
    press(&mut tui, &[KeyPress::Enter, ch('f')]);
    assert!(
        bar(&mut tui).contains("removable only (1 of 61)"),
        "{}",
        bar(&mut tui)
    );
    let n = notice(&mut tui);
    assert!(n.contains("removable") && n.contains("1 of 61"), "{n}");
    press(&mut tui, &[ch('f'), ch('f'), ch('f')]);
    assert!(bar(&mut tui).contains("all projects"));
    press(&mut tui, &[ch('1'), ch('1'), ch('f')]);
    press(&mut tui, &[ch('r')]);
    let n = notice(&mut tui);
    assert_eq!(
        n,
        "View reset: sort reclaimable, largest first, all projects."
    );
    let b = bar(&mut tui);
    assert!(
        b.contains("sort reclaimable ▼ largest first") && b.contains("all projects"),
        "{b}"
    );
}

#[test]
fn a_filter_never_drops_a_mark_and_the_way_row_still_counts_it() {
    let (fx, store) = (Fixture::new(), Fixture::new());
    world(&fx);
    let mut tui = tui(&fx, &store);
    // The one removable project is first in the arrival view: mark it.
    press(&mut tui, &[KeyPress::Enter, KeyPress::Space]);
    let before = text(&mut tui);
    assert!(before.contains("1 marked"), "{before}");
    // Every filter in turn: the mark stays, the way row still counts it.
    for _ in 0..4 {
        press(&mut tui, &[ch('f')]);
        let t = text(&mut tui);
        assert!(t.contains("1 marked"), "{t}");
    }
    // Marked shows the project that has it, and only that one.
    press(&mut tui, &[ch('f'), ch('f')]);
    let b = bar(&mut tui);
    assert!(b.contains("marked only (1 of 61)"), "{b}");
    assert!(text(&mut tui).contains("zz-one"));
    // Quiet hides it (the project is active), and the mark is still there.
    press(&mut tui, &[ch('f')]);
    assert!(!text(&mut tui).contains("zz-one"));
    assert!(text(&mut tui).contains("1 marked"));
    // Back on Marked, Space takes the mark off and the row goes with it.
    press(&mut tui, &[ch('f'), ch('f'), ch('f')]);
    assert!(
        bar(&mut tui).contains("marked only (1 of 61)"),
        "{}",
        bar(&mut tui)
    );
    press(&mut tui, &[KeyPress::Space]);
    let t = text(&mut tui);
    assert!(t.contains("No project is marked."), "{t}");
    assert!(!t.contains(" marked ("), "{t}");
}

#[test]
fn a_mark_below_the_window_still_shows_in_the_marked_view() {
    let (fx, store) = (Fixture::new(), Fixture::new());
    // More projects with something to rebuild than a window holds, so a mark on
    // the last of them is below the window in every filter that could show it.
    for i in 0..40 {
        fx.file(&format!("r{i:02}/package.json"), b"{}");
        fx.file(
            &format!("r{i:02}/node_modules/dep/blob.bin"),
            &vec![2u8; 3000],
        );
    }
    let mut tui = tui(&fx, &store);
    press(
        &mut tui,
        &[KeyPress::Enter, ch('1'), ch('G'), KeyPress::Space],
    );
    assert!(text(&mut tui).contains("1 marked"));
    // Round the whole cycle and on to marked again, from the top: the mark was
    // made below the window and no filter on the way may have let go of it.
    press(&mut tui, &[ch('g')]);
    for _ in 0..6 {
        press(&mut tui, &[ch('f')]);
        let _ = text(&mut tui);
    }
    let b = bar(&mut tui);
    assert!(b.contains("marked only (1 of 40)"), "{b}");
    assert!(text(&mut tui).contains("r39"));
}

#[test]
fn f_and_r_change_nothing_that_is_offered_or_planned() {
    let plan_after = |keys: &[KeyPress]| {
        let (fx, store) = (Fixture::new(), Fixture::new());
        world(&fx);
        let mut tui = tui(&fx, &store);
        press(&mut tui, &[KeyPress::Enter]);
        press(&mut tui, keys);
        press(
            &mut tui,
            &[
                ch('r'),
                KeyPress::Space,
                KeyPress::Enter,
                ch('a'),
                KeyPress::Enter,
            ],
        );
        assert_eq!(tui.app().screen(), Screen::Review);
        // The fixture's own directory name differs from run to run.
        let mut shown = String::new();
        let mut skipping = false;
        for c in text(&mut tui).chars() {
            match (skipping, c) {
                (_, '/') => {
                    skipping = false;
                    shown.push(c);
                }
                (true, _) => {}
                (false, '.') => {
                    skipping = true;
                    shown.push(c);
                }
                (false, _) => shown.push(c),
            }
        }
        shown
    };
    let plain = plan_after(&[]);
    let busy = plan_after(&[ch('f'), ch('f'), ch('1'), ch('f'), ch('3'), ch('3')]);
    assert_eq!(plain, busy);
}

#[test]
fn the_view_bar_is_part_of_the_screen_and_outlives_the_notice() {
    let (fx, store) = (Fixture::new(), Fixture::new());
    world(&fx);
    let mut tui = tui(&fx, &store);
    press(&mut tui, &[KeyPress::Enter, ch('2')]);
    let later = Instant::now() + dev_cleaner::tui::NOTICE_TTL * 3;
    tui.tick(later);
    assert!(
        !notice(&mut tui).contains("Sorted by"),
        "the notice timed out"
    );
    assert!(bar(&mut tui).contains("sort unique ▼ largest first"));
}

#[test]
fn the_bar_is_drawn_at_every_size_the_interface_supports() {
    let (fx, store) = (Fixture::new(), Fixture::new());
    world(&fx);
    let mut tui = tui(&fx, &store);
    press(&mut tui, &[KeyPress::Enter]);
    for (w, h) in [(80, 24), (100, 34), (120, 40)] {
        let area = Rect::new(0, 0, w, h);
        let b = frame_at(&mut tui, area)
            .into_iter()
            .find(|l| l.trim_start().starts_with("view"))
            .unwrap_or_else(|| panic!("no bar at {w}x{h}"));
        assert!(b.contains("sort reclaimable"), "{w}x{h}: {b}");
        assert!(b.contains("r reset"), "{w}x{h}: {b}");
        assert!(b.contains("1 of 61"), "{w}x{h}: {b}");
    }
}

#[test]
fn a_project_with_nothing_opens_a_body_not_an_empty_screen() {
    let (fx, store) = (Fixture::new(), Fixture::new());
    world(&fx);
    let mut tui = tui(&fx, &store);
    // Last row of the arrival view: a project with nothing to rebuild.
    press(&mut tui, &[KeyPress::Enter, ch('G'), KeyPress::Enter]);
    assert_eq!(tui.app().screen(), Screen::Candidates);
    let shown = text(&mut tui);
    assert!(shown.contains("Nothing to rebuild in "), "{shown}");
    assert!(
        shown.contains("zz-one"),
        "names where something is:\n{shown}"
    );
    assert!(shown.contains("Tab all projects"), "{shown}");
    assert!(shown.contains("Esc projects"), "{shown}");
}

#[test]
fn the_candidates_view_bar_names_sort_scope_and_reset() {
    let (fx, store) = (Fixture::new(), Fixture::new());
    world(&fx);
    let mut tui = tui(&fx, &store);
    press(&mut tui, &[KeyPress::Enter, ch('G'), KeyPress::Enter]);
    let b = bar(&mut tui);
    assert!(b.contains("sort size ▼ largest first"), "{b}");
    assert!(b.contains("this project"), "{b}");
    assert!(b.contains("r reset"), "{b}");
    press(&mut tui, &[KeyPress::Tab, ch('1')]);
    let b = bar(&mut tui);
    assert!(b.contains("all projects") && b.contains("path"), "{b}");
    press(&mut tui, &[ch('r')]);
    let n = notice(&mut tui);
    assert!(n.starts_with("View reset: sort size, largest first"), "{n}");
    let b = bar(&mut tui);
    assert!(
        b.contains("sort size ▼ largest first") && b.contains("this project"),
        "{b}"
    );
}

#[test]
fn no_screen_scope_or_filter_of_the_fixture_has_an_empty_body() {
    let (fx, store) = (Fixture::new(), Fixture::new());
    world(&fx);
    for (w, h) in [(80, 24), (100, 34), (160, 40)] {
        let area = Rect::new(0, 0, w, h);
        let mut tui = tui(&fx, &store);
        let body = |tui: &mut Tui, what: &str| {
            let rows = frame_at(tui, area);
            // Under the header band, over the notice and the key bar.
            let start = rows
                .iter()
                .position(|l| l.trim_start().starts_with("view"))
                .unwrap_or_else(|| panic!("{what}: no bar at {w}x{h}\n{}", rows.join("\n")));
            let used = rows[start + 1..rows.len() - 2]
                .iter()
                .filter(|l| !l.trim().is_empty())
                .count();
            assert!(
                used >= 2,
                "{what} at {w}x{h} has an empty body:\n{}",
                rows.join("\n")
            );
        };
        press(&mut tui, &[KeyPress::Enter]);
        for filter in 0..4 {
            body(&mut tui, &format!("projects, filter {filter}"));
            press(&mut tui, &[ch('f')]);
        }
        // Candidates of a project with something, of one with nothing, and of
        // every project, for each project row.
        press(&mut tui, &[ch('g'), KeyPress::Enter]);
        body(&mut tui, "candidates, project with something");
        press(&mut tui, &[KeyPress::Tab]);
        body(&mut tui, "candidates, all projects");
        press(
            &mut tui,
            &[KeyPress::Tab, KeyPress::Esc, ch('G'), KeyPress::Enter],
        );
        body(&mut tui, "candidates, project with nothing");
        press(&mut tui, &[KeyPress::Tab]);
        body(&mut tui, "candidates, all projects from an empty one");
    }
}

#[test]
fn nothing_anywhere_says_so_in_the_body() {
    let (fx, store) = (Fixture::new(), Fixture::new());
    fx.file("only/package.json", b"{}");
    fx.file("only/data.bin", &vec![1u8; 2000]);
    let mut tui = tui(&fx, &store);
    press(&mut tui, &[KeyPress::Enter, KeyPress::Enter]);
    let shown = text(&mut tui);
    assert!(shown.contains("Nothing to rebuild in only."), "{shown}");
    assert!(
        shown.contains("Nothing is offered in any other project."),
        "{shown}"
    );
    press(&mut tui, &[KeyPress::Esc, ch('f')]);
    let shown = text(&mut tui);
    assert!(
        shown.contains("Nothing to rebuild in the one project"),
        "{shown}"
    );
}

#[test]
fn enter_on_an_empty_view_stays_and_says_why() {
    // Found by the poka-yoke re-audit: with nothing under the cursor, Enter
    // used to leave for the candidates screen on whatever scope it last had.
    let (fx, store) = (Fixture::new(), Fixture::new());
    world(&fx);
    let mut tui = tui(&fx, &store);
    press(&mut tui, &[KeyPress::Enter, ch('f'), ch('f')]);
    assert!(text(&mut tui).contains("No project is marked."));
    press(&mut tui, &[KeyPress::Enter]);
    assert_eq!(tui.app().screen(), Screen::Projects);
    let n = notice(&mut tui);
    assert!(
        n.contains("no project to open") && n.contains("r shows"),
        "{n}"
    );
}

// ---- with #151's repo order, badge column and detail line ------------------

use dev_cleaner::classify::{Checkout, Kind};

/// A main checkout and eleven worktrees of it, among plain projects; one
/// worktree has something to rebuild.
fn repos() -> Projects {
    let mut rows: Vec<ProjectSummary> = (0..20)
        .map(|i| row(&format!("plain-{i:02}"), (i + 1) * MB, Activity::Active, 0))
        .collect();
    let mut main = row("main", 5 * MB, Activity::Active, 0);
    main.path = PathBuf::from("/k/main");
    main.checkout = Checkout {
        kind: Kind::Main,
        repo: Some(PathBuf::from("/k/main")),
        branch: Some("main".into()),
        linked: 11,
        ..Checkout::default()
    };
    rows.push(main);
    for w in 0..11 {
        let mut r = row(&format!("wt-{w:02}"), MB, Activity::Dormant, 0);
        r.path = PathBuf::from(format!("/k/wt-{w:02}/app"));
        r.checkout = Checkout {
            kind: Kind::Worktree,
            repo: Some(PathBuf::from("/k/main")),
            worktree: Some(format!("wt-{w:02}")),
            branch: Some(format!("feat/{w}")),
            ..Checkout::default()
        };
        if w == 4 {
            r.reclaimable = 9 * MB;
        }
        rows.push(r);
    }
    let mut t = Projects::new(rows);
    t.reset_view();
    t
}

#[test]
fn the_repo_order_is_named_in_the_bar_in_words_with_its_direction() {
    for (w, h) in [(80, 24), (100, 34), (160, 40)] {
        let mut t = repos();
        t.sort_by(Column::Repo);
        let up = table_lines(&t, w, h)[0].clone();
        assert!(up.contains("sort by repo ▲"), "{w}x{h}: {up}");
        assert!(up.contains("r reset"), "{w}x{h}: {up}");
        t.sort_by(Column::Repo);
        let down = table_lines(&t, w, h)[0].clone();
        assert!(down.contains("sort by repo ▼"), "{w}x{h}: {down}");
        if w >= 100 {
            assert!(
                up.contains("main first") && down.contains("main last"),
                "{up} / {down}"
            );
        }
    }
}

#[test]
fn r_undoes_the_repo_order_and_the_filter_together() {
    let fresh = repos();
    let mut t = repos();
    t.sort_by(Column::Repo);
    t.cycle_filter();
    t.cycle_filter();
    t.cycle_filter();
    assert_eq!(t.filter(), Filter::Quiet);
    assert!(t.ordering().starts_with("repo"), "{}", t.ordering());
    t.reset_view();
    assert_eq!(t.ordering(), fresh.ordering());
    assert_eq!(t.filter(), Filter::All);
    let names =
        |t: &Projects| -> Vec<String> { t.shown().iter().map(|r| r.name().to_string()).collect() };
    assert_eq!(names(&t), names(&fresh));
}

#[test]
fn every_filter_composes_with_the_repo_order() {
    let mut t = repos();
    t.sort_by(Column::Repo);
    let path = |r: &&ProjectSummary| r.path.to_str().unwrap().to_string();
    let all: Vec<String> = t.shown().iter().map(path).collect();
    // The main checkout leads its worktrees whatever the filter lets through.
    let at = |n: &str| all.iter().position(|x| x == n).expect(n);
    assert!(at("/k/main") < at("/k/wt-00/app"), "{all:?}");
    t.cycle_filter();
    let removable: Vec<String> = t.shown().iter().map(path).collect();
    assert_eq!(removable, ["/k/wt-04/app"]);
    t.cycle_filter();
    t.cycle_filter();
    let quiet: Vec<String> = t.shown().iter().map(path).collect();
    assert_eq!(quiet.len(), 11, "the worktrees are dormant: {quiet:?}");
    assert!(
        quiet.windows(2).all(|w| w[0] < w[1]),
        "still in repo order: {quiet:?}"
    );
}

#[test]
fn the_bar_and_the_detail_line_never_share_a_row_and_the_filter_and_badge_survive_80_columns() {
    let mut t = repos();
    t.sort_by(Column::Repo);
    t.focus(&PathBuf::from("/k/wt-04/app"));
    for (w, h) in [(80, 24), (100, 34), (160, 40)] {
        let rows = table_lines(&t, w, h);
        // Top row: the bar. Bottom row: the position. The row above it: the
        // selected project in full. None of them is another's.
        assert!(
            rows[0].trim_start().starts_with("view"),
            "{w}x{h}: {rows:?}"
        );
        let last = rows.len() - 1;
        assert!(rows[last].contains("showing"), "{w}x{h}: {}", rows[last]);
        assert!(
            !rows[last - 1].trim_start().starts_with("view") && rows[last - 1].contains("wt-04"),
            "{w}x{h}: the detail line names the selection: {}",
            rows[last - 1]
        );
        assert_eq!(
            rows.iter()
                .filter(|r| r.trim_start().starts_with("view"))
                .count(),
            1,
            "{w}x{h}"
        );
        // The badge column is drawn beside the names, and the bar still has
        // its filter facts, at every width.
        assert!(
            rows[1].contains('⎇'),
            "{w}x{h}: the badge header: {}",
            rows[1]
        );
        assert!(
            rows.iter().any(|r| r.contains('⎇') && r.contains("wt-0")),
            "{w}x{h}"
        );
        let bar = &rows[0];
        assert!(
            bar.contains("all") && bar.contains("of 32"),
            "{w}x{h}: {bar}"
        );
        if w >= 160 {
            assert!(bar.contains("f filter"), "{w}x{h}: {bar}");
        }
    }
    // The key survives on the key bar where the bar's own hint is shed.
    let keys = dev_cleaner::tui::footer(Screen::Projects, 78);
    assert!(
        keys.contains("f filter") && keys.contains("r reset view"),
        "{keys}"
    );
    // And with the filter on, the same two lines, under a narrower area.
    t.cycle_filter();
    let rows = table_lines(&t, 80, 24);
    assert!(rows[0].contains("removable only (1 of 32)"), "{}", rows[0]);
    assert!(rows[1].contains('⎇'), "{}", rows[1]);
}
