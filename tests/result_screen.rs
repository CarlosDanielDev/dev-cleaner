//! The result screen: what a purge actually did, and what it did not.
//!
//! Every assertion here is about the difference between a prediction and a
//! result. The first end-to-end run of this tool reported a 97% shortfall and
//! blamed hardlinks for space the Trash was simply still holding, which is the
//! mistake this screen is shaped to make impossible.

pub mod common;

use common::purge::{ImmediateRecorder, Recorder, candidate, confirmed};
use dev_cleaner::purge::{Manifest, execute, restore_steps, trash_note};
use dev_cleaner::tui::Report;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use std::path::{Path, PathBuf};

const MB: u64 = 1024 * 1024;

fn record() -> PathBuf {
    PathBuf::from("/Users/test/.local/state/dev-cleaner/manifests/purge-1750000000.md")
}

fn drawn(m: &Manifest, path: Option<&Path>, width: u16) -> String {
    let area = Rect::new(0, 0, width, 60);
    let mut buf = Buffer::empty(area);
    Report::new().render(m, path, area, &mut buf);
    (0..area.height)
        .map(|y| {
            (0..area.width)
                .map(|x| buf[(x, y)].symbol().to_string())
                .collect::<String>()
                .trim_end()
                .to_string()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn text(m: &Manifest) -> String {
    drawn(m, Some(&record()), 110)
}

/// Whitespace removed, so a sentence can be looked for regardless of where the
/// wrapping happened to break it.
fn squashed(s: &str) -> String {
    s.chars().filter(|c| !c.is_whitespace()).collect()
}

/// A trashed run where one of three items could not be moved.
fn partial() -> Manifest {
    execute(
        confirmed(vec![
            candidate("/p/a/node_modules", 100 * MB),
            candidate("/p/b/target", 200 * MB),
        ]),
        &Recorder {
            fail_on: Some("target"),
            ..Default::default()
        },
    )
}

#[test]
fn what_was_planned_and_what_actually_moved_are_separate_lines() {
    // The plan expected 300 MB. 100 MB moved. A screen that shows one number
    // has to choose, and choosing the prediction is how the tool lies.
    let m = partial();
    let text = text(&m);

    assert!(text.contains("100.00 MB"), "moved bytes missing:\n{text}");
    assert!(text.contains("300.00 MB"), "planned bytes missing:\n{text}");
    assert!(
        text.to_lowercase().contains("planned"),
        "nothing says which number was the prediction:\n{text}"
    );
}

#[test]
fn a_trashed_run_that_freed_nothing_is_not_reported_as_a_shortfall() {
    // Free space genuinely did not move: the Trash is on the same disk. The
    // screen must say that rather than raise an alarm about it.
    let mut m = execute(
        confirmed(vec![candidate("/p/a/node_modules", 100 * MB)]),
        &Recorder::default(),
    );
    m.record_actual(0);

    assert_eq!(m.shortfall(), None, "a trashed run cannot fall short");

    let text = text(&m);
    assert!(
        squashed(&text).contains(&squashed(trash_note())),
        "the screen does not explain why free space is unchanged:\n{text}"
    );
    assert!(
        !text.to_lowercase().contains("less than"),
        "a trashed run must not be described as a deficit:\n{text}"
    );
    assert!(
        text.contains("100.00 MB"),
        "the bytes waiting in the Trash are not shown:\n{text}"
    );
}

#[test]
fn a_run_that_frees_space_at_once_reports_what_the_disk_returned() {
    let mut m = execute(
        confirmed(vec![candidate("/p/a/node_modules", 100 * MB)]),
        &ImmediateRecorder,
    );
    m.record_actual(100 * MB);

    let text = text(&m).to_lowercase();
    assert!(
        text.contains("reclaimed"),
        "a measured result is not shown:\n{text}"
    );
    // Nothing went to the Trash, so an instruction to recover it from there
    // would send the user somewhere the files have never been.
    assert!(
        !text.contains("put back"),
        "this run deleted outright; there is nothing to put back:\n{text}"
    );
}

#[test]
fn a_real_shortfall_is_still_explained_when_removal_frees_space_at_once() {
    // The guard above must not have turned into silence. A remover that does
    // free space immediately and returned far less still owes an explanation.
    let mut m = execute(
        confirmed(vec![candidate("/p/a/node_modules", 100 * MB)]),
        &ImmediateRecorder,
    );
    m.record_actual(10 * MB);

    assert!(m.shortfall().is_some(), "90% less is a shortfall");
    let text = text(&m).to_lowercase();
    assert!(
        text.contains("less than"),
        "a measured shortfall goes unexplained:\n{text}"
    );
}

#[test]
fn every_failure_is_listed_with_its_own_error() {
    // Two items fail for the same kind of reason at different paths. A count,
    // or one shared sentence, loses which path the user has to go and fix.
    let m = execute(
        confirmed(vec![
            candidate("/p/a/node_modules", 100 * MB),
            candidate("/p/b/target", 200 * MB),
            candidate("/p/c/target", 300 * MB),
        ]),
        &Recorder {
            fail_on: Some("target"),
            ..Default::default()
        },
    );
    assert_eq!(m.failed().count(), 2);

    let text = text(&m);
    for failed in m.failed() {
        let path = failed.path.display().to_string();
        let error = match &failed.result {
            dev_cleaner::purge::Outcome::Failed { error } => error.clone(),
            other => panic!("expected a failure, got {other:?}"),
        };
        assert!(
            text.contains(&path),
            "{path} failed and is not on the screen:\n{text}"
        );
        assert!(
            squashed(&text).contains(&squashed(&error)),
            "the error for {path} is missing:\n{text}"
        );
    }
}

#[test]
fn a_clean_run_lists_no_failures_at_all() {
    // The section above must be absent rather than empty: a heading with
    // nothing under it reads as something having gone wrong.
    let m = execute(
        confirmed(vec![candidate("/p/a/node_modules", 100 * MB)]),
        &Recorder::default(),
    );

    assert!(m.is_complete());
    assert!(
        !text(&m).to_lowercase().contains("not moved"),
        "a clean run must not show a failure section"
    );
}

#[test]
fn the_record_path_is_shown_whole_and_never_elided() {
    // A path with a mark where the middle used to be cannot be opened, copied
    // or pasted. On a narrow terminal it wraps instead.
    let m = partial();
    let path = record();

    for width in [110, 60, 40, 24] {
        let text = drawn(&m, Some(&path), width);
        assert!(
            !text.contains('…'),
            "something was elided at width {width}:\n{text}"
        );
        assert!(
            squashed(&text).contains(&squashed(&path.display().to_string())),
            "the record path is not on screen whole at width {width}:\n{text}"
        );
    }
}

#[test]
fn a_record_that_could_not_be_written_says_so_instead_of_naming_a_file() {
    let m = partial();
    let text = drawn(&m, None, 110);

    assert!(
        !text.contains(".md"),
        "a file that does not exist must not be offered:\n{text}"
    );
    assert!(
        text.to_lowercase().contains("could not be written"),
        "the missing record goes unmentioned:\n{text}"
    );
}

#[test]
fn the_screen_says_how_to_put_things_back_and_that_the_trash_must_be_emptied() {
    let m = partial();
    let text = text(&m).to_lowercase();

    assert!(text.contains("put back"), "no restore instruction:\n{text}");
    assert!(
        text.contains("empty"),
        "must say the space is not reclaimed until the Trash is emptied:\n{text}"
    );
}

#[test]
fn the_screen_and_the_written_record_say_the_same_thing() {
    // Two wordings drift, and the one that drifts is the one nobody reads
    // while testing. Both read from the same sentences.
    let m = partial();
    let screen = squashed(&text(&m));
    let file = squashed(&m.render());

    assert!(screen.contains(&squashed(trash_note())));
    assert!(file.contains(&squashed(trash_note())));
    for step in restore_steps(false) {
        assert!(
            file.contains(&squashed(step)),
            "the record dropped a restore step: {step}"
        );
        assert!(
            screen.contains(&squashed(step)),
            "the screen dropped a restore step: {step}"
        );
    }
}

#[test]
fn the_screen_draws_into_an_area_too_small_for_it_without_panicking() {
    let m = partial();
    for (w, h) in [(0, 0), (1, 1), (10, 3), (40, 5)] {
        let area = Rect::new(0, 0, w, h);
        let mut buf = Buffer::empty(area);
        Report::new().render(&m, Some(&record()), area, &mut buf);
    }
}
