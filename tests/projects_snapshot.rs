//! The Projects table, pinned cell by cell.
//!
//! The table is the look the other table screens are built to match, and it was
//! ported onto the shared primitives with no visible change. These goldens were
//! taken from the table before the port: every cell's symbol and style, in the
//! default view and after `f`, `r` and `7`, at the three sizes the screens are
//! reviewed at. Set `DEV_CLEANER_UPDATE_SNAPSHOTS=1` to rewrite them on purpose.

use dev_cleaner::classify::{Activity, Checkout, Kind};
use dev_cleaner::tui::{Column, ProjectSummary, Projects, Tally, palette::Theme};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use std::collections::BTreeMap;
use std::fmt::Write;
use std::path::PathBuf;

const MB: u64 = 1024 * 1024;

fn project(
    path: &str,
    apparent: u64,
    unique: u64,
    activity: Activity,
    reclaimable: u64,
    checkout: Checkout,
) -> ProjectSummary {
    ProjectSummary {
        path: PathBuf::from(path),
        bytes_apparent: apparent,
        bytes_unique: unique,
        inodes: unique / 4096 + 7,
        activity,
        reclaimable,
        checkout,
    }
}

fn worktree(repo: &str, name: &str, branch: &str) -> Checkout {
    Checkout {
        kind: Kind::Worktree,
        repo: Some(PathBuf::from(repo)),
        worktree: Some(name.to_string()),
        branch: Some(branch.to_string()),
        ..Checkout::default()
    }
}

fn table() -> Projects {
    let main = Checkout {
        kind: Kind::Main,
        repo: Some(PathBuf::from("/w/kyte-app")),
        branch: Some("main".to_string()),
        linked: 2,
        ..Checkout::default()
    };
    let orphan = Checkout {
        kind: Kind::Orphan,
        worktree: Some("gone".to_string()),
        ..Checkout::default()
    };
    Projects::new(vec![
        project(
            "/w/kyte-app/app/ios",
            2100 * MB,
            2100 * MB,
            Activity::Active,
            1400 * MB,
            main,
        ),
        project(
            "/w/wt/issue-942/app/ios",
            1900 * MB,
            1700 * MB,
            Activity::Active,
            1200 * MB,
            worktree(
                "/w/kyte-app",
                "issue-942",
                "feat/942-a-rather-long-branch-name",
            ),
        ),
        project(
            "/w/wt/issue-7/app/ios",
            800 * MB,
            800 * MB,
            Activity::Dormant,
            300 * MB,
            worktree("/w/kyte-app", "issue-7", "issue-7"),
        ),
        project(
            "/w/dev-cleaner",
            600 * MB,
            600 * MB,
            Activity::Active,
            564 * MB,
            Checkout::default(),
        ),
        project(
            "/w/notes",
            3 * MB,
            3 * MB,
            Activity::Dead,
            0,
            Checkout::default(),
        ),
        project(
            "/w/old/very-long-project-name-that-needs-the-name-column",
            120 * MB,
            40 * MB,
            Activity::Dead,
            90 * MB,
            orphan,
        ),
    ])
}

fn marks() -> BTreeMap<PathBuf, Tally> {
    let mut m = BTreeMap::new();
    m.insert(
        PathBuf::from("/w/dev-cleaner"),
        Tally {
            offered: (1, 564 * MB),
            marked: (1, 564 * MB),
        },
    );
    m.insert(
        PathBuf::from("/w/kyte-app/app/ios"),
        Tally {
            offered: (3, 1400 * MB),
            marked: (1, 400 * MB),
        },
    );
    m.insert(
        PathBuf::from("/w/wt/issue-7/app/ios"),
        Tally {
            offered: (2, 300 * MB),
            marked: (0, 0),
        },
    );
    m
}

/// Every cell as `symbol`, then a line per row of the styles in runs.
fn dump(t: &Projects, theme: &Theme, w: u16, h: u16) -> String {
    let area = Rect::new(0, 0, w, h);
    let mut buf = Buffer::empty(area);
    t.render(theme, area, &mut buf);
    let mut out = String::new();
    for y in 0..h {
        let text: String = (0..w).map(|x| buf[(x, y)].symbol()).collect();
        writeln!(out, "{y:>2}|{text}|").unwrap();
        let mut run: Option<(u16, String)> = None;
        let mut runs = Vec::new();
        for x in 0..w {
            let c = &buf[(x, y)];
            let s = format!("{:?}/{:?}/{:?}", c.fg, c.bg, c.modifier);
            match &run {
                Some((_, prev)) if *prev == s => {}
                _ => {
                    if let Some((from, prev)) = run.take() {
                        runs.push(format!("{from}-{}:{prev}", x - 1));
                    }
                    run = Some((x, s));
                }
            }
        }
        if let Some((from, prev)) = run {
            runs.push(format!("{from}-{}:{prev}", w - 1));
        }
        writeln!(out, "  {}", runs.join(" ")).unwrap();
    }
    out
}

fn states(theme: &Theme, w: u16, h: u16) -> String {
    let mut out = String::new();
    let mut t = table();
    writeln!(out, "## default, no marks known").unwrap();
    out += &dump(&t, theme, w, h);
    t.set_marks(marks());
    writeln!(out, "## default, marks").unwrap();
    out += &dump(&t, theme, w, h);
    t.cycle_filter();
    writeln!(out, "## f: removable only").unwrap();
    out += &dump(&t, theme, w, h);
    t.cycle_filter();
    writeln!(out, "## f f: marked only").unwrap();
    out += &dump(&t, theme, w, h);
    t.cycle_filter();
    t.cycle_filter();
    t.sort_by(Column::Repo);
    writeln!(out, "## 7: by repo").unwrap();
    out += &dump(&t, theme, w, h);
    t.sort_by(Column::Name);
    t.down();
    t.down();
    writeln!(out, "## by name, cursor on third").unwrap();
    out += &dump(&t, theme, w, h);
    t.reset_view();
    writeln!(out, "## r: reset").unwrap();
    out += &dump(&t, theme, w, h);
    let mut empty = Projects::new(Vec::new());
    empty.set_marks(BTreeMap::new());
    writeln!(out, "## no projects").unwrap();
    out += &dump(&empty, theme, w, h);
    out
}

fn check(name: &str, theme: &Theme, w: u16, h: u16) {
    let got = states(theme, w, h);
    let path = format!("{}/tests/snapshots/{name}.txt", env!("CARGO_MANIFEST_DIR"));
    if std::env::var_os("DEV_CLEANER_UPDATE_SNAPSHOTS").is_some() {
        std::fs::write(&path, &got).unwrap();
        return;
    }
    let want =
        std::fs::read_to_string(&path).expect("golden missing: set DEV_CLEANER_UPDATE_SNAPSHOTS=1");
    if got != want {
        let first = got
            .lines()
            .zip(want.lines())
            .position(|(a, b)| a != b)
            .unwrap_or(0);
        panic!(
            "{name} drifted from its golden at line {}:\n got: {}\nwant: {}",
            first + 1,
            got.lines().nth(first).unwrap_or(""),
            want.lines().nth(first).unwrap_or("")
        );
    }
}

#[test]
fn projects_80x24_is_unchanged() {
    check("projects_80x24", &Theme::neon(), 80, 24);
}

#[test]
fn projects_100x34_is_unchanged() {
    check("projects_100x34", &Theme::neon(), 100, 34);
}

#[test]
fn projects_160x40_is_unchanged() {
    check("projects_160x40", &Theme::neon(), 160, 40);
}

#[test]
fn projects_without_colour_is_unchanged() {
    check("projects_mono_100x34", &Theme::mono(), 100, 34);
}

#[test]
fn projects_in_256_colours_is_unchanged() {
    check("projects_ansi_100x34", &Theme::ansi(), 100, 34);
}
