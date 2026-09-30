//! The driver: one walk becoming three screens, and keys becoming moves.
//!
//! Neither half needs a terminal. The adapters are a function of a directory
//! tree, and the loop's dispatch is a function of a key and a screen, so both
//! are driven here the way `tests/tui.rs` drives the router.

pub mod common;

use std::path::PathBuf;
use std::time::{Duration, Instant};

use common::Fixture;
use common::purge::{Recorder, candidate, confirmed};
use dev_cleaner::bytes::human;
use dev_cleaner::config::Config;
use dev_cleaner::purge::execute;
use dev_cleaner::store::Store;
use dev_cleaner::tui::{
    Confirm, KeyPress, PURGE, Report, Screen, Screens, Step, Tui, bindings_for, collect, footer,
    palette, wayfinding,
};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier};

/// Build every screen from a fixture tree, with the history kept outside it.
///
/// The store deliberately lives in a second temporary directory: a database
/// inside the scanned root would be walked, measured, and counted as part of
/// what the fixture holds.
fn screens(fx: &Fixture, store: &Fixture) -> Screens {
    let cfg = Config {
        roots: vec![fx.root().to_path_buf()],
        // No caches: probing them would reach out of the fixture and into the
        // developer's own home directory.
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

/// A node project with a build directory holding `bytes` of content.
fn node_project(fx: &Fixture, name: &str, bytes: usize) {
    fx.file(&format!("{name}/package.json"), b"{}");
    fx.file(&format!("{name}/src/index.js"), b"console.log(1)");
    fx.file(
        &format!("{name}/node_modules/dep/blob.bin"),
        &vec![0xABu8; bytes],
    );
}

#[test]
fn one_walk_builds_the_dashboard_the_table_and_the_candidates() {
    // The adapters are the only place a scan becomes screens. A change to the
    // shape of a scan has to break here rather than in the binary.
    let fx = Fixture::new();
    let store = Fixture::new();
    node_project(&fx, "app", 4096);
    fx.file("lib/Cargo.toml", b"[package]\nname = \"lib\"\n");
    fx.file("lib/src/lib.rs", b"// lib");
    fx.file("lib/target/debug/blob.bin", &vec![0xCDu8; 8192]);

    let screens = screens(&fx, &store);

    assert_eq!(
        screens.projects.rows().len(),
        2,
        "a package.json and a Cargo.toml are two projects"
    );
    assert_eq!(
        screens.candidates.selectable().len(),
        2,
        "node_modules and target are both offerable"
    );
    assert!(
        screens.dashboard.reclaimable > 0,
        "the dashboard has to know what the candidates add up to"
    );
    assert!(
        !screens.dashboard.consumers.is_empty(),
        "the top-consumers list is built from the same grouping"
    );
    let rust = screens
        .projects
        .rows()
        .iter()
        .find(|r| r.name() == "lib")
        .expect("the rust project is in the table");
    assert!(
        rust.reclaimable > 0 && rust.reclaimable <= rust.bytes_unique,
        "a project's reclaimable part is measured inside it, not beside it"
    );
}

#[test]
fn a_file_hardlinked_into_two_artifact_directories_is_counted_once_on_the_disk() {
    // #45: every artifact directory is measured with `Usage::of` over its
    // group, so each one reports what deleting it on its own would return. The
    // disk-level total is a measurement of the union, not a sum of those, or it
    // would promise the same blocks twice.
    let fx = Fixture::new();
    let store = Fixture::new();
    fx.file("a/package.json", b"{}");
    fx.file("b/package.json", b"{}");
    let blob = fx.file("a/node_modules/.store/blob.bin", &vec![0x5Au8; 262_144]);
    fx.hardlink("b/node_modules/.store/blob.bin", &blob);

    let screens = screens(&fx, &store);

    assert_eq!(
        screens.candidates.selectable().len(),
        2,
        "both directories are offerable on their own"
    );
    let summed: u64 = screens
        .candidates
        .selectable()
        .iter()
        .map(|c| c.bytes)
        .sum();
    assert!(
        screens.dashboard.reclaimable < summed,
        "the disk holds one copy of the blob; adding the two directories up \
         offers it twice ({summed} summed against {} on the disk)",
        screens.dashboard.reclaimable
    );
}

#[test]
fn the_history_keeps_the_number_the_gauge_shows() {
    // The gauge and the sparkline under it must be the same measurement, or
    // the first number on the dashboard that is not the number the tool
    // promises would be the one drawn beneath it. Hardlinked so that a sum
    // of the directories and the union of their files differ: equality here
    // proves the store kept the union.
    let fx = Fixture::new();
    let store = Fixture::new();
    fx.file("a/package.json", b"{}");
    fx.file("b/package.json", b"{}");
    let blob = fx.file("a/node_modules/.store/blob.bin", &vec![0x5Au8; 262_144]);
    fx.hardlink("b/node_modules/.store/blob.bin", &blob);

    let screens = screens(&fx, &store);

    let history = Store::open(&store.root().join("history.sqlite3"))
        .expect("open history")
        .history(&[fx.root().to_path_buf()], 10)
        .expect("history");
    assert_eq!(history.len(), 1, "one walk is one recorded scan");
    assert_eq!(
        history[0].1,
        Some(screens.dashboard.reclaimable),
        "the recorded total is not the total the gauge shows"
    );
}

#[test]
fn a_sparse_file_counts_as_what_it_occupies_not_as_what_it_claims() {
    let fx = Fixture::new();
    let store = Fixture::new();
    fx.file("p/package.json", b"{}");
    fx.sparse_file("p/node_modules/dep/huge.img", 64 * 1024 * 1024);

    let screens = screens(&fx, &store);

    let row = &screens.projects.rows()[0];
    assert!(
        row.bytes_apparent > row.bytes_unique,
        "a 64 MB sparse file is not 64 MB of disk"
    );
    assert!(
        row.reclaimable < 64 * 1024 * 1024,
        "and it must not be offered as though it were"
    );
}

/// Every key a terminal can report through this keymap.
fn every_key() -> Vec<KeyPress> {
    let mut keys: Vec<KeyPress> = ('a'..='z')
        .chain('A'..='Z')
        .chain('0'..='9')
        .chain(['?', '/', '.', ',', ';', '\''])
        .map(KeyPress::Char)
        .collect();
    keys.extend([
        KeyPress::Enter,
        KeyPress::Esc,
        KeyPress::Tab,
        KeyPress::Space,
        KeyPress::Backspace,
        KeyPress::Delete,
        KeyPress::Up,
        KeyPress::Down,
        KeyPress::Left,
        KeyPress::Right,
        KeyPress::Home,
        KeyPress::End,
        KeyPress::PageUp,
        KeyPress::PageDown,
    ]);
    keys
}

/// A driver sitting on `screen`, reached by walking the router's own path.
fn driver_on(fx: &Fixture, store: &Fixture, screen: Screen) -> Tui {
    let mut tui = Tui::new(screens(fx, store));
    let now = Instant::now();
    while tui.app().screen() != screen {
        let before = tui.app().screen();
        assert_eq!(
            tui.press(KeyPress::Enter, now),
            Step::Stay,
            "advancing must never be a purge"
        );
        assert_ne!(
            tui.app().screen(),
            before,
            "{screen:?} is not reachable by advancing"
        );
    }
    tui
}

#[test]
fn no_single_key_on_any_screen_reaches_a_purge() {
    // The loop has no navigation of its own: it looks a key up in the table and
    // hands the result to the screen that owns it. This is that claim, driven
    // over every key the table can name rather than over the ones a loop was
    // written to expect.
    let fx = Fixture::new();
    let store = Fixture::new();
    node_project(&fx, "app", 4096);

    for screen in [
        Screen::Dashboard,
        Screen::Projects,
        Screen::Candidates,
        Screen::Review,
        Screen::Confirm,
    ] {
        for key in every_key() {
            let mut tui = driver_on(&fx, &store, screen);
            assert_ne!(
                tui.press(key, Instant::now()),
                Step::Purge,
                "{key:?} purged from {screen:?} on a single press"
            );
        }
    }
}

#[test]
fn only_a_sustained_hold_on_the_confirm_screen_reaches_a_purge() {
    // The counterpart to the test above: without this one, a loop that never
    // purged at all would pass it.
    let fx = Fixture::new();
    let store = Fixture::new();
    node_project(&fx, "app", 4096);
    let mut tui = driver_on(&fx, &store, Screen::Confirm);

    let start = Instant::now();
    let mut reached = None;
    for repeat in 0..80u32 {
        let at = start + Duration::from_millis(50 * u64::from(repeat));
        if tui.press(PURGE, at) == Step::Purge {
            reached = Some(repeat);
            break;
        }
    }

    let repeats = reached.expect("holding the key long enough must arm the purge");
    assert!(
        repeats >= 30,
        "arming took {repeats} key repeats; the hold is supposed to be a decision"
    );
}

#[test]
fn a_hold_that_stops_starts_again_from_nothing() {
    // A terminal reports no key release, so the loop infers one from a repeat
    // that never arrived. Whatever had been held must count for nothing, or a
    // user could fill the gauge in two sittings without ever holding the key.
    let fx = Fixture::new();
    let store = Fixture::new();
    node_project(&fx, "app", 4096);
    let mut tui = driver_on(&fx, &store, Screen::Confirm);

    let start = Instant::now();
    for repeat in 0..25u32 {
        tui.press(PURGE, start + Duration::from_millis(50 * u64::from(repeat)));
    }
    tui.tick(start + Duration::from_secs(5));

    let again = start + Duration::from_secs(10);
    let mut step = Step::Stay;
    for repeat in 0..25u32 {
        step = tui.press(PURGE, again + Duration::from_millis(50 * u64::from(repeat)));
    }
    assert_ne!(
        step,
        Step::Purge,
        "two partial holds must not add up to one"
    );
}

/// Hold the purge key the way macOS delivers it at its default settings: the
/// press, the first repeat after ~375 ms, then one every ~90 ms. The loop ticks
/// between events, as `drive` does. Returns how long the key had been down when
/// the purge was reached, if it was within `limit`.
fn hold_like_macos(tui: &mut Tui, start: Instant, limit: Duration) -> Option<Duration> {
    let mut at = Duration::ZERO;
    while at <= limit {
        tui.tick(start + at);
        if tui.press(PURGE, start + at) == Step::Purge {
            return Some(at);
        }
        at += if at.is_zero() {
            Duration::from_millis(375)
        } else {
            Duration::from_millis(90)
        };
    }
    None
}

#[test]
fn holding_the_key_at_the_default_macos_repeat_fills_the_gauge_in_the_time_it_claims() {
    // The gauge used to count capped events, so at a real repeat rate it filled
    // at about half the speed of the clock and took ~2.7 s against a 1.5 s
    // threshold. A hold is a length of time; the bar has to agree with a watch.
    let fx = Fixture::new();
    let store = Fixture::new();
    node_project(&fx, "app", 4096);
    let mut tui = driver_on(&fx, &store, Screen::Confirm);

    let held = hold_like_macos(&mut tui, Instant::now(), Duration::from_secs(5))
        .expect("a continuous hold at the default repeat must purge");
    assert!(
        held >= Confirm::HOLD,
        "purged after {held:?}, sooner than the {:?} the screen asks for",
        Confirm::HOLD
    );
    assert!(
        held <= Confirm::HOLD + Duration::from_millis(90),
        "purged after {held:?}; the hold is {:?} and one repeat is 90 ms",
        Confirm::HOLD
    );
}

#[test]
fn taps_spaced_wider_than_the_grace_never_fill_the_gauge() {
    // Discrete presses are what a slip looks like. However many there are, a
    // gap the key could not have been down through starts the hold over.
    let fx = Fixture::new();
    let store = Fixture::new();
    node_project(&fx, "app", 4096);
    let mut tui = driver_on(&fx, &store, Screen::Confirm);

    let start = Instant::now();
    for tap in 0..100u32 {
        let at = start + Duration::from_millis(700 * u64::from(tap));
        tui.tick(at);
        assert_ne!(
            tui.press(PURGE, at),
            Step::Purge,
            "tap {tap} purged; spaced taps are not a hold"
        );
    }
}

#[test]
fn an_event_arriving_after_a_stall_cannot_complete_a_hold() {
    // A laptop waking or a debugger resuming hands the loop one event with a
    // huge gap and no tick in between — `poll` was blocked the whole time. That
    // event must not carry the gauge the rest of the way, and must not count
    // what was held before the stall either.
    let fx = Fixture::new();
    let store = Fixture::new();
    node_project(&fx, "app", 4096);
    let mut tui = driver_on(&fx, &store, Screen::Confirm);

    let start = Instant::now();
    let mut at = start;
    for _ in 0..12 {
        assert_eq!(tui.press(PURGE, at), Step::Stay);
        at += Duration::from_millis(90);
    }
    // No tick: the stall is precisely the stretch the loop never saw.
    let woke = at + Duration::from_secs(30);
    assert_ne!(
        tui.press(PURGE, woke),
        Step::Purge,
        "one event after a stall purged"
    );

    // And the hold restarts from the stalled event, so it takes a full hold
    // again rather than whatever was left before the stall.
    let held = hold_like_macos(
        &mut tui,
        woke + Duration::from_millis(90),
        Duration::from_secs(5),
    )
    .expect("holding again after the stall still purges");
    assert!(
        held + Duration::from_millis(90) >= Confirm::HOLD,
        "purged {held:?} into the second hold; the first one leaked through the stall"
    );
}

#[test]
fn a_hold_does_not_survive_leaving_the_screen() {
    // #78: the gauge and the clock behind it lived beside the router, so a
    // hold that had run 1.4 s of its 1.5 s survived Esc, Enter and the round
    // trip through review, and the next single event of the key finished it.
    // Leaving the screen by any route has to count for nothing, exactly as
    // the key coming up does.
    let fx = Fixture::new();
    let store = Fixture::new();
    node_project(&fx, "app", 4096);
    let mut tui = driver_on(&fx, &store, Screen::Confirm);

    let start = Instant::now();
    let last = Duration::from_millis(1455);
    assert!(
        hold_like_macos(&mut tui, start, last).is_none(),
        "the gauge must not arm within {last:?}"
    );

    let mut at = start + last;
    for key in [KeyPress::Esc, KeyPress::Enter] {
        at += Duration::from_millis(50);
        tui.tick(at);
        assert_eq!(tui.press(key, at), Step::Stay);
    }
    assert_eq!(
        tui.app().screen(),
        Screen::Confirm,
        "Enter from review lands on confirm again"
    );

    at += Duration::from_millis(50);
    tui.tick(at);
    assert_eq!(
        tui.press(PURGE, at),
        Step::Stay,
        "a single tap after Esc and Enter completed the hold from before them"
    );
}

#[test]
fn a_deliberate_esc_is_not_a_lapse() {
    // The lapse notice is for a key that went quiet on the screen. After Esc
    // nobody is holding anything, so coming back must not read "the hold
    // lapsed": that is a report of an event that did not happen.
    let fx = Fixture::new();
    let store = Fixture::new();
    node_project(&fx, "app", 4096);
    let mut tui = driver_on(&fx, &store, Screen::Confirm);

    let start = Instant::now();
    let part = Duration::from_millis(735);
    assert!(hold_like_macos(&mut tui, start, part).is_none());

    let mut at = start + part + Duration::from_millis(50);
    tui.tick(at);
    assert_eq!(tui.press(KeyPress::Esc, at), Step::Stay);
    assert_eq!(tui.app().screen(), Screen::Review);
    // The loop keeps ticking on review, well past the grace.
    at += Confirm::GRACE * 2;
    tui.tick(at);
    assert_eq!(tui.press(KeyPress::Enter, at), Step::Stay);
    assert_eq!(tui.app().screen(), Screen::Confirm);

    let shown = text_of(&frame(&mut tui));
    assert!(
        !shown.to_lowercase().contains("lapsed"),
        "Esc was pressed; nothing lapsed:\n{shown}"
    );
}

#[test]
fn going_back_to_the_candidates_and_forward_again_does_not_double_the_plan() {
    // The plan is rebuilt from what is marked every time review is entered.
    // Adding the marks to a draft that already held them would double every
    // path, and the confirmation phrase counts items and bytes.
    let fx = Fixture::new();
    let store = Fixture::new();
    node_project(&fx, "app", 4096);
    let mut tui = driver_on(&fx, &store, Screen::Candidates);
    let now = Instant::now();

    tui.press(KeyPress::Space, now);
    tui.press(KeyPress::Enter, now);
    assert_eq!(tui.app().screen(), Screen::Review);
    let first = tui.app().phrase().expect("a reviewed plan has a phrase");

    tui.press(KeyPress::Esc, now);
    assert_eq!(tui.app().screen(), Screen::Candidates);
    tui.press(KeyPress::Enter, now);

    assert_eq!(tui.app().screen(), Screen::Review);
    assert_eq!(
        tui.app().phrase().expect("reviewed again"),
        first,
        "the same marks must describe the same plan"
    );
}

#[test]
fn an_unmarked_candidate_never_enters_the_plan() {
    let fx = Fixture::new();
    let store = Fixture::new();
    node_project(&fx, "app", 4096);
    node_project(&fx, "web", 8192);
    let mut tui = driver_on(&fx, &store, Screen::Candidates);
    let now = Instant::now();

    tui.press(KeyPress::Space, now);
    tui.press(KeyPress::Enter, now);

    assert_eq!(
        tui.app().phrase().expect("reviewed"),
        {
            let marked = 1;
            let bytes = tui.app().reviewing().expect("reviewed").total_bytes();
            format!("purge {marked} items {bytes} bytes")
        },
        "only what the cursor marked is in the plan"
    );
    assert_eq!(
        tui.app().reviewing().expect("reviewed").items().len(),
        1,
        "the second candidate was never marked"
    );
}

#[test]
fn the_roots_the_screens_were_built_from_travel_with_them() {
    // Free space, and the volume the purge measures, are questions about a
    // root. Losing them between the walk and the loop is how a run comes to
    // measure the wrong disk.
    let fx = Fixture::new();
    let store = Fixture::new();
    node_project(&fx, "app", 4096);

    let screens = screens(&fx, &store);

    assert_eq!(screens.roots, vec![PathBuf::from(fx.root())]);
}

/// Every phrase that may be drawn muted: labels and hints, none of them a fact.
///
/// An allowlist rather than a rule about what counts as meaningful, so text
/// added in muted later fails here until someone decides it belongs on it.
const MAY_BE_MUTED: &[&str] = &[
    "by size",
    "by inodes",
    "project",
    "unique",
    "apparent",
    "inodes",
    "reclaimable",
    "activity",
    "Each line names the command that rebuilds it. Esc to change the plan.",
    "Any key closes this.",
];

/// A fixture that puts something on every row type every screen can draw:
/// candidates, a blocked entry, an apparent size that differs from the real one.
fn busy_fixture(fx: &Fixture) {
    node_project(fx, "app", 4096);
    fx.sparse_file("app/node_modules/dep/huge.img", 16 * 1024 * 1024);
    fx.file("lib/Cargo.toml", b"[package]\nname = \"lib\"\n");
    fx.file("lib/target/debug/blob.bin", &vec![0xCDu8; 8192]);
    // A repository with work nobody committed, so the guards refuse it.
    fx.git_repo("wip", 0);
    node_project(fx, "wip", 1024);
}

/// The body and chrome of `screen`, with everything marked and a lapsed hold on
/// the confirm screen so its notice is drawn too.
fn drawn(fx: &Fixture, store: &Fixture, screen: Screen) -> Buffer {
    let area = Rect::new(0, 0, 120, 40);
    let mut buf = Buffer::empty(area);
    if screen == Screen::Result {
        // ponytail: the result screen is only reachable through a purge, which
        // writes a record under $HOME. Its body is drawn directly; its chrome
        // is the same code every other screen's is.
        let manifest = execute(
            confirmed(vec![candidate("/p/a/node_modules", 1024)]),
            &Recorder::default(),
        );
        Report::new().render(&manifest, None, area, &mut buf);
        return buf;
    }
    let now = Instant::now();
    let mut tui = driver_on(fx, store, Screen::Candidates);
    if matches!(screen, Screen::Dashboard | Screen::Projects) {
        tui = driver_on(fx, store, screen);
    }
    tui.press(KeyPress::Char('a'), now);
    while tui.app().screen() != screen {
        tui.press(KeyPress::Enter, now);
    }
    if screen == Screen::Confirm {
        tui.press(PURGE, now);
        tui.press(PURGE, now + Duration::from_millis(50));
        tui.tick(now + Duration::from_secs(5));
    }
    tui.render(area, &mut buf);
    buf
}

/// Runs of consecutive muted cells on each row, trimmed.
fn muted_runs(buf: &Buffer) -> Vec<String> {
    let area = buf.area;
    let muted = |x, y| {
        let cell = &buf[(x, y)];
        cell.modifier.contains(palette::MUTED.add_modifier)
            && palette::MUTED.fg.is_none_or(|fg| fg == cell.fg)
    };
    let mut runs = Vec::new();
    for y in 0..area.height {
        let mut run = String::new();
        for x in 0..=area.width {
            if x < area.width && muted(x, y) {
                run.push_str(buf[(x, y)].symbol());
            } else if !run.trim().is_empty() {
                runs.push(std::mem::take(&mut run).trim().to_string());
            } else {
                run.clear();
            }
        }
    }
    runs
}

#[test]
fn no_screen_draws_a_fact_in_muted() {
    // Muted is for what can be skipped. A size, a path, a command, a reason or a
    // key binding drawn in it is the information this issue was about losing.
    let fx = Fixture::new();
    let store = Fixture::new();
    busy_fixture(&fx);

    for screen in Screen::all() {
        let buf = drawn(&fx, &store, screen);
        for run in muted_runs(&buf) {
            assert!(
                MAY_BE_MUTED.iter().any(|allowed| allowed.contains(&run)),
                "{screen:?} draws {run:?} muted, and it is not a label or a hint"
            );
        }
    }
}

#[test]
fn nothing_is_drawn_in_the_colour_terminals_paint_like_the_background() {
    // Bright black is the ANSI colour many profiles render within a shade of
    // their own background. No palette choice rescues it, so none may use it.
    let fx = Fixture::new();
    let store = Fixture::new();
    busy_fixture(&fx);

    for screen in Screen::all() {
        let buf = drawn(&fx, &store, screen);
        assert!(
            buf.content.iter().all(|c| c.fg != Color::DarkGray),
            "{screen:?} draws in bright black"
        );
    }
}

#[test]
fn the_confirm_screen_is_told_apart_by_more_than_colour() {
    // The one screen that deletes must not look like the ones that list. Told
    // apart by weight, so a monochrome terminal and a colour-blind reader see it.
    let fx = Fixture::new();
    let store = Fixture::new();
    busy_fixture(&fx);

    for screen in Screen::all() {
        if screen == Screen::Result {
            continue;
        }
        let buf = drawn(&fx, &store, screen);
        let banded = (0..buf.area.width).all(|x| buf[(x, 0)].modifier.contains(Modifier::REVERSED));
        assert_eq!(
            banded,
            screen == Screen::Confirm,
            "{screen:?}: only the confirm screen's title is a reversed band"
        );
    }
}

#[test]
fn colours_are_named_in_the_palette_and_nowhere_else() {
    let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/src/tui");
    for entry in std::fs::read_dir(dir).expect("src/tui") {
        let path = entry.expect("entry").path();
        if path.file_name().is_some_and(|n| n == "palette.rs") {
            continue;
        }
        let source = std::fs::read_to_string(&path).expect("source");
        assert!(
            !source.contains("Color::"),
            "{} builds a colour outside the palette",
            path.display()
        );
    }
}

#[test]
fn the_blocked_fixture_really_draws_a_blocked_row() {
    // Without one the muted sweep above never sees the rows most likely to go
    // grey again.
    let fx = Fixture::new();
    let store = Fixture::new();
    busy_fixture(&fx);
    assert!(!screens(&fx, &store).candidates.blocked().is_empty());
}

/// The size every list test draws at: 27 rows of body under a two-row title.
const LIST_AREA: Rect = Rect::new(0, 0, 120, 30);

/// How many items the long plan holds: more than the 23 rows its list is given
/// at [`LIST_AREA`], fewer than the 27 the body has. That gap is the plan the
/// recording caught, whose last rows no key could scroll to.
const LONG_PLAN: usize = 25;

/// A tree of `n` projects with one candidate each, sized apart so they sort
/// the same way every run.
fn many_projects(fx: &Fixture, n: usize) {
    for i in 0..n {
        node_project(fx, &format!("p{i:02}"), 1024 + i);
    }
}

/// The whole interface at [`LIST_AREA`], as the loop would draw it.
fn frame(tui: &mut Tui) -> Buffer {
    let mut buf = Buffer::empty(LIST_AREA);
    tui.render(LIST_AREA, &mut buf);
    buf
}

/// Every row of `buf`, joined.
fn text_of(buf: &Buffer) -> String {
    (0..buf.area.height)
        .map(|y| {
            (0..buf.area.width)
                .map(|x| buf[(x, y)].symbol())
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// A driver on `screen`, part way down a list longer than the screen, with one
/// thing marked: a state from which every key the screen binds has somewhere
/// to go.
fn primed(fx: &Fixture, store: &Fixture, screen: Screen) -> Tui {
    let now = Instant::now();
    let reviewing = matches!(screen, Screen::Review | Screen::Confirm);
    let mut tui = driver_on(
        fx,
        store,
        if reviewing {
            Screen::Candidates
        } else {
            screen
        },
    );
    // Drawn first, as the loop does: scrolling moves by what the last frame
    // showed.
    frame(&mut tui);
    let mut press = |key, times| {
        for _ in 0..times {
            tui.press(key, now);
        }
    };
    match screen {
        Screen::Projects => press(KeyPress::Down, 5),
        Screen::Candidates => {
            press(KeyPress::PageDown, 3);
            press(KeyPress::Space, 1);
        }
        Screen::Review | Screen::Confirm => {
            for _ in 0..LONG_PLAN {
                press(KeyPress::Space, 1);
                press(KeyPress::Down, 1);
            }
            press(KeyPress::Enter, 1);
            press(
                if screen == Screen::Review {
                    KeyPress::Down
                } else {
                    KeyPress::Enter
                },
                1,
            );
        }
        _ => {}
    }
    frame(&mut tui);
    tui
}

#[test]
fn every_key_a_screen_binds_changes_what_it_shows() {
    // A key in the table is a promise, read back to the user in the footer and
    // the help. One that moves nothing cannot be told apart from one that is
    // broken, so each is pressed once from a state where it has room to act,
    // and the screen has to differ afterwards.
    //
    // The result screen is left out: it is reached only through a real purge.
    // What it binds is global, and every global is driven on the screens here.
    let fx = Fixture::new();
    let store = Fixture::new();
    many_projects(&fx, 60);

    let mut dead = Vec::new();
    for screen in Screen::all() {
        if screen == Screen::Result {
            continue;
        }
        for binding in bindings_for(screen) {
            let mut tui = primed(&fx, &store, screen);
            let before = frame(&mut tui);
            let now = Instant::now();
            let mut step = tui.press(binding.key, now);
            if binding.key == PURGE {
                // A hold is a stretch of time: the first press only starts it.
                step = tui.press(PURGE, now + Duration::from_millis(200));
            }
            if step == Step::Stay && before == frame(&mut tui) {
                dead.push(format!("{screen:?}: `{}` ({})", binding.key, binding.label));
            }
        }
    }
    assert!(
        dead.is_empty(),
        "bound, and change nothing:\n{}",
        dead.join("\n")
    );
}

#[test]
fn every_list_says_where_it_is_even_when_it_shows_everything() {
    // Pressing `j` on a list that already shows all of itself does nothing, and
    // that is only readable as "complete" rather than "broken" if the list
    // says it is complete.
    let now = Instant::now();
    let short = Fixture::new();
    let long = Fixture::new();
    let store = Fixture::new();
    many_projects(&short, 2);
    many_projects(&long, 60);

    for (fx, marks, projects, candidates, plan) in [
        (
            &short,
            2,
            "showing 1-2 of 2",
            "showing 1-2 of 2",
            "showing 1-2 of 2",
        ),
        (
            &long,
            LONG_PLAN,
            "showing 1-25 of 60",
            "showing 1-26 of 60",
            "showing 1-23 of 25",
        ),
    ] {
        let mut tui = driver_on(fx, &store, Screen::Projects);
        let shown = text_of(&frame(&mut tui));
        assert!(shown.contains(projects), "projects:\n{shown}");

        tui.press(KeyPress::Enter, now);
        let shown = text_of(&frame(&mut tui));
        assert!(shown.contains(candidates), "candidates:\n{shown}");

        for _ in 0..marks {
            tui.press(KeyPress::Space, now);
            tui.press(KeyPress::Down, now);
        }
        tui.press(KeyPress::Enter, now);
        let shown = text_of(&frame(&mut tui));
        assert!(shown.contains(plan), "the plan:\n{shown}");
    }
}

#[test]
fn the_selected_row_is_highlighted_across_the_whole_width() {
    // Highlighted cell by cell, a row reads as separate blocks with gaps
    // between them, which looks like a fault in drawing rather than a cursor.
    let fx = Fixture::new();
    let store = Fixture::new();
    many_projects(&fx, 3);

    for screen in [Screen::Projects, Screen::Candidates] {
        let mut tui = driver_on(&fx, &store, screen);
        let buf = frame(&mut tui);
        let reversed = |x, y| buf[(x, y)].modifier.contains(Modifier::REVERSED);
        let rows: Vec<u16> = (0..LIST_AREA.height)
            .filter(|&y| (0..LIST_AREA.width).any(|x| reversed(x, y)))
            .collect();
        assert_eq!(rows.len(), 1, "{screen:?}: one row is selected");
        assert!(
            (0..LIST_AREA.width).all(|x| reversed(x, rows[0])),
            "{screen:?}: the selected row has gaps in its highlight"
        );
    }
}

#[test]
fn every_screen_says_where_its_keys_lead_before_they_are_pressed() {
    for screen in Screen::all() {
        let line = wayfinding(screen, (2, 2048));
        if let Some(previous) = screen.previous() {
            assert!(
                line.contains(&format!("Esc ← {}", previous.name())),
                "{screen:?} does not say where Esc goes: {line:?}"
            );
        }
        match screen.next() {
            Some(next) => assert!(
                line.contains(&format!("→ {}", next.name())),
                "{screen:?} does not say where it leads: {line:?}"
            ),
            // The one screen with nowhere forward says so, and says what the
            // keys it still has are for.
            None => {
                assert!(line.contains("the run is over"), "{line:?}");
                for binding in bindings_for(screen) {
                    let key = format!("{} {}", binding.key, binding.label);
                    assert!(line.contains(&key), "{line:?} leaves out {key:?}");
                }
            }
        }
    }
}

#[test]
fn leaving_the_candidates_says_the_plan_is_built_from_the_marks() {
    // The plan is built on the way out of candidates, from what is marked at
    // that moment. Both sides of the step say so, with the count and the total,
    // so leaving the screen reads as the thing that committed the marks.
    let fx = Fixture::new();
    let store = Fixture::new();
    node_project(&fx, "a", 4096);
    node_project(&fx, "b", 4096);
    let now = Instant::now();
    let mut tui = driver_on(&fx, &store, Screen::Candidates);
    tui.press(KeyPress::Char('a'), now);
    let total: u64 = candidates_total(&fx, &store);

    let row = |tui: &mut Tui| {
        text_of(&frame(tui))
            .lines()
            .nth(1)
            .unwrap_or("")
            .to_string()
    };
    let before = row(&mut tui);
    assert!(
        before.contains(&format!(
            "the plan, built from the 2 marked ({})",
            human(total)
        )),
        "{before:?}"
    );
    tui.press(KeyPress::Enter, now);
    assert_eq!(tui.app().screen(), Screen::Review);
    let after = row(&mut tui);
    assert!(
        after.contains(&format!("built from the 2 you marked ({})", human(total))),
        "{after:?}"
    );
}

/// What the two node projects' candidates add up to, as the screen measures it.
fn candidates_total(fx: &Fixture, store: &Fixture) -> u64 {
    screens(fx, store)
        .candidates
        .selectable()
        .iter()
        .map(|c| c.bytes)
        .sum()
}

/// The widths the key bar is read at: a split pane, an ordinary window, a wide one.
const WIDTHS: [u16; 3] = [60, 100, 200];

/// Whether `entry` is one of `screen`'s bindings drawn whole: every key that
/// shares a label, then that label, with nothing cut off either end.
fn is_whole_entry(screen: Screen, entry: &str) -> bool {
    let own = bindings_for(screen);
    own.iter().any(|b| {
        entry
            .strip_suffix(b.label)
            .and_then(|keys| keys.strip_suffix(' '))
            .is_some_and(|keys| {
                keys.split('/').all(|k| {
                    own.iter()
                        .any(|o| o.label == b.label && o.key.to_string() == k)
                })
            })
    })
}

#[test]
fn the_key_bar_never_cuts_an_entry_and_always_keeps_the_way_out() {
    // The recording's projects bar ended in `5 by reclaim`, cut inside the word,
    // with nothing to say the line went on. Every entry drawn is drawn whole,
    // anything left out is admitted to, and `?` — which lists all of them — is
    // always there to press. Every screen at every width is checked before
    // anything fails, so a regression reports all of what it broke.
    let fx = Fixture::new();
    let store = Fixture::new();
    many_projects(&fx, 60);

    let mut wrong = Vec::new();
    for screen in Screen::all() {
        for width in WIDTHS {
            let bar = footer(screen, width as usize - 2);
            let mut check = |ok: bool, what: &str| {
                if !ok {
                    wrong.push(format!("{screen:?} at {width}: {what} in {bar:?}"));
                }
            };
            let entries: Vec<&str> = bar.split("   ").collect();
            check(
                bar.chars().count() <= width as usize - 2,
                "wider than the room",
            );
            for entry in &entries {
                check(
                    *entry == "…" || is_whole_entry(screen, entry),
                    &format!("{entry:?} is not a whole entry"),
                );
            }
            check(entries.contains(&"q quit"), "no `q`");
            check(entries.last() == Some(&"? keys"), "`?` is not last");

            let labels: Vec<&str> = entries
                .iter()
                .filter_map(|e| e.split_once(' '))
                .map(|(_, l)| l)
                .collect();
            let mut unique = labels.clone();
            unique.sort_unstable();
            unique.dedup();
            check(unique.len() == labels.len(), "a label is shown twice");
            let dropped = bindings_for(screen)
                .iter()
                .any(|b| !labels.contains(&b.label));
            check(
                dropped == entries.contains(&"…"),
                "what was left out and the `…` disagree",
            );

            // And the bar the loop draws is this one. The result screen is only
            // reachable through a real purge; its footer is the same function.
            if screen != Screen::Result {
                let mut tui = primed(&fx, &store, screen);
                let area = Rect::new(0, 0, width, LIST_AREA.height);
                let mut buf = Buffer::empty(area);
                tui.render(area, &mut buf);
                let text = text_of(&buf);
                let last = text.lines().last().unwrap_or("").trim();
                check(last == bar, &format!("the loop draws {last:?}"));
            }
        }
    }
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
}

#[test]
fn keys_that_do_the_same_thing_share_one_entry() {
    let bar = footer(Screen::Projects, 198);
    assert!(bar.contains("k/↑ up a row"), "{bar:?}");
    assert!(bar.contains("j/↓ down a row"), "{bar:?}");
    assert!(bar.contains("6 by activity"), "{bar:?}");
}

#[test]
fn what_the_key_bar_gives_up_is_what_matters_least() {
    // At the narrowest width: the key that deletes, the keys that mark, and
    // the keys nobody could guess survive. Arrows and Enter are what anyone
    // tries first, and the row under the title already names Esc and Enter.
    let confirm = footer(Screen::Confirm, 58);
    assert!(confirm.starts_with("x hold to purge"), "{confirm:?}");
    let candidates = footer(Screen::Candidates, 58);
    assert!(candidates.starts_with("Space mark"), "{candidates:?}");
    let projects = footer(Screen::Projects, 58);
    assert!(projects.starts_with("1 by name"), "{projects:?}");
}

#[test]
fn the_key_list_says_when_it_ran_out_of_room() {
    // The overlay used to stop at the bottom edge and say nothing, which reads
    // as a complete list.
    let fx = Fixture::new();
    let store = Fixture::new();
    many_projects(&fx, 3);
    let mut tui = driver_on(&fx, &store, Screen::Candidates);
    tui.press(KeyPress::Char('?'), Instant::now());

    let short = Rect::new(0, 0, 120, 12);
    let mut buf = Buffer::empty(short);
    tui.render(short, &mut buf);
    let shown = text_of(&buf);
    assert!(shown.contains("more than fit here"), "{shown}");

    let full = text_of(&frame(&mut tui));
    assert!(!full.contains("more than fit here"), "{full}");
    assert!(full.contains("j/↓"), "{full}");
    assert!(full.contains("Space"), "{full}");
}

#[test]
fn the_confirm_screen_names_what_the_plan_it_confirms_holds() {
    // The last thing on screen before a purge used to be a count. What is
    // confirmed has to be the plan review showed, total for total, and it has
    // to be named, not counted: at least its largest entries, by path.
    let fx = Fixture::new();
    let store = Fixture::new();
    many_projects(&fx, 30);
    // Sorted last by name, so only ordering by size puts it in view.
    node_project(&fx, "zz-big", 256 * 1024);
    let now = Instant::now();
    let mut tui = driver_on(&fx, &store, Screen::Candidates);
    tui.press(KeyPress::Char('a'), now);
    tui.press(KeyPress::Enter, now);
    assert_eq!(tui.app().screen(), Screen::Review);
    let review = text_of(&frame(&mut tui));
    let heading = review
        .lines()
        .find_map(|l| l.split_once("The plan  (")?.1.split_once(')'))
        .map(|(total, _)| total.to_string())
        .unwrap_or_else(|| panic!("review shows no total:\n{review}"));

    tui.press(KeyPress::Enter, now);
    assert_eq!(tui.app().screen(), Screen::Confirm);
    let confirm = text_of(&frame(&mut tui));
    assert!(
        confirm.contains(&format!("to purge {heading}")),
        "review showed {heading:?}; confirm must show the same:\n{confirm}"
    );

    let plan = tui.app().reviewing().expect("confirm holds a plan");
    let largest = plan.items().iter().max_by_key(|c| c.bytes).expect("items");
    assert!(
        confirm.contains(&largest.path.display().to_string()),
        "the largest entry is not named:\n{confirm}"
    );
    assert!(
        confirm.contains("more"),
        "a plan longer than the screen must say how much is not shown:\n{confirm}"
    );
}
