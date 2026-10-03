//! Every screen of the neon look, pinned cell by cell in all three colour modes.
//!
//! These goldens were taken from `main` before the theme registry (#166) and the
//! matrix theme existed: the symbol and style of every cell of every screen, in
//! truecolor, 256 colours and `NO_COLOR`. The registry must leave neon exactly
//! as it was, so a drift here is a bug in the change, not a golden to refresh.
//! Set `DEV_CLEANER_UPDATE_SNAPSHOTS=1` to rewrite them on purpose.

pub mod common;

use std::fmt::Write;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, channel};
use std::sync::{Mutex, mpsc::Sender};
use std::time::{Duration, Instant};

use common::Fixture;
use common::purge::{candidate, confirmed};
use dev_cleaner::config::Config;
use dev_cleaner::purge::{Remover, execute};
use dev_cleaner::scan::Progress;
use dev_cleaner::tui::palette::Theme;
use dev_cleaner::tui::{KeyPress, PURGE, Report, Screen, Screens, Step, Tui, collect};
use dev_cleaner::volume::Volume;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;

const MB: u64 = 1024 * 1024;

/// The length of the scratch root the goldens were taken with, on `main`:
/// `/var/folders/jl/6mh_pdz17t315ptjq74fxdg80000gn/T/.tmpXXXXXX`, 59 characters. A path is
/// drawn, elided and measured against, so the fixture's root has this length
/// wherever the temp directory is; only its letters differ, and `canon` masks
/// those.
const ROOT_LEN: usize = 59;

fn fixture() -> (Fixture, Fixture) {
    let fx = Fixture::with_path_len(ROOT_LEN);
    let store = Fixture::new();
    for (name, bytes) in [("app", 4096usize), ("web", 9000), ("api", 1500)] {
        fx.file(&format!("{name}/package.json"), b"{}");
        fx.file(&format!("{name}/src/index.js"), b"console.log(1)");
        fx.file(
            &format!("{name}/node_modules/dep/blob.bin"),
            &vec![0xABu8; bytes],
        );
    }
    fx.file("lib/Cargo.toml", b"[package]\nname = \"lib\"\n");
    fx.file("lib/src/lib.rs", b"// lib");
    fx.file("lib/target/debug/blob.bin", &vec![0xCDu8; 20000]);
    (fx, store)
}

fn screens(fx: &Fixture, store: &Fixture) -> Screens {
    let cfg = Config {
        roots: vec![fx.root().to_path_buf()],
        caches: Vec::new(),
        denylist: Vec::new(),
    };
    let mut screens = collect(
        &[fx.root().to_path_buf()],
        &cfg,
        fx.root(),
        &store.root().join("history.sqlite3"),
    );
    // The disk and the clock are the machine's, not the fixture's.
    screens.dashboard.volume = Some(Volume {
        total: 500 * 1024 * MB,
        free: 80 * 1024 * MB,
    });
    screens.dashboard.analysed.elapsed = Duration::from_millis(1500);
    screens
}

/// A driver on `screen`; `mark` marks everything on the way through candidates.
fn driver(fx: &Fixture, store: &Fixture, screen: Screen, theme: Theme, mark: bool) -> Tui {
    let mut tui = Tui::new(screens(fx, store)).with_theme(theme);
    let now = Instant::now();
    while tui.app().screen() != screen {
        if tui.app().screen() == Screen::Candidates {
            tui.press(KeyPress::Char('a'), now);
        }
        tui.press(KeyPress::Enter, now);
    }
    if mark && screen == Screen::Candidates {
        tui.press(KeyPress::Char('a'), now);
    }
    tui
}

/// Every cell as text, then a line per row of the styles in runs. The scratch
/// paths are masked so a run on another machine agrees.
fn dump(buf: &Buffer, mask: &[String]) -> String {
    let area = buf.area;
    let mut out = String::new();
    for y in 0..area.height {
        let mut text: String = (0..area.width).map(|x| buf[(x, y)].symbol()).collect();
        for m in mask {
            text = text.replace(m.as_str(), &"R".repeat(m.chars().count()));
        }
        text = mask_tmp(&text);
        writeln!(out, "{y:>2}|{text}|").unwrap();
        let mut runs: Vec<String> = Vec::new();
        let mut run: Option<(u16, String)> = None;
        for x in 0..area.width {
            let c = &buf[(x, y)];
            let s = format!("{:?}/{:?}/{:?}", c.fg, c.bg, c.modifier);
            match &run {
                Some((_, prev)) if *prev == s => {}
                _ => {
                    if let Some((from, prev)) = run.take() {
                        runs.push(format!("{from}-{}:{prev}", x - 1));
                    }
                    run = Some((x, s));
                }
            }
        }
        if let Some((from, prev)) = run {
            runs.push(format!("{from}-{}:{prev}", area.width - 1));
        }
        writeln!(out, "  {}", runs.join(" ")).unwrap();
    }
    out
}

/// Both sides of a comparison, with the machine's own letters masked: in every
/// text row, a word that holds a scratch directory (`.tmp` and the random name
/// after it) has everything before it replaced by `#` and the name by `X`.
/// Lengths are kept, so a layout that moved still shows. The goldens on disk are
/// never touched: they stay what `main` drew.
fn canon(doc: &str) -> String {
    doc.lines()
        .map(|line| {
            let is_text = line.split_once('|').is_some_and(|(n, _)| {
                !n.is_empty() && n.trim().chars().all(|c| c.is_ascii_digit())
            });
            if !is_text {
                return line.to_string();
            }
            let chars: Vec<char> = line.chars().collect();
            let mut out = chars.clone();
            let mut at = 0;
            while let Some(i) = (at..chars.len().saturating_sub(3))
                .find(|&i| chars[i..i + 4] == ['.', 't', 'm', 'p'])
            {
                let mut start = i;
                while start > 0 && !matches!(chars[start - 1], ' ' | '|') {
                    start -= 1;
                }
                for c in &mut out[start..i] {
                    *c = '#';
                }
                let mut end = i + 4;
                while end < chars.len() && chars[end].is_ascii_alphanumeric() {
                    out[end] = 'X';
                    end += 1;
                }
                at = end;
            }
            out.into_iter().collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// `.tmpAb12Cd`: the random part of a scratch directory, wherever the path was
/// elided to.
fn mask_tmp(text: &str) -> String {
    let mut out = String::new();
    let mut rest = text;
    while let Some(at) = rest.find(".tmp") {
        let (head, tail) = rest.split_at(at + 4);
        out.push_str(head);
        let n = tail
            .chars()
            .take(6)
            .take_while(|c| c.is_ascii_alphanumeric())
            .count();
        out.push_str(&"X".repeat(n));
        rest = &tail[n..];
    }
    out.push_str(rest);
    out
}

fn masks(fx: &Fixture) -> Vec<String> {
    let root = fx.root().display().to_string();
    let mut m = vec![root.clone()];
    if let Ok(canon) = fx.root().canonicalize() {
        let canon = canon.display().to_string();
        if canon != root {
            m.insert(0, canon);
        }
    }
    m
}

fn frame(tui: &mut Tui, cols: u16, rows: u16) -> Buffer {
    let area = Rect::new(0, 0, cols, rows);
    let mut buf = Buffer::empty(area);
    tui.render(area, &mut buf);
    buf
}

/// A remover that holds every item until its sender goes, so the running
/// screen can be drawn at its very first moment.
struct Held(Mutex<Receiver<()>>);

impl Remover for Held {
    fn remove(&self, path: &std::path::Path) -> std::io::Result<PathBuf> {
        let _ = self.0.lock().unwrap().recv_timeout(Duration::from_secs(20));
        Ok(PathBuf::from("/Users/test/.Trash").join(path.file_name().unwrap()))
    }
}

fn running(fx: &Fixture, store: &Fixture, records: &Fixture, theme: Theme) -> (Tui, Sender<()>) {
    let mut tui = Tui::new(screens(fx, store))
        .with_theme(theme)
        .with_manifest_dir(records.root().to_path_buf());
    let now = Instant::now();
    while tui.app().screen() != Screen::Candidates {
        tui.press(KeyPress::Enter, now);
    }
    tui.press(KeyPress::Tab, now);
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
    let (tx, rx) = channel();
    tui.purge(Box::new(Held(Mutex::new(rx))));
    (tui, tx)
}

/// Every screen the interface draws, in `theme`, as one document.
fn scenes(theme: Theme) -> String {
    let (fx, store) = fixture();
    let mask = masks(&fx);
    let mut out = String::new();
    let mut put = |title: &str, buf: &Buffer| {
        writeln!(out, "## {title}").unwrap();
        out += &dump(buf, &mask);
    };
    for (cols, rows) in [(100, 34), (80, 24)] {
        for screen in [
            Screen::Dashboard,
            Screen::Projects,
            Screen::Candidates,
            Screen::Review,
            Screen::Confirm,
        ] {
            let mut tui = driver(&fx, &store, screen, theme, false);
            put(
                &format!("{screen:?} {cols}x{rows}"),
                &frame(&mut tui, cols, rows),
            );
        }
        let mut tui = driver(&fx, &store, Screen::Candidates, theme, true);
        put(
            &format!("Candidates marked {cols}x{rows}"),
            &frame(&mut tui, cols, rows),
        );
        let mut tui = driver(&fx, &store, Screen::Dashboard, theme, false);
        tui.press(KeyPress::Char('?'), Instant::now());
        put(&format!("Keys {cols}x{rows}"), &frame(&mut tui, cols, rows));
        let mut tui = Tui::starting(
            Screens::pending(
                vec![fx.root().to_path_buf()],
                fx.root().join("none.sqlite3"),
            ),
            Arc::new(Progress::default()),
            Instant::now(),
        )
        .with_theme(theme);
        put(&format!("Scan {cols}x{rows}"), &frame(&mut tui, cols, rows));
    }
    let mut tui = driver(&fx, &store, Screen::Dashboard, theme, false);
    put("Too small 79x23", &frame(&mut tui, 79, 23));
    let mut tui = driver(&fx, &store, Screen::Dashboard, theme, false);
    put("Wide 160x40", &frame(&mut tui, 160, 40));

    let records = Fixture::new();
    let (mut tui, release) = running(&fx, &store, &records, theme);
    put("Running 100x34", &frame(&mut tui, 100, 34));
    drop(release);

    let manifest = execute(
        confirmed(vec![
            candidate("/p/a/node_modules", 100 * MB),
            candidate("/p/b/target", 200 * MB),
        ]),
        &common::purge::Recorder {
            fail_on: Some("target"),
            ..Default::default()
        },
    );
    let area = Rect::new(0, 0, 100, 30);
    let mut buf = Buffer::empty(area);
    buf.set_style(area, theme.ground);
    Report::new().render(
        &theme,
        &manifest,
        Some(std::path::Path::new(
            "/Users/test/.local/state/dev-cleaner/manifests/purge-1.md",
        )),
        area,
        &mut buf,
    );
    put("Result 100x30", &buf);
    out
}

fn check(name: &str, theme: Theme) {
    let got = scenes(theme);
    let path = format!("{}/tests/snapshots/{name}.txt", env!("CARGO_MANIFEST_DIR"));
    if std::env::var_os("DEV_CLEANER_UPDATE_SNAPSHOTS").is_some() {
        std::fs::write(&path, &got).unwrap();
        return;
    }
    let want =
        std::fs::read_to_string(&path).expect("golden missing: set DEV_CLEANER_UPDATE_SNAPSHOTS=1");
    let (got, want) = (canon(&got), canon(&want));
    if got != want {
        let first = got
            .lines()
            .zip(want.lines())
            .position(|(a, b)| a != b)
            .unwrap_or(0);
        panic!(
            "{name} drifted from its golden at line {}:\n got: {}\nwant: {}",
            first + 1,
            got.lines().nth(first).unwrap_or(""),
            want.lines().nth(first).unwrap_or("")
        );
    }
}

#[test]
fn neon_in_truecolor_is_unchanged() {
    check("neon_truecolor", Theme::neon());
}

#[test]
fn neon_in_256_colours_is_unchanged() {
    check("neon_256", Theme::ansi());
}

#[test]
fn neon_without_colour_is_unchanged() {
    check("neon_nocolor", Theme::mono());
}
