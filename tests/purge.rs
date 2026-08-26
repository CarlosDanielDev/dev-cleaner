pub mod common;

use std::cell::RefCell;
use std::path::{Path, PathBuf};

use dev_cleaner::purge::{Outcome, Remover, execute};
use dev_cleaner::safety::{Candidate, Plan, RegenCommand, Safety};

/// Records what it was asked to remove instead of removing it, so the suite
/// never puts anything in the real Trash.
#[derive(Default)]
struct Recorder {
    seen: RefCell<Vec<PathBuf>>,
    fail_on: Option<&'static str>,
}

impl Remover for Recorder {
    fn remove(&self, path: &Path) -> std::io::Result<PathBuf> {
        self.seen.borrow_mut().push(path.to_path_buf());
        if self.fail_on.is_some_and(|f| path.ends_with(f)) {
            return Err(std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                "permission denied",
            ));
        }
        Ok(PathBuf::from("/Users/test/.Trash").join(path.file_name().unwrap()))
    }
}

/// Stands in for a sanctioned cleanup command, which deletes immediately
/// instead of routing through the Trash.
struct ImmediateRecorder;

impl Remover for ImmediateRecorder {
    fn remove(&self, path: &Path) -> std::io::Result<PathBuf> {
        Ok(path.to_path_buf())
    }
    fn frees_space_immediately(&self) -> bool {
        true
    }
}

fn candidate(name: &str, bytes: u64) -> Candidate {
    Candidate {
        path: PathBuf::from(name),
        bytes,
        safety: Safety::Regenerable {
            regen: RegenCommand::new("npm install").expect("valid"),
        },
    }
}

fn confirmed(items: Vec<Candidate>) -> Plan<dev_cleaner::safety::Confirmed> {
    let mut draft = Plan::draft();
    for c in items {
        draft.add(c).expect("selectable");
    }
    let reviewed = draft.review();
    let phrase = reviewed.confirmation_phrase();
    reviewed.confirm(&phrase).expect("phrase matches")
}

#[test]
fn every_item_goes_through_the_remover() {
    let rec = Recorder::default();
    let plan = confirmed(vec![
        candidate("a/node_modules", 100),
        candidate("b/target", 200),
    ]);

    let manifest = execute(plan, &rec);

    assert_eq!(rec.seen.borrow().len(), 2, "both items were handed over");
    assert_eq!(manifest.bytes_expected, 300);
    assert!(
        manifest
            .items
            .iter()
            .all(|i| matches!(i.result, Outcome::Removed { .. }))
    );
}

#[test]
fn one_failure_does_not_abort_the_rest() {
    let rec = Recorder {
        fail_on: Some("target"),
        ..Default::default()
    };
    let plan = confirmed(vec![
        candidate("a/node_modules", 100),
        candidate("b/target", 200),
        candidate("c/.venv", 300),
    ]);

    let manifest = execute(plan, &rec);

    assert_eq!(
        rec.seen.borrow().len(),
        3,
        "the run continued past the failing item"
    );
    assert_eq!(manifest.removed().count(), 2);
    assert_eq!(manifest.failed().count(), 1);
}

#[test]
fn a_failed_item_records_why_and_is_not_counted_as_freed() {
    let rec = Recorder {
        fail_on: Some("target"),
        ..Default::default()
    };
    let plan = confirmed(vec![
        candidate("a/node_modules", 100),
        candidate("b/target", 200),
    ]);

    let manifest = execute(plan, &rec);

    let failed = manifest.failed().next().expect("one failure");
    assert!(failed.path.ends_with("target"));
    match &failed.result {
        Outcome::Failed { error } => assert!(
            error.contains("permission denied"),
            "the real cause must survive into the record: {error}"
        ),
        other => panic!("expected a failure, got {other:?}"),
    }

    assert_eq!(
        manifest.bytes_moved(),
        100,
        "bytes for a failed item must not be reported as moved"
    );
    assert_eq!(manifest.bytes_expected, 300, "the plan still expected both");
}

#[test]
fn a_run_reports_whether_it_was_complete() {
    let ok = execute(confirmed(vec![candidate("a", 1)]), &Recorder::default());
    assert!(ok.is_complete());

    let partial = execute(
        confirmed(vec![candidate("a", 1), candidate("b/target", 1)]),
        &Recorder {
            fail_on: Some("target"),
            ..Default::default()
        },
    );
    assert!(
        !partial.is_complete(),
        "a partial run must be distinguishable from a clean one"
    );
}

mod manifest {
    use super::*;
    use dev_cleaner::purge::{manifest_dir, write_manifest};

    #[test]
    fn the_record_names_every_path_its_size_and_where_it_went() {
        let m = execute(
            confirmed(vec![
                candidate("a/node_modules", 1024),
                candidate("b/target", 2048),
            ]),
            &Recorder::default(),
        );
        let text = m.render();

        assert!(text.contains("a/node_modules"), "source path missing");
        assert!(text.contains("b/target"), "source path missing");
        assert!(text.contains(".Trash"), "destination missing");
        assert!(text.contains("npm install"), "regeneration command missing");
    }

    #[test]
    fn the_record_explains_how_to_undo_it() {
        // The instructions have to live in the file. A user reading this months
        // later will not have the terminal session that produced it.
        let m = execute(confirmed(vec![candidate("a", 1)]), &Recorder::default());
        let text = m.render().to_lowercase();

        assert!(text.contains("restore"), "no restore section");
        assert!(text.contains("trash"), "does not say where things went");
        assert!(
            text.contains("empty"),
            "must say the space is not reclaimed until the Trash is emptied"
        );
    }

    #[test]
    fn a_partial_run_still_produces_a_record_naming_the_failure() {
        let m = execute(
            confirmed(vec![candidate("a", 1), candidate("b/target", 2)]),
            &Recorder {
                fail_on: Some("target"),
                ..Default::default()
            },
        );
        let text = m.render();

        assert!(text.contains("permission denied"), "failure cause missing");
        assert!(text.contains("b/target"), "failed path missing");
    }

    #[test]
    fn a_record_is_written_to_disk_and_read_back() {
        let tmp = super::common::Fixture::new();
        let m = execute(
            confirmed(vec![candidate("a/node_modules", 1)]),
            &Recorder::default(),
        );

        let path = write_manifest(&m, tmp.root()).expect("written");

        assert!(path.exists(), "manifest file was not created");
        let text = std::fs::read_to_string(&path).expect("readable");
        assert!(text.contains("a/node_modules"));
    }

    #[test]
    fn manifests_are_stored_where_the_tool_would_never_clean() {
        // A record that the tool could later delete is not a record.
        let home = std::path::PathBuf::from(std::env::var("HOME").unwrap());
        let dir = manifest_dir();

        for cache in dev_cleaner::classify::cache_kinds() {
            let cache_path = home.join(cache.rel_path);
            assert!(
                !dir.starts_with(&cache_path),
                "manifests would sit inside the {} cache",
                cache.name
            );
        }
        assert!(
            !dir.starts_with(home.join("projects")),
            "manifests would sit inside a scanned project root"
        );
    }
}

mod verify {
    use super::*;
    use dev_cleaner::purge::free_bytes;

    #[test]
    fn free_space_is_read_from_the_volume_holding_the_given_path() {
        // Never hardcode "/": a scanned root may live on an external disk or a
        // separate mount, where the root filesystem's numbers say nothing about
        // what a purge there would return. Verified against df on the reference
        // machine, where both report 110.31 GB.
        let home = std::path::PathBuf::from(std::env::var("HOME").unwrap());
        let here = free_bytes(&home).expect("home is on a real volume");

        assert!(here > 0, "a mounted volume reports free space");

        let nested = free_bytes(&home.join(".")).expect("same volume");
        let drift = here.abs_diff(nested) as f64 / here as f64;
        assert!(
            drift < 0.01,
            "two paths on one volume should agree: {here} vs {nested}"
        );
    }

    #[test]
    fn a_path_that_does_not_exist_has_no_free_space_to_report() {
        assert!(free_bytes(std::path::Path::new("/nope/not/here")).is_none());
    }

    #[test]
    fn expected_and_actual_are_kept_apart() {
        let mut m = execute(
            confirmed(vec![candidate("a", 1000), candidate("b", 2000)]),
            &Recorder::default(),
        );
        assert_eq!(m.bytes_expected, 3000);
        assert_eq!(m.bytes_actual, None, "actual is unknown until measured");

        m.record_actual(2900);
        assert_eq!(m.bytes_actual, Some(2900));
        assert_eq!(m.bytes_expected, 3000, "the prediction is not overwritten");
    }

    #[test]
    fn trashed_bytes_are_reported_as_pending_not_as_reclaimed() {
        // Moving to the Trash does not free anything: the Trash is on the same
        // disk. A near-zero change in free space is the expected outcome here,
        // so reporting it as a shortfall would raise an alarm about normal
        // behaviour and blame a cause that had nothing to do with it.
        let mut m = execute(
            confirmed(vec![candidate("a", 12_000_000)]),
            &Recorder::default(),
        );
        m.record_actual(0);

        assert_eq!(m.pending_in_trash(), 12_000_000);
        assert!(
            m.shortfall().is_none(),
            "trashed items must not be reported as a shortfall"
        );

        let text = m.render();
        assert!(
            !text.contains("hardlinked"),
            "must not blame hardlinks for space the Trash is still holding"
        );
        assert!(
            text.to_lowercase().contains("empty the trash"),
            "must say what actually reclaims the space"
        );
    }

    #[test]
    fn a_large_shortfall_is_surfaced_for_immediate_deletion() {
        // Sanctioned cleanup commands (docker prune, go clean -modcache) delete
        // at once rather than trashing, so their free-space delta is meaningful
        // and a large gap is worth explaining.
        let mut m = execute(confirmed(vec![candidate("a", 1000)]), &ImmediateRecorder);

        m.record_actual(950);
        assert!(m.shortfall().is_none(), "5% is within tolerance");

        m.record_actual(400);
        let gap = m.shortfall().expect("60% shortfall must be reported");
        assert!((gap - 0.6).abs() < 0.001);

        assert!(
            m.render().contains("hardlinked"),
            "the record should explain why the disk gave back less"
        );
    }
}
