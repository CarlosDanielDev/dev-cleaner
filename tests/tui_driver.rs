//! The driver: one walk becoming three screens, and keys becoming moves.
//!
//! Neither half needs a terminal. The adapters are a function of a directory
//! tree, and the loop's dispatch is a function of a key and a screen, so both
//! are driven here the way `tests/tui.rs` drives the router.

pub mod common;

use std::path::PathBuf;
use std::time::{Duration, Instant};

use common::Fixture;
use common::contrast::{contrast, rgb};
use common::purge::{Recorder, candidate, confirmed};
use dev_cleaner::bytes::human;
use dev_cleaner::classify::Activity;
use dev_cleaner::config::Config;
use dev_cleaner::purge::execute;
use dev_cleaner::store::Store;
use dev_cleaner::tui::{
    Confirm, KeyPress, NOTICE_TTL, PURGE, Report, Screen, Screens, Step, Trend, Tui, bindings,
    bindings_for, collect, footer, logo,
    palette::{self, Theme},
    wayfinding,
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
fn each_walk_adds_a_point_to_the_sparkline_and_the_newest_is_the_gauge() {
    let fx = Fixture::new();
    let store = Fixture::new();
    node_project(&fx, "a", 4096);

    let first = screens(&fx, &store);
    assert_eq!(first.dashboard.history.len(), 1);
    let second = screens(&fx, &store);

    assert_eq!(second.dashboard.history.len(), 2);
    assert_eq!(
        second.dashboard.history.last(),
        Some(&Some(second.dashboard.reclaimable)),
        "the newest point is the number the gauge shows"
    );
}

#[test]
fn a_store_that_will_not_open_costs_the_line_and_not_the_screen() {
    let fx = Fixture::new();
    let store = Fixture::new();
    node_project(&fx, "a", 4096);
    // A file where the history's directory should be: the database cannot open.
    let blocker = store.file("blocker", b"x");
    let cfg = Config {
        roots: vec![fx.root().to_path_buf()],
        caches: Vec::new(),
        denylist: Vec::new(),
    };

    let screens = collect(
        &[fx.root().to_path_buf()],
        &cfg,
        fx.root(),
        &blocker.join("history.sqlite3"),
    );

    assert!(matches!(screens.dashboard.trend, Trend::Unavailable(_)));
    assert!(screens.dashboard.history.is_empty());
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
    driver_in(fx, store, screen, Theme::ansi())
}

/// [`driver_on`], drawing in `theme`.
fn driver_in(fx: &Fixture, store: &Fixture, screen: Screen, theme: Theme) -> Tui {
    let mut tui = Tui::new(screens(fx, store)).with_theme(theme);
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
    if screen == Screen::Projects {
        // Arriving puts the cursor on the project the dashboard's lead insight
        // is about (#149). These tests start from the top of the table.
        tui.press(KeyPress::Char('g'), now);
    }
    if screen == Screen::Candidates {
        // Arriving shows the table's project alone (#144). These tests are
        // about the whole list, so they widen to it and start from its top.
        tui.press(KeyPress::Tab, now);
        tui.press(KeyPress::Char('g'), now);
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
fn the_plan_built_after_a_sort_is_the_plan_the_user_marked() {
    // Marks are made on rows the user can see, and the plan is built from
    // them on the way out. A reorder in between must not change which
    // directories that is: a mark keyed by row would name whatever moved into
    // the row, and the plan is what reaches the Trash.
    let fx = Fixture::new();
    let store = Fixture::new();
    node_project(&fx, "app", 4096);
    fx.file("lib/Cargo.toml", b"[package]\nname = \"lib\"\n");
    fx.file("lib/target/debug/blob.bin", &vec![0xCDu8; 65_536]);
    node_project(&fx, "web", 262_144);

    // The two the cursor will mark: the first two rows as the screen opens.
    let opening: Vec<PathBuf> = screens(&fx, &store)
        .candidates
        .selectable()
        .iter()
        .map(|c| c.path.clone())
        .collect();
    let mut chosen = opening[..2].to_vec();
    chosen.sort();
    let mut by_path = opening.clone();
    by_path.sort();
    assert_ne!(
        chosen,
        by_path[..2],
        "sorting by path must move a marked entry, or this proves nothing"
    );

    let mut tui = driver_on(&fx, &store, Screen::Candidates);
    let now = Instant::now();
    tui.press(KeyPress::Space, now);
    tui.press(KeyPress::Down, now);
    tui.press(KeyPress::Space, now);
    tui.press(KeyPress::Char('1'), now);
    tui.press(KeyPress::Enter, now);

    let mut planned: Vec<PathBuf> = tui
        .app()
        .reviewing()
        .expect("reviewed")
        .items()
        .iter()
        .map(|c| c.path.clone())
        .collect();
    planned.sort();
    assert_eq!(
        planned, chosen,
        "the plan holds what was marked, not what moved into its rows"
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
    // The header row of the candidates screen and of the plan.
    "size",
    "kind",
    "project / path",
    "comes back as",
    "project",
    "unique",
    "apparent",
    "inodes",
    "reclaimable",
    "activity",
    "Each line names the command that rebuilds it. Esc to change the plan.",
    "Any key closes this.",
    // What there is none of, in the table's reclaimable column (#152).
    "0 B",
    // Field labels and the labels of the disk gauge's legend.
    "Planned",
    "Moved",
    "Reclaimed on disk",
    "Waiting in the Trash",
    "rebuildable",
    "other",
    "free",
    // The dashboard's glue words: what a count is a count of.
    "with something to rebuild",
    "entries walked",
    "directories measured",
    "roots",
    "most files",
    "inodes",
    // Structure between parts of a row: a breadcrumb's arrows and its dots.
    "←",
    "→",
    "·",
    " of the plan",
];

/// A fixture that puts something on every row type every screen can draw:
/// candidates, a blocked entry, an apparent size that differs from the real
/// one, a project the table calls dead.
fn busy_fixture(fx: &Fixture) {
    node_project(fx, "app", 4096);
    fx.sparse_file("app/node_modules/dep/huge.img", 16 * 1024 * 1024);
    fx.file("lib/Cargo.toml", b"[package]\nname = \"lib\"\n");
    fx.file("lib/target/debug/blob.bin", &vec![0xCDu8; 8192]);
    // A repository with work nobody committed, so the guards refuse it.
    fx.git_repo("wip", 0);
    node_project(fx, "wip", 1024);
    dead_project(fx, "old", 2048);
}

/// A node project nobody has touched in 200 days, every commit on a remote,
/// with a build directory the guards clear.
///
/// The walker reads hidden directories, so `.git` itself counts as source
/// evidence: every file under the project is backdated, or the checkout
/// written a moment ago reads as work done today. The index is then settled
/// and dated after the files it describes, because the dirty-tree guard runs
/// `git status`, and `git status` rewrites an index whose stat cache is stale
/// or racy — which would make the second walk over this tree read the
/// project as touched today.
fn dead_project(fx: &Fixture, name: &str, bytes: usize) {
    fx.file(&format!("{name}/.gitignore"), b"node_modules\n");
    fx.file(&format!("{name}/package.json"), b"{}");
    fx.file(&format!("{name}/src/index.js"), b"console.log(1)");
    fx.git_repo(name, 200);
    fx.mark_pushed(name);
    fx.file(
        &format!("{name}/node_modules/dep/blob.bin"),
        &vec![0xABu8; bytes],
    );

    fn set_mtime(path: &std::path::Path, when: std::time::SystemTime) {
        std::fs::File::open(path)
            .expect("open")
            .set_modified(when)
            .expect("set mtime");
    }
    fn backdate(dir: &std::path::Path, when: std::time::SystemTime) {
        for entry in std::fs::read_dir(dir).expect("read_dir") {
            let path = entry.expect("entry").path();
            if path.is_dir() {
                backdate(&path, when);
            } else {
                set_mtime(&path, when);
            }
        }
    }
    let day = Duration::from_secs(86_400);
    let when = std::time::SystemTime::now() - 200 * day;
    backdate(&fx.root().join(name), when);
    fx.git(name, &["status", "--porcelain"]);
    set_mtime(&fx.root().join(name).join(".git/index"), when + day);
}

/// The body and chrome of `screen`, with everything marked and a lapsed hold on
/// the confirm screen so its notice is drawn too.
fn drawn(fx: &Fixture, store: &Fixture, screen: Screen) -> Buffer {
    drawn_in(fx, store, screen, Theme::ansi())
}

/// [`drawn`], in `theme`.
fn drawn_in(fx: &Fixture, store: &Fixture, screen: Screen, theme: Theme) -> Buffer {
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
        buf.set_style(area, theme.ground);
        Report::new().render(&theme, &manifest, None, area, &mut buf);
        return buf;
    }
    let now = Instant::now();
    let mut tui = driver_in(fx, store, Screen::Candidates, theme);
    if matches!(screen, Screen::Dashboard | Screen::Projects) {
        tui = driver_in(fx, store, screen, theme);
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
        cell.modifier.contains(Theme::ansi().muted.add_modifier)
            && Theme::ansi().muted.fg.is_none_or(|fg| fg == cell.fg)
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
            // The unfilled cells of a bar are the muted role: a bar's empty
            // part is what is still to go, and it is the cells that say so.
            let cells = run.chars().all(|c| ['▱', '-', '[', ']'].contains(&c));
            // What a sparkline measures is its label; its figures are not.
            let label = run.starts_with("over the last ");
            assert!(
                cells
                    || label
                    || MAY_BE_MUTED.iter().any(|allowed| allowed.contains(&run))
                    || bindings().iter().any(|b| b.label == run),
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
        // On the screen's header band the red bar is the row after the icon's.
        let banded = (0..buf.area.width).all(|x| {
            buf[(x, logo::TOP - 1)]
                .modifier
                .contains(Modifier::REVERSED)
        });
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
    let mut dirs = vec![std::path::PathBuf::from(dir)];
    while let Some(dir) = dirs.pop() {
        for entry in std::fs::read_dir(&dir).expect("src/tui") {
            let path = entry.expect("entry").path();
            if path.is_dir() {
                dirs.push(path);
                continue;
            }
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
}

#[test]
fn the_dashboard_counts_the_objects_the_candidates_screen_and_the_table_hold() {
    // The opening screen's numbers are read off the screens it points at, not
    // counted again from the scan. A second count by another formula would be
    // the first number on the dashboard free to disagree with the screen behind
    // it.
    let fx = Fixture::new();
    let store = Fixture::new();
    busy_fixture(&fx);

    // One `Screens`, counted and then drawn. A second walk is a second scan.
    let screens = screens(&fx, &store);
    let now = screens.dashboard.now.clone();
    let offered = screens.candidates.selectable();
    let blocked = screens.candidates.blocked().len();
    let dead: Vec<_> = screens
        .projects
        .rows()
        .iter()
        .filter(|r| r.activity == Activity::Dead)
        .collect();

    assert!(!offered.is_empty() && blocked > 0 && !dead.is_empty());
    assert_eq!(now.offerable, offered.len());
    assert_eq!(
        now.offerable_bytes,
        offered.iter().map(|c| c.bytes).sum::<u64>()
    );
    assert_eq!(
        now.blocked.iter().map(|(_, n)| n).sum::<usize>(),
        blocked,
        "every blocked entry is under exactly one reason"
    );
    assert_eq!(now.dead, dead.len());
    assert_eq!(
        now.dead_reclaimable,
        dead.iter().map(|r| r.reclaimable).sum::<u64>()
    );

    let mut tui = Tui::new(screens);
    let shown = text_of(&frame(&mut tui));
    for line in [
        format!(
            "{blocked} {} kept",
            if blocked == 1 { "entry" } else { "entries" }
        ),
        format!("{} dead project", now.dead),
        "Biggest win".to_string(),
    ] {
        assert!(shown.contains(&line), "{line:?} is not drawn:\n{shown}");
    }
}

#[test]
fn the_breakdown_adds_up_to_the_reclaimable_total_even_across_a_hardlink() {
    // #45 one level up: an inode reachable from two kinds is counted once, so
    // the rows of "Where it is" add up to the figure the disk shows.
    let fx = Fixture::new();
    let store = Fixture::new();
    busy_fixture(&fx);
    let shared = fx.root().join("app/node_modules/dep/huge.img");
    fx.file("app/node_modules/dep/real.bin", &vec![0x5Au8; 64 * 1024]);
    fx.hardlink(
        "lib/target/debug/same-inode.bin",
        &fx.root().join("app/node_modules/dep/real.bin"),
    );
    assert!(shared.exists());

    let screens = screens(&fx, &store);
    let dash = &screens.dashboard;
    let kinds: Vec<&str> = dash.groups.iter().map(|g| g.label.as_str()).collect();

    assert!(
        kinds.contains(&"node_modules") && kinds.contains(&"target"),
        "{kinds:?}"
    );
    assert_eq!(
        dash.groups.iter().map(|g| g.bytes).sum::<u64>(),
        dash.reclaimable,
        "the rows and the gauge disagree: {:?}",
        dash.groups
    );
    assert_eq!(dash.analysed.projects, screens.projects.rows().len());
    assert_eq!(
        dash.analysed.measured,
        dash.groups.iter().map(|g| g.dirs).sum::<usize>()
    );
    let offered: u64 = screens
        .candidates
        .selectable()
        .iter()
        .map(|c| c.bytes)
        .sum();
    assert_eq!(
        dash.groups.iter().map(|g| g.offerable_bytes).sum::<u64>(),
        offered,
        "what the groups say is offerable is what the candidates screen offers"
    );
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

/// The size every list test draws at: 26 rows of body under the header band's
/// six rows, over the notice row and the key bar.
const LIST_AREA: Rect = Rect::new(0, 0, 120, 34);

/// The row the way is drawn on: the band's fourth, under the screen's name.
const WAY_ROW: u16 = 3;

/// How many items the long plan holds: more than the 22 rows its list is given
/// at [`LIST_AREA`], fewer than the 26 the body has. That gap is the plan the
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
            // One page is a window, which is under half of sixty entries: far
            // enough down to have room above and below.
            press(KeyPress::PageDown, 1);
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
            "showing 1-22 of 60",
            "showing 1-20 of 60",
            "showing 1-11 of 25",
        ),
    ] {
        let mut tui = driver_on(fx, &store, Screen::Projects);
        let shown = text_of(&frame(&mut tui));
        assert!(shown.contains(projects), "projects:\n{shown}");

        tui.press(KeyPress::Enter, now);
        // Every project's entries, which is what these counts are of (#144).
        tui.press(KeyPress::Tab, now);
        tui.press(KeyPress::Char('g'), now);
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
            // The one screen with nowhere forward says so. Its keys are the
            // footer's, and said once: here they read as the same list twice.
            None => {
                assert_eq!(line, "the run is over");
                let bar = footer(screen, 200);
                for binding in bindings_for(screen) {
                    let key = binding.key.to_string();
                    assert!(
                        bar.contains(&key) && bar.contains(binding.label),
                        "{bar:?} leaves out {key:?} {}",
                        binding.label
                    );
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
            .nth(WAY_ROW as usize)
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
    let bar = footer(Screen::Projects, 240);
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
    assert!(projects.starts_with("Space mark project"), "{projects:?}");
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

/// One row of `buf`, trimmed.
fn row_text(buf: &Buffer, y: u16) -> String {
    (0..buf.area.width)
        .map(|x| buf[(x, y)].symbol())
        .collect::<String>()
        .trim()
        .to_string()
}

/// The row under the body: where a notice is drawn, and empty until there is
/// one.
fn notice_row(buf: &Buffer) -> String {
    row_text(buf, buf.area.height - 2)
}

/// A driver with the key list open, on a list with room to move.
fn with_keys_open(fx: &Fixture, store: &Fixture, now: Instant) -> Tui {
    let mut tui = primed(fx, store, Screen::Candidates);
    tui.press(KeyPress::Char('?'), now);
    tui
}

#[test]
fn closing_the_key_list_says_which_key_it_swallowed() {
    // The overlay closes on any key and nothing under it moves. Correct, and
    // silent: a user who presses `j` to move sees the list close and the
    // cursor stay, with nothing to say the key went nowhere.
    let fx = Fixture::new();
    let store = Fixture::new();
    many_projects(&fx, 60);
    let now = Instant::now();
    let mut tui = primed(&fx, &store, Screen::Candidates);
    // `primed` marks a row, and that has its own notice; let it go.
    tui.tick(Instant::now() + NOTICE_TTL);
    let before = frame(&mut tui);
    assert_eq!(
        notice_row(&before),
        "",
        "the row under the body is kept empty"
    );

    tui.press(KeyPress::Char('?'), now);
    tui.press(KeyPress::Char('j'), now);
    let after = frame(&mut tui);
    assert_eq!(notice_row(&after), "Keys closed. j was not applied.");

    let last = LIST_AREA.height - 1;
    assert_eq!(
        row_text(&after, last),
        row_text(&before, last),
        "the key bar keeps the last row"
    );
    for y in 0..last - 1 {
        assert_eq!(
            row_text(&after, y),
            row_text(&before, y),
            "row {y} changed: the swallowed key must not have moved anything"
        );
    }
}

#[test]
fn the_swallowed_key_is_named_the_way_the_key_bar_names_it() {
    let fx = Fixture::new();
    let store = Fixture::new();
    many_projects(&fx, 3);
    let now = Instant::now();
    for (key, name) in [
        (KeyPress::Down, "↓"),
        (KeyPress::PageDown, "PageDown"),
        (KeyPress::Space, "Space"),
    ] {
        let mut tui = with_keys_open(&fx, &store, now);
        tui.press(key, now);
        assert_eq!(
            notice_row(&frame(&mut tui)),
            format!("Keys closed. {name} was not applied.")
        );
    }
}

#[test]
fn a_notice_is_gone_once_its_time_is_up() {
    // Expiry is on the clock, not on a count of frames: a resize storm or a
    // slow terminal redraws at its own pace, and a notice has to last the
    // same on every machine.
    let fx = Fixture::new();
    let store = Fixture::new();
    many_projects(&fx, 3);
    let now = Instant::now();
    let mut tui = with_keys_open(&fx, &store, now);
    tui.press(KeyPress::Char('j'), now);

    tui.tick(now + NOTICE_TTL - Duration::from_millis(1));
    assert_eq!(
        notice_row(&frame(&mut tui)),
        "Keys closed. j was not applied."
    );

    tui.tick(now + NOTICE_TTL);
    assert_eq!(notice_row(&frame(&mut tui)), "");
}

#[test]
fn a_newer_notice_replaces_an_older_one_and_starts_the_clock_again() {
    let fx = Fixture::new();
    let store = Fixture::new();
    many_projects(&fx, 3);
    let now = Instant::now();
    let mut tui = with_keys_open(&fx, &store, now);
    tui.press(KeyPress::Char('j'), now);

    let later = now + Duration::from_secs(2);
    tui.press(KeyPress::Char('?'), later);
    tui.press(KeyPress::Char('k'), later);
    assert_eq!(
        notice_row(&frame(&mut tui)),
        "Keys closed. k was not applied."
    );

    // Past the first notice's time, within the second's.
    tui.tick(now + NOTICE_TTL + Duration::from_secs(1));
    assert_eq!(
        notice_row(&frame(&mut tui)),
        "Keys closed. k was not applied."
    );

    tui.tick(later + NOTICE_TTL);
    assert_eq!(notice_row(&frame(&mut tui)), "");
}

#[test]
fn the_notice_row_is_never_muted() {
    // A notice is a fact about the last key, and muted is for what can be
    // skipped.
    let fx = Fixture::new();
    let store = Fixture::new();
    many_projects(&fx, 3);
    let now = Instant::now();
    let mut tui = with_keys_open(&fx, &store, now);
    tui.press(KeyPress::Char('j'), now);
    let buf = frame(&mut tui);
    let y = buf.area.height - 2;
    assert!(!row_text(&buf, y).is_empty());
    for x in 0..buf.area.width {
        let cell = &buf[(x, y)];
        assert!(
            !cell.modifier.contains(Theme::ansi().muted.add_modifier),
            "cell {x} of the notice row is muted: {:?}",
            cell.symbol()
        );
    }
}

#[test]
fn a_notice_is_left_out_of_an_area_with_no_row_for_it() {
    let fx = Fixture::new();
    let store = Fixture::new();
    many_projects(&fx, 3);
    let now = Instant::now();
    let mut tui = with_keys_open(&fx, &store, now);
    tui.press(KeyPress::Char('j'), now);
    // Three rows: the title, the way, the key bar. No row is the notice's.
    let area = Rect::new(0, 0, 40, 3);
    let mut buf = Buffer::empty(area);
    tui.render(area, &mut buf);
    assert!(!text_of(&buf).contains("Keys closed"), "{}", text_of(&buf));

    let area = Rect::new(0, 0, 40, 4);
    let mut buf = Buffer::empty(area);
    tui.render(area, &mut buf);
    assert_eq!(notice_row(&buf), "Keys closed. j was not applied.");
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

/// The whole interface at `area`, as the loop would draw it.
fn frame_at(tui: &mut Tui, area: Rect) -> Buffer {
    let mut buf = Buffer::empty(area);
    tui.render(area, &mut buf);
    buf
}

/// Every word drawn, in reading order, joined by single spaces: what a
/// sentence reads as once the rows it was wrapped over are put back together.
fn prose(buf: &Buffer) -> String {
    text_of(buf)
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// The paragraph drawn in place of the body on a terminal below the minimum.
fn too_small_notice(screen: Screen, cols: u16, rows: u16) -> String {
    let mut text = format!(
        "dev-cleaner needs 80×24 and this terminal is {cols}×{rows}. \
         Resize it, or press q to quit."
    );
    if screen == Screen::Confirm {
        text.push_str(" The plan cannot be shown at this size; the hold is disabled until it can.");
    }
    text
}

#[test]
fn below_the_minimum_the_interface_says_what_it_needs_and_draws_no_body() {
    // At 40×8 the body was a heading, a row or two, and a key bar past the
    // edge, with nothing to say what was not being shown. The title still
    // names the screen; the body says what size it needs and how to leave.
    let fx = Fixture::new();
    let store = Fixture::new();
    node_project(&fx, "app", 4096);
    let mut tui = driver_on(&fx, &store, Screen::Candidates);

    let small = frame_at(&mut tui, Rect::new(0, 0, 40, 10));
    let shown = prose(&small);
    assert!(
        shown.contains(&too_small_notice(Screen::Candidates, 40, 10)),
        "the notice is not drawn whole:\n{}",
        text_of(&small)
    );
    assert!(
        text_of(&small).starts_with(" dev-cleaner  ·  candidates"),
        "the title row is not drawn as usual:\n{}",
        text_of(&small)
    );
    assert!(
        !shown.contains("Can be rebuilt"),
        "the body is drawn under the notice:\n{}",
        text_of(&small)
    );
    assert!(
        !shown.contains("hold"),
        "the candidates screen has no hold to disable:\n{}",
        text_of(&small)
    );

    let enough = prose(&frame_at(&mut tui, Rect::new(0, 0, 80, 24)));
    assert!(enough.contains("Can be rebuilt"), "{enough}");
    assert!(!enough.contains("this terminal is"), "{enough}");
}

#[test]
fn a_hold_on_a_terminal_too_small_to_show_the_plan_does_not_arm() {
    // The confirm screen exists to show what is about to be deleted. A hold
    // given while that is off-screen is a blind one, so the key is refused
    // and the paragraph in the body's place says so.
    let fx = Fixture::new();
    let store = Fixture::new();
    node_project(&fx, "app", 4096);
    let mut tui = driver_on(&fx, &store, Screen::Confirm);

    let small = frame_at(&mut tui, Rect::new(0, 0, 40, 10));
    assert!(
        prose(&small).contains(&too_small_notice(Screen::Confirm, 40, 10)),
        "{}",
        text_of(&small)
    );
    let start = Instant::now();
    assert_eq!(
        hold_like_macos(&mut tui, start, Duration::from_secs(5)),
        None,
        "a hold on a frame that could not show the plan armed"
    );
    assert_eq!(tui.app().screen(), Screen::Confirm);

    // Grown back to the minimum, the same hold purges.
    frame_at(&mut tui, Rect::new(0, 0, 80, 24));
    let again = start + Duration::from_secs(10);
    assert!(
        hold_like_macos(&mut tui, again, Duration::from_secs(5)).is_some(),
        "the hold does not come back with the room to show the plan"
    );
}

#[test]
fn q_quits_at_any_size() {
    let fx = Fixture::new();
    let store = Fixture::new();
    node_project(&fx, "app", 4096);

    for screen in [Screen::Dashboard, Screen::Candidates, Screen::Confirm] {
        for (cols, rows) in [(40, 10), (80, 24), (200, 50)] {
            let mut tui = driver_on(&fx, &store, screen);
            frame_at(&mut tui, Rect::new(0, 0, cols, rows));
            assert_eq!(
                tui.press(KeyPress::Char('q'), Instant::now()),
                Step::Quit,
                "{screen:?} at {cols}×{rows} does not quit on q"
            );
        }
    }
}

/// The sizes the sweep draws at (#103): a split pane, the minimum, an ordinary
/// window and a wide one, by a short, a standard and a tall terminal.
const SWEEP_COLS: [u16; 4] = [60, 80, 100, 200];
const SWEEP_ROWS: [u16; 3] = [10, 24, 50];

/// Room past `area`'s right and bottom edges. Drawn into a buffer this much
/// larger, a string that runs past the edge lands where the sweep can see it,
/// where the terminal would have clipped it silently.
const MARGIN: u16 = 100;

/// `screen`, drawn at any area into a buffer with [`MARGIN`] past its edges.
///
/// One state, drawn as often as asked: the free space on the disk moves
/// between two walks, and the sweep compares one frame against another.
/// The result screen is reached only through a purge, which writes a record
/// under $HOME; its body is drawn directly, as `drawn` does.
fn swept(fx: &Fixture, store: &Fixture, screen: Screen) -> impl FnMut(Rect) -> Buffer {
    let mut tui = (screen != Screen::Result).then(|| primed(fx, store, screen));
    let manifest = execute(
        confirmed(vec![candidate("/p/a/node_modules", 1024)]),
        &Recorder::default(),
    );
    move |area: Rect| {
        let mut buf = Buffer::empty(Rect::new(0, 0, area.width + MARGIN, area.height + MARGIN));
        match tui.as_mut() {
            Some(tui) => tui.render(area, &mut buf),
            None => {
                let body = Rect::new(
                    area.x,
                    area.y + 2,
                    area.width,
                    area.height.saturating_sub(3),
                );
                Report::new().render(&Theme::ansi(), &manifest, None, body, &mut buf);
            }
        }
        buf
    }
}

/// The words drawn inside `area`, row by row.
fn words_within(buf: &Buffer, area: Rect) -> Vec<String> {
    (area.top()..area.bottom())
        .flat_map(|y| {
            let row: String = (area.left()..area.right())
                .map(|x| buf[(x, y)].symbol())
                .collect();
            row.split_whitespace()
                .map(str::to_string)
                .collect::<Vec<_>>()
        })
        .collect()
}

/// The words on the rows of `area` that begin with `start`.
fn words_within_rows_starting(buf: &Buffer, area: Rect, start: &str) -> Vec<String> {
    (area.top()..area.bottom())
        .filter_map(|y| {
            let row: String = (area.left()..area.right())
                .map(|x| buf[(x, y)].symbol())
                .collect();
            row.trim_start().starts_with(start).then(|| {
                row.split_whitespace()
                    .map(str::to_string)
                    .collect::<Vec<_>>()
            })
        })
        .flatten()
        .collect()
}

/// Draw `screen` at every size in the sweep and collect what went wrong: a
/// cell written past the area, or a word that is neither drawn whole nor
/// marked as cut.
///
/// "Whole" is judged against the same state drawn 400 columns wide at the
/// same height, where nothing has to be cut: a word that appears there is
/// whole here. A word carrying `…` was cut and says so; a run of glyphs with
/// no letter or digit in it is a gauge, whose length is its meaning.
fn sweep(screen: Screen) {
    let fx = Fixture::new();
    let store = Fixture::new();
    many_projects(&fx, 60);
    busy_fixture(&fx);

    let mut wrong = Vec::new();
    for cols in SWEEP_COLS {
        for rows in SWEEP_ROWS {
            let area = Rect::new(0, 0, cols, rows);
            let mut draw = swept(&fx, &store, screen);
            let buf = draw(area);
            let outside = buf.area;
            for y in outside.top()..outside.bottom() {
                for x in outside.left()..outside.right() {
                    if !area.contains((x, y).into()) && buf[(x, y)] != ratatui::buffer::Cell::EMPTY
                    {
                        wrong.push(format!(
                            "{screen:?} at {cols}×{rows}: {:?} drawn at ({x}, {y}), past the area",
                            buf[(x, y)].symbol()
                        ));
                    }
                }
            }

            // The reference has the same body as the frame under test. From
            // 34 rows the header is as tall as the logo, but only from 100
            // columns, so a frame narrower than that is compared with one
            // that much taller than itself.
            let wide = Rect::new(
                0,
                0,
                400,
                if cols < logo::MIN_COLS && rows >= logo::MIN_ROWS {
                    rows + logo::TOP - 2
                } else {
                    rows
                },
            );
            let mut known = words_within(&draw(wide), wide);
            known.extend(
                too_small_notice(screen, cols, rows)
                    .split_whitespace()
                    .map(str::to_string),
            );
            // Chrome that exists only at narrow widths: the projects table names
            // the columns it hid, and the 400-column reference hides nothing, so
            // those words are added here the way the too-small notice is. The
            // column names themselves are headers the reference does draw. The
            // view bar likewise has a short wording the wide reference never
            // needs (`removable`, for `have something to remove`). The too-small
            // screen heads its paragraph, and the 400-column reference is not small.
            known.extend(
                [
                    "hidden",
                    "at",
                    "this",
                    "width",
                    "and",
                    "Too",
                    "small",
                    "removable",
                ]
                .iter()
                .map(|w| w.to_string()),
            );
            // The result screen says where its scrolling part is only when that
            // part does not fit, which a 400-column reference may well do.
            let position = words_within_rows_starting(&buf, area, "showing ");
            for word in words_within(&buf, area) {
                let marked = word.contains('…') || position.contains(&word);
                let gauge = word.chars().all(|c| !c.is_alphanumeric());
                if !(marked || gauge || known.contains(&word)) {
                    wrong.push(format!(
                        "{screen:?} at {cols}×{rows}: {word:?} is cut with no mark"
                    ));
                }
            }
        }
    }
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
}

#[test]
fn the_dashboard_survives_the_sweep() {
    sweep(Screen::Dashboard);
}

#[test]
fn the_projects_table_survives_the_sweep() {
    sweep(Screen::Projects);
}

#[test]
fn the_candidates_screen_survives_the_sweep() {
    sweep(Screen::Candidates);
}

#[test]
fn the_review_screen_survives_the_sweep() {
    sweep(Screen::Review);
}

#[test]
fn the_confirm_screen_survives_the_sweep() {
    sweep(Screen::Confirm);
}

#[test]
fn the_result_screen_survives_the_sweep() {
    sweep(Screen::Result);
}

#[test]
fn the_interface_draws_into_any_area_without_panicking() {
    // A pane being dragged passes through one row and no rows on the way to
    // its size. The loop redraws on every resize, so each of those is a frame.
    let fx = Fixture::new();
    let store = Fixture::new();
    node_project(&fx, "app", 4096);

    for screen in [Screen::Dashboard, Screen::Candidates, Screen::Confirm] {
        for (cols, rows) in [(0, 0), (1, 1), (80, 1), (80, 2), (1, 24), (20, 5), (40, 10)] {
            let mut tui = driver_on(&fx, &store, screen);
            let area = Rect::new(0, 0, cols, rows);
            let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                frame_at(&mut tui, area);
            }));
            assert!(
                outcome.is_ok(),
                "{screen:?} panics drawing into {cols}×{rows}"
            );
        }
    }
}

#[test]
fn a_page_on_the_projects_table_is_what_the_last_frame_showed() {
    // The table pages by what the last frame drew, so one PageDown from the top selects the first row that
    // was out of view — not one still inside the window, not one past it.
    let fx = Fixture::new();
    let store = Fixture::new();
    many_projects(&fx, 60);
    let now = Instant::now();
    let mut tui = driver_on(&fx, &store, Screen::Projects);
    let shown = text_of(&frame(&mut tui));
    // Read from the frame rather than fixed: the rows a body has is a fact
    // about the chrome around it, and the chrome has changed once already.
    let rows: usize = shown
        .lines()
        .find_map(|l| l.trim().strip_prefix("showing 1-"))
        .and_then(|rest| rest.split(' ').next())
        .and_then(|n| n.parse().ok())
        .expect("the table says where it is");
    assert!(
        rows > 1 && rows < 60,
        "a window smaller than the table: {shown}"
    );

    tui.press(KeyPress::PageDown, now);
    let shown = text_of(&frame(&mut tui));
    let expected = format!("showing 2-{} of 60", rows + 1);
    assert!(
        shown.contains(&expected),
        "a page down should land on row {}, the first row that was out of view:\n{shown}",
        rows + 1
    );
}

/// The count and total the wayfinding row of `buf` carries for the marks.
fn wayfinding_marks(buf: &Buffer) -> (String, String) {
    let row = row_text(buf, WAY_ROW);
    let rest = row
        .split("built from the ")
        .nth(1)
        .unwrap_or_else(|| panic!("the wayfinding row names the marks: {row:?}"));
    let count = rest.split(' ').next().unwrap().to_string();
    let total = rest
        .split('(')
        .nth(1)
        .and_then(|s| s.split(')').next())
        .unwrap()
        .to_string();
    (count, total)
}

#[test]
fn marking_and_clearing_say_what_they_changed_with_the_wayfinding_numbers() {
    let fx = Fixture::new();
    let store = Fixture::new();
    node_project(&fx, "a", 4096);
    node_project(&fx, "b", 65536);
    let now = Instant::now();
    let mut tui = driver_on(&fx, &store, Screen::Candidates);
    let listed = screens(&fx, &store);
    let first = &listed.candidates.selectable()[0];
    let total = |bytes: u64| human(bytes);
    let all: u64 = candidates_total(&fx, &store);

    // Empty of marks, `c` has nothing to clear and says so.
    tui.press(KeyPress::Char('c'), now);
    assert_eq!(notice_row(&frame(&mut tui)), "Nothing marked.");

    tui.press(KeyPress::Space, now);
    let buf = frame(&mut tui);
    let (count, sum) = wayfinding_marks(&buf);
    assert_eq!(
        notice_row(&buf),
        format!(
            "Marked +  b/node_modules  ({}).  {count} marked, {sum}.",
            total(first.bytes)
        )
    );

    tui.press(KeyPress::Space, now);
    let buf = frame(&mut tui);
    assert_eq!(
        notice_row(&buf),
        format!(
            "Unmarked  b/node_modules  ({}).  0 marked, {}.",
            total(first.bytes),
            human(0)
        )
    );

    tui.press(KeyPress::Char('a'), now);
    let buf = frame(&mut tui);
    let (count, sum) = wayfinding_marks(&buf);
    assert_eq!(
        notice_row(&buf),
        format!("Marked all {count}  ({}).", total(all))
    );
    assert_eq!(sum, total(all), "the two lines read one total");

    tui.press(KeyPress::Char('c'), now);
    assert_eq!(
        notice_row(&frame(&mut tui)),
        format!(
            "Cleared {count} marks  ({}).  c again restores them.",
            total(all)
        )
    );
}

#[test]
fn c_again_restores_what_c_cleared_and_the_plan_is_the_restored_set() {
    let fx = Fixture::new();
    let store = Fixture::new();
    node_project(&fx, "a", 4096);
    node_project(&fx, "b", 65536);
    let now = Instant::now();
    let mut tui = driver_on(&fx, &store, Screen::Candidates);
    let all = candidates_total(&fx, &store);

    tui.press(KeyPress::Char('a'), now);
    tui.press(KeyPress::Char('c'), now);
    tui.press(KeyPress::Char('c'), now);
    assert_eq!(
        notice_row(&frame(&mut tui)),
        format!("Restored 2 marks  ({}).", human(all))
    );

    tui.press(KeyPress::Enter, now);
    assert_eq!(tui.app().screen(), Screen::Review);
    let plan = tui.app().reviewing().expect("reviewed");
    assert_eq!(plan.items().len(), 2);
    assert_eq!(plan.total_bytes(), all);
}

#[test]
fn a_mark_made_after_clearing_means_c_clears_that_one_and_nothing_comes_back() {
    let fx = Fixture::new();
    let store = Fixture::new();
    node_project(&fx, "a", 4096);
    node_project(&fx, "b", 65536);
    let now = Instant::now();
    let mut tui = driver_on(&fx, &store, Screen::Candidates);

    tui.press(KeyPress::Char('a'), now);
    tui.press(KeyPress::Char('c'), now);
    tui.press(KeyPress::Space, now);
    tui.press(KeyPress::Char('c'), now);
    assert!(
        notice_row(&frame(&mut tui)).starts_with("Cleared 1 mark  ("),
        "{}",
        notice_row(&frame(&mut tui))
    );
    // What comes back now is the one just cleared, not the two before it.
    tui.press(KeyPress::Char('c'), now);
    assert!(
        notice_row(&frame(&mut tui)).starts_with("Restored 1 mark  ("),
        "{}",
        notice_row(&frame(&mut tui))
    );
}

#[test]
fn leaving_the_candidates_drops_what_c_would_have_restored() {
    let fx = Fixture::new();
    let store = Fixture::new();
    node_project(&fx, "a", 4096);
    let now = Instant::now();
    let mut tui = driver_on(&fx, &store, Screen::Candidates);

    tui.press(KeyPress::Char('a'), now);
    tui.press(KeyPress::Char('c'), now);
    tui.press(KeyPress::Esc, now);
    assert_ne!(tui.app().screen(), Screen::Candidates);
    tui.press(KeyPress::Enter, now);
    assert_eq!(tui.app().screen(), Screen::Candidates);

    tui.press(KeyPress::Char('c'), now);
    assert_eq!(notice_row(&frame(&mut tui)), "Nothing marked.");
}

#[test]
fn marking_with_nothing_to_mark_says_so() {
    let fx = Fixture::new();
    let store = Fixture::new();
    let now = Instant::now();
    let mut tui = driver_on(&fx, &store, Screen::Candidates);
    tui.press(KeyPress::Space, now);
    assert_eq!(notice_row(&frame(&mut tui)), "Nothing to mark.");
    tui.press(KeyPress::Char('a'), now);
    assert_eq!(notice_row(&frame(&mut tui)), "Nothing to mark.");
}

#[test]
fn sorting_the_projects_says_how_and_where_the_cursor_is() {
    let fx = Fixture::new();
    let store = Fixture::new();
    node_project(&fx, "alpha", 65536);
    node_project(&fx, "bravo", 262144);
    node_project(&fx, "charlie", 131072);
    let now = Instant::now();
    let mut tui = driver_on(&fx, &store, Screen::Projects);
    // Largest first: bravo, charlie, alpha. One down is charlie.
    tui.press(KeyPress::Down, now);

    for (key, notice) in [
        ('1', "Sorted by name, A to Z.  Cursor on charlie."),
        ('1', "Sorted by name, Z to A.  Cursor on charlie."),
        (
            '5',
            "Sorted by reclaimable, largest first.  Cursor on charlie.",
        ),
        (
            '5',
            "Sorted by reclaimable, smallest first.  Cursor on charlie.",
        ),
        (
            '6',
            "Sorted by activity, most active first.  Cursor on charlie.",
        ),
    ] {
        tui.press(KeyPress::Char(key), now);
        assert_eq!(notice_row(&frame(&mut tui)), notice);
    }
}

#[test]
fn a_page_on_the_candidates_screen_is_what_the_last_frame_showed() {
    // The driver hands the screen the rows its last frame gave the body, so one
    // PageDown from the top selects the first entry that was out of view.
    let fx = Fixture::new();
    let store = Fixture::new();
    many_projects(&fx, 60);
    let now = Instant::now();
    let mut tui = driver_on(&fx, &store, Screen::Candidates);
    let shown = text_of(&frame(&mut tui));
    let window: usize = shown
        .lines()
        .find_map(|l| l.split("showing 1-").nth(1))
        .and_then(|rest| rest.split(' ').next())
        .and_then(|n| n.parse().ok())
        .expect("the list says where it is");
    assert!(window > 1, "a window of more than one row: {shown}");

    tui.press(KeyPress::PageDown, now);
    let shown = text_of(&frame(&mut tui));
    let expected = format!("showing 2-{} of 60", window + 1);
    assert!(
        shown.contains(&expected),
        "a page down should land on entry {}, the first that was out of view:\n{shown}",
        window + 1
    );
}

/// The keys on `screen` that the table gives no binding: what a hand can hit
/// that the screen does not answer to.
fn unbound_keys(screen: Screen) -> Vec<KeyPress> {
    let bound: Vec<KeyPress> = bindings_for(screen).iter().map(|b| b.key).collect();
    every_key()
        .into_iter()
        .filter(|key| !bound.contains(key))
        .collect()
}

#[test]
fn every_unbound_key_on_every_screen_says_so() {
    // A key that does nothing looks like a keyboard that stopped working. Each
    // one gets a notice that names it, on every screen, so a silent key is a
    // failure that names the key and the screen.
    let fx = Fixture::new();
    let store = Fixture::new();
    node_project(&fx, "app", 4096);
    let now = Instant::now();

    for screen in Screen::all() {
        if screen == Screen::Result {
            // Reached only through a real purge, which writes a record under
            // $HOME; the unit test in `run.rs` covers its notice.
            continue;
        }
        for key in unbound_keys(screen) {
            let mut tui = driver_on(&fx, &store, screen);
            frame(&mut tui);
            assert_eq!(tui.press(key, now), Step::Stay);
            let notice = notice_row(&frame(&mut tui));
            assert!(
                notice.starts_with(&format!("{key} does nothing here. ")),
                "{key} on {screen:?} did not say so; the notice row reads {notice:?}"
            );
        }
    }
}

#[test]
fn the_notice_for_an_unbound_key_names_only_keys_the_screen_binds() {
    // Built from the table, so it cannot advise a key that does nothing. The
    // walk reads the keys back out of the drawn row rather than trusting the
    // builder: each entry after the sentence starts with the keys it is for.
    let fx = Fixture::new();
    let store = Fixture::new();
    node_project(&fx, "app", 4096);
    let now = Instant::now();

    for screen in Screen::all() {
        if screen == Screen::Result {
            continue;
        }
        let bound: Vec<String> = bindings_for(screen)
            .iter()
            .map(|b| b.key.to_string())
            .collect();
        for key in unbound_keys(screen) {
            let mut tui = driver_on(&fx, &store, screen);
            frame(&mut tui);
            tui.press(key, now);
            let notice = notice_row(&frame(&mut tui));
            let advice = notice
                .strip_prefix(&format!("{key} does nothing here. "))
                .unwrap_or_else(|| panic!("{key} on {screen:?}: {notice:?}"));
            let entries: Vec<&str> = advice.trim_end_matches('.').split(" · ").collect();
            assert_eq!(
                entries.len(),
                2,
                "{key} on {screen:?} should name two entries: {notice:?}"
            );
            for entry in entries {
                let keys = entry.split(' ').next().unwrap_or_default();
                for named in keys.split('/') {
                    assert!(
                        bound.iter().any(|b| b == named),
                        "{key} on {screen:?} names {named:?}, which the screen does not bind: {notice:?}"
                    );
                }
            }
        }
    }
}

#[test]
fn a_stray_key_during_a_hold_does_not_touch_the_hold() {
    // The hold belongs to `held_at` and the tick. A key that is not bound sets
    // a notice and nothing else, so the gauge fills on the same schedule with
    // or without it.
    let fx = Fixture::new();
    let store = Fixture::new();
    node_project(&fx, "app", 4096);

    let mut undisturbed = driver_on(&fx, &store, Screen::Confirm);
    let expected = hold_like_macos(&mut undisturbed, Instant::now(), Duration::from_secs(5))
        .expect("an undisturbed hold must purge");

    let mut tui = driver_on(&fx, &store, Screen::Confirm);
    let start = Instant::now();
    let mut at = Duration::ZERO;
    let mut stray = false;
    let purged = loop {
        assert!(at <= Duration::from_secs(5), "the hold never completed");
        tui.tick(start + at);
        if !stray && at >= Duration::from_millis(1200) {
            stray = true;
            assert_eq!(tui.press(KeyPress::Enter, start + at), Step::Stay);
        }
        if tui.press(PURGE, start + at) == Step::Purge {
            break at;
        }
        at += if at.is_zero() {
            Duration::from_millis(375)
        } else {
            Duration::from_millis(90)
        };
    };

    assert_eq!(purged, expected, "a stray Enter moved the hold");
}

/// The candidates screen with everything marked, over two projects.
fn marked_driver(fx: &Fixture, store: &Fixture) -> Tui {
    node_project(fx, "a", 4096);
    node_project(fx, "b", 65536);
    let mut tui = driver_on(fx, store, Screen::Candidates);
    tui.press(KeyPress::Char('a'), Instant::now());
    tui
}

const Q: KeyPress = KeyPress::Char('q');

#[test]
fn q_with_nothing_marked_quits_on_the_first_press() {
    let fx = Fixture::new();
    let store = Fixture::new();
    node_project(&fx, "app", 4096);

    for screen in [Screen::Dashboard, Screen::Projects, Screen::Candidates] {
        let mut tui = driver_on(&fx, &store, screen);
        assert_eq!(tui.press(Q, Instant::now()), Step::Quit, "{screen:?}");
    }
}

#[test]
fn q_with_marks_asks_once_with_the_wayfinding_numbers_and_the_second_q_quits() {
    let fx = Fixture::new();
    let store = Fixture::new();
    let now = Instant::now();
    let mut tui = marked_driver(&fx, &store);
    let (count, sum) = wayfinding_marks(&frame(&mut tui));

    assert_eq!(tui.press(Q, now), Step::Stay, "the first q only asks");
    let buf = frame(&mut tui);
    assert_eq!(
        notice_row(&buf),
        format!("{count} marked ({sum}) would be dropped. q again within 3 s quits.")
    );
    assert_eq!(
        wayfinding_marks(&buf),
        (count, sum),
        "the notice and the row read one frame"
    );

    assert_eq!(
        tui.press(Q, now + NOTICE_TTL - Duration::from_millis(1)),
        Step::Quit
    );
}

#[test]
fn q_on_review_and_confirm_names_the_plan_and_asks_the_same_way() {
    let fx = Fixture::new();
    let store = Fixture::new();
    let now = Instant::now();
    let mut tui = marked_driver(&fx, &store);
    tui.press(KeyPress::Enter, now);
    assert_eq!(tui.app().screen(), Screen::Review);
    let (count, sum) = wayfinding_marks(&frame(&mut tui));

    for screen in [Screen::Review, Screen::Confirm] {
        while tui.app().screen() != screen {
            tui.press(KeyPress::Enter, now);
        }
        assert_eq!(tui.press(Q, now), Step::Stay, "{screen:?}");
        assert_eq!(
            notice_row(&frame(&mut tui)),
            format!(
                "The plan of {count} items ({sum}) would be dropped. q again within 3 s quits."
            ),
            "{screen:?}"
        );
        assert_eq!(tui.press(Q, now), Step::Quit, "{screen:?}");
    }
}

/// The project the table's cursor is on, read off the notice a forward move
/// leaves: it opens with the project's name and a colon.
fn noticed_project(tui: &mut Tui) -> String {
    let notice = notice_row(&frame(tui));
    notice
        .split_once(':')
        .map(|(name, _)| name.to_string())
        .unwrap_or_else(|| panic!("the notice does not name a project: {notice:?}"))
}

#[test]
fn enter_on_a_project_lands_the_candidates_cursor_on_its_first_entry() {
    // Two projects, the one the table opens on holding the smaller directory,
    // so the candidates screen's own first row (largest first) is never the
    // focused project's by luck.
    let fx = Fixture::new();
    let store = Fixture::new();
    node_project(&fx, "big", 64 * 1024);
    node_project(&fx, "small", 1024);
    let now = Instant::now();
    let mut tui = driver_on(&fx, &store, Screen::Projects);

    for row in 0..2 {
        tui.press(KeyPress::Enter, now);
        assert_eq!(tui.app().screen(), Screen::Candidates);
        let project = noticed_project(&mut tui);
        assert!(
            notice_row(&frame(&mut tui)).contains("1 directory can be rebuilt"),
            "row {row}: {}",
            notice_row(&frame(&mut tui))
        );

        // Marking acts on the cursor's entry, which is how the cursor is read.
        tui.press(KeyPress::Space, now);
        let marked = notice_row(&frame(&mut tui));
        assert!(
            marked.contains(&format!("{project}/node_modules")),
            "row {row}: the cursor is not on {project}'s entry: {marked}"
        );

        // Back to the table: its cursor is where it was, so Enter again is the
        // same project, and the table moves on only when told to.
        tui.press(KeyPress::Esc, now);
        assert_eq!(tui.app().screen(), Screen::Projects);
        tui.press(KeyPress::Enter, now);
        assert_eq!(noticed_project(&mut tui), project, "row {row}: Esc, Enter");
        tui.press(KeyPress::Esc, now);
        tui.press(KeyPress::Down, now);
    }
}

#[test]
fn another_key_between_the_two_q_disarms() {
    let fx = Fixture::new();
    let store = Fixture::new();
    let now = Instant::now();
    let mut tui = marked_driver(&fx, &store);

    assert_eq!(tui.press(Q, now), Step::Stay);
    tui.press(KeyPress::Down, now);
    assert_eq!(
        tui.press(Q, now),
        Step::Stay,
        "the key between them took the first q back"
    );
    assert_eq!(tui.press(Q, now), Step::Quit);
}

#[test]
fn after_the_ttl_the_next_q_arms_again_instead_of_quitting() {
    let fx = Fixture::new();
    let store = Fixture::new();
    let now = Instant::now();
    let mut tui = marked_driver(&fx, &store);

    assert_eq!(tui.press(Q, now), Step::Stay);
    // The tick lets the arming go with its notice ...
    tui.tick(now + NOTICE_TTL);
    assert_eq!(tui.press(Q, now + NOTICE_TTL), Step::Stay);
    // ... and a press the tick never saw is late all the same.
    let mut tui = marked_driver(&fx, &store);
    assert_eq!(tui.press(Q, now), Step::Stay);
    assert_eq!(tui.press(Q, now + NOTICE_TTL), Step::Stay);
    assert_eq!(tui.press(Q, now + NOTICE_TTL), Step::Quit);
}

#[test]
fn enter_on_a_project_with_nothing_offerable_says_why_and_moves_nothing() {
    let fx = Fixture::new();
    let store = Fixture::new();
    node_project(&fx, "clean", 1024);
    fx.git_repo("held", 10);
    fx.file("held/package.json", b"{}");
    fx.file("held/node_modules/react/index.js", b"x");
    let now = Instant::now();
    let mut tui = driver_on(&fx, &store, Screen::Projects);

    let mut held = None;
    for _ in 0..2 {
        tui.press(KeyPress::Enter, now);
        let notice = notice_row(&frame(&mut tui));
        if notice.starts_with("held:") {
            held = Some(notice);
            break;
        }
        tui.press(KeyPress::Esc, now);
        tui.press(KeyPress::Down, now);
    }

    let notice = held.expect("the held project's notice");
    assert!(notice.contains("nothing can be rebuilt here"), "{notice}");
    assert!(notice.contains("1 held back"), "{notice}");
    // The notice leads with where something is (#149), so a narrow row cuts the
    // reason; the screen under it still says it in full.
    let shown = text_of(&frame(&mut tui));
    assert!(shown.contains("Untracked source files"), "{shown}");
}

// ---- The theme (#133) ----

/// Text roles are measured against the ground; the roles drawn as a fill carry
/// their own ground, and are measured against that. Muted is held to the same
/// 4.5:1: it is quieter, never unreadable.
#[test]
fn every_role_of_the_neon_theme_reads_on_the_ground_it_is_drawn_on() {
    let theme = Theme::neon();
    let ground = palette::GROUND_RGB;
    let on_ground = [
        ("text", theme.text),
        ("muted", theme.muted),
        ("head", theme.head),
        ("accent", theme.accent),
        ("safe", theme.safe),
        ("blocked", theme.blocked),
        ("danger", theme.danger),
        ("violet", theme.violet),
        ("verdict_safe", theme.verdict_safe),
        ("verdict_blocked", theme.verdict_blocked),
        ("size small", theme.size(0)),
        ("size warm", theme.size(palette::SIZE_WARM)),
        ("size hot", theme.size(palette::SIZE_HOT)),
    ];
    for (name, style) in on_ground {
        let ratio = contrast(rgb(style.fg), ground);
        assert!(ratio >= 4.5, "{name} is {ratio:.2}:1 on the ground");
    }
    for (name, style) in [
        ("key", theme.key),
        ("selected", theme.selected),
        ("warning_band", theme.warning_band),
    ] {
        let ratio = contrast(rgb(style.fg), rgb(style.bg));
        assert!(ratio >= 4.5, "{name} is {ratio:.2}:1 on its own fill");
    }
    assert_eq!(
        rgb(theme.ground.bg),
        ground,
        "the ground is the one measured"
    );
}

#[test]
fn every_cell_of_every_screen_in_neon_has_its_own_colours_and_reads() {
    let fx = Fixture::new();
    let store = Fixture::new();
    busy_fixture(&fx);

    for screen in Screen::all() {
        let buf = drawn_in(&fx, &store, screen, Theme::neon());
        common::contrast::assert_readable(&buf, &format!("{screen:?}"));
    }
}

#[test]
fn neon_reads_at_the_smallest_sizes_and_with_the_key_list_open() {
    // Below 80x24 the body is a paragraph, in a column the width of one the
    // chrome still has to be readable, and the key list is an overlay: the
    // three places a colour run could be cut or left on a ground of its own.
    let fx = Fixture::new();
    let store = Fixture::new();
    busy_fixture(&fx);
    let now = Instant::now();
    for screen in Screen::all() {
        if screen == Screen::Result {
            continue;
        }
        for (w, h) in [(120, 40), (79, 23), (40, 10), (20, 6), (1, 4)] {
            let mut tui = driver_in(&fx, &store, screen, Theme::neon());
            let area = Rect::new(0, 0, w, h);
            let mut buf = Buffer::empty(area);
            tui.render(area, &mut buf);
            common::contrast::assert_readable(&buf, &format!("{screen:?} at {w}x{h}"));
            tui.press(KeyPress::Char('?'), now);
            let mut buf = Buffer::empty(area);
            tui.render(area, &mut buf);
            common::contrast::assert_readable(&buf, &format!("{screen:?} keys at {w}x{h}"));
        }
    }
}

#[test]
fn under_no_colour_nothing_on_any_screen_sets_a_colour() {
    let fx = Fixture::new();
    let store = Fixture::new();
    busy_fixture(&fx);

    for screen in Screen::all() {
        let buf = drawn_in(&fx, &store, screen, Theme::mono());
        for cell in &buf.content {
            assert_eq!(cell.fg, Color::Reset, "{screen:?} sets a foreground");
            assert_eq!(cell.bg, Color::Reset, "{screen:?} sets a background");
        }
    }
}

#[test]
fn on_a_profile_with_its_own_colours_the_ground_stays_the_profiles() {
    let fx = Fixture::new();
    let store = Fixture::new();
    busy_fixture(&fx);

    for screen in Screen::all() {
        let buf = drawn_in(&fx, &store, screen, Theme::ansi());
        // The logo is art, not text: a cell of it holds two inks, and the
        // second is its background. Everything else is the profile's.
        let own_ground = |(i, c): (usize, &ratatui::buffer::Cell)| {
            let y = i / buf.area.width as usize;
            c.bg == Color::Reset
                || (y < logo::HEIGHT as usize
                    && (c
                        .symbol()
                        .chars()
                        .all(|c| ('\u{2801}'..='\u{28ff}').contains(&c))
                        || "▀▄█".contains(c.symbol())))
        };
        assert!(
            buf.content.iter().enumerate().all(own_ground),
            "{screen:?} paints a background the profile did not choose"
        );
    }
}

#[test]
fn the_environment_picks_the_look_and_no_color_wins() {
    use palette::Mode::{Ansi, Mono, Truecolor};
    let mode = |no_color, colorterm| Theme::choose(no_color, colorterm).mode();
    assert_eq!(mode(None, Some("truecolor")), Truecolor);
    assert_eq!(mode(None, Some("24bit")), Truecolor);
    assert_eq!(mode(None, Some("256color")), Ansi);
    assert_eq!(mode(None, None), Ansi);
    assert_eq!(mode(Some("1"), Some("truecolor")), Mono);
    assert_eq!(mode(Some(""), Some("truecolor")), Truecolor);
}

#[test]
fn the_scan_line_is_coloured_only_where_there_is_colour() {
    assert_eq!(Theme::mono().progress_line("scanning"), "scanning");
    assert!(
        Theme::ansi()
            .progress_line("scanning")
            .starts_with("\x1b[36m")
    );
    assert!(
        Theme::neon()
            .progress_line("scanning")
            .starts_with("\x1b[38;2;")
    );
    assert!(Theme::neon().progress_line("scanning").ends_with("\x1b[0m"));
}

#[test]
fn a_size_is_louder_the_bigger_it_is() {
    let theme = Theme::neon();
    let (small, warm, hot) = (
        theme.size(0),
        theme.size(palette::SIZE_WARM),
        theme.size(palette::SIZE_HOT),
    );
    assert_eq!(theme.size(palette::SIZE_WARM - 1), small);
    assert_eq!(theme.size(palette::SIZE_HOT - 1), warm);
    assert!(small != warm && warm != hot && small != hot);
    for look in [Theme::ansi(), Theme::mono()] {
        assert_ne!(
            look.size(0),
            look.size(palette::SIZE_HOT),
            "{:?}",
            look.mode()
        );
    }
}

/// The cells of row `y` that draw `word`, found by its text.
fn cells_of<'a>(buf: &'a Buffer, y: u16, word: &str) -> Vec<&'a ratatui::buffer::Cell> {
    let line: String = (0..buf.area.width).map(|x| buf[(x, y)].symbol()).collect();
    let start = line
        .find(word)
        .unwrap_or_else(|| panic!("{word:?} not in {line:?}"));
    let x = line[..start].chars().count() as u16;
    (x..x + word.chars().count() as u16)
        .map(|x| &buf[(x, y)])
        .collect()
}

#[test]
fn the_key_bar_draws_each_key_as_a_cap_and_its_label_muted() {
    let fx = Fixture::new();
    let store = Fixture::new();
    busy_fixture(&fx);
    let theme = Theme::neon();
    let buf = drawn_in(&fx, &store, Screen::Candidates, theme);
    let y = buf.area.height - 1;
    let cap_bg = theme.key.bg.expect("a cap has a fill");

    let entry = cells_of(&buf, y, "q quit");
    assert_eq!(entry[0].bg, cap_bg, "the key is not drawn as a cap");
    assert!(
        entry[2..]
            .iter()
            .all(|c| Some(c.fg) == theme.muted.fg && c.bg != cap_bg),
        "the label is not muted"
    );
}

#[test]
fn the_result_screen_keeps_its_keys_in_the_footer_and_its_state_in_the_way_row() {
    // Said twice in the same weight, the key list read as noise (#133).
    assert_eq!(wayfinding(Screen::Result, (2, 2048)), "the run is over");
    assert!(footer(Screen::Result, 100).contains("quit"));
}

#[test]
fn the_cursor_row_is_one_unmistakable_band_in_every_colour_mode() {
    let fx = Fixture::new();
    let store = Fixture::new();
    many_projects(&fx, 3);

    for theme in [Theme::neon(), Theme::ansi(), Theme::mono()] {
        for screen in [Screen::Projects, Screen::Candidates] {
            let mut tui = driver_in(&fx, &store, screen, theme);
            let buf = frame(&mut tui);
            let band = |x, y| {
                let cell = &buf[(x, y)];
                cell.modifier.contains(Modifier::REVERSED)
                    || theme.selected.bg.is_some_and(|bg| cell.bg == bg)
            };
            let mode = theme.mode();
            // The icon's cyan is a background too, and it is not a row.
            let rows: Vec<u16> = (logo::TOP..LIST_AREA.height)
                .filter(|&y| (0..LIST_AREA.width).any(|x| band(x, y)))
                .collect();
            assert_eq!(rows.len(), 1, "{mode:?} {screen:?}: one row is selected");
            assert!(
                (0..LIST_AREA.width).all(|x| band(x, rows[0])),
                "{mode:?} {screen:?}: the band has gaps"
            );
        }
    }
}

#[test]
fn without_colour_blocked_safe_and_danger_are_still_told_apart() {
    let fx = Fixture::new();
    let store = Fixture::new();
    busy_fixture(&fx);
    let theme = Theme::mono();

    // Blocked: a word and a glyph.
    let candidates = text_of(&drawn_in(&fx, &store, Screen::Candidates, theme));
    assert!(candidates.contains("Not offered"), "{candidates}");
    assert!(candidates.contains("⊘ "), "{candidates}");
    // Danger: the only screen that removes anything is a reversed bold band,
    // and says so in words.
    let confirm = drawn_in(&fx, &store, Screen::Confirm, theme);
    assert!((0..confirm.area.width).all(|x| {
        let m = confirm[(x, logo::TOP - 1)].modifier;
        m.contains(Modifier::REVERSED) && m.contains(Modifier::BOLD)
    }));
    assert!(text_of(&confirm).contains("to purge"));
    // Safe: the verdict names itself.
    assert!(text_of(&drawn_in(&fx, &store, Screen::Result, theme)).contains("✓ SAFE"));
}

#[test]
fn a_notice_is_tinted_by_what_it_is() {
    let fx = Fixture::new();
    let store = Fixture::new();
    many_projects(&fx, 3);
    let theme = Theme::neon();
    let now = Instant::now();
    let mut tui = driver_in(&fx, &store, Screen::Candidates, theme);
    let ink = |tui: &mut Tui| {
        let buf = frame(tui);
        let y = buf.area.height - 2;
        let x = (0..buf.area.width)
            .find(|&x| buf[(x, y)].symbol() != " ")
            .expect("a notice");
        buf[(x, y)].fg
    };

    tui.press(KeyPress::Space, now);
    assert_eq!(
        Some(ink(&mut tui)),
        theme.safe.fg,
        "a mark confirms a change"
    );
    tui.press(KeyPress::Char('z'), now);
    assert_eq!(
        Some(ink(&mut tui)),
        theme.blocked.fg,
        "a refused key is a hold"
    );
}
