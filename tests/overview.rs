//! The dashboard as an overview: the disk in three parts, what was analysed,
//! where the rebuildable bytes are, and ranked insights, each with an icon.

mod common;

use common::contrast::assert_readable;
use dev_cleaner::bytes::human;
use dev_cleaner::classify::Ecosystem;
use dev_cleaner::store::{Change, TrendRow};
use dev_cleaner::tui::bar;
use dev_cleaner::tui::icons::{Icon, IconSet};
use dev_cleaner::tui::palette::Theme;
use dev_cleaner::tui::{
    Action, Aim, Analysed, Consumer, Dashboard, Group, Now, Screen, Trend, bindings_for,
};
use dev_cleaner::volume::Volume;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Color;
use std::path::PathBuf;
use std::time::Duration;

const MB: u64 = 1024 * 1024;
const GB: u64 = 1024 * MB;

fn group(label: &str, eco: Ecosystem, regen: &str, bytes: u64, dirs: usize) -> Group {
    Group {
        label: label.to_string(),
        ecosystem: eco,
        regen: regen.to_string(),
        bytes,
        dirs,
        offerable_bytes: bytes - bytes / 10,
        offerable_dirs: dirs - 1,
    }
}

/// Eight kinds, deliberately out of order, so the screen has to rank them and
/// to cap them.
fn groups() -> Vec<Group> {
    vec![
        group(".venv", Ecosystem::Python, "pip install", 2 * GB, 4),
        group("target", Ecosystem::Rust, "cargo build", 6 * GB, 9),
        group(
            "__pycache__",
            Ecosystem::Python,
            "on next import",
            10 * MB,
            30,
        ),
        group("node_modules", Ecosystem::Node, "npm install", 12 * GB, 14),
        group("Pods", Ecosystem::Swift, "pod install", GB, 2),
        group(
            "DerivedData",
            Ecosystem::Swift,
            "rebuild in Xcode",
            3 * GB / 2,
            3,
        ),
        group(".gradle", Ecosystem::Java, "gradle build", 300 * MB, 2),
        group(".next", Ecosystem::Node, "next build", 500 * MB, 3),
    ]
}

fn dash() -> Dashboard {
    let groups = groups();
    let reclaimable = groups.iter().map(|g| g.bytes).sum();
    Dashboard {
        volume: Some(Volume {
            total: 460 * GB,
            free: 94 * GB,
        }),
        reclaimable,
        trend: Trend::Since(vec![TrendRow {
            path: PathBuf::from("/p/app/target"),
            bytes: GB,
            change: Change::Grew { by: GB },
        }]),
        consumers: vec![
            Consumer {
                label: "app/target".into(),
                bytes: 6 * GB,
                inodes: 40_000,
            },
            Consumer {
                label: "web/node_modules".into(),
                bytes: 12 * GB,
                inodes: 120_000,
            },
        ],
        now: Now {
            offerable: 30,
            offerable_bytes: 20 * GB,
            blocked: vec![
                (
                    "Uncommitted changes are present in this repository.".into(),
                    5,
                ),
                ("Untracked source files here exist nowhere else.".into(), 2),
            ],
            dead: 3,
            dead_reclaimable: 2 * GB,
        },
        history: vec![Some(GB), Some(5 * GB), Some(20 * GB), Some(reclaimable)],
        groups,
        analysed: Analysed {
            projects: 267,
            with_rebuild: 41,
            entries: 136_432,
            measured: 67,
            elapsed: Duration::from_millis(1040),
            roots: vec![PathBuf::from("/Users/me/projects")],
        },
        aim: Aim {
            win: Some(PathBuf::from("/p/web")),
            quiet: Some(PathBuf::from("/p/old")),
            held: Some(PathBuf::from("/p/held")),
        },
    }
}

fn draw(dash: &Dashboard, theme: &Theme, w: u16, h: u16) -> Buffer {
    let area = Rect::new(0, 0, w, h);
    let mut buf = Buffer::empty(area);
    buf.set_style(area, theme.ground);
    dash.render(theme, area, &mut buf);
    buf
}

fn rows(buf: &Buffer) -> Vec<String> {
    (0..buf.area.height)
        .map(|y| {
            (0..buf.area.width)
                .map(|x| buf[(x, y)].symbol())
                .collect::<String>()
                .trim_end()
                .to_string()
        })
        .collect()
}

fn lines(dash: &Dashboard, w: u16, h: u16) -> Vec<String> {
    rows(&draw(dash, &Theme::ansi(), w, h))
}

fn text(dash: &Dashboard, w: u16, h: u16) -> String {
    lines(dash, w, h).join("\n")
}

fn forward_key() -> String {
    bindings_for(Screen::Dashboard)
        .iter()
        .find(|b| b.action == Action::Forward)
        .map(|b| b.key.to_string())
        .expect("the dashboard has a way forward")
}

// ---------------------------------------------------------------- layout

#[test]
fn every_block_is_drawn_at_the_sizes_that_have_room_for_all_four() {
    for (w, h) in [(100, 30), (110, 30), (200, 50)] {
        let out = text(&dash(), w, h);
        for heading in ["Disk", "Analysed", "Where it is", "Insights"] {
            assert!(
                out.contains(heading),
                "{heading} is missing at {w}x{h}:\n{out}"
            );
        }
    }
}

#[test]
fn eighty_by_twenty_four_keeps_the_disk_and_the_best_insight_and_drops_whole_blocks() {
    let out = text(&dash(), 80, 24);
    assert!(out.contains("Disk"), "{out}");
    assert!(out.contains("% used"), "{out}");
    assert!(
        out.contains("Biggest win"),
        "the best insight is dropped:\n{out}"
    );
    // Whatever is missing is missing whole: a heading is never left without
    // its first row, and nothing is cut off at the bottom edge.
    if out.contains("Where it is") {
        assert!(out.contains("node_modules"), "{out}");
    }
}

#[test]
fn wide_puts_disk_and_analysed_side_by_side_and_narrow_stacks_them() {
    let wide = lines(&dash(), 110, 30);
    let row = wide
        .iter()
        .find(|r| r.contains("Disk"))
        .expect("a Disk heading");
    assert!(
        row.contains("Analysed"),
        "at 110 columns the two share a row: {row:?}"
    );
    assert!(row.find("Disk") < row.find("Analysed"));

    let narrow = lines(&dash(), 100, 40);
    let row = narrow
        .iter()
        .find(|r| r.contains("Disk"))
        .expect("a Disk heading");
    assert!(!row.contains("Analysed"), "below 110 they stack: {row:?}");
}

#[test]
fn blocks_do_not_overlap_in_the_wide_layout() {
    // Disk owns the left column, Analysed the right: a Disk row never carries
    // Analysed's words, and the other way round.
    let wide = lines(&dash(), 110, 30);
    let split = wide
        .iter()
        .find(|r| r.contains("Analysed"))
        .and_then(|r| r.find("Analysed").map(|b| r[..b].chars().count()))
        .expect("heading");
    for row in &wide {
        let left: String = row.chars().take(split - 1).collect();
        let right: String = row.chars().skip(split).collect();
        assert!(!left.contains("267"), "Analysed leaks left: {row:?}");
        assert!(!right.contains("% used"), "Disk leaks right: {row:?}");
    }
}

#[test]
fn no_height_cuts_a_block_in_half_and_none_panics() {
    for w in [80, 110, 200] {
        for h in 8..=50 {
            let out = text(&dash(), w, h);
            if out.contains("Disk") {
                assert!(
                    out.contains("free"),
                    "Disk cut mid-block at {w}x{h}:\n{out}"
                );
            }
            if out.contains("Where it is") {
                assert!(out.contains("node_modules"), "Where cut at {w}x{h}:\n{out}");
            }
            if out.contains("Insights") {
                assert!(
                    out.contains("Biggest win"),
                    "Insights cut at {w}x{h}:\n{out}"
                );
            }
        }
    }
}

// ------------------------------------------------------------- disk bar

#[test]
fn the_parts_always_fill_the_bar_and_a_nonzero_part_is_never_zero_cells() {
    for cells in [10, 40, 64] {
        for bytes in [
            [GB, 365 * GB, 94 * GB],
            [1, 400 * GB, 60 * GB],
            [GB, 1, 460 * GB],
            [0, 300 * GB, 100 * GB],
            [5 * GB, 0, 100 * GB],
            [GB, 100 * GB, 0],
            [GB, GB, GB],
        ] {
            let parts = bar::split(cells, bytes);
            assert_eq!(parts.iter().sum::<usize>(), cells, "{bytes:?} in {cells}");
            for (n, b) in parts.iter().zip(bytes) {
                assert_eq!(*n == 0, b == 0, "{bytes:?} in {cells} drew {parts:?}");
            }
        }
    }
    assert_eq!(bar::split(40, [0, 0, 0]), [0, 0, 0], "no disk, no cells");
}

#[test]
fn the_legend_names_exactly_the_parts_that_are_drawn() {
    let nothing = Dashboard {
        reclaimable: 0,
        groups: Vec::new(),
        ..dash()
    };
    let out = text(&nothing, 110, 30);
    assert!(
        !out.contains('▮'),
        "a part with no bytes drew cells:\n{out}"
    );
    assert!(
        !out.contains("rebuildable"),
        "the legend names a part that is not drawn:\n{out}"
    );

    let out = text(&dash(), 110, 30);
    for word in ["other", "rebuildable", "free"] {
        assert!(out.contains(word), "{word} missing from the legend:\n{out}");
    }
    assert!(out.contains('▮'));
}

#[test]
fn the_disk_says_how_much_of_what_is_used_can_be_rebuilt() {
    let d = dash();
    let out = text(&d, 110, 30);
    assert!(out.contains(&human(460 * GB)), "{out}");
    assert!(out.contains("of what is used can be rebuilt"), "{out}");
}

// ---------------------------------------------------------- where it is

#[test]
fn where_it_is_is_ranked_by_bytes_capped_at_six_and_counts_the_rest() {
    let out = lines(&dash(), 100, 40);
    let at = |needle: &str| {
        out.iter()
            .position(|r| r.contains(needle))
            .unwrap_or_else(|| panic!("{needle:?} missing:\n{}", out.join("\n")))
    };
    let order = [
        at("node_modules"),
        at("target"),
        at(".venv"),
        at("DerivedData"),
        at("Pods"),
        at(".next"),
    ];
    assert!(
        order.windows(2).all(|w| w[0] < w[1]),
        "not by bytes: {order:?}"
    );
    let joined = out.join("\n");
    assert!(
        !joined.contains(".gradle"),
        "a seventh row is drawn:\n{joined}"
    );
    assert!(!joined.contains("__pycache__"), "{joined}");
    assert!(joined.contains("+2 more"), "the cap is silent:\n{joined}");
}

#[test]
fn where_it_is_adds_up_to_the_figure_the_disk_shows() {
    let d = dash();
    let out = text(&d, 100, 40);
    // The heading carries the total, and it is the disk's own figure.
    let heading = out
        .lines()
        .find(|r| r.contains("Where it is"))
        .expect("heading");
    assert!(heading.contains(&human(d.reclaimable)), "{heading:?}");
    assert!(
        out.contains(&format!("{} rebuildable", human(d.reclaimable))),
        "the disk legend and the breakdown disagree:\n{out}"
    );
    // Six rows and the remainder are the whole.
    let mut sorted = d.groups.clone();
    sorted.sort_by_key(|g| std::cmp::Reverse(g.bytes));
    let rest: u64 = sorted.iter().skip(6).map(|g| g.bytes).sum();
    assert!(
        out.contains("+2 more") && out.contains(&human(rest)),
        "{out}"
    );
}

#[test]
fn a_row_carries_its_bytes_its_count_and_the_command_that_brings_it_back() {
    let out = text(&dash(), 100, 40);
    let row = out
        .lines()
        .find(|r| r.contains("node_modules") && r.contains("npm install"))
        .unwrap_or_else(|| panic!("no node_modules row:\n{out}"));
    assert!(row.contains(&human(12 * GB)), "{row:?}");
    assert!(row.contains("14 dirs"), "{row:?}");
}

#[test]
fn the_by_inodes_ranking_stays_reachable() {
    let out = text(&dash(), 110, 30);
    assert!(out.contains("most files"), "{out}");
    assert!(out.contains("web/node_modules"), "{out}");
    assert!(out.contains("120,000"), "{out}");
}

#[test]
fn wide_terminals_lay_the_rows_out_in_two_columns() {
    let narrow = lines(&dash(), 110, 40);
    let wide = lines(&dash(), 200, 40);
    let rows_of = |l: &Vec<String>| {
        l.iter()
            .filter(|r| {
                ["node_modules", "target", ".venv", "Pods"]
                    .iter()
                    .any(|n| r.contains(n))
            })
            .count()
    };
    assert!(rows_of(&wide) < rows_of(&narrow), "no second column at 200");
}

// -------------------------------------------------------------- insights

#[test]
fn insights_are_ranked_and_at_most_four() {
    let out = lines(&dash(), 110, 40);
    let at = |needle: &str| {
        out.iter()
            .position(|r| r.contains(needle))
            .unwrap_or_else(|| panic!("{needle:?} missing:\n{}", out.join("\n")))
    };
    let order = [
        at("Biggest win"),
        at("Gone quiet"),
        at("Held back"),
        at("Since last scan"),
    ];
    assert!(order.windows(2).all(|w| w[0] < w[1]), "{order:?}");
    let headlines = out
        .iter()
        .filter(|r| {
            ["Biggest win", "Gone quiet", "Held back", "Since last scan"]
                .iter()
                .any(|h| r.contains(h))
        })
        .count();
    assert_eq!(headlines, 4);
}

#[test]
fn an_insight_with_nothing_to_say_is_not_shown() {
    let d = Dashboard {
        now: Now {
            blocked: Vec::new(),
            dead: 0,
            dead_reclaimable: 0,
            ..dash().now
        },
        history: Vec::new(),
        trend: Trend::FirstScan,
        ..dash()
    };
    let out = text(&d, 110, 40);
    assert!(out.contains("Biggest win"), "{out}");
    for gone in ["Gone quiet", "Held back", "Since last scan"] {
        assert!(
            !out.contains(gone),
            "{gone} is drawn with nothing to say:\n{out}"
        );
    }
}

#[test]
fn when_there_is_nothing_to_rebuild_one_calm_line_says_so() {
    let d = Dashboard {
        reclaimable: 0,
        groups: Vec::new(),
        now: Now::default(),
        history: Vec::new(),
        trend: Trend::FirstScan,
        ..dash()
    };
    let out = text(&d, 110, 30);
    assert!(out.contains("Insights"), "{out}");
    assert!(
        out.contains(&format!(
            "Nothing to rebuild on these roots. {} is yours.",
            human(366 * GB)
        )),
        "{out}"
    );
}

#[test]
fn an_insight_names_where_enter_goes_only_where_it_goes() {
    let key = forward_key();
    let out = text(&dash(), 110, 40);
    // One Enter follows one insight, the first that has a project to lead to,
    // and only that one says where. The candidates are two steps from here.
    assert!(out.contains(&format!("{key} twice → candidates")), "{out}");
    let hinted: Vec<&str> = out.lines().filter(|r| r.contains('→')).collect();
    assert_eq!(hinted.len(), 2, "the header's and the lead's: {out}");
    assert!(hinted[1].contains("Biggest win"), "{hinted:?}");

    // With no win to lead to, the next insight with a project does, and the
    // projects table is one step.
    let mut d = dash();
    d.aim.win = None;
    let out = text(&d, 110, 40);
    let quiet = out.lines().find(|r| r.contains("Gone quiet")).unwrap();
    assert!(quiet.contains(&format!("{key} → projects")), "{quiet}");

    // With no project anywhere, no insight promises anything.
    d.aim = Aim::default();
    let out = text(&d, 110, 40);
    assert_eq!(out.lines().filter(|r| r.contains('→')).count(), 1, "{out}");
}

#[test]
fn the_trend_insight_says_which_way_reclaimable_moved() {
    let up = text(&dash(), 110, 40);
    assert!(up.contains("reclaimable is up"), "{up}");
    let mut d = dash();
    d.history = vec![Some(30 * GB), Some(d.reclaimable)];
    assert!(text(&d, 110, 40).contains("reclaimable is down"));
}

// ----------------------------------------------------------------- icons

fn all_icons() -> Vec<Icon> {
    Icon::ALL.to_vec()
}

#[test]
fn the_unicode_set_is_single_width_distinct_and_no_emoji() {
    let glyphs: Vec<char> = all_icons()
        .into_iter()
        .map(|i| Theme::ansi().icon(i))
        .collect();
    for g in &glyphs {
        let n = *g as u32;
        assert!(n < 0x1_0000, "{g:?} is outside the basic plane");
        assert!(
            !(0x2600..=0x27BF).contains(&n),
            "{g:?} is an emoji-range dingbat"
        );
        assert!(!g.is_whitespace() && !g.is_control());
    }
    let mut unique = glyphs.clone();
    unique.sort();
    unique.dedup();
    assert_eq!(
        unique.len(),
        glyphs.len(),
        "two concepts share a glyph: {glyphs:?}"
    );
}

#[test]
fn the_ascii_set_is_ascii_and_the_nerd_set_is_private_use() {
    for icon in all_icons() {
        let a = Theme::ansi().with_icons(IconSet::Ascii).icon(icon);
        assert!(a.is_ascii() && !a.is_whitespace(), "{icon:?} -> {a:?}");
        let n = Theme::ansi().with_icons(IconSet::Nerd).icon(icon) as u32;
        assert!((0xE000..=0xF8FF).contains(&n), "{icon:?} -> {n:#x}");
    }
}

#[test]
fn the_set_is_chosen_with_the_other_fallbacks() {
    assert_eq!(IconSet::choose(None, None), IconSet::Unicode);
    assert_eq!(IconSet::choose(Some("nerd"), Some("xterm")), IconSet::Nerd);
    assert_eq!(IconSet::choose(Some("ascii"), None), IconSet::Ascii);
    for term in ["linux", "dumb"] {
        assert_eq!(IconSet::choose(None, Some(term)), IconSet::Ascii);
        assert_eq!(
            IconSet::choose(Some("nerd"), Some(term)),
            IconSet::Ascii,
            "a console that cannot draw cells cannot draw a Nerd Font either"
        );
    }
    // NO_COLOR is not an icon decision: icons are not colour.
    assert_eq!(
        Theme::mono().icon(Icon::Disk),
        Theme::ansi().icon(Icon::Disk)
    );
    assert_eq!(
        Theme::neon().ascii().icon(Icon::Disk),
        Theme::ansi().ascii().icon(Icon::Disk)
    );
    assert!(Theme::ansi().ascii().icon(Icon::Disk).is_ascii());
}

#[test]
fn every_meaning_is_carried_by_a_word_as_well_as_an_icon() {
    let theme = Theme::ansi();
    let icons: Vec<char> = all_icons().into_iter().map(|i| theme.icon(i)).collect();
    let stripped: String = text(&dash(), 110, 40)
        .chars()
        .map(|c| if icons.contains(&c) { ' ' } else { c })
        .collect();
    for word in [
        "Disk",
        "Analysed",
        "Where it is",
        "Insights",
        "Biggest win",
        "Gone quiet",
        "Held back",
        "Since last scan",
        "projects",
        "entries walked",
        "node_modules",
    ] {
        assert!(
            stripped.contains(word),
            "{word} rests on an icon:\n{stripped}"
        );
    }
}

#[test]
fn the_rows_draw_the_icon_of_their_ecosystem() {
    let theme = Theme::ansi();
    let out = text(&dash(), 100, 40);
    let row = out.lines().find(|r| r.contains("node_modules")).unwrap();
    assert!(row.contains(theme.icon(Icon::Node)), "{row:?}");
    let row = out.lines().find(|r| r.contains("target")).unwrap();
    assert!(row.contains(theme.icon(Icon::Rust)), "{row:?}");
}

// -------------------------------------------------------------- contrast

#[test]
fn the_overview_reads_in_neon_at_every_size() {
    for (w, h) in [(80, 24), (100, 30), (110, 30), (200, 50)] {
        assert_readable(
            &draw(&dash(), &Theme::neon(), w, h),
            &format!("neon {w}x{h}"),
        );
        let nerd = Theme::neon().with_icons(IconSet::Nerd);
        assert_readable(&draw(&dash(), &nerd, w, h), &format!("neon nerd {w}x{h}"));
    }
}

#[test]
fn the_256_colour_fallback_uses_named_colours_only() {
    let buf = draw(&dash(), &Theme::ansi(), 110, 30);
    for cell in &buf.content {
        for colour in [cell.fg, cell.bg] {
            assert!(
                !matches!(colour, Color::Rgb(..) | Color::Indexed(_)),
                "{:?} is drawn in {colour:?}",
                cell.symbol()
            );
        }
    }
}

#[test]
fn under_no_colour_the_overview_sets_no_colour() {
    let buf = draw(&dash(), &Theme::mono(), 110, 30);
    for cell in &buf.content {
        assert_eq!(cell.fg, Color::Reset, "{:?}", cell.symbol());
        assert_eq!(cell.bg, Color::Reset, "{:?}", cell.symbol());
    }
}

// ---------------------------------------------------------------- states

#[test]
fn the_empty_the_first_run_the_one_project_and_the_unknown_disk_each_draw_something() {
    let states: [(&str, Dashboard); 4] = [
        ("empty", Dashboard::default()),
        (
            "first run",
            Dashboard {
                trend: Trend::FirstScan,
                history: Vec::new(),
                ..dash()
            },
        ),
        (
            "one project",
            Dashboard {
                groups: vec![group("target", Ecosystem::Rust, "cargo build", GB, 2)],
                reclaimable: GB,
                analysed: Analysed {
                    projects: 1,
                    with_rebuild: 1,
                    entries: 1,
                    measured: 1,
                    elapsed: Duration::ZERO,
                    roots: vec![PathBuf::from("/p")],
                },
                now: Now {
                    offerable: 1,
                    offerable_bytes: GB,
                    blocked: Vec::new(),
                    dead: 1,
                    dead_reclaimable: GB,
                },
                ..dash()
            },
        ),
        (
            "disk unknown",
            Dashboard {
                volume: None,
                ..dash()
            },
        ),
    ];
    for (name, state) in &states {
        for (w, h) in [(80, 24), (100, 30), (110, 30), (200, 50), (40, 10), (1, 1)] {
            let out = text(state, w, h);
            if w >= 80 {
                assert!(
                    out.contains("Disk"),
                    "{name} at {w}x{h} draws no disk block:\n{out}"
                );
            }
            assert!(
                out.lines().all(|l| l.chars().count() <= w as usize),
                "{name} at {w}x{h} is wider than its area"
            );
        }
    }
    let unknown = text(&states[3].1, 110, 30);
    assert!(
        unknown.contains("unavailable"),
        "an unreadable disk says so rather than showing a zero:\n{unknown}"
    );
    assert!(
        unknown.contains("Where it is"),
        "the rest still draws:\n{unknown}"
    );
    let one = text(&states[2].1, 110, 30);
    assert!(one.contains("1 project"), "{one}");
    assert!(!one.contains("1 projects"), "{one}");
    assert!(one.contains("2 dirs"), "{one}");
    let empty = text(&states[0].1, 110, 30);
    assert!(
        empty.contains("Nothing to rebuild on these roots."),
        "{empty}"
    );
}

#[test]
fn the_analysed_block_says_what_the_scan_looked_at() {
    let out = text(&dash(), 110, 30);
    for expected in [
        "267 projects",
        "41 with something to rebuild",
        "136,432 entries walked",
        "67 directories measured",
        "1.0 s",
        "/Users/me/projects",
    ] {
        assert!(out.contains(expected), "{expected:?} missing:\n{out}");
    }
}

#[test]
fn a_history_that_could_not_be_read_is_said_and_a_first_scan_is_said() {
    let first = Dashboard {
        trend: Trend::FirstScan,
        history: Vec::new(),
        ..dash()
    };
    assert!(text(&first, 110, 30).to_lowercase().contains("first scan"));
    let broken = Dashboard {
        trend: Trend::Unavailable("locked".into()),
        history: Vec::new(),
        ..dash()
    };
    let out = text(&broken, 110, 30);
    assert!(out.contains("History unavailable"), "{out}");
    assert!(out.contains("locked"), "{out}");
}

#[test]
fn a_flat_history_has_no_trend_to_report() {
    let mut d = dash();
    d.trend = Trend::FirstScan;
    d.history = vec![Some(d.reclaimable), Some(d.reclaimable)];
    assert!(!text(&d, 110, 40).contains("Since last scan"));
}
