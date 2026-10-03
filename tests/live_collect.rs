//! A scan that can be stopped, and what it leaves behind when it is not.

mod common;

use std::path::PathBuf;
use std::sync::Arc;

use common::Fixture;
use dev_cleaner::candidates::from_scan_with;
use dev_cleaner::config::Config;
use dev_cleaner::safety::Guards;
use dev_cleaner::scan::{Phase, Progress, Walker};
use dev_cleaner::store::{Store, read_baseline};
use dev_cleaner::tui::scan_with;

fn project(fx: &Fixture, name: &str) {
    fx.file(&format!("{name}/package.json"), b"{}");
    fx.file(&format!("{name}/src/index.js"), b"console.log(1)");
    fx.file(&format!("{name}/node_modules/dep/blob.bin"), &[7u8; 4096]);
}

fn cfg(fx: &Fixture) -> Config {
    Config {
        roots: vec![fx.root().to_path_buf()],
        caches: Vec::new(),
        denylist: Vec::new(),
    }
}

fn roots(fx: &Fixture) -> Vec<PathBuf> {
    vec![fx.root().to_path_buf()]
}

#[test]
fn a_finished_scan_is_stored_with_its_shape_and_the_next_one_starts_from_it() {
    let fx = Fixture::new();
    let store = Fixture::new();
    project(&fx, "app");
    project(&fx, "lib");
    let db = store.root().join("history.sqlite3");

    let first = Arc::new(Progress::default());
    let screens = scan_with(&roots(&fx), &cfg(&fx), fx.root(), &db, &first);
    assert!(screens.is_some(), "nobody cancelled it");
    assert!(first.baseline().is_none(), "nothing came before the first");

    let base = read_baseline(&db, &roots(&fx)).expect("the finished scan is a baseline");
    assert_eq!(base.entries, 6, "two projects of three files");
    assert!(base.children.contains_key(&fx.root().join("app")));
    assert!(base.children.contains_key(&fx.root().join("lib")));

    let second = Arc::new(Progress::default());
    scan_with(&roots(&fx), &cfg(&fx), fx.root(), &db, &second).expect("finished");
    let seen = second.baseline().expect("the second run knows the first");
    assert_eq!(seen.entries, 6);
    assert_eq!(
        second.read().phase,
        Phase::Saving,
        "it ended in the last phase"
    );
}

#[test]
fn a_scan_stopped_before_it_starts_writes_nothing_and_builds_nothing() {
    let fx = Fixture::new();
    let store = Fixture::new();
    project(&fx, "app");
    let db = store.root().join("history.sqlite3");
    // One finished scan, so there is a store to compare with.
    scan_with(
        &roots(&fx),
        &cfg(&fx),
        fx.root(),
        &db,
        &Arc::new(Progress::default()),
    )
    .expect("finished");
    let before = std::fs::read(&db).expect("db");
    let ids = Store::open(&db).expect("open").scan_ids().expect("ids");

    let stopped = Arc::new(Progress::default());
    stopped.cancel();
    let got = scan_with(&roots(&fx), &cfg(&fx), fx.root(), &db, &stopped);

    assert!(
        got.is_none(),
        "a cancelled scan builds no screens, so no plan"
    );
    assert_eq!(
        Store::open(&db).expect("open").scan_ids().expect("ids"),
        ids,
        "no new scan row"
    );
    assert_eq!(
        std::fs::read(&db).expect("db"),
        before,
        "the file is byte-equal"
    );
}

#[test]
fn a_scan_stopped_halfway_through_the_walk_writes_nothing() {
    let fx = Fixture::new();
    let store = Fixture::new();
    for i in 0..300 {
        fx.file(&format!("p{i}/package.json"), b"{}");
    }
    let db = store.root().join("history.sqlite3");
    let progress = Arc::new(Progress::default());

    let watcher = {
        let progress = Arc::clone(&progress);
        std::thread::spawn(move || {
            while progress.read().entries < 20 {
                std::thread::yield_now();
            }
            progress.cancel();
        })
    };
    let got = scan_with(&roots(&fx), &cfg(&fx), fx.root(), &db, &progress);
    watcher.join().expect("watcher");

    assert!(got.is_none());
    assert!(
        Store::open(&db)
            .expect("open")
            .scan_ids()
            .expect("ids")
            .is_empty(),
        "an interrupted scan is not a scan"
    );
}

#[test]
fn the_denylist_is_applied_by_the_walk_and_changes_nothing_downstream() {
    let fx = Fixture::new();
    let store = Fixture::new();
    project(&fx, "app");
    project(&fx, "private");
    let db = store.root().join("history.sqlite3");
    let mut config = cfg(&fx);
    config.denylist = vec![fx.root().join("private")];

    let progress = Arc::new(Progress::default());
    let screens = scan_with(&roots(&fx), &config, fx.root(), &db, &progress).expect("finished");

    assert_eq!(
        screens.projects.rows().len(),
        1,
        "the denied project is not on the table"
    );
    assert_eq!(
        progress.read().entries,
        3,
        "and its files were never counted"
    );
}

#[test]
fn guarding_the_artifacts_reports_each_one_and_stops_when_told() {
    let fx = Fixture::new();
    project(&fx, "a");
    project(&fx, "b");
    project(&fx, "c");
    let files = Walker::new([fx.root()]).walk().files;
    let guards = Guards::new(roots(&fx), Vec::new());

    let mut seen = Vec::new();
    let built = from_scan_with(&files, &guards, |done, total| {
        seen.push((done, total));
        true
    })
    .expect("not stopped");
    assert_eq!(seen, [(1, 3), (2, 3), (3, 3)]);
    assert_eq!(built.candidates.len() + built.rejected.len(), 3);

    let mut calls = 0;
    let stopped = from_scan_with(&files, &guards, |_, _| {
        calls += 1;
        calls < 2
    });
    assert!(stopped.is_none(), "told to stop, it builds nothing");
    assert_eq!(calls, 2, "and stops there");
}
