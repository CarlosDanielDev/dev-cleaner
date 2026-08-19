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
    },
}
