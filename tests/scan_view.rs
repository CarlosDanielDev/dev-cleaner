//! The interface while a scan runs behind it: alive from the first frame, honest
//! about how far along it is, stoppable, and never offering what it has not
//! finished measuring.

mod common;

use std::collections::BTreeMap;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use common::Fixture;
use dev_cleaner::config::Config;
use dev_cleaner::scan::{Baseline, Phase, Progress, Walker};
use dev_cleaner::tui::palette::Theme;
use dev_cleaner::tui::{KeyPress, NOTICE_TTL, Screen, Screens, Step, Tui, collect};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;

const SEC: Duration = Duration::from_secs(1);

fn pending(fx: &Fixture) -> Screens {
    Screens::pending(
        vec![fx.root().to_path_buf()],
        fx.root().join("none.sqlite3"),
    )
}

/// A scan that has just begun, on a terminal-less interface.
fn starting(fx: &Fixture, now: Instant) -> (Tui, Arc<Progress>) {
    let progress = Arc::new(Progress::default());
    let tui = Tui::starting(pending(fx), Arc::clone(&progress), now).with_theme(Theme::neon());
    (tui, progress)
}

fn frame(tui: &mut Tui, cols: u16, rows: u16) -> Buffer {
    let area = Rect::new(0, 0, cols, rows);
    let mut buf = Buffer::empty(area);
    tui.render(area, &mut buf);
    buf
}

fn lines(buf: &Buffer) -> Vec<String> {
    let area = buf.area;
    (area.y..area.bottom())
        .map(|y| {
            (area.x..area.right())
                .map(|x| buf[(x, y)].symbol())
                .collect::<String>()
        })
        .collect()
}

fn text(tui: &mut Tui, cols: u16, rows: u16) -> String {
    lines(&frame(tui, cols, rows)).join("\n")
}

/// The cells the master logo is painted with: nothing else on screen is a
/// block or half block.
fn logo_cells(buf: &Buffer) -> usize {
    lines(buf)
        .iter()
        .flat_map(|l| l.chars())
        .filter(|c| matches!(c, '▀' | '▄' | '█'))
        .count()
}

/// Two top-level folders of `a` and `b` files, walked into `progress` so the
/// folder table is the real one.
fn walked(fx: &Fixture, progress: &Arc<Progress>, a: usize, b: usize) {
    for i in 0..a {
        fx.file(&format!("a/f{i}"), b"x");
    }
    for i in 0..b {
        fx.file(&format!("b/f{i}"), b"x");
    }
    Walker::new([fx.root()]).walk_with(progress);
}

fn last_scan(fx: &Fixture, a: u64, b: u64) -> Baseline {
    Baseline {
        entries: a + b,
        wall: Duration::from_secs(200),
        children: BTreeMap::from([(fx.root().join("a"), a), (fx.root().join("b"), b)]),
    }
}

#[test]
fn the_interface_is_drawn_before_the_scan_has_found_anything() {
    let fx = Fixture::new();
    let t0 = Instant::now();
    let (mut tui, _progress) = starting(&fx, t0);

    let shown = text(&mut tui, 100, 34);

    assert!(
        shown.contains("Scanning"),
        "the progress block is there\n{shown}"
    );
    assert!(
        shown.contains("Dashboard"),
        "and so is the stepper\n{shown}"
    );
    assert!(shown.contains("Candidates"), "{shown}");
    assert!(shown.contains("Esc"), "and the way to stop it\n{shown}");
    assert_eq!(tui.app().screen(), Screen::Dashboard);
}

#[test]
fn the_logo_is_the_empty_state_for_one_second_and_never_comes_back() {
    let fx = Fixture::new();
    let t0 = Instant::now();
    let (mut tui, progress) = starting(&fx, t0);

    tui.tick(t0 + Duration::from_millis(500));
    let early = logo_cells(&frame(&mut tui, 160, 50));
    tui.tick(t0 + Duration::from_millis(1_100));
    let late = logo_cells(&frame(&mut tui, 160, 50));
    assert!(
        early > 100,
        "the logo is up at half a second ({early} cells)"
    );
    assert_eq!(late, 0, "and gone after one");

    // Finding something ends it sooner.
    let (mut busy, progress2) = starting(&fx, t0);
    fx.file("p/package.json", b"{}");
    Walker::new([fx.root()]).walk_with(&progress2);
    busy.tick(t0 + Duration::from_millis(200));
    assert_eq!(logo_cells(&frame(&mut busy, 160, 50)), 0);
    drop(progress);
}

#[test]
fn without_an_earlier_scan_there_is_no_percentage_and_the_counts_are_there() {
    let fx = Fixture::new();
    let t0 = Instant::now();
    let (mut tui, progress) = starting(&fx, t0);
    progress.entries.store(7_412, Ordering::Relaxed);
    progress
        .bytes
        .store(3 * 1024 * 1024 * 1024, Ordering::Relaxed);

    tui.tick(t0 + 2 * SEC);
    let shown = text(&mut tui, 100, 34);

    assert!(!shown.contains('%'), "no percentage was earned\n{shown}");
    assert!(shown.contains("no earlier scan"), "{shown}");
    assert!(shown.contains("7,412 entries"), "{shown}");
    assert!(shown.contains("3.00 GB"), "{shown}");
}

#[test]
fn with_an_earlier_scan_the_bar_is_an_estimate_and_says_so() {
    let fx = Fixture::new();
    let t0 = Instant::now();
    let (mut tui, progress) = starting(&fx, t0);
    walked(&fx, &progress, 10, 10);
    // Last time /a held 10 and /b held 30: of 40, 20 are seen.
    progress.set_baseline(Some(last_scan(&fx, 10, 30)));

    tui.tick(t0 + 3 * SEC);
    let shown = text(&mut tui, 100, 34);

    assert!(shown.contains("~50%"), "{shown}");
    assert!(shown.contains("of last scan"), "{shown}");
}

#[test]
fn the_estimate_stops_at_ninety_five_and_past_the_last_scan_it_says_so() {
    let fx = Fixture::new();
    let t0 = Instant::now();
    let (mut tui, progress) = starting(&fx, t0);
    walked(&fx, &progress, 10, 10);
    progress.set_baseline(Some(last_scan(&fx, 10, 10)));
    tui.tick(t0 + 3 * SEC);
    let shown = text(&mut tui, 100, 34);
    assert!(shown.contains("~95%"), "{shown}");
    assert!(!shown.contains("100%") && !shown.contains("99%"), "{shown}");

    // The folders grew: there is more than last time.
    let (mut over, progress) = starting(&fx, t0);
    Walker::new([fx.root()]).walk_with(&progress);
    progress.set_baseline(Some(last_scan(&fx, 4, 4)));
    over.tick(t0 + 3 * SEC);
    let shown = text(&mut over, 100, 34);
    assert!(shown.contains("past last scan"), "{shown}");
    assert!(
        !shown.contains('%'),
        "no full-looking bar to sit on\n{shown}"
    );
}

#[test]
fn rate_and_elapsed_are_on_every_frame_at_every_size() {
    let fx = Fixture::new();
    let t0 = Instant::now();
    let (mut tui, progress) = starting(&fx, t0);
    progress.entries.store(20_000, Ordering::Relaxed);
    tui.tick(t0 + 4 * SEC);

    for (cols, rows) in [(80, 24), (100, 34), (160, 40)] {
        let shown = text(&mut tui, cols, rows);
        assert!(shown.contains("5,000 entries/s"), "{cols}x{rows}\n{shown}");
        assert!(shown.contains("4 s"), "{cols}x{rows}\n{shown}");
    }
}

#[test]
fn a_range_of_time_left_waits_for_a_baseline_a_quarter_and_ten_seconds() {
    let fx = Fixture::new();
    let t0 = Instant::now();
    let (mut tui, progress) = starting(&fx, t0);
    walked(&fx, &progress, 10, 10);
    tui.tick(t0 + 30 * SEC);
    assert!(
        !text(&mut tui, 100, 34).contains("left"),
        "no baseline, no range"
    );

    progress.set_baseline(Some(last_scan(&fx, 10, 30)));
    tui.tick(t0 + 5 * SEC);
    assert!(!text(&mut tui, 100, 34).contains("left"), "too soon");

    // Time passing without the walk moving is a stall, and withholds the
    // range: the sampled moments below must each see the walk move.
    let (mut tui, progress) = starting(&fx, t0);
    Walker::new([fx.root()]).walk_with(&progress);
    progress.set_baseline(Some(last_scan(&fx, 10, 30)));
    tui.tick(t0 + 12 * SEC);
    let shown = text(&mut tui, 100, 34);
    assert!(
        shown.contains("about") && shown.contains(" to ") && shown.contains("left"),
        "{shown}"
    );
}

#[test]
fn a_counter_that_stops_says_so_in_the_warning_ink() {
    let fx = Fixture::new();
    let t0 = Instant::now();
    let (mut tui, progress) = starting(&fx, t0);
    progress.entries.store(100, Ordering::Relaxed);
    tui.tick(t0 + SEC);
    tui.tick(t0 + 8 * SEC);

    let buf = frame(&mut tui, 100, 34);
    let rows = lines(&buf);
    let (y, row) = rows
        .iter()
        .enumerate()
        .find(|(_, l)| l.contains("no progress for 7 s"))
        .expect("the stall is named");
    let x = row.find("no progress").expect("column");
    let col = row[..x].chars().count() as u16;
    assert_eq!(
        buf[(col, y as u16)].fg,
        Theme::neon().blocked.fg.expect("fg")
    );
}

#[test]
fn the_phases_after_the_walk_have_exact_percentages_and_keep_the_entry_count() {
    let fx = Fixture::new();
    let t0 = Instant::now();
    let (mut tui, progress) = starting(&fx, t0);
    progress.entries.store(1_491_788, Ordering::Relaxed);
    progress.set_phase(Phase::Classifying, 40);
    progress.tick_by(18);

    tui.tick(t0 + 60 * SEC);
    let shown = text(&mut tui, 100, 34);

    assert!(shown.contains("classifying"), "{shown}");
    assert!(
        shown.contains("45%") && !shown.contains("~45%"),
        "exact, not a guess\n{shown}"
    );
    assert!(shown.contains("18 of 40"), "{shown}");
    assert!(
        shown.contains("1,491,788 entries"),
        "the walk's count does not vanish when the walk is over\n{shown}"
    );
}

#[test]
fn unreadable_folders_are_counted_and_not_listed() {
    let fx = Fixture::new();
    fx.file("open/a", b"a");
    fx.file("locked/x", b"x");
    let locked = fx.root().join("locked");
    fs::set_permissions(&locked, fs::Permissions::from_mode(0o000)).expect("chmod");
    let t0 = Instant::now();
    let (mut tui, progress) = starting(&fx, t0);
    Walker::new([fx.root()]).walk_with(&progress);
    fs::set_permissions(&locked, fs::Permissions::from_mode(0o755)).expect("chmod");

    tui.tick(t0 + 2 * SEC);
    let shown = text(&mut tui, 100, 34);

    assert!(shown.contains("1 unreadable folder"), "{shown}");
    assert!(!shown.contains("1 unreadable folders"), "{shown}");
}

#[test]
fn esc_and_c_cancel_and_the_walk_is_told() {
    for key in [KeyPress::Esc, KeyPress::Char('c')] {
        let fx = Fixture::new();
        let t0 = Instant::now();
        let (mut tui, progress) = starting(&fx, t0);

        assert_eq!(tui.press(key, t0 + SEC), Step::Stay);

        assert!(progress.is_cancelled(), "{key} must reach the walk");
        tui.tick(t0 + 2 * SEC);
        assert!(text(&mut tui, 100, 34).contains("Stopping"));
    }
}

#[test]
fn a_cancelled_scan_says_so_offers_a_rescan_and_builds_nothing() {
    let fx = Fixture::new();
    let t0 = Instant::now();
    let (mut tui, progress) = starting(&fx, t0);
    progress.entries.store(412_330, Ordering::Relaxed);
    tui.tick(t0 + 72 * SEC);
    tui.press(KeyPress::Esc, t0 + 72 * SEC);

    tui.scan_cancelled(t0 + 73 * SEC);
    let shown = text(&mut tui, 100, 34);

    assert!(
        shown.contains("Scan cancelled after 1 min 12 s · 412,330 entries · nothing was written"),
        "{shown}"
    );
    assert!(shown.contains("R"), "{shown}");
    assert!(!tui.is_scanning());
    // Nothing to open: every key but R and q says why.
    assert_eq!(tui.press(KeyPress::Enter, t0 + 74 * SEC), Step::Stay);
    assert_eq!(tui.app().screen(), Screen::Dashboard);
    assert!(text(&mut tui, 100, 34).contains("R scans again"));
    assert_eq!(tui.press(KeyPress::Char('R'), t0 + 75 * SEC), Step::Rescan);
    assert_eq!(tui.press(KeyPress::Char('q'), t0 + 76 * SEC), Step::Quit);
}

#[test]
fn no_key_but_the_scan_keys_does_anything_while_it_runs() {
    let fx = Fixture::new();
    let t0 = Instant::now();
    let (mut tui, progress) = starting(&fx, t0);

    for key in [
        KeyPress::Enter,
        KeyPress::Space,
        KeyPress::Tab,
        KeyPress::Char('x'),
        KeyPress::Char('2'),
        KeyPress::Char('a'),
        KeyPress::Char('R'),
    ] {
        assert_eq!(tui.press(key, t0 + SEC), Step::Stay, "{key}");
        assert_eq!(
            tui.app().screen(),
            Screen::Dashboard,
            "{key} left the dashboard"
        );
    }
    let shown = text(&mut tui, 100, 34);
    assert!(
        shown.contains("The scan is running"),
        "a refused key says why\n{shown}"
    );
    assert!(!progress.is_cancelled(), "and none of them cancelled it");
}

#[test]
fn q_asks_once_names_what_is_dropped_and_a_second_q_quits() {
    let fx = Fixture::new();
    let t0 = Instant::now();
    let (mut tui, progress) = starting(&fx, t0);
    progress.entries.store(5_000, Ordering::Relaxed);
    tui.tick(t0 + 20 * SEC);

    assert_eq!(tui.press(KeyPress::Char('q'), t0 + 20 * SEC), Step::Stay);
    let shown = text(&mut tui, 120, 34);
    assert!(
        shown.contains("5,000 entries"),
        "names what is dropped\n{shown}"
    );
    assert!(shown.contains("nothing is written"), "{shown}");
    assert!(shown.contains("q again"), "{shown}");

    assert_eq!(tui.press(KeyPress::Char('q'), t0 + 21 * SEC), Step::Quit);
}

#[test]
fn a_key_between_two_qs_takes_the_first_back_and_so_does_waiting() {
    let fx = Fixture::new();
    let t0 = Instant::now();
    let (mut tui, _p) = starting(&fx, t0);

    tui.press(KeyPress::Char('q'), t0 + SEC);
    tui.press(KeyPress::Tab, t0 + 2 * SEC);
    assert_eq!(tui.press(KeyPress::Char('q'), t0 + 3 * SEC), Step::Stay);

    let later = t0 + 3 * SEC + NOTICE_TTL + SEC;
    assert_eq!(
        tui.press(KeyPress::Char('q'), later),
        Step::Stay,
        "asks again after the TTL"
    );
}

#[test]
fn a_finished_scan_replaces_the_progress_with_the_dashboard() {
    let fx = Fixture::new();
    fx.file("app/package.json", b"{}");
    fx.file("app/node_modules/d/b.bin", &[1u8; 4096]);
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
        &store.root().join("h.sqlite3"),
    );
    let t0 = Instant::now();
    let (mut tui, _p) = starting(&fx, t0);

    tui.finish_scan(screens, t0 + 5 * SEC);

    assert!(!tui.is_scanning());
    let shown = text(&mut tui, 100, 34);
    assert!(!shown.contains("Scanning"), "{shown}");
    assert!(shown.contains("1 project"), "{shown}");
    assert!(shown.contains("Scan finished"), "{shown}");
}

#[test]
fn a_rescan_after_a_purge_hides_the_numbers_it_is_replacing() {
    let fx = Fixture::new();
    fx.file("oldname/package.json", b"{}");
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
        &store.root().join("h.sqlite3"),
    );
    let mut tui = Tui::new(screens).with_theme(Theme::neon());
    let t0 = Instant::now();

    tui.begin_scan(Arc::new(Progress::default()), t0);
    let shown = text(&mut tui, 100, 34);

    assert!(shown.contains("Scanning"), "{shown}");
    assert!(shown.contains("measured again"), "{shown}");
    assert!(
        !shown.contains("1 project"),
        "the old count is not shown as current\n{shown}"
    );
    assert!(!shown.contains("scanned"), "{shown}");
}

#[test]
fn a_reader_hammered_by_a_million_counts_still_draws_and_never_goes_backwards() {
    let fx = Fixture::new();
    let t0 = Instant::now();
    let (mut tui, progress) = starting(&fx, t0);

    let source = {
        let progress = Arc::clone(&progress);
        std::thread::spawn(move || {
            for _ in 0..1_000_000 {
                progress.entries.fetch_add(1, Ordering::Relaxed);
            }
        })
    };
    let began = Instant::now();
    let mut last = 0u64;
    let mut frames = 0;
    while !source.is_finished() && frames < 400 {
        tui.tick(t0 + Duration::from_millis(100 * frames));
        let seen = progress.read().entries;
        assert!(seen >= last, "a counter that goes backwards");
        last = seen;
        let _ = frame(&mut tui, 100, 34);
        frames += 1;
    }
    source.join().expect("source");
    assert!(
        began.elapsed() < Duration::from_secs(20),
        "{frames} frames took {:?}: drawing waited on the counters",
        began.elapsed()
    );
    // However many events there were, there were only as many redraws as ticks.
    assert!(frames <= 400);
}

#[test]
fn resizing_mid_scan_never_panics() {
    let fx = Fixture::new();
    let t0 = Instant::now();
    let (mut tui, progress) = starting(&fx, t0);
    walked(&fx, &progress, 3, 3);
    progress.set_baseline(Some(last_scan(&fx, 6, 6)));
    tui.tick(t0 + 30 * SEC);
    for cols in [0u16, 1, 2, 10, 40, 79, 80, 81, 120] {
        for rows in [0u16, 1, 2, 3, 4, 10, 23, 24, 25, 60] {
            let _ = frame(&mut tui, cols, rows);
        }
    }
    let shown = text(&mut tui, 60, 20);
    assert!(shown.contains("80×24"), "{shown}");
    assert!(
        shown.contains("scan keeps running"),
        "a terminal too small says the scan goes on without it\n{shown}"
    );
}

#[test]
fn a_scan_that_finished_while_being_stopped_says_it_was_kept() {
    // Past the point of stopping the scan writes itself down. Esc was pressed
    // and answered "nothing is written", so the result has to say what became of it.
    let fx = Fixture::new();
    fx.file("app/package.json", b"{}");
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
        &store.root().join("h.sqlite3"),
    );
    let t0 = Instant::now();
    let (mut tui, _p) = starting(&fx, t0);
    tui.press(KeyPress::Esc, t0 + SEC);

    tui.finish_scan(screens, t0 + 2 * SEC);

    let shown = text(&mut tui, 120, 34);
    assert!(
        shown.contains("already saving") && shown.contains("kept"),
        "{shown}"
    );
}
