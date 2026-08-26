use std::collections::HashMap;
use std::path::PathBuf;
use std::time::SystemTime;

use rusqlite::OptionalExtension;

use super::{Result, Store, from_nanos, join_paths, path_str, split_paths, to_nanos};
use crate::safety::{BlockReason, Safety};

/// One scan, as it is stored and as it comes back.
///
/// The shape is deliberately flat and owns its data: a snapshot read from the
/// store must stand on its own months later, without the walk that produced it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Snapshot {
    pub started_at: SystemTime,
    pub roots: Vec<PathBuf>,
    /// Sum of `st_size`. Reported alongside, never instead of, the unique total.
    pub total_bytes_apparent: u64,
    /// Allocated blocks, each inode counted once. What deletion actually returns.
    pub total_bytes_unique: u64,
    pub total_inodes: u64,
    pub projects: Vec<ProjectRow>,
    pub entries: Vec<EntryRow>,
}

/// A project as it stood during one scan.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectRow {
    pub path: PathBuf,
    /// The ecosystems its markers imply, comma separated.
    pub kind: String,
    pub vcs_remote: Option<String>,
    pub last_commit_at: Option<SystemTime>,
    /// `None` where the scan did not determine it. Answering costs a `git
    /// status` per project, which a read-only walk deliberately does not pay;
    /// the purge path determines it per candidate instead.
    pub dirty: Option<bool>,
}

/// One measured directory: a build artifact, or a global cache.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EntryRow {
    /// The project that owns it, by path. `None` for a cache outside every
    /// project, which is most of the reclaimable space on a real machine.
    pub project: Option<PathBuf>,
    pub path: PathBuf,
    pub kind: String,
    pub bytes_apparent: u64,
    pub bytes_unique: u64,
    pub inodes: u64,
    pub safety: StoredSafety,
}

/// [`Safety`], in the form the store keeps.
///
/// Identical in content, but owning its strings. `Safety::Cache` carries a
/// `&'static str` that comes from the compiled-in registry, and no value read
/// back from a database can be one, so storing the live type would either leak
/// memory or quietly lose the payload.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StoredSafety {
    Cache { refills_on: String },
    Regenerable { regen: String },
    Unproven { reason: String },
    Protected { reason: BlockReason },
}

impl From<&Safety> for StoredSafety {
    fn from(safety: &Safety) -> Self {
        match safety {
            Safety::Cache { refills_on } => StoredSafety::Cache {
                refills_on: (*refills_on).to_string(),
            },
            Safety::Regenerable { regen } => StoredSafety::Regenerable {
                regen: regen.as_str().to_string(),
            },
            Safety::Unproven { reason } => StoredSafety::Unproven {
                reason: reason.clone(),
            },
            Safety::Protected { reason } => StoredSafety::Protected { reason: *reason },
        }
    }
}

impl StoredSafety {
    /// The tier, and the payload that tier carries.
    fn columns(&self) -> (&'static str, String) {
        match self {
            StoredSafety::Cache { refills_on } => ("cache", refills_on.clone()),
            StoredSafety::Regenerable { regen } => ("regenerable", regen.clone()),
            StoredSafety::Unproven { reason } => ("unproven", reason.clone()),
            StoredSafety::Protected { reason } => ("protected", reason.tag().to_string()),
        }
    }

    /// Decode a stored tier.
    ///
    /// Anything unrecognised becomes `Unproven`, never a selectable tier. A
    /// database written by a newer version, or edited by hand, must not be able
    /// to make something offerable that this build cannot vouch for.
    fn from_columns(tag: &str, detail: String) -> Self {
        match tag {
            "cache" => StoredSafety::Cache { refills_on: detail },
            "regenerable" => StoredSafety::Regenerable { regen: detail },
            "protected" => match BlockReason::from_tag(&detail) {
                Some(reason) => StoredSafety::Protected { reason },
                None => StoredSafety::Unproven {
                    reason: format!("stored as protected with an unknown reason: {detail}"),
                },
            },
            _ => StoredSafety::Unproven { reason: detail },
        }
    }
}

/// A root set, order and repetition removed.
fn as_set(roots: &[PathBuf]) -> std::collections::BTreeSet<&PathBuf> {
    roots.iter().collect()
}

impl Store {
    /// Write a scan, returning its id.
    ///
    /// One transaction for the whole snapshot. A reader therefore sees a scan
    /// either complete or absent, never half of one, however many scans are
    /// committing at the same time.
    pub fn write_snapshot(&mut self, snap: &Snapshot) -> Result<i64> {
        let tx = self.conn.transaction()?;

        tx.execute(
            "INSERT INTO scan (started_at, root_set, total_bytes_apparent, \
             total_bytes_unique, total_inodes) VALUES (?1, ?2, ?3, ?4, ?5)",
            rusqlite::params![
                to_nanos(snap.started_at),
                join_paths(&snap.roots),
                snap.total_bytes_apparent as i64,
                snap.total_bytes_unique as i64,
                snap.total_inodes as i64,
            ],
        )?;
        let scan_id = tx.last_insert_rowid();

        let mut project_ids: HashMap<&PathBuf, i64> = HashMap::new();
        {
            let mut insert = tx.prepare(
                "INSERT INTO project (scan_id, path, kind, vcs_remote, last_commit_at, dirty) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            )?;
            for p in &snap.projects {
                insert.execute(rusqlite::params![
                    scan_id,
                    path_str(&p.path),
                    p.kind,
                    p.vcs_remote,
                    p.last_commit_at.map(to_nanos),
                    p.dirty,
                ])?;
                project_ids.insert(&p.path, tx.last_insert_rowid());
            }

            let mut insert = tx.prepare(
                "INSERT INTO entry (scan_id, project_id, path, kind, bytes_apparent, \
                 bytes_unique, inodes, safety, safety_detail) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            )?;
            for e in &snap.entries {
                let (safety, detail) = e.safety.columns();
                insert.execute(rusqlite::params![
                    scan_id,
                    e.project.as_ref().and_then(|p| project_ids.get(p)),
                    path_str(&e.path),
                    e.kind,
                    e.bytes_apparent as i64,
                    e.bytes_unique as i64,
                    e.inodes as i64,
                    safety,
                    detail,
                ])?;
            }
        }

        tx.commit()?;
        Ok(scan_id)
    }

    /// Read a scan back.
    ///
    /// Rows come back in insertion order, so the snapshot is reproduced as it
    /// was written rather than in whatever order the query planner prefers.
    pub fn read_snapshot(&self, scan_id: i64) -> Result<Snapshot> {
        let (started_at, root_set, apparent, unique, inodes) = self.conn.query_row(
            "SELECT started_at, root_set, total_bytes_apparent, total_bytes_unique, \
             total_inodes FROM scan WHERE id = ?1",
            [scan_id],
            |r| {
                Ok((
                    r.get::<_, i64>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, i64>(2)?,
                    r.get::<_, i64>(3)?,
                    r.get::<_, i64>(4)?,
                ))
            },
        )?;

        let mut stmt = self.conn.prepare(
            "SELECT path, kind, vcs_remote, last_commit_at, dirty FROM project \
             WHERE scan_id = ?1 ORDER BY id",
        )?;
        let projects = stmt
            .query_map([scan_id], |r| {
                Ok(ProjectRow {
                    path: PathBuf::from(r.get::<_, String>(0)?),
                    kind: r.get(1)?,
                    vcs_remote: r.get(2)?,
                    last_commit_at: r.get::<_, Option<i64>>(3)?.map(from_nanos),
                    dirty: r.get(4)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;

        let mut stmt = self.conn.prepare(
            "SELECT p.path, e.path, e.kind, e.bytes_apparent, e.bytes_unique, e.inodes, \
             e.safety, e.safety_detail FROM entry e \
             LEFT JOIN project p ON p.id = e.project_id \
             WHERE e.scan_id = ?1 ORDER BY e.rowid",
        )?;
        let entries = stmt
            .query_map([scan_id], |r| {
                Ok(EntryRow {
                    project: r.get::<_, Option<String>>(0)?.map(PathBuf::from),
                    path: PathBuf::from(r.get::<_, String>(1)?),
                    kind: r.get(2)?,
                    bytes_apparent: r.get::<_, i64>(3)? as u64,
                    bytes_unique: r.get::<_, i64>(4)? as u64,
                    inodes: r.get::<_, i64>(5)? as u64,
                    safety: StoredSafety::from_columns(&r.get::<_, String>(6)?, r.get(7)?),
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;

        Ok(Snapshot {
            started_at: from_nanos(started_at),
            roots: split_paths(&root_set),
            total_bytes_apparent: apparent as u64,
            total_bytes_unique: unique as u64,
            total_inodes: inodes as u64,
            projects,
            entries,
        })
    }

    /// The most recently written scan, if there is one.
    pub fn latest_scan(&self) -> Result<Option<i64>> {
        Ok(self
            .conn
            .query_row("SELECT MAX(id) FROM scan", [], |r| r.get(0))
            .optional()?
            .flatten())
    }

    /// The most recent scan that looked at the same roots.
    ///
    /// A trend only means anything between scans of the same territory. Diffing
    /// against a scan of a wider root set reports every path outside the
    /// narrower one as removed, when nothing was deleted at all.
    ///
    /// Roots are compared as a set: the same directories in a different order,
    /// or one named twice, are the same territory.
    pub fn latest_scan_for(&self, roots: &[PathBuf]) -> Result<Option<i64>> {
        let wanted = as_set(roots);
        let mut stmt = self
            .conn
            .prepare("SELECT id, root_set FROM scan ORDER BY id DESC")?;
        let mut rows = stmt.query([])?;
        while let Some(row) = rows.next()? {
            let stored: String = row.get(1)?;
            if as_set(&split_paths(&stored)) == wanted {
                return Ok(Some(row.get(0)?));
            }
        }
        Ok(None)
    }

    /// Every scan, oldest first.
    pub fn scan_ids(&self) -> Result<Vec<i64>> {
        let mut stmt = self.conn.prepare("SELECT id FROM scan ORDER BY id")?;
        let ids = stmt
            .query_map([], |r| r.get(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(ids)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::UNIX_EPOCH;

    /// A tier this build does not recognise must never decode to one the
    /// cursor can reach. A database written by a newer version, restored from
    /// a backup, or edited by hand is untrusted input like any other.
    #[test]
    fn an_unrecognised_tier_decodes_to_something_unselectable() {
        for tag in [
            "",
            "regenerable_v2",
            "safe",
            "cache ",
            "PROTECTED",
            "deletable",
        ] {
            let decoded = StoredSafety::from_columns(tag, "npm install".into());
            assert!(
                matches!(decoded, StoredSafety::Unproven { .. }),
                "the tag {tag:?} decoded to {decoded:?}, which is not the unproven tier"
            );
        }
        // A known tier with an unknown payload is the same problem one level
        // down: it must not become a block reason this build cannot explain.
        let decoded = StoredSafety::from_columns("protected", "reason_from_the_future".into());
        assert!(matches!(decoded, StoredSafety::Unproven { .. }));
    }

    /// A write that fails part way must leave no scan behind.
    ///
    /// A scan row with only some of its entries is worse than no scan at all:
    /// the trend view would read the missing paths as removed and tell the user
    /// space came back that never went anywhere.
    ///
    /// Lives here rather than in `tests/` because forcing the failure needs the
    /// connection: capping the page count makes SQLite report a full disk part
    /// way through the insert, which is the real-world version of this.
    #[test]
    fn a_write_that_fails_part_way_leaves_no_scan() {
        let dir = tempfile::TempDir::new().expect("tempdir");
        let mut store = Store::open(&dir.path().join("history.sqlite3")).expect("open");

        let snap = Snapshot {
            started_at: UNIX_EPOCH,
            roots: vec![PathBuf::from("/r")],
            total_bytes_apparent: 1,
            total_bytes_unique: 1,
            total_inodes: 1,
            projects: Vec::new(),
            entries: (0..20_000)
                .map(|i| EntryRow {
                    project: None,
                    path: PathBuf::from(format!("/r/{i}/node_modules")),
                    kind: "node_modules".into(),
                    bytes_apparent: i,
                    bytes_unique: i,
                    inodes: i,
                    safety: StoredSafety::Regenerable {
                        regen: "npm install".into(),
                    },
                })
                .collect(),
        };

        let pages: i64 = store
            .conn
            .query_row("PRAGMA page_count", [], |r| r.get(0))
            .expect("page count");
        store
            .conn
            .pragma_update(None, "max_page_count", pages + 2)
            .expect("cap the file");

        let err = store
            .write_snapshot(&snap)
            .expect_err("the write must fail");
        assert!(
            format!("{err}").contains("full"),
            "expected a full-disk failure, got: {err}"
        );

        store
            .conn
            .pragma_update(None, "max_page_count", 1_073_741_823_i64)
            .expect("lift the cap");
        assert_eq!(
            store.scan_ids().expect("ids"),
            Vec::<i64>::new(),
            "a half-written scan survived the failure"
        );
    }
}
