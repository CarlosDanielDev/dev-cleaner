//! The event loop: terminal keys becoming the moves the screens already have.
//!
//! The loop owns no navigation rules. It looks a key up in the binding table,
//! and hands the result to whichever screen owns that move — routing is the
//! router's (#33), selection is the candidates screen's (#36), confirmation is
//! the confirm screen's (#37). A key the current screen does not answer to does
//! nothing at all; there is no fallback behaviour for the loop to invent.
//!
//! Dispatch is separated from the terminal for the same reason drawing was:
//! [`Tui::press`] is a function of a key and a screen, so the claim that no key
//! reaches a deletion can be driven over every key in CI, with no tty.

use std::io;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use ratatui::Frame;
use ratatui::buffer::Buffer;
use ratatui::crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind};
use ratatui::init::DefaultTerminal;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};

use super::data::Screens;
use super::{
    Action, App, Confirm, KeyPress, Motion, Report, Review, Screen, bindings_for, terminal,
};
use crate::purge::{Remover, TrashRemover, execute, free_bytes, manifest_dir, write_manifest};
use crate::safety::Plan;

/// How often the loop wakes with nothing to read.
///
/// Short enough that a hold which stopped is noticed promptly, long enough that
/// an idle interface is not redrawing for the sake of it.
const TICK: Duration = Duration::from_millis(100);

/// How long the purge key may go quiet before it counts as released.
///
/// A plain terminal reports no key-release event, so a hold that stopped shows
/// up as a repeat that never arrived. The window has to clear the gap before
/// the *first* repeat, which macOS defaults to around 375 ms.
///
/// ponytail: a constant, not the terminal's own repeat rate, which no terminal
/// reports. Someone who has set the slowest repeat macOS offers cannot fill the
/// gauge; read it from the kitty keyboard protocol's real release events if
/// that ever turns up.
const GRACE: Duration = Duration::from_millis(600);

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
    /// When the purge key last arrived.
    held_at: Option<Instant>,
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
            held_at: None,
        }
    }

    /// The router, for anything that only needs to look at it.
    pub fn app(&self) -> &App {
        self.app.as_ref().expect("the router is always present")
    }

    /// Handle one key.
    pub fn press(&mut self, key: KeyPress, now: Instant) -> Step {
        if self.help {
            // The overlay closes on any key and nothing underneath it moves,
            // which is what makes "?" safe to press while reading a plan.
            self.help = false;
            return Step::Stay;
        }

        let screen = self.app().screen();
        let Some(action) = bindings_for(screen)
            .iter()
            .find(|b| b.key == key)
            .map(|b| b.action)
        else {
            return Step::Stay;
        };

        match action {
            Action::Quit => Step::Quit,
            Action::Help => {
                self.help = true;
                Step::Stay
            }
            Action::Back => {
                self.transition(App::back);
                Step::Stay
            }
            Action::Forward => {
                self.forward(screen);
                Step::Stay
            }
            Action::Move(motion) => {
                self.move_within(screen, motion);
                Step::Stay
            }
            Action::Candidate(key) => {
                self.screens.candidates.press(key);
                Step::Stay
            }
            Action::Sort(column) => {
                self.screens.projects.sort_by(column);
                Step::Stay
            }
            Action::Purge => self.hold(now),
        }
    }

    /// Notice a hold that stopped.
    ///
    /// Called every time round the loop, including the times nothing was read,
    /// because a key going quiet is exactly the event a terminal does not send.
    pub fn tick(&mut self, now: Instant) {
        if self
            .held_at
            .is_some_and(|last| now.duration_since(last) > GRACE)
        {
            self.held_at = None;
            self.confirm.release();
        }
    }

    /// Account for another repeat of the purge key.
    fn hold(&mut self, now: Instant) -> Step {
        // The first press contributes nothing: there is no earlier event to
        // measure from, and a hold has to be a stretch of time rather than a
        // keystroke. `Confirm::hold` caps each step in turn, so no single
        // delayed event can arm the gauge on its own.
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
    fn forward(&mut self, screen: Screen) {
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
        self.app = Some(App::new(draft).forward().forward().forward());
        self.review = Review::new();
    }

    fn move_within(&mut self, screen: Screen, motion: Motion) {
        match screen {
            Screen::Projects => match motion {
                Motion::Up => self.screens.projects.up(),
                Motion::Down => self.screens.projects.down(),
                // ponytail: the table binds a row at a time and nothing else.
                // Give it the rest of the motions when a corpus makes paging
                // through a few hundred rows worth the keys.
                _ => {}
            },
            Screen::Review => {
                if let Some(plan) = self.app.as_ref().and_then(App::reviewing) {
                    self.review.scroll(motion, plan, self.rows);
                }
            }
            _ => {}
        }
    }

    /// Apply a move that consumes the router.
    fn transition(&mut self, move_to: impl FnOnce(App) -> App) {
        let app = self.app.take().expect("the router is always present");
        self.app = Some(move_to(app));
    }

    /// Carry out the plan.
    ///
    /// Reached only from [`Step::Purge`], which `press` returns only from the
    /// confirm screen with the hold complete. Everything after that is the same
    /// code `purge --execute` runs: the phrase comes from the plan, the plan
    /// confirms itself, and `execute` is what produces a record.
    pub fn purge(&mut self, remover: &dyn Remover) {
        let app = self.app.take().expect("the router is always present");
        let Some(phrase) = app.phrase() else {
            self.app = Some(app);
            return;
        };
        let plan = match app.confirm(&phrase) {
            Ok(plan) => plan,
            // The phrase describes this exact plan, so a refusal means the two
            // disagree. The app comes back rather than the plan being lost.
            Err(app) => {
                self.app = Some(app);
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
        let mut manifest = execute(plan, remover);
        if let (Some(before), Some(after)) = (before, free_bytes(&measure_at)) {
            manifest.record_actual(after.saturating_sub(before));
        }
        self.record = write_manifest(&manifest, &manifest_dir()).ok();

        // `confirm` consumed the router along with the plan it held, and
        // `finished` takes the manifest, which only `execute` produces. A fresh
        // router carries the record forward; there is no plan left to carry,
        // and the result screen is the end of the road either way.
        self.app = Some(App::new(Plan::draft()).finished(manifest));
    }

    fn draw(&mut self, frame: &mut Frame) {
        let area = frame.area();
        let body = Rect {
            x: area.x,
            y: area.y.saturating_add(2),
            width: area.width,
            height: area.height.saturating_sub(3),
        };
        self.rows = body.height as usize;

        let screen = self.app().screen();
        let help = self.help;
        let buf = frame.buffer_mut();

        buf.set_string(
            area.x + 1,
            area.y,
            title(screen),
            Style::new().add_modifier(Modifier::BOLD),
        );
        if help {
            render_keys(screen, body, buf);
        } else {
            self.render_screen(screen, body, buf);
        }
        render_footer(screen, area, buf);
    }

    fn render_screen(&self, screen: Screen, body: Rect, buf: &mut Buffer) {
        match screen {
            Screen::Dashboard => self.screens.dashboard.render(body, buf),
            Screen::Projects => self.screens.projects.render(body, buf),
            Screen::Candidates => self.screens.candidates.render(body, buf),
            // The plan and the record are borrowed from the router, which is
            // the only thing that has either. A screen that cannot reach one
            // draws nothing rather than inventing something to show.
            Screen::Review => {
                if let Some(plan) = self.app.as_ref().and_then(App::reviewing) {
                    self.review.render(plan, body, buf);
                }
            }
            Screen::Confirm => {
                if let Some(plan) = self.app.as_ref().and_then(App::reviewing) {
                    self.confirm.render(plan, body, buf);
                }
            }
            Screen::Result => {
                if let Some(manifest) = self.app.as_ref().and_then(App::result) {
                    self.report
                        .render(manifest, self.record.as_deref(), body, buf);
                }
            }
        }
    }
}

/// Open the interface on `screens` and give the terminal back on every exit.
pub fn run(screens: Screens) -> io::Result<()> {
    let mut terminal = terminal::enter()?;
    let outcome = drive(&mut terminal, screens);
    // Not `?` above: an error on the way out is still reported, but not before
    // the terminal is usable enough to read it in.
    terminal::leave();
    outcome
}

fn drive(terminal: &mut DefaultTerminal, screens: Screens) -> io::Result<()> {
    let mut tui = Tui::new(screens);
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
            Step::Purge => tui.purge(&TrashRemover),
        }
    }
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

fn title(screen: Screen) -> &'static str {
    match screen {
        Screen::Dashboard => "dev-cleaner",
        Screen::Projects => "dev-cleaner  ·  projects",
        Screen::Candidates => "dev-cleaner  ·  candidates",
        Screen::Review => "dev-cleaner  ·  the plan",
        Screen::Confirm => "dev-cleaner  ·  confirm",
        Screen::Result => "dev-cleaner  ·  result",
    }
}

/// Every key this screen answers to, read from the table rather than described.
fn render_keys(screen: Screen, area: Rect, buf: &mut Buffer) {
    let left = area.x + 2;
    let mut y = area.y;
    buf.set_string(left, y, "Keys", Style::new().add_modifier(Modifier::BOLD));
    y += 2;
    for binding in bindings_for(screen) {
        if y >= area.bottom() {
            return;
        }
        buf.set_string(
            left,
            y,
            format!("{:<10}{}", binding.key, binding.label),
            Style::new(),
        );
        y += 1;
    }
    if y + 1 < area.bottom() {
        buf.set_string(
            left,
            y + 1,
            "Any key closes this.",
            Style::new().fg(Color::DarkGray),
        );
    }
}

/// The footer, built from the same table the dispatch reads.
fn render_footer(screen: Screen, area: Rect, buf: &mut Buffer) {
    let line = bindings_for(screen)
        .iter()
        .map(|b| format!("{} {}", b.key, b.label))
        .collect::<Vec<_>>()
        .join("   ");
    let width = area.width.saturating_sub(2) as usize;
    let line: String = line.chars().take(width).collect();
    buf.set_string(
        area.x + 1,
        area.bottom().saturating_sub(1),
        line,
        Style::new().fg(Color::DarkGray),
    );
}
