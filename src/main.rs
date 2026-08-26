use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::SystemTime;

mod history;

use clap::Parser;
use dev_cleaner::candidates::from_scan;
use dev_cleaner::classify::{Activity, CacheEntry, ProjectIndex, artifact_for, probe_caches};
use dev_cleaner::cli::{Cli, Command, PurgeAction, purge_action};
use dev_cleaner::config::Config;
use dev_cleaner::purge::{
    TrashRemover, execute as run_purge, free_bytes, manifest_dir, write_manifest,
};
use dev_cleaner::safety::Guards;
use dev_cleaner::safety::Plan;
use dev_cleaner::scan::{FileMeta, Usage, Walker};
use dev_cleaner::store::snapshot;

fn main() -> ExitCode {
    match Cli::parse().command {
        Command::Scan { roots } => scan(roots),
        Command::Purge { execute, confirm } => match purge_action(execute, confirm) {
            Ok(action) => purge(action),
            Err(refusal) => {
                eprintln!("{refusal}");
                ExitCode::FAILURE
            }
        },
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
    let guards = Guards::new(roots.clone(), cfg.denylist.clone());
    let caches: Vec<(CacheEntry, Usage)> = probe_caches(&home(), &cfg.caches)
        .into_iter()
        .map(|c| {
            let usage = c.usage();
            (c, usage)
        })
        .collect();

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

    report_activity(&projects, &kept, now, &guards);
    report_artifacts(&kept);
    report_caches(&caches);
    history::remember(&snapshot(
        started, &roots, &kept, &projects, &guards, &caches,
    ));
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

fn report_activity(index: &ProjectIndex, files: &[FileMeta], now: SystemTime, guards: &Guards) {
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

    // Being idle is not sufficient. Run the hard guards over the dead set so
    // the report shows what would actually survive to a plan, and why the rest
    // would not.
    let mut clear = 0;
    let mut blocked: BTreeMap<&str, usize> = BTreeMap::new();
    for root in &dead_roots {
        match guards.check(root) {
            Ok(()) => clear += 1,
            Err(reason) => *blocked.entry(reason.explain()).or_default() += 1,
        }
    }
    println!("      {clear} of {dead} clear every guard");
    for (why, n) in &blocked {
        println!("      {n} blocked: {why}");
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

fn report_caches(found: &[(CacheEntry, Usage)]) {
    if found.is_empty() {
        return;
    }
    println!("\nglobal caches");
    let mut total = 0;
    for (entry, usage) in found {
        let bytes = usage.bytes_unique;
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

fn home() -> PathBuf {
    PathBuf::from(std::env::var("HOME").unwrap_or_else(|_| "/".into()))
}

fn config_path() -> PathBuf {
    home().join(".config/dev-cleaner/config.toml")
}

fn gb(bytes: u64) -> f64 {
    bytes as f64 / 1024.0 / 1024.0 / 1024.0
}

fn purge(action: PurgeAction) -> ExitCode {
    let cfg = match Config::load(&config_path()) {
        Ok(cfg) => cfg,
        Err(err) => {
            eprintln!("config is not valid TOML: {err}");
            return ExitCode::FAILURE;
        }
    };
    let roots = cfg.roots.clone();
    let files: Vec<FileMeta> = Walker::new(&roots)
        .walk()
        .files
        .into_iter()
        .filter(|f| !cfg.is_denied(&f.path))
        .collect();

    let guards = Guards::new(roots.clone(), cfg.denylist.clone());
    let built = from_scan(&files, &guards);

    let mut draft = Plan::draft();
    for candidate in built.candidates {
        // add() refuses anything not provably recoverable. Nothing reaches a
        // plan on the strength of having been listed.
        if let Err(rejected) = draft.add(candidate) {
            eprintln!("skipped {rejected}");
        }
    }
    let reviewed = draft.review();

    if reviewed.items().is_empty() {
        println!("Nothing to reclaim.");
        if !built.rejected.is_empty() {
            println!("\n{} candidate(s) were blocked:", built.rejected.len());
            for r in built.rejected.iter().take(10) {
                println!("  {r}");
            }
        }
        return ExitCode::SUCCESS;
    }

    println!(
        "Plan: {} item(s), {:.2} GB",
        reviewed.items().len(),
        gb(reviewed.total_bytes())
    );
    let mut items: Vec<_> = reviewed.items().iter().collect();
    items.sort_by_key(|c| std::cmp::Reverse(c.bytes));
    for c in items.iter().take(15) {
        println!("  {:>8.2} GB  {}", gb(c.bytes), c.path.display());
    }
    if items.len() > 15 {
        println!("  ... and {} more", items.len() - 15);
    }
    if !built.rejected.is_empty() {
        println!("\nBlocked, not in the plan ({}):", built.rejected.len());
        for r in built.rejected.iter().take(10) {
            println!("  {r}");
        }
    }

    let phrase = reviewed.confirmation_phrase();
    let PurgeAction::Execute { phrase: typed } = action else {
        println!("\nThis was a dry run. Nothing has been touched.");
        println!("To carry it out:\n  dev-cleaner purge --execute --confirm \"{phrase}\"");
        return ExitCode::SUCCESS;
    };

    let confirmed = match reviewed.confirm(&typed) {
        Ok(plan) => plan,
        Err(_) => {
            eprintln!("\nThat phrase does not match this plan, so nothing was touched.");
            eprintln!("Expected: {phrase}");
            eprintln!(
                "The phrase describes the exact plan above; if the disk changed since \
                       your last dry run, run one again."
            );
            return ExitCode::FAILURE;
        }
    };

    let measure_at = roots.first().cloned().unwrap_or_else(|| PathBuf::from("/"));
    let before = free_bytes(&measure_at);

    let mut manifest = run_purge(confirmed, &TrashRemover);

    if let (Some(before), Some(after)) = (before, free_bytes(&measure_at)) {
        manifest.record_actual(after.saturating_sub(before));
    }

    match write_manifest(&manifest, &manifest_dir()) {
        Ok(path) => println!("\nRecord written to {}", path.display()),
        Err(err) => eprintln!("\ncould not write the record: {err}"),
    }

    println!("  moved to Trash {:.2} GB", gb(manifest.bytes_moved()));
    if manifest.freed_immediately {
        match manifest.bytes_actual {
            Some(actual) => println!("  freed on disk  {:.2} GB", gb(actual)),
            None => println!("  freed on disk  not measured"),
        }
        if let Some(gap) = manifest.shortfall() {
            println!(
                "  note: the disk returned {:.0}% less than was moved, which usually means \
                 hardlinked content still referenced elsewhere",
                gap * 100.0
            );
        }
    } else {
        // Free space is deliberately not reported as reclaimed here. The Trash
        // is on the same disk, so it has not changed, and presenting it as a
        // result would be the tool taking credit for space the user does not
        // yet have.
        println!(
            "  waiting in Trash {:.2} GB",
            gb(manifest.pending_in_trash())
        );
    }
    println!("\nEverything went to the Trash and can be put back from Finder.");
    println!("Free space has not changed yet; empty the Trash to reclaim it.");

    if manifest.is_complete() {
        ExitCode::SUCCESS
    } else {
        eprintln!(
            "\n{} item(s) could not be moved; see the record.",
            manifest.failed().count()
        );
        ExitCode::FAILURE
    }
}
