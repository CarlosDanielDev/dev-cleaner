//! The projects table: every project, measured six ways, sortable by each.

use dev_cleaner::classify::Activity;
use dev_cleaner::tui::{Column, ProjectSummary, Projects};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use std::path::PathBuf;

const MB: u64 = 1024 * 1024;

fn row(
    name: &str,
    apparent: u64,
    unique: u64,
    inodes: u64,
    activity: Activity,
    reclaimable: u64,
) -> ProjectSummary {
    ProjectSummary {
        path: PathBuf::from("/p").join(name),
        bytes_apparent: apparent,
        bytes_unique: unique,
        inodes,
        activity,
        reclaimable,
    }
}

/// Three projects that disagree about which is "biggest", so a sort that
/// silently ignores its column cannot pass by accident.
fn table() -> Projects {
    Projects::new(vec![
        row("carol", 300 * MB, 300 * MB, 10, Activity::Dead, 50 * MB),
        row("alice", 100 * MB, 90 * MB, 900, Activity::Active, 200 * MB),
        row("bob", 200 * MB, 200 * MB, 5000, Activity::Dormant, 10 * MB),
    ])
}

fn names(t: &Projects) -> Vec<String> {
    t.rows().iter().map(|r| r.name().to_string()).collect()
}

fn text(t: &Projects, w: u16, h: u16) -> String {
    let area = Rect::new(0, 0, w, h);
    let mut buf = Buffer::empty(area);
    t.render(area, &mut buf);
    (0..area.height)
        .map(|y| {
            (0..area.width)
                .map(|x| buf[(x, y)].symbol())
                .collect::<String>()
                .trim_end()
                .to_string()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn every_column_can_order_the_table() {
    let expected = [
        (Column::Name, ["alice", "bob", "carol"]),
        (Column::Apparent, ["carol", "bob", "alice"]),
        (Column::Unique, ["carol", "bob", "alice"]),
        (Column::Inodes, ["bob", "alice", "carol"]),
        (Column::Reclaimable, ["alice", "carol", "bob"]),
        (Column::Activity, ["alice", "bob", "carol"]),
    ];

    for (column, order) in expected {
        let mut t = table();
        // Step off via some other column first, so the one under test is being
        // asked for fresh rather than toggled.
        let elsewhere = if column == Column::Name {
            Column::Inodes
        } else {
            Column::Name
        };
        t.sort_by(elsewhere);
        t.sort_by(column);
        assert_eq!(names(&t), order, "{column:?} ordered the table wrongly");
    }
}

#[test]
fn asking_for_the_same_column_again_reverses_it() {
    let mut t = table();
    t.sort_by(Column::Name);
    t.sort_by(Column::Unique);
    assert_eq!(names(&t), ["carol", "bob", "alice"]);

    t.sort_by(Column::Unique);
    assert_eq!(
        names(&t),
        ["alice", "bob", "carol"],
        "a second press reverses"
    );

    t.sort_by(Column::Name);
    t.sort_by(Column::Unique);
    assert_eq!(
        names(&t),
        ["carol", "bob", "alice"],
        "coming back to a column starts from its own default again"
    );
}

#[test]
fn activity_is_told_apart_by_symbol_and_not_by_colour_alone() {
    let symbols: std::collections::BTreeSet<char> =
        [Activity::Active, Activity::Dormant, Activity::Dead]
            .iter()
            .map(|a| a.symbol())
            .collect();

    assert_eq!(
        symbols.len(),
        3,
        "each class needs its own mark: {symbols:?}"
    );
}

#[test]
fn apparent_size_is_shown_where_it_differs_from_unique() {
    // They differ when a project hardlinks into a shared store. That gap is the
    // difference between what the directory looks like and what deleting it
    // would return, so it has to be visible.
    let mut t = Projects::new(vec![row(
        "linked",
        100 * MB,
        40 * MB,
        10,
        Activity::Active,
        0,
    )]);
    t.sort_by(Column::Name);
    let out = text(&t, 100, 10);

    assert!(
        out.contains("40.00 MB"),
        "the unique size is missing:\n{out}"
    );
    assert!(
        out.contains("100.00 MB"),
        "the apparent size is missing:\n{out}"
    );
}

#[test]
fn apparent_size_is_left_out_when_it_says_nothing_new() {
    // Most projects hardlink nothing, and a column repeating the same figure on
    // every row is noise that hides the rows where the two really differ.
    let t = Projects::new(vec![row(
        "plain",
        100 * MB,
        100 * MB,
        10,
        Activity::Active,
        0,
    )]);
    let out = text(&t, 100, 10);

    assert_eq!(
        out.matches("100.00 MB").count(),
        1,
        "the same size should not be printed twice on one row:\n{out}"
    );
}

#[test]
fn only_a_screenful_is_ever_drawn() {
    // The table has to stay responsive with a few hundred projects. Drawing is
    // bounded by the window rather than by the number of rows, which is the
    // property that makes that true rather than a hope.
    let many: Vec<ProjectSummary> = (0..5_000)
        .map(|i| row(&format!("p{i:04}"), MB, MB, 1, Activity::Active, 0))
        .collect();
    let t = Projects::new(many);

    assert_eq!(
        t.visible(20).len(),
        20,
        "more than a screenful was prepared"
    );
    assert_eq!(t.visible(0).len(), 0);
}

#[test]
fn a_new_table_opens_on_the_biggest_unique_size() {
    // The first question is what is worst, so that is the order it opens in.
    assert_eq!(names(&table()), ["carol", "bob", "alice"]);
}

#[test]
fn the_cursor_stays_inside_the_table_and_inside_the_window() {
    let mut t = table();
    assert_eq!(t.selected().expect("a row").name(), "carol");

    t.up();
    assert_eq!(
        t.selected().expect("a row").name(),
        "carol",
        "the top does not wrap"
    );

    for _ in 0..10 {
        t.down();
    }
    let last = t.selected().expect("a row").name().to_string();
    assert_eq!(
        last,
        names(&t).last().cloned().expect("a last row"),
        "the bottom does not wrap past the last row"
    );

    let window = t.visible(2);
    assert!(
        window.iter().any(|r| r.name() == last),
        "the selected row scrolled out of view"
    );
}

#[test]
fn an_empty_table_has_nothing_selected_and_still_draws() {
    let t = Projects::new(vec![]);

    assert!(t.selected().is_none());
    assert!(t.visible(10).is_empty());
    assert!(
        !text(&t, 60, 6).is_empty(),
        "the header should still be drawn"
    );
}

fn row_at(path: &str) -> ProjectSummary {
    ProjectSummary {
        path: PathBuf::from(path),
        bytes_apparent: MB,
        bytes_unique: MB,
        inodes: 1,
        activity: Activity::Active,
        reclaimable: 0,
    }
}

#[test]
fn projects_sharing_a_name_are_told_apart_by_their_parent() {
    // Found on the real corpus: `akasha-bot/web` and `.worktrees/akasha-saude/web`
    // are different projects with the same directory name, and two rows reading
    // "web" with near-identical figures cannot be acted on.
    let t = Projects::new(vec![
        row_at("/p/akasha-bot/web"),
        row_at("/p/.worktrees/akasha-saude/web"),
        row_at("/p/solo"),
    ]);
    let out = text(&t, 110, 8);

    assert!(
        out.contains("akasha-bot/web"),
        "the first is unqualified:\n{out}"
    );
    assert!(
        out.contains("akasha-saude/web"),
        "the second is unqualified:\n{out}"
    );
    assert!(
        out.contains("solo") && !out.contains("p/solo"),
        "a name nothing collides with should not be padded with its parent:\n{out}"
    );
}
