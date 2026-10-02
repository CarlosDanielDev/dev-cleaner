//! What the tool remembers of its own runs.
//!
//! One row per purge, written by the interface and the command line alike, so
//! the result screen can set a run beside the ones before it. The markdown
//! manifest is for a person to read afterwards; reading numbers back out of it
//! would make a renderer into a parser, and the store is already here.

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use super::{Result, Store, from_nanos, path_str, to_nanos};
use crate::purge::Manifest;

/// One run, as stored.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PurgeRun {
    pub id: i64,
    pub executed_at: SystemTime,
    pub items_planned: u64,
    pub items_moved: u64,
    pub items_failed: u64,
    pub items_skipped: u64,
    pub bytes_expected: u64,
    pub bytes_moved: u64,
    pub elapsed: Duration,
    /// Where the record was written, or `None` when writing it failed.
    pub manifest_path: Option<PathBuf>,
}

impl PurgeRun {
    /// Nothing failed, nothing was left out, and something moved.
    fn is_whole(&self) -> bool {
        self.items_failed == 0 && self.items_skipped == 0 && self.items_moved > 0
    }
}

/// All the runs at a glance, and where one of them stands among the rest.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunSummary {
    pub runs: usize,
    /// When the oldest run on record was made.
    pub since: SystemTime,
    pub bytes_moved: u64,
    /// The quickest run that moved everything it set out to. A run that gave
    /// up early is quick for the wrong reason.
    pub fastest: Option<Duration>,
    pub largest: u64,
    /// 1 for the largest run, among those stored, of the run asked about. `None`
    /// when that run is not among them.
    pub this_rank: Option<usize>,
}

/// Read a summary off `runs`, ranking the one with id `this`.
///
/// Nothing here looks at a manifest: every number is a column.
pub fn summarize(runs: &[PurgeRun], this: Option<i64>) -> Option<RunSummary> {
    let since = runs.iter().map(|r| r.executed_at).min()?;
    let this_rank = this.and_then(|id| {
        let mine = runs.iter().find(|r| r.id == id)?;
        Some(
            1 + runs
                .iter()
                .filter(|r| r.bytes_moved > mine.bytes_moved)
                .count(),
        )
    });
    Some(RunSummary {
        runs: runs.len(),
        since,
        bytes_moved: runs.iter().map(|r| r.bytes_moved).sum(),
        fastest: runs
            .iter()
            .filter(|r| r.is_whole())
            .map(|r| r.elapsed)
            .min(),
        largest: runs.iter().map(|r| r.bytes_moved).max().unwrap_or(0),
        this_rank,
    })
}

impl Store {
    /// Remember a run, returning its id.
    pub fn record_purge(&self, manifest: &Manifest, record: Option<&Path>) -> Result<i64> {
        self.conn.execute(
            "INSERT INTO purge (executed_at, items_planned, items_moved, items_failed, \
             items_skipped, bytes_expected, bytes_moved, elapsed_ms, manifest_path) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            rusqlite::params![
                to_nanos(manifest.executed_at),
                manifest.planned as i64,
                manifest.removed().count() as i64,
                manifest.failed().count() as i64,
                manifest.skipped().count() as i64,
                manifest.bytes_expected as i64,
                manifest.bytes_moved() as i64,
                manifest.elapsed.as_millis() as i64,
                record.map(path_str),
            ],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    /// Every run, oldest first.
    pub fn purge_runs(&self) -> Result<Vec<PurgeRun>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, executed_at, items_planned, items_moved, items_failed, \
             items_skipped, bytes_expected, bytes_moved, elapsed_ms, manifest_path \
             FROM purge ORDER BY id",
        )?;
        let rows = stmt
            .query_map([], |r| {
                Ok(PurgeRun {
                    id: r.get(0)?,
                    executed_at: from_nanos(r.get(1)?),
                    items_planned: r.get::<_, i64>(2)? as u64,
                    items_moved: r.get::<_, i64>(3)? as u64,
                    items_failed: r.get::<_, i64>(4)? as u64,
                    items_skipped: r.get::<_, i64>(5)? as u64,
                    bytes_expected: r.get::<_, i64>(6)? as u64,
                    bytes_moved: r.get::<_, i64>(7)? as u64,
                    elapsed: Duration::from_millis(r.get::<_, i64>(8)? as u64),
                    manifest_path: r.get::<_, Option<String>>(9)?.map(PathBuf::from),
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }
}

/// Open the store at `db` and remember `manifest` in it.
///
/// The one door the command line and the interface both go through, so the two
/// cannot come to write different rows for the same run.
pub fn record_purge_run(db: &Path, manifest: &Manifest, record: Option<&Path>) -> Result<i64> {
    Store::open(db)?.record_purge(manifest, record)
}
