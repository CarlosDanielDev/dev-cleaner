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
use dev_cleaner::config::Config;
use dev_cleaner::purge::execute;
use dev_cleaner::tui::{
    Confirm, KeyPress, PURGE, Report, Screen, Screens, Step, Tui, collect, palette,
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
