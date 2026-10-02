//! Persistence: schema, migrations, snapshot round-trip and trends.

use std::path::Path;

use dev_cleaner::store::{Store, db_path};
use tempfile::TempDir;

fn scratch() -> (TempDir, std::path::PathBuf) {
    let dir = TempDir::new().expect("tempdir");
    let path = dir.path().join("state/db.sqlite3");
    (dir, path)
}

/// A store exactly as the release before this one left it: the schema alone,
/// at version 1, holding one scan recorded before the reclaimable total was
/// kept. Built with a raw connection so nothing in `Store` can quietly bring
/// it up to date first.
fn store_at_version_one(path: &Path) {
    std::fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
    let conn = rusqlite::Connection::open(path).expect("open raw");
    conn.execute_batch(&format!(
        "BEGIN; {} PRAGMA user_version = 1; COMMIT;",
        Store::MIGRATIONS[0]
    ))
    .expect("apply the first migration");
    conn.execute(
        "INSERT INTO scan (started_at, root_set, total_bytes_apparent, \
         total_bytes_unique, total_inodes) VALUES (1, '/r', 3, 2, 1)",
        [],
    )
    .expect("record a scan the old way");
}

mod schema {
    use super::*;

    #[test]
    fn opening_creates_the_schema() {
        let (_dir, path) = scratch();
        let store = Store::open(&path).expect("open");

        for table in ["scan", "project", "entry"] {
            assert!(
                store.has_table(table).expect("query"),
                "expected a `{table}` table after opening"
            );
        }
    }

    #[test]
    fn opening_creates_the_parent_directory() {
        let (_dir, path) = scratch();
        assert!(!path.parent().expect("parent").exists());

        Store::open(&path).expect("open");

        assert!(path.exists(), "the database file was not created");
    }

    #[test]
    fn migrations_are_idempotent() {
        let (_dir, path) = scratch();

        let first = Store::open(&path).expect("first open");
        let version = first.schema_version().expect("version");
        drop(first);

        // Re-opening must not re-run a migration that already applied. If it
        // did, the second `CREATE TABLE` would error rather than reaching here.
        let second = Store::open(&path).expect("second open");
        assert_eq!(
            second.schema_version().expect("version"),
            version,
            "re-opening changed the schema version"
        );
    }

    #[test]
    fn the_schema_version_matches_the_migrations_that_exist() {
        let (_dir, path) = scratch();
        let store = Store::open(&path).expect("open");

        assert_eq!(
            store.schema_version().expect("version"),
            Store::MIGRATIONS.len() as i64,
            "user_version must count the migrations that ran"
        );
    }

    /// A database left by the previous release is carried forward, not started
    /// over: the scan it holds is the baseline the next trend is measured
    /// against, and it must land on the same version a fresh store does.
    #[test]
    fn a_store_at_version_one_migrates_to_the_latest_in_place() {
        let (_dir, path) = scratch();
        store_at_version_one(&path);

        let migrated = Store::open(&path).expect("open and migrate");
        let (_fresh_dir, fresh_path) = scratch();
        let fresh = Store::open(&fresh_path).expect("open fresh");

        assert_eq!(
            migrated.schema_version().expect("version"),
            Store::MIGRATIONS.len() as i64,
            "an old store must land where a fresh one does"
        );
        assert_eq!(
            fresh.schema_version().expect("version"),
            migrated.schema_version().expect("version"),
            "a migrated store and a fresh one must agree on their version"
        );
        assert_eq!(
            migrated.scan_ids().expect("ids").len(),
            1,
            "the migration lost the scan it was supposed to carry forward"
        );
    }

    /// The store must not sit anywhere the tool would later offer to delete.
    /// A history that a purge can erase is not a history.
    #[test]
    fn the_database_is_outside_everything_the_tool_scans() {
        let path = db_path();
        let home = std::path::PathBuf::from(std::env::var("HOME").unwrap_or_else(|_| "/".into()));

        for cache in dev_cleaner::classify::cache_kinds() {
            assert!(
                !path.starts_with(home.join(cache.rel_path)),
                "the database would sit inside the {} cache",
                cache.name
            );
        }

        for kind in dev_cleaner::classify::artifact_kinds() {
            assert!(
                !path.components().any(|c| c.as_os_str() == kind.dir_name),
                "the database path contains the artifact directory {}",
                kind.dir_name
            );
        }

        for root in dev_cleaner::config::Config::default().roots {
            assert!(
                !path.starts_with(&root),
                "the database would sit inside the scanned root {}",
                root.display()
            );
        }
    }
}

mod round_trip {
    use super::*;

    use std::path::PathBuf;
    use std::time::{Duration, SystemTime, UNIX_EPOCH};

    use dev_cleaner::safety::{BlockReason, RegenCommand, Safety};
    use dev_cleaner::store::{EntryRow, ProjectRow, Snapshot, StoredSafety};

    fn at(secs: u64, nanos: u32) -> SystemTime {
        UNIX_EPOCH + Duration::new(secs, nanos)
    }

    /// One snapshot exercising every column, every optional field in both
    /// states, and every safety tier.
    fn full_snapshot() -> Snapshot {
        Snapshot {
            started_at: at(1_756_000_000, 123_456_789),
            roots: vec![
                PathBuf::from("/Users/t/projects"),
                PathBuf::from("/opt/src"),
            ],
            total_bytes_apparent: 60_000_000_000,
            total_bytes_unique: 34_000_000_000,
            total_inodes: 1_048_576,
            reclaimable_unique: Some(2_111_000_000),
            projects: vec![
                ProjectRow {
                    path: PathBuf::from("/Users/t/projects/web"),
                    kind: "node,rust".into(),
                    vcs_remote: Some("git@example.com:me/web.git".into()),
                    last_commit_at: Some(at(1_750_000_000, 987_654_321)),
                    dirty: Some(true),
                },
                ProjectRow {
                    path: PathBuf::from("/Users/t/projects/lone"),
                    kind: "python".into(),
                    vcs_remote: None,
                    last_commit_at: None,
                    dirty: None,
                },
            ],
            entries: vec![
                EntryRow {
                    project: Some(PathBuf::from("/Users/t/projects/web")),
                    path: PathBuf::from("/Users/t/projects/web/node_modules"),
                    kind: "node_modules".into(),
                    bytes_apparent: 950_000_000,
                    bytes_unique: 911_000_000,
                    inodes: 84_211,
                    safety: StoredSafety::Regenerable {
                        regen: "npm install".into(),
                    },
                },
                EntryRow {
                    project: None,
                    path: PathBuf::from("/Users/t/.npm/_cacache"),
                    kind: "npm".into(),
                    bytes_apparent: 1_200_000_000,
                    bytes_unique: 1_200_000_000,
                    inodes: 51_004,
                    safety: StoredSafety::Cache {
                        refills_on: "re-downloaded on next install".into(),
                    },
                },
                EntryRow {
                    project: Some(PathBuf::from("/Users/t/projects/lone")),
                    path: PathBuf::from("/Users/t/projects/lone/mystery"),
                    kind: "mystery".into(),
                    bytes_apparent: 5,
                    bytes_unique: 4_096,
                    inodes: 1,
                    safety: StoredSafety::Unproven {
                        reason: "nothing in the registry claims this".into(),
                    },
                },
                EntryRow {
                    project: Some(PathBuf::from("/Users/t/projects/web")),
                    path: PathBuf::from("/Users/t/projects/web/.venv"),
                    kind: ".venv".into(),
                    bytes_apparent: 425_000_000,
                    bytes_unique: 425_000_000,
                    inodes: 22_811,
                    safety: StoredSafety::Protected {
                        reason: BlockReason::StashEntries,
                    },
                },
            ],
        }
    }

    #[test]
    fn a_snapshot_reads_back_exactly_as_it_was_written() {
        let (_dir, path) = scratch();
        let mut store = Store::open(&path).expect("open");
        let written = full_snapshot();

        let id = store.write_snapshot(&written).expect("write");
        let read = store.read_snapshot(id).expect("read");

        assert_eq!(read, written, "the snapshot did not survive the round trip");
    }

    /// Apparent and unique bytes measure different things: `st_size` versus
    /// allocated blocks. Collapsing them would be the tool promising space that
    /// deletion cannot return, so both must survive independently.
    #[test]
    fn both_byte_measures_survive_independently() {
        let (_dir, path) = scratch();
        let mut store = Store::open(&path).expect("open");
        let written = full_snapshot();

        let id = store.write_snapshot(&written).expect("write");
        let read = store.read_snapshot(id).expect("read");

        assert_ne!(
            written.total_bytes_apparent, written.total_bytes_unique,
            "the fixture must distinguish the two measures for this to prove anything"
        );
        assert_eq!(read.total_bytes_apparent, 60_000_000_000);
        assert_eq!(read.total_bytes_unique, 34_000_000_000);

        let sparse = read
            .entries
            .iter()
            .find(|e| e.kind == "mystery")
            .expect("entry");
        assert_eq!(sparse.bytes_apparent, 5, "apparent size was overwritten");
        assert_eq!(sparse.bytes_unique, 4_096, "unique size was overwritten");
    }

    /// A scan recorded before the total was kept has no value, and it must
    /// come back as none. A `0` would read as a clean disk on the day the
    /// tool was first installed, which is the opposite of what was true.
    #[test]
    fn a_scan_recorded_before_the_total_was_kept_reads_back_none() {
        let (_dir, path) = scratch();
        store_at_version_one(&path);
        let mut store = Store::open(&path).expect("open");

        let old = store.latest_scan().expect("query").expect("the old scan");
        assert_eq!(
            store.read_snapshot(old).expect("read").reclaimable_unique,
            None,
            "a scan that never measured the total must not claim one"
        );

        let new = store.write_snapshot(&full_snapshot()).expect("write");
        assert_eq!(
            store.read_snapshot(new).expect("read").reclaimable_unique,
            Some(2_111_000_000),
            "a scan written with the total must keep it"
        );
    }

    /// A candidate whose owning project is not recorded is a candidate the
    /// trend view cannot attribute. The link has to survive, and the absence of
    /// one has to stay absent rather than becoming an arbitrary project.
    #[test]
    fn entries_keep_the_project_they_belong_to() {
        let (_dir, path) = scratch();
        let mut store = Store::open(&path).expect("open");

        let id = store.write_snapshot(&full_snapshot()).expect("write");
        let read = store.read_snapshot(id).expect("read");

        let owned = &read.entries[0];
        assert_eq!(
            owned.project.as_deref(),
            Some(std::path::Path::new("/Users/t/projects/web"))
        );
        let global = &read.entries[1];
        assert_eq!(global.project, None, "a global cache gained an owner");
    }

    #[test]
    fn every_safety_tier_round_trips() {
        let (_dir, path) = scratch();
        let mut store = Store::open(&path).expect("open");

        let id = store.write_snapshot(&full_snapshot()).expect("write");
        let read = store.read_snapshot(id).expect("read");

        let tiers: Vec<_> = read.entries.iter().map(|e| &e.safety).collect();
        assert!(matches!(tiers[0], StoredSafety::Regenerable { regen } if regen == "npm install"));
        assert!(matches!(tiers[1], StoredSafety::Cache { .. }));
        assert!(matches!(tiers[2], StoredSafety::Unproven { .. }));
        assert!(matches!(
            tiers[3],
            StoredSafety::Protected {
                reason: BlockReason::StashEntries
            }
        ));
    }

    /// Every block reason must survive as itself. A reason that decoded to the
    /// wrong variant would show the user the wrong explanation for why
    /// something is unavailable.
    #[test]
    fn every_block_reason_round_trips() {
        let (_dir, path) = scratch();
        let mut store = Store::open(&path).expect("open");

        let mut snap = full_snapshot();
        snap.entries = BlockReason::all()
            .into_iter()
            .enumerate()
            .map(|(i, reason)| EntryRow {
                project: None,
                path: PathBuf::from(format!("/blocked/{i}")),
                kind: "target".into(),
                bytes_apparent: 1,
                bytes_unique: 1,
                inodes: 1,
                safety: StoredSafety::Protected { reason },
            })
            .collect();

        let id = store.write_snapshot(&snap).expect("write");
        let read = store.read_snapshot(id).expect("read");

        assert_eq!(read.entries, snap.entries);
    }

    /// The safety tiers carry what the user is shown. Converting a live
    /// `Safety` for storage must not drop the command or the reason behind it.
    #[test]
    fn converting_a_live_safety_keeps_its_payload() {
        assert_eq!(
            StoredSafety::from(&Safety::Regenerable {
                regen: RegenCommand::new("cargo build").expect("valid"),
            }),
            StoredSafety::Regenerable {
                regen: "cargo build".into()
            }
        );
        assert_eq!(
            StoredSafety::from(&Safety::Cache {
                refills_on: "next build"
            }),
            StoredSafety::Cache {
                refills_on: "next build".into()
            }
        );
        assert_eq!(
            StoredSafety::from(&Safety::for_unknown("no registry entry")),
            StoredSafety::Unproven {
                reason: "no registry entry".into()
            }
        );
        assert_eq!(
            StoredSafety::from(&Safety::Protected {
                reason: BlockReason::DockerVolume
            }),
            StoredSafety::Protected {
                reason: BlockReason::DockerVolume
            }
        );
    }

    #[test]
    fn the_latest_scan_is_the_most_recently_written() {
        let (_dir, path) = scratch();
        let mut store = Store::open(&path).expect("open");
        assert_eq!(store.latest_scan().expect("query"), None);

        let first = store.write_snapshot(&full_snapshot()).expect("write");
        let second = store.write_snapshot(&full_snapshot()).expect("write");

        assert_ne!(first, second, "each write is its own scan");
        assert_eq!(store.latest_scan().expect("query"), Some(second));
    }

    /// A trend is only meaningful between scans that looked at the same
    /// territory. Diffing a scan of one root against a scan of two reports
    /// every path in the second root as removed, when it is untouched on disk —
    /// the tool claiming space came back that never went anywhere.
    #[test]
    fn the_baseline_is_the_last_scan_of_the_same_roots() {
        let (_dir, path) = scratch();
        let mut store = Store::open(&path).expect("open");

        let both = |roots: Vec<&str>| {
            let mut snap = full_snapshot();
            snap.roots = roots.into_iter().map(PathBuf::from).collect();
            snap
        };

        let wide = store
            .write_snapshot(&both(vec!["/a", "/b"]))
            .expect("write");
        let narrow = store.write_snapshot(&both(vec!["/a"])).expect("write");

        assert_eq!(
            store
                .latest_scan_for(&[PathBuf::from("/a")])
                .expect("query"),
            Some(narrow)
        );
        assert_eq!(
            store
                .latest_scan_for(&[PathBuf::from("/a"), PathBuf::from("/b")])
                .expect("query"),
            Some(wide),
            "the wider scan was skipped over for a narrower one"
        );
        assert_eq!(
            store
                .latest_scan_for(&[PathBuf::from("/never-scanned")])
                .expect("query"),
            None,
            "an unseen root set must have no baseline at all"
        );
    }

    /// The same roots named in a different order, or twice, are the same
    /// territory. Treating them as different would silently start the history
    /// over every time the user retyped the arguments.
    #[test]
    fn a_root_set_is_matched_as_a_set_not_as_a_list() {
        let (_dir, path) = scratch();
        let mut store = Store::open(&path).expect("open");

        let mut snap = full_snapshot();
        snap.roots = vec![PathBuf::from("/a"), PathBuf::from("/b")];
        let id = store.write_snapshot(&snap).expect("write");

        for asked in [
            vec![PathBuf::from("/b"), PathBuf::from("/a")],
            vec![
                PathBuf::from("/a"),
                PathBuf::from("/b"),
                PathBuf::from("/a"),
            ],
        ] {
            assert_eq!(
                store.latest_scan_for(&asked).expect("query"),
                Some(id),
                "{asked:?} did not match the scan of the same roots"
            );
        }
    }

    /// The same path twice in one scan would double its bytes in every total
    /// and fan the trend join out into a cross product. It is refused outright
    /// rather than merged, because merging would hide the bug that produced it.
    #[test]
    fn one_path_cannot_appear_twice_in_a_single_scan() {
        let (_dir, path) = scratch();
        let mut store = Store::open(&path).expect("open");

        let mut snap = full_snapshot();
        let duplicate = snap.entries[0].clone();
        snap.entries.push(duplicate);

        store
            .write_snapshot(&snap)
            .expect_err("a repeated path must be refused");
        assert_eq!(
            store.scan_ids().expect("ids"),
            Vec::<i64>::new(),
            "the refused scan was left behind"
        );
    }

    /// The reference corpus, at its measured shape.
    fn corpus() -> Snapshot {
        let mut snap = full_snapshot();
        snap.projects = (0..103)
            .map(|i| ProjectRow {
                path: PathBuf::from(format!("/Users/t/projects/p{i}")),
                kind: "node".into(),
                vcs_remote: Some(format!("git@example.com:me/p{i}.git")),
                last_commit_at: Some(at(1_750_000_000 + i, 0)),
                dirty: Some(i % 3 == 0),
            })
            .collect();
        snap.entries = (0..972)
            .map(|i| EntryRow {
                project: Some(PathBuf::from(format!("/Users/t/projects/p{}", i % 103))),
                path: PathBuf::from(format!("/Users/t/projects/p{}/a{i}/node_modules", i % 103)),
                kind: "node_modules".into(),
                bytes_apparent: 5_000_000 + i,
                bytes_unique: 4_000_000 + i,
                inodes: 900 + i,
                safety: StoredSafety::Regenerable {
                    regen: "npm install".into(),
                },
            })
            .collect();
        snap
    }

    #[test]
    fn the_reference_corpus_persists_in_under_a_second() {
        let (_dir, path) = scratch();
        let mut store = Store::open(&path).expect("open");
        let snap = corpus();

        let started = std::time::Instant::now();
        let id = store.write_snapshot(&snap).expect("write");
        let elapsed = started.elapsed();

        assert!(
            elapsed < Duration::from_secs(1),
            "persisting 103 projects and 972 entries took {elapsed:?}"
        );
        assert_eq!(store.read_snapshot(id).expect("read"), snap);
    }

    /// Scans running at once must not bleed into each other.
    ///
    /// Every writer starts on the same barrier and writes enough rows to still
    /// be writing when the others begin, so the threads genuinely contend for
    /// the database rather than finishing one after another. Each scan is
    /// stamped, and every row it owns carries the same stamp, so an entry
    /// landing on the wrong scan is visible rather than merely improbable.
    ///
    /// What this proves is that contention is handled: dropping the busy
    /// timeout makes it fail every time. It does not prove scan-id attribution,
    /// which no reachable interleaving exercises — that rests on
    /// `last_insert_rowid` being per-connection, not on this test.
    #[test]
    fn concurrent_scans_cannot_corrupt_the_store() {
        const WORKERS: u64 = 4;
        const ROUNDS: u64 = 3;
        const ENTRIES: u64 = 500;

        let (_dir, path) = scratch();
        Store::open(&path).expect("create");
        let gate = std::sync::Barrier::new(WORKERS as usize);

        std::thread::scope(|s| {
            for worker in 0..WORKERS {
                let path = path.clone();
                let gate = &gate;
                s.spawn(move || {
                    let mut store = Store::open(&path).expect("open");
                    gate.wait();
                    for round in 0..ROUNDS {
                        let stamp = worker * 1_000 + round;
                        let mut snap = full_snapshot();
                        snap.total_inodes = stamp;
                        snap.entries = (0..ENTRIES)
                            .map(|i| EntryRow {
                                project: None,
                                path: PathBuf::from(format!("/w{worker}/r{round}/{i}")),
                                kind: format!("stamp-{stamp}"),
                                bytes_apparent: i,
                                bytes_unique: i,
                                inodes: i,
                                safety: StoredSafety::Regenerable {
                                    regen: "npm install".into(),
                                },
                            })
                            .collect();
                        store.write_snapshot(&snap).expect("write");
                    }
                });
            }
        });

        let store = Store::open(&path).expect("reopen");
        let ids = store.scan_ids().expect("ids");
        assert_eq!(ids.len(), (WORKERS * ROUNDS) as usize, "a scan was lost");

        let mut stamps = Vec::new();
        for id in ids {
            let read = store.read_snapshot(id).expect("read");
            assert_eq!(
                read.entries.len(),
                ENTRIES as usize,
                "scan {id} holds another scan's entries, or lost its own"
            );
            let stamp = read.total_inodes;
            assert!(
                read.entries
                    .iter()
                    .all(|e| e.kind == format!("stamp-{stamp}")),
                "scan {id} was stamped {stamp} but holds rows written by another scan"
            );
            stamps.push(stamp);
        }

        stamps.sort_unstable();
        let mut wanted: Vec<u64> = (0..WORKERS)
            .flat_map(|w| (0..ROUNDS).map(move |r| w * 1_000 + r))
            .collect();
        wanted.sort_unstable();
        assert_eq!(stamps, wanted, "a scan was overwritten by another");
    }
}

mod trends {
    use super::*;

    use std::path::PathBuf;
    use std::time::{Duration, UNIX_EPOCH};

    use dev_cleaner::store::{Change, EntryRow, Snapshot, StoredSafety, TrendRow};

    fn entry(path: &str, bytes: u64) -> EntryRow {
        EntryRow {
            project: None,
            path: PathBuf::from(path),
            kind: "node_modules".into(),
            bytes_apparent: bytes,
            bytes_unique: bytes,
            inodes: 1,
            safety: StoredSafety::Regenerable {
                regen: "npm install".into(),
            },
        }
    }

    fn snapshot(entries: Vec<EntryRow>) -> Snapshot {
        Snapshot {
            started_at: UNIX_EPOCH + Duration::from_secs(1_756_000_000),
            roots: vec![PathBuf::from("/Users/t/projects")],
            total_bytes_apparent: entries.iter().map(|e| e.bytes_apparent).sum(),
            total_bytes_unique: entries.iter().map(|e| e.bytes_unique).sum(),
            total_inodes: entries.len() as u64,
            reclaimable_unique: Some(entries.iter().map(|e| e.bytes_unique).sum()),
            projects: Vec::new(),
            entries,
        }
    }

    fn find<'a>(rows: &'a [TrendRow], path: &str) -> &'a TrendRow {
        rows.iter()
            .find(|r| r.path == std::path::Path::new(path))
            .unwrap_or_else(|| panic!("{path} is missing from the trend"))
    }

    /// The four states a path can be in between two scans, in one diff.
    fn two_scans(store: &mut Store) -> (i64, i64) {
        let before = store
            .write_snapshot(&snapshot(vec![
                entry("/p/grows/node_modules", 571_000_000),
                entry("/p/shrinks/node_modules", 800_000_000),
                entry("/p/steady/.venv", 425_000_000),
                entry("/p/gone/target", 289_000_000),
            ]))
            .expect("write");
        let after = store
            .write_snapshot(&snapshot(vec![
                entry("/p/grows/node_modules", 911_000_000),
                entry("/p/shrinks/node_modules", 300_000_000),
                entry("/p/steady/.venv", 425_000_000),
                entry("/p/fresh/target", 289_000_000),
            ]))
            .expect("write");
        (before, after)
    }

    #[test]
    fn growth_shrinkage_and_new_entries_are_distinguished() {
        let (_dir, path) = scratch();
        let mut store = Store::open(&path).expect("open");
        let (before, after) = two_scans(&mut store);

        let rows = store.trend(before, after).expect("trend");

        assert_eq!(
            find(&rows, "/p/grows/node_modules").change,
            Change::Grew { by: 340_000_000 }
        );
        assert_eq!(
            find(&rows, "/p/shrinks/node_modules").change,
            Change::Shrank { by: 500_000_000 }
        );
        assert_eq!(find(&rows, "/p/steady/.venv").change, Change::Unchanged);
        assert_eq!(find(&rows, "/p/fresh/target").change, Change::New);
    }

    /// The reported size is what the path holds now, so the caller never has to
    /// reconstruct it from the delta.
    #[test]
    fn each_row_carries_the_size_it_holds_in_the_later_scan() {
        let (_dir, path) = scratch();
        let mut store = Store::open(&path).expect("open");
        let (before, after) = two_scans(&mut store);

        let rows = store.trend(before, after).expect("trend");

        assert_eq!(find(&rows, "/p/grows/node_modules").bytes, 911_000_000);
        assert_eq!(find(&rows, "/p/shrinks/node_modules").bytes, 300_000_000);
        assert_eq!(find(&rows, "/p/fresh/target").bytes, 289_000_000);
    }

    /// A path that disappeared is not a path that shrank to nothing. Reporting
    /// it as zero bytes would read as "still there, now empty" and hide the one
    /// thing the user would want to know: it is gone.
    #[test]
    fn a_path_absent_from_the_later_scan_is_removed_not_zero() {
        let (_dir, path) = scratch();
        let mut store = Store::open(&path).expect("open");
        let (before, after) = two_scans(&mut store);

        let rows = store.trend(before, after).expect("trend");
        let gone = find(&rows, "/p/gone/target");

        assert_eq!(gone.change, Change::Removed);
        assert_ne!(
            gone.change,
            Change::Shrank { by: 289_000_000 },
            "a removed path was reported as having shrunk away"
        );
        assert_eq!(
            gone.bytes, 289_000_000,
            "a removed path must still report what it held, not zero"
        );
    }

    #[test]
    fn every_path_from_either_scan_appears_exactly_once() {
        let (_dir, path) = scratch();
        let mut store = Store::open(&path).expect("open");
        let (before, after) = two_scans(&mut store);

        let rows = store.trend(before, after).expect("trend");

        let mut paths: Vec<_> = rows.iter().map(|r| r.path.clone()).collect();
        paths.sort();
        let mut unique = paths.clone();
        unique.dedup();
        assert_eq!(paths, unique, "a path appeared more than once");
        assert_eq!(rows.len(), 5, "expected the union of both scans");
    }

    #[test]
    fn rows_come_back_largest_first() {
        let (_dir, path) = scratch();
        let mut store = Store::open(&path).expect("open");
        let (before, after) = two_scans(&mut store);

        let rows = store.trend(before, after).expect("trend");
        let sizes: Vec<u64> = rows.iter().map(|r| r.bytes).collect();

        let mut sorted = sizes.clone();
        sorted.sort_unstable_by(|a, b| b.cmp(a));
        assert_eq!(sizes, sorted, "the trend was not ordered by size");
    }

    /// Two scans with nothing in common still diff correctly: everything in the
    /// earlier one is removed, everything in the later one is new.
    #[test]
    fn disjoint_scans_produce_only_new_and_removed() {
        let (_dir, path) = scratch();
        let mut store = Store::open(&path).expect("open");

        let before = store
            .write_snapshot(&snapshot(vec![entry("/old/target", 10)]))
            .expect("write");
        let after = store
            .write_snapshot(&snapshot(vec![entry("/new/target", 20)]))
            .expect("write");

        let rows = store.trend(before, after).expect("trend");

        assert_eq!(find(&rows, "/old/target").change, Change::Removed);
        assert_eq!(find(&rows, "/new/target").change, Change::New);
    }

    /// A diff must read the two scans it was asked for, not whatever else is in
    /// the history. Scans written between them must not leak into the answer.
    #[test]
    fn the_diff_reads_only_the_two_scans_it_was_given() {
        let (_dir, path) = scratch();
        let mut store = Store::open(&path).expect("open");

        // Noise on both sides: one scan older than the pair, one between them.
        // A diff that reached for "every scan up to this one" would pick up the
        // older sizes and report the wrong delta.
        store
            .write_snapshot(&snapshot(vec![
                entry("/p/target", 5),
                entry("/ancient/target", 4_000),
            ]))
            .expect("write");
        let before = store
            .write_snapshot(&snapshot(vec![entry("/p/target", 100)]))
            .expect("write");
        store
            .write_snapshot(&snapshot(vec![
                entry("/p/target", 999),
                entry("/noise/target", 777),
            ]))
            .expect("write");
        let after = store
            .write_snapshot(&snapshot(vec![entry("/p/target", 150)]))
            .expect("write");

        let rows = store.trend(before, after).expect("trend");

        assert_eq!(rows.len(), 1, "another scan leaked into the diff");
        assert_eq!(find(&rows, "/p/target").change, Change::Grew { by: 50 });
    }

    /// History accumulates forever. If the diff degraded into a scan of every
    /// row ever written, the dashboard would get slower every time it is used.
    #[test]
    fn the_diff_stays_fast_as_history_accumulates() {
        let (_dir, path) = scratch();
        let mut store = Store::open(&path).expect("open");

        let rows_per_scan = 400;
        let make = |offset: u64| {
            snapshot(
                (0..rows_per_scan)
                    .map(|i| entry(&format!("/p/{i}/node_modules"), 1_000 + offset))
                    .collect(),
            )
        };

        let first = store.write_snapshot(&make(0)).expect("write");
        let second = store.write_snapshot(&make(1)).expect("write");

        let started = std::time::Instant::now();
        let baseline = store.trend(first, second).expect("trend");
        let cold = started.elapsed();
        assert_eq!(baseline.len(), rows_per_scan as usize);

        for round in 2..120 {
            store.write_snapshot(&make(round)).expect("write");
        }
        let ids = store.scan_ids().expect("ids");
        let (a, b) = (ids[ids.len() - 2], ids[ids.len() - 1]);

        let started = std::time::Instant::now();
        let latest = store.trend(a, b).expect("trend");
        let warm = started.elapsed();

        assert_eq!(latest.len(), rows_per_scan as usize);
        assert!(
            warm < cold.max(Duration::from_millis(10)) * 4,
            "the diff took {warm:?} against {cold:?} with 120 scans of history"
        );
    }

    #[test]
    fn a_scan_diffed_against_itself_reports_no_change() {
        let (_dir, path) = scratch();
        let mut store = Store::open(&path).expect("open");
        let id = store
            .write_snapshot(&snapshot(vec![entry("/p/target", 42)]))
            .expect("write");

        let rows = store.trend(id, id).expect("trend");

        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].change, Change::Unchanged);
    }
}

mod history {
    use super::*;

    use std::path::PathBuf;
    use std::time::{Duration, SystemTime, UNIX_EPOCH};

    use dev_cleaner::store::Snapshot;

    fn at(secs: u64) -> SystemTime {
        UNIX_EPOCH + Duration::from_secs(secs)
    }

    /// A scan of `roots` at `secs`, stamped with a reclaimable total that
    /// says which scan it is.
    fn scan_of(roots: &[&str], secs: u64, reclaimable: u64) -> Snapshot {
        Snapshot {
            started_at: at(secs),
            roots: roots.iter().map(PathBuf::from).collect(),
            total_bytes_apparent: 0,
            total_bytes_unique: 0,
            total_inodes: 0,
            reclaimable_unique: Some(reclaimable),
            projects: Vec::new(),
            entries: Vec::new(),
        }
    }

    /// Scans of two root sets, written turn about, so a history that read
    /// the table in order would mix them.
    fn interleaved(store: &mut Store) {
        for (secs, roots) in [
            (1, &["/a"][..]),
            (2, &["/a", "/b"][..]),
            (3, &["/a"][..]),
            (4, &["/a", "/b"][..]),
            (5, &["/a"][..]),
            (6, &["/a", "/b"][..]),
            (7, &["/a"][..]),
        ] {
            store
                .write_snapshot(&scan_of(roots, secs, secs * 10))
                .expect("write");
        }
    }

    /// A sparkline of one root set drawn from another's scans would show the
    /// disk jumping by the size of a whole directory tree that was never
    /// touched. The history is of these roots and nothing else.
    #[test]
    fn the_history_is_of_the_same_root_set_and_nothing_else() {
        let (_dir, path) = scratch();
        let mut store = Store::open(&path).expect("open");
        interleaved(&mut store);

        assert_eq!(
            store.history(&[PathBuf::from("/a")], 10).expect("history"),
            vec![
                (at(1), Some(10)),
                (at(3), Some(30)),
                (at(5), Some(50)),
                (at(7), Some(70)),
            ]
        );
        assert_eq!(
            store
                .history(&[PathBuf::from("/a"), PathBuf::from("/b")], 10)
                .expect("history"),
            vec![(at(2), Some(20)), (at(4), Some(40)), (at(6), Some(60))]
        );
        assert_eq!(
            store
                .history(&[PathBuf::from("/never-scanned")], 10)
                .expect("history"),
            Vec::new(),
            "an unseen root set has no history at all"
        );
    }

    /// The cap keeps the newest scans, not the oldest: a line of the last
    /// thirty runs is the point, and the order stays oldest first so a
    /// caller draws it left to right without turning it round.
    #[test]
    fn the_cap_keeps_the_newest_scans_and_the_order_stays_oldest_first() {
        let (_dir, path) = scratch();
        let mut store = Store::open(&path).expect("open");
        interleaved(&mut store);

        assert_eq!(
            store.history(&[PathBuf::from("/a")], 2).expect("history"),
            vec![(at(5), Some(50)), (at(7), Some(70))]
        );
        assert_eq!(
            store.history(&[PathBuf::from("/a")], 0).expect("history"),
            Vec::new()
        );
    }

    /// The same territory named in a different order is the same history,
    /// exactly as the trend's baseline treats it. Two matching rules would
    /// let the trend find a previous scan the sparkline does not show.
    #[test]
    fn the_root_set_is_matched_as_a_set_like_the_baseline_is() {
        let (_dir, path) = scratch();
        let mut store = Store::open(&path).expect("open");
        interleaved(&mut store);

        let asked = [
            PathBuf::from("/b"),
            PathBuf::from("/a"),
            PathBuf::from("/b"),
        ];
        assert_eq!(
            store.history(&asked, 10).expect("history").len(),
            3,
            "{asked:?} did not match the scans of the same roots"
        );
        assert_eq!(
            store.latest_scan_for(&asked).expect("baseline").is_some(),
            !store.history(&asked, 1).expect("history").is_empty(),
            "the baseline and the history disagree about whether these roots were scanned"
        );
    }

    /// A scan recorded before the total was kept is a gap in the line, not a
    /// point at zero. Zero is what the line would show after a purge that
    /// cleared everything, and that is a different day.
    #[test]
    fn a_scan_recorded_before_the_total_was_kept_is_a_gap_not_a_zero() {
        let (_dir, path) = scratch();
        store_at_version_one(&path);
        let mut store = Store::open(&path).expect("open");
        store
            .write_snapshot(&scan_of(&["/r"], 2, 42))
            .expect("write");

        assert_eq!(
            store.history(&[PathBuf::from("/r")], 10).expect("history"),
            vec![
                (UNIX_EPOCH + Duration::from_nanos(1), None),
                (at(2), Some(42))
            ]
        );
    }
}

/// What the tool remembers of its own runs.
///
/// Migration 2, after the reclaimable column: a `purge` row per run, written by
/// the interface and the command line alike, so the result screen can compare a
/// run with the ones before it without parsing a markdown file.
mod purge_history {
    use super::*;
    use dev_cleaner::purge::Remover;
    use dev_cleaner::purge::{Manifest, execute, execute_with};
    use dev_cleaner::safety::{Candidate, Confirmed, Plan, RegenCommand, Safety};
    use dev_cleaner::store::{PurgeRun, RunSummary, record_purge_run, summarize};
    use std::path::PathBuf;
    use std::sync::atomic::AtomicBool;
    use std::time::Duration;

    struct Naps(Duration);

    impl Remover for Naps {
        fn remove(&self, path: &Path) -> std::io::Result<PathBuf> {
            std::thread::sleep(self.0);
            if path.ends_with("bad") {
                return Err(std::io::Error::other("no"));
            }
            Ok(PathBuf::from("/t").join(path.file_name().expect("name")))
        }
    }

    fn plan(names: &[(&str, u64)]) -> Plan<Confirmed> {
        let mut draft = Plan::draft();
        for (name, bytes) in names {
            draft
                .add(Candidate {
                    path: PathBuf::from(name),
                    bytes: *bytes,
                    safety: Safety::Regenerable {
                        regen: RegenCommand::new("npm install").expect("valid"),
                    },
                })
                .expect("selectable");
        }
        let reviewed = draft.review();
        let phrase = reviewed.confirmation_phrase();
        reviewed.confirm(&phrase).expect("phrase")
    }

    fn run(names: &[(&str, u64)], nap: u64) -> Manifest {
        execute(plan(names), &Naps(Duration::from_millis(nap)))
    }

    #[test]
    fn a_store_at_version_two_gains_the_purge_table_and_keeps_its_scan() {
        let (_dir, path) = scratch();
        std::fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
        let conn = rusqlite::Connection::open(&path).expect("raw");
        conn.execute_batch(&format!(
            "BEGIN; {} {} PRAGMA user_version = 2; COMMIT;",
            Store::MIGRATIONS[0],
            Store::MIGRATIONS[1]
        ))
        .expect("the first two migrations");
        conn.execute(
            "INSERT INTO scan (started_at, root_set, total_bytes_apparent, \
             total_bytes_unique, total_inodes) VALUES (1, '/r', 3, 2, 1)",
            [],
        )
        .expect("a scan");
        drop(conn);

        let store = Store::open(&path).expect("migrate");

        assert_eq!(store.schema_version().expect("version"), 3);
        assert!(store.has_table("purge").expect("query"));
        assert_eq!(store.scan_ids().expect("ids").len(), 1);
    }

    #[test]
    fn a_purge_row_round_trips() {
        let (_dir, path) = scratch();
        let store = Store::open(&path).expect("open");
        let m = run(&[("/p/a/ok", 100), ("/p/b/bad", 200), ("/p/c/ok", 300)], 5);
        let record = PathBuf::from("/state/manifests/purge-1.md");

        let id = store.record_purge(&m, Some(&record)).expect("record");
        let rows = store.purge_runs().expect("read");

        assert_eq!(rows.len(), 1);
        let row = &rows[0];
        assert_eq!(row.id, id);
        assert_eq!(row.executed_at, m.executed_at);
        assert_eq!(
            (
                row.items_planned,
                row.items_moved,
                row.items_failed,
                row.items_skipped
            ),
            (3, 2, 1, 0)
        );
        assert_eq!((row.bytes_expected, row.bytes_moved), (600, 400));
        assert!(
            row.elapsed >= Duration::from_millis(15),
            "{:?}",
            row.elapsed
        );
        assert_eq!(row.manifest_path.as_deref(), Some(record.as_path()));
    }

    #[test]
    fn a_run_whose_record_could_not_be_written_is_still_remembered() {
        let (_dir, path) = scratch();
        let store = Store::open(&path).expect("open");
        store
            .record_purge(&run(&[("/p/a/ok", 1)], 0), None)
            .expect("record");

        assert_eq!(store.purge_runs().expect("read")[0].manifest_path, None);
    }

    #[test]
    fn a_stopped_run_is_stored_with_its_skipped_items() {
        let (_dir, path) = scratch();
        let store = Store::open(&path).expect("open");
        let stop = AtomicBool::new(false);
        let m = execute_with(
            plan(&[("/p/a/ok", 1), ("/p/b/ok", 2), ("/p/c/ok", 3)]),
            &Naps(Duration::ZERO),
            &stop,
            &mut |r| {
                if r.items.len() == 1 {
                    stop.store(true, std::sync::atomic::Ordering::SeqCst)
                }
            },
        );

        store.record_purge(&m, None).expect("record");
        let row = &store.purge_runs().expect("read")[0];

        assert_eq!(
            (
                row.items_planned,
                row.items_moved,
                row.items_failed,
                row.items_skipped
            ),
            (3, 1, 0, 2)
        );
    }

    #[test]
    fn the_command_line_and_the_interface_write_through_the_same_door() {
        let (_dir, path) = scratch();
        let m = run(&[("/p/a/ok", 100)], 0);

        let id = record_purge_run(&path, &m, None).expect("record");

        let rows = Store::open(&path)
            .expect("open")
            .purge_runs()
            .expect("read");
        assert_eq!(rows.iter().map(|r| r.id).collect::<Vec<_>>(), vec![id]);
    }

    fn row(id: i64, secs: u64, moved: u64, bytes: u64, ms: u64, failed: u64) -> PurgeRun {
        PurgeRun {
            id,
            executed_at: std::time::UNIX_EPOCH + Duration::from_secs(secs),
            items_planned: moved + failed,
            items_moved: moved,
            items_failed: failed,
            items_skipped: 0,
            bytes_expected: bytes,
            bytes_moved: bytes,
            elapsed: Duration::from_millis(ms),
            manifest_path: None,
        }
    }

    #[test]
    fn the_summary_comes_from_the_rows() {
        let rows = vec![
            row(1, 1000, 5, 500, 4000, 0),
            row(2, 2000, 5, 900, 2100, 0),
            row(3, 3000, 5, 100, 3000, 0),
        ];

        let s = summarize(&rows, Some(1)).expect("some");

        assert_eq!(s.runs, 3);
        assert_eq!(s.since, std::time::UNIX_EPOCH + Duration::from_secs(1000));
        assert_eq!(s.bytes_moved, 1500);
        assert_eq!(s.largest, 900);
        assert_eq!(s.fastest, Some(Duration::from_millis(2100)));
        assert_eq!(
            s.this_rank,
            Some(2),
            "500 is the second largest of 900, 500, 100"
        );
    }

    #[test]
    fn a_run_that_failed_is_not_the_fastest() {
        // Quick because it gave up. It would win every time otherwise.
        let rows = vec![row(1, 1, 5, 500, 4000, 0), row(2, 2, 0, 0, 10, 5)];

        assert_eq!(
            summarize(&rows, Some(1)).expect("some").fastest,
            Some(Duration::from_millis(4000))
        );
    }

    #[test]
    fn a_run_that_was_never_stored_has_no_rank() {
        let rows = vec![row(1, 1, 5, 500, 4000, 0)];

        assert_eq!(summarize(&rows, Some(99)).expect("some").this_rank, None);
        assert_eq!(summarize(&rows, None).expect("some").this_rank, None);
    }

    #[test]
    fn no_rows_no_summary() {
        let none: Option<RunSummary> = summarize(&[], None);
        assert!(none.is_none());
    }
}
