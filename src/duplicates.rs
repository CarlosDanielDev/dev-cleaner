//! The same package, at the same version, installed into many projects.
//!
//! This is a report and nothing else. Nothing here becomes a `Candidate`: every
//! package counted below already sits inside the `node_modules` that is offered
//! for deletion, so offering both would count the same bytes twice in a plan
//! total. The bytes named here are not additional reclaimable space.
//!
//! Two numbers could be reported and only one of them is true. Adding up the N
//! copies of a package promises the whole package back; collapsing N copies
//! into one frees N-1 of them, because one copy has to stay. The second is what
//! this reports.
//!
//! The lockfiles say what a project resolved; they almost never say how large
//! it is. Sizes therefore come from the disk, measured with [`Usage`] over the
//! whole set of copies at once so that a package pnpm has already hardlinked
//! into a shared store reads as shared rather than as N-1 duplicates.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::{Path, PathBuf};

use crate::classify::lockfiles_in;
use crate::scan::{FileMeta, Usage};

/// One `name@version` held by more than one project.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Duplicate {
    pub name: String,
    pub version: String,
    /// Projects holding a copy on disk. Not the number of projects whose
    /// lockfile names it: a lockfile records what was resolved, and a package
    /// that was resolved but never installed holds no bytes.
    pub projects: usize,
    /// How many of those copies were measured rather than inferred.
    pub measured: usize,
    /// What collapsing every copy into one would free. Hardlinked copies
    /// already share their blocks, so their union is a single copy and this is
    /// zero.
    pub bytes: u64,
    /// True when at least one copy's contribution was inferred. Such a row is
    /// never added to the measured total.
    pub estimated: bool,
}

/// Every duplicated package a scan found, and what could not be sized.
#[derive(Debug, Default)]
pub struct Report {
    /// Sorted by `bytes`, descending. Only packages that duplicate something.
    pub rows: Vec<Duplicate>,
    /// Projects with a lockfile that parsed.
    pub projects: usize,
    /// Distinct `name@version` across every lockfile read.
    pub packages: usize,
    /// Of those, how many are named by two or more projects.
    pub shared: usize,
    /// Of the shared ones, how many had two or more copies inside the scanned
    /// roots and so reached the arithmetic at all.
    pub sized: usize,
    /// Lockfiles that could not be read, each naming its path.
    pub warnings: Vec<String>,
}

impl Report {
    /// Duplicated bytes that were measured on disk, start to finish.
    pub fn measured_bytes(&self) -> u64 {
        self.rows
            .iter()
            .filter(|d| !d.estimated)
            .map(|d| d.bytes)
            .sum()
    }

    /// Duplicated bytes from rows where at least one copy was inferred. Kept
    /// apart from the measured total so an estimate is never presented beside a
    /// number that came off the disk.
    pub fn estimated_bytes(&self) -> u64 {
        self.rows
            .iter()
            .filter(|d| d.estimated)
            .map(|d| d.bytes)
            .sum()
    }

    /// Shared packages with fewer than two copies inside the scanned roots.
    ///
    /// Named rather than rendered as zero duplicated bytes. Either the projects
    /// never installed — a lockfile records what was resolved, not what is on
    /// disk — or the toolchain installs into a store outside the project, as
    /// cargo, poetry and bundler do, where the copies are already shared.
    pub fn unmeasurable(&self) -> usize {
        self.shared - self.sized
    }

    /// Packages that were measured and turned out to duplicate nothing: every
    /// copy is the same inode, already hardlinked into a shared store.
    pub fn already_shared(&self) -> usize {
        self.sized - self.rows.len()
    }
}

/// Every installed package in a walk, by the project that owns it and its name.
type Installed<'a> = HashMap<PathBuf, HashMap<String, Vec<&'a FileMeta>>>;

/// Build the duplicate report from a finished walk.
pub fn report(files: &[FileMeta]) -> Report {
    let (lockfiles, warnings) = lockfiles_in(files);
    let installed = installed_packages(files);

    // Which projects resolved each package, and which names a single project
    // resolved at more than one version.
    let mut by_package: BTreeMap<(&str, &str), BTreeSet<&Path>> = BTreeMap::new();
    let mut versions: BTreeMap<(&Path, &str), BTreeSet<&str>> = BTreeMap::new();
    let mut projects: BTreeSet<&Path> = BTreeSet::new();

    for lock in &lockfiles {
        let Some(project) = lock.path.parent() else {
            continue;
        };
        projects.insert(project);
        for p in &lock.packages {
            by_package
                .entry((&p.name, &p.version))
                .or_default()
                .insert(project);
            versions
                .entry((project, &p.name))
                .or_default()
                .insert(&p.version);
        }
    }

    let mut out = Report {
        projects: projects.len(),
        packages: by_package.len(),
        warnings,
        ..Report::default()
    };

    for ((name, version), in_projects) in &by_package {
        if in_projects.len() < 2 {
            continue;
        }
        out.shared += 1;

        let mut copies: Vec<&Vec<&FileMeta>> = Vec::new();
        let mut unattributable = 0usize;
        for project in in_projects {
            let Some(here) = installed.get(*project) else {
                // Nothing installed at all. The lockfile is a list of what was
                // resolved, not of what is on disk, and a project holding no
                // bytes cannot be holding a duplicate of them.
                continue;
            };
            // ponytail: a project that installed two versions of one name has
            // one set of files covering both, and nothing here says which files
            // belong to which version. Ceiling, measured on the reference
            // corpus: it drops lru-cache, held at 5.1.1 top level and at 11.5.2
            // under jsdom in two projects, and with it 2.89 MB of 121.38 MB.
            // The row is omitted rather than attributed to either version, so
            // the error is always downward. To lift it, read the version from
            // each package directory's own package.json, but only for the names
            // a project resolved more than once.
            let split = versions
                .get(&(*project, *name))
                .is_some_and(|v| v.len() > 1);
            match here.get(*name) {
                Some(files) if !split => copies.push(files),
                // Installed, but its bytes cannot be attributed to one version.
                // A copy is certainly there, so its size is inferred from the
                // copies that could be measured and the row says so.
                Some(_) => unattributable += 1,
                // Not on disk, and that is the answer rather than a gap. A
                // lockfile records what was resolved, not what was installed:
                // optional and platform-specific packages are routinely
                // resolved and never installed, so counting this as a copy
                // would invent one. Checked against the corpus, where fsevents
                // resolved in five projects and exists in two.
                None => {}
            }
        }

        let holders = copies.len() + unattributable;
        // Nothing measured means nothing to scale an estimate from either, and
        // inventing a size would be a prediction wearing a result's clothes.
        // Counted by `unmeasurable()`, never rendered as zero.
        if copies.is_empty() || holders < 2 {
            continue;
        }
        out.sized += 1;

        // Measured over the whole set at once, never summed per project. pnpm,
        // uv and cargo hardlink into a shared store, so one inode is reachable
        // from every project holding the package; the union counts those blocks
        // once and the row correctly reads as nothing duplicated.
        let union = Usage::of(copies.iter().flat_map(|c| c.iter().copied())).bytes_unique;

        let mut sizes: Vec<u64> = copies
            .iter()
            .map(|c| Usage::of(c.iter().copied()).bytes_unique)
            .collect();
        sizes.sort_unstable();
        let keep = *sizes.last().expect("copies is not empty");

        // One copy has to stay, so it is the union less the copy that remains.
        // For independent copies that is size x (N-1); for hardlinked ones the
        // union is already a single copy and this is zero.
        let measured = union.saturating_sub(keep);
        let bytes = measured + inferred(measured, &sizes, unattributable);

        if bytes == 0 {
            continue;
        }
        out.rows.push(Duplicate {
            name: name.to_string(),
            version: version.to_string(),
            projects: holders,
            measured: copies.len(),
            bytes,
            estimated: unattributable > 0,
        });
    }

    out.rows.sort_by(|a, b| {
        b.bytes
            .cmp(&a.bytes)
            .then_with(|| (&a.name, &a.version).cmp(&(&b.name, &b.version)))
    });
    out
}

/// What the copies that are on disk but could not be attributed contribute.
///
/// Scaled by the duplication the measured copies actually showed, not by their
/// size: where two or more copies were measured, what one extra copy costs on
/// this disk is observed rather than assumed, and a set that turned out to be
/// hardlinked infers zero instead of inventing N-1 independent copies. With a
/// single copy measured there is no sharing to observe, so its own size is what
/// a second copy would cost.
fn inferred(measured: u64, sizes: &[u64], unattributable: usize) -> u64 {
    if unattributable == 0 {
        return 0;
    }
    let per_copy = match sizes.len() {
        // ponytail: the upper middle on an even count. Splitting the difference
        // would invent a size no copy has.
        0 | 1 => sizes.first().copied().unwrap_or_default(),
        n => measured / (n as u64 - 1),
    };
    per_copy.saturating_mul(unattributable as u64)
}

/// Group every installed node package in a walk by its project and its name.
///
/// One pass. Asking the walk for each package's files separately is a scan of
/// a million paths per package.
fn installed_packages(files: &[FileMeta]) -> Installed<'_> {
    let mut installed: Installed<'_> = HashMap::new();
    for f in files {
        if let Some((project, name)) = node_package(&f.path) {
            installed
                .entry(project)
                .or_default()
                .entry(name)
                .or_default()
                .push(f);
        }
    }
    installed
}

/// The project and package a file under `node_modules` belongs to.
///
/// The outermost `node_modules` names the project, so a hoisted dependency is
/// attributed to the install that holds it. The innermost names the package,
/// which is what makes one rule cover every layout in use: npm's flat tree,
/// npm's nested fallback, and pnpm's store, where the real files live under
/// `.pnpm/<name>@<version>/node_modules/<name>` and the top-level entry is a
/// symlink the walk does not follow.
fn node_package(path: &Path) -> Option<(PathBuf, String)> {
    let parts: Vec<_> = path.components().collect();
    let first = parts.iter().position(|c| c.as_os_str() == "node_modules")?;
    let last = parts
        .iter()
        .rposition(|c| c.as_os_str() == "node_modules")?;

    let head = parts.get(last + 1)?.as_os_str().to_str()?;
    // A leading @ is a scope, which is a directory of its own.
    let name = if let Some(scope) = head.strip_prefix('@') {
        let tail = parts.get(last + 2)?.as_os_str().to_str()?;
        format!("@{scope}/{tail}")
    } else {
        head.to_string()
    };

    Some((parts[..first].iter().collect(), name))
}
