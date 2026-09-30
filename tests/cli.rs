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
