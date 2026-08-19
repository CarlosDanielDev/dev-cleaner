use std::path::PathBuf;
use std::process::ExitCode;

use clap::Parser;
use dev_cleaner::cli::{Cli, Command};
use dev_cleaner::config::Config;
use dev_cleaner::scan::{Usage, Walker};

fn main() -> ExitCode {
    match Cli::parse().command {
        Command::Scan { roots } => scan(roots),
        Command::Purge { .. } => {
            eprintln!("purge is not available yet: execution lands with the safety gate in M2");
            ExitCode::FAILURE
        }
    }
}

fn scan(roots: Vec<PathBuf>) -> ExitCode {
    let cfg = match Config::load(&config_path()) {
        Ok(cfg) => cfg,
        Err(err) => {
            eprintln!("config is not valid TOML: {err}");
            return ExitCode::FAILURE;
        }
    };

    let roots = if roots.is_empty() {
        for missing in cfg.missing_roots() {
            eprintln!("configured root does not exist: {}", missing.display());
        }
        cfg.roots.clone()
    } else {
        roots
    };

    let started = std::time::Instant::now();
    let result = Walker::new(&roots).walk();
    let elapsed = started.elapsed();

    // The denylist is the outermost boundary: entries inside it never reach
    // any later stage, so they cannot be counted, ranked, or offered.
    let (denied, kept): (Vec<_>, Vec<_>) = result
        .files
        .into_iter()
        .partition(|f| cfg.is_denied(&f.path));
    let usage = Usage::of(&kept);

    println!("scanned {} root(s) in {:.2?}", roots.len(), elapsed);
    println!("  entries        {}", usage.files);
    println!("  inodes         {}", usage.inodes);
    println!("  apparent       {:.2} GB", gb(usage.bytes_apparent));
    println!("  actual/unique  {:.2} GB", gb(usage.bytes_unique));
    if !denied.is_empty() {
        println!("  denylisted     {} path(s) excluded", denied.len());
    }
    if !result.errors.is_empty() {
        println!("  unreadable     {} path(s)", result.errors.len());
    }
    ExitCode::SUCCESS
}

fn config_path() -> PathBuf {
    PathBuf::from(std::env::var("HOME").unwrap_or_else(|_| "/".into()))
        .join(".config/dev-cleaner/config.toml")
}

fn gb(bytes: u64) -> f64 {
    bytes as f64 / 1024.0 / 1024.0 / 1024.0
}
