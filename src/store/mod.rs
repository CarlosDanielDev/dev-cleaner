//! Remembering what previous scans found.
//!
//! A one-shot cleaner makes the user rediscover the same directories every time
//! the disk fills. History is what turns the same walk into an answer about what
//! regrew, and it only works if the store outlives the thing it describes: the
//! database deliberately sits outside every scanned root and every registered
//! cache, so no purge this tool offers can erase its own history.

mod collect;
mod snapshot;
mod trend;

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use rusqlite::Connection;

pub use collect::snapshot;
pub use snapshot::{EntryRow, ProjectRow, Snapshot, StoredSafety};
pub use trend::{Change, TrendRow};

/// Anything that can stop the store from answering.
#[derive(Debug)]
pub enum StoreError {
    /// The database file or its directory could not be created.
    Io(std::io::Error),
    Db(rusqlite::Error),
}

impl std::fmt::Display for StoreError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            StoreError::Io(e) => write!(f, "{e}"),
            StoreError::Db(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for StoreError {}

impl From<std::io::Error> for StoreError {
    fn from(e: std::io::Error) -> Self {
        StoreError::Io(e)
    }
}

impl From<rusqlite::Error> for StoreError {
    fn from(e: rusqlite::Error) -> Self {
        StoreError::Db(e)
    }
}

pub type Result<T> = std::result::Result<T, StoreError>;

/// The scan history, on disk.
pub struct Store {
    conn: Connection,
}

impl Store {
    /// Schema steps, in order. The index of a statement is its version, so a
    /// migration is never renumbered and never re-run.
    ///
    /// Snapshots are per-scan rather than per-path: `project` carries a
    /// `scan_id` so a stored scan reproduces exactly the facts observed at the
    /// time. A project table keyed only by path would be overwritten by the next
    /// scan, and reading an old snapshot back would silently return today's
    /// answers for `dirty` and `last_commit_at`.
    pub const MIGRATIONS: &'static [&'static str] = &[r#"
        CREATE TABLE scan (
            id                   INTEGER PRIMARY KEY,
            started_at           INTEGER NOT NULL,
            root_set             TEXT    NOT NULL,
            total_bytes_apparent INTEGER NOT NULL,
            total_bytes_unique   INTEGER NOT NULL,
            total_inodes         INTEGER NOT NULL
        );

        CREATE TABLE project (
            id             INTEGER PRIMARY KEY,
            scan_id        INTEGER NOT NULL REFERENCES scan(id) ON DELETE CASCADE,
            path           TEXT    NOT NULL,
            kind           TEXT    NOT NULL,
            vcs_remote     TEXT,
            last_commit_at INTEGER,
            dirty          INTEGER
        );

        CREATE TABLE entry (
            scan_id        INTEGER NOT NULL REFERENCES scan(id) ON DELETE CASCADE,
            project_id     INTEGER REFERENCES project(id) ON DELETE CASCADE,
            path           TEXT    NOT NULL,
            kind           TEXT    NOT NULL,
            bytes_apparent INTEGER NOT NULL,
            bytes_unique   INTEGER NOT NULL,
            inodes         INTEGER NOT NULL,
            safety         TEXT    NOT NULL,
            safety_detail  TEXT    NOT NULL
        );

        CREATE INDEX project_by_scan ON project(scan_id);
        -- Unique as well as fast. Two rows for one path in one scan would
        -- double its bytes in every total and fan the trend join out, so the
        -- constraint is the guard and the index is the side effect.
        CREATE UNIQUE INDEX entry_by_scan ON entry(scan_id, path);
    "#];

    /// Open the store, creating and migrating it as needed.
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let conn = Connection::open(path)?;

        // WAL lets a reader work while a writer commits, and the busy timeout
        // makes two concurrent scans queue instead of one failing outright.
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.busy_timeout(std::time::Duration::from_secs(15))?;
        conn.pragma_update(None, "foreign_keys", true)?;

        let store = Self { conn };
        store.migrate()?;
        Ok(store)
    }

    /// Apply every migration this database has not seen yet.
    ///
    /// Idempotent by construction: `user_version` records how many statements
    /// have run, and only the ones past that point are executed.
    fn migrate(&self) -> Result<()> {
        let applied = self.schema_version()? as usize;
        for (i, sql) in Self::MIGRATIONS.iter().enumerate().skip(applied) {
            // One transaction per step, so a failure half way through leaves the
            // database on the last version that fully applied.
            self.conn.execute_batch(&format!(
                "BEGIN; {sql} PRAGMA user_version = {}; COMMIT;",
                i + 1
            ))?;
        }
        Ok(())
    }

    pub fn schema_version(&self) -> Result<i64> {
        Ok(self
            .conn
            .query_row("PRAGMA user_version", [], |r| r.get(0))?)
    }

    pub fn has_table(&self, name: &str) -> Result<bool> {
        Ok(self.conn.query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = ?1",
            [name],
            |r| r.get::<_, i64>(0),
        )? > 0)
    }
}

/// Where the history lives.
///
/// Under `~/.local/state`, beside the purge manifests and outside every scanned
/// root and every registered cache. A test pins that against both registries, so
/// a newly registered cache cannot start shadowing it.
pub fn db_path() -> PathBuf {
    PathBuf::from(std::env::var("HOME").unwrap_or_else(|_| "/".into()))
        .join(".local/state/dev-cleaner/history.sqlite3")
}

/// ponytail: nanoseconds since the epoch in a signed 64-bit column, which runs
/// out in 2262. Chosen over whole seconds so a timestamp survives the round trip
/// unchanged rather than silently losing its fractional part.
fn to_nanos(t: SystemTime) -> i64 {
    t.duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos() as i64)
        .unwrap_or(0)
}

fn from_nanos(n: i64) -> SystemTime {
    UNIX_EPOCH + Duration::from_nanos(n.max(0) as u64)
}

/// ponytail: newline separated. A configured root containing a newline would
/// round-trip as two roots. Roots are directories the user names in a config
/// file, and the field is descriptive rather than something a deletion decision
/// reads, so a side table to escape a character nobody types is not worth it.
fn join_paths(paths: &[PathBuf]) -> String {
    paths
        .iter()
        .map(|p| path_str(p))
        .collect::<Vec<_>>()
        .join("\n")
}

fn split_paths(joined: &str) -> Vec<PathBuf> {
    joined
        .split('\n')
        .filter(|s| !s.is_empty())
        .map(PathBuf::from)
        .collect()
}

/// A path as text. Non-UTF-8 paths are stored lossily rather than dropped: a
/// directory that cannot be named is still a directory taking up space, and
/// omitting it would understate the total.
fn path_str(p: &std::path::Path) -> String {
    p.to_string_lossy().into_owned()
}
