//! The candidates screen in the projects table's look: a header row with the
//! sort arrow on the sorted column, aligned columns, the project and the path
//! inside it, a section head per list, the held-back entries grouped by reason,
//! and the selected entry in full.

use dev_cleaner::classify::{Activity, Checkout, Kind};
use dev_cleaner::safety::{Candidate, RegenCommand, Rejected, Safety};
use dev_cleaner::tui::{Candidates, Key, Order, ProjectSummary, Projects, palette::Theme};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use std::path::PathBuf;

const MB: u64 = 1024 * 1024;
const UNTRACKED: &str = "Untracked source files here exist nowhere else.";

fn project(path: &str, checkout: Checkout) -> ProjectSummary {
    ProjectSummary {
        path: PathBuf::from(path),
        bytes_apparent: MB,
        bytes_unique: MB,
        inodes: 1,
        activity: Activity::Active,
        reclaimable: MB,
        checkout,
    }
}

fn worktree(name: &str, branch: &str) -> Checkout {
    Checkout {
        kind: Kind::Worktree,
        repo: Some(PathBuf::from("/w/kyte-app")),
        worktree: Some(name.to_string()),
        branch: Some(branch.to_string()),
        ..Checkout::default()
    }
}

fn projects() -> Projects {
    Projects::new(vec![
        project("/w/dev-cleaner", Checkout::default()),
        project(
            "/w/kyte-app",
            Checkout {
                kind: Kind::Main,
                repo: Some(PathBuf::from("/w/kyte-app")),
                branch: Some("main".into()),
                linked: 1,
                ..Checkout::default()
            },
        ),
        project("/w/wt/issue-942", worktree("issue-942", "issue-942")),
    ])
}

fn entry(path: &str, bytes: u64, regen: &str) -> Candidate {
    Candidate {
        path: PathBuf::from(path),
        bytes,
        safety: Safety::Regenerable {
            regen: RegenCommand::new(regen).unwrap(),
        },
    }
}

fn screen(extra: Vec<Candidate>, held: usize) -> Candidates {
    let mut entries = vec![
        entry("/w/dev-cleaner/target", 564 * MB, "cargo build"),
        entry("/w/kyte-app/app/ios/Pods", 300 * MB, "pod install"),
        entry("/w/wt/issue-942/node_modules", 40 * MB, "npm install"),
    ];
    entries.extend(extra);
    let rejected = (0..held)
        .map(|i| Rejected {
            path: PathBuf::from(format!("/w/wt/issue-942/packages/p{i}/vendor")),
            because: UNTRACKED.to_string(),
        })
        .collect();
    let mut c = Candidates::new(entries, rejected);
    c.set_locator(projects().locator());
    c
}

fn frame(c: &Candidates, w: u16, h: u16) -> Vec<String> {
    let area = Rect::new(0, 0, w, h);
    let mut buf = Buffer::empty(area);
    c.render(&Theme::ansi(), area, &mut buf);
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

fn col(line: &str, needle: &str) -> usize {
    let at = line
        .find(needle)
        .unwrap_or_else(|| panic!("{needle:?} not in {line:?}"));
    line[..at].chars().count()
}

const SIZES: [(u16, u16); 3] = [(80, 24), (100, 34), (160, 40)];

#[test]
fn the_header_row_names_the_columns_and_the_sorted_one_carries_the_arrow() {
    for (w, h) in SIZES {
        let lines = frame(&screen(vec![], 0), w, h);
        let header = lines
            .iter()
            .find(|l| l.contains("project / path"))
            .unwrap_or_else(|| panic!("{w}: no header row in {lines:#?}"));
        assert!(header.contains("size ▼"), "{w}: {header:?}");
        assert!(header.contains("project / path"), "{w}: {header:?}");
        assert!(
            !header.contains("project / path ▲"),
            "{w}: only the sorted one"
        );
        if w >= 100 {
            assert!(header.contains("kind"), "{w}: {header:?}");
            assert!(header.contains("comes back as"), "{w}: {header:?}");
        }
    }
}

#[test]
fn a_sort_key_moves_the_arrow_to_its_column() {
    let mut c = screen(vec![], 0);
    c.press(Key::Sort(Order::Path), 24);
    let lines = frame(&c, 100, 34);
    let header = lines.iter().find(|l| l.contains("project / path")).unwrap();
    assert!(header.contains("project / path ▲"), "{header:?}");
    assert!(
        !header.contains("size ▼") && !header.contains("size ▲"),
        "{header:?}"
    );
    c.press(Key::Sort(Order::Kind), 24);
    let lines = frame(&c, 100, 34);
    let header = lines.iter().find(|l| l.contains("project / path")).unwrap();
    assert!(header.contains("kind ▲"), "{header:?}");
}

#[test]
fn the_order_is_said_once_in_the_view_bar_and_not_again_in_the_head() {
    for (w, h) in SIZES {
        let lines = frame(&screen(vec![], 0), w, h);
        let said = lines.iter().filter(|l| l.contains("largest first")).count();
        assert_eq!(said, 1, "{w}: {lines:#?}");
        assert!(
            !lines
                .iter()
                .any(|l| l.contains("showing 1-3 of 3") && l.contains("rebuilt")),
            "{w}: the head and the position are two lines"
        );
    }
}

#[test]
fn every_column_starts_where_the_one_above_it_does() {
    for (w, h) in [(100, 34), (160, 40)] {
        let lines = frame(&screen(vec![], 0), w, h);
        let rows: Vec<&String> = lines.iter().filter(|l| l.contains("[ ]")).collect();
        assert_eq!(rows.len(), 3, "{w}: {lines:#?}");
        let header = lines.iter().find(|l| l.contains("project / path")).unwrap();
        // Sizes are figures: they end where the header's does.
        let size_end = col(header, "size ▼") + "size ▼".chars().count();
        for r in &rows {
            let unit = r.find(" MB").expect("a size in MB");
            let end = r[..unit].chars().count() + 3;
            assert_eq!(
                end, size_end,
                "{w}: size ends at {end}, header at {size_end}: {r:?}"
            );
        }
        // Text starts where the header does.
        for name in ["kind", "project / path", "comes back as"] {
            let at = col(header, name);
            for r in &rows {
                let cell = r.chars().nth(at).unwrap_or(' ');
                let before = r.chars().nth(at - 1).unwrap_or(' ');
                assert!(
                    cell != ' ' && before == ' ',
                    "{w}: {name} column starts at {at} but {r:?} has {before:?}{cell:?}"
                );
            }
        }
    }
}

#[test]
fn a_row_says_the_project_and_the_path_inside_it_and_never_the_absolute_path() {
    for (w, h) in SIZES {
        let lines = frame(&screen(vec![], 0), w, h);
        let row = |needle: &str| {
            lines
                .iter()
                .find(|l| l.contains("[ ]") && l.contains(needle))
                .unwrap_or_else(|| panic!("{w}: no row for {needle}: {lines:#?}"))
                .clone()
        };
        assert!(row("dev-cleaner").contains("dev-cleaner  target"), "{w}");
        let kyte = row("kyte-app");
        assert!(kyte.contains("kyte-app"), "{w}");
        assert!(kyte.contains("app/ios/Pods"), "{w}: {kyte:?}");
        let wt = row("issue-942");
        assert!(
            wt.contains("⎇ issue-942"),
            "{w}: the checkout's badge: {wt:?}"
        );
        assert!(wt.contains("node_modules"), "{w}");
        for l in lines.iter().filter(|l| l.contains("[ ]")) {
            assert!(!l.contains("/w/"), "{w}: absolute path in a row: {l:?}");
        }
    }
}

#[test]
fn a_row_carries_its_kind_icon_and_the_command_that_brings_it_back() {
    let lines = frame(&screen(vec![], 0), 100, 34);
    let target = lines
        .iter()
        .find(|l| l.contains("dev-cleaner  target"))
        .unwrap();
    assert!(
        target.contains("◆ target"),
        "the toolchain's icon: {target:?}"
    );
    assert!(target.contains("cargo build"), "{target:?}");
    let modules = lines.iter().find(|l| l.contains("issue-942")).unwrap();
    assert!(modules.contains("⬢ node_modules"), "{modules:?}");
    assert!(modules.contains("npm install"), "{modules:?}");
}

#[test]
fn the_selected_entry_is_said_in_full_on_the_detail_line() {
    for (w, h) in SIZES {
        let lines = frame(&screen(vec![], 0), w, h);
        let detail = &lines[lines.len() - 2];
        assert!(detail.contains("/w/dev-cleaner/target"), "{w}: {detail:?}");
        assert!(detail.contains("cargo build"), "{w}: {detail:?}");
        assert!(detail.contains("564.00 MB"), "{w}: {detail:?}");
        assert_eq!(
            lines
                .iter()
                .filter(|l| l.contains("/w/dev-cleaner/target"))
                .count(),
            1,
            "{w}: the absolute path is on the detail line only"
        );
    }
}

#[test]
fn the_position_is_the_last_line() {
    for (w, h) in SIZES {
        let lines = frame(&screen(vec![], 0), w, h);
        assert!(
            lines[lines.len() - 1].starts_with(" showing 1-3 of 3"),
            "{w}: {lines:#?}"
        );
    }
}

#[test]
fn a_long_path_is_cut_in_the_middle_and_keeps_the_directory_it_is() {
    let long = format!(
        "/w/kyte-app/{}/Pods",
        (0..8)
            .map(|i| format!("segment-number-{i}"))
            .collect::<Vec<_>>()
            .join("/")
    );
    assert!(long.chars().count() > 90);
    let lines = frame(
        &screen(vec![entry(&long, 5 * MB, "pod install")], 0),
        80,
        24,
    );
    let row = lines
        .iter()
        .find(|l| l.contains("[ ]") && l.contains("Pods") && l.contains("segment"))
        .unwrap_or_else(|| panic!("{lines:#?}"));
    assert!(row.contains("…"), "marked as cut: {row:?}");
    assert!(
        row.contains("/Pods"),
        "the last segment survives whole: {row:?}"
    );
    assert!(row.contains("kyte-app"), "and so does the project: {row:?}");
    assert!(row.chars().count() <= 80, "{row:?}");
}

#[test]
fn held_back_entries_are_grouped_under_their_reason_with_a_count() {
    for (w, h) in SIZES {
        let lines = frame(&screen(vec![], 40), w, h);
        let head = lines
            .iter()
            .find(|l| l.contains("Not offered"))
            .unwrap_or_else(|| panic!("{w}: {lines:#?}"));
        assert!(head.contains("40"), "{w}: {head:?}");
        let said = lines.iter().filter(|l| l.contains(UNTRACKED)).count();
        assert_eq!(
            said, 1,
            "{w}: the reason is said once, not once per entry: {lines:#?}"
        );
        let group = lines.iter().find(|l| l.contains(UNTRACKED)).unwrap();
        assert!(group.contains("40"), "{w}: the count: {group:?}");
        assert!(
            lines
                .iter()
                .any(|l| l.contains("issue-942") && l.contains("packages/p0/vendor")),
            "{w}: the entries follow, named by project: {lines:#?}"
        );
        for l in lines.iter().filter(|l| l.contains("vendor")) {
            assert!(!l.contains("/w/wt"), "{w}: {l:?}");
        }
    }
}

#[test]
fn what_does_not_fit_of_the_held_back_is_counted_and_not_dropped_in_silence() {
    let lines = frame(&screen(vec![], 40), 100, 24);
    assert!(
        lines.iter().any(|l| l.contains("more held back")),
        "{lines:#?}"
    );
}

#[test]
fn a_column_that_does_not_fit_goes_whole_and_is_named() {
    let lines = frame(&screen(vec![], 0), 80, 24);
    let header = lines.iter().find(|l| l.contains("project / path")).unwrap();
    assert!(
        !header.contains("ki…") && !header.contains("comes b…"),
        "{header:?}"
    );
    for name in ["kind", "comes back as"] {
        let here = header.contains(name);
        let said = lines.last().unwrap().contains(&format!("{name} hidden"))
            || lines.last().unwrap().contains("hidden at this width");
        assert!(here || said, "{name} neither drawn nor said: {lines:#?}");
    }
}

#[test]
fn offering_is_unchanged() {
    let c = screen(vec![], 40);
    assert_eq!(c.selectable().len(), 3);
    assert_eq!(c.blocked().len(), 40);
}

const DIRTY: &str = "Uncommitted changes are present in this repository.";

/// The owner's second case: six offered entries and forty held back across two
/// worktrees, for two different reasons.
fn busy() -> Candidates {
    let extra = vec![
        entry("/w/kyte-app/node_modules", 20 * MB, "npm install"),
        entry("/w/wt/issue-942/app/ios/Pods", 120 * MB, "pod install"),
        entry("/w/dev-cleaner/other/target", 8 * MB, "cargo build"),
    ];
    let mut c = screen(extra, 30);
    let mut all: Vec<Rejected> = c
        .blocked()
        .iter()
        .map(|b| Rejected {
            path: b.path.clone(),
            because: b.reason.clone(),
        })
        .collect();
    all.extend((0..10).map(|i| Rejected {
        path: PathBuf::from(format!("/w/kyte-app/packages/q{i}/vendor")),
        because: DIRTY.to_string(),
    }));
    let mut offered = c.selectable().to_vec();
    offered.truncate(6);
    c = Candidates::new(offered, all);
    c.set_locator(projects().locator());
    c
}

#[test]
fn six_entries_and_forty_held_back_read_as_two_lists_with_their_reasons_counted() {
    for (w, h) in [(100, 34), (160, 40)] {
        let lines = frame(&busy(), w, h);
        let text = lines.join("\n");
        assert_eq!(
            lines.iter().filter(|l| l.contains("[ ]")).count(),
            6,
            "{w}:\n{text}"
        );
        let head = lines
            .iter()
            .find(|l| l.contains("Not offered"))
            .expect("a head");
        assert!(head.contains("40 held back"), "{w}: {head:?}");
        let (untracked, dirty) = (
            lines
                .iter()
                .position(|l| l.contains(UNTRACKED))
                .expect("a group"),
            lines
                .iter()
                .position(|l| l.contains(DIRTY))
                .expect("a group"),
        );
        assert!(untracked < dirty, "{w}: the bigger group first:\n{text}");
        assert!(
            lines[untracked].ends_with("30 entries"),
            "{w}: {:?}",
            lines[untracked]
        );
        assert!(
            lines[dirty].ends_with("10 entries"),
            "{w}: {:?}",
            lines[dirty]
        );
        assert_eq!(
            lines.iter().filter(|l| l.contains(UNTRACKED)).count(),
            1,
            "{w}"
        );
        // A big group does not push the small one's entries off the screen.
        assert!(
            lines[dirty + 1..]
                .iter()
                .any(|l| l.contains("kyte-app") && l.contains("vendor")),
            "{w}: the second group lists some of its entries:\n{text}"
        );
        assert!(
            !text.contains("/w/wt/"),
            "{w}: absolute paths stay on the detail line:\n{text}"
        );
    }
}

#[test]
fn nothing_is_carried_by_colour_alone() {
    use dev_cleaner::tui::palette::Theme;
    for theme in [Theme::neon(), Theme::ansi(), Theme::mono()] {
        let mut c = busy();
        c.press(Key::Toggle, 24);
        let area = Rect::new(0, 0, 100, 34);
        let mut buf = Buffer::empty(area);
        c.render(&theme, area, &mut buf);
        let text: String = (0..34)
            .flat_map(|y| (0..100).map(move |x| (x, y)))
            .map(|(x, y)| buf[(x, y)].symbol().to_string())
            .collect();
        // Marked, sorted, the tier, and held back: each has a glyph or a word.
        for carrier in ["[x]", "size ▼", "+", "⊘", "Not offered", "held back"] {
            assert!(
                text.contains(carrier),
                "{carrier:?} is missing without colour"
            );
        }
    }
}

#[test]
fn the_same_look_in_every_theme_has_the_same_cells_of_text() {
    use dev_cleaner::tui::palette::Theme;
    let drawn = |theme: Theme| {
        let c = busy();
        let area = Rect::new(0, 0, 100, 34);
        let mut buf = Buffer::empty(area);
        c.render(&theme, area, &mut buf);
        (0..34)
            .map(|y| (0..100).map(|x| buf[(x, y)].symbol()).collect::<String>())
            .collect::<Vec<_>>()
    };
    assert_eq!(drawn(Theme::neon()), drawn(Theme::mono()));
    assert_eq!(drawn(Theme::neon()), drawn(Theme::ansi()));
}

#[test]
fn the_size_is_on_the_size_ramp() {
    use dev_cleaner::tui::palette::Theme;
    let theme = Theme::neon();
    let mut c = screen(
        vec![entry("/w/dev-cleaner/big", 2048 * MB, "cargo build")],
        0,
    );
    // Off the first row, which the cursor band paints in its own ink.
    c.press(Key::Down, 24);
    let area = Rect::new(0, 0, 100, 34);
    let mut buf = Buffer::empty(area);
    c.render(&theme, area, &mut buf);
    let ink = |needle: &str| {
        (0..34)
            .find_map(|y| {
                let row: String = (0..100).map(|x| buf[(x, y)].symbol()).collect();
                let x = row.find(needle).filter(|_| row.contains("[ ]"))?;
                Some(buf[(row[..x].chars().count() as u16, y)].fg)
            })
            .unwrap_or_else(|| panic!("no row with {needle}"))
    };
    assert_eq!(Some(ink("2.00 GB")), theme.size(2048 * MB).fg);
    assert_eq!(Some(ink("40.00 MB")), theme.size(40 * MB).fg);
    assert_ne!(ink("2.00 GB"), ink("40.00 MB"));
}
