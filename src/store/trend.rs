use std::path::PathBuf;

use super::{Result, Store};

/// What happened to one path between two scans.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Change {
    /// Not present in the earlier scan.
    New,
    Grew {
        by: u64,
    },
    Shrank {
        by: u64,
    },
    Unchanged,
    /// Present earlier, absent now.
    ///
    /// Deliberately its own state rather than a shrink to zero. "0 B" reads as
    /// a directory that is still there and now empty, which is a different fact
    /// and would hide the only thing worth reporting.
    Removed,
}

impl Change {
    /// How this reads in a report.
    ///
    /// `Removed` deliberately has no number attached. Printing "-289 MB" beside
    /// a path that no longer exists reads as a directory that shrank, and the
    /// user would go looking for what is left of it.
    pub fn describe(&self) -> String {
        match self {
            Change::New => "new".to_string(),
            Change::Grew { by } => format!("+{}", crate::bytes::human(*by)),
            Change::Shrank { by } => format!("-{}", crate::bytes::human(*by)),
            Change::Unchanged => "unchanged".to_string(),
            Change::Removed => "removed".to_string(),
        }
    }
}

/// One line of the trend view.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrendRow {
    pub path: PathBuf,
    /// What the path holds in the later scan, or what it held in the earlier
    /// one if it is gone. Never a delta: the change carries that separately.
    pub bytes: u64,
    pub change: Change,
}

impl Store {
    /// Diff two scans by path.
    ///
    /// Sizes come from `bytes_unique`, the measure that survives hardlinks, so
    /// a trend never claims growth in space that deletion could not return.
    ///
    /// Ordered largest first, which is the order every caller wants and the
    /// order the report prints.
    pub fn trend(&self, before: i64, after: i64) -> Result<Vec<TrendRow>> {
        // Two halves rather than a full outer join. The first walks the later
        // scan and looks each path up in the earlier one; the second picks up
        // only what the first could not see, the paths that no longer exist.
        //
        // Every reference to `entry` names both a scan and a path, or a scan
        // alone, so the unique `entry_by_scan(scan_id, path)` index serves all
        // four: two driving searches and two exact-match lookups. The work is
        // proportional to the two scans involved, not to the whole history.
        let mut stmt = self.conn.prepare(
            "SELECT later.path, earlier.bytes_unique, later.bytes_unique \
               FROM entry later \
               LEFT JOIN entry earlier \
                 ON earlier.path = later.path AND earlier.scan_id = ?1 \
              WHERE later.scan_id = ?2 \
             UNION ALL \
             SELECT earlier.path, earlier.bytes_unique, NULL \
               FROM entry earlier \
               LEFT JOIN entry later \
                 ON later.path = earlier.path AND later.scan_id = ?2 \
              WHERE earlier.scan_id = ?1 AND later.path IS NULL",
        )?;

        let mut rows = stmt
            .query_map([before, after], |r| {
                Ok((
                    PathBuf::from(r.get::<_, String>(0)?),
                    r.get::<_, Option<i64>>(1)?.map(|b| b as u64),
                    r.get::<_, Option<i64>>(2)?.map(|b| b as u64),
                ))
            })?
            .map(|row| {
                row.map(|(path, was, now)| match (was, now) {
                    (Some(was), Some(now)) => TrendRow {
                        path,
                        bytes: now,
                        change: match now.cmp(&was) {
                            std::cmp::Ordering::Greater => Change::Grew { by: now - was },
                            std::cmp::Ordering::Less => Change::Shrank { by: was - now },
                            std::cmp::Ordering::Equal => Change::Unchanged,
                        },
                    },
                    (None, Some(now)) => TrendRow {
                        path,
                        bytes: now,
                        change: Change::New,
                    },
                    (Some(was), None) => TrendRow {
                        path,
                        bytes: was,
                        change: Change::Removed,
                    },
                    // The first half only emits rows from the later scan and the
                    // second only rows from the earlier one, so a row with
                    // neither size cannot be produced.
                    (None, None) => unreachable!("every row comes from one scan or the other"),
                })
            })
            .collect::<rusqlite::Result<Vec<_>>>()?;

        rows.sort_by(|a, b| b.bytes.cmp(&a.bytes).then_with(|| a.path.cmp(&b.path)));
        Ok(rows)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The diff must be served by the indexes, not by reading the table.
    ///
    /// A wall-clock assertion alone would pass on a small fixture and rot as
    /// the history grows on a real machine. This asks the planner directly.
    #[test]
    fn the_diff_is_served_by_the_indexes() {
        let dir = tempfile::TempDir::new().expect("tempdir");
        let store = Store::open(&dir.path().join("history.sqlite3")).expect("open");

        let plan: Vec<String> = {
            let mut stmt = store
                .conn
                .prepare(
                    "EXPLAIN QUERY PLAN \
                     SELECT later.path, earlier.bytes_unique, later.bytes_unique \
                       FROM entry later \
                       LEFT JOIN entry earlier \
                         ON earlier.path = later.path AND earlier.scan_id = ?1 \
                      WHERE later.scan_id = ?2 \
                     UNION ALL \
                     SELECT earlier.path, earlier.bytes_unique, NULL \
                       FROM entry earlier \
                       LEFT JOIN entry later \
                         ON later.path = earlier.path AND later.scan_id = ?2 \
                      WHERE earlier.scan_id = ?1 AND later.path IS NULL",
                )
                .expect("prepare");
            stmt.query_map([1i64, 2i64], |r| r.get::<_, String>(3))
                .expect("explain")
                .collect::<rusqlite::Result<Vec<_>>>()
                .expect("rows")
        };

        let steps = plan.join("\n");
        assert!(
            !steps.contains("SCAN "),
            "the diff reads a whole table instead of searching it:\n{steps}"
        );
        assert_eq!(
            steps.matches("entry_by_scan (scan_id=?)").count(),
            2,
            "each half must be driven by the one scan it diffs:\n{steps}"
        );
        // Both columns in the lookup, not just the scan. An index on `scan_id`
        // alone would still show up here, then filter every row of that scan
        // for each path it is asked about.
        assert_eq!(
            steps
                .matches("entry_by_scan (scan_id=? AND path=?)")
                .count(),
            2,
            "each lookup must match on scan and path together:\n{steps}"
        );
    }
}
