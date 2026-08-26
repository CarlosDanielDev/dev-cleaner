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
