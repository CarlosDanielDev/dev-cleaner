use std::process::ExitCode;

use clap::Parser;
use dev_cleaner::cli::{Cli, Command};
use dev_cleaner::scan::{Usage, Walker};

fn main() -> ExitCode {
    match Cli::parse().command {
        Command::Scan { roots } => {
            if roots.is_empty() {
                eprintln!("no roots given; configured roots arrive with the config work");
                return ExitCode::FAILURE;
            }
            let started = std::time::Instant::now();
            let result = Walker::new(&roots).walk();
            let usage = Usage::of(&result.files);
            let elapsed = started.elapsed();

            println!("scanned {} root(s) in {:.2?}", roots.len(), elapsed);
            println!("  entries        {}", usage.files);
            println!("  inodes         {}", usage.inodes);
            println!("  apparent       {:.2} GB", gb(usage.bytes_apparent));
            println!("  actual/unique  {:.2} GB", gb(usage.bytes_unique));
            if !result.errors.is_empty() {
                println!("  unreadable     {} path(s)", result.errors.len());
            }
            ExitCode::SUCCESS
        }
        Command::Purge { .. } => {
            eprintln!("purge is not available yet: execution lands with the safety gate in M2");
            ExitCode::FAILURE
        }
    }
}

fn gb(bytes: u64) -> f64 {
    bytes as f64 / 1024.0 / 1024.0 / 1024.0
}
