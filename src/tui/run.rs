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

use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::thread;
use std::time::{Duration, Instant};

use ratatui::Frame;
use ratatui::buffer::Buffer;
use ratatui::crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind};
use ratatui::init::DefaultTerminal;
use ratatui::layout::Rect;
use ratatui::style::Style;

use super::data::{Screens, label_for};
use super::logo;
use super::palette::Theme;
use super::projects::truncate;
use super::result::wrap;
use super::review;
use super::row::{RULE, put, section};
use super::running::Running;
use super::{
    Action, App, Binding, Confirm, Effect, Key, KeyPress, Marking, Motion, PURGE, Report, Review,
    Screen, bindings_for, terminal,
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
const WAY_SEPARATOR: &str = "   ·   ";

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
    /// How far the scan that follows a result has got, while it runs.
    scanning: Option<String>,
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
            scanning: None,
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

    /// Where the list on `screen` stands: the row the cursor or the window is
    /// on, how many rows there are, and how many fit when the list scrolls
    /// rather than moves a cursor. What a motion that stayed put is told by.
    fn place(&self, screen: Screen) -> Place {
        match screen {
            Screen::Projects => Place {
                at: self.screens.projects.cursor(),
                len: self.screens.projects.rows().len(),
                window: None,
            },
            Screen::Candidates => Place {
                at: self.screens.candidates.cursor(),
                len: self.screens.candidates.selectable().len(),
                window: None,
            },
            Screen::Review => {
                let len = self
                    .app
                    .as_ref()
                    .and_then(App::reviewing)
                    .map_or(0, |plan| plan.items().len());
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

    /// Show how far the scan behind a result has got, or stop showing it.
    pub fn scanning(&mut self, line: Option<String>) {
        self.scanning = line;
    }

    /// Land on the dashboard of a scan taken after a run.
    ///
    /// Everything the run left behind goes: the marks and the plan with the old
    /// screens, the record's path and the verdict with the result. The record is
    /// already on disk, so nothing is lost, and the notice says what happened.
    pub fn resume(&mut self, screens: Screens, now: Instant) {
        let projects = screens.projects.rows().len();
        self.screens = screens;
        self.review = Review::new();
        self.report = Report::new();
        self.record = None;
        self.ended_early = None;
        self.help = false;
        self.scanning = None;
        self.notice = None;
        self.arrive(App::new(Plan::draft()));
        let s = if projects == 1 { "" } else { "s" };
        self.notify(
            format!("Back at the dashboard. Rescanned {projects} project{s}."),
            Tone::Done,
            now,
        );
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
        match marking {
            Marking::Marked(path, size) => entry("Marked +", &path, size),
            Marking::Unmarked(path, size) => entry("Unmarked", &path, size),
            Marking::MarkedAll(n, size) => format!("Marked all {n}  ({}).", human(size)),
            Marking::Cleared(0, _) => "Nothing marked.".to_string(),
            Marking::Cleared(n, size) => {
                let s = if n == 1 { "" } else { "s" };
                format!(
                    "Cleared {n} mark{s}  ({}).  c again restores them.",
                    human(size)
                )
            }
            Marking::Restored(n, size) => {
                let s = if n == 1 { "" } else { "s" };
                format!("Restored {n} mark{s}  ({}).", human(size))
            }
            Marking::NothingToMark => "Nothing to mark.".to_string(),
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
        if screen == Screen::Projects {
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

    /// Put the candidates cursor on the project the table's cursor was on, and
    /// say what is there.
    ///
    /// The router carries the plan and nothing else, so the project is not
    /// carried through it: the entries are found by their path under the
    /// project's, which the table already knows.
    fn land_on_project(&mut self, now: Instant) {
        let Some(row) = self.screens.projects.selected() else {
            return;
        };
        let (root, name) = (
            row.path.clone(),
            self.screens.projects.label(row).to_string(),
        );
        let candidates = &mut self.screens.candidates;
        let offered: Vec<u64> = candidates
            .selectable()
            .iter()
            .filter(|c| c.path.starts_with(&root))
            .map(|c| c.bytes)
            .collect();
        let candidates_focused = candidates.focus(&root).is_some();
        let text = if candidates_focused {
            let n = offered.len();
            let (noun, verb) = if n == 1 {
                ("directory", "can")
            } else {
                ("directories", "can")
            };
            format!(
                "{name}: {n} {noun} {verb} be rebuilt, {}. The cursor is on the first.",
                human(offered.iter().sum())
            )
        } else {
            let mut reasons: Vec<(&str, usize)> = Vec::new();
            for b in candidates
                .blocked()
                .iter()
                .filter(|b| b.path.starts_with(&root))
            {
                match reasons.iter_mut().find(|(r, _)| *r == b.reason) {
                    Some((_, n)) => *n += 1,
                    None => reasons.push((&b.reason, 1)),
                }
            }
            let held = reasons
                .iter()
                .map(|(reason, n)| format!("{n} held back — {reason}"))
                .collect::<Vec<_>>()
                .join("  ");
            if held.is_empty() {
                format!("{name}: nothing can be rebuilt here.")
            } else {
                format!("{name}: nothing can be rebuilt here. {held}")
            }
        };
        let tone = if candidates_focused {
            Tone::Done
        } else {
            Tone::Refused
        };
        self.notify(text, tone, now);
    }

    fn move_within(&mut self, screen: Screen, motion: Motion) {
        match screen {
            Screen::Projects => {
                // A page is what the user sees: the body less the header row
                // and the position line the table draws.
                let rows = self.rows.saturating_sub(2);
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
                    self.review.scroll(motion, plan, rows);
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
    /// or more the header grows to the icon's height, which the title shares.
    /// Under it the header is the two rows it was before there was an icon, and
    /// every screen keeps the room it has at [`MIN_COLS`]×[`MIN_ROWS`].
    pub fn render(&mut self, area: Rect, buf: &mut Buffer) {
        let theme = self.theme;
        let theme = &theme;
        let tall = area.width >= logo::MIN_COLS && area.height >= logo::MIN_ROWS;
        let header = if tall { logo::HEIGHT } else { 2 };
        let body = Rect {
            x: area.x,
            y: area.y.saturating_add(header),
            width: area.width,
            height: area.height.saturating_sub(header + 2),
        };
        self.rows = body.height as usize;
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

        // The icon stands at the left edge with the wordmark on its centre line
        // and the way under it, and only where no word of either would reach
        // the right edge: text wins, and a frame that cannot fit both draws the
        // text alone, at the left edge, in a header that keeps its height. The
        // band screens keep their band whole, and the running screen is that
        // screen still.
        let band = screen == Screen::Confirm || running;
        let way = (area.height > 1).then(|| {
            let way = if running {
                "no way back   ·   files go to the Trash   ·   the record is written as items move"
                    .to_string()
            } else {
                wayfinding(screen, self.captured(screen))
            };
            // Cut with a mark: at the minimum width a long plan's count and
            // total already carry the row past the edge.
            truncate(&way, area.width.saturating_sub(2) as usize)
        });
        let title = if running {
            "Purging".to_string()
        } else {
            screen.title()
        };
        let beside = area.x + 1 + logo::WIDTH + logo::GAP;
        let widest = title
            .chars()
            .count()
            .max(way.as_deref().map_or(0, |w| w.chars().count()));
        let icon = tall && !band && logo::fits(area.right(), beside, widest);
        let (left, top) = if icon {
            (beside, area.y + logo::HEIGHT / 2)
        } else {
            (area.x + 1, area.y)
        };

        // The confirm screen's title is a band across the whole width, set
        // apart by weight so it reads on a terminal with no colour at all: the
        // one screen that removes anything must not look like one that lists.
        if band {
            let blank = " ".repeat(area.width as usize);
            buf.set_string(area.x, area.y, blank, theme.warning_band);
            buf.set_string(area.x + 1, area.y, title, theme.warning_band);
        } else {
            let mut parts = logo::wordmark(theme);
            parts.push(("  ·  ".to_string(), theme.violet));
            parts.push((screen.name().to_string(), theme.text));
            let end = put(buf, left, top, &parts);
            // Drawn out to the right edge, so the title is a heading and not
            // one more line of text.
            let room = (area.right().saturating_sub(end) as usize).saturating_sub(2);
            if room > 0 {
                buf.set_string(end + 1, top, RULE.to_string().repeat(room), theme.violet);
            }
        }
        if let Some(line) = way {
            put(buf, left, top + 1, &way_parts(theme, &line));
        }
        if icon {
            logo::draw(theme, buf, area.x + 1, area.y);
        }
        // A body with no rows draws nothing, rather than its first line over
        // the row below it.
        if body.height > 0 {
            if let Some(run) = &self.running {
                // Drawn at any size: it takes no confirmation, and the way out
                // that the too-small paragraph offers is refused while it runs.
                run.render(theme, body, buf);
            } else if let Some(line) = &self.scanning {
                render_scanning(theme, line, body, buf);
            } else if help {
                // Over the screen rather than part of it: the list already says
                // when it ran out of room, and any key closes it onto whatever
                // is beneath, the notice included.
                render_keys(theme, screen, body, buf);
            } else if too_small {
                render_too_small(theme, screen, area, body, buf);
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
        } else {
            render_footer(theme, screen, area, buf);
        }
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
                    self.review.render(theme, plan, body, buf);
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

/// In place of a body while the scan after a result runs: what is happening,
/// and why the numbers on the old screen are not being shown.
fn render_scanning(theme: &Theme, line: &str, body: Rect, buf: &mut Buffer) {
    let left = body.x + 2;
    let width = body.width.saturating_sub(4) as usize;
    let mut y = section(buf, theme, left, body.y, width, "Scanning");
    if y < body.bottom() {
        put(buf, left, y, &[(line, theme.accent)]);
        y += 1;
    }
    if y < body.bottom() {
        buf.set_string(
            left,
            y,
            "The purge changed the disk, so the old numbers are being measured again.",
            theme.muted,
        );
    }
}

/// How the scan after a result says how far it has got.
fn scan_line(entries: u64, bytes: u64, elapsed: Duration) -> String {
    format!(
        "scanning · {entries} entries · {} · {:.1} s",
        human(bytes),
        elapsed.as_secs_f64()
    )
}

/// Open the interface on `screens` and give the terminal back on every exit.
///
/// `rescan` measures the same roots again, counting into the progress it is
/// handed, whenever a result is left for the dashboard.
pub fn run(
    screens: Screens,
    mut rescan: impl FnMut(&Arc<Progress>) -> Screens + Send,
) -> io::Result<()> {
    let mut terminal = terminal::enter()?;
    let outcome = drive(&mut terminal, screens, &mut rescan);
    // Not `?` above: an error on the way out is still reported, but not before
    // the terminal is usable enough to read it in.
    terminal::leave();
    outcome
}

fn drive(
    terminal: &mut DefaultTerminal,
    screens: Screens,
    rescan: &mut (impl FnMut(&Arc<Progress>) -> Screens + Send),
) -> io::Result<()> {
    // Read once, here: a draw never looks at the environment.
    let mut tui = Tui::new(screens).with_theme(Theme::detect());
    loop {
        terminal.draw(|frame| tui.draw(frame))?;
        tui.tick(Instant::now());

        if !event::poll(TICK)? {
            continue;
        }
        let Event::Key(key) = event::read()? else {
            // Resize and focus events redraw on the next pass, which the draw
            // at the top of the loop already does.
            continue;
        };
        let Some(press) = translate(key) else {
            continue;
        };
        match tui.press(press, Instant::now()) {
            Step::Stay => {}
            Step::Quit => return Ok(()),
            Step::Purge => tui.purge(Box::new(TrashRemover)),
            Step::Rescan => {
                let fresh = scan_again(terminal, &mut tui, rescan)?;
                tui.resume(fresh, Instant::now());
                // Keys pressed while the scan ran were meant for the screen
                // that was there, not for the dashboard it ended on.
                while event::poll(Duration::ZERO)? {
                    event::read()?;
                }
            }
        }
    }
}

/// Scan again on a thread of its own, drawing how far it has got until it ends.
fn scan_again(
    terminal: &mut DefaultTerminal,
    tui: &mut Tui,
    rescan: &mut (impl FnMut(&Arc<Progress>) -> Screens + Send),
) -> io::Result<Screens> {
    let progress = Arc::new(Progress::default());
    let started = Instant::now();
    thread::scope(|scope| {
        let job = scope.spawn(|| rescan(&progress));
        while !job.is_finished() {
            tui.scanning(Some(scan_line(
                progress.entries.load(Ordering::Relaxed),
                progress.bytes.load(Ordering::Relaxed),
                started.elapsed(),
            )));
            terminal.draw(|frame| tui.draw(frame))?;
            thread::sleep(TICK);
        }
        Ok(job
            .join()
            .unwrap_or_else(|panic| std::panic::resume_unwind(panic)))
    })
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
        None => parts.push("the first screen".to_string()),
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
fn render_too_small(theme: &Theme, screen: Screen, area: Rect, body: Rect, buf: &mut Buffer) {
    let mut text = format!(
        "dev-cleaner needs {MIN_COLS}×{MIN_ROWS} and this terminal is {}×{}. \
         Resize it, or press q to quit.",
        area.width, area.height
    );
    if screen == Screen::Confirm {
        text.push_str(" The plan cannot be shown at this size; the hold is disabled until it can.");
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

/// The way row, drawn as a breadcrumb: key caps, muted arrows and separators,
/// the screen `Esc` goes back to in the head colour and the one the way forward
/// leads to in the accent, the state of the screen in the head colour, and
/// every fact in text.
fn way_parts<'a>(theme: &Theme, line: &'a str) -> Vec<(&'a str, Style)> {
    let mut parts = Vec::new();
    for (i, part) in line.split(WAY_SEPARATOR).enumerate() {
        if i > 0 {
            parts.push((WAY_SEPARATOR, theme.muted));
        }
        // `Esc ← Back`, `Enter → Next`, `hold P → Next`: the key, then the arrow.
        let (key, rest) = match part.split_once(' ') {
            Some((key @ ("Esc" | "Enter"), rest)) => (Some(key), rest),
            Some(("hold", rest)) => {
                parts.push(("hold ", theme.text));
                let (k, rest) = rest.split_once(' ').unwrap_or((rest, ""));
                (Some(k), rest)
            }
            _ => (None, part),
        };
        if let Some(key) = key {
            parts.push((key, theme.key));
            parts.push((" ", theme.text));
        }
        let Some(at) = rest.find(['←', '→']) else {
            // What the screen is, not what to press: the screen's own words.
            let own = matches!(rest, "the first screen" | "the run is over");
            parts.push((rest, if own { theme.head } else { theme.text }));
            continue;
        };
        let arrow = at + rest[at..].chars().next().map_or(0, char::len_utf8);
        let name_style = if rest[at..].starts_with('←') {
            theme.head
        } else {
            theme.accent
        };
        parts.push((&rest[..at], theme.text));
        parts.push((&rest[at..arrow], theme.muted));
        let after = &rest[arrow..];
        let name = after.trim_start();
        let end = name.find(',').unwrap_or(name.len());
        parts.push((&after[..after.len() - name.len()], theme.text));
        parts.push((&name[..end], name_style));
        parts.push((&name[end..], theme.text));
    }
    parts
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
