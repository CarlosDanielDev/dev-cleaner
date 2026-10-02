//! Every screen is laid out by one rule: what belongs together lines up.
//!
//! Names, paths and words start at the same column on every row; figures end
//! at the same column on every row, so the digits stack and a longer number is
//! visibly a bigger one. Sections are separated by one blank row.

pub mod common;

use std::path::PathBuf;
use std::time::Instant;

use common::Fixture;
use common::purge::{Recorder, candidate, confirmed};
use dev_cleaner::classify::{Activity, Ecosystem};
use dev_cleaner::config::Config;
use dev_cleaner::purge::execute;
use dev_cleaner::safety::{Candidate, Plan, RegenCommand, Reviewed, Safety};
use dev_cleaner::tui::{
    Candidates, Confirm, Dashboard, Group, KeyPress, ProjectSummary, Projects, Report, Review,
    Screen, Trend, Tui, collect, palette::Theme,
};
use dev_cleaner::volume::Volume;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;

const MB: u64 = 1024 * 1024;
const GB: u64 = 1024 * MB;

fn rows_of(draw: impl FnOnce(Rect, &mut Buffer)) -> Vec<Vec<char>> {
    let area = Rect::new(0, 0, 120, 40);
    let mut buf = Buffer::empty(area);
    draw(area, &mut buf);
    (0..area.height)
        .map(|y| {
            (0..area.width)
                .flat_map(|x| buf[(x, y)].symbol().chars().next())
                .collect()
        })
        .collect()
}

/// The column `needle` starts at in `row`, counted in cells.
fn start(row: &[char], needle: &str) -> Option<usize> {
    let needle: Vec<char> = needle.chars().collect();
    (0..row.len().saturating_sub(needle.len() - 1))
        .find(|&i| row[i..i + needle.len()] == needle[..])
}

/// The column `needle` ends at.
fn end(row: &[char], needle: &str) -> Option<usize> {
    start(row, needle).map(|s| s + needle.chars().count())
}

/// The row that holds `needle`, and fails the test if none does.
fn row_with<'a>(rows: &'a [Vec<char>], needle: &str) -> &'a Vec<char> {
    rows.iter()
        .find(|r| start(r, needle).is_some())
        .unwrap_or_else(|| {
            let shown: Vec<String> = rows.iter().map(|r| r.iter().collect()).collect();
            panic!("{needle:?} is on no row:\n{}", shown.join("\n"))
        })
}

/// All of `columns` start together: every needle at the same column.
fn all_start_together(rows: &[Vec<char>], needles: &[&str], what: &str) {
    let at: Vec<usize> = needles
        .iter()
        .map(|n| start(row_with(rows, n), n).expect("found"))
        .collect();
    assert!(
        at.windows(2).all(|w| w[0] == w[1]),
        "{what} start at {at:?}"
    );
}

/// All of `needles` end together.
fn all_end_together(rows: &[Vec<char>], needles: &[&str], what: &str) {
    let at: Vec<usize> = needles
        .iter()
        .map(|n| end(row_with(rows, n), n).expect("found"))
        .collect();
    assert!(at.windows(2).all(|w| w[0] == w[1]), "{what} end at {at:?}");
}

#[test]
fn the_dashboards_breakdown_rows_line_up_row_by_row() {
    let group = |label: &str, bytes: u64, dirs: usize, regen: &str| Group {
        label: label.into(),
        ecosystem: Ecosystem::Node,
        regen: regen.into(),
        bytes,
        dirs,
        offerable_bytes: bytes,
        offerable_dirs: dirs,
    };
    let dash = Dashboard {
        volume: Some(Volume {
            total: 460 * GB,
            free: 68 * GB,
        }),
        reclaimable: 4 * GB + 512 * MB + 88 * 1024,
        trend: Trend::FirstScan,
        groups: vec![
            group("target", 4 * GB, 9, "cargo build"),
            group("node_modules", 512 * MB, 120, "npm install"),
            group("dist", 88 * 1024, 1, "npm run build"),
        ],
        ..Default::default()
    };
    let rows = rows_of(|a, b| dash.render(&Theme::ansi(), a, b));

    // Names start in one column, and the figures and counts end in one each,
    // whatever their length.
    all_start_together(&rows, &["target", "node_modules", "dist"], "names");
    all_end_together(&rows, &["4.00 GB", "512.00 MB", "88.00 KB"], "sizes");
    all_end_together(&rows, &["9 dirs", "120 dirs", "1 dir"], "counts");
    all_start_together(
        &rows,
        &["cargo build", "npm install", "npm run build"],
        "commands",
    );
}

#[test]
fn the_projects_table_starts_names_together_and_ends_figures_together() {
    let project = |name: &str, unique: u64, inodes: u64, reclaimable: u64| ProjectSummary {
        path: PathBuf::from(format!("/p/{name}")),
        bytes_apparent: unique,
        bytes_unique: unique,
        inodes,
        activity: Activity::Active,
        reclaimable,
    };
    let table = Projects::new(vec![
        project("a", 3 * GB, 10, 2 * GB),
        project("bb", 20 * MB, 5_000, 900 * 1024),
        project("ccc", 700 * MB, 900_000, 50 * MB),
    ]);
    let rows = rows_of(|a, b| table.render(&Theme::ansi(), a, b));

    all_start_together(&rows, &["project", "a  ", "bb ", "ccc"], "names");
    all_end_together(
        &rows,
        &["3.00 GB", "700.00 MB", "20.00 MB"],
        "the unique column",
    );
    all_end_together(&rows, &["10", "5000", "900000"], "inodes");
    all_end_together(
        &rows,
        &["2.00 GB", "50.00 MB", "900.00 KB"],
        "the reclaimable column",
    );
}

fn regenerable(path: &str, bytes: u64) -> Candidate {
    Candidate {
        path: PathBuf::from(path),
        bytes,
        safety: Safety::Regenerable {
            regen: RegenCommand::new("npm install").expect("valid"),
        },
    }
}

fn three() -> Vec<Candidate> {
    vec![
        regenerable("/p/a/node_modules", 3 * GB),
        regenerable("/p/much-longer-name/node_modules", 20 * MB),
        regenerable("/q/target", 700 * MB),
    ]
}

#[test]
fn the_candidates_screen_lines_up_sizes_paths_and_commands() {
    let screen = Candidates::new(three(), Vec::new());
    let rows = rows_of(|a, b| screen.render(&Theme::ansi(), a, b));

    all_end_together(&rows, &["3.00 GB", "700.00 MB", "20.00 MB"], "sizes");
    all_start_together(
        &rows,
        &[
            "/p/a/node_modules",
            "/p/much-longer-name/node_modules",
            "/q/target",
        ],
        "paths",
    );
    let commands: Vec<usize> = rows
        .iter()
        .filter_map(|r| start(r, "npm install"))
        .collect();
    assert_eq!(commands.len(), 3);
    assert!(
        commands.windows(2).all(|w| w[0] == w[1]),
        "commands: {commands:?}"
    );
}

fn reviewed() -> Plan<Reviewed> {
    let mut draft = Plan::draft();
    for c in three() {
        draft.add(c).expect("selectable");
    }
    draft.review()
}

#[test]
fn the_plan_lines_up_sizes_paths_and_commands() {
    let plan = reviewed();
    let review = Review::new();
    let rows = rows_of(|a, b| review.render(&Theme::ansi(), &plan, a, b));

    all_end_together(&rows, &["3.00 GB", "700.00 MB", "20.00 MB"], "sizes");
    all_start_together(
        &rows,
        &[
            "/p/a/node_modules",
            "/p/much-longer-name/node_modules",
            "/q/target",
        ],
        "paths",
    );
}

#[test]
fn the_confirm_screen_lists_the_plan_the_way_the_plan_does() {
    let plan = reviewed();
    let confirm = Confirm::new();
    let rows = rows_of(|a, b| confirm.render(&Theme::ansi(), &plan, a, b));

    all_end_together(&rows, &["3.00 GB", "700.00 MB", "20.00 MB"], "sizes");
    all_start_together(
        &rows,
        &[
            "/p/a/node_modules",
            "/p/much-longer-name/node_modules",
            "/q/target",
        ],
        "paths",
    );
}

#[test]
fn the_result_screen_starts_every_value_under_its_label_in_one_column() {
    let manifest = execute(
        confirmed(vec![candidate("/p/a/node_modules", 1024)]),
        &Recorder::default(),
    );
    let rows = rows_of(|a, b| Report::new().render(&Theme::ansi(), &manifest, None, a, b));

    let value = |label: &str| {
        let row = row_with(&rows, label);
        let after = end(row, label).expect("found");
        (after..row.len())
            .find(|&i| row[i] != ' ')
            .unwrap_or_else(|| panic!("{label:?} has no value"))
    };
    let at = [
        value("Planned"),
        value("Moved"),
        value("Waiting in the Trash"),
    ];
    assert!(
        at.windows(2).all(|w| w[0] == w[1]),
        "values start at {at:?}"
    );
}

/// A screen and a fixture to open the key list on.
fn scan(fx: &Fixture, store: &Fixture) -> dev_cleaner::tui::Screens {
    fx.file("a/package.json", b"{}");
    fx.file("a/node_modules/dep/blob.bin", &[1u8; 4096]);
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

#[test]
fn the_key_list_puts_every_label_in_one_column() {
    let (fx, store) = (Fixture::new(), Fixture::new());
    let mut tui = Tui::new(scan(&fx, &store));
    let now = Instant::now();
    tui.press(KeyPress::Enter, now);
    tui.press(KeyPress::Char('?'), now);
    let rows = rows_of(|a, b| tui.render(a, b));

    all_start_together(
        &rows,
        &["up a row", "down a row", "first", "last", "quit", "keys"],
        "labels",
    );
    assert_eq!(tui.app().screen(), Screen::Projects);
}

#[test]
fn sections_are_separated_by_one_blank_row_everywhere() {
    // A heading is a row with a rule after it; the row above one is blank, and
    // never two blanks in a row.
    let (fx, store) = (Fixture::new(), Fixture::new());
    let mut tui = Tui::new(scan(&fx, &store));
    let now = Instant::now();
    for screen in [Screen::Dashboard, Screen::Projects, Screen::Candidates] {
        while tui.app().screen() != screen {
            tui.press(KeyPress::Enter, now);
        }
        let rows = rows_of(|a, b| tui.render(a, b));
        for (y, row) in rows.iter().enumerate().skip(3) {
            if row.iter().filter(|c| **c == '─').count() > 10 {
                let above: String = rows[y - 1].iter().collect();
                assert!(
                    above.trim().is_empty() || y <= 3,
                    "{screen:?}: no blank row above the heading on row {y}"
                );
            }
        }
    }
}
