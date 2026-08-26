//! Command line surface.
//!
//! Dry-run is not a flag the user has to remember; it is what happens when
//! nothing is passed. Only `purge --execute` may touch the disk.

use std::path::PathBuf;

use clap::{Parser, Subcommand};

#[derive(Parser, Debug)]
#[command(name = "dev-cleaner", version, about)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Subcommand, Debug)]
pub enum Command {
    /// Walk the registered roots and report what is there. Always read-only.
    Scan {
        /// Roots to scan. Falls back to the configured roots when omitted.
        roots: Vec<PathBuf>,
    },
    /// Show what would be reclaimed. Deletes nothing without `--execute`.
    Purge {
        /// Actually move the planned entries to the Trash.
        #[arg(long)]
        execute: bool,
        /// The confirmation phrase printed by a dry run, describing this exact
        /// plan. Required alongside `--execute`.
        #[arg(long)]
        confirm: Option<String>,
    },
}

/// What a `purge` invocation is actually asking for.
#[derive(Debug, PartialEq, Eq)]
pub enum PurgeAction {
    DryRun,
    Execute { phrase: String },
}

/// Resolve the flags into an action, refusing the dangerous half-specified case.
///
/// `--execute` on its own is not sufficient. A flag can be typed from muscle
/// memory or recalled from shell history; a phrase describing the exact plan
/// cannot be known without having read that plan. Requiring both means the
/// review step cannot be skipped from the command line any more than it can
/// from the type system.
pub fn purge_action(execute: bool, confirm: Option<String>) -> Result<PurgeAction, String> {
    match (execute, confirm) {
        (true, Some(phrase)) => Ok(PurgeAction::Execute { phrase }),
        (true, None) => Err(
            "--execute also needs --confirm with the phrase from a dry run. \
             Run `dev-cleaner purge` first to see the plan and its phrase. \
             Nothing has been touched."
                .to_string(),
        ),
        (false, _) => Ok(PurgeAction::DryRun),
    }
}
