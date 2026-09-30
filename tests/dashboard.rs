//! The dashboard: what the disk looks like, and what changed since last time.

use dev_cleaner::bytes::human;
use dev_cleaner::store::{Change, TrendRow};
use dev_cleaner::tui::{Action, Consumer, Dashboard, Now, Screen, Trend, bindings_for};
use dev_cleaner::volume::Volume;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use std::path::PathBuf;

const MB: u64 = 1024 * 1024;
const GB: u64 = 1024 * MB;

/// Draw into an in-memory buffer and read the text back.
///
/// No terminal, no raw mode: the screen is a function from data to cells, so a
/// test can look at the cells.
fn rendered(dash: &Dashboard) -> Vec<String> {
    let area = Rect::new(0, 0, 90, 30);
    let mut buf = Buffer::empty(area);
    dash.render(area, &mut buf);
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

fn text(dash: &Dashboard) -> String {
    rendered(dash).join("\n")
}

fn consumer(label: &str, bytes: u64, inodes: u64) -> Consumer {
    Consumer {
        label: label.to_string(),
        bytes,
        inodes,
    }
}

/// What one step forward would offer, with the reasons deliberately out of
/// order so the screen has to rank them itself.
fn now() -> Now {
    Now {
        offerable: 88,
        offerable_bytes: 4980 * MB,
        blocked: vec![
            (
                "This path lies outside every configured root.".to_string(),
                24,
            ),
            (
                "Untracked source files here exist nowhere else.".to_string(),
                63,
            ),
            (
                "Uncommitted changes are present in this repository.".to_string(),
                41,
            ),
        ],
        dead: 3,
        dead_reclaimable: 2150 * MB,
    }
}

fn dashboard() -> Dashboard {
    Dashboard {
        volume: Some(Volume {
            total: 460 * GB,
            free: 68 * GB,
        }),
        reclaimable: 5 * GB,
        trend: Trend::FirstScan,
        consumers: vec![
            consumer("astral-system", 4 * GB, 40_000),
            consumer("claud-framework", 2 * GB, 120_000),
        ],
        now: now(),
    }
}

/// The key the table binds to the way forward on `screen`.
fn forward_key(screen: Screen) -> String {
    bindings_for(screen)
        .iter()
        .find(|b| b.action == Action::Forward)
        .map(|b| b.key.to_string())
        .expect("the screen has a way forward")
}

#[test]
fn the_gauge_separates_reclaimable_from_the_rest_of_what_is_used() {
    // Reclaimable space is part of what is *used*, not part of what is free.
    // A two-part bar would put it on the wrong side and overstate the disk.
    // The bar itself, not the legend beneath it: the legend names all three
    // symbols whatever the bar does, so matching on "contains a block" would
    // pass against a two-part bar. The bar is the line made only of fill.
    let fill = ['█', '▒', '·'];
    let lines = rendered(&dashboard());
    let gauge = lines
        .iter()
        .find(|l| {
            let bar: Vec<char> = l.chars().filter(|c| !c.is_whitespace()).collect();
            bar.len() >= 10 && bar.iter().all(|c| fill.contains(c))
        })
        .expect("no gauge bar was drawn");

    let symbols: std::collections::BTreeSet<char> =
        gauge.chars().filter(|c| fill.contains(c)).collect();
    assert!(
        symbols.len() >= 3,
        "the bar must show reclaimable, in use and free as three segments, told \
         apart by symbol and not by colour alone; got {symbols:?} in {gauge:?}"
    );
}

#[test]
fn the_gauge_reports_the_figures_it_is_drawn_from() {
    let out = text(&dashboard());

    for expected in ["68.00 GB", "5.00 GB", "460.00 GB"] {
        assert!(out.contains(expected), "{expected} is missing from:\n{out}");
    }
}

#[test]
fn a_first_run_says_so_rather_than_showing_an_empty_trend() {
    // An empty history is the ordinary state of a first run, not a failure, and
    // it is handled in one place rather than as a hole in every field.
    let out = text(&dashboard()).to_lowercase();

    assert!(
        out.contains("first scan"),
        "a first run should say why there is no comparison:\n{out}"
    );
}

#[test]
fn a_later_run_shows_what_moved() {
    let dash = Dashboard {
        trend: Trend::Since(vec![
            TrendRow {
                path: PathBuf::from("/p/astral-system/node_modules"),
                bytes: 340 * 1024 * 1024,
                change: Change::Grew {
                    by: 340 * 1024 * 1024,
                },
            },
            TrendRow {
                path: PathBuf::from("/p/old/target"),
                bytes: 0,
                change: Change::Removed,
            },
        ]),
        ..dashboard()
    };
    let out = text(&dash);

    assert!(
        out.contains("astral-system"),
        "a changed path is missing:\n{out}"
    );
    assert!(
        out.contains("+340.00 MB"),
        "the size of the change is missing:\n{out}"
    );
    assert!(
        out.contains("removed"),
        "a path that disappeared must be named as removed, not shown as 0 B:\n{out}"
    );
}

#[test]
fn consumers_are_ranked_by_bytes_and_by_inodes_separately() {
    // The two orders answer different questions. Bytes say what fills the disk;
    // inodes say what makes every backup and every Spotlight pass slow, and the
    // biggest directory is often not the one with the most files in it.
    let dash = dashboard();

    assert_eq!(
        dash.top_by_bytes(2)
            .iter()
            .map(|c| c.label.as_str())
            .collect::<Vec<_>>(),
        ["astral-system", "claud-framework"]
    );
    assert_eq!(
        dash.top_by_inodes(2)
            .iter()
            .map(|c| c.label.as_str())
            .collect::<Vec<_>>(),
        ["claud-framework", "astral-system"],
        "the largest directory is not the one with the most files in it"
    );

    let out = text(&dash);
    assert!(
        out.to_lowercase().contains("inode"),
        "the inode ranking is not shown:\n{out}"
    );
}

#[test]
fn an_unmeasurable_volume_does_not_take_the_rest_of_the_screen_with_it() {
    // statvfs can fail on an unmounted or vanished path. The scan's other
    // answers are still worth showing.
    let dash = Dashboard {
        volume: None,
        ..dashboard()
    };
    let out = text(&dash);

    assert!(
        out.contains("astral-system"),
        "the rest of the dashboard should still render:\n{out}"
    );
    assert!(
        out.to_lowercase().contains("unavailable") || out.to_lowercase().contains("not measured"),
        "the missing figure should say so rather than showing a zero:\n{out}"
    );
}

#[test]
fn a_volume_reports_what_is_used_as_the_difference() {
    let v = Volume {
        total: 100,
        free: 30,
    };
    assert_eq!(v.used(), 70);
}

#[test]
fn free_space_is_read_from_the_volume_holding_the_path() {
    let home = PathBuf::from(std::env::var("HOME").expect("HOME"));
    let v = Volume::of(&home).expect("home is on a real volume");

    assert!(v.total > 0 && v.free > 0);
    assert!(v.free <= v.total, "free cannot exceed capacity");
}

#[test]
fn a_small_reclaimable_share_is_still_visible_on_a_large_disk() {
    // 5 GB of 460 GB is nine tenths of a cell at this width. Truncating drew no
    // reclaimable segment at all, which is the ordinary case for this tool: the
    // one quantity the screen exists to show was the one that rounded away.
    let dash = Dashboard {
        volume: Some(Volume {
            total: 4000 * GB,
            free: 100 * GB,
        }),
        reclaimable: GB,
        ..dashboard()
    };
    let lines = rendered(&dash);
    let bar = lines
        .iter()
        .find(|l| {
            let chars: Vec<char> = l.chars().filter(|c| !c.is_whitespace()).collect();
            chars.len() >= 10 && chars.iter().all(|c| ['█', '▒', '·'].contains(c))
        })
        .expect("no gauge bar");

    assert!(
        bar.contains('█'),
        "a reclaimable share under one cell must still be drawn, not rounded away: {bar:?}"
    );
}

#[test]
fn the_trend_shows_what_moved_and_not_what_stayed_put() {
    // Found by rendering the real history: most rows in a scan are unchanged,
    // and listing them buries the handful that are not.
    let dash = Dashboard {
        trend: Trend::Since(vec![
            TrendRow {
                path: PathBuf::from("/p/quiet/node_modules"),
                bytes: 5 * GB,
                change: Change::Unchanged,
            },
            TrendRow {
                path: PathBuf::from("/p/busy/target"),
                bytes: GB,
                change: Change::Grew { by: GB },
            },
        ]),
        ..dashboard()
    };
    let out = text(&dash);

    assert!(out.contains("busy/target"), "the change is missing:\n{out}");
    assert!(
        !out.contains("quiet/node_modules"),
        "an unchanged path is noise on a screen about what moved:\n{out}"
    );
}

#[test]
fn a_scan_where_nothing_moved_says_so() {
    let dash = Dashboard {
        trend: Trend::Since(vec![TrendRow {
            path: PathBuf::from("/p/quiet/node_modules"),
            bytes: 5 * GB,
            change: Change::Unchanged,
        }]),
        ..dashboard()
    };

    assert!(
        text(&dash).to_lowercase().contains("nothing changed"),
        "an all-quiet comparison must say so rather than showing a blank section"
    );
}

#[test]
fn the_now_section_counts_what_can_be_rebuilt_and_names_the_key_that_gets_there() {
    // The count and the bytes are the candidates screen's own, and the key is
    // read from the table: a line that said `Enter` on its own authority could
    // name a key that does nothing.
    let out = text(&dashboard());
    let dash = dashboard();

    assert!(
        out.contains("88 directories can be rebuilt"),
        "the offerable count is missing:\n{out}"
    );
    assert!(
        out.contains(&human(dash.now.offerable_bytes)),
        "the offerable bytes are missing:\n{out}"
    );
    let way = format!(
        "{} twice → {}",
        forward_key(Screen::Dashboard),
        Screen::Candidates.name()
    );
    assert!(
        out.contains(&way),
        "the line does not say how to reach the candidates ({way:?}):\n{out}"
    );
}

#[test]
fn blocked_entries_are_grouped_by_reason_largest_first_and_the_rest_are_counted() {
    let mut now = now();
    now.blocked
        .push(("Stashed work is present and would be lost.".to_string(), 7));
    let dash = Dashboard { now, ..dashboard() };
    let lines = rendered(&dash);
    let out = lines.join("\n");

    assert!(
        out.contains("135 held back by a guard"),
        "the blocked total is missing:\n{out}"
    );
    let row = |needle: &str| {
        lines
            .iter()
            .position(|l| l.contains(needle))
            .unwrap_or_else(|| panic!("{needle:?} is missing from:\n{out}"))
    };
    let untracked = row("63  Untracked source files here exist nowhere else.");
    let dirty = row("41  Uncommitted changes are present in this repository.");
    let outside = row("24  This path lies outside every configured root.");
    assert!(
        untracked < dirty && dirty < outside,
        "reasons are ranked by how many entries they held back:\n{out}"
    );
    assert!(
        !out.contains("Stashed work"),
        "the fourth reason is folded into the remainder, not listed:\n{out}"
    );
    assert!(
        out.contains("and 7 more"),
        "what the three reasons do not cover is counted:\n{out}"
    );
}

#[test]
fn dead_projects_are_counted_with_the_build_output_inside_them() {
    // The same per-project measurement the table shows, and nothing more: the
    // tool does not offer a whole project, so the line must not read as one.
    let dash = dashboard();
    let out = text(&dash);

    assert!(
        out.contains("3 dead projects"),
        "the dead count is missing:\n{out}"
    );
    assert!(
        out.contains(&format!(
            "{} of build output inside them",
            human(dash.now.dead_reclaimable)
        )),
        "the build output inside the dead projects is missing:\n{out}"
    );
}

#[test]
fn a_scan_with_nothing_offerable_says_so_rather_than_counting_to_zero() {
    let dash = Dashboard {
        now: Now::default(),
        ..dashboard()
    };
    let out = text(&dash);

    assert!(
        out.contains("Nothing can be rebuilt on these roots"),
        "an empty offer is a sentence, not a zero:\n{out}"
    );
    for absent in ["0 directories", "held back", "dead project"] {
        assert!(
            !out.contains(absent),
            "{absent:?} is drawn for a scan that has none:\n{out}"
        );
    }
}

#[test]
fn one_of_anything_is_not_plural() {
    let dash = Dashboard {
        now: Now {
            offerable: 1,
            offerable_bytes: GB,
            blocked: Vec::new(),
            dead: 1,
            dead_reclaimable: MB,
        },
        ..dashboard()
    };
    let out = text(&dash);

    assert!(out.contains("1 directory can be rebuilt"), "{out}");
    assert!(out.contains("1 dead project "), "{out}");
    assert!(out.contains("inside it"), "{out}");
}

#[test]
fn a_short_terminal_cuts_the_screen_off_rather_than_crashing_it() {
    // The rows the Now section adds push the trend down. On a 24-row terminal
    // that is past the bottom of the body, and a cell outside the buffer is a
    // panic, not a blank.
    let dash = Dashboard {
        trend: Trend::Since(vec![TrendRow {
            path: PathBuf::from("/p/busy/target"),
            bytes: GB,
            change: Change::Grew { by: GB },
        }]),
        ..dashboard()
    };
    let area = Rect::new(0, 0, 90, 16);
    let mut buf = Buffer::empty(area);

    dash.render(area, &mut buf);
}
