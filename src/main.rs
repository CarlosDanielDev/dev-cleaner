use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::SystemTime;

use clap::Parser;
use dev_cleaner::classify::{Activity, ProjectIndex, artifact_for, probe_caches};
use dev_cleaner::cli::{Cli, Command};
use dev_cleaner::config::Config;
use dev_cleaner::scan::{FileMeta, Usage, Walker};

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

    let started = SystemTime::now();
    let result = Walker::new(&roots).walk();
    let elapsed = started.elapsed().unwrap_or_default();

    // The denylist is the outermost boundary: entries inside it never reach any
    // later stage, so they cannot be counted, ranked, or offered.
    let (denied, kept): (Vec<_>, Vec<_>) = result
        .files
        .into_iter()
        .partition(|f| cfg.is_denied(&f.path));

    let usage = Usage::of(&kept);
    let projects = ProjectIndex::from_files(&kept);
    let now = SystemTime::now();

    println!("scanned {} root(s) in {:.2?}", roots.len(), elapsed);
    println!("  projects       {}", projects.len());
    println!("  entries        {}", usage.files);
    println!("  inodes         {}", usage.inodes);
    println!("  actual/unique  {:.2} GB", gb(usage.bytes_unique));
    if !denied.is_empty() {
        println!("  denylisted     {} path(s) excluded", denied.len());
    }
    if !result.errors.is_empty() {
        println!("  unreadable     {} path(s)", result.errors.len());
    }

    report_activity(&projects, &kept, now);
    report_artifacts(&kept);
    report_caches(&cfg);
    ExitCode::SUCCESS
}

/// Newest source file per project, used as activity evidence alongside git.
fn newest_source(files: &[FileMeta], index: &ProjectIndex) -> BTreeMap<PathBuf, SystemTime> {
    let mut newest: BTreeMap<PathBuf, SystemTime> = BTreeMap::new();
    for f in files {
        // Build output is regenerated constantly and says nothing about whether
        // a human has touched the project.
        if is_inside_artifact(&f.path) {
            continue;
        }
        if let Some(p) = index.owner_of(&f.path) {
            newest
                .entry(p.root.clone())
                .and_modify(|t| {
                    if f.mtime > *t {
                        *t = f.mtime
                    }
                })
                .or_insert(f.mtime);
        }
    }
    newest
}

fn report_activity(index: &ProjectIndex, files: &[FileMeta], now: SystemTime) {
    let newest = newest_source(files, index);
    let (mut active, mut dormant, mut dead) = (0, 0, 0);
    let mut dead_roots = Vec::new();

    for p in index.projects() {
        match Activity::of(&p.root, newest.get(&p.root).copied(), now) {
            Activity::Active => active += 1,
            Activity::Dormant => dormant += 1,
            Activity::Dead => {
                dead += 1;
                dead_roots.push(&p.root);
            }
        }
    }

    println!("\nactivity");
    println!("  active         {active}");
    println!("  dormant        {dormant}");
    println!("  dead           {dead}  (idle >180d, every commit on a remote)");
    for root in dead_roots.iter().take(5) {
        println!("      {}", root.display());
    }
    if dead_roots.len() > 5 {
        println!("      ... and {} more", dead_roots.len() - 5);
    }
}

fn report_artifacts(files: &[FileMeta]) {
    let mut by_kind: BTreeMap<&str, (u64, u64)> = BTreeMap::new();
    for f in files {
        if let Some(kind) = outermost_artifact(&f.path) {
            let e = by_kind.entry(kind).or_default();
            e.0 += f.bytes_actual;
            e.1 += 1;
        }
    }
    if by_kind.is_empty() {
        return;
    }
    let mut rows: Vec<_> = by_kind.into_iter().collect();
    rows.sort_by_key(|(_, (bytes, _))| std::cmp::Reverse(*bytes));

    println!("\nbuild artifacts");
    let total: u64 = rows.iter().map(|(_, (b, _))| b).sum();
    for (kind, (bytes, count)) in rows.iter().take(8) {
        let regen = artifact_for(kind).map(|k| k.regen).unwrap_or("");
        println!(
            "  {:<14} {:>7.2} GB  {:>7} files   {}",
            kind,
            gb(*bytes),
            count,
            regen
        );
    }
    println!("  {:<14} {:>7.2} GB  reclaimable", "total", gb(total));
}

fn report_caches(cfg: &Config) {
    let home = PathBuf::from(std::env::var("HOME").unwrap_or_else(|_| "/".into()));
    let found = probe_caches(&home, &cfg.caches);
    if found.is_empty() {
        return;
    }
    println!("\nglobal caches");
    let mut total = 0;
    for entry in &found {
        let bytes = entry.usage().bytes_unique;
        total += bytes;
        println!(
            "  {:<22} {:>7.2} GB   {}",
            entry.kind.name,
            gb(bytes),
            entry.kind.cleanup.unwrap_or(entry.kind.regen)
        );
    }
    println!("  {:<22} {:>7.2} GB   reclaimable", "total", gb(total));
}

fn is_inside_artifact(path: &Path) -> bool {
    outermost_artifact(path).is_some()
}

/// The shallowest artifact directory on this path, so nested build output is
/// attributed to the directory that would actually be deleted.
fn outermost_artifact(path: &Path) -> Option<&'static str> {
    path.components()
        .filter_map(|c| c.as_os_str().to_str())
        .find_map(|c| artifact_for(c).map(|k| k.dir_name))
}

fn config_path() -> PathBuf {
    PathBuf::from(std::env::var("HOME").unwrap_or_else(|_| "/".into()))
        .join(".config/dev-cleaner/config.toml")
}

fn gb(bytes: u64) -> f64 {
    bytes as f64 / 1024.0 / 1024.0 / 1024.0
}
