//! The result screen: what a purge actually did, and what it did not.
//!
//! Every assertion here is about the difference between a prediction and a
//! result. The first end-to-end run of this tool reported a 97% shortfall and
//! blamed hardlinks for space the Trash was simply still holding, which is the
//! mistake this screen is shaped to make impossible.

pub mod common;

use common::purge::{ImmediateRecorder, Recorder, Sleeper, candidate, confirmed};
use dev_cleaner::bytes::human;
use dev_cleaner::purge::{Manifest, execute, execute_with, restore_steps, took, trash_note};
use dev_cleaner::store::RunSummary;
use dev_cleaner::tui::{Motion, Report};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, UNIX_EPOCH};

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

const NOT_ATTEMPTED: &str =
    "These are untouched and still on disk. They will be offered again by the next scan.";

/// Three items, stopped once the first had moved.
fn stopped() -> Manifest {
    let stop = AtomicBool::new(false);
    execute_with(
        confirmed(vec![
            candidate("/p/a/node_modules", 100 * MB),
            candidate("/p/b/target", 200 * MB),
            candidate("/p/c/.venv", 300 * MB),
        ]),
        &Recorder::default(),
        &stop,
        &mut |record| {
            if record.items.len() == 1 {
                stop.store(true, Ordering::SeqCst);
            }
        },
    )
}

#[test]
fn a_stopped_run_lists_what_was_not_attempted_in_its_own_section() {
    let m = stopped();
    let text = text(&m);

    assert!(
        text.contains("Not attempted"),
        "no section:
{text}"
    );
    assert!(
        text.contains("/p/b/target") && text.contains("/p/c/.venv"),
        "{text}"
    );
    assert!(
        squashed(&text).contains(&squashed(NOT_ATTEMPTED)),
        "the sentence is missing:
{text}"
    );
    assert!(
        text.contains("2 not attempted"),
        "the headline hides the stop:
{text}"
    );
    assert!(
        !text.contains("Not moved"),
        "nothing failed, so nothing was 'not moved':
{text}"
    );
}

#[test]
fn the_not_attempted_section_is_drawn_in_default_because_nothing_went_wrong() {
    let m = stopped();
    let area = Rect::new(0, 0, 110, 60);
    let mut buf = Buffer::empty(area);
    Report::new().render(&m, Some(&record()), area, &mut buf);

    let rows: Vec<u16> = (0..area.height)
        .filter(|&y| {
            let line: String = (0..area.width).map(|x| buf[(x, y)].symbol()).collect();
            line.contains("/p/b/target")
                || line.contains("/p/c/.venv")
                || line.contains("200.00 MB")
                || line.contains("300.00 MB")
        })
        .collect();
    assert!(rows.len() >= 2, "the skipped rows were not found");
    for y in rows {
        for x in 0..area.width {
            assert_eq!(
                buf[(x, y)].fg,
                ratatui::style::Color::Reset,
                "row {y} is coloured; a stop is not a warning"
            );
        }
    }
}

#[test]
fn the_written_record_has_the_same_section_and_the_same_sentence() {
    let m = stopped();
    let file = m.render();

    assert!(file.contains("## Not attempted"), "{file}");
    assert!(
        file.contains("/p/b/target") && file.contains("/p/c/.venv"),
        "{file}"
    );
    assert!(squashed(&file).contains(&squashed(NOT_ATTEMPTED)), "{file}");
    assert!(!file.contains("## Not moved"), "{file}");
    assert!(
        !file.contains("every item moved"),
        "a stopped run is not a complete one:\n{file}"
    );
    assert!(file.contains("1 moved, 2 not attempted"), "{file}");
}

#[test]
fn a_run_that_both_failed_and_stopped_lists_each_in_its_own_section() {
    let stop = AtomicBool::new(false);
    let m = execute_with(
        confirmed(vec![
            candidate("/p/a/target", 100 * MB),
            candidate("/p/b/node_modules", 200 * MB),
        ]),
        &Recorder {
            fail_on: Some("target"),
            ..Default::default()
        },
        &stop,
        &mut |_| stop.store(true, Ordering::SeqCst),
    );
    let text = text(&m);

    assert!(
        text.contains("Not moved") && text.contains("Not attempted"),
        "{text}"
    );
    assert!(
        text.contains("0 of 2 items")
            && text.contains("1 failed")
            && text.contains("1 not attempted"),
        "{text}"
    );
}

#[test]
fn a_complete_run_has_no_not_attempted_section() {
    let m = execute(
        confirmed(vec![candidate("/p/a/node_modules", 100 * MB)]),
        &Recorder::default(),
    );

    assert!(!text(&m).contains("Not attempted"));
    assert!(!m.render().contains("Not attempted"));
}

/// The first line of the screen, which is the verdict.
fn headline(m: &Manifest) -> String {
    text(m).lines().next().unwrap_or_default().to_string()
}

#[test]
fn a_complete_run_opens_on_a_check_mark_and_the_word_safe() {
    let m = execute(
        confirmed(vec![candidate("/p/a/node_modules", 100 * MB)]),
        &Recorder::default(),
    );
    let line = headline(&m);

    assert!(line.contains('✓'), "no glyph:\n{line}");
    assert!(line.contains("SAFE"), "no word:\n{line}");
    assert!(line.contains("1 of 1 items"), "{line}");
    assert!(line.contains("100.00 MB"), "{line}");
    assert!(!line.contains("BLOCKED"), "{line}");
    assert!(text(&m).contains("100% of the plan"), "{}", text(&m));
}

#[test]
fn a_run_with_a_failure_opens_on_an_exclamation_mark_and_the_word_blocked() {
    let line = headline(&partial());

    assert!(line.contains('!'), "no glyph:\n{line}");
    assert!(line.contains("BLOCKED"), "no word:\n{line}");
    assert!(line.contains("1 of 2 items"), "{line}");
    assert!(line.contains("1 failed"), "{line}");
    assert!(!line.contains("SAFE") && !line.contains('✓'), "{line}");
}

#[test]
fn a_stopped_run_says_how_many_were_not_attempted_and_does_not_call_them_failures() {
    let line = headline(&stopped());

    assert!(line.contains('!') && line.contains("BLOCKED"), "{line}");
    assert!(line.contains("1 of 3 items"), "{line}");
    assert!(line.contains("2 not attempted"), "{line}");
    assert!(
        !line.contains("failed"),
        "a stop is not a failure, and the headline must not add them:\n{line}"
    );
}

#[test]
fn the_bar_counts_items_and_never_bytes() {
    // One of two items moved. By items that is 50%; by bytes it would be 33%,
    // and bytes are a prediction until the Trash is emptied.
    let screen = text(&partial());

    assert!(screen.contains("50% of the plan"), "{screen}");
    assert!(!screen.contains("33%"), "{screen}");
    assert!(
        screen.contains('█') && screen.contains('·'),
        "the bar shares the gauge's glyphs:\n{screen}"
    );
}

#[test]
fn the_time_the_run_took_is_measured_and_shown() {
    let nap = Duration::from_millis(150);
    let m = execute(
        confirmed(vec![candidate("/p/a/node_modules", 100 * MB)]),
        &Sleeper(nap),
    );

    assert!(m.elapsed >= nap, "measured {:?}, slept {nap:?}", m.elapsed);
    let shown = took(m.elapsed);
    assert_ne!(
        shown,
        took(Duration::ZERO),
        "a run that slept shows as instant"
    );
    assert!(
        headline(&m).contains(&format!("in {shown}")),
        "elapsed is not on the headline:\n{}",
        headline(&m)
    );
    assert!(
        m.render().contains(&shown),
        "the written record leaves out how long it took:\n{}",
        m.render()
    );
}

fn summary() -> RunSummary {
    RunSummary {
        runs: 7,
        // 2026-08-19, UTC.
        since: UNIX_EPOCH + Duration::from_secs(1_787_097_600),
        bytes_moved: 41 * 1024 * MB + 800 * MB,
        fastest: Some(Duration::from_millis(2100)),
        largest: 12 * 1024 * MB + 300 * MB,
        this_rank: Some(3),
    }
}

#[test]
fn all_runs_says_how_many_since_when_and_where_this_one_stands() {
    let m = partial();
    let mut report = Report::new();
    report.set_history(Ok(summary()));
    let area = Rect::new(0, 0, 110, 60);
    let mut buf = Buffer::empty(area);
    report.render(&m, Some(&record()), area, &mut buf);
    let screen = squashed_rows(&buf);

    assert!(screen.contains("All runs"), "{screen}");
    assert!(screen.contains("7 runs since 2026-08-19"), "{screen}");
    assert!(
        screen.contains(&format!(
            "{} moved to the Trash",
            human(summary().bytes_moved)
        )),
        "{screen}"
    );
    assert!(screen.contains("fastest 2.1 s"), "{screen}");
    assert!(
        screen.contains(&format!("largest {}", human(summary().largest))),
        "{screen}"
    );
    assert!(screen.contains("this run is the 3rd largest"), "{screen}");
}

fn squashed_rows(buf: &Buffer) -> String {
    (0..buf.area.height)
        .map(|y| {
            (0..buf.area.width)
                .map(|x| buf[(x, y)].symbol().to_string())
                .collect::<String>()
                .trim_end()
                .to_string()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn a_history_that_could_not_be_read_costs_the_section_and_nothing_else() {
    let m = partial();
    let mut report = Report::new();
    report.set_history(Err("unable to open database file".to_string()));
    let area = Rect::new(0, 0, 110, 60);
    let mut buf = Buffer::empty(area);
    report.render(&m, Some(&record()), area, &mut buf);
    let screen = squashed_rows(&buf);

    assert!(screen.contains("All runs"), "{screen}");
    assert!(screen.contains("unable to open database file"), "{screen}");
    assert!(screen.contains("BLOCKED"), "{screen}");
    assert!(screen.contains("Restore"), "{screen}");
    assert!(
        !screen.contains("fastest"),
        "numbers that were never read were drawn:\n{screen}"
    );
}

/// A run where every one of `n` items failed, so the screen has far more rows
/// than any terminal.
fn many_failures(n: usize) -> Manifest {
    let items = (0..n)
        .map(|i| candidate(&format!("/p/{i:03}/target"), MB))
        .collect();
    execute(
        confirmed(items),
        &Recorder {
            fail_on: Some("target"),
            ..Default::default()
        },
    )
}

fn window(report: &Report, m: &Manifest, rows: u16) -> String {
    let area = Rect::new(0, 0, 100, rows);
    let mut buf = Buffer::empty(area);
    report.render(m, Some(&record()), area, &mut buf);
    squashed_rows(&buf)
}

#[test]
fn a_run_with_more_failures_than_rows_scrolls_and_says_where_it_is() {
    let m = many_failures(43);
    let mut report = Report::new();

    let first = window(&report, &m, 20);
    assert!(first.contains("showing 1-"), "no position:\n{first}");
    assert!(
        first.contains("BLOCKED"),
        "the verdict scrolled away:\n{first}"
    );
    assert!(first.contains("/p/000/target"), "{first}");
    assert!(!first.contains("/p/042/target"), "{first}");

    report.scroll(Motion::Bottom);
    let last = window(&report, &m, 20);
    assert!(
        last.contains("/p/042/target"),
        "G did not reach the end:\n{last}"
    );
    assert!(
        last.contains("BLOCKED"),
        "the verdict scrolled away:\n{last}"
    );
    let flat = squashed(&last);
    assert!(
        flat.contains(&squashed(restore_steps(false).last().expect("steps"))),
        "the last line of the screen is out of reach:\n{last}"
    );

    report.scroll(Motion::Top);
    assert_eq!(window(&report, &m, 20), first, "g did not come back");
}

#[test]
fn scrolling_down_one_row_moves_the_window_by_one_row() {
    let m = many_failures(43);
    let mut report = Report::new();
    let _ = window(&report, &m, 20);
    report.scroll(Motion::Down);
    let after = window(&report, &m, 20);

    assert!(after.contains("showing 2-"), "{after}");
}

#[test]
fn a_run_that_fits_says_nothing_about_scrolling() {
    let m = execute(
        confirmed(vec![candidate("/p/a/node_modules", 100 * MB)]),
        &Recorder::default(),
    );

    assert!(!text(&m).contains("showing"), "{}", text(&m));
}
