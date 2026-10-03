//! A project the tool has not touched must not look touched because the tool
//! looked at it.
//!
//! The dirty-tree guard asks git for the status of every candidate, and git
//! rewrites `.git/index` when its stat cache is stale. The walk reads hidden
//! directories, so a freshly written index must not count as someone working on
//! the project.

mod common;

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use common::Fixture;
use dev_cleaner::classify::Activity;
use dev_cleaner::config::Config;
use dev_cleaner::tui::collect;

const DAY: Duration = Duration::from_secs(86_400);

fn set_mtime(path: &Path, when: SystemTime) {
    std::fs::File::open(path)
        .expect("open")
        .set_modified(when)
        .expect("set mtime");
}

fn backdate(dir: &Path, when: SystemTime) {
    for entry in std::fs::read_dir(dir).expect("read_dir") {
        let path = entry.expect("entry").path();
        if path.is_dir() {
            backdate(&path, when);
        } else {
            set_mtime(&path, when);
        }
    }
}

/// A project idle for 200 days, every commit pushed, with a build directory the
/// guards clear. Everything under it is dated 200 days ago, `.git` included, and
/// no `git status` has run since: the index describes the files as they were
/// when committed, so the guard's `git status` finds its stat cache stale.
fn dead_project(fx: &Fixture) -> PathBuf {
    fx.file("old/.gitignore", b"node_modules\n");
    fx.file("old/package.json", b"{}");
    fx.file("old/src/index.js", b"console.log(1)");
    fx.git_repo("old", 200);
    fx.mark_pushed("old");
    fx.file("old/node_modules/dep/blob.bin", &[0xABu8; 2048]);
    backdate(&fx.root().join("old"), SystemTime::now() - 200 * DAY);
    fx.root().join("old")
}

fn activity_of(fx: &Fixture, store: &Fixture, root: &Path) -> Activity {
    let cfg = Config {
        roots: vec![fx.root().to_path_buf()],
        caches: Vec::new(),
        denylist: Vec::new(),
    };
    let db = store.root().join("history.sqlite3");
    let screens = collect(&cfg.roots, &cfg, fx.root(), &db);
    screens
        .projects
        .rows()
        .iter()
        .find(|p| p.path == root)
        .expect("the project is listed")
        .activity
}

#[test]
fn a_scan_does_not_turn_a_dead_project_active_for_the_next_one() {
    let fx = Fixture::new();
    let store = Fixture::new();
    let root = dead_project(&fx);

    let first = activity_of(&fx, &store, &root);
    let second = activity_of(&fx, &store, &root);

    assert_eq!(first, Activity::Dead, "the fixture is a dead project");
    assert_eq!(
        second, first,
        "the first scan changed what the second reads"
    );
}

#[test]
fn a_freshly_written_git_index_is_not_evidence_of_work() {
    let fx = Fixture::new();
    let store = Fixture::new();
    let root = dead_project(&fx);

    // Anything that touches `.git`, not only the guard: a fetch, an editor's
    // git integration, a `git gc`.
    set_mtime(&root.join(".git/index"), SystemTime::now());
    set_mtime(&root.join(".git/HEAD"), SystemTime::now());

    assert_eq!(activity_of(&fx, &store, &root), Activity::Dead);
}

#[test]
fn the_scan_command_does_not_date_a_project_by_its_git_index_either() {
    let fx = Fixture::new();
    let home = Fixture::new();
    let root = dead_project(&fx);
    set_mtime(&root.join(".git/index"), SystemTime::now());

    let out = std::process::Command::new(env!("CARGO_BIN_EXE_dev-cleaner"))
        .args(["scan"])
        .arg(fx.root())
        .env("HOME", home.root())
        .output()
        .expect("run dev-cleaner scan");
    let text = String::from_utf8_lossy(&out.stdout);

    assert!(out.status.success(), "scan failed: {text}");
    let dead = text
        .lines()
        .find(|l| l.trim_start().starts_with("dead"))
        .expect("the activity block names dead projects");
    assert!(dead.split_whitespace().nth(1) == Some("1"), "{dead}");
}
