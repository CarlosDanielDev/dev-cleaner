//! `T`: the live switch (#166). It cycles the theme on every screen, saves the
//! choice, says so, and is refused on the two screens that remove things.

pub mod common;

use std::fs;
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use common::Fixture;
use dev_cleaner::config::Config;
use dev_cleaner::purge::Remover;
use dev_cleaner::scan::Progress;
use dev_cleaner::tui::palette::{Mode, Theme, ThemeName};
use dev_cleaner::tui::{
    Action, Effect, KeyPress, PURGE, Screen, Screens, Step, Tui, bindings, collect, footer,
};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;

const T: KeyPress = KeyPress::Char('T');

fn fixture() -> (Fixture, Fixture) {
    let fx = Fixture::new();
    let store = Fixture::new();
    for (name, bytes) in [("app", 4096usize), ("web", 9000)] {
        fx.file(&format!("{name}/package.json"), b"{}");
        fx.file(&format!("{name}/src/index.js"), b"console.log(1)");
        fx.file(
            &format!("{name}/node_modules/dep/blob.bin"),
            &vec![0xABu8; bytes],
        );
    }
    (fx, store)
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

/// A driver on `screen` that saves the theme to `file`, starting in neon.
fn driver(fx: &Fixture, store: &Fixture, screen: Screen, file: &std::path::Path) -> Tui {
    let mut tui = Tui::new(screens(fx, store))
        .with_theme(Theme::neon())
        .with_theme_file(Some(file.to_path_buf()));
    let now = Instant::now();
    while tui.app().screen() != screen {
        if tui.app().screen() == Screen::Candidates {
            tui.press(KeyPress::Char('a'), now);
        }
        tui.press(KeyPress::Enter, now);
    }
    tui
}

fn frame(tui: &mut Tui) -> Buffer {
    let area = Rect::new(0, 0, 100, 34);
    let mut buf = Buffer::empty(area);
    tui.render(area, &mut buf);
    buf
}

fn row(buf: &Buffer, y: u16) -> String {
    (0..buf.area.width).map(|x| buf[(x, y)].symbol()).collect()
}

fn text(tui: &mut Tui) -> String {
    let buf = frame(tui);
    (0..buf.area.height)
        .map(|y| row(&buf, y))
        .collect::<Vec<_>>()
        .join("\n")
}

/// The notice row: the second from the bottom.
fn notice(tui: &mut Tui) -> String {
    row(&frame(tui), 32).trim().to_string()
}

fn saved(file: &std::path::Path) -> Option<String> {
    fs::read_to_string(file).ok().map(|s| s.trim().to_string())
}

#[test]
fn t_is_a_global_navigation_key_with_a_label_and_nothing_destructive_about_it() {
    let hits: Vec<_> = bindings().iter().filter(|b| b.key == T).collect();
    assert_eq!(hits.len(), 1, "one binding for T");
    assert!(hits[0].screen.is_none(), "T works wherever a key can");
    assert_eq!(hits[0].action, Action::Theme);
    assert_eq!(hits[0].label, "theme");
    assert_eq!(hits[0].effect(), Effect::Navigate);
}

#[test]
fn t_is_listed_in_the_key_list_and_left_to_it_by_the_key_bar() {
    let (fx, store) = fixture();
    let dir = Fixture::new();
    let file = dir.root().join("theme");
    for screen in [Screen::Dashboard, Screen::Projects, Screen::Candidates] {
        assert!(
            !footer(screen, 200).contains("theme"),
            "{screen:?}: the bar keeps its room for the keys it had"
        );
        let mut tui = driver(&fx, &store, screen, &file);
        tui.press(KeyPress::Char('?'), Instant::now());
        let keys = text(&mut tui);
        assert!(
            keys.lines().any(|l| l.contains('T') && l.contains("theme")),
            "{screen:?}:\n{keys}"
        );
    }
}

#[test]
fn t_cycles_the_theme_on_every_screen_but_the_two_that_remove_things() {
    let (fx, store) = fixture();
    for screen in [
        Screen::Dashboard,
        Screen::Projects,
        Screen::Candidates,
        Screen::Review,
    ] {
        let dir = Fixture::new();
        let file = dir.root().join("state/theme");
        let mut tui = driver(&fx, &store, screen, &file);
        let now = Instant::now();
        assert_eq!(tui.theme().name(), ThemeName::Neon);

        assert_eq!(tui.press(T, now), Step::Stay);
        assert_eq!(tui.theme().name(), ThemeName::Matrix, "{screen:?}");
        assert_eq!(tui.app().screen(), screen, "{screen:?}: T went nowhere");
        assert_eq!(notice(&mut tui), "Theme: matrix (saved)", "{screen:?}");
        assert_eq!(saved(&file).as_deref(), Some("matrix"), "{screen:?}");
        assert!(
            text(&mut tui).contains("C:\\DEV-CLEANER\\"),
            "{screen:?}: the next frame is drawn in the new theme"
        );

        tui.press(T, now);
        assert_eq!(tui.theme().name(), ThemeName::Neon, "{screen:?}");
        assert_eq!(notice(&mut tui), "Theme: neon (saved)", "{screen:?}");
        assert_eq!(saved(&file).as_deref(), Some("neon"), "{screen:?}");
    }
}

#[test]
fn a_switch_keeps_the_colour_mode_and_the_marks() {
    let (fx, store) = fixture();
    let dir = Fixture::new();
    let mut tui = driver(&fx, &store, Screen::Candidates, &dir.root().join("theme"));
    tui.press(KeyPress::Char('a'), Instant::now());
    // The way row says what the plan will be built from: the marks.
    let built = |t: &mut Tui| {
        text(t)
            .lines()
            .find(|l| l.contains("built from the"))
            .map(|l| l[l.find("built from the").unwrap()..].trim().to_string())
            .expect("the way row names the marks")
    };
    let before = built(&mut tui);
    assert!(before.contains("1 marked"), "{before}");
    tui.press(T, Instant::now());
    assert_eq!(tui.theme().mode(), Mode::Truecolor);
    assert_eq!(built(&mut tui), before);
    tui.press(T, Instant::now());
    assert_eq!(built(&mut tui), before);
}

#[test]
fn the_switch_works_while_a_scan_runs() {
    let fx = Fixture::new();
    let dir = Fixture::new();
    let file = dir.root().join("theme");
    let now = Instant::now();
    let mut tui = Tui::starting(
        Screens::pending(vec![fx.root().to_path_buf()], fx.root().join("n.sqlite3")),
        Arc::new(Progress::default()),
        now,
    )
    .with_theme(Theme::neon())
    .with_theme_file(Some(file.clone()));
    assert_eq!(tui.press(T, now), Step::Stay);
    assert_eq!(tui.theme().name(), ThemeName::Matrix);
    assert_eq!(notice(&mut tui), "Theme: matrix (saved)");
    assert_eq!(saved(&file).as_deref(), Some("matrix"));
    assert!(tui.is_scanning(), "the scan went on");
}

#[test]
fn the_help_list_closes_on_any_key_and_t_is_no_exception() {
    let (fx, store) = fixture();
    let dir = Fixture::new();
    let file = dir.root().join("theme");
    let mut tui = driver(&fx, &store, Screen::Dashboard, &file);
    let now = Instant::now();
    tui.press(KeyPress::Char('?'), now);
    tui.press(T, now);
    assert_eq!(
        tui.theme().name(),
        ThemeName::Neon,
        "T only closed the list"
    );
    assert!(notice(&mut tui).contains("T was not applied"));
    assert_eq!(saved(&file), None);
}

#[test]
fn without_a_place_to_save_it_still_switches_and_says_it_is_not_kept() {
    let (fx, store) = fixture();
    let mut tui = Tui::new(screens(&fx, &store)).with_theme(Theme::neon());
    tui.press(T, Instant::now());
    assert_eq!(tui.theme().name(), ThemeName::Matrix);
    assert_eq!(notice(&mut tui), "Theme: matrix (not saved)");
}

#[test]
fn a_state_directory_that_cannot_be_written_still_switches_for_the_session() {
    use std::os::unix::fs::PermissionsExt;
    // SAFETY: geteuid has no preconditions.
    if unsafe { libc::geteuid() } == 0 {
        return;
    }
    let (fx, store) = fixture();
    let dir = Fixture::new();
    let ro = dir.root().join("ro");
    fs::create_dir_all(&ro).unwrap();
    fs::set_permissions(&ro, fs::Permissions::from_mode(0o555)).unwrap();
    let mut tui = driver(&fx, &store, Screen::Dashboard, &ro.join("theme"));
    tui.press(T, Instant::now());
    assert_eq!(tui.theme().name(), ThemeName::Matrix);
    let said = notice(&mut tui);
    assert!(
        said.starts_with("Theme: matrix (not saved: "),
        "the notice says what failed: {said}"
    );
    tui.press(T, Instant::now());
    assert_eq!(
        tui.theme().name(),
        ThemeName::Neon,
        "and it goes on switching"
    );
    fs::set_permissions(&ro, fs::Permissions::from_mode(0o755)).unwrap();
}

// --- the screens that remove things -------------------------------------

#[test]
fn t_is_refused_on_confirm_with_the_reason_and_changes_nothing() {
    let (fx, store) = fixture();
    let dir = Fixture::new();
    let file = dir.root().join("theme");
    let mut tui = driver(&fx, &store, Screen::Confirm, &file);
    assert_eq!(tui.press(T, Instant::now()), Step::Stay);
    assert_eq!(tui.theme().name(), ThemeName::Neon);
    assert_eq!(notice(&mut tui), "The theme cannot change during a purge.");
    assert_eq!(saved(&file), None, "nothing was written");
    assert_eq!(tui.app().screen(), Screen::Confirm);
}

/// The step every press of `x` returned, 50 ms apart, with `extra` pressed
/// between two of them.
fn hold(tui: &mut Tui, extra: Option<(usize, KeyPress)>) -> Vec<Step> {
    let t0 = Instant::now();
    let mut steps = Vec::new();
    for i in 0..80usize {
        let at = t0 + Duration::from_millis(50 * i as u64);
        if let Some((when, key)) = extra
            && when == i
        {
            assert_eq!(tui.press(key, at), Step::Stay);
        }
        let step = tui.press(PURGE, at);
        steps.push(step);
        if step == Step::Purge {
            break;
        }
    }
    steps
}

#[test]
fn a_theme_key_in_the_middle_of_the_hold_does_not_move_it() {
    let (fx, store) = fixture();
    let dir = Fixture::new();
    let plain = hold(
        &mut driver(&fx, &store, Screen::Confirm, &dir.root().join("a")),
        None,
    );
    assert_eq!(plain.last(), Some(&Step::Purge), "the plain hold completes");
    for at in [1, 5, plain.len() / 2, plain.len() - 1] {
        let mut tui = driver(&fx, &store, Screen::Confirm, &dir.root().join("b"));
        let with_t = hold(&mut tui, Some((at, T)));
        assert_eq!(
            with_t,
            plain,
            "T before press {at} moved the hold ({} presses against {})",
            with_t.len(),
            plain.len()
        );
        assert_eq!(tui.theme().name(), ThemeName::Neon);
    }
}

/// Holds every item until the sender goes.
struct Held(Mutex<Receiver<()>>);

impl Remover for Held {
    fn remove(&self, path: &std::path::Path) -> std::io::Result<PathBuf> {
        let _ = self.0.lock().unwrap().recv_timeout(Duration::from_secs(20));
        Ok(PathBuf::from("/Users/test/.Trash").join(path.file_name().unwrap()))
    }
}

#[test]
fn t_is_refused_while_the_purge_runs_and_the_run_is_not_disturbed() {
    let (fx, store) = fixture();
    let dir = Fixture::new();
    let records = Fixture::new();
    let file = dir.root().join("theme");
    let mut tui =
        driver(&fx, &store, Screen::Confirm, &file).with_manifest_dir(records.root().to_path_buf());
    assert_eq!(hold(&mut tui, None).last(), Some(&Step::Purge));
    let (tx, rx): (Sender<()>, Receiver<()>) = channel();
    tui.purge(Box::new(Held(Mutex::new(rx))));
    assert_eq!(tui.press(T, Instant::now()), Step::Stay);
    assert_eq!(tui.theme().name(), ThemeName::Neon);
    assert_eq!(notice(&mut tui), "The theme cannot change during a purge.");
    assert_eq!(saved(&file), None);
    drop(tx);
}

#[test]
fn the_notice_a_theme_switch_leaves_is_a_notice_like_the_others_and_lets_go() {
    let (fx, store) = fixture();
    let dir = Fixture::new();
    let mut tui = driver(&fx, &store, Screen::Dashboard, &dir.root().join("theme"));
    let now = Instant::now();
    tui.press(T, now);
    assert_eq!(notice(&mut tui), "Theme: matrix (saved)");
    tui.tick(now + Duration::from_secs(4));
    assert_eq!(notice(&mut tui), "");
}

#[test]
fn an_announced_notice_is_shown_when_the_interface_opens() {
    let (fx, store) = fixture();
    let mut tui = Tui::new(screens(&fx, &store)).with_theme(Theme::neon());
    tui.announce(
        "Saved theme \"x\" is unknown; using neon".to_string(),
        Instant::now(),
    );
    assert_eq!(notice(&mut tui), "Saved theme \"x\" is unknown; using neon");
}

#[test]
fn a_startup_notice_is_still_there_when_the_scan_finishes() {
    let (fx, store) = fixture();
    let now = Instant::now();
    let mut tui = Tui::starting(
        Screens::pending(
            vec![fx.root().to_path_buf()],
            store.root().join("n.sqlite3"),
        ),
        Arc::new(Progress::default()),
        now,
    )
    .with_theme(Theme::neon());
    tui.announce("Saved theme \"x\" is unknown; using neon".to_string(), now);
    tui.finish_scan(screens(&fx, &store), now + Duration::from_secs(1));
    let said = notice(&mut tui);
    assert!(
        said.starts_with("Saved theme \"x\" is unknown; using neon")
            && said.contains("Scan finished"),
        "{said}"
    );
}
