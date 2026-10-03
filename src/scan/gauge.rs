//! What a scan says about how far it has got, without saying more than it knows.
//!
//! A walk has no known total, so the only honest percentage comes from a
//! previous complete scan of the same roots: each top-level folder is weighed
//! by what it held last time and capped at that, so one folder that grew cannot
//! stand in for the progress of the others. Without such a baseline there is no
//! percentage at all, only counts. The phases after the walk know their
//! denominator, and those are exact.
//!
//! This is a pure function of counters, a baseline and a clock, so the rules
//! can be tested without a terminal, a filesystem or a wait.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::Duration;

/// What the last complete scan of the same roots held, by top-level folder.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Baseline {
    pub entries: u64,
    pub wall: Duration,
    /// Entries per top-level folder (and the root itself, for files directly in it).
    pub children: BTreeMap<PathBuf, u64>,
}

/// Where a scan is. Declared in the order they run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Phase {
    /// Walking the roots. No total is known.
    Discovering,
    /// Working out which directories are projects and which are build output.
    Indexing,
    /// Running the guards over every artifact directory.
    Classifying,
    /// Totalling each project.
    Measuring,
    /// Writing the finished scan down.
    Saving,
}

impl Phase {
    pub const fn name(self) -> &'static str {
        match self {
            Phase::Discovering => "discovering",
            Phase::Indexing => "indexing",
            Phase::Classifying => "classifying",
            Phase::Measuring => "measuring",
            Phase::Saving => "saving",
        }
    }

    pub(crate) const fn code(self) -> u8 {
        self as u8
    }

    pub(crate) fn from_code(code: u8) -> Self {
        match code {
            1 => Phase::Indexing,
            2 => Phase::Classifying,
            3 => Phase::Measuring,
            4 => Phase::Saving,
            _ => Phase::Discovering,
        }
    }
}

/// One top-level folder as the walk sees it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TopReading {
    pub path: PathBuf,
    pub entries: u64,
    /// A folder, as opposed to the bucket for files directly in a root.
    pub folder: bool,
    /// Every directory inside it has been read.
    pub done: bool,
}

/// A snapshot of the counters a walk keeps.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reading {
    pub entries: u64,
    pub bytes: u64,
    pub projects: u64,
    pub unreadable: u64,
    pub phase: Phase,
    pub phase_done: u64,
    pub phase_total: u64,
    /// `None` when the walk held the lock at that moment: the reader keeps what it had.
    pub tops: Option<Vec<TopReading>>,
    pub current: Option<PathBuf>,
}

/// How the bar is drawn.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Bar {
    /// Moving, but not measuring.
    Indeterminate,
    /// Against the last scan: a guess, drawn as one.
    Estimated(f64),
    /// Against a known total.
    Exact(f64),
}

/// Everything the progress block says, decided.
#[derive(Debug, Clone, PartialEq)]
pub struct Display {
    pub phase: Phase,
    pub bar: Bar,
    /// Set once the walk has passed what the last scan held.
    pub past_last: Option<u64>,
    /// A range, never a point.
    pub eta: Option<(Duration, Duration)>,
    pub rate: u64,
    /// How long nothing has moved, once that is worth saying.
    pub stalled: Option<Duration>,
    /// Top-level folders finished, and how many there are.
    pub folders: (usize, usize),
}

impl Display {
    /// The percentage the bar says, if it says one.
    pub fn percent(&self) -> Option<u8> {
        match self.bar {
            Bar::Indeterminate => None,
            Bar::Estimated(f) | Bar::Exact(f) => Some((f * 100.0).floor() as u8),
        }
    }
}

/// No bar claims more than this before the scan has finished.
pub const CAP: f64 = 0.95;
/// No range is offered before this much of the baseline has been seen.
pub const ETA_AFTER_FRACTION: f64 = 0.25;
/// Nor before this long has passed.
pub const ETA_AFTER: Duration = Duration::from_secs(10);
/// Nothing moving for this long is said out loud.
pub const STALL: Duration = Duration::from_secs(5);
/// The range around the straight-line estimate: the rate of a walk swings by
/// a factor of five or more across a tree, so a point would be a lie.
const ETA_LOW: f64 = 0.7;
const ETA_HIGH: f64 = 1.6;

/// The bar's memory: it only ever moves forward.
#[derive(Debug, Default)]
pub struct Gauge {
    shown: f64,
    phase: Option<Phase>,
    past_last: bool,
    tops: Vec<TopReading>,
    mark: Option<(u64, Phase, u64)>,
    moved_at: Duration,
}

impl Gauge {
    pub fn new() -> Self {
        Self::default()
    }

    /// Decide what to show for `reading` at `elapsed` into the scan.
    pub fn display(
        &mut self,
        reading: &Reading,
        baseline: Option<&Baseline>,
        elapsed: Duration,
    ) -> Display {
        if let Some(tops) = &reading.tops {
            self.tops.clone_from(tops);
        }
        if self.phase != Some(reading.phase) {
            self.phase = Some(reading.phase);
            self.shown = 0.0;
        }
        self.note_movement(reading, elapsed);

        let folders = (
            self.tops.iter().filter(|t| t.folder && t.done).count(),
            self.tops.iter().filter(|t| t.folder).count(),
        );
        let stalled = self.stalled(elapsed);
        let mut eta = None;
        let mut fraction = None;

        let bar = if reading.phase != Phase::Discovering {
            match reading.phase_total {
                0 => Bar::Indeterminate,
                total => {
                    let f = (reading.phase_done as f64 / total as f64).min(1.0);
                    self.shown = self.shown.max(f);
                    Bar::Exact(self.shown)
                }
            }
        } else {
            match baseline.filter(|b| b.entries > 0) {
                None => Bar::Indeterminate,
                Some(base) => {
                    if reading.entries > base.entries {
                        self.past_last = true;
                    }
                    if self.past_last {
                        Bar::Indeterminate
                    } else {
                        let f = self.weighted(base).min(CAP);
                        self.shown = self.shown.max(f);
                        fraction = Some(self.shown);
                        Bar::Estimated(self.shown)
                    }
                }
            }
        };

        if let Some(f) = fraction
            && f >= ETA_AFTER_FRACTION
            && elapsed >= ETA_AFTER
            && stalled.is_none()
        {
            let to_go = elapsed.as_secs_f64() * (1.0 - f) / f;
            eta = Some((
                Duration::from_secs_f64(to_go * ETA_LOW),
                Duration::from_secs_f64(to_go * ETA_HIGH),
            ));
        }

        Display {
            phase: reading.phase,
            bar,
            past_last: self.past_last.then(|| baseline.map_or(0, |b| b.entries)),
            eta,
            rate: if elapsed >= Duration::from_millis(500) {
                (reading.entries as f64 / elapsed.as_secs_f64()) as u64
            } else {
                0
            },
            stalled,
            folders,
        }
    }

    /// What share of the last scan's weight has been seen, folder by folder.
    ///
    /// Each folder counts for no more than it held last time, so a folder that
    /// grew adds nothing beyond its old share, and a folder that is new adds
    /// nothing at all: neither can be measured against a size it never had.
    fn weighted(&self, base: &Baseline) -> f64 {
        let total: u64 = base.children.values().sum();
        if total == 0 {
            return 0.0;
        }
        let seen: u64 = base
            .children
            .iter()
            .map(|(path, before)| {
                let now = self
                    .tops
                    .iter()
                    .find(|t| &t.path == path)
                    .map_or(0, |t| t.entries);
                now.min(*before)
            })
            .sum();
        seen as f64 / total as f64
    }

    /// Remember when something last moved.
    fn note_movement(&mut self, reading: &Reading, elapsed: Duration) {
        let mark = (reading.entries, reading.phase, reading.phase_done);
        if self.mark != Some(mark) {
            self.mark = Some(mark);
            self.moved_at = elapsed;
        }
    }

    fn stalled(&self, elapsed: Duration) -> Option<Duration> {
        let quiet = elapsed.saturating_sub(self.moved_at);
        (quiet >= STALL).then_some(quiet)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn top(path: &str, entries: u64, done: bool) -> TopReading {
        TopReading {
            path: PathBuf::from(path),
            entries,
            folder: true,
            done,
        }
    }

    fn walking(entries: u64, tops: Vec<TopReading>) -> Reading {
        Reading {
            entries,
            bytes: 0,
            projects: 0,
            unreadable: 0,
            phase: Phase::Discovering,
            phase_done: 0,
            phase_total: 0,
            tops: Some(tops),
            current: None,
        }
    }

    fn after(phase: Phase, done: u64, total: u64) -> Reading {
        Reading {
            phase,
            phase_done: done,
            phase_total: total,
            ..walking(1000, Vec::new())
        }
    }

    fn last_scan() -> Baseline {
        Baseline {
            entries: 1000,
            wall: Duration::from_secs(200),
            children: BTreeMap::from([(PathBuf::from("/r/a"), 600), (PathBuf::from("/r/b"), 400)]),
        }
    }

    const SEC: Duration = Duration::from_secs(1);

    #[test]
    fn without_a_baseline_there_is_no_percentage() {
        let mut gauge = Gauge::new();
        let shown = gauge.display(&walking(500, vec![top("/r/a", 500, false)]), None, 3 * SEC);
        assert_eq!(shown.bar, Bar::Indeterminate);
        assert_eq!(shown.percent(), None);
        assert_eq!(shown.eta, None, "no range without a fraction to base it on");
    }

    #[test]
    fn a_baseline_weighs_each_folder_by_what_it_held_last_time() {
        let mut gauge = Gauge::new();
        let tops = vec![top("/r/a", 300, false), top("/r/b", 0, false)];
        let shown = gauge.display(&walking(300, tops), Some(&last_scan()), 3 * SEC);
        assert_eq!(shown.bar, Bar::Estimated(0.3));
    }

    #[test]
    fn a_folder_that_outgrew_its_baseline_does_not_stand_in_for_the_others() {
        let mut gauge = Gauge::new();
        // /r/a held 600 before and now holds 700; /r/b has not started.
        let tops = vec![top("/r/a", 700, false), top("/r/b", 0, false)];
        let shown = gauge.display(&walking(700, tops), Some(&last_scan()), 3 * SEC);
        assert_eq!(
            shown.bar,
            Bar::Estimated(0.6),
            "capped at its own last size"
        );
    }

    #[test]
    fn the_estimate_never_claims_more_than_ninety_five_percent() {
        let mut gauge = Gauge::new();
        let tops = vec![top("/r/a", 600, true), top("/r/b", 400, false)];
        let shown = gauge.display(&walking(1000, tops), Some(&last_scan()), 3 * SEC);
        assert_eq!(shown.bar, Bar::Estimated(CAP));
        assert_eq!(shown.past_last, None, "equal to last time is not past it");
    }

    #[test]
    fn the_estimate_never_goes_backwards() {
        let mut gauge = Gauge::new();
        let base = last_scan();
        let a = vec![top("/r/a", 500, false), top("/r/b", 0, false)];
        let first = gauge.display(&walking(500, a), Some(&base), 3 * SEC);
        // The same walk, read again with fewer entries counted in the folder
        // table than before: a reader that lost the lock race, a reordered read.
        let b = vec![top("/r/a", 100, false), top("/r/b", 0, false)];
        let second = gauge.display(&walking(500, b), Some(&base), 4 * SEC);
        assert!(
            second.percent() >= first.percent(),
            "{first:?} then {second:?}"
        );
    }

    #[test]
    fn past_the_last_scan_the_bar_stops_pretending_and_says_so() {
        let mut gauge = Gauge::new();
        let base = last_scan();
        let tops = vec![top("/r/a", 900, false), top("/r/b", 600, false)];
        let shown = gauge.display(&walking(1500, tops), Some(&base), 30 * SEC);
        assert_eq!(shown.past_last, Some(1000));
        assert_eq!(shown.bar, Bar::Indeterminate, "never parked on a full bar");
        assert_eq!(shown.eta, None);
        // Sticky: a later reading cannot bring the bar back.
        let again = gauge.display(
            &walking(1500, vec![top("/r/a", 10, false)]),
            Some(&base),
            31 * SEC,
        );
        assert_eq!(again.bar, Bar::Indeterminate);
        assert_eq!(again.past_last, Some(1000));
    }

    #[test]
    fn a_baseline_of_nothing_is_no_baseline() {
        let mut gauge = Gauge::new();
        let shown = gauge.display(
            &walking(5, vec![top("/r/a", 5, false)]),
            Some(&Baseline::default()),
            SEC,
        );
        assert_eq!(shown.bar, Bar::Indeterminate);
    }

    #[test]
    fn a_range_waits_for_a_quarter_of_the_baseline_and_ten_seconds() {
        let base = last_scan();
        let tops = |a| vec![top("/r/a", a, false), top("/r/b", 0, false)];
        let eta = |entries: u64, secs: u64| {
            Gauge::new()
                .display(
                    &walking(entries, tops(entries)),
                    Some(&base),
                    Duration::from_secs(secs),
                )
                .eta
        };
        assert_eq!(eta(300, 9), None, "too soon in time");
        assert_eq!(eta(100, 30), None, "too little of the baseline");
        let (low, high) = eta(300, 30).expect("30% after 30 s");
        // 30 s at 30% is 70 s to go in a straight line.
        assert!(low < Duration::from_secs(70) && Duration::from_secs(70) < high);
        assert!(low > Duration::ZERO && high > low, "a range, not a point");
    }

    #[test]
    fn the_phases_after_the_walk_are_exact() {
        let mut gauge = Gauge::new();
        let shown = gauge.display(&after(Phase::Classifying, 18, 40), None, 60 * SEC);
        assert_eq!(shown.bar, Bar::Exact(0.45));
        assert_eq!(shown.percent(), Some(45));
        assert_eq!(shown.phase, Phase::Classifying);
    }

    #[test]
    fn a_phase_with_no_total_is_not_given_one() {
        let mut gauge = Gauge::new();
        let shown = gauge.display(&after(Phase::Indexing, 0, 0), None, 60 * SEC);
        assert_eq!(shown.bar, Bar::Indeterminate);
    }

    #[test]
    fn each_phase_starts_its_own_bar_from_nothing() {
        let mut gauge = Gauge::new();
        let end = gauge.display(&after(Phase::Classifying, 40, 40), None, 60 * SEC);
        assert_eq!(end.percent(), Some(100));
        let next = gauge.display(&after(Phase::Measuring, 10, 1000), None, 61 * SEC);
        assert_eq!(
            next.percent(),
            Some(1),
            "a new phase is not held up by the last one"
        );
    }

    #[test]
    fn rate_is_entries_over_elapsed() {
        let mut gauge = Gauge::new();
        let shown = gauge.display(&walking(7_400, Vec::new()), None, 2 * SEC);
        assert_eq!(shown.rate, 3_700);
    }

    #[test]
    fn nothing_moving_for_five_seconds_is_said_out_loud() {
        let mut gauge = Gauge::new();
        let r = walking(500, vec![top("/r/a", 500, false)]);
        assert_eq!(gauge.display(&r, None, 2 * SEC).stalled, None);
        assert_eq!(
            gauge.display(&r, None, 6 * SEC).stalled,
            None,
            "moved at 2 s, not 6"
        );
        let shown = gauge.display(&r, None, 8 * SEC);
        assert_eq!(shown.stalled, Some(6 * SEC), "last moved at 2 s");
        // Moving again clears it.
        let moved = walking(501, vec![top("/r/a", 501, false)]);
        assert_eq!(gauge.display(&moved, None, 9 * SEC).stalled, None);
    }

    #[test]
    fn folders_are_counted_done_over_all() {
        let mut gauge = Gauge::new();
        let tops = vec![
            top("/r/a", 5, true),
            top("/r/b", 1, false),
            top("/r/c", 0, false),
        ];
        let shown = gauge.display(&walking(6, tops), None, SEC);
        assert_eq!(shown.folders, (1, 3));
    }

    #[test]
    fn a_reading_that_lost_the_lock_keeps_the_folders_it_had() {
        let mut gauge = Gauge::new();
        let base = last_scan();
        let tops = vec![top("/r/a", 300, false), top("/r/b", 0, false)];
        gauge.display(&walking(300, tops), Some(&base), 3 * SEC);
        let blind = Reading {
            tops: None,
            ..walking(310, Vec::new())
        };
        let shown = gauge.display(&blind, Some(&base), 4 * SEC);
        assert_eq!(shown.folders, (0, 2));
        assert_eq!(shown.bar, Bar::Estimated(0.3));
    }
}
