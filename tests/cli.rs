pub mod common;

use clap::Parser;
use dev_cleaner::cli::{Cli, Command};

#[test]
fn purge_defaults_to_a_dry_run() {
    let cli = Cli::parse_from(["dev-cleaner", "purge"]);
    match cli.command {
        Command::Purge { execute, .. } => assert!(
            !execute,
            "purge must be a dry run unless --execute is passed explicitly"
        ),
        other => panic!("expected Purge, got {other:?}"),
    }
}

#[test]
fn purge_touches_disk_only_with_the_explicit_flag() {
    let cli = Cli::parse_from(["dev-cleaner", "purge", "--execute"]);
    match cli.command {
        Command::Purge { execute, .. } => assert!(execute),
        other => panic!("expected Purge, got {other:?}"),
    }
}

#[test]
fn scan_accepts_roots_and_defaults_to_none() {
    let cli = Cli::parse_from(["dev-cleaner", "scan"]);
    match cli.command {
        Command::Scan { roots } => assert!(roots.is_empty(), "no roots means use the config"),
        other => panic!("expected Scan, got {other:?}"),
    }
}

mod purge_flow {
    use clap::Parser;
    use dev_cleaner::cli::{Cli, Command, PurgeAction, purge_action};

    #[test]
    fn execute_alone_is_not_enough_to_delete_anything() {
        // Knowing the phrase requires having seen the plan. A flag can be typed
        // from muscle memory; a phrase describing this exact plan cannot.
        let err = purge_action(true, None).expect_err("must refuse");
        assert!(
            err.to_lowercase().contains("confirm"),
            "the refusal should say what is missing: {err}"
        );
    }

    #[test]
    fn no_flags_at_all_is_a_dry_run() {
        assert!(matches!(purge_action(false, None), Ok(PurgeAction::DryRun)));
    }

    #[test]
    fn a_confirmation_without_execute_still_does_not_delete() {
        assert!(
            matches!(
                purge_action(false, Some("purge 1 items 2 bytes".into())),
                Ok(PurgeAction::DryRun)
            ),
            "both signals are required, in the right order"
        );
    }

    #[test]
    fn execute_with_a_phrase_carries_it_through_for_checking() {
        match purge_action(true, Some("purge 3 items 99 bytes".into())) {
            Ok(PurgeAction::Execute { phrase }) => assert_eq!(phrase, "purge 3 items 99 bytes"),
            other => panic!("expected Execute, got {other:?}"),
        }
    }

    #[test]
    fn the_parser_accepts_both_flags() {
        let cli = Cli::parse_from([
            "dev-cleaner",
            "purge",
            "--execute",
            "--confirm",
            "purge 2 items 10 bytes",
        ]);
        match cli.command {
            Command::Purge { execute, confirm } => {
                assert!(execute);
                assert_eq!(confirm.as_deref(), Some("purge 2 items 10 bytes"));
            }
            other => panic!("expected Purge, got {other:?}"),
        }
    }
}

#[test]
fn duplicates_accepts_roots_and_defaults_to_none() {
    let cli = Cli::parse_from(["dev-cleaner", "duplicates"]);
    match cli.command {
        Command::Duplicates { roots } => assert!(roots.is_empty(), "no roots means use the config"),
        other => panic!("expected Duplicates, got {other:?}"),
    }
}

/// The report reads the disk and nothing else. There is no flag on it that
/// could be mistaken for one that acts.
#[test]
fn duplicates_has_no_flag_that_touches_the_disk() {
    let err = Cli::try_parse_from(["dev-cleaner", "duplicates", "--execute"])
        .expect_err("duplicates takes roots and nothing else");
    assert!(
        err.to_string().contains("--execute"),
        "the refusal should name the flag: {err}"
    );
}

/// `purge --execute` reports each item as it moves and keeps the record on disk
/// current while the run is still going, through the same function the binary
/// runs. Never the real remover: the run here lands in a tempdir, not the Trash.
mod purge_execute {
    use std::cell::RefCell;
    use std::path::{Path, PathBuf};

    use crate::common::Fixture;
    use crate::common::purge::{candidate, confirmed};
    use dev_cleaner::purge::{Remover, execute_and_record};

    /// Reads the record on disk each time it is asked to move something, so
    /// the test can see what the record said while the run was still going.
    struct Watcher {
        dir: PathBuf,
        seen: RefCell<Vec<Option<String>>>,
        fail_on: Option<&'static str>,
    }

    impl Remover for Watcher {
        fn remove(&self, path: &Path) -> std::io::Result<PathBuf> {
            self.seen.borrow_mut().push(record_in(&self.dir));
            if self.fail_on.is_some_and(|f| path.ends_with(f)) {
                return Err(std::io::Error::other(format!(
                    "permission denied: {}",
                    path.display()
                )));
            }
            Ok(PathBuf::from("/Users/test/.Trash").join(path.file_name().unwrap()))
        }
    }

    /// The one record in `dir`, if a write has happened yet.
    fn record_in(dir: &Path) -> Option<String> {
        let mut files: Vec<_> = std::fs::read_dir(dir)
            .ok()?
            .flatten()
            .map(|e| e.path())
            .collect();
        assert!(files.len() <= 1, "one run, one record: {files:?}");
        std::fs::read_to_string(files.pop()?).ok()
    }

    #[test]
    fn each_item_gets_a_line_as_it_moves_and_the_record_grows_with_it() {
        let tmp = Fixture::new();
        let watcher = Watcher {
            dir: tmp.root().to_path_buf(),
            seen: RefCell::new(Vec::new()),
            fail_on: Some("target"),
        };
        let plan = confirmed(vec![
            candidate("a/node_modules", 100),
            candidate("b/target", 200),
            candidate("c/.venv", 300),
        ]);
        let mut lines: Vec<String> = Vec::new();

        let manifest = execute_and_record(plan, &watcher, tmp.root(), &mut |line| {
            lines.push(line.to_string())
        });

        assert_eq!(lines.len(), 3, "one line per item: {lines:?}");
        assert!(lines[0].contains("a/node_modules"), "{}", lines[0]);
        assert!(
            lines[1].contains("b/target") && lines[1].contains("permission denied"),
            "a failed item says so, and why: {}",
            lines[1]
        );
        assert!(lines[2].contains("c/.venv"), "{}", lines[2]);

        let seen = watcher.seen.borrow();
        assert!(seen[0].is_none(), "nothing to record before the first item");
        let after_one = seen[1].as_deref().expect("a record after the first item");
        assert!(
            after_one.contains("a/node_modules") && !after_one.contains("b/target"),
            "the record after item 1 lists item 1 alone:\n{after_one}"
        );
        let after_two = seen[2].as_deref().expect("a record after the second item");
        assert!(
            after_two.contains("b/target") && !after_two.contains("c/.venv"),
            "the record after item 2 lists the failure and not item 3:\n{after_two}"
        );

        assert_eq!(manifest.items.len(), 3);
        assert_eq!(
            manifest.skipped().count(),
            0,
            "the command line has no way to stop, so nothing is skipped"
        );
        let at_the_end = record_in(tmp.root()).expect("a record after the run");
        assert!(at_the_end.contains("c/.venv"), "the last item is on record");
    }
}

/// `scan` records the reclaimable total through the same snapshot the
/// interface does, so the history the dashboard draws is of every run, not
/// only of the runs that opened the dashboard.
#[test]
fn scan_records_the_reclaimable_total() {
    let home = common::Fixture::new();
    let corpus = common::Fixture::new();
    corpus.file("app/package.json", b"{}");
    corpus.file("app/src/index.js", b"console.log(1)");
    corpus.file("app/node_modules/react/index.js", &vec![0x42u8; 8192]);

    let out = std::process::Command::new(env!("CARGO_BIN_EXE_dev-cleaner"))
        .arg("scan")
        .arg(corpus.root())
        .env("HOME", home.root())
        .output()
        .expect("run scan");
    assert!(
        out.status.success(),
        "scan failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    let store = dev_cleaner::store::Store::open(
        &home.root().join(".local/state/dev-cleaner/history.sqlite3"),
    )
    .expect("open history");
    let history = store
        .history(&[corpus.root().to_path_buf()], 10)
        .expect("history");
    assert_eq!(history.len(), 1, "the scan was not recorded");
    assert!(
        matches!(history[0].1, Some(bytes) if bytes > 0),
        "the scan did not record what node_modules holds: {:?}",
        history[0].1
    );
}

mod progress_line {
    use std::process::Command;

    use super::common::Fixture;

    /// Run `scan` with stdout captured, which makes it a pipe and not a terminal.
    fn scan_to_a_pipe() -> String {
        let home = Fixture::new();
        let corpus = Fixture::new();
        corpus.file("app/package.json", b"{}");
        corpus.file("app/src/index.js", b"console.log(1)");

        let out = Command::new(env!("CARGO_BIN_EXE_dev-cleaner"))
            .arg("scan")
            .arg(corpus.root())
            .env("HOME", home.root())
            .output()
            .expect("run scan");
        assert!(out.status.success(), "scan failed: {out:?}");
        String::from_utf8(out.stdout).expect("utf8")
    }

    #[test]
    fn a_pipe_gets_no_carriage_returns() {
        // A CI log is a pipe. A redrawn line there is a thousand lines of noise.
        let stdout = scan_to_a_pipe();
        assert!(
            !stdout.contains('\r') && !stdout.contains('\x1b'),
            "redraw bytes leaked into a pipe: {stdout:?}"
        );
        assert!(!stdout.contains("scanning"), "{stdout:?}");
    }

    #[test]
    fn a_pipe_gets_the_final_line_first() {
        let stdout = scan_to_a_pipe();
        let first = stdout.lines().next().expect("a first line");
        assert!(
            first.starts_with("scanned 1 project, 2 entries in "),
            "unexpected first line: {first:?}"
        );
        assert!(first.ends_with(" s"), "{first:?}");
    }
}
