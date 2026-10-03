//! The size a finished scan had, kept so the next one can say how far along it is.
//!
//! Written only for a scan that ran to its end, in the transaction that records
//! the scan: a cancelled or failed scan has no entry count worth comparing
//! against, and a bar measured against half a scan would lie high.

use std::path::{Path, PathBuf};
use std::time::Duration;

use rusqlite::Transaction;

use super::{Result, Store, path_str};
use crate::scan::Baseline;

/// How big a complete scan was, in the units the walk counts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScanShape {
    pub entries: u64,
    pub wall: Duration,
    /// Entries per top-level folder (and per root, for files directly in it).
    pub children: Vec<(PathBuf, u64)>,
}

pub(super) fn insert(tx: &Transaction, scan_id: i64, shape: &ScanShape) -> Result<()> {
    tx.execute(
        "INSERT INTO scan_shape (scan_id, entries, wall_ms, complete) VALUES (?1, ?2, ?3, 1)",
        rusqlite::params![scan_id, shape.entries as i64, shape.wall.as_millis() as i64],
    )?;
    let mut child =
        tx.prepare("INSERT INTO scan_child (scan_id, child, entries) VALUES (?1, ?2, ?3)")?;
    for (path, entries) in &shape.children {
        child.execute(rusqlite::params![scan_id, path_str(path), *entries as i64])?;
    }
    Ok(())
}

/// How many recent scans of a root set are looked through for one with a shape.
const LOOK_BACK: usize = 20;

impl Store {
    /// What the newest complete scan of exactly these roots weighed, if any did.
    pub fn baseline_for(&self, roots: &[PathBuf]) -> Result<Option<Baseline>> {
        for (id, ..) in self.scans_of(roots, LOOK_BACK)? {
            let shape: Option<(i64, i64)> = self
                .conn
                .query_row(
                    "SELECT entries, wall_ms FROM scan_shape WHERE scan_id = ?1 AND complete = 1",
                    [id],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .ok();
            let Some((entries, wall_ms)) = shape else {
                continue;
            };
            let mut stmt = self
                .conn
                .prepare("SELECT child, entries FROM scan_child WHERE scan_id = ?1")?;
            let children = stmt
                .query_map([id], |r| {
                    Ok((
                        PathBuf::from(r.get::<_, String>(0)?),
                        r.get::<_, i64>(1)? as u64,
                    ))
                })?
                .collect::<rusqlite::Result<_>>()?;
            return Ok(Some(Baseline {
                entries: entries as u64,
                wall: Duration::from_millis(wall_ms as u64),
                children,
            }));
        }
        Ok(None)
    }
}

/// [`Store::baseline_for`] on the store at `db`, where a store that cannot be
/// opened, read or understood is no baseline rather than an error: the bar then
/// shows counts and no percentage, which is true, and the scan goes on.
pub fn read_baseline(db: &Path, roots: &[PathBuf]) -> Option<Baseline> {
    Store::open(db).ok()?.baseline_for(roots).ok().flatten()
}
