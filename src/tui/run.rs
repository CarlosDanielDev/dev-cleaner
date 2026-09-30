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

use super::data::Screens;
use super::palette::{DEFAULT, HEAD, MUTED, WARNING_BAND};
use super::review;
use super::{
    Action, App, Binding, Confirm, Effect, Key, KeyPress, Motion, PURGE, Report, Review, Screen,
    bindings_for, terminal,
};
use crate::bytes::human;
use crate::purge::{Remover, TrashRemover, execute, free_bytes, manifest_dir, write_manifest};
use crate::safety::Plan;

/// How often the loop wakes with nothing to read.
///
/// Short enough that a hold which stopped is noticed promptly, long enough that
/// an idle interface is not redrawing for the sake of it.
const TICK: Duration = Duration::from_millis(100);

/// How long a notice stays on its row once nothing newer replaces it.
///
/// On the clock rather than a count of frames: a resize storm or a slow
/// terminal redraws at its own pace, and a notice has to last the same on
/// every machine.
pub const NOTICE_TTL: Duration = Duration::from_secs(3);

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

/// One line under the body: what the last key did, or did not do.
#[derive(Debug)]
struct Notice {
    text: String,
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
    /// When the purge key last arrived.
    held_at: Option<Instant>,
    /// What the last key did, until the tick lets it go.
    notice: Option<Notice>,
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
            notice: None,
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
            // which is what makes "?" safe to press while reading a plan. The
            // key it closed on went nowhere, and the row under the body says so.
            self.help = false;
            self.notify(format!("Keys closed. {key} was not applied."), now);
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

    /// Say what the last key did, on the row under the body.
    ///
    /// A newer notice replaces an older one and its time starts again: the row
    /// is for the last key, not for a queue of them.
    fn notify(&mut self, text: String, now: Instant) {
        self.notice = Some(Notice { text, at: now });
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
        self.arrive(App::new(draft).forward().forward().forward());
        self.review = Review::new();
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
        self.confirm = Confirm::new();
        self.held_at = None;
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
        let mut manifest = execute(plan, remover);
        if let (Some(before), Some(after)) = (before, free_bytes(&measure_at)) {
            manifest.record_actual(after.saturating_sub(before));
        }
        self.record = write_manifest(&manifest, &manifest_dir()).ok();

        // `confirm` consumed the router along with the plan it held, and
        // `finished` takes the manifest, which only `execute` produces. A fresh
        // router carries the record forward; there is no plan left to carry,
        // and the result screen is the end of the road either way.
        self.arrive(App::new(Plan::draft()).finished(manifest));
    }

    fn draw(&mut self, frame: &mut Frame) {
        let area = frame.area();
        self.render(area, frame.buffer_mut());
    }

    /// Paint the whole interface into `buf`, with no terminal behind it.
    ///
    /// Two rows of title above the body, and two below it: the notice row,
    /// then the key bar.
    pub fn render(&mut self, area: Rect, buf: &mut Buffer) {
        let body = Rect {
            x: area.x,
            y: area.y.saturating_add(2),
            width: area.width,
            height: area.height.saturating_sub(4),
        };
        self.rows = body.height as usize;

        let screen = self.app().screen();
        let help = self.help;

        // The confirm screen's title is a band across the whole width, set
        // apart by weight so it reads on a terminal with no colour at all: the
        // one screen that removes anything must not look like one that lists.
        if screen == Screen::Confirm {
            let blank = " ".repeat(area.width as usize);
            buf.set_string(area.x, area.y, blank, WARNING_BAND);
            buf.set_string(area.x + 1, area.y, screen.title(), WARNING_BAND);
        } else {
            buf.set_string(area.x + 1, area.y, screen.title(), HEAD);
        }
        let line: String = wayfinding(screen, self.captured(screen))
            .chars()
            .take(area.width.saturating_sub(2) as usize)
            .collect();
        buf.set_string(area.x + 1, area.y + 1, line, DEFAULT);
        // A body with no rows draws nothing, rather than its first line over
        // the row below it.
        if body.height > 0 {
            if help {
                render_keys(screen, body, buf);
            } else {
                self.render_screen(screen, body, buf);
            }
        }
        // A fact, so never `MUTED`. Where the area has no row of its own for
        // it, the notice is left out rather than drawn over the way or the
        // title.
        if let Some(notice) = &self.notice
            && area.height >= 4
        {
            let line: String = notice
                .text
                .chars()
                .take(area.width.saturating_sub(2) as usize)
                .collect();
            buf.set_string(area.x + 1, area.bottom() - 2, line, DEFAULT);
        }
        render_footer(screen, area, buf);
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
        // Read from the table, so the keys named here are the ones that work.
        None => parts.extend(
            bindings_for(screen)
                .iter()
                .map(|b| format!("{} {}", b.key, b.label)),
        ),
    }
    parts.join("   ·   ")
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
fn render_keys(screen: Screen, area: Rect, buf: &mut Buffer) {
    let left = area.x + 2;
    let mut y = area.y;
    buf.set_string(left, y, "Keys", HEAD);
    y += 2;
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
        buf.set_string(
            left,
            y,
            format!("{:<10}{}", entry.keys, entry.label),
            DEFAULT,
        );
        y += 1;
    }
    if shown < entries.len() {
        if y < area.bottom() {
            buf.set_string(
                left,
                y,
                format!("… {} more than fit here", entries.len() - shown),
                DEFAULT,
            );
        }
        return;
    }
    if y + 1 < area.bottom() {
        buf.set_string(left, y + 1, "Any key closes this.", MUTED);
    }
}

/// The footer, built from the same table the dispatch reads.
fn render_footer(screen: Screen, area: Rect, buf: &mut Buffer) {
    let line = footer(screen, area.width.saturating_sub(2) as usize);
    buf.set_string(area.x + 1, area.bottom().saturating_sub(1), line, DEFAULT);
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
    const GAP: &str = "   ";
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
