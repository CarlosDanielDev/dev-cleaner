//! `execute_with`: the record reported as it grows, not only once it is done.

pub mod common;

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use common::purge::{Recorder, candidate, confirmed};
use dev_cleaner::purge::{Outcome, Remover, execute, execute_with, write_manifest};

#[test]
fn every_item_is_reported_once_in_plan_order_with_its_outcome() {
    let rec = Recorder {
        fail_on: Some("target"),
        ..Default::default()
    };
    let plan = confirmed(vec![
        candidate("a/node_modules", 100),
        candidate("b/target", 200),
        candidate("c/.venv", 300),
    ]);
    let mut reported: Vec<(PathBuf, Outcome)> = Vec::new();

    let manifest = execute_with(plan, &rec, &mut |record| {
        let item = record.items.last().expect("reported after an item moved");
        reported.push((item.path.clone(), item.result.clone()));
    });

    let recorded: Vec<(PathBuf, Outcome)> = manifest
        .items
        .iter()
        .map(|i| (i.path.clone(), i.result.clone()))
        .collect();
    assert_eq!(
        reported, recorded,
        "once per item, in order, with the outcome the record holds"
    );
    assert!(
        matches!(reported[1].1, Outcome::Failed { .. }),
        "the failure in the middle is reported as one, not smoothed over"
    );
}

#[test]
fn the_report_sees_the_record_grow_by_one_each_time() {
    let plan = confirmed(vec![
        candidate("a", 1),
        candidate("b", 2),
        candidate("c", 3),
    ]);
    let mut lengths = Vec::new();

    execute_with(plan, &Recorder::default(), &mut |record| {
        lengths.push(record.items.len());
    });

    assert_eq!(lengths, [1, 2, 3]);
}

/// Takes 10 ms over each item and notes when it finished.
struct Slow {
    done_at: RefCell<Vec<SystemTime>>,
}

impl Remover for Slow {
    fn remove(&self, path: &Path) -> std::io::Result<PathBuf> {
        std::thread::sleep(Duration::from_millis(10));
        self.done_at.borrow_mut().push(SystemTime::now());
        Ok(PathBuf::from("/Users/test/.Trash").join(path.file_name().unwrap()))
    }
}

#[test]
fn the_record_is_stamped_when_the_run_begins_not_when_it_ends() {
    // The stamp names the file. A record written while the run is still going
    // can only land on the same file as the final one if the stamp is already
    // fixed before the first item moves.
    let slow = Slow {
        done_at: RefCell::new(Vec::new()),
    };

    let manifest = execute(confirmed(vec![candidate("a", 1), candidate("b", 2)]), &slow);

    let first_done = slow.done_at.borrow()[0];
    assert!(
        manifest.executed_at < first_done,
        "stamped at {:?}, but the first item had already finished at {first_done:?}",
        manifest.executed_at
    );
}

#[test]
fn a_record_written_mid_run_lands_on_the_same_file_as_the_final_one() {
    let tmp = common::Fixture::new();
    let plan = confirmed(
        (1..=5)
            .map(|i| candidate(&format!("p{i}/node_modules"), i))
            .collect(),
    );
    let mut partial: Option<(PathBuf, String)> = None;

    let manifest = execute_with(plan, &Recorder::default(), &mut |record| {
        if record.items.len() == 2 {
            let path = write_manifest(record, tmp.root()).expect("written mid-run");
            let text = std::fs::read_to_string(&path).expect("readable");
            partial = Some((path, text));
        }
    });
    let final_path = write_manifest(&manifest, tmp.root()).expect("written at the end");

    let (partial_path, partial_text) = partial.expect("the callback saw item 2 of 5");
    assert_eq!(partial_path, final_path, "one run, one file");
    assert!(
        partial_text.contains("p1/node_modules") && partial_text.contains("p2/node_modules"),
        "the mid-run record lists the two items that had moved"
    );
    assert!(
        !partial_text.contains("p3/node_modules"),
        "the mid-run record does not list an item that had not moved yet"
    );

    let final_text = std::fs::read_to_string(&final_path).expect("readable");
    assert!(
        (1..=5).all(|i| final_text.contains(&format!("p{i}/node_modules"))),
        "the final record lists all five"
    );
    assert_eq!(
        std::fs::read_dir(tmp.root()).expect("dir").count(),
        1,
        "the final write replaced the mid-run one rather than sitting beside it"
    );
}
