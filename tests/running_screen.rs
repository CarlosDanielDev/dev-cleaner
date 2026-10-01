//! The running screen: the purge drawn while it happens.
//!
//! Every test here drives the real loop object with a remover double that
//! blocks on a channel, so "between items" is a state the test holds still
//! rather than a moment it hopes to catch.

pub mod common;

use std::path::PathBuf;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use common::Fixture;
use dev_cleaner::bytes::human;
use dev_cleaner::config::Config;
use dev_cleaner::purge::Remover;
use dev_cleaner::tui::{KeyPress, PURGE, Screen, Screens, Step, Tui, collect};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;

const ITEMS: usize = 12;
const AREA: Rect = Rect::new(0, 0, 110, 40);

/// Each `remove` waits for the test to release it, and says which paths it saw.
struct Gated {
    release: Mutex<Receiver<()>>,
    seen: Arc<Mutex<Vec<PathBuf>>>,
    /// Panic on this call, counting from one.
    panic_on: Option<usize>,
}

impl Remover for Gated {
    fn remove(&self, path: &std::path::Path) -> std::io::Result<PathBuf> {
        let call = {
            let mut seen = self.seen.lock().expect("lock");
            seen.push(path.to_path_buf());
            seen.len()
        };
        // A test that never releases must fail, not hang.
        self.release
            .lock()
            .expect("lock")
            .recv_timeout(Duration::from_secs(20))
            .map_err(|_| std::io::Error::other("the test never released this item"))?;
        if self.panic_on == Some(call) {
            panic!("the remover fell over on item {call}");
        }
        Ok(PathBuf::from("/Users/test/.Trash").join(path.file_name().expect("name")))
    }
}

/// A gated remover, the handle that releases it, and what it has been asked to
/// remove.
fn gated(panic_on: Option<usize>) -> (Box<Gated>, Sender<()>, Arc<Mutex<Vec<PathBuf>>>) {
    let (tx, rx) = channel();
    let seen = Arc::default();
    let remover = Box::new(Gated {
        release: Mutex::new(rx),
        seen: Arc::clone(&seen),
        panic_on,
    });
    (remover, tx, seen)
}

fn screens(fx: &Fixture, store: &Fixture) -> Screens {
    let cfg = Config {
        roots: vec![fx.root().to_path_buf()],
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

/// The confirm screen with every candidate marked, and a purge started on
/// `remover`. Records go to `records`, never to the user's own directory.
fn running(records: &Fixture, remover: Box<Gated>) -> (Tui, Fixture, Fixture) {
    let fx = Fixture::new();
    let store = Fixture::new();
    for n in 0..ITEMS {
        fx.file(&format!("app{n:02}/package.json"), b"{}");
        fx.file(&format!("app{n:02}/src/index.js"), b"1");
        fx.file(
            &format!("app{n:02}/node_modules/dep/blob.bin"),
            &[7u8; 2048],
        );
    }
    let mut tui = Tui::new(screens(&fx, &store)).with_manifest_dir(records.root().to_path_buf());
    let now = Instant::now();
    while tui.app().screen() != Screen::Candidates {
        tui.press(KeyPress::Enter, now);
    }
    tui.press(KeyPress::Char('a'), now);
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
    tui.purge(remover);
    (tui, fx, store)
}

fn frame(tui: &mut Tui) -> Buffer {
    let mut buf = Buffer::empty(AREA);
    tui.render(AREA, &mut buf);
    buf
}

fn lines(buf: &Buffer) -> Vec<String> {
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

fn text(buf: &Buffer) -> String {
    lines(buf).join("\n")
}

/// Rows of the list that begin with `glyph`.
fn rows_with(buf: &Buffer, glyph: char) -> usize {
    lines(buf)
        .iter()
        .filter(|l| l.trim_start().starts_with(glyph))
        .count()
}

/// Tick until `done` holds, or fail after a while. The thread has its own pace.
fn settle(tui: &mut Tui, what: &str, done: impl Fn(&mut Tui) -> bool) {
    let start = Instant::now();
    let mut at = start;
    while !done(tui) {
        assert!(
            start.elapsed() < Duration::from_secs(20),
            "still waiting for {what}:\n{}",
            text(&frame(tui))
        );
        std::thread::sleep(Duration::from_millis(2));
        at += Duration::from_millis(100);
        tui.tick(at);
    }
}

fn shows(what: &'static str) -> impl Fn(&mut Tui) -> bool {
    move |tui| text(&frame(tui)).contains(what)
}

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

#[test]
fn each_item_flips_from_pending_to_moved_as_it_is_released() {
    let records = Fixture::new();
    let (remover, release, _seen) = gated(None);
    let (mut tui, _fx, _store) = running(&records, remover);

    let before = frame(&mut tui);
    assert_eq!(rows_with(&before, '+'), 0, "{}", text(&before));
    assert_eq!(rows_with(&before, '·'), ITEMS, "{}", text(&before));
    assert!(text(&before).contains(&format!("Purging  0 of {ITEMS}")));

    release.send(()).expect("release the first item");
    settle(&mut tui, "the first item to show as moved", shows("1 of"));

    let after = frame(&mut tui);
    assert_eq!(rows_with(&after, '+'), 1, "{}", text(&after));
    assert_eq!(rows_with(&after, '·'), ITEMS - 1, "{}", text(&after));
    assert!(
        text(&after).contains(&format!("Purging  1 of {ITEMS}")),
        "{}",
        text(&after)
    );
}

#[test]
fn a_slow_item_is_visibly_not_a_hang() {
    let records = Fixture::new();
    let (remover, _release, _seen) = gated(None);
    let (mut tui, _fx, _store) = running(&records, remover);

    // The same instant twice: the clock on the running line cannot be what
    // differs, so the spinner has to be.
    let now = Instant::now();
    tui.tick(now);
    let first = frame(&mut tui);
    tui.tick(now);
    let second = frame(&mut tui);

    assert_ne!(
        text(&first),
        text(&second),
        "two ticks with an item in flight drew the same frame"
    );
}

#[test]
fn the_record_exists_after_the_first_item_and_lists_all_at_the_end() {
    let records = Fixture::new();
    let (remover, release, seen) = gated(None);
    let (mut tui, _fx, _store) = running(&records, remover);

    release.send(()).expect("release the first item");
    settle(&mut tui, "the first item", shows("1 of"));

    let files: Vec<_> = std::fs::read_dir(records.root())
        .expect("records dir")
        .map(|e| e.expect("entry").path())
        .collect();
    assert_eq!(files.len(), 1, "one record for one run: {files:?}");
    let first = std::fs::read_to_string(&files[0]).expect("read");
    let first_path = seen.lock().expect("lock")[0].display().to_string();
    assert!(first.contains(&first_path), "{first}");
    assert!(
        !first.contains(&seen.lock().expect("lock")[1].display().to_string()),
        "the record already names an item that has not moved:\n{first}"
    );

    for _ in 1..ITEMS {
        release.send(()).expect("release");
    }
    settle(&mut tui, "the result screen", |t| {
        t.app().screen() == Screen::Result
    });
    let last = std::fs::read_to_string(&files[0]).expect("read");
    for path in seen.lock().expect("lock").iter() {
        assert!(last.contains(&path.display().to_string()), "{last}");
    }
}

#[test]
fn every_key_during_the_run_says_so_and_changes_nothing() {
    let records = Fixture::new();
    let (remover, _release, _seen) = gated(None);
    let (mut tui, _fx, _store) = running(&records, remover);
    let now = Instant::now();
    let notice_row = AREA.height as usize - 2;

    let before = lines(&frame(&mut tui));
    for key in every_key() {
        let step = tui.press(key, now);
        assert_eq!(step, Step::Stay, "{key:?} did something during the run");
        let after = lines(&frame(&mut tui));
        assert!(
            after[notice_row].contains("A purge is running"),
            "{key:?} left no notice: {:?}",
            after[notice_row]
        );
        for (y, (a, b)) in before.iter().zip(&after).enumerate() {
            if y != notice_row {
                assert_eq!(a, b, "{key:?} changed row {y}");
            }
        }
    }
}

#[test]
fn the_result_is_reached_when_the_stream_closes_and_says_what_the_record_says() {
    let records = Fixture::new();
    let (remover, release, _seen) = gated(None);
    let (mut tui, _fx, _store) = running(&records, remover);

    for _ in 0..ITEMS {
        release.send(()).expect("release");
    }
    settle(&mut tui, "the result screen", |t| {
        t.app().screen() == Screen::Result
    });

    let manifest = tui.app().result().expect("a finished run has a record");
    assert_eq!(manifest.items.len(), ITEMS);
    let record = tui.record().expect("the record was written");
    assert_eq!(
        std::fs::read_to_string(record).expect("read"),
        manifest.render(),
        "the file and the record the screen holds disagree"
    );
    let moved = human(manifest.bytes_moved());
    let screen = text(&frame(&mut tui));
    assert!(
        screen.contains(&format!("Purged  ({ITEMS} items, {moved})")),
        "{screen}"
    );
}

#[test]
fn the_gauge_and_the_hold_are_empty_when_the_run_starts() {
    let records = Fixture::new();
    let (remover, _release, _seen) = gated(None);
    let (tui, _fx, _store) = running(&records, remover);

    let state = format!("{tui:?}");
    assert!(state.contains("held_at: None"), "{state}");
    assert!(state.contains("armed: false"), "{state}");
}

#[test]
fn a_worker_that_panics_ends_the_run_early_instead_of_hanging_the_loop() {
    let records = Fixture::new();
    let (remover, release, _seen) = gated(Some(2));
    let (mut tui, _fx, _store) = running(&records, remover);

    release.send(()).expect("release the first item");
    release.send(()).expect("release the second, which panics");
    settle(&mut tui, "the result screen", |t| {
        t.app().screen() == Screen::Result
    });

    let manifest = tui.app().result().expect("a record");
    assert_eq!(manifest.items.len(), 1, "only the first item completed");
    let screen = text(&frame(&mut tui));
    assert!(screen.contains("The run ended early"), "{screen}");
    assert!(screen.contains("1 of"), "{screen}");
    assert!(
        screen.contains("the remover fell over on item 2"),
        "{screen}"
    );
    let record = std::fs::read_to_string(tui.record().expect("written")).expect("read");
    assert!(record.contains("app"), "{record}");
}

#[test]
fn the_running_screen_draws_into_any_area_without_panicking() {
    let records = Fixture::new();
    let (remover, release, _seen) = gated(None);
    let (mut tui, _fx, _store) = running(&records, remover);
    release.send(()).expect("release the first item");
    settle(&mut tui, "the first item", shows("1 of"));

    for (w, h) in [(0, 0), (1, 1), (10, 3), (40, 5), (80, 24), (300, 90)] {
        let area = Rect::new(0, 0, w, h);
        let mut buf = Buffer::empty(area);
        tui.render(area, &mut buf);
    }
}

#[test]
fn q_on_the_result_screen_quits_on_the_first_press() {
    // The marks are still on the candidates screen behind it, and the run is
    // over: nothing is left to drop.
    let records = Fixture::new();
    let (remover, release, _seen) = gated(None);
    let (mut tui, _fx, _store) = running(&records, remover);
    for _ in 0..ITEMS {
        release.send(()).expect("release");
    }
    settle(&mut tui, "the result screen", |t| {
        t.app().screen() == Screen::Result
    });

    assert_eq!(tui.press(KeyPress::Char('q'), Instant::now()), Step::Quit);
}
