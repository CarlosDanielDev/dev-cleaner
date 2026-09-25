//! What a shared, content-addressed store would recover across projects.
//!
//! An estimate, and it says so everywhere it is rendered. Nobody has run a
//! migration and measured the disk afterwards; this is arithmetic over what a
//! store would collapse, and it is deliberately kept off every screen that
//! shows a measured reclaimable total. The first end-to-end purge in this
//! project reported a prediction in a result's units and blamed hardlinks for
//! space the Trash was still holding. A number in gigabytes is taken for a
//! measurement unless it is labelled otherwise, so this one is labelled.
//!
//! It reports and never runs. `pnpm import` rewrites a project's dependency
//! layout, and a tool whose whole premise is that deletion is provably
//! reversible has not earned the right to do that on a guess. The command is
//! printed for the user to run.
//!
//! The arithmetic is the duplicate report's, unchanged: for each `name@version`
//! held by two or more projects, the blocks its copies occupy with each inode
//! counted once, less the largest single copy, which the store keeps. Copies
//! already hardlinked together are one inode, so their union *is* that largest
//! copy and the row contributes nothing. That is the whole guard against
//! promising back space pnpm has already recovered.

use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::path::{Path, PathBuf};

use crate::classify::{is_inside_artifact, lockfile_for};
use crate::duplicates::{self, Report};
use crate::scan::FileMeta;

/// What the user runs to move one project into the pnpm store.
///
/// Text, printed. Nothing in this crate executes it, and `no_code_path_can_run_
/// a_package_manager` in the test suite holds that open.
pub const MIGRATION_COMMAND: &str = "pnpm import && pnpm install";

/// Lockfiles whose toolchain already installs through a shared store.
///
/// Detected by the lockfile, never by a directory name: `my-pnpm-app` is a
/// name, and `pnpm-lock.yaml` is evidence.
const STORED: &[(&str, &str)] = &[("pnpm-lock.yaml", "pnpm"), ("uv.lock", "uv")];

/// Lockfiles whose toolchain copies packages into the project itself, which is
/// the only situation a store has anything to collapse.
const MIGRATABLE: &[&str] = &["package-lock.json", "yarn.lock"];

/// Why a project took no part in the estimate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Reason {
    /// Already installs through a shared store, named here.
    AlreadyStored(&'static str),
    /// Holds a store's lockfile *and* a copying package manager's. The two
    /// describe two different installs and neither one is authoritative.
    Migrating,
    /// No migration to offer. Cargo hardlinks into `~/.cargo` by default;
    /// bundler and poetry install outside the project to begin with.
    OutOfScope,
}

/// One project and the lockfiles that decided its fate.
#[derive(Debug, Clone)]
pub struct Exclusion {
    pub project: PathBuf,
    pub reason: Reason,
    /// Every lockfile found at the project root, sorted. The report names
    /// these rather than summarising them: a project excluded for holding two
    /// lockfiles is owed the two names.
    pub lockfiles: Vec<&'static str>,
}

/// What a shared store would recover, and who was left out of the sum.
pub struct Estimate {
    /// The duplicate arithmetic, run over the migratable projects alone.
    pub duplicates: Report,
    /// Sorted by path. Every project that has a lockfile and is not counted.
    pub excluded: Vec<Exclusion>,
}

impl Estimate {
    /// Projects that could move to a store.
    pub fn projects(&self) -> usize {
        self.duplicates.projects
    }

    /// The estimate itself: bytes a store would collapse, every copy measured.
    ///
    /// Each inode counted once across the whole estimate, not once per
    /// package. Summing the packages would count npm's hardlinked platform
    /// binaries twice, once under `esbuild` and again under
    /// `@esbuild/darwin-arm64`, and promise back blocks that are one set.
    pub fn bytes(&self) -> u64 {
        self.duplicates.collapsible_bytes()
    }

    /// Bytes from packages where at least one copy's size was inferred rather
    /// than measured. Kept apart so an inference is never folded into a figure
    /// the disk supported.
    pub fn inferred_bytes(&self) -> u64 {
        self.duplicates.estimated_bytes()
    }

    /// Excluded projects sharing one reason, in path order.
    pub fn excluded_for(&self, reason: &Reason) -> Vec<&Exclusion> {
        self.excluded
            .iter()
            .filter(|x| &x.reason == reason)
            .collect()
    }
}

/// Estimate what a shared store would recover across a finished walk.
pub fn estimate(files: &[FileMeta]) -> Estimate {
    let lockfiles = lockfiles_by_project(files);
    let mut migratable: HashSet<&Path> = HashSet::new();
    let mut excluded = Vec::new();

    for (project, names) in &lockfiles {
        let names: Vec<&'static str> = names.iter().copied().collect();
        match verdict(&names) {
            None => {
                migratable.insert(project.as_path());
            }
            Some(reason) => excluded.push(Exclusion {
                project: project.clone(),
                reason,
                lockfiles: names,
            }),
        }
    }

    // ponytail: the migratable projects' files are cloned so the duplicate
    // report can take the slice it already takes. Ceiling: one PathBuf
    // allocation per file that survives the filter, measured at 0.4s over
    // 1.7M paths on the reference corpus. Take an iterator in
    // `duplicates::report` if that ever shows up in a profile.
    let mine: Vec<FileMeta> = files
        .iter()
        .filter(|f| owner(&f.path).is_some_and(|p| migratable.contains(p.as_path())))
        .cloned()
        .collect();

    Estimate {
        duplicates: duplicates::report(&mine),
        excluded,
    }
}

/// Why this set of lockfiles is excluded, or `None` when it can migrate.
///
/// A project holding both a store's lockfile and a copying manager's is in the
/// middle of a migration, and there is no honest way to pick one. Reading the
/// pnpm lockfile would report an install that may not exist yet; reading the
/// npm one would report an install that is on its way out. It leaves the
/// estimate and the report says so, which is the answer a user can act on.
fn verdict(names: &[&'static str]) -> Option<Reason> {
    let store = STORED
        .iter()
        .find(|(lock, _)| names.contains(lock))
        .map(|(_, store)| *store);
    let copies = names.iter().any(|n| MIGRATABLE.contains(n));

    match (store, copies) {
        (Some(_), true) => Some(Reason::Migrating),
        (Some(store), false) => Some(Reason::AlreadyStored(store)),
        (None, true) => None,
        (None, false) => Some(Reason::OutOfScope),
    }
}

/// Every lockfile name sitting at each project root.
///
/// Read from the file names alone, never from a parse. A `pnpm-lock.yaml` that
/// this crate cannot parse still proves the project installs through the pnpm
/// store, and treating a parse failure as absence would quietly count the
/// project as migratable and sell it a migration it already made.
fn lockfiles_by_project(files: &[FileMeta]) -> BTreeMap<PathBuf, BTreeSet<&'static str>> {
    let mut found: BTreeMap<PathBuf, BTreeSet<&'static str>> = BTreeMap::new();

    for file in files {
        // A lockfile under `node_modules` belongs to a dependency, not to a
        // project, on exactly the grounds `ProjectIndex` skips markers there.
        if is_inside_artifact(&file.path) {
            continue;
        }
        let Some(kind) = file
            .path
            .file_name()
            .and_then(|n| n.to_str())
            .and_then(lockfile_for)
        else {
            continue;
        };
        let Some(root) = file.path.parent() else {
            continue;
        };
        found
            .entry(root.to_path_buf())
            .or_default()
            .insert(kind.file_name);
    }

    found
}

/// The project a file is attributed to.
///
/// The path above the outermost `node_modules`, matching how the duplicate
/// report attributes an installed package, and the containing directory for
/// everything else, which is where a lockfile lives. A file the duplicate
/// report would never look at maps somewhere harmless and is dropped, which
/// only makes the report faster.
fn owner(path: &Path) -> Option<PathBuf> {
    let parts: Vec<_> = path.components().collect();
    match parts.iter().position(|c| c.as_os_str() == "node_modules") {
        Some(at) => Some(parts[..at].iter().collect()),
        None => path.parent().map(Path::to_path_buf),
    }
}
