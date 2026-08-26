//! Persistence: schema, migrations, snapshot round-trip and trends.

use dev_cleaner::store::{Store, db_path};
use tempfile::TempDir;

fn scratch() -> (TempDir, std::path::PathBuf) {
    let dir = TempDir::new().expect("tempdir");
    let path = dir.path().join("state/db.sqlite3");
    (dir, path)
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
