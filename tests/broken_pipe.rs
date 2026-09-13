//! What happens when whatever was reading the report goes away.

pub mod common;

use std::process::{Command, Stdio};

use common::Fixture;
use dev_cleaner::store::Store;

/// Run `scan` with a stdout whose reader is already gone.
///
/// The read end is closed before the child is spawned, so every write fails
/// immediately. `| head` is the same situation with a race in it; closing first
/// makes the test deterministic rather than dependent on who gets scheduled.
fn scan_into_a_closed_pipe(home: &Fixture, corpus: &Fixture) -> std::process::Output {
    let (reader, writer) = std::io::pipe().expect("pipe");
    drop(reader);

    Command::new(env!("CARGO_BIN_EXE_dev-cleaner"))
        .arg("scan")
        .arg(corpus.root())
        .env("HOME", home.root())
        .stdout(Stdio::from(writer))
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn")
        .wait_with_output()
        .expect("wait")
}

fn corpus() -> Fixture {
    let fx = Fixture::new();
    fx.file("app/package.json", b"{}");
    fx.file("app/src/index.js", b"console.log(1)");
    fx.file("app/node_modules/react/index.js", b"x");
    fx
}

#[test]
fn a_scan_is_recorded_even_when_nothing_reads_its_output() {
    // Piping a long report into `head` or `less` is the ordinary way to read
    // one. The history has to survive it, because the next scan's trend is
    // measured against the last recorded scan: a row lost here makes a later
    // run attribute weeks of change to one interval.
    let home = Fixture::new();
    let corpus = corpus();

    scan_into_a_closed_pipe(&home, &corpus);

    let db = home.root().join(".local/state/dev-cleaner/history.sqlite3");
    assert!(db.exists(), "the scan never opened the history database");

    let store = Store::open(&db).expect("open history");
    assert_eq!(
        store.scan_ids().expect("scan ids").len(),
        1,
        "the scan ran to completion but was not recorded"
    );
}

#[test]
fn a_reader_that_went_away_is_not_an_error_or_a_panic() {
    let home = Fixture::new();
    let corpus = corpus();

    let out = scan_into_a_closed_pipe(&home, &corpus);
    let stderr = String::from_utf8_lossy(&out.stderr);

    assert!(
        !stderr.contains("panicked"),
        "a closed reader must not panic: {stderr}"
    );
    assert!(
        out.status.success(),
        "the run should finish normally, got {:?}: {stderr}",
        out.status
    );
}
