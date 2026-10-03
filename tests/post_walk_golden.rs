//! The post-walk phase is speed work: nothing it prints may change.
//!
//! One fixture (nested projects, hardlinks across projects, symlinks, a dangling
//! link, a denylist with a plain directory and a symlinked entry) is scanned by
//! the binary and collected into the TUI's screens. The text of both is pinned
//! under `tests/golden/`, taken on `main` before the post-walk phase was
//! rewritten. A diff here means a number or a path moved.
//!
//! Regenerate deliberately with `UPDATE_GOLDEN=1 cargo test --test post_walk_golden`.

mod common;

use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use common::Fixture;
use dev_cleaner::config::Config;
use dev_cleaner::tui::collect;

/// The scanned tree. Returns the denylist entries, which live inside it.
fn build(fx: &Fixture) -> Vec<PathBuf> {
    let root = fx.root();

    // An app with a nested project, a nested node_modules and a dependency
    // that carries its own package.json.
    fx.file("app/package.json", b"{}");
    fx.file("app/src/index.js", b"console.log(1)");
    fx.file("app/node_modules/x/package.json", b"{}");
    fx.file("app/node_modules/x/lib.js", b"module.exports = 1");
    fx.file("app/node_modules/x/node_modules/y/index.js", b"y");
    fx.file("app/ios/Package.swift", b"// swift");
    fx.file("app/ios/Pods/p/p.h", b"#define P 1");
    fx.file("app/ios/Sources/main.swift", b"print(1)");

    // A sibling whose name shares a string prefix with `app`, plus a cargo
    // target holding hardlinks, and a `target` below `src`.
    fx.file("app2/Cargo.toml", b"[package]");
    fx.file("app2/src/main.rs", b"fn main() {}");
    fx.file("app2/src/target/stray.o", b"stray");
    let first = fx.file("app2/target/debug/deps/a", b"object code");
    let second = root.join("app2/target/debug/deps/b");
    fs::hard_link(&first, &second).expect("hardlink within a target");
    let shared = fx.file("app2/target/debug/shared", b"shared blob");
    fs::hard_link(&shared, root.join("app/node_modules/x/shared")).expect("hardlink across");

    // A project nobody touched in a long time.
    fx.file("old/go.mod", b"module old");
    fx.file("old/main.go", b"package main");
    let stale = std::time::SystemTime::now() - std::time::Duration::from_secs(400 * 86_400);
    for f in ["old/go.mod", "old/main.go"] {
        fs::File::options()
            .write(true)
            .open(root.join(f))
            .expect("open")
            .set_modified(stale)
            .expect("backdate");
    }

    // Loose files outside any project.
    fx.file("notes/todo.txt", b"todo");

    // Symlinks: to a directory, to a file, and one that dangles.
    std::os::unix::fs::symlink(root.join("app/src"), root.join("app/link-dir")).expect("link");
    std::os::unix::fs::symlink(root.join("app/package.json"), root.join("app/link-file"))
        .expect("link");
    std::os::unix::fs::symlink(root.join("nowhere"), root.join("app/dangling")).expect("link");

    // Denied: a directory holding a project, and one reached only through a
    // symlink named in the denylist.
    fx.file("denied/Cargo.toml", b"[package]");
    fx.file("denied/target/x", b"x");
    fx.file("hidden/Cargo.toml", b"[package]");
    fx.file("hidden/target/y", b"y");
    std::os::unix::fs::symlink(root.join("hidden"), root.join("alias")).expect("link");

    vec![root.join("denied"), root.join("alias")]
}

/// Make paths independent of where the tempdir landed, and of whether the
/// tempdir sits behind a symlink (`/var` on macOS).
fn scrub(text: &str, root: &Path) -> String {
    let mut out = text.replace(&root.display().to_string(), "<ROOT>");
    if let Ok(real) = root.canonicalize() {
        out = out.replace(&real.display().to_string(), "<ROOT>");
    }
    out
}

fn screens_text(fx: &Fixture, denylist: &[PathBuf]) -> String {
    let home = Fixture::new();
    let cfg = Config {
        roots: vec![fx.root().to_path_buf()],
        caches: Vec::new(),
        denylist: denylist.to_vec(),
    };
    let screens = collect(
        &[fx.root().to_path_buf()],
        &cfg,
        home.root(),
        &home.root().join("history.db"),
    );

    let d = &screens.dashboard;
    let mut out = String::new();
    // Not the volume (free space moves) nor the elapsed time.
    writeln!(out, "reclaimable: {}", d.reclaimable).unwrap();
    writeln!(out, "trend: {:#?}", d.trend).unwrap();
    writeln!(out, "consumers: {:#?}", d.consumers).unwrap();
    writeln!(out, "now: {:#?}", d.now).unwrap();
    writeln!(out, "history: {:?}", d.history).unwrap();
    writeln!(out, "groups: {:#?}", d.groups).unwrap();
    writeln!(out, "aim: {:#?}", d.aim).unwrap();
    let a = &d.analysed;
    writeln!(
        out,
        "analysed: projects={} with_rebuild={} entries={} measured={} worktrees={} repos={}",
        a.projects, a.with_rebuild, a.entries, a.measured, a.worktrees, a.repos
    )
    .unwrap();
    writeln!(out, "projects: {:#?}", screens.projects.rows()).unwrap();
    writeln!(out, "offerable: {:#?}", screens.candidates.selectable()).unwrap();
    writeln!(out, "blocked: {:#?}", screens.candidates.blocked()).unwrap();
    scrub(&out, fx.root())
}

fn scan_text(fx: &Fixture, denylist: &[PathBuf]) -> String {
    let home = Fixture::new();
    let list = denylist
        .iter()
        .map(|p| format!("{:?}", p.display().to_string()))
        .collect::<Vec<_>>()
        .join(", ");
    home.file(
        ".config/dev-cleaner/config.toml",
        format!("caches = []\ndenylist = [{list}]\n").as_bytes(),
    );
    let out = Command::new(env!("CARGO_BIN_EXE_dev-cleaner"))
        .arg("scan")
        .arg(fx.root())
        .env("HOME", home.root())
        .output()
        .expect("run scan");
    assert!(
        out.status.success(),
        "scan failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8(out.stdout).expect("utf8");
    // The first line carries the elapsed time.
    let rest = stdout.split_once('\n').map_or("", |(_, rest)| rest);
    scrub(rest, fx.root())
}

fn check(name: &str, actual: &str) {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/golden")
        .join(name);
    if std::env::var_os("UPDATE_GOLDEN").is_some() {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, actual).unwrap();
        return;
    }
    let expected = fs::read_to_string(&path).expect("golden file missing; see module docs");
    assert_eq!(actual, expected, "{name} changed");
}

#[test]
fn scan_output_is_unchanged() {
    let fx = Fixture::new();
    let denylist = build(&fx);
    check("scan_stdout.txt", &scan_text(&fx, &denylist));
}

#[test]
fn tui_screens_are_unchanged() {
    let fx = Fixture::new();
    let denylist = build(&fx);
    check("tui_screens.txt", &screens_text(&fx, &denylist));
}

#[test]
fn scan_output_without_a_denylist_is_unchanged() {
    let fx = Fixture::new();
    build(&fx);
    check("scan_stdout_open.txt", &scan_text(&fx, &[]));
}

#[test]
fn tui_screens_without_a_denylist_are_unchanged() {
    let fx = Fixture::new();
    build(&fx);
    check("tui_screens_open.txt", &screens_text(&fx, &[]));
}
