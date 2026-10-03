//! A scan again that will take long asks first, and says what it knows about
//! the cost: only what the last complete scan of these roots measured.
//!
//! Two keys start a full scan from inside the interface: `Enter`/`Esc` on a
//! result, and `R` after a cancelled scan. Both go through one estimate.

pub mod common;

use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::thread::sleep;
use std::time::{Duration, Instant, SystemTime};

use common::Fixture;
use dev_cleaner::config::{CONFIRM_RESCAN_AFTER, Config, RescanPolicy, rescan_policy};
use dev_cleaner::purge::Remover;
use dev_cleaner::scan::Progress;
use dev_cleaner::store::{ScanShape, Snapshot, Store};
use dev_cleaner::tui::palette::{Mode, Theme, ThemeName};
use dev_cleaner::tui::{KeyPress, PURGE, Screen, Screens, Step, Tui, collect};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;

const SEC: Duration = Duration::from_secs(1);

/// Long enough after the dialog opened for an `Enter` to be a second decision.
const LATER: Duration = Duration::from_secs(3);

fn roots(fx: &Fixture) -> Vec<PathBuf> {
    vec![fx.root().to_path_buf()]
}

/// Record a complete scan of `roots` that took `wall` and read `entries`.
fn seed(db: &Path, roots: &[PathBuf], wall: Duration, entries: u64) {
    let mut store = Store::open(db).expect("open the store");
    let snap = Snapshot {
        started_at: SystemTime::now() + Duration::from_secs(3600),
        roots: roots.to_vec(),
        total_bytes_apparent: 3,
        total_bytes_unique: 2,
        total_inodes: 1,
        reclaimable_unique: Some(0),
        projects: Vec::new(),
        entries: Vec::new(),
    };
    let shape = ScanShape {
        entries,
        wall,
        children: Vec::new(),
    };
    store.write_snapshot_shaped(&snap, &shape).expect("write");
}

/// The interface right after a scan was cancelled: `R` is the way on.
fn cancelled(fx: &Fixture, store: &Fixture, policy: RescanPolicy, t0: Instant) -> Tui {
    let db = store.root().join("history.sqlite3");
    let progress = Arc::new(Progress::default());
    let mut tui = Tui::starting(Screens::pending(roots(fx), db), progress, t0)
        .with_theme(Theme::neon())
        .with_rescan_policy(policy);
    tui.press(KeyPress::Esc, t0 + SEC);
    tui.scan_cancelled(t0 + 2 * SEC);
    tui
}

fn frame(tui: &mut Tui, cols: u16, rows: u16) -> Buffer {
    let area = Rect::new(0, 0, cols, rows);
    let mut buf = Buffer::empty(area);
    tui.render(area, &mut buf);
    buf
}

fn lines(buf: &Buffer) -> Vec<String> {
    (0..buf.area.height)
        .map(|y| {
            (0..buf.area.width)
                .map(|x| buf[(x, y)].symbol())
                .collect::<String>()
        })
        .collect()
}

fn text(tui: &mut Tui, cols: u16, rows: u16) -> String {
    lines(&frame(tui, cols, rows)).join("\n")
}

/// Whitespace removed, so a sentence is found wherever it wrapped.
fn squashed(s: &str) -> String {
    s.chars().filter(|c| !c.is_whitespace()).collect()
}

fn asks(tui: &mut Tui) -> bool {
    text(tui, 100, 34).contains("Scan again?")
}

const EXPENSIVE: Duration = Duration::from_secs(180);

// ---- the estimate decides ----

#[test]
fn r_after_a_cancel_asks_when_the_last_scan_was_slow_and_says_what_it_cost() {
    let (fx, store) = (Fixture::new(), Fixture::new());
    seed(
        &store.root().join("history.sqlite3"),
        &roots(&fx),
        EXPENSIVE,
        1_490_000,
    );
    let t0 = Instant::now();
    let mut tui = cancelled(&fx, &store, RescanPolicy::default(), t0);

    assert_eq!(tui.press(KeyPress::Char('R'), t0 + 3 * SEC), Step::Stay);

    let shown = text(&mut tui, 100, 34);
    assert!(shown.contains("Scan again?"), "{shown}");
    let flat = squashed(&shown);
    assert!(flat.contains("1,490,000entries"), "{shown}");
    assert!(flat.contains("Lasttimeittook3min0s"), "{shown}");
    assert!(
        !flat.contains("willtake"),
        "a dialog states what was measured, never what will happen: {shown}"
    );
}

#[test]
fn a_cheap_last_scan_starts_without_asking() {
    let (fx, store) = (Fixture::new(), Fixture::new());
    seed(
        &store.root().join("history.sqlite3"),
        &roots(&fx),
        Duration::from_secs(2),
        900,
    );
    let t0 = Instant::now();
    let mut tui = cancelled(&fx, &store, RescanPolicy::default(), t0);

    assert_eq!(tui.press(KeyPress::Char('R'), t0 + 3 * SEC), Step::Rescan);
    assert!(!asks(&mut tui));
}

#[test]
fn no_complete_scan_on_record_asks_and_says_the_cost_is_unknown() {
    let (fx, store) = (Fixture::new(), Fixture::new());
    let t0 = Instant::now();
    let mut tui = cancelled(&fx, &store, RescanPolicy::default(), t0);

    assert_eq!(tui.press(KeyPress::Char('R'), t0 + 3 * SEC), Step::Stay);

    let shown = text(&mut tui, 100, 34);
    assert!(shown.contains("Scan again?"), "{shown}");
    assert!(squashed(&shown).contains("isunknown"), "{shown}");
    assert!(!squashed(&shown).contains("Lasttime"), "{shown}");
}

#[test]
fn the_threshold_is_the_policys_and_the_default_is_ten_seconds() {
    assert_eq!(CONFIRM_RESCAN_AFTER, Duration::from_secs(10));
    assert_eq!(RescanPolicy::default().threshold, CONFIRM_RESCAN_AFTER);
    let (fx, store) = (Fixture::new(), Fixture::new());
    seed(
        &store.root().join("history.sqlite3"),
        &roots(&fx),
        EXPENSIVE,
        1_490_000,
    );
    let t0 = Instant::now();
    let patient = RescanPolicy {
        confirm: true,
        threshold: Duration::from_secs(600),
    };
    let mut tui = cancelled(&fx, &store, patient, t0);

    assert_eq!(tui.press(KeyPress::Char('R'), t0 + 3 * SEC), Step::Rescan);
}

#[test]
fn confirm_rescan_false_never_asks_even_when_the_cost_is_unknown() {
    let (fx, store) = (Fixture::new(), Fixture::new());
    let t0 = Instant::now();
    let off = RescanPolicy {
        confirm: false,
        ..RescanPolicy::default()
    };
    let mut tui = cancelled(&fx, &store, off, t0);

    assert_eq!(tui.press(KeyPress::Char('R'), t0 + 3 * SEC), Step::Rescan);
}

#[test]
fn an_interface_with_no_policy_never_asks() {
    // The loop opts in; a bare `Tui` has no store to read and asks nothing.
    let (fx, store) = (Fixture::new(), Fixture::new());
    let t0 = Instant::now();
    let progress = Arc::new(Progress::default());
    let db = store.root().join("history.sqlite3");
    let mut tui = Tui::starting(Screens::pending(roots(&fx), db), progress, t0);
    tui.press(KeyPress::Esc, t0 + SEC);
    tui.scan_cancelled(t0 + 2 * SEC);

    assert_eq!(tui.press(KeyPress::Char('R'), t0 + 3 * SEC), Step::Rescan);
}

// ---- the dialog is modal ----

fn open(fx: &Fixture, store: &Fixture, t0: Instant) -> Tui {
    seed(
        &store.root().join("history.sqlite3"),
        &roots(fx),
        EXPENSIVE,
        1_490_000,
    );
    let mut tui = cancelled(fx, store, RescanPolicy::default(), t0);
    tui.press(KeyPress::Char('R'), t0 + 3 * SEC);
    assert!(asks(&mut tui));
    tui
}

#[test]
fn enter_starts_the_scan_and_closes_the_dialog() {
    let (fx, store) = (Fixture::new(), Fixture::new());
    let t0 = Instant::now();
    let mut tui = open(&fx, &store, t0);

    assert_eq!(
        tui.press(KeyPress::Enter, t0 + 3 * SEC + LATER),
        Step::Rescan
    );
    assert!(!asks(&mut tui));
}

#[test]
fn escape_closes_the_dialog_and_changes_nothing() {
    let (fx, store) = (Fixture::new(), Fixture::new());
    let t0 = Instant::now();
    let mut tui = open(&fx, &store, t0);
    let behind = {
        // What is under the dialog, for comparison: a twin that never opened it.
        let mut twin = cancelled(&fx, &store, RescanPolicy::default(), t0);
        text(&mut twin, 100, 34)
    };

    assert_eq!(tui.press(KeyPress::Esc, t0 + 4 * SEC), Step::Stay);

    assert!(!asks(&mut tui));
    assert!(!tui.is_scanning());
    let shown = text(&mut tui, 100, 34);
    assert!(
        shown.contains("R scans again"),
        "still where it was: {shown}"
    );
    // The same body; only the notice row may differ.
    let body = |s: &str| s.lines().take(30).collect::<Vec<_>>().join("\n");
    assert_eq!(body(&shown), body(&behind));
}

#[test]
fn every_other_key_is_ignored_and_the_dialog_stays() {
    let (fx, store) = (Fixture::new(), Fixture::new());
    let t0 = Instant::now();
    let mut tui = open(&fx, &store, t0);
    let theme = tui.theme().name();

    for (i, key) in [
        KeyPress::Char('q'),
        KeyPress::Char('R'),
        KeyPress::Char('T'),
        KeyPress::Char('c'),
        KeyPress::Char('?'),
        KeyPress::Space,
        KeyPress::Tab,
        KeyPress::Up,
    ]
    .into_iter()
    .enumerate()
    {
        let at = t0 + 3 * SEC + LATER + SEC * i as u32;
        assert_eq!(tui.press(key, at), Step::Stay, "{key} must do nothing");
        assert!(asks(&mut tui), "{key} closed the dialog");
    }
    assert_eq!(tui.theme().name(), theme, "T must not switch under a modal");
}

#[test]
fn the_key_bar_offers_enter_and_escape_and_nothing_else() {
    let (fx, store) = (Fixture::new(), Fixture::new());
    let t0 = Instant::now();
    let mut tui = open(&fx, &store, t0);

    let rows = lines(&frame(&mut tui, 100, 34));
    let bar = rows.last().expect("a last row");

    assert!(bar.contains("Enter"), "{bar}");
    assert!(bar.contains("scan again"), "{bar}");
    assert!(bar.contains("Esc"), "{bar}");
    assert!(bar.contains("stay"), "{bar}");
    assert!(!bar.contains("quit"), "{bar}");
    assert!(!bar.contains("R "), "{bar}");
}

#[test]
fn the_key_that_opened_it_does_not_confirm_it() {
    let (fx, store) = (Fixture::new(), Fixture::new());
    let t0 = Instant::now();
    let mut tui = open(&fx, &store, t0);

    // A double tap, and a held key's repeats: none of them is a decision.
    for ms in [40u64, 120, 400, 700, 1000] {
        let at = t0 + 3 * SEC + Duration::from_millis(ms);
        assert_eq!(tui.press(KeyPress::Enter, at), Step::Stay, "{ms} ms");
    }
    assert!(asks(&mut tui));
    // Let go, read it, press once: that is.
    assert_eq!(
        tui.press(
            KeyPress::Enter,
            t0 + 3 * SEC + Duration::from_millis(1000) + LATER
        ),
        Step::Rescan
    );
}

#[test]
fn it_survives_a_resize_centred_and_readable_at_80_by_24() {
    let (fx, store) = (Fixture::new(), Fixture::new());
    let t0 = Instant::now();
    let mut tui = open(&fx, &store, t0);

    for (cols, rows) in [(80u16, 24u16), (100, 34), (140, 50), (80, 24)] {
        let buf = frame(&mut tui, cols, rows);
        let all = lines(&buf);
        let flat = squashed(&all.join("\n"));
        assert!(flat.contains("Scanagain?"), "{cols}x{rows}");
        assert!(flat.contains("1,490,000entries"), "{cols}x{rows}");
        let title = all
            .iter()
            .find(|l| l.contains("Scan again?"))
            .unwrap_or_else(|| panic!("{cols}x{rows}: no title row"));
        // The box's top edge, corner to corner, sits on the middle of the screen.
        let chars: Vec<char> = title.chars().collect();
        let left = chars.iter().position(|c| !c.is_whitespace()).unwrap() as i32;
        let right = chars.iter().rposition(|c| !c.is_whitespace()).unwrap() as i32;
        let centre = (left + right) / 2;
        assert!(
            (centre - i32::from(cols) / 2).abs() <= 1,
            "{cols}x{rows}: box from {left} to {right}"
        );
        let bar = &all[all.len() - 1];
        assert!(
            bar.contains("Enter") && bar.contains("Esc"),
            "{cols}x{rows}: {bar}"
        );
    }
}

#[test]
fn it_is_drawn_in_every_theme_and_colour_mode_with_words_for_everything() {
    for name in [ThemeName::Neon, ThemeName::Matrix] {
        for mode in [Mode::Truecolor, Mode::Ansi, Mode::Mono] {
            let (fx, store) = (Fixture::new(), Fixture::new());
            let t0 = Instant::now();
            let mut tui = open(&fx, &store, t0).with_theme(Theme::named(name, mode));
            let shown = text(&mut tui, 80, 24);
            let flat = squashed(&shown);
            // The meaning is the words: with no colour at all they are all there.
            for want in ["Scanagain?", "1,490,000entries", "Enter", "Esc", "stay"] {
                assert!(
                    flat.contains(want),
                    "{name:?}/{mode:?} lacks {want}: {shown}"
                );
            }
        }
    }
}

#[test]
fn the_dialog_is_drawn_in_the_kit_roles() {
    let (fx, store) = (Fixture::new(), Fixture::new());
    let t0 = Instant::now();
    for name in [ThemeName::Neon, ThemeName::Matrix] {
        let theme = Theme::named(name, Mode::Truecolor);
        let mut tui = open(&fx, &store, t0).with_theme(theme);
        let buf = frame(&mut tui, 100, 34);
        let row = lines(&buf)
            .iter()
            .position(|l| l.contains("Scan again?"))
            .expect("title row") as u16;
        let x = lines(&buf)[row as usize]
            .chars()
            .position(|c| c == 'S')
            .expect("title") as u16;
        assert_eq!(
            buf[(x, row)].fg,
            theme.head.fg.unwrap_or_default(),
            "{name:?}"
        );
        assert_eq!(
            buf[(x, row)].bg,
            theme.ground.bg.unwrap_or_default(),
            "{name:?}"
        );
    }
}

// ---- already scanning ----

#[test]
fn r_while_a_scan_runs_says_already_scanning_and_starts_nothing() {
    let (fx, store) = (Fixture::new(), Fixture::new());
    let t0 = Instant::now();
    let progress = Arc::new(Progress::default());
    let db = store.root().join("history.sqlite3");
    let mut tui = Tui::starting(Screens::pending(roots(&fx), db), progress, t0)
        .with_theme(Theme::neon())
        .with_rescan_policy(RescanPolicy::default());

    assert_eq!(tui.press(KeyPress::Char('R'), t0 + SEC), Step::Stay);

    let shown = text(&mut tui, 100, 34);
    assert!(shown.contains("Already scanning"), "{shown}");
    assert!(!asks(&mut tui));
    // And again, once it is stopping.
    tui.press(KeyPress::Esc, t0 + 2 * SEC);
    assert_eq!(tui.press(KeyPress::Char('R'), t0 + 3 * SEC), Step::Stay);
    assert!(text(&mut tui, 100, 34).contains("Already scanning"));
}

// ---- the result screen ----

struct Away(PathBuf);

impl Remover for Away {
    fn remove(&self, path: &Path) -> io::Result<PathBuf> {
        let to = self.0.join(path.to_string_lossy().replace('/', "_"));
        std::fs::rename(path, &to)?;
        Ok(to)
    }
}

fn scan(fx: &Fixture, store: &Fixture) -> Screens {
    let cfg = Config {
        roots: roots(fx),
        caches: Vec::new(),
        denylist: Vec::new(),
    };
    collect(
        &roots(fx),
        &cfg,
        fx.root(),
        &store.root().join("history.sqlite3"),
    )
}

/// An interface sitting on a result of a purge, with `wall` the last complete
/// scan of these roots cost (`None`: none on record).
fn on_a_result(
    fx: &Fixture,
    store: &Fixture,
    away: &Fixture,
    records: &Fixture,
    last: Option<(Duration, u64)>,
    now: Instant,
) -> Tui {
    fx.file("app/package.json", b"{}");
    fx.file("app/src/index.js", b"1");
    fx.file("app/node_modules/dep/blob.bin", &[7u8; 4096]);
    let screens = scan(fx, store);
    let db = store.root().join("history.sqlite3");
    if let Some((wall, entries)) = last {
        seed(&db, &roots(fx), wall, entries);
    }
    let mut tui = Tui::new(screens)
        .with_theme(Theme::neon())
        .with_manifest_dir(records.root().to_path_buf())
        .with_rescan_policy(RescanPolicy::default());
    while tui.app().screen() != Screen::Candidates {
        tui.press(KeyPress::Enter, now);
    }
    tui.press(KeyPress::Space, now);
    while tui.app().screen() != Screen::Confirm {
        tui.press(KeyPress::Enter, now);
    }
    let mut step = Step::Stay;
    for repeat in 0..80u32 {
        step = tui.press(PURGE, now + Duration::from_millis(50 * u64::from(repeat)));
        if step == Step::Purge {
            break;
        }
    }
    assert_eq!(step, Step::Purge, "the hold never armed");
    tui.purge(Box::new(Away(away.root().to_path_buf())));
    for _ in 0..1000 {
        tui.tick(Instant::now());
        if tui.app().screen() == Screen::Result {
            return tui;
        }
        sleep(Duration::from_millis(10));
    }
    panic!("the purge never reached its result");
}

#[test]
fn enter_and_escape_on_a_result_ask_when_the_last_scan_was_slow() {
    for key in [KeyPress::Enter, KeyPress::Esc] {
        let (fx, store, away, records) = (
            Fixture::new(),
            Fixture::new(),
            Fixture::new(),
            Fixture::new(),
        );
        let now = Instant::now();
        let mut tui = on_a_result(
            &fx,
            &store,
            &away,
            &records,
            Some((EXPENSIVE, 1_490_000)),
            now,
        );

        assert_eq!(tui.press(key, now), Step::Stay, "{key}");

        let shown = text(&mut tui, 100, 34);
        assert!(shown.contains("Scan again?"), "{key}: {shown}");
        assert!(squashed(&shown).contains("1,490,000entries"), "{key}");
        assert_eq!(
            tui.app().screen(),
            Screen::Result,
            "{key}: stays where it is"
        );
    }
}

#[test]
fn enter_on_a_result_scans_at_once_when_the_last_scan_was_quick() {
    let (fx, store, away, records) = (
        Fixture::new(),
        Fixture::new(),
        Fixture::new(),
        Fixture::new(),
    );
    let now = Instant::now();
    let mut tui = on_a_result(
        &fx,
        &store,
        &away,
        &records,
        Some((Duration::from_secs(2), 900)),
        now,
    );

    assert_eq!(tui.press(KeyPress::Enter, now), Step::Rescan);
}

#[test]
fn escape_on_the_dialog_over_a_result_leaves_the_result_standing() {
    let (fx, store, away, records) = (
        Fixture::new(),
        Fixture::new(),
        Fixture::new(),
        Fixture::new(),
    );
    let now = Instant::now();
    let mut tui = on_a_result(
        &fx,
        &store,
        &away,
        &records,
        Some((EXPENSIVE, 1_490_000)),
        now,
    );
    tui.press(KeyPress::Enter, now);

    assert_eq!(tui.press(KeyPress::Esc, now + SEC), Step::Stay);

    assert!(!text(&mut tui, 100, 34).contains("Scan again?"));
    assert_eq!(tui.app().screen(), Screen::Result);
    // And the next Enter asks again rather than scanning.
    assert_eq!(tui.press(KeyPress::Enter, now + 2 * SEC), Step::Stay);
    assert!(text(&mut tui, 100, 34).contains("Scan again?"));
}

// ---- the config ----

#[test]
fn the_config_keys_default_to_asking_after_ten_seconds() {
    let fx = Fixture::new();
    let missing = fx.root().join("none.toml");
    assert_eq!(rescan_policy(&missing), RescanPolicy::default());
    assert!(RescanPolicy::default().confirm);
}

#[test]
fn confirm_rescan_and_its_threshold_are_read_from_the_config() {
    let fx = Fixture::new();
    let file = fx.root().join("config.toml");

    std::fs::write(&file, "confirm_rescan = false\n").unwrap();
    assert!(!rescan_policy(&file).confirm);

    std::fs::write(&file, "confirm_rescan_after_secs = 90\n").unwrap();
    let policy = rescan_policy(&file);
    assert!(policy.confirm);
    assert_eq!(policy.threshold, Duration::from_secs(90));

    // A value of the wrong type is a typo, and a typo keeps the safe default.
    std::fs::write(
        &file,
        "confirm_rescan = \"no\"\nconfirm_rescan_after_secs = -3\n",
    )
    .unwrap();
    assert_eq!(rescan_policy(&file), RescanPolicy::default());

    // The existing keys still load beside them.
    std::fs::write(&file, "roots = [\"/r\"]\nconfirm_rescan = false\n").unwrap();
    assert!(Config::load(&file).is_ok());
}
