//! Turning a scan into the set of things that may be offered for deletion.

use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;

use crate::classify::{ArtifactKind, artifact_root};
use crate::safety::{Candidate, Guards, RegenCommand, Rejected, Safety};
use crate::scan::{FileMeta, Usage};

/// What a scan yielded: what may be offered, and what was refused and why.
///
/// Rejections are kept rather than discarded. A user who cannot see why a
/// directory is missing from the list has no way to act on it, and silence
/// reads as "there was nothing there".
#[derive(Debug, Default)]
pub struct Build {
    pub candidates: Vec<Candidate>,
    pub rejected: Vec<Rejected>,
}

/// What [`group_by_artifact_root`] returns, named so a scan can compute it once
/// and hand it to everything that needs it.
pub type Grouped<'a> = BTreeMap<PathBuf, (Vec<&'a FileMeta>, &'static ArtifactKind)>;

/// Every artifact directory in a scan, with the files that live under it.
///
/// Grouped by the outermost artifact directory on each path, which is the one a
/// user would actually delete. Files are borrowed rather than copied: a real
/// walk holds on the order of a million of them.
///
/// The one place this grouping is defined. Anything that totals an artifact
/// directory measures the group with [`Usage`], so no caller can reintroduce a
/// total that counts a hardlinked inode once per path.
pub fn group_by_artifact_root(files: &[FileMeta]) -> Grouped<'_> {
    // The walk yields a directory's files together, so a file usually sits under
    // the root the previous one found. One prefix test then replaces both the
    // root lookup and the ordered-map search; the map the callers iterate is
    // built once, from the roots rather than from every file.
    let mut groups: Vec<(PathBuf, (Vec<&FileMeta>, &'static ArtifactKind))> = Vec::new();
    let mut index: HashMap<PathBuf, usize> = HashMap::new();
    let mut last: Option<usize> = None;
    for file in files {
        // Under the previous root means this root too: it is the outermost
        // artifact directory on the path, and the files share that prefix.
        if let Some(i) = last
            && file.path.starts_with(&groups[i].0)
        {
            groups[i].1.0.push(file);
            continue;
        }
        let Some((root, kind)) = artifact_root(&file.path) else {
            continue;
        };
        let i = *index.entry(root.clone()).or_insert_with(|| {
            groups.push((root, (Vec::new(), kind)));
            groups.len() - 1
        });
        groups[i].1.0.push(file);
        last = Some(i);
    }
    groups.into_iter().collect()
}

/// Group scanned files into candidates, one per artifact directory.
///
/// Only registered artifact directories are ever considered. Source files are
/// not candidates under any circumstances: the registry is an allowlist, not a
/// set of heuristics.
pub fn from_scan(files: &[FileMeta], guards: &Guards) -> Build {
    from_groups(&group_by_artifact_root(files), guards)
}

/// [`from_scan`] over a grouping the caller already has.
pub fn from_groups(grouped: &Grouped<'_>, guards: &Guards) -> Build {
    let mut build = Build::default();

    for (path, (group, kind)) in grouped {
        let path = path.clone();
        // Allocated blocks with each inode counted once, so the number offered
        // is the number deletion returns. pnpm, uv and cargo all hardlink, and
        // summing per path would promise the same blocks several times over.
        let bytes = Usage::of(group.iter().copied()).bytes_unique;
        let regen = kind.regen;
        match guards.check(&path) {
            Ok(()) => build.candidates.push(Candidate {
                path,
                bytes,
                safety: match RegenCommand::new(regen) {
                    Some(regen) => Safety::Regenerable { regen },
                    // Unreachable while the registry's constructor enforces a
                    // non-empty command, but falling back to Unproven keeps the
                    // failure safe rather than selectable.
                    None => Safety::for_unknown("registry entry has no command"),
                },
            }),
            Err(reason) => build.rejected.push(Rejected {
                path,
                because: reason.explain().to_string(),
            }),
        }
    }
    build
}

#[cfg(test)]
mod tests {
    use std::path::Path;
    use std::time::UNIX_EPOCH;

    use super::*;

    fn file(path: &str) -> FileMeta {
        FileMeta {
            path: PathBuf::from(path),
            bytes_apparent: 0,
            bytes_actual: 0,
            dev: 0,
            ino: 0,
            mtime: UNIX_EPOCH,
        }
    }

    /// The per-file lookup this replaced: a `PathBuf` rebuilt per component.
    fn artifact_root_slow(path: &Path) -> Option<(PathBuf, &'static ArtifactKind)> {
        let mut prefix = PathBuf::new();
        for component in path.components() {
            prefix.push(component);
            if let Some(name) = component.as_os_str().to_str()
                && let Some(kind) = crate::classify::artifact_for(name)
            {
                return Some((prefix, kind));
            }
        }
        None
    }

    /// The grouping this replaced, one `BTreeMap` lookup per file.
    fn group_slow(
        files: &[FileMeta],
    ) -> BTreeMap<PathBuf, (Vec<&FileMeta>, &'static ArtifactKind)> {
        let mut grouped = BTreeMap::new();
        for file in files {
            if let Some((root, kind)) = artifact_root_slow(&file.path) {
                let entry: &mut (Vec<&FileMeta>, _) =
                    grouped.entry(root).or_insert_with(|| (Vec::new(), kind));
                entry.0.push(file);
            }
        }
        grouped
    }

    struct Lcg(u64);
    impl Lcg {
        fn next(&mut self, n: usize) -> usize {
            self.0 = self
                .0
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            ((self.0 >> 33) as usize) % n
        }
    }

    fn assert_same(files: &[FileMeta], what: &str) {
        let fast = group_by_artifact_root(files);
        let slow = group_slow(files);
        assert_eq!(
            fast.keys().collect::<Vec<_>>(),
            slow.keys().collect::<Vec<_>>(),
            "{what}: roots"
        );
        for (root, (group, kind)) in &fast {
            let (want, want_kind) = &slow[root];
            assert_eq!(
                kind.dir_name, want_kind.dir_name,
                "{what}: kind of {root:?}"
            );
            let paths = |g: &[&FileMeta]| g.iter().map(|f| f.path.clone()).collect::<Vec<_>>();
            assert_eq!(paths(group), paths(want), "{what}: files of {root:?}");
        }
        for f in files {
            let named = |r: Option<(PathBuf, &ArtifactKind)>| r.map(|(p, k)| (p, k.dir_name));
            assert_eq!(
                named(artifact_root(&f.path)),
                named(artifact_root_slow(&f.path)),
                "{what}: {}",
                f.path.display()
            );
        }
    }

    #[test]
    fn grouping_matches_the_per_file_lookup_on_interleaved_roots() {
        // Files of one root split by files of another, a nested node_modules,
        // a `target` below `src`, and names that only look like a root.
        assert_same(
            &[
                file("/r/app/node_modules/x/a.js"),
                file("/r/app/src/main.js"),
                file("/r/app/node_modules/x/node_modules/y/b.js"),
                file("/r/app/node_modules2/z.js"),
                file("/r/app/node_modules"),
                file("/r/app2/target/debug/a"),
                file("/r/app2/src/target/b"),
                file("/r/app2/src/c.rs"),
                file("/r/app2/target/debug/d"),
                file("/r/app/node_modules/x/e.js"),
                file("/r/target/f"),
                file("/r/app/target/g"),
                file("relative/node_modules/h"),
                file("node_modules/i"),
                file("/node_modules/j"),
            ],
            "fixed",
        );
    }

    #[test]
    fn grouping_matches_the_per_file_lookup_on_random_trees() {
        const NAMES: &[&str] = &[
            "app",
            "src",
            "target",
            "node_modules",
            "Pods",
            "x",
            "target2",
        ];
        let mut rng = Lcg(11);
        for round in 0..200 {
            let files: Vec<FileMeta> = (0..60)
                .map(|_| {
                    let mut p = String::from("/r");
                    for _ in 0..1 + rng.next(6) {
                        p.push('/');
                        p.push_str(NAMES[rng.next(NAMES.len())]);
                    }
                    file(&p)
                })
                .collect();
            assert_same(&files, &format!("round {round}"));
        }
    }
}
