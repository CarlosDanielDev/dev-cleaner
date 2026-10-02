//! The plan in the candidates table's look, grouped by project: a head per
//! project with its subtotal, the entries under it naming only the path inside
//! the project, a total at the foot, and nothing about what the plan holds
//! changed by being drawn this way.

use dev_cleaner::classify::{Activity, Checkout, Kind};
use dev_cleaner::safety::{Candidate, Plan, RegenCommand, Reviewed, Safety};
use dev_cleaner::tui::{Locator, Motion, ProjectSummary, Projects, Review, palette::Theme};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use std::path::PathBuf;

const MB: u64 = 1024 * 1024;

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

fn locator() -> Locator {
    Projects::new(vec![
        project("/w/dev-cleaner", Checkout::default()),
        project(
            "/w/wt/issue-942",
            Checkout {
                kind: Kind::Worktree,
                repo: Some(PathBuf::from("/w/kyte-app")),
                worktree: Some("issue-942".into()),
                branch: Some("issue-942".into()),
                ..Checkout::default()
            },
        ),
    ])
    .locator()
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

fn plan_of(items: Vec<Candidate>) -> Plan<Reviewed> {
    let mut draft = Plan::draft();
    for c in items {
        draft.add(c).expect("selectable");
    }
    draft.review()
}

/// Two projects, the smaller one's entries listed first, so a plan shown in its
/// own order would interleave them.
fn plan() -> Plan<Reviewed> {
    plan_of(vec![
        entry("/w/wt/issue-942/node_modules", 40 * MB, "npm install"),
        entry("/w/dev-cleaner/target", 564 * MB, "cargo build"),
        entry("/w/wt/issue-942/app/ios/Pods", 300 * MB, "pod install"),
    ])
}

fn frame(plan: &Plan<Reviewed>, review: &Review, w: u16, h: u16) -> Vec<String> {
    let area = Rect::new(0, 0, w, h);
    let mut buf = Buffer::empty(area);
    review.render(&Theme::ansi(), plan, &locator(), area, &mut buf);
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

const SIZES: [(u16, u16); 3] = [(80, 24), (100, 34), (160, 40)];

#[test]
fn the_plan_is_grouped_by_project_biggest_subtotal_first() {
    for (w, h) in SIZES {
        let lines = frame(&plan(), &Review::new(), w, h);
        let at = |needle: &str| {
            lines
                .iter()
                .position(|l| l.contains(needle))
                .unwrap_or_else(|| panic!("{w}: {needle:?} not drawn:\n{}", lines.join("\n")))
        };
        let (dev, wt) = (at("dev-cleaner  1 entry"), at("issue-942"));
        assert!(dev < wt, "{w}: 564 MB before 340 MB");
        let head = &lines[wt];
        assert!(
            head.contains("⎇ issue-942"),
            "{w}: the checkout's badge: {head:?}"
        );
        assert!(
            head.contains("2 entries · 340.00 MB"),
            "{w}: the subtotal: {head:?}"
        );
        assert!(head.contains('▰'), "{w}: the share of the plan: {head:?}");
        // The entries sit under their own head, in the plan's order.
        assert!(at("target") == dev + 1, "{w}");
        assert!(at("node_modules") == wt + 1, "{w}");
        assert!(at("app/ios/Pods") == wt + 2, "{w}");
    }
}

#[test]
fn an_entry_under_its_head_says_the_path_inside_the_project_and_not_the_absolute_one() {
    for (w, h) in SIZES {
        let lines = frame(&plan(), &Review::new(), w, h);
        for l in lines.iter().filter(|l| l.contains("npm install")) {
            assert!(!l.contains("/w/"), "{w}: {l:?}");
            assert!(!l.contains("issue-942"), "{w}: the head says it: {l:?}");
        }
    }
}

#[test]
fn the_foot_says_how_many_entries_in_how_many_projects_and_how_much() {
    for (w, h) in SIZES {
        let lines = frame(&plan(), &Review::new(), w, h);
        let total = &lines[lines.len() - 2];
        assert!(
            total.contains("3 entries · 2 projects · 904.00 MB"),
            "{w}: {total:?}"
        );
        assert_eq!(
            lines.last().unwrap(),
            " Each line names the command that rebuilds it. Esc to change the plan.",
            "{w}: the sentence under the list is the plan's own"
        );
    }
}

#[test]
fn the_head_says_the_phrase_the_confirm_screen_repeats() {
    let lines = frame(&plan(), &Review::new(), 100, 34);
    assert!(
        lines[0].starts_with(" The plan  (3 items, 904.00 MB)"),
        "{:?}",
        lines[0]
    );
}

#[test]
fn the_header_row_has_the_candidates_columns_and_no_mark_column() {
    for (w, h) in [(100, 34), (160, 40)] {
        let lines = frame(&plan(), &Review::new(), w, h);
        let header = &lines[1];
        for name in ["size", "kind", "project / path", "comes back as"] {
            assert!(header.contains(name), "{w}: {name} in {header:?}");
        }
        assert!(
            !lines.iter().any(|l| l.contains("[ ]") || l.contains("[x]")),
            "{w}"
        );
    }
}

#[test]
fn drawing_the_plan_does_not_change_what_it_holds() {
    let plan = plan();
    let before: Vec<_> = plan
        .items()
        .iter()
        .map(|c| (c.path.clone(), c.bytes, c.safety.clone()))
        .collect();
    let total = plan.total_bytes();
    let mut review = Review::new();
    for motion in [Motion::Down, Motion::PageDown, Motion::Bottom, Motion::Top] {
        review.scroll(motion, &plan, &locator(), 3);
        frame(&plan, &review, 100, 34);
    }
    let after: Vec<_> = plan
        .items()
        .iter()
        .map(|c| (c.path.clone(), c.bytes, c.safety.clone()))
        .collect();
    assert_eq!(before, after, "the plan keeps its order and its entries");
    assert_eq!(plan.total_bytes(), total);
}

#[test]
fn an_entry_whose_head_has_scrolled_away_still_says_its_project() {
    let plan = plan();
    let mut review = Review::new();
    // Lines: head, target, head, node_modules, Pods. A window of two lines
    // from the fourth starts on an entry whose head is above it.
    review.scroll(Motion::Bottom, &plan, &locator(), 2);
    // Six rows: the head, the header row, two lines of list, the total and
    // the sentence.
    let lines = frame(&plan, &review, 100, 6);
    let row = lines
        .iter()
        .find(|l| l.contains("app/ios/Pods"))
        .expect("a row");
    assert!(row.contains("issue-942"), "{row:?}");
}

#[test]
fn an_entry_in_no_project_is_named_by_its_path_under_a_head_that_says_so() {
    let plan = plan_of(vec![entry(
        "/elsewhere/deep/node_modules",
        MB,
        "npm install",
    )]);
    let lines = frame(&plan, &Review::new(), 100, 34);
    assert!(
        lines.iter().any(|l| l.contains("Outside any project")),
        "{lines:#?}"
    );
    assert!(
        lines
            .iter()
            .any(|l| l.contains("/elsewhere/deep/node_modules")),
        "{lines:#?}"
    );
}

#[test]
fn a_plan_of_one_project_says_one_project() {
    let plan = plan_of(vec![entry("/w/dev-cleaner/target", MB, "cargo build")]);
    let lines = frame(&plan, &Review::new(), 100, 34);
    assert!(
        lines[lines.len() - 2].contains("1 entry · 1 project · 1.00 MB"),
        "{lines:#?}"
    );
}
