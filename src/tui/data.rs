//! One walk becoming every screen.
//!
//! The screens are state and drawing, with no idea where their contents came
//! from; that is what lets them be tested with no terminal and no filesystem.
//! Something still has to turn a walk into them, and this is the only place
//! that does. Spread across the event loop instead, the rule that decides how
//! an artifact directory is measured would sit beside key dispatch, where a
//! later edit to one is free to diverge from the other.
//!
//! Every byte total on every screen is [`Usage::of`] over a set of files. None
//! of them is a sum of other totals. That is #45: adding `bytes_actual` up per
//! file counted cargo's hardlinked build output twice and read `target` as
//! 3.21 GB against 1.84 GB actual. The grouping those sets come from is
//! [`group_by_artifact_root`], which is the only place that grouping exists.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use super::{Candidates, Consumer, Dashboard, ProjectSummary, Projects, Trend};
use crate::candidates::{from_scan, group_by_artifact_root};
use crate::classify::{Activity, CacheEntry, ProjectIndex, artifact_root, probe_caches};
use crate::config::Config;
use crate::safety::Guards;
use crate::scan::{FileMeta, Usage, Walker};
use crate::store::{Store, snapshot};
use crate::volume::Volume;

/// Everything the interface draws, built from a single walk.
#[derive(Debug)]
pub struct Screens {
    /// The roots this was built from, kept so the run measures free space on
    /// the volume it actually scanned.
    pub roots: Vec<PathBuf>,
    pub dashboard: Dashboard,
    pub projects: Projects,
    pub candidates: Candidates,
}

/// Walk `roots` and build the three screens a scan can fill.
///
/// `home` and `db` are passed rather than read from the environment so a test
/// can point them at a fixture. A store under the developer's real home would
/// make the suite write to the history the binary reports from.
pub fn collect(roots: &[PathBuf], cfg: &Config, home: &Path, db: &Path) -> Screens {
    let started = SystemTime::now();

    // The denylist is the outermost boundary, applied here exactly as `scan`
    // applies it: an entry inside it never reaches any later stage, so it
    // cannot be counted, ranked, or offered.
    let files: Vec<FileMeta> = Walker::new(roots)
        .walk()
        .files
        .into_iter()
        .filter(|f| !cfg.is_denied(&f.path))
        .collect();

    let index = ProjectIndex::from_files(&files);
    let guards = Guards::new(roots.to_vec(), cfg.denylist.clone());
    let caches: Vec<(CacheEntry, Usage)> = probe_caches(home, &cfg.caches)
        .into_iter()
        .map(|c| {
            let usage = c.usage();
            (c, usage)
        })
        .collect();

    let grouped = group_by_artifact_root(&files);

    // Measured over the union of every artifact file, not by adding the
    // directories up. An inode reachable from two artifact directories is
    // returned once by deleting both, and this is the number the disk gauge
    // claims is available. Each directory on its own is measured separately
    // below, where counting it inside each is the right answer.
    let reclaimable = Usage::of(
        grouped
            .values()
            .flat_map(|(group, _)| group.iter().copied()),
    )
    .bytes_unique;

    let consumers: Vec<Consumer> = grouped
        .iter()
        .map(|(path, (group, _))| {
            let usage = Usage::of(group.iter().copied());
            Consumer {
                label: label_for(path),
                bytes: usage.bytes_unique,
                inodes: usage.inodes,
            }
        })
        .collect();

    let dashboard = Dashboard {
        // The first root, not the root filesystem: a scanned root may sit on an
        // external disk, where `/` says nothing about what a purge there frees.
        volume: roots.first().and_then(|r| Volume::of(r)),
        reclaimable,
        trend: record_and_compare(
            db,
            &snapshot(started, roots, &files, &index, &guards, &caches),
        ),
        consumers,
    };

    let built = from_scan(&files, &guards);

    Screens {
        roots: roots.to_vec(),
        dashboard,
        projects: Projects::new(summarise_projects(&files, &index)),
        candidates: Candidates::new(built.candidates, built.rejected),
    }
}

/// What a project holds, and how much of that is build output.
///
/// ponytail: one pass over the walk, asking `ProjectIndex` who owns each file,
/// which is a scan of the project list per file. `scan` already pays exactly
/// this to find each project's newest source file. Index the roots by prefix if
/// a corpus ever makes it show.
fn summarise_projects(files: &[FileMeta], index: &ProjectIndex) -> Vec<ProjectSummary> {
    /// Everything accumulated for one project: all its files, the artifact
    /// subset, and the newest thing a human plausibly wrote.
    #[derive(Default)]
    struct Owned<'a> {
        all: Vec<&'a FileMeta>,
        artifacts: Vec<&'a FileMeta>,
        newest_source: Option<SystemTime>,
    }

    let mut owned: BTreeMap<&Path, Owned> = BTreeMap::new();
    for file in files {
        let Some(project) = index.owner_of(&file.path) else {
            continue;
        };
        let entry = owned.entry(project.root.as_path()).or_default();
        entry.all.push(file);
        if artifact_root(&file.path).is_some() {
            entry.artifacts.push(file);
        } else {
            // Build output is regenerated constantly and says nothing about
            // whether anyone has touched the project.
            entry.newest_source = entry.newest_source.max(Some(file.mtime));
        }
    }

    let now = SystemTime::now();
    owned
        .into_iter()
        .map(|(root, project)| {
            let usage = Usage::of(project.all.iter().copied());
            ProjectSummary {
                path: root.to_path_buf(),
                bytes_apparent: usage.bytes_apparent,
                bytes_unique: usage.bytes_unique,
                inodes: usage.inodes,
                activity: Activity::of(root, project.newest_source, now),
                // Measured inside the project rather than summed from its
                // directories, for the same reason the disk total is.
                reclaimable: Usage::of(project.artifacts.iter().copied()).bytes_unique,
            }
        })
        .collect()
}

/// Record this scan and say what moved since the last one of the same roots.
///
/// The interface records what it saw, exactly as `scan` does. A run that looked
/// but did not record would make the next run's trend skip it, and the two
/// commands would disagree about when the disk was last measured.
///
/// The baseline is the latest scan *of these roots*. Against a wider root set,
/// every path outside this one reads as removed, which is the tool reporting
/// deletions that never happened.
fn record_and_compare(db: &Path, snap: &crate::store::Snapshot) -> Trend {
    let mut store = match Store::open(db) {
        Ok(store) => store,
        Err(err) => return Trend::Unavailable(err.to_string()),
    };
    let previous = store.latest_scan_for(&snap.roots).unwrap_or(None);
    let current = match store.write_snapshot(snap) {
        Ok(id) => id,
        Err(err) => return Trend::Unavailable(err.to_string()),
    };
    let Some(previous) = previous else {
        return Trend::FirstScan;
    };
    match store.trend(previous, current) {
        Ok(rows) => Trend::Since(rows),
        Err(err) => Trend::Unavailable(err.to_string()),
    }
}

/// What to call an artifact directory in a list narrow enough to read.
///
/// The last two components: the directory and the project it belongs to, which
/// is what tells two `target`s apart. The full path is on the candidates
/// screen, where there is room for it and where it is about to be acted on.
fn label_for(path: &Path) -> String {
    let tail: Vec<_> = path
        .components()
        .rev()
        .take(2)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    tail.iter()
        .map(|c| c.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/")
}
