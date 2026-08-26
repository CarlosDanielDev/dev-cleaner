use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::SystemTime;

use super::{EntryRow, ProjectRow, Snapshot, StoredSafety};
use crate::classify::{CacheEntry, ProjectIndex, artifact_root, last_activity};
use crate::safety::{Guards, RegenCommand, Safety};
use crate::scan::{FileMeta, Usage};

/// Turn a finished walk into the snapshot the store keeps.
///
/// Measures each entry on its own rather than reusing the scan-wide total: an
/// inode reachable from two directories counts once inside each of them, which
/// is what deleting that one directory would actually return.
pub fn snapshot(
    started_at: SystemTime,
    roots: &[PathBuf],
    files: &[FileMeta],
    projects: &ProjectIndex,
    guards: &Guards,
    caches: &[(CacheEntry, Usage)],
) -> Snapshot {
    let total = Usage::of(files);

    // Keyed by path, so an artifact directory and a cache pointing at the same
    // place cannot both be recorded. Artifacts go in first and win, because the
    // artifact entry is the one the purge path would offer.
    let mut entries: BTreeMap<PathBuf, EntryRow> = BTreeMap::new();

    // Borrowed, not copied: a real walk holds on the order of a million files.
    let mut grouped: BTreeMap<PathBuf, (Vec<&FileMeta>, &'static str, &'static str)> =
        BTreeMap::new();
    for file in files {
        if let Some((root, kind)) = artifact_root(&file.path) {
            let group = grouped
                .entry(root)
                .or_insert_with(|| (Vec::new(), kind.dir_name, kind.regen));
            group.0.push(file);
        }
    }

    for (path, (files, dir_name, regen)) in grouped {
        let usage = Usage::of(files);
        let row = EntryRow {
            project: projects.owner_of(&path).map(|p| p.root.clone()),
            kind: dir_name.to_string(),
            bytes_apparent: usage.bytes_apparent,
            bytes_unique: usage.bytes_unique,
            inodes: usage.inodes,
            safety: StoredSafety::from(&tier_for(&path, guards, regen)),
            path: path.clone(),
        };
        entries.insert(path, row);
    }

    for (cache, usage) in caches {
        entries
            .entry(cache.path.clone())
            .or_insert_with(|| EntryRow {
                project: None,
                path: cache.path.clone(),
                kind: cache.kind.name.to_string(),
                bytes_apparent: usage.bytes_apparent,
                bytes_unique: usage.bytes_unique,
                inodes: usage.inodes,
                safety: StoredSafety::Cache {
                    refills_on: cache.kind.regen.to_string(),
                },
            });
    }

    Snapshot {
        started_at,
        roots: roots.to_vec(),
        total_bytes_apparent: total.bytes_apparent,
        total_bytes_unique: total.bytes_unique,
        total_inodes: total.inodes,
        projects: projects
            .projects()
            .map(|p| ProjectRow {
                path: p.root.clone(),
                kind: p
                    .ecosystems
                    .iter()
                    .map(|e| e.name())
                    .collect::<Vec<_>>()
                    .join(","),
                // ponytail: not read here. Both answers cost more than a walk
                // should: the remote means parsing `.git/config`, and `dirty`
                // means a `git status` for every project on the machine. The
                // purge path already determines `dirty` per candidate, which is
                // where the answer actually changes a decision.
                vcs_remote: None,
                last_commit_at: last_activity(&p.root),
                dirty: None,
            })
            .collect(),
        entries: entries.into_values().collect(),
    }
}

/// The tier this directory would be offered under, if it were offered.
///
/// Recorded rather than inferred later. The history is a record of what the
/// tool concluded at the time, and a guard that blocked a directory last month
/// is part of that record even if it would pass today.
fn tier_for(path: &std::path::Path, guards: &Guards, regen: &'static str) -> Safety {
    match guards.check(path) {
        Err(reason) => Safety::Protected { reason },
        Ok(()) => match RegenCommand::new(regen) {
            Some(regen) => Safety::Regenerable { regen },
            None => Safety::for_unknown("registry entry has no command"),
        },
    }
}
