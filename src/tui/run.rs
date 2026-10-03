//! The event loop: terminal keys becoming the moves the screens already have.
//!
//! The loop owns no navigation rules. It looks a key up in the binding table,
//! and hands the result to whichever screen owns that move — routing is the
//! router's (#33), selection is the candidates screen's (#36), confirmation is
//! the confirm screen's (#37). A key the current screen does not answer to does
//! nothing but say so; there is no fallback behaviour for the loop to invent.
//!
//! Dispatch is separated from the terminal for the same reason drawing was:
//! [`Tui::press`] is a function of a key and a screen, so the claim that no key
//! reaches a deletion can be driven over every key in CI, with no tty.

use std::collections::{BTreeMap, BTreeSet};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use ratatui::Frame;
use ratatui::buffer::Buffer;
use ratatui::crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::init::DefaultTerminal;
use ratatui::layout::Rect;

use super::data::{Screens, label_for};
use super::header::{self, Header};
use super::job::{Finished, ScanJob};
use super::logo;
use super::palette::Theme;
use super::projects::{FRAME, truncate};
use super::result::wrap;
use super::review;
use super::row::{put, section};
use super::running::Running;
use super::scan_view::{ScanView, Stage, span};
use super::{
    Action, App, Binding, Confirm, Effect, Filter, Key, KeyPress, Marking, Motion, PURGE,
    ProjectMarking, Report, Review, Screen, bindings_for, signals, terminal,
};
use crate::bytes::human;
use crate::purge::{Manifest, Remover, TrashRemover, free_bytes, manifest_dir, write_manifest};
use crate::safety::Plan;
use crate::scan::Progress;
use crate::store::{RunSummary, Store, summarize};

/// How often the loop wakes with nothing to read.
///
/// Short enough that a hold which stopped is noticed promptly, long enough that
/// an idle interface is not redrawing for the sake of it.
const TICK: Duration = Duration::from_millis(100);

/// The smallest terminal every screen is laid out for.
///
/// Below it on either axis the body is not drawn at all. A screen cut to fit
/// reads as a screen that ends there, and the one screen that removes anything
/// must not take a confirmation from a frame that could not show the plan.
const MIN_COLS: u16 = 80;
const MIN_ROWS: u16 = 24;

/// Rows of the body the projects table does not give to projects: its view bar
/// and the table's own frame.
const TABLE_FRAME: usize = FRAME as usize + 1;

/// How long a notice stays on its row once nothing newer replaces it.
///
/// On the clock rather than a count of frames: a resize storm or a slow
/// terminal redraws at its own pace, and a notice has to last the same on
/// every machine.
pub const NOTICE_TTL: Duration = Duration::from_secs(3);

/// What a key says while the purge is running: it was heard, and did nothing.
const RUNNING_NOTICE: &str = "A purge is running. Esc stops it after the item in flight.";

/// The key bar's place while the purge runs. One key does anything.
const RUNNING_KEYS: &str = "Esc  stop after the item in flight   No other key does anything.";

/// Between two entries of the key bar. [`footer`] builds with it and the bar is
/// coloured by splitting on it, so the two cannot drift.
const FOOTER_GAP: &str = "   ";

/// Between two parts of the way row, for the same reason.
pub(super) const WAY_SEPARATOR: &str = "   ·   ";

/// What a keypress asked the loop to do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    /// Whatever happened, happened inside a screen.
    Stay,
    /// Leave the interface.
    Quit,
    /// The hold completed. The only step in this enum that deletes anything,
    /// and [`Tui::press`] returns it from one screen and one key.
    Purge,
    /// The result is done with. The caller scans the same roots again and hands
    /// the answer to [`Tui::resume`]: the purge just made the old numbers false.
    Rescan,
}

/// What a notice is, which is what it is tinted by.
///
/// The words say the same: a tint is the second carrier, never the first.
#[derive(Debug, Clone, Copy)]
enum Tone {
    /// A change was made.
    Done,
    /// A key was refused, or a stop is being waited for.
    Refused,
    /// Neither: a fact about the last key.
    Plain,
}

/// One line under the body: what the last key did, or did not do.
#[derive(Debug)]
struct Notice {
    text: String,
    tone: Tone,
    at: Instant,
}

/// The interface, driven by keys.
#[derive(Debug)]
pub struct Tui {
    /// The router.
    ///
    /// Held in an `Option` because every move it makes consumes it — which is
    /// what stops a plan being confirmed and then quietly altered. It is
    /// emptied only for the length of [`Tui::transition`] and is never
    /// observably absent.
    app: Option<App>,
    screens: Screens,
    review: Review,
    confirm: Confirm,
    report: Report,
    /// Where the record of a run was written, once there is one.
    record: Option<PathBuf>,
    /// Whether the key list is over the screen.
    help: bool,
    /// Rows the last frame gave the body, so scrolling moves by what the user
    /// can actually see.
    rows: usize,
    /// Whether the last frame was below [`MIN_COLS`]×[`MIN_ROWS`], so the body
    /// was not drawn.
    too_small: bool,
    /// When the purge key last arrived.
    held_at: Option<Instant>,
    /// What the last key did, until the tick lets it go.
    notice: Option<Notice>,
    /// When `q` last asked to be pressed again, while it still has something to
    /// drop. Any other key takes it back, and so does the tick with the notice.
    quit_armed: Option<Instant>,
    /// Where the record of a run is written.
    manifest_dir: PathBuf,
    /// The purge, while it is under way. Checked before the screen, the way
    /// `help` is: the router knows a plan and a record and nothing between.
    running: Option<Running>,
    /// Why the run stopped before its last item, once it has.
    ended_early: Option<String>,
    /// What the interface draws in, chosen once at startup.
    theme: Theme,
    /// The scan running behind the interface, while there is one, and what it
    /// ended as if it did not finish.
    scan: Option<ScanView>,
}

impl Tui {
    pub fn new(screens: Screens) -> Self {
        Self {
            app: Some(App::new(Plan::draft())),
            screens,
            review: Review::new(),
            confirm: Confirm::new(),
            report: Report::new(),
            record: None,
            help: false,
            rows: 0,
            too_small: false,
            held_at: None,
            notice: None,
            quit_armed: None,
            manifest_dir: manifest_dir(),
            running: None,
            ended_early: None,
            theme: Theme::default(),
            scan: None,
        }
    }

    /// The interface at the first moment of a scan that has only begun.
    ///
    /// `screens` are [`Screens::pending`]: nothing on them is reachable until
    /// the scan finishes, and the body is the scan's own progress block.
    pub fn starting(screens: Screens, progress: Arc<Progress>, now: Instant) -> Self {
        let mut tui = Self::new(screens);
        tui.scan = Some(ScanView::new(progress, now, false));
        tui
    }

    /// Scan again behind the interface, hiding what the old scan showed: after
    /// a purge those numbers are known to be false.
    pub fn begin_scan(&mut self, progress: Arc<Progress>, now: Instant) {
        self.scan = Some(ScanView::new(progress, now, true));
        self.notice = None;
        self.quit_armed = None;
    }

    /// Whether a scan is running (or stopping) behind the interface.
    pub fn is_scanning(&self) -> bool {
        self.scan.as_ref().is_some_and(ScanView::running)
    }

    /// The scan ended because it was told to. Nothing it found is kept.
    pub fn scan_cancelled(&mut self, now: Instant) {
        if let Some(view) = &mut self.scan {
            view.cancelled();
            self.notify(
                "The scan was cancelled. Nothing was written.".to_string(),
                Tone::Plain,
                now,
            );
        }
    }

    /// The scan finished: show what it built.
    pub fn finish_scan(&mut self, screens: Screens, now: Instant) {
        let Some(view) = self.scan.take() else {
            return self.resume(screens, now);
        };
        let unreadable = view.unreadable();
        let stopping = view.stage == Stage::Stopping;
        let projects = screens.projects.rows().len();
        let elapsed = view.elapsed();
        if view.again {
            self.resume(screens, now);
        } else {
            self.adopt(screens);
            let s = if projects == 1 { "" } else { "s" };
            self.notify(
                format!("Scan finished: {projects} project{s} in {}.", span(elapsed)),
                Tone::Done,
                now,
            );
        }
        if stopping {
            // Esc was answered with "nothing is written"; past the point of
            // stopping the scan wrote itself down, and the notice says so.
            let said = self
                .notice
                .as_ref()
                .map(|n| n.text.clone())
                .unwrap_or_default();
            self.notify(
                format!("{said} It was already saving when stopped, so it finished and was kept."),
                Tone::Done,
                now,
            );
        }
        if unreadable > 0 {
            let s = if unreadable == 1 { "" } else { "s" };
            let said = self
                .notice
                .as_ref()
                .map(|n| n.text.clone())
                .unwrap_or_default();
            self.notify(
                format!("{said} {unreadable} unreadable folder{s}."),
                Tone::Done,
                now,
            );
        }
    }

    /// Draw in `theme` instead of the profile's own colours.
    pub fn with_theme(mut self, theme: Theme) -> Self {
        self.theme = theme;
        self
    }

    /// Write records under `dir` instead of the user's own state directory.
    pub fn with_manifest_dir(mut self, dir: PathBuf) -> Self {
        self.manifest_dir = dir;
        self
    }

    /// Where the record of the run was written, once there is one.
    pub fn record(&self) -> Option<&std::path::Path> {
        self.record.as_deref()
    }

    /// The router, for anything that only needs to look at it.
    pub fn app(&self) -> &App {
        self.app.as_ref().expect("the router is always present")
    }

    /// Handle one key.
    pub fn press(&mut self, key: KeyPress, now: Instant) -> Step {
        // Every key takes the arming back; only a `q` that finds it still live
        // can use it, so a key in between needs no bookkeeping of its own.
        let armed = self
            .quit_armed
            .take()
            .is_some_and(|at| now.duration_since(at) < NOTICE_TTL);
        if let Some(run) = &self.running {
            // Esc is the only key with a meaning here, and what it means is
            // between items: it stops the run, it does not leave the screen.
            // Every other key, `q` included: the terminal is not handed back
            // while a thread is moving files.
            let notice = if key == KeyPress::Esc {
                run.stop()
            } else {
                RUNNING_NOTICE.to_string()
            };
            self.notify(notice, Tone::Refused, now);
            return Step::Stay;
        }
        if self.scan.is_some() {
            return self.press_scanning(key, armed, now);
        }
        if self.help {
            // The overlay closes on any key and nothing underneath it moves,
            // which is what makes "?" safe to press while reading a plan. The
            // key it closed on went nowhere, and the row under the body says so.
            self.help = false;
            self.notify(
                format!("Keys closed. {key} was not applied."),
                Tone::Plain,
                now,
            );
            return Step::Stay;
        }

        let screen = self.app().screen();
        // The filter that shows what is marked reads the marks, and the marks
        // move between frames: look at them as they are now.
        if screen == Screen::Projects {
            self.refresh_marks();
        }
        let Some(action) = bindings_for(screen)
            .iter()
            .find(|b| b.key == key)
            .map(|b| b.action)
        else {
            // Silence reads as a keyboard that stopped working. The notice is
            // all this does: the hold is `held_at`'s and the tick's.
            self.notify(unbound(screen, key), Tone::Refused, now);
            return Step::Stay;
        };

        match action {
            Action::Quit => self.quit(armed, now),
            Action::Help => {
                self.help = true;
                Step::Stay
            }
            Action::Back => {
                self.transition(App::back);
                Step::Stay
            }
            Action::Forward => {
                self.forward(screen, now);
                Step::Stay
            }
            Action::Rescan => Step::Rescan,
            Action::Move(motion) => {
                let before = self.place(screen);
                self.move_within(screen, motion);
                if self.place(screen) == before {
                    self.notify(stayed(motion, before), Tone::Plain, now);
                }
                Step::Stay
            }
            Action::Candidate(key) => {
                let before = self.place(screen);
                let marking = self.screens.candidates.press(key, self.rows);
                if marking.is_none()
                    && let Some(motion) = motion_of(key)
                    && self.place(screen) == before
                {
                    self.notify(stayed(motion, before), Tone::Plain, now);
                }
                if let Some(marking) = marking {
                    let tone = match marking {
                        Marking::NothingToMark => Tone::Refused,
                        Marking::Cleared(0, _) => Tone::Plain,
                        _ => Tone::Done,
                    };
                    let text = self.describe(marking);
                    self.notify(text, tone, now);
                }
                Step::Stay
            }
            Action::MarkProject => {
                self.mark_project(now);
                Step::Stay
            }
            Action::Scope => {
                let candidates = &mut self.screens.candidates;
                if !candidates.toggle_scope() {
                    self.notify(
                        "Every project is showing already.".to_string(),
                        Tone::Plain,
                        now,
                    );
                    return Step::Stay;
                }
                let (count, bytes) = self.captured(Screen::Candidates);
                let shown = match self.screens.candidates.scope_name() {
                    Some(name) => format!("Showing {name} only."),
                    None => "Showing every project.".to_string(),
                };
                self.notify(
                    format!("{shown} {count} marked, {}.", human(bytes)),
                    Tone::Done,
                    now,
                );
                Step::Stay
            }
            Action::Filter => {
                let table = &mut self.screens.projects;
                table.cycle_filter();
                let (kept, total) = (table.shown().len(), table.rows().len());
                let filter = table.filter();
                let mut text = match filter {
                    Filter::All => format!("Showing all projects: {total}."),
                    f => format!("Showing {}: {kept} of {total} projects.", f.words()),
                };
                if kept == 0 {
                    text.push_str(" r shows every project.");
                }
                self.notify(text, Tone::Done, now);
                Step::Stay
            }
            Action::Reset => {
                let text = if screen == Screen::Projects {
                    self.screens.projects.reset_view();
                    format!(
                        "View reset: sort {}, all projects.",
                        self.screens.projects.ordering()
                    )
                } else {
                    let candidates = &mut self.screens.candidates;
                    candidates.reset_view();
                    let scope = candidates
                        .scope_name()
                        .map_or("all projects", |_| "this project");
                    format!("View reset: sort {}, {scope}.", candidates.ordering())
                };
                self.notify(text, Tone::Done, now);
                Step::Stay
            }
            Action::Sort(column) => {
                let table = &mut self.screens.projects;
                table.sort_by(column);
                let mut text = format!("Sorted by {}.", table.ordering());
                if let Some(row) = table.selected() {
                    text.push_str(&format!("  Cursor on {}.", table.label(row)));
                }
                self.notify(text, Tone::Done, now);
                Step::Stay
            }
            // The confirm screen exists to show what is about to be deleted. A
            // hold on a frame that could not show the plan is a blind one, so
            // the key is refused, and the paragraph in the body's place says so.
            Action::Purge if self.too_small => Step::Stay,
            Action::Purge => self.hold(now),
        }
    }

    /// A key while a scan runs, or after one was cancelled.
    ///
    /// The screens are not reachable: nothing was finished measuring, and a
    /// candidate offered from a half-read tree is a deletion decided on a guess.
    /// So no key reaches the router. Three keys do anything, and every other
    /// says why it did nothing.
    fn press_scanning(&mut self, key: KeyPress, armed: bool, now: Instant) -> Step {
        let Some(view) = self.scan.as_mut() else {
            return Step::Stay;
        };
        let cancelled = matches!(view.stage, Stage::Cancelled { .. });
        match key {
            KeyPress::Esc | KeyPress::Char('c') if view.stage == Stage::Running => {
                view.stop();
                self.notify(
                    "Stopping the scan: it ends at the next entry. Nothing is written.".to_string(),
                    Tone::Refused,
                    now,
                );
                Step::Stay
            }
            KeyPress::Char('R') if cancelled => Step::Rescan,
            KeyPress::Char('q') if cancelled || armed => Step::Quit,
            KeyPress::Char('q') => {
                let dropping = view.dropping();
                self.quit_armed = Some(now);
                self.notify(
                    format!(
                        "The scan ({dropping}) would be dropped; nothing is written. \
                         q again within {} s quits.",
                        NOTICE_TTL.as_secs()
                    ),
                    Tone::Refused,
                    now,
                );
                Step::Stay
            }
            _ => {
                let said = if cancelled {
                    "The scan was cancelled. R scans again; q quits."
                } else if view.stage == Stage::Stopping {
                    "The scan is stopping. q quits."
                } else {
                    "The scan is running. Esc stops it; q quits."
                };
                self.notify(said.to_string(), Tone::Refused, now);
                Step::Stay
            }
        }
    }

    /// Where the list on `screen` stands: the row the cursor or the window is
    /// on, how many rows there are, and how many fit when the list scrolls
    /// rather than moves a cursor. What a motion that stayed put is told by.
    fn place(&self, screen: Screen) -> Place {
        match screen {
            Screen::Projects => Place {
                at: self.screens.projects.cursor(),
                len: self.screens.projects.shown().len(),
                window: None,
            },
            Screen::Candidates => Place {
                at: self.screens.candidates.cursor(),
                len: self.screens.candidates.visible().len(),
                window: None,
            },
            Screen::Review => {
                let locator = self.screens.candidates.locator();
                let len = self
                    .app
                    .as_ref()
                    .and_then(App::reviewing)
                    .map_or(0, |plan| self.review.lines(plan, locator));
                let window = self.rows.saturating_sub(review::CHROME);
                Place {
                    at: self.review.offset(len, window),
                    len,
                    window: Some(window),
                }
            }
            Screen::Result => {
                let (at, len, window) = self.report.place();
                Place {
                    at,
                    len,
                    window: Some(window),
                }
            }
            _ => Place {
                at: 0,
                len: 0,
                window: None,
            },
        }
    }

    /// Land on the dashboard of a scan taken after a run.
    ///
    /// Everything the run left behind goes: the marks and the plan with the old
    /// screens, the record's path and the verdict with the result. The record is
    /// already on disk, so nothing is lost, and the notice says what happened.
    pub fn resume(&mut self, screens: Screens, now: Instant) {
        let projects = screens.projects.rows().len();
        self.adopt(screens);
        let s = if projects == 1 { "" } else { "s" };
        self.notify(
            format!("Back at the dashboard. Rescanned {projects} project{s}."),
            Tone::Done,
            now,
        );
    }

    /// Take `screens` as the current ones and leave everything of the old ones behind.
    fn adopt(&mut self, screens: Screens) {
        self.screens = screens;
        self.review = Review::new();
        self.report = Report::new();
        self.record = None;
        self.ended_early = None;
        self.help = false;
        self.scan = None;
        self.notice = None;
        self.quit_armed = None;
        self.arrive(App::new(Plan::draft()));
    }

    /// Leave, or say what leaving would drop and wait for a second `q`.
    ///
    /// The record of a finished run is already on disk, and with nothing marked
    /// or planned there is nothing to lose: both quit at once.
    fn quit(&mut self, armed: bool, now: Instant) -> Step {
        let screen = self.app().screen();
        let (count, bytes) = match screen {
            Screen::Result => return Step::Quit,
            Screen::Review | Screen::Confirm => self.captured(screen),
            _ => self.captured(Screen::Candidates),
        };
        if armed || count == 0 {
            return Step::Quit;
        }
        let what = match screen {
            Screen::Review | Screen::Confirm => {
                let s = if count == 1 { "" } else { "s" };
                format!("The plan of {count} item{s}")
            }
            _ => format!("{count} marked"),
        };
        self.quit_armed = Some(now);
        self.notify(
            format!(
                "{what} ({}) would be dropped. q again within {} s quits.",
                human(bytes),
                NOTICE_TTL.as_secs()
            ),
            Tone::Refused,
            now,
        );
        Step::Stay
    }

    /// A mark key's effect in words, with the count and total the wayfinding
    /// row carries: both come from [`Tui::captured`], so they cannot disagree.
    fn describe(&self, marking: Marking) -> String {
        let (count, bytes) = self.captured(Screen::Candidates);
        let now = format!("{count} marked, {}.", human(bytes));
        let entry = |verb: &str, path: &Path, size: u64| {
            format!("{verb}  {}  ({}).  {now}", label_for(path), human(size))
        };
        // In a project, a key acts on that project alone; what stays marked
        // elsewhere is said, so the count is never a surprise.
        let scope = self.screens.candidates.scope_name();
        let elsewhere = |others: usize| match others {
            0 => String::new(),
            n => format!(" {n} more marked elsewhere."),
        };
        match (marking, scope) {
            (Marking::Marked(path, size), _) => entry("Marked +", &path, size),
            (Marking::Unmarked(path, size), _) => entry("Unmarked", &path, size),
            (Marking::MarkedAll(n, size), None) => format!("Marked all {n}  ({}).", human(size)),
            (Marking::MarkedAll(n, size), Some(name)) => format!(
                "Marked all {n} in {name}  ({}).  {count} marked in total ({}).",
                human(size),
                human(bytes)
            ),
            (Marking::Cleared(0, _), Some(name)) if count > 0 => {
                format!("Nothing marked in {name}.{}", elsewhere(count))
            }
            (Marking::Cleared(0, _), _) => "Nothing marked.".to_string(),
            (Marking::Cleared(n, size), scope) => {
                let s = if n == 1 { "" } else { "s" };
                let place = scope.map(|name| format!(" in {name}")).unwrap_or_default();
                let others = if scope.is_some() {
                    elsewhere(count)
                } else {
                    String::new()
                };
                format!(
                    "Cleared {n} mark{s}{place}  ({}).{others}  c again restores them.",
                    human(size)
                )
            }
            (Marking::Restored(n, size), scope) => {
                let s = if n == 1 { "" } else { "s" };
                let place = scope.map(|name| format!(" in {name}")).unwrap_or_default();
                let others = if scope.is_some() {
                    elsewhere(count.saturating_sub(n))
                } else {
                    String::new()
                };
                format!("Restored {n} mark{s}{place}  ({}).{others}", human(size))
            }
            (Marking::NothingToMark, _) => "Nothing to mark.".to_string(),
        }
    }

    /// Say what the last key did, on the row under the body.
    ///
    /// A newer notice replaces an older one and its time starts again: the row
    /// is for the last key, not for a queue of them.
    fn notify(&mut self, text: String, tone: Tone, now: Instant) {
        self.notice = Some(Notice {
            text,
            tone,
            at: now,
        });
    }

    /// Notice a hold that stopped, and let a notice go once its time is up.
    ///
    /// Called every time round the loop, including the times nothing was read,
    /// because a key going quiet is exactly the event a terminal does not send.
    pub fn tick(&mut self, now: Instant) {
        if let Some(view) = &mut self.scan {
            view.sample(now);
        }
        if self
            .held_at
            .is_some_and(|last| now.duration_since(last) > Confirm::GRACE)
        {
            self.held_at = None;
            self.confirm.release();
        }
        if self
            .notice
            .as_ref()
            .is_some_and(|notice| now.duration_since(notice.at) >= NOTICE_TTL)
        {
            self.notice = None;
        }
        if self
            .quit_armed
            .is_some_and(|at| now.duration_since(at) >= NOTICE_TTL)
        {
            self.quit_armed = None;
        }
        if self.running.as_mut().is_some_and(|run| {
            run.advance(now);
            run.is_over()
        }) {
            self.finish_run();
        }
    }

    /// Account for another repeat of the purge key.
    fn hold(&mut self, now: Instant) -> Step {
        // The first press contributes nothing: there is no earlier event to
        // measure from, and a hold has to be a stretch of time rather than a
        // keystroke. `Confirm::hold` treats a gap past its grace as a new hold,
        // so an event arriving after a stall the tick never saw arms nothing.
        let delta = self
            .held_at
            .map(|last| now.duration_since(last))
            .unwrap_or_default();
        self.held_at = Some(now);
        if self.confirm.hold(delta) {
            Step::Purge
        } else {
            Step::Stay
        }
    }

    /// Advance one screen.
    ///
    /// Leaving the candidates screen is the one move that is not simply the
    /// router's: the plan is built from what is marked, at the moment of
    /// leaving. Adding marks to the draft as they are made would double every
    /// path on a second visit, because going back from review amends the plan
    /// rather than emptying it.
    fn forward(&mut self, screen: Screen, now: Instant) {
        if screen == Screen::Dashboard {
            self.transition(App::forward);
            // Arriving puts what can be removed first and hides nothing,
            // whatever view the table was left in.
            self.screens.projects.reset_view();
            // The insight Enter follows says which project the hint is about;
            // the cursor goes there, and the order of the table stays.
            if let Some(target) = self.screens.dashboard.lead() {
                self.screens.projects.focus(&target.project);
            }
            return;
        }
        if screen == Screen::Projects {
            // A filter that hides every project leaves none to open; leaving
            // would land on whatever scope the candidates screen last had.
            // (A scan that found no project has no filter to blame, and its
            // way on is the candidates screen, as it always was.)
            if self.screens.projects.selected().is_none()
                && !self.screens.projects.rows().is_empty()
            {
                self.notify(
                    "There is no project to open in this view. r shows every project.".to_string(),
                    Tone::Refused,
                    now,
                );
                return;
            }
            self.transition(App::forward);
            self.land_on_project(now);
            return;
        }
        if screen != Screen::Candidates {
            self.transition(App::forward);
            return;
        }
        let mut draft = Plan::draft();
        for candidate in self.screens.candidates.marked() {
            // `add` refuses anything not provably recoverable. The cursor can
            // only reach selectable entries in the first place, so a refusal
            // here would be a disagreement between the screen and the plan
            // rather than a user error — and the plan wins.
            let _ = draft.add(candidate.clone());
        }
        // Walked rather than short-circuited: the plan reaches review through
        // the router's own `review()`, which is what gives it a phrase.
        self.arrive(App::new(draft).forward().forward().forward());
        self.review = Review::new();
    }

    /// The project under the table's cursor: its root, its name, and the roots
    /// of the projects inside it.
    fn project_under_cursor(&self) -> Option<(PathBuf, String, Vec<PathBuf>)> {
        let table = &self.screens.projects;
        let row = table.selected()?;
        Some((
            row.path.clone(),
            table.label(row).to_string(),
            table.inner_roots(&row.path),
        ))
    }

    /// Where the offerable entries are when the project at `root` has none, in
    /// words, for a notice: the totals are the candidates screen's own, which is
    /// the dashboard's.
    fn where_something_is(&self, root: &Path) -> String {
        let table = &self.screens.projects;
        let mut by: BTreeMap<&Path, u64> = BTreeMap::new();
        let mut total = 0;
        for c in self.screens.candidates.selectable() {
            total += c.bytes;
            if let Some(owner) = table.owner_of(&c.path).filter(|o| *o != root) {
                *by.entry(owner).or_default() += c.bytes;
            }
        }
        let Some((top, top_bytes)) = by
            .iter()
            .max_by_key(|(path, bytes)| (**bytes, std::cmp::Reverse(**path)))
        else {
            return " Nothing is offered anywhere else.".to_string();
        };
        let named = table
            .rows()
            .iter()
            .find(|r| r.path == *top)
            .map_or_else(|| top.display().to_string(), |r| table.label(r).to_string());
        match by.len() {
            1 => format!(
                " 1 project has something: {named}, {}. Tab shows all.",
                human(*top_bytes)
            ),
            n => format!(
                " {n} projects have something, {} in all; most in {named}, {}. Tab shows all.",
                human(total),
                human(*top_bytes)
            ),
        }
    }

    /// Where something can be rebuilt when the candidates screen has nothing to
    /// list, for its body: up to three projects, largest first, named as the
    /// table names them.
    fn elsewhere(&self) -> Vec<String> {
        let candidates = &self.screens.candidates;
        if candidates.scope_name().is_none() || !candidates.visible().is_empty() {
            return Vec::new();
        }
        let table = &self.screens.projects;
        let mut by: BTreeMap<&Path, u64> = BTreeMap::new();
        for c in candidates.selectable() {
            if let Some(owner) = table.owner_of(&c.path) {
                *by.entry(owner).or_default() += c.bytes;
            }
        }
        if by.is_empty() {
            return vec!["Nothing is offered in any other project.".to_string()];
        }
        let mut ranked: Vec<(&Path, u64)> = by.into_iter().collect();
        ranked.sort_by_key(|(path, bytes)| (std::cmp::Reverse(*bytes), *path));
        let n = ranked.len();
        let (noun, have) = if n == 1 {
            ("project", "has")
        } else {
            ("projects", "have")
        };
        let mut lines = vec![format!(
            "{n} {noun} {have} something to rebuild, largest first:"
        )];
        for (path, bytes) in ranked.iter().take(3) {
            let named = table.rows().iter().find(|r| r.path == *path).map_or_else(
                || path.display().to_string(),
                |r| table.label(r).to_string(),
            );
            lines.push(format!("  {named}  {}", human(*bytes)));
        }
        if n > 3 {
            lines.push(format!("  … and {} more", n - 3));
        }
        lines
    }

    /// What a project holds back, in words, for a notice.
    fn held_back(&self, root: &Path, inner: &[PathBuf]) -> String {
        self.screens
            .candidates
            .held_back(root, inner)
            .iter()
            .map(|(reason, n)| format!("{n} held back — {reason}"))
            .collect::<Vec<_>>()
            .join("  ")
    }

    /// Open the candidates on the project the table's cursor was on, and say
    /// what is there.
    ///
    /// The router carries the plan and nothing else, so the project is not
    /// carried through it: the screen is told which project to show, and shows
    /// the entries under its path. The marks are not the project's: they stay
    /// one set, keyed by path, whichever project is open.
    fn land_on_project(&mut self, now: Instant) {
        let Some((root, name, inner)) = self.project_under_cursor() else {
            return;
        };
        self.screens
            .candidates
            .scope_to(&root, &name, inner.clone());
        let offered: Vec<u64> = self
            .screens
            .candidates
            .visible()
            .iter()
            .map(|c| c.bytes)
            .collect();
        let (text, tone) = if offered.is_empty() {
            let held = self.held_back(&root, &inner);
            let mut text = format!("{name}: nothing can be rebuilt here.");
            text.push_str(&self.where_something_is(&root));
            if !held.is_empty() {
                text.push(' ');
                text.push_str(&held);
            }
            (text, Tone::Refused)
        } else {
            let n = offered.len();
            let noun = if n == 1 { "directory" } else { "directories" };
            let text = format!(
                "{name}: {n} {noun} can be rebuilt, {}. The cursor is on the first.",
                human(offered.iter().sum())
            );
            (text, Tone::Done)
        };
        self.notify(text, tone, now);
    }

    /// `Space` on a project of the table: mark everything it offers, or unmark
    /// it when all of it is marked, and say the total across every project.
    fn mark_project(&mut self, now: Instant) {
        let Some((root, name, inner)) = self.project_under_cursor() else {
            self.notify("No project to mark.".to_string(), Tone::Refused, now);
            return;
        };
        let outcome = self.screens.candidates.toggle_project(&root, &inner);
        self.refresh_marks();
        let (count, bytes) = self.captured(Screen::Candidates);
        let total = format!("{count} marked in total ({}).", human(bytes));
        let entries = |n: usize| if n == 1 { "entry" } else { "entries" };
        let (text, tone) = match outcome {
            ProjectMarking::Marked(n, size) => (
                format!(
                    "Marked {n} {} in {name} ({}). {total}",
                    entries(n),
                    human(size)
                ),
                Tone::Done,
            ),
            ProjectMarking::Unmarked(n, size) => (
                format!(
                    "Unmarked {n} {} in {name} ({}). {total}",
                    entries(n),
                    human(size)
                ),
                Tone::Done,
            ),
            ProjectMarking::NothingOffered => {
                let held = self.held_back(&root, &inner);
                let said = format!("{name}: nothing can be rebuilt here, so nothing was marked.");
                (
                    if held.is_empty() {
                        said
                    } else {
                        format!("{said} {held}")
                    },
                    Tone::Refused,
                )
            }
        };
        self.notify(text, tone, now);
    }

    /// Tell the table what is marked in the projects it is about to draw.
    fn refresh_marks(&mut self) {
        let table = &self.screens.projects;
        let candidates = &self.screens.candidates;
        // The marked filter decides which rows there are from the marks, so it
        // needs every project's; any other only draws the window.
        let rows = if table.filter() == Filter::Marked {
            table.rows().iter().collect()
        } else {
            table.visible(self.rows.saturating_sub(TABLE_FRAME))
        };
        let marks = rows
            .iter()
            .map(|r| {
                (
                    r.path.clone(),
                    candidates.tally(&r.path, &table.inner_roots(&r.path)),
                )
            })
            .collect();
        self.screens.projects.set_marks(marks);
    }

    /// The marks across every project, for the table's way row: how many, how
    /// many bytes, and in how many projects.
    fn marked_in_projects(&self) -> Option<String> {
        let marked = self.screens.candidates.marked();
        if marked.is_empty() {
            return None;
        }
        let table = &self.screens.projects;
        let owners: BTreeSet<&Path> = marked
            .iter()
            .filter_map(|c| table.owner_of(&c.path))
            .collect();
        let s = if owners.len() == 1 { "" } else { "s" };
        Some(format!(
            "{} marked ({}) in {} project{s}",
            marked.len(),
            human(marked.iter().map(|c| c.bytes).sum()),
            owners.len()
        ))
    }

    fn move_within(&mut self, screen: Screen, motion: Motion) {
        match screen {
            Screen::Projects => {
                // A page is what the user sees: the body less the view bar, the
                // header row, the selected project and the position line.
                let rows = self.rows.saturating_sub(TABLE_FRAME);
                let table = &mut self.screens.projects;
                match motion {
                    Motion::Up => table.up(),
                    Motion::Down => table.down(),
                    Motion::Top => table.top(),
                    Motion::Bottom => table.bottom(),
                    Motion::PageUp => table.page_up(rows),
                    Motion::PageDown => table.page_down(rows),
                }
            }
            Screen::Review => {
                if let Some(plan) = self.app.as_ref().and_then(App::reviewing) {
                    // The list's own rows, not the body's: scrolling by the
                    // body left the last few rows of a plan out of reach.
                    let rows = self.rows.saturating_sub(review::CHROME);
                    self.review
                        .scroll(motion, plan, self.screens.candidates.locator(), rows);
                }
            }
            Screen::Result => self.report.scroll(motion),
            _ => {}
        }
    }

    /// Apply a move that consumes the router.
    fn transition(&mut self, move_to: impl FnOnce(App) -> App) {
        let app = self.app.take().expect("the router is always present");
        self.arrive(move_to(app));
    }

    /// Put the router on a screen.
    ///
    /// Every route onto one passes through here, and every one of them empties
    /// the gauge and forgets the clock behind it. A hold is a fact about the
    /// confirm screen; kept beside the router it outlived the screen, and a
    /// hold at 1.4 s of its 1.5 s survived Esc, Enter and one more tap (#78).
    ///
    /// ponytail: two fields reset in one place rather than carried inside
    /// `Stage::Confirm`. Move them into the stage if a second piece of
    /// per-screen state ever turns up here.
    fn arrive(&mut self, app: App) {
        self.app = Some(app);
        // Marks belong to one visit, and so does what `c` would restore.
        self.screens.candidates.forget_cleared();
        self.confirm = Confirm::new();
        self.held_at = None;
    }

    /// Carry out the plan.
    ///
    /// Reached only from [`Step::Purge`], which `press` returns only from the
    /// confirm screen with the hold complete. Everything after that is the same
    /// code `purge --execute` runs: the phrase comes from the plan, the plan
    /// confirms itself, and `execute` is what produces a record.
    pub fn purge(&mut self, remover: Box<dyn Remover + Send>) {
        let app = self.app.take().expect("the router is always present");
        let Some(phrase) = app.phrase() else {
            self.arrive(app);
            self.confirm.refuse();
            return;
        };
        let plan = match app.confirm(&phrase) {
            Ok(plan) => plan,
            // The phrase describes this exact plan, so a refusal means the two
            // disagree. The app comes back rather than the plan being lost,
            // and the screen says so rather than sitting on a full gauge.
            Err(app) => {
                // `arrive` first: it empties the gauge, and the notice goes on
                // the emptied one.
                self.arrive(app);
                self.confirm.refuse();
                return;
            }
        };

        let measure_at = self
            .screens
            .roots
            .first()
            .cloned()
            .unwrap_or_else(|| PathBuf::from("/"));
        let before = free_bytes(&measure_at);

        // `confirm` consumed the router along with the plan it held. A fresh one
        // stands in while the run is under way, which is also what empties the
        // gauge and the clock behind it (#78); the renderer checks `running`
        // first, and `finish_run` moves it on to the result.
        self.arrive(App::new(Plan::draft()));
        self.running = Some(Running::spawn(
            plan,
            remover,
            self.manifest_dir.clone(),
            before,
            measure_at,
            Instant::now(),
        ));
    }

    /// The thread has stopped: measure the disk, write the record as it now
    /// stands, and move to the result.
    ///
    /// The last write is the file the thread has been rewriting after every
    /// item, now with the free-space measurement in it.
    fn finish_run(&mut self) {
        let Some(run) = self.running.take() else {
            return;
        };
        let (before, measure_at) = (run.before, run.measure_at.clone());
        let (mut manifest, ended_early) = run.finish();
        if let (Some(before), Some(after)) = (before, free_bytes(&measure_at)) {
            manifest.record_actual(after.saturating_sub(before));
        }
        self.record = write_manifest(&manifest, &self.manifest_dir).ok();
        self.report.set_history(self.remember(&manifest));
        self.ended_early = ended_early;
        // `finished` takes the manifest, which only `execute_with` produces, or
        // the record rebuilt from what it reported before it stopped.
        self.arrive(App::new(Plan::draft()).finished(manifest));
    }

    /// Put the run in the store, and read back what the store holds of every
    /// run, this one included.
    ///
    /// A store that cannot be opened or read costs the answer and nothing else:
    /// the record is already on disk, and the screen says what is missing.
    fn remember(&self, manifest: &Manifest) -> Result<RunSummary, String> {
        let store = Store::open(&self.screens.db).map_err(|e| e.to_string())?;
        let this = store
            .record_purge(manifest, self.record.as_deref())
            .map_err(|e| format!("this run was not stored ({e})"))?;
        let runs = store.purge_runs().map_err(|e| e.to_string())?;
        summarize(&runs, Some(this)).ok_or_else(|| "no run is on record".to_string())
    }

    fn draw(&mut self, frame: &mut Frame) {
        let area = frame.area();
        self.render(area, frame.buffer_mut());
    }

    /// Paint the whole interface into `buf`, with no terminal behind it.
    ///
    /// Two rows of title above the body, and two below it: the notice row,
    /// then the key bar. On a terminal of [`logo::MIN_COLS`]×[`logo::MIN_ROWS`]
    /// or more the header grows to the band of [`logo::TOP`] rows, which
    /// [`render_header`] draws. Under it the header is the two rows it was
    /// before there was an icon, and every screen keeps the room it has at
    /// [`MIN_COLS`]×[`MIN_ROWS`].
    pub fn render(&mut self, area: Rect, buf: &mut Buffer) {
        let theme = self.theme;
        let theme = &theme;
        let tall = area.width >= logo::MIN_COLS && area.height >= logo::MIN_ROWS;
        let header = if tall { logo::TOP } else { 2 };
        let body = Rect {
            x: area.x,
            y: area.y.saturating_add(header),
            width: area.width,
            height: area.height.saturating_sub(header + 2),
        };
        self.rows = body.height as usize;
        if self.app().screen() == Screen::Projects {
            self.refresh_marks();
        }
        if self.app().screen() == Screen::Candidates {
            let lines = self.elsewhere();
            self.screens.candidates.set_elsewhere(lines);
        }
        let too_small = area.width < MIN_COLS || area.height < MIN_ROWS;
        self.too_small = too_small;
        // A pane being dragged passes through no rows on the way to its size,
        // and the buffer panics on a row it does not have.
        if area.is_empty() {
            return;
        }
        // Under everything else, so a look that owns its background owns every
        // cell of it, and every style below only has to say what ink it wants.
        buf.set_style(area, theme.ground);

        let screen = self.app().screen();
        let help = self.help;
        let running = self.running.is_some();

        // The band screens carry their danger in a bar of their own, and the
        // running screen is the confirm screen still.
        let band = screen == Screen::Confirm || running;
        let way = (area.height > 1).then(|| {
            if running {
                "no way back   ·   files go to the Trash   ·   the record is written as items move"
                    .to_string()
            } else if let Some(view) = &self.scan {
                view.way().to_string()
            } else {
                wayfinding(screen, self.captured(screen))
            }
        });
        let name = if running { "Purging" } else { screen.name() };
        let (place, context) = self.context(screen);
        header::render(
            theme,
            buf,
            area,
            tall,
            &Header {
                name,
                stage: if running { Screen::Confirm } else { screen },
                way: way.as_deref(),
                danger: band,
                place: place.as_deref(),
                context,
            },
        );
        // A body with no rows draws nothing, rather than its first line over
        // the row below it.
        if body.height > 0 {
            if let Some(run) = &self.running {
                // Drawn at any size: it takes no confirmation, and the way out
                // that the too-small paragraph offers is refused while it runs.
                run.render(theme, body, buf);
            } else if let Some(view) = &self.scan {
                // The scan's own progress, at any size it fits; below the
                // minimum the paragraph says so and says the scan goes on.
                if too_small {
                    render_too_small(theme, screen, area, body, buf, true);
                } else {
                    view.render(theme, body, buf);
                }
            } else if help {
                // Over the screen rather than part of it: the list already says
                // when it ran out of room, and any key closes it onto whatever
                // is beneath, the notice included.
                render_keys(theme, screen, body, buf);
            } else if too_small {
                render_too_small(theme, screen, area, body, buf, false);
            } else {
                self.render_screen(theme, screen, body, buf);
            }
        }
        // A fact, so never `MUTED`. Where the area has no row of its own for
        // it, the notice is left out rather than drawn over the way or the
        // title.
        if let Some(notice) = &self.notice
            && area.height >= 4
        {
            // Cut with a mark, as the wayfinding row is: the count and total sit
            // at the end of a mark notice and a narrow terminal loses them first.
            let line = truncate(&notice.text, area.width.saturating_sub(2) as usize);
            let style = match notice.tone {
                Tone::Done => theme.safe,
                Tone::Refused => theme.blocked,
                Tone::Plain => theme.text,
            };
            buf.set_string(area.x + 1, area.bottom() - 2, line, style);
        }
        if running {
            let (key, rest) = RUNNING_KEYS.split_once("  ").unwrap_or((RUNNING_KEYS, ""));
            let (label, fact) = rest.split_once("   ").unwrap_or((rest, ""));
            put(
                buf,
                area.x + 1,
                area.bottom().saturating_sub(1),
                &[
                    (key, theme.key),
                    ("  ", theme.text),
                    (label, theme.muted),
                    ("   ", theme.text),
                    (fact, theme.text),
                ],
            );
        } else if let Some(view) = &self.scan {
            let mut x = area.x + 1;
            for (i, (key, label)) in view.keys().into_iter().enumerate() {
                if i > 0 {
                    x = put(
                        buf,
                        x,
                        area.bottom().saturating_sub(1),
                        &[(FOOTER_GAP, theme.text)],
                    );
                }
                x = put(
                    buf,
                    x,
                    area.bottom().saturating_sub(1),
                    &[(key, theme.key), (" ", theme.text), (label, theme.muted)],
                );
            }
        } else {
            render_footer(theme, screen, area, buf);
        }
    }

    /// What the title line says about the scan, where the screen has it: the
    /// root it was run on, how many projects they hold, how long the scan took, and on the
    /// two screens whose way row says nothing of them, what is marked.
    fn context(&self, screen: Screen) -> (Option<String>, Vec<String>) {
        if self.running.is_some() || screen == Screen::Result {
            return (None, Vec::new());
        }
        let seen = &self.screens.dashboard.analysed;
        let place = seen.roots.first().map(|first| match seen.roots.len() - 1 {
            0 => tilde(first),
            more => format!("{} +{more}", tilde(first)),
        });
        // Nothing of the scan before this one is said as current: its project
        // count and its time describe a disk that has since changed.
        if let Some(view) = &self.scan {
            return (place, vec![view.chip()]);
        }
        let mut facts = Vec::new();
        if seen.projects > 0 {
            let s = if seen.projects == 1 { "" } else { "s" };
            facts.push(format!("{} project{s}", seen.projects));
        }
        if !seen.elapsed.is_zero() {
            facts.push(format!("scanned {}", scan_time(seen.elapsed)));
        }
        if matches!(screen, Screen::Dashboard | Screen::Projects)
            && let Some(marked) = self.marked_in_projects()
        {
            facts.push(marked);
        }
        (place, facts)
    }

    /// What the plan is built from on the way out of candidates: the marks
    /// before that step, the plan they became after it.
    fn captured(&self, screen: Screen) -> (usize, u64) {
        match screen {
            Screen::Candidates => {
                let marked = self.screens.candidates.marked();
                (marked.len(), marked.iter().map(|c| c.bytes).sum())
            }
            _ => self
                .app()
                .reviewing()
                .map_or((0, 0), |plan| (plan.items().len(), plan.total_bytes())),
        }
    }

    fn render_screen(&self, theme: &Theme, screen: Screen, body: Rect, buf: &mut Buffer) {
        match screen {
            Screen::Dashboard => self.screens.dashboard.render(theme, body, buf),
            Screen::Projects => self.screens.projects.render(theme, body, buf),
            Screen::Candidates => self.screens.candidates.render(theme, body, buf),
            // The plan and the record are borrowed from the router, which is
            // the only thing that has either. A screen that cannot reach one
            // draws nothing rather than inventing something to show.
            Screen::Review => {
                if let Some(plan) = self.app.as_ref().and_then(App::reviewing) {
                    self.review
                        .render(theme, plan, self.screens.candidates.locator(), body, buf);
                }
            }
            Screen::Confirm => {
                if let Some(plan) = self.app.as_ref().and_then(App::reviewing) {
                    self.confirm.render(theme, plan, body, buf);
                }
            }
            Screen::Result => {
                let mut body = body;
                // A run that stopped short says so above its own account, which
                // counts only what it knew about.
                if let Some(why) = &self.ended_early {
                    let width = body.width.saturating_sub(2) as usize;
                    let lines = wrap(why, width);
                    for (line, y) in lines.iter().zip(body.y..body.bottom()) {
                        buf.set_string(body.x + 1, y, line, theme.blocked);
                    }
                    let used = (lines.len() as u16 + 1).min(body.height);
                    body.y += used;
                    body.height -= used;
                }
                if let Some(manifest) = self.app.as_ref().and_then(App::result) {
                    self.report
                        .render(theme, manifest, self.record.as_deref(), body, buf);
                }
            }
        }
    }
}

/// Where a list stands, as far as telling a key that moved nothing why.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Place {
    at: usize,
    len: usize,
    /// Rows that fit, for a list that scrolls; `None` for one whose cursor
    /// moves, which has something to do even when every row is on screen.
    window: Option<usize>,
}

/// The motion a candidates key stands for, when it is one.
fn motion_of(key: Key) -> Option<Motion> {
    match key {
        Key::Up => Some(Motion::Up),
        Key::Down => Some(Motion::Down),
        Key::Top => Some(Motion::Top),
        Key::Bottom => Some(Motion::Bottom),
        Key::PageUp => Some(Motion::PageUp),
        Key::PageDown => Some(Motion::PageDown),
        _ => None,
    }
}

/// Why a motion that is bound moved nothing. A key that is bound and silent
/// reads as a keyboard that stopped working, which is the one thing the
/// notice row exists to prevent.
fn stayed(motion: Motion, at: Place) -> String {
    if at.len == 0 {
        return "The list is empty; nothing to scroll.".to_string();
    }
    if at.window.is_some_and(|window| at.len <= window) {
        return "Everything fits; nothing to scroll.".to_string();
    }
    match motion {
        Motion::Up | Motion::Top | Motion::PageUp => "Already at the top.".to_string(),
        Motion::Down | Motion::Bottom | Motion::PageDown => "Already at the bottom.".to_string(),
    }
}

/// How the interface ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Exit {
    /// The user quit.
    Quit,
    /// A signal ended it, or Ctrl-C while a scan ran. The terminal is back; the
    /// code is the shell's convention, 128 plus the signal.
    Interrupted(u8),
}

/// A scan the loop can start: it counts into the progress it is handed and
/// answers `None` if it was cancelled.
type Scan = Arc<dyn Fn(&Arc<Progress>) -> Option<Screens> + Send + Sync>;

fn start(scan: &Scan) -> io::Result<ScanJob> {
    let scan = Arc::clone(scan);
    ScanJob::spawn(move |progress| scan(progress))
}

/// Open the interface at once, on a scan that has only begun, and give the
/// terminal back on every exit.
///
/// `scan` measures `roots` and builds the screens; it runs on a thread of its
/// own, first now and again whenever a result is left for the dashboard, while
/// this draws how far it has got and listens for keys.
pub fn run(
    roots: Vec<PathBuf>,
    db: PathBuf,
    scan: impl Fn(&Arc<Progress>) -> Option<Screens> + Send + Sync + 'static,
) -> io::Result<Exit> {
    let scan: Scan = Arc::new(scan);
    signals::install();
    let mut terminal = terminal::enter()?;
    let outcome = drive(&mut terminal, Screens::pending(roots, db), &scan);
    // Not `?` above: an error on the way out is still reported, but not before
    // the terminal is usable enough to read it in.
    terminal::leave();
    outcome
}

fn drive(terminal: &mut DefaultTerminal, pending: Screens, scan: &Scan) -> io::Result<Exit> {
    let job = start(scan)?;
    // Read once, here: a draw never looks at the environment.
    let mut tui = Tui::starting(pending, Arc::clone(job.progress()), Instant::now())
        .with_theme(Theme::detect());
    let mut job = Some(job);
    loop {
        terminal.draw(|frame| tui.draw(frame))?;
        let now = Instant::now();
        tui.tick(now);

        if let Some(signal) = signals::pending() {
            if let Some(job) = &job {
                job.cancel();
            }
            return Ok(Exit::Interrupted(128 + signal as u8));
        }
        match job.as_ref().and_then(ScanJob::poll) {
            None => {}
            Some(Finished::Done(screens)) => {
                job = None;
                tui.finish_scan(*screens, now);
            }
            Some(Finished::Cancelled) => {
                job = None;
                tui.scan_cancelled(now);
            }
            // Handed on once the terminal is usable again: the panic hook has
            // already given it back and printed, and the thread that was
            // drawing it has nothing more to say.
            Some(Finished::Panicked(panic)) => {
                terminal::leave();
                std::panic::resume_unwind(panic);
            }
        }

        if !event::poll(TICK)? {
            continue;
        }
        let Event::Key(key) = event::read()? else {
            // Resize and focus events redraw on the next pass, which the draw
            // at the top of the loop already does.
            continue;
        };
        // In raw mode Ctrl-C is a key and not a signal. While a scan runs it
        // does what the signal does, which is to end the run.
        if tui.is_scanning() && is_interrupt(&key) {
            if let Some(job) = &job {
                job.cancel();
            }
            return Ok(Exit::Interrupted(130));
        }
        let Some(press) = translate(key) else {
            continue;
        };
        match tui.press(press, Instant::now()) {
            Step::Stay => {}
            Step::Quit => {
                if let Some(job) = &job {
                    job.cancel();
                }
                return Ok(Exit::Quit);
            }
            Step::Purge => tui.purge(Box::new(TrashRemover)),
            Step::Rescan => {
                let again = start(scan)?;
                tui.begin_scan(Arc::clone(again.progress()), Instant::now());
                job = Some(again);
                // Keys pressed in the instant before were meant for the screen
                // that was there, not for the one that just replaced it.
                while event::poll(Duration::ZERO)? {
                    event::read()?;
                }
            }
        }
    }
}

fn is_interrupt(key: &KeyEvent) -> bool {
    key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c')
}

/// A terminal's key event as the binding table names it.
///
/// Repeats count as presses: holding a key is how the confirmation is given,
/// and a terminal that distinguishes the two still means "the key is down".
/// Releases are dropped, because a terminal that reports them is the exception
/// and the loop must behave the same either way.
fn translate(key: KeyEvent) -> Option<KeyPress> {
    if !matches!(key.kind, KeyEventKind::Press | KeyEventKind::Repeat) {
        return None;
    }
    Some(match key.code {
        // Space arrives as a character and is bound as a key of its own, which
        // is how it reads in a footer.
        KeyCode::Char(' ') => KeyPress::Space,
        KeyCode::Char(c) => KeyPress::Char(c),
        KeyCode::Enter => KeyPress::Enter,
        KeyCode::Esc => KeyPress::Esc,
        KeyCode::Tab => KeyPress::Tab,
        KeyCode::Backspace => KeyPress::Backspace,
        KeyCode::Delete => KeyPress::Delete,
        KeyCode::Up => KeyPress::Up,
        KeyCode::Down => KeyPress::Down,
        KeyCode::Left => KeyPress::Left,
        KeyCode::Right => KeyPress::Right,
        KeyCode::Home => KeyPress::Home,
        KeyCode::End => KeyPress::End,
        KeyCode::PageUp => KeyPress::PageUp,
        KeyCode::PageDown => KeyPress::PageDown,
        _ => return None,
    })
}

/// The row under the title: where `Esc` goes back to, and where the way
/// forward leads.
///
/// `captured` is the count and total the way out of candidates builds the plan
/// from — the marks while on candidates, the plan they became on review.
pub fn wayfinding(screen: Screen, captured: (usize, u64)) -> String {
    let (count, bytes) = captured;
    let mut parts = Vec::new();
    match screen.previous() {
        Some(previous) => parts.push(format!("Esc ← {}", previous.name())),
        None if screen == Screen::Result => parts.push("the run is over".to_string()),
        None => {}
    }
    if screen == Screen::Review {
        parts.push(format!(
            "built from the {count} you marked ({})",
            human(bytes)
        ));
    }
    match screen.next() {
        Some(next) if screen == Screen::Candidates => parts.push(format!(
            "Enter → {}, built from the {count} marked ({})",
            next.name(),
            human(bytes)
        )),
        Some(next) if screen == Screen::Confirm => {
            parts.push(format!("hold {PURGE} → {}", next.name()))
        }
        Some(next) => parts.push(format!("Enter → {}", next.name())),
        // The keys are the footer's: said here too, in the same weight, the
        // list read as noise. The way row keeps the screen's state.
        None => {}
    }
    parts.join(WAY_SEPARATOR)
}

/// What to say about a key the screen does not answer to: the key, then the two
/// entries the screen most wants a hand to find.
///
/// Read from [`entries`], so it cannot name a key that does nothing.
fn unbound(screen: Screen, key: KeyPress) -> String {
    let advice: Vec<String> = entries(screen)
        .iter()
        .take(2)
        .map(|e| format!("{} {}", e.keys, e.label))
        .collect();
    format!("{key} does nothing here. {}.", advice.join(" · "))
}

/// One line of the key bar or the key list: every key that does one thing.
struct Entry {
    keys: String,
    label: &'static str,
    global: bool,
}

/// What a screen answers to, as it is worth showing: one entry per action, in
/// the order the room should go to them.
///
/// Grouped here rather than in the table. Dispatch matches one key press to one
/// row, which is what keeps `bindings_for` a filter and every invariant over it
/// a plain loop; `↑` and `k` sharing an entry is a matter of how they are read.
fn entries(screen: Screen) -> Vec<Entry> {
    let mut groups: Vec<(Vec<KeyPress>, &Binding)> = Vec::new();
    for binding in bindings_for(screen) {
        match groups
            .iter_mut()
            .find(|(_, b)| b.action == binding.action && b.label == binding.label)
        {
            Some((keys, _)) => keys.push(binding.key),
            None => groups.push((vec![binding.key], binding)),
        }
    }
    // Stable, so the table's own order holds within a rank. What deletes and
    // what marks come first; then what nobody would guess, like a digit that
    // sorts; then motion, which anyone tries; then Esc and Enter, which the row
    // under the title already names; the way out last, where it is kept.
    groups.sort_by_key(|(_, b)| match (b.effect(), b.action) {
        (Effect::Destructive, _) => 0,
        (Effect::Mark, _) => 1,
        _ if b.screen.is_none() => 5,
        (_, Action::Back | Action::Forward) => 4,
        (_, Action::Candidate(Key::Sort(_))) => 2,
        (_, Action::Move(_) | Action::Candidate(_)) => 3,
        _ => 2,
    });
    groups
        .into_iter()
        .map(|(mut keys, b)| {
            // The letter first: `j/↓`, the way the keys are usually written.
            keys.sort_by_key(|k| !matches!(k, KeyPress::Char(_)));
            Entry {
                keys: keys
                    .iter()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
                    .join("/"),
                label: b.label,
                global: b.screen.is_none(),
            }
        })
        .collect()
}

/// Every key this screen answers to, read from the table rather than described.
fn render_keys(theme: &Theme, screen: Screen, area: Rect, buf: &mut Buffer) {
    let left = area.x + 2;
    let mut y = area.y;
    y = section(
        buf,
        theme,
        left,
        y,
        area.width.saturating_sub(4) as usize,
        "Keys",
    );
    y += 1;
    let entries = entries(screen);
    let room = area.bottom().saturating_sub(y) as usize;
    // A list that stops at the edge reads as complete, so the last row it has
    // says what did not fit instead of showing one more key.
    let shown = if entries.len() > room {
        room.saturating_sub(1)
    } else {
        entries.len()
    };
    for entry in &entries[..shown] {
        let pad = " ".repeat(10usize.saturating_sub(entry.keys.chars().count()));
        put(
            buf,
            left,
            y,
            &[
                (entry.keys.as_str(), theme.key),
                (pad.as_str(), theme.text),
                (entry.label, theme.muted),
            ],
        );
        y += 1;
    }
    if shown < entries.len() {
        if y < area.bottom() {
            buf.set_string(
                left,
                y,
                format!("… {} more than fit here", entries.len() - shown),
                theme.blocked,
            );
        }
        return;
    }
    if y + 1 < area.bottom() {
        buf.set_string(left, y + 1, "Any key closes this.", theme.muted);
    }
}

/// In place of a body there is no room for: what the interface needs, what it
/// has, and the way out. On the confirm screen, also what the size costs, since
/// that is the one screen where a key is refused because of it.
fn render_too_small(
    theme: &Theme,
    screen: Screen,
    area: Rect,
    body: Rect,
    buf: &mut Buffer,
    scanning: bool,
) {
    let mut text = format!(
        "dev-cleaner needs {MIN_COLS}×{MIN_ROWS} and this terminal is {}×{}. \
         Resize it, or press q to quit.",
        area.width, area.height
    );
    if screen == Screen::Confirm {
        text.push_str(" The plan cannot be shown at this size; the hold is disabled until it can.");
    }
    if scanning {
        text.push_str(" The scan keeps running behind it.");
    }
    let width = body.width.saturating_sub(2) as usize;
    let lines = wrap(&text, width);
    // The paragraph is what matters here, so a heading above it is only drawn
    // where the paragraph still fits under it.
    let mut y = body.y;
    if body.height as usize > lines.len() {
        y = section(buf, theme, body.x + 1, y, width, "Too small");
    }
    for (line, y) in lines.iter().zip(y..body.bottom()) {
        buf.set_string(body.x + 1, y, line, theme.blocked);
    }
}

/// The footer, built from the same table the dispatch reads.
fn render_footer(theme: &Theme, screen: Screen, area: Rect, buf: &mut Buffer) {
    let line = footer(screen, area.width.saturating_sub(2) as usize);
    let y = area.bottom().saturating_sub(1);
    let mut x = area.x + 1;
    // The line is built by `footer` to fit, then coloured entry by entry: a key
    // cap, then its label muted. The gaps and the `…` that says an entry was
    // dropped are text, because a fact is never muted.
    for (i, entry) in line.split(FOOTER_GAP).enumerate() {
        if i > 0 {
            x = put(buf, x, y, &[(FOOTER_GAP, theme.text)]);
        }
        x = match entry.split_once(' ') {
            Some((key, label)) => put(
                buf,
                x,
                y,
                &[(key, theme.key), (" ", theme.text), (label, theme.muted)],
            ),
            None => put(buf, x, y, &[(entry, theme.text)]),
        };
    }
}

/// The key bar for `screen`, in at most `width` columns.
///
/// Entries are dropped whole, least important first, never cut; a line that
/// dropped any says so with `…`. The global keys are kept at the end whatever
/// the width, because `?` lists everything the bar had no room for.
///
/// ponytail: narrower than the globals themselves (about twenty columns), the
/// line is clipped by the buffer's edge. No terminal that narrow shows a table.
pub fn footer(screen: Screen, width: usize) -> String {
    const GAP: &str = FOOTER_GAP;
    let (globals, own): (Vec<_>, Vec<_>) = entries(screen)
        .into_iter()
        .map(|e| (e.global, format!("{} {}", e.keys, e.label)))
        .partition(|(global, _)| *global);
    let tail = globals
        .into_iter()
        .map(|(_, text)| text)
        .collect::<Vec<_>>()
        .join(GAP);
    let own: Vec<String> = own.into_iter().map(|(_, text)| text).collect();

    let whole = [own.as_slice(), std::slice::from_ref(&tail)]
        .concat()
        .join(GAP);
    if whole.chars().count() <= width {
        return whole;
    }
    // Room for the kept entries, then `GAP … GAP` and the globals.
    let room = width.saturating_sub(tail.chars().count() + 2 * GAP.len() + 1);
    let mut kept: Vec<&str> = Vec::new();
    let mut used = 0;
    for text in &own {
        let cost = text.chars().count() + if kept.is_empty() { 0 } else { GAP.len() };
        if used + cost > room {
            break;
        }
        used += cost;
        kept.push(text);
    }
    kept.push("…");
    kept.push(&tail);
    kept.join(GAP)
}

/// `path` with the home directory as `~`.
fn tilde(path: &Path) -> String {
    let shown = path.display().to_string();
    match std::env::var_os("HOME").map(PathBuf::from) {
        Some(home) => match path.strip_prefix(&home) {
            Ok(rest) if rest.as_os_str().is_empty() => "~".to_string(),
            Ok(rest) => format!("~/{}", rest.display()),
            Err(_) => shown,
        },
        None => shown,
    }
}

/// A scan time: tenths of a second, then minutes once it is long.
fn scan_time(d: Duration) -> String {
    let secs = d.as_secs_f64();
    if secs < 60.0 {
        format!("{secs:.1} s")
    } else {
        format!("{} min {} s", d.as_secs() / 60, d.as_secs() % 60)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_result_screen_names_the_first_two_keys_it_has() {
        // Reached only through a real purge, so the integration walk cannot
        // stand on it. It scrolls now, and what it names is what it does most.
        assert_eq!(
            unbound(Screen::Result, KeyPress::Tab),
            "Tab does nothing here. Enter/Esc dashboard · k/↑ up."
        );
    }
}
