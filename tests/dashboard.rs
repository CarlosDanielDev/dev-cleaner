//! The dashboard: what the disk looks like, and what changed since last time.

use dev_cleaner::bytes::human;
use dev_cleaner::store::{Change, TrendRow};
use dev_cleaner::tui::{
    Action, Consumer, Dashboard, Now, Screen, Trend, bindings_for, palette::Theme,
};
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
    dash.render(&Theme::ansi(), area, &mut buf);
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
        history: Vec::new(),
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

/// How many cells of the gauge a row starts with: the bar is the row that
/// begins with cells, and the legend and the sparkline do not.
fn gauge_cells(line: &str) -> usize {
    line.trim_start()
        .chars()
        .take_while(|c| ['▰', '▮', '▱'].contains(c))
        .count()
}

#[test]
fn the_gauge_separates_reclaimable_from_the_rest_of_what_is_used() {
    // Reclaimable space is part of what is *used*, not part of what is free.
    // A two-part bar would put it on the wrong side and overstate the disk.
    // The bar itself, not the legend beneath it: the legend names all three
    // symbols whatever the bar does, so matching on "contains a block" would
    // pass against a two-part bar. The bar is the line made only of fill.
    let fill = ['▰', '▮', '▱'];
    let lines = rendered(&dashboard());
    let gauge = lines
        .iter()
        .find(|l| gauge_cells(l) >= 10)
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
        .find(|l| gauge_cells(l) >= 10)
        .expect("no gauge bar");

    assert!(
        bar.contains('▮'),
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

    dash.render(&Theme::ansi(), area, &mut buf);
}

const RAMP: [char; 8] = ['▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];

/// The row directly under the gauge's legend, if anything is drawn there.
fn sparkline_row(dash: &Dashboard, width: u16) -> Option<String> {
    let area = Rect::new(0, 0, width, 30);
    let mut buf = Buffer::empty(area);
    dash.render(&Theme::ansi(), area, &mut buf);
    let rows: Vec<String> = (0..area.height)
        .map(|y| {
            (0..area.width)
                .map(|x| buf[(x, y)].symbol())
                .collect::<String>()
                .trim_end()
                .to_string()
        })
        .collect();
    let legend = rows.iter().position(|r| r.contains("in use"))?;
    Some(rows[legend + 1].clone()).filter(|row| !row.is_empty())
}

fn with_history(history: Vec<Option<u64>>) -> Dashboard {
    Dashboard {
        history,
        ..dashboard()
    }
}

#[test]
fn the_sparkline_scales_to_the_highest_value_in_the_window() {
    let row = sparkline_row(&with_history(vec![Some(1), Some(8), Some(4)]), 90)
        .expect("three scans draw a line");
    let glyphs: Vec<char> = row.chars().filter(|c| RAMP.contains(c)).take(3).collect();

    assert_eq!(glyphs, vec![RAMP[0], RAMP[7], RAMP[3]]);
}

#[test]
fn a_scan_with_no_value_is_a_gap_and_a_lone_value_draws_nothing() {
    let row = sparkline_row(&with_history(vec![Some(1), None, Some(8)]), 90)
        .expect("two values draw a line");
    assert!(row.trim_start().starts_with("▁·█"), "{row:?}");

    assert_eq!(
        sparkline_row(&with_history(vec![None, Some(5), None]), 90),
        None,
        "one value is not a trend"
    );
    assert_eq!(sparkline_row(&with_history(vec![]), 90), None);
}

#[test]
fn the_sparkline_keeps_the_newest_scans_that_fit() {
    // 100 scans climbing 1..=100: the newest `width - 4` are the highest.
    let history: Vec<Option<u64>> = (1..=100).map(Some).collect();
    let row = sparkline_row(&with_history(history), 70).expect("a line");
    let row = row.trim_start();

    assert!(
        row.chars().count() <= 66,
        "{} cells in a 70-wide body: {row:?}",
        row.chars().count()
    );
    let glyphs: String = row.chars().filter(|c| RAMP.contains(c)).collect();
    assert_eq!(glyphs.chars().count(), 66);
    assert!(glyphs.ends_with('█'), "newest scan is the right-hand edge");
}

#[test]
fn the_figures_are_the_stored_low_high_and_newest() {
    let row = sparkline_row(
        &with_history(vec![Some(2 * GB), Some(8 * GB), Some(5 * GB)]),
        120,
    )
    .expect("a line");

    assert!(row.contains("3 scans"), "{row:?}");
    assert!(row.contains(&format!("low {}", human(2 * GB))), "{row:?}");
    assert!(row.contains(&format!("high {}", human(8 * GB))), "{row:?}");
    assert!(row.contains(&format!("now {}", human(5 * GB))), "{row:?}");
}

/// The dashboard in `theme`, `width` columns wide.
fn drawn_at(dash: &Dashboard, theme: &Theme, width: u16) -> Buffer {
    let area = Rect::new(0, 0, width, 30);
    let mut buf = Buffer::empty(area);
    buf.set_style(area, theme.ground);
    dash.render(theme, area, &mut buf);
    buf
}

fn row_text(buf: &Buffer, y: u16) -> String {
    (0..buf.area.width).map(|x| buf[(x, y)].symbol()).collect()
}

#[test]
fn the_legend_has_no_stray_glyph_and_every_swatch_is_the_colour_of_its_cells() {
    for theme in [Theme::neon(), Theme::ansi()] {
        let buf = drawn_at(&dashboard(), &theme, 100);
        let legend_y = (0..buf.area.height)
            .find(|&y| row_text(&buf, y).contains("reclaimable"))
            .expect("a legend");
        let gauge_y = legend_y - 1;
        let legend = row_text(&buf, legend_y);

        // Nothing in the legend is a symbol but the three swatches.
        let strays: Vec<char> = legend
            .chars()
            .filter(|c| !c.is_alphanumeric() && !" .,".contains(*c) && !"▮▰▱".contains(*c))
            .collect();
        assert!(strays.is_empty(), "stray glyphs {strays:?} in {legend:?}");

        // Each swatch is drawn in the colour of the cells that glyph draws.
        for glyph in ['▮', '▰', '▱'] {
            let swatch = (0..buf.area.width)
                .find(|&x| buf[(x, legend_y)].symbol() == glyph.to_string())
                .unwrap_or_else(|| panic!("{glyph} is not in the legend {legend:?}"));
            let cell = (0..buf.area.width)
                .find(|&x| buf[(x, gauge_y)].symbol() == glyph.to_string())
                .unwrap_or_else(|| panic!("{glyph} is not in the gauge"));
            assert_eq!(
                buf[(swatch, legend_y)].fg,
                buf[(cell, gauge_y)].fg,
                "{glyph}: the legend names a colour the cells are not drawn in"
            );
        }
    }
}

#[test]
fn the_gauge_is_a_bar_at_eighty_and_two_hundred_columns_and_only_shorter_below() {
    let cells = |width: u16| {
        let buf = drawn_at(&dashboard(), &Theme::ansi(), width);
        let y = (0..buf.area.height)
            .find(|&y| row_text(&buf, y).contains(" used"))
            .expect("the gauge row");
        let row = row_text(&buf, y);
        let n = row.chars().filter(|c| "▮▰▱".contains(*c)).count();
        // Whole cells and then the figure, both inside the width.
        assert!(row.trim_end().ends_with("% used"), "{width}: {row:?}");
        assert!(
            row.trim_end().chars().count() <= width as usize,
            "{width}: {row:?}"
        );
        n
    };
    let sizes: Vec<usize> = [200, 120, 80, 60, 40, 24].map(cells).to_vec();
    assert!(sizes[2] >= 40, "80 columns: {sizes:?}");
    assert!(
        sizes[0] >= sizes[2],
        "200 columns draws at least what 80 does: {sizes:?}"
    );
    assert!(
        sizes.windows(2).all(|w| w[0] >= w[1]),
        "narrower never means more: {sizes:?}"
    );
}
