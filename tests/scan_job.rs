//! The scan's thread: how it answers, and what it does with a panic.

use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use dev_cleaner::scan::Progress;
use dev_cleaner::tui::{Finished, ScanJob, Screens};

fn pending() -> Screens {
    Screens::pending(vec!["/r".into()], "/none".into())
}

fn answer(job: &ScanJob) -> Finished {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if let Some(done) = job.poll() {
            return done;
        }
        assert!(Instant::now() < deadline, "the scan never answered");
        std::thread::sleep(Duration::from_millis(1));
    }
}

#[test]
fn a_scan_that_finishes_hands_over_its_screens() {
    let job = ScanJob::spawn(|_| Some(pending())).expect("spawn");
    assert!(matches!(answer(&job), Finished::Done(_)));
}

#[test]
fn a_scan_that_was_cancelled_hands_over_nothing() {
    let job = ScanJob::spawn(|_| None).expect("spawn");
    assert!(matches!(answer(&job), Finished::Cancelled));
}

#[test]
fn polling_a_scan_that_is_still_running_returns_at_once() {
    let (release, hold) = std::sync::mpsc::channel::<()>();
    let job = ScanJob::spawn(move |_| {
        let _ = hold.recv();
        None
    })
    .expect("spawn");

    let began = Instant::now();
    assert!(job.poll().is_none());
    assert!(began.elapsed() < Duration::from_millis(200));
    release.send(()).expect("release");
    assert!(matches!(answer(&job), Finished::Cancelled));
}

#[test]
fn the_scan_counts_into_the_progress_the_job_hands_out() {
    let job = ScanJob::spawn(|progress: &Arc<Progress>| {
        progress.entries.store(42, Ordering::Relaxed);
        None
    })
    .expect("spawn");
    answer(&job);
    assert_eq!(job.progress().entries.load(Ordering::Relaxed), 42);
}

#[test]
fn cancelling_reaches_the_scan() {
    let job = ScanJob::spawn(|progress: &Arc<Progress>| {
        while !progress.is_cancelled() {
            std::thread::yield_now();
        }
        None
    })
    .expect("spawn");
    job.cancel();
    assert!(matches!(answer(&job), Finished::Cancelled));
}

#[test]
fn a_panic_in_the_scan_comes_back_as_a_panic_and_not_as_silence() {
    let job = ScanJob::spawn(|_| -> Option<Screens> { panic!("the walk blew up") }).expect("spawn");
    match answer(&job) {
        Finished::Panicked(payload) => {
            assert_eq!(payload.downcast_ref::<&str>(), Some(&"the walk blew up"));
        }
        _ => panic!("a panic was reported as something else"),
    }
}
