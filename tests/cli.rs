use clap::Parser;
use dev_cleaner::cli::{Cli, Command};

#[test]
fn purge_defaults_to_a_dry_run() {
    let cli = Cli::parse_from(["dev-cleaner", "purge"]);
    match cli.command {
        Command::Purge { execute } => assert!(
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
        Command::Purge { execute } => assert!(execute),
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
