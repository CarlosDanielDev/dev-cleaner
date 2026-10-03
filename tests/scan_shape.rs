//! What a finished scan leaves behind for the next one's progress bar.

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use dev_cleaner::store::{ScanShape, Snapshot, Store, read_baseline};
use tempfile::TempDir;

fn scratch() -> (TempDir, PathBuf) {
    let dir = TempDir::new().expect("tempdir");
    let path = dir.path().join("state/db.sqlite3");
    (dir, path)
}

fn snap(roots: &[&str]) -> Snapshot {
    Snapshot {
        started_at: SystemTime::UNIX_EPOCH + Duration::from_secs(1),
        roots: roots.iter().map(PathBuf::from).collect(),
        total_bytes_apparent: 3,
        total_bytes_unique: 2,
        total_inodes: 1,
        reclaimable_unique: Some(0),
        projects: Vec::new(),
        entries: Vec::new(),
    }
}

fn shape(entries: u64, children: &[(&str, u64)]) -> ScanShape {
    ScanShape {
        entries,
        wall: Duration::from_millis(1_500),
        children: children
            .iter()
            .map(|(p, n)| (PathBuf::from(p), *n))
            .collect(),
    }
}

#[test]
fn a_complete_scan_leaves_its_size_its_time_and_its_folders() {
    let (_dir, path) = scratch();
    let mut store = Store::open(&path).expect("open");
    store
        .write_snapshot_shaped(&snap(&["/r"]), &shape(10, &[("/r/a", 6), ("/r/b", 4)]))
        .expect("write");

    let base = read_baseline(&path, &[PathBuf::from("/r")]).expect("a baseline");

    assert_eq!(base.entries, 10);
    assert_eq!(base.wall, Duration::from_millis(1_500));
    assert_eq!(base.children.get(Path::new("/r/a")), Some(&6));
    assert_eq!(base.children.get(Path::new("/r/b")), Some(&4));
}

#[test]
fn the_newest_complete_scan_of_these_roots_is_the_baseline() {
    let (_dir, path) = scratch();
    let mut store = Store::open(&path).expect("open");
    store
        .write_snapshot_shaped(&snap(&["/r"]), &shape(10, &[("/r/a", 10)]))
        .expect("write");
    store
        .write_snapshot_shaped(&snap(&["/r"]), &shape(20, &[("/r/a", 20)]))
        .expect("write");
    // Another set of roots, newer than both: not this one's baseline.
    store
        .write_snapshot_shaped(&snap(&["/elsewhere"]), &shape(99, &[("/elsewhere/x", 99)]))
        .expect("write");

    let base = read_baseline(&path, &[PathBuf::from("/r")]).expect("a baseline");
    assert_eq!(base.entries, 20);
    assert!(read_baseline(&path, &[PathBuf::from("/nowhere")]).is_none());
}

#[test]
fn a_scan_written_without_a_shape_is_not_a_baseline() {
    let (_dir, path) = scratch();
    let mut store = Store::open(&path).expect("open");
    store
        .write_snapshot_shaped(&snap(&["/r"]), &shape(10, &[("/r/a", 10)]))
        .expect("write");
    // `scan` on the command line records the way it always did, newer.
    store.write_snapshot(&snap(&["/r"])).expect("write");

    let base = read_baseline(&path, &[PathBuf::from("/r")]).expect("falls back to the shaped one");
    assert_eq!(
        base.entries, 10,
        "the unshaped scan says nothing about size"
    );
}

#[test]
fn a_store_from_before_the_migration_is_carried_forward_and_is_no_baseline() {
    let (_dir, path) = scratch();
    std::fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
    {
        let conn = rusqlite::Connection::open(&path).expect("raw");
        // Exactly the three migrations the release before this one had.
        for (i, sql) in Store::MIGRATIONS[..3].iter().enumerate() {
            conn.execute_batch(&format!(
                "BEGIN; {sql} PRAGMA user_version = {}; COMMIT;",
                i + 1
            ))
            .expect("old schema");
        }
        conn.execute(
            "INSERT INTO scan (started_at, root_set, total_bytes_apparent, \
             total_bytes_unique, total_inodes) VALUES (1, '/r', 3, 2, 1)",
            [],
        )
        .expect("an old scan");
    }

    assert!(
        read_baseline(&path, &[PathBuf::from("/r")]).is_none(),
        "an old row has no size to measure against"
    );
    let store = Store::open(&path).expect("migrated in place");
    assert_eq!(
        store.schema_version().expect("v"),
        Store::MIGRATIONS.len() as i64
    );
    assert_eq!(
        store.scan_ids().expect("ids").len(),
        1,
        "the old scan is kept"
    );
    assert!(store.has_table("scan_shape").expect("q"));
}

#[test]
fn a_store_that_is_not_a_database_is_no_baseline_and_no_crash() {
    let (_dir, path) = scratch();
    std::fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
    std::fs::write(
        &path,
        b"this is not sqlite, it is a text file of some length",
    )
    .expect("junk");

    assert!(read_baseline(&path, &[PathBuf::from("/r")]).is_none());
}

#[test]
fn a_shape_and_its_scan_are_one_transaction() {
    let (_dir, path) = scratch();
    let mut store = Store::open(&path).expect("open");
    // Two children of one scan with the same name break the primary key
    // after the scan row is already inserted.
    let clash = shape(2, &[("/r/a", 1), ("/r/a", 1)]);
    assert!(store.write_snapshot_shaped(&snap(&["/r"]), &clash).is_err());

    assert!(
        store.scan_ids().expect("ids").is_empty(),
        "a scan whose shape could not be written must not be left half-recorded"
    );
}
