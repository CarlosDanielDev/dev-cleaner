//! Turning a scan into the set of things that may be offered for deletion.

use std::collections::BTreeMap;
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

/// Every artifact directory in a scan, with the files that live under it.
///
/// Grouped by the outermost artifact directory on each path, which is the one a
/// user would actually delete. Files are borrowed rather than copied: a real
/// walk holds on the order of a million of them.
///
/// The one place this grouping is defined. Anything that totals an artifact
/// directory measures the group with [`Usage`], so no caller can reintroduce a
/// total that counts a hardlinked inode once per path.
pub fn group_by_artifact_root(
    files: &[FileMeta],
) -> BTreeMap<PathBuf, (Vec<&FileMeta>, &'static ArtifactKind)> {
    let mut grouped: BTreeMap<PathBuf, (Vec<&FileMeta>, &'static ArtifactKind)> = BTreeMap::new();
    for file in files {
        if let Some((root, kind)) = artifact_root(&file.path) {
            grouped
                .entry(root)
                .or_insert_with(|| (Vec::new(), kind))
                .0
                .push(file);
        }
    }
    grouped
}

/// Group scanned files into candidates, one per artifact directory.
///
/// Only registered artifact directories are ever considered. Source files are
/// not candidates under any circumstances: the registry is an allowlist, not a
/// set of heuristics.
pub fn from_scan(files: &[FileMeta], guards: &Guards) -> Build {
    let mut build = Build::default();

    for (path, (group, kind)) in group_by_artifact_root(files) {
        // Allocated blocks with each inode counted once, so the number offered
        // is the number deletion returns. pnpm, uv and cargo all hardlink, and
        // summing per path would promise the same blocks several times over.
        let bytes = Usage::of(group).bytes_unique;
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
