//! Telling projects apart: labels that never collide, the checkout badge, the
//! selected project in full, and the repo order.

mod common;

use common::Fixture;
use dev_cleaner::classify::{Activity, Checkout, Kind};
use dev_cleaner::tui::{Column, ProjectSummary, Projects, palette::Theme};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use std::collections::BTreeSet;
use std::path::PathBuf;

const MB: u64 = 1024 * 1024;

fn project(path: &str, checkout: Checkout) -> ProjectSummary {
    ProjectSummary {
        path: PathBuf::from(path),
        bytes_apparent: MB,
        bytes_unique: MB,
        inodes: 1,
        activity: Activity::Active,
        reclaimable: 0,
        checkout,
    }
}

fn plain(path: &str) -> ProjectSummary {
    project(path, Checkout::default())
}

fn wt(path: &str, repo: &str, name: &str, branch: &str) -> ProjectSummary {
    project(
        path,
        Checkout {
            kind: Kind::Worktree,
            repo: Some(PathBuf::from(repo)),
            worktree: Some(name.to_string()),
            branch: Some(branch.to_string()),
            ..Checkout::default()
        },
    )
}

fn main_of(path: &str, linked: usize) -> ProjectSummary {
    project(
        path,
        Checkout {
            kind: Kind::Main,
            repo: Some(PathBuf::from(path)),
            branch: Some("main".to_string()),
            linked,
            ..Checkout::default()
        },
    )
}

fn frame(t: &Projects, w: u16, h: u16) -> Vec<String> {
    let area = Rect::new(0, 0, w, h);
    let mut buf = Buffer::empty(area);
    t.render(&Theme::ansi(), area, &mut buf);
    (0..h)
        .map(|y| {
            (0..w)
                .map(|x| buf[(x, y)].symbol())
                .collect::<String>()
                .trim_end()
                .to_string()
        })
        .collect()
}

/// The name cell of every row, as drawn: what stands before the figures.
fn name_cells(t: &Projects, w: u16) -> Vec<String> {
    let h = t.rows().len() as u16 + 3;
    frame(t, w, h)
        .into_iter()
        .skip(1)
        .take(t.rows().len())
        .map(|l| {
            l.split("  ")
                .map(str::trim)
                .find(|c| !c.is_empty() && *c != "◆" && *c != "⎇" && *c != "⌀")
                .unwrap_or("")
                .to_string()
        })
        .collect()
}

#[test]
fn the_same_project_in_four_worktrees_gets_four_different_labels() {
    // The owner's screen: `app/ios` x4, one per worktree, whose parent is `app`
    // in every one, so one level of qualification changed nothing.
    let t = Projects::new(vec![
        plain("/k/wt-a/app/ios"),
        plain("/k/wt-b/app/ios"),
        plain("/k/wt-c/app/ios"),
        plain("/k/wt-d/app/ios"),
    ]);
    let labels: BTreeSet<String> = t.rows().iter().map(|r| t.label(r).to_string()).collect();
    assert_eq!(labels.len(), 4, "{labels:?}");
    assert!(labels.contains("wt-a/app/ios"), "{labels:?}");
}

#[test]
fn a_label_is_only_as_long_as_it_has_to_be() {
    let t = Projects::new(vec![
        plain("/k/wt-a/app/ios"),
        plain("/k/wt-b/app/ios"),
        plain("/k/solo"),
        plain("/k/x/web"),
        plain("/k/y/web"),
    ]);
    let label = |p: &str| {
        t.label(
            t.rows()
                .iter()
                .find(|r| r.path.as_path() == std::path::Path::new(p))
                .unwrap(),
        )
        .to_string()
    };
    assert_eq!(label("/k/solo"), "solo");
    assert_eq!(label("/k/x/web"), "x/web");
    assert_eq!(label("/k/wt-a/app/ios"), "wt-a/app/ios");
}

#[test]
fn a_worktree_label_carries_its_branch_and_the_main_checkout_says_main() {
    let t = Projects::new(vec![
        main_of("/k/app", 2),
        wt("/k/wt/issue-942/app", "/k/app", "issue-942", "feat/942"),
        main_of("/k/alone", 0),
    ]);
    let notes: Vec<Option<String>> = t.rows().iter().map(|r| t.note(r)).collect();
    assert!(notes.contains(&Some("◆ main".to_string())), "{notes:?}");
    assert!(notes.contains(&Some("⎇ feat/942".to_string())), "{notes:?}");
    assert_eq!(
        notes.iter().filter(|n| n.is_none()).count(),
        1,
        "a repository with no worktrees needs no note: {notes:?}"
    );
}

#[test]
fn no_two_of_sixty_projects_with_twelve_repeated_names_read_alike_at_80_columns() {
    // Twelve names, each repeated across five worktrees, in two nesting depths.
    let names = [
        "ios",
        "android",
        "web",
        "api",
        "docs",
        "fastlane",
        "detox",
        "robot",
        "deps",
        "shared",
        "app",
        "backoffice",
    ];
    let mut rows = Vec::new();
    for (i, name) in names.iter().enumerate() {
        for w in 0..5 {
            let wtree = format!("kyte-wt-{w:02}");
            let path = if i % 2 == 0 {
                format!("/Users/carlos/kyte/{wtree}/app/{name}")
            } else {
                format!("/Users/carlos/kyte/{wtree}/{name}")
            };
            rows.push(wt(
                &path,
                "/Users/carlos/kyte/main",
                &wtree,
                &format!("feat/{i}-{w}"),
            ));
        }
    }
    assert_eq!(rows.len(), 60);
    let t = Projects::new(rows);

    for width in [80u16, 100, 160] {
        let cells = name_cells(&t, width);
        let unique: BTreeSet<&String> = cells.iter().collect();
        assert_eq!(
            unique.len(),
            60,
            "at {width} columns two rows read alike: {cells:#?}"
        );
    }
    // The distinguishing segment survives at 80 columns: every drawn name still
    // carries its worktree directory, the first segment of its suffix.
    for (cell, row) in name_cells(&t, 80).iter().zip(t.rows()) {
        let wtree = row.checkout.worktree.as_deref().unwrap();
        assert!(
            cell.contains(wtree),
            "{cell:?} lost {wtree:?} at 80 columns"
        );
    }
}

#[test]
fn a_long_label_loses_its_middle_never_its_ends() {
    let t = Projects::new(vec![
        plain("/k/a-very-long-worktree-directory-name-here/some/deep/place/ios"),
        plain("/k/another-very-long-worktree-directory-name/some/deep/place/ios"),
    ]);
    for cell in name_cells(&t, 80) {
        assert!(cell.ends_with("ios"), "{cell:?}");
        assert!(cell.contains('…') || cell.chars().count() <= 24, "{cell:?}");
    }
    let cells = name_cells(&t, 80);
    assert_ne!(cells[0], cells[1]);
}

#[test]
fn the_selected_project_is_given_in_full_at_80_and_200_columns() {
    let row = wt(
        "/Users/carlos/kyte/kyte-wt-02/app/ios",
        "/Users/carlos/kyte/main",
        "kyte-wt-02",
        "feat/942-login",
    );
    let t = Projects::new(vec![row]);
    for width in [80u16, 200] {
        let line = frame(&t, width, 6)[4].clone();
        for part in ["worktree kyte-wt-02", "of main", "feat/942-login"] {
            assert!(
                line.contains(part),
                "at {width}: {part:?} missing from {line:?}"
            );
        }
        if width == 200 {
            assert!(
                line.contains("/Users/carlos/kyte/kyte-wt-02/app/ios"),
                "{line}"
            );
        }
        assert!(line.chars().count() <= width as usize);
    }
}

#[test]
fn the_detail_names_what_the_label_was_built_from() {
    let orphan = project(
        "/k/gone/app",
        Checkout {
            kind: Kind::Orphan,
            repo: Some(PathBuf::from("/k/main")),
            worktree: Some("gone".to_string()),
            ..Checkout::default()
        },
    );
    let t = Projects::new(vec![orphan]);
    let line = t.detail(&t.rows()[0], 200);
    assert!(
        line.contains("orphan") && line.contains("gone") && line.contains("/k/gone/app"),
        "{line}"
    );
}

#[test]
fn the_badge_column_and_its_legend_exist_only_where_a_project_is_in_a_repository() {
    let none = Projects::new(vec![plain("/k/a"), plain("/k/b")]);
    assert!(!frame(&none, 120, 8).join("\n").contains("worktree"));

    let some = Projects::new(vec![plain("/k/a"), main_of("/k/b", 1)]);
    let out = frame(&some, 120, 8).join("\n");
    assert!(
        out.contains("◆ main checkout") && out.contains("⎇ worktree") && out.contains("⌀ orphan"),
        "{out}"
    );
}

#[test]
fn the_badge_column_goes_whole_when_the_width_cannot_hold_it() {
    let t = Projects::new(vec![main_of("/k/b", 1), wt("/k/w/b", "/k/b", "w", "f")]);
    for width in [30u16, 40, 50] {
        for line in frame(&t, width, 8).iter().take(3) {
            // A badge is a whole glyph in its own cell or it is not there at all.
            let first = line.trim_start().chars().next();
            if line.starts_with("  ") || line.starts_with(' ') {
                assert!(
                    !matches!(first, Some('◆' | '⎇' | '⌀'))
                        || line.starts_with(" ◆ ")
                        || line.starts_with(" ⎇ ")
                        || line.starts_with(" ⌀ "),
                    "{line:?}"
                );
            }
        }
    }
}

#[test]
fn sorting_by_repo_keeps_each_repositorys_worktrees_together_main_first() {
    let mut t = Projects::new(vec![
        wt("/k/wt/b2/app", "/k/b", "b2", "x"),
        plain("/k/loose"),
        wt("/k/wt/a1/app", "/k/a", "a1", "x"),
        main_of("/k/b", 2),
        wt("/k/wt/b1/app", "/k/b", "b1", "x"),
        main_of("/k/a", 1),
    ]);
    t.sort_by(Column::Repo);
    let order: Vec<&str> = t.rows().iter().map(|r| r.path.to_str().unwrap()).collect();
    assert_eq!(
        order,
        [
            "/k/a",
            "/k/wt/a1/app",
            "/k/b",
            "/k/wt/b1/app",
            "/k/wt/b2/app",
            "/k/loose"
        ]
    );
    assert_eq!(
        t.ordering(),
        "repo, each repository's worktrees together, main first"
    );
}

#[test]
fn a_real_repository_with_worktrees_scans_to_a_labelled_table() {
    // End to end over real `git worktree add` output: the scan finds the
    // projects, classifies them, and the table tells them apart.
    use dev_cleaner::config::Config;
    use dev_cleaner::tui::collect;

    let fx = Fixture::new();
    fx.git_repo("main/kyte", 3);
    fx.file("main/kyte/app/ios/Podfile", b"");
    fx.git_worktree("main/kyte", "wt/issue-942/kyte", "feat/942");
    fx.file("wt/issue-942/kyte/app/ios/Podfile", b"");
    fx.git_worktree("main/kyte", "wt/hotfix/kyte", "hotfix");
    fx.file("wt/hotfix/kyte/app/ios/Podfile", b"");
    for i in 0..5 {
        fx.file(&format!("wt/hotfix/kyte/app/ios/__pycache__/{i}.pyc"), b"x");
    }
    let store = Fixture::new();
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
    let t = &screens.projects;
    let ios: Vec<_> = t.rows().iter().filter(|r| r.name() == "ios").collect();
    assert_eq!(ios.len(), 3);
    let labels: BTreeSet<&str> = ios.iter().map(|r| t.label(r)).collect();
    assert_eq!(labels.len(), 3, "{labels:?}");
    let kinds: Vec<Kind> = ios.iter().map(|r| r.checkout.kind).collect();
    assert_eq!(
        kinds.iter().filter(|k| **k == Kind::Worktree).count(),
        2,
        "{kinds:?}"
    );
    assert_eq!(
        kinds.iter().filter(|k| **k == Kind::Main).count(),
        1,
        "{kinds:?}"
    );
    assert_eq!(screens.dashboard.analysed.worktrees, 2);
    assert_eq!(screens.dashboard.analysed.repos, 1);

    // Classifying changes what is shown, not what is offered: the entries the
    // candidates screen holds are the artifact directories, whatever checkout
    // they sit in. (`scan` over the same tree prints byte-identical output
    // before and after; the PR pastes both.)
    let held: BTreeSet<PathBuf> = screens
        .candidates
        .selectable()
        .iter()
        .map(|c| c.path.clone())
        .chain(screens.candidates.blocked().iter().map(|b| b.path.clone()))
        .collect();
    let expected: BTreeSet<PathBuf> = ["main/kyte", "wt/issue-942/kyte", "wt/hotfix/kyte"]
        .iter()
        .map(|d| fx.root().join(d).join("app/ios/__pycache__"))
        .filter(|p| p.exists())
        .collect();
    assert!(held.is_subset(&expected), "{held:?} vs {expected:?}");

    // Key 7 orders by repo, main checkout first, and says so.
    let mut tui = dev_cleaner::tui::Tui::new(collect(
        &[fx.root().to_path_buf()],
        &cfg,
        fx.root(),
        &store.root().join("history2.sqlite3"),
    ))
    .with_theme(Theme::ansi());
    let now = std::time::Instant::now();
    tui.press(dev_cleaner::tui::KeyPress::Enter, now);
    tui.press(dev_cleaner::tui::KeyPress::Char('7'), now);
    let area = Rect::new(0, 0, 160, 24);
    let mut buf = Buffer::empty(area);
    tui.render(area, &mut buf);
    let shown: String = (0..area.height)
        .map(|y| {
            (0..area.width)
                .map(|x| buf[(x, y)].symbol())
                .collect::<String>()
                + "\n"
        })
        .collect();
    assert!(shown.contains("Sorted by repo"), "{shown}");
    let first = shown
        .lines()
        .find(|l| l.contains("/ios") || l.contains("app/ios"))
        .unwrap();
    assert!(
        first.contains("main"),
        "the main checkout leads its repository: {first}"
    );

    // The dashboard counts them, and names the directory with most files by the
    // project the table names, not by a path that fits every worktree.
    let area = Rect::new(0, 0, 140, 40);
    let mut buf = Buffer::empty(area);
    screens.dashboard.render(&Theme::ansi(), area, &mut buf);
    let out: String = (0..area.height)
        .map(|y| {
            (0..area.width)
                .map(|x| buf[(x, y)].symbol())
                .collect::<String>()
                + "\n"
        })
        .collect();
    assert!(out.contains("2 linked worktrees of 1 repo"), "{out}");
    let hot = t
        .rows()
        .iter()
        .find(|r| r.path.to_string_lossy().contains("hotfix"))
        .unwrap();
    assert!(
        out.contains(&format!("{}/__pycache__", t.label(hot))),
        "{out}"
    );
}
