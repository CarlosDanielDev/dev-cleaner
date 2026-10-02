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

use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use super::{
    Aim, Analysed, Candidates, Consumer, Dashboard, Group, Now, ProjectSummary, Projects, Trend,
};
use crate::candidates::{from_scan, group_by_artifact_root};
use crate::classify::{
    Activity, ArtifactKind, CacheEntry, Checkout, Kind, ProjectIndex, artifact_root, probe_caches,
};
use crate::config::Config;
use crate::safety::Guards;
use crate::scan::{FileMeta, Progress, Usage, Walker};
use crate::store::{Store, snapshot};
use crate::volume::Volume;

/// How many scans the sparkline can draw from; the screen keeps the newest that fit.
const HISTORY_SCANS: usize = 24;

/// Everything the interface draws, built from a single walk.
#[derive(Debug)]
pub struct Screens {
    /// The roots this was built from, kept so the run measures free space on
    /// the volume it actually scanned.
    pub roots: Vec<PathBuf>,
    /// The history database, kept so a finished run is remembered in the same
    /// store the scans were.
    pub db: PathBuf,
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
    collect_with(roots, cfg, home, db, &Arc::new(Progress::default()))
}

/// [`collect`], counting the walk into `progress` so another thread can say how far it is.
pub fn collect_with(
    roots: &[PathBuf],
    cfg: &Config,
    home: &Path,
    db: &Path,
    progress: &Arc<Progress>,
) -> Screens {
    let started = SystemTime::now();

    // The denylist is the outermost boundary, applied here exactly as `scan`
    // applies it: an entry inside it never reaches any later stage, so it
    // cannot be counted, ranked, or offered.
    let files: Vec<FileMeta> = Walker::new(roots)
        .walk_with(progress)
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
    let snap = snapshot(started, roots, &files, &index, &guards, &caches);

    // Built before the dashboard, which counts them rather than the scan.
    let built = from_scan(&files, &guards);
    let mut candidates = Candidates::new(built.candidates, built.rejected);
    let projects = Projects::new(summarise_projects(&files, &index));
    // Named as the table names the project each entry is in.
    candidates.set_locator(projects.locator());

    // Named as the projects table names the project they are in, so a
    // directory on the dashboard can be found again by that name.
    let consumers: Vec<Consumer> = grouped
        .iter()
        .map(|(path, (group, _))| {
            let usage = Usage::of(group.iter().copied());
            Consumer {
                label: projects
                    .name_inside(path)
                    .unwrap_or_else(|| label_for(path)),
                bytes: usage.bytes_unique,
                inodes: usage.inodes,
            }
        })
        .collect();

    let (worktrees, repos) = worktree_counts(projects.rows());
    let (trend, history) = record_and_compare(db, &snap);
    let dashboard = Dashboard {
        // The first root, not the root filesystem: a scanned root may sit on an
        // external disk, where `/` says nothing about what a purge there frees.
        volume: roots.first().and_then(|r| Volume::of(r)),
        // The gauge shows the measurement the store keeps, so the history
        // drawn under it can never disagree with it. `snapshot` measures it
        // on every walk; `None` only ever comes back out of the store.
        reclaimable: snap
            .reclaimable_unique
            .expect("a fresh snapshot measures its reclaimable total"),
        trend,
        consumers,
        now: actionable(&candidates, projects.rows()),
        history,
        groups: breakdown(&grouped, &candidates),
        aim: Aim::default(),
        analysed: Analysed {
            projects: projects.rows().len(),
            with_rebuild: projects.rows().iter().filter(|p| p.reclaimable > 0).count(),
            entries: files.len() as u64,
            measured: grouped.len(),
            elapsed: started.elapsed().unwrap_or(Duration::ZERO),
            roots: roots.to_vec(),
            worktrees,
            repos,
        },
    };

    let mut dashboard = dashboard;
    dashboard.aim = aim(&dashboard, &candidates, &projects);

    Screens {
        roots: roots.to_vec(),
        db: db.to_path_buf(),
        dashboard,
        projects,
        candidates,
    }
}

/// Reclaimable bytes by kind of directory, summing to the reclaimable total.
///
/// Each kind measured on its own would count an inode reachable from two kinds
/// in both, and the rows would add up to more than the gauge above them says
/// (#45 again, one level up). So the kinds are taken heaviest first and each
/// counts only the inodes the heavier ones have not: the same union the total
/// is, divided between the kinds instead of added across them.
///
/// What each kind has cleared to offer comes from the candidates screen, so the
/// "biggest win" is a number that screen will show.
fn breakdown(
    grouped: &BTreeMap<PathBuf, (Vec<&FileMeta>, &'static ArtifactKind)>,
    candidates: &Candidates,
) -> Vec<Group> {
    struct Kind<'a> {
        kind: &'static ArtifactKind,
        dirs: usize,
        files: Vec<&'a FileMeta>,
    }
    let mut kinds: BTreeMap<&str, Kind> = BTreeMap::new();
    for (group, kind) in grouped.values() {
        let entry = kinds.entry(kind.dir_name).or_insert_with(|| Kind {
            kind,
            dirs: 0,
            files: Vec::new(),
        });
        entry.dirs += 1;
        entry.files.extend(group.iter().copied());
    }

    let mut ranked: Vec<Kind> = kinds.into_values().collect();
    ranked.sort_by_key(|k| {
        (
            std::cmp::Reverse(Usage::of(k.files.iter().copied()).bytes_unique),
            k.kind.dir_name,
        )
    });

    let mut seen: HashSet<(u64, u64)> = HashSet::new();
    ranked
        .into_iter()
        .map(|k| {
            let bytes = k
                .files
                .iter()
                .filter(|f| seen.insert((f.dev, f.ino)))
                .map(|f| f.bytes_actual)
                .sum();
            let (offerable_bytes, offerable_dirs) = candidates
                .selectable()
                .iter()
                .filter(|c| {
                    artifact_root(&c.path).is_some_and(|(_, kind)| kind.dir_name == k.kind.dir_name)
                })
                .fold((0, 0), |(b, n), c| (b + c.bytes, n + 1));
            Group {
                label: k.kind.dir_name.to_string(),
                ecosystem: k.kind.ecosystem,
                regen: k.kind.regen.to_string(),
                bytes,
                dirs: k.dirs,
                offerable_bytes,
                offerable_dirs,
            }
        })
        .collect()
}

/// The project each insight is about, read off the screens the insight counts.
///
/// Ties go to the path that sorts first, so the cursor lands in the same place
/// on every run.
fn aim(dashboard: &Dashboard, candidates: &Candidates, projects: &Projects) -> Aim {
    let top = |by: BTreeMap<&Path, u64>| {
        by.into_iter()
            .max_by_key(|(path, n)| (*n, std::cmp::Reverse(*path)))
            .map(|(path, _)| path.to_path_buf())
    };

    let mut holding: BTreeMap<&Path, u64> = BTreeMap::new();
    if let Some(win) = dashboard.biggest_win() {
        for c in candidates
            .selectable()
            .iter()
            .filter(|c| artifact_root(&c.path).is_some_and(|(_, kind)| kind.dir_name == win.label))
        {
            if let Some(owner) = projects.owner_of(&c.path) {
                *holding.entry(owner).or_default() += c.bytes;
            }
        }
    }

    let mut held: BTreeMap<&Path, u64> = BTreeMap::new();
    for b in candidates.blocked() {
        if let Some(owner) = projects.owner_of(&b.path) {
            *held.entry(owner).or_default() += 1;
        }
    }

    Aim {
        win: top(holding),
        quiet: projects
            .rows()
            .iter()
            .find(|p| p.activity == Activity::Dead)
            .map(|p| p.path.clone()),
        held: top(held),
    }
}

/// What one step forward would offer, read off the screens it leads to.
///
/// The candidates screen has already sorted every entry into offerable or
/// blocked, and the table has already classified every project. Counting those
/// is what keeps the opening screen from being a second count by another
/// formula, free to disagree with the screen behind it.
fn actionable(candidates: &Candidates, projects: &[ProjectSummary]) -> Now {
    let mut held: BTreeMap<&str, usize> = BTreeMap::new();
    for blocked in candidates.blocked() {
        *held.entry(&blocked.reason).or_default() += 1;
    }
    let dead: Vec<&ProjectSummary> = projects
        .iter()
        .filter(|p| p.activity == Activity::Dead)
        .collect();
    Now {
        offerable: candidates.selectable().len(),
        offerable_bytes: candidates.selectable().iter().map(|c| c.bytes).sum(),
        blocked: held
            .into_iter()
            .map(|(reason, n)| (reason.to_string(), n))
            .collect(),
        dead: dead.len(),
        dead_reclaimable: dead.iter().map(|p| p.reclaimable).sum(),
    }
}

/// How many distinct linked worktrees the projects sit in, and of how many
/// repositories. One worktree holds several projects (`app/ios`, `app/android`),
/// so the rows are not the count.
fn worktree_counts(projects: &[ProjectSummary]) -> (usize, usize) {
    let linked: HashSet<(&Path, &str)> = projects
        .iter()
        .filter(|p| matches!(p.checkout.kind, Kind::Worktree | Kind::Orphan))
        .filter_map(|p| Some((p.checkout.repo.as_deref()?, p.checkout.worktree.as_deref()?)))
        .collect();
    let repos: HashSet<&Path> = linked.iter().map(|(repo, _)| *repo).collect();
    (linked.len(), repos.len())
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
                checkout: Checkout::of(root),
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
fn record_and_compare(db: &Path, snap: &crate::store::Snapshot) -> (Trend, Vec<Option<u64>>) {
    let mut store = match Store::open(db) {
        Ok(store) => store,
        Err(err) => return (Trend::Unavailable(err.to_string()), Vec::new()),
    };
    let previous = store.latest_scan_for(&snap.roots).unwrap_or(None);
    let current = match store.write_snapshot(snap) {
        Ok(id) => id,
        Err(err) => return (Trend::Unavailable(err.to_string()), Vec::new()),
    };
    // Read after the write, so the newest point is this scan. Failing to read
    // it costs the line and nothing else: the trend is the part that reports.
    let history = store
        .history(&snap.roots, HISTORY_SCANS)
        .map(|scans| scans.into_iter().map(|(_, bytes)| bytes).collect())
        .unwrap_or_default();
    let Some(previous) = previous else {
        return (Trend::FirstScan, history);
    };
    match store.trend(previous, current) {
        Ok(rows) => (Trend::Since(rows), history),
        Err(err) => (Trend::Unavailable(err.to_string()), history),
    }
}

/// What to call an artifact directory in a list narrow enough to read.
///
/// The last two components: the directory and the project it belongs to, which
/// is what tells two `target`s apart. The full path is on the candidates
/// screen, where there is room for it and where it is about to be acted on.
pub(super) fn label_for(path: &Path) -> String {
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
