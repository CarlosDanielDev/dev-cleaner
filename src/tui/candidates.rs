use std::collections::BTreeSet;
use std::ffi::OsStr;
use std::path::{Path, PathBuf};

use super::palette::Theme;
use super::row::{columns, describe, elide_path, elide_tail, put, section, widest};
use super::{showing, window_start};
use crate::bytes::human;
use crate::safety::{Candidate, Rejected};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;

/// Something the tool will not offer, and the reason in the user's words.
#[derive(Debug, Clone)]
pub struct Blocked {
    pub path: PathBuf,
    pub reason: String,
}

/// A key the screen answers to.
///
/// Enumerable on purpose: the guarantee this screen exists to make is tested by
/// driving every key in every order, and a keymap that cannot be enumerated
/// cannot be exhaustively tested.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Key {
    Up,
    Down,
    Top,
    Bottom,
    PageUp,
    PageDown,
    Toggle,
    MarkAll,
    ClearMarks,
    /// Order the offerable entries, or reverse them if already so ordered.
    Sort(Order),
}

impl Key {
    pub fn all() -> &'static [Key] {
        &[
            Key::Up,
            Key::Down,
            Key::Top,
            Key::Bottom,
            Key::PageUp,
            Key::PageDown,
            Key::Toggle,
            Key::MarkAll,
            Key::ClearMarks,
            Key::Sort(Order::Path),
            Key::Sort(Order::Size),
            Key::Sort(Order::Kind),
        ]
    }
}

/// What the offerable entries can be ordered by.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Order {
    Path,
    Size,
    /// The artifact directory's own name: `target`, `node_modules`, `.venv`.
    Kind,
}

impl Order {
    /// Which way round this order is most useful first: the projects table's
    /// own reasoning. Sizes answer "what is worst", so they start at the
    /// largest; names answer "where is X", so they start at A.
    fn starts_descending(self) -> bool {
        matches!(self, Order::Size)
    }

    /// The order's own name, for the view bar.
    fn name(self) -> &'static str {
        match self {
            Order::Path => "path",
            Order::Size => "size",
            Order::Kind => "kind",
        }
    }

    /// Which end comes first, in words, for the view bar.
    fn way(self, descending: bool) -> &'static str {
        match (self, descending) {
            (Order::Size, true) => "largest first",
            (Order::Size, false) => "smallest first",
            (_, false) => "A to Z",
            (_, true) => "Z to A",
        }
    }

    /// The order in words, for the heading.
    fn words(self, descending: bool) -> &'static str {
        match (self, descending) {
            (Order::Size, true) => "largest first",
            (Order::Size, false) => "smallest first",
            (Order::Path, false) => "by path",
            (Order::Path, true) => "by path, reversed",
            (Order::Kind, false) => "by kind",
            (Order::Kind, true) => "by kind, reversed",
        }
    }
}

/// What a key did to the marks, for the row that says so.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Marking {
    Marked(PathBuf, u64),
    Unmarked(PathBuf, u64),
    /// How many, and how many bytes.
    MarkedAll(usize, u64),
    Cleared(usize, u64),
    /// `c` again after clearing: how many came back, and how many bytes.
    Restored(usize, u64),
    /// A mark key on a list with nothing to mark.
    NothingToMark,
}

/// What the marks say about one project: the entries it offers and how many of
/// them are marked, each with its bytes.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Tally {
    pub offered: (usize, u64),
    pub marked: (usize, u64),
}

/// What `Space` on a project of the projects table did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProjectMarking {
    /// Every offerable entry of the project is marked now: how many, how many bytes.
    Marked(usize, u64),
    /// Every offerable entry of the project was marked, and none is now.
    Unmarked(usize, u64),
    /// The project offers nothing, so nothing changed.
    NothingOffered,
}

/// The project the screen is showing: its root, what to call it, and the roots
/// of the projects inside it, whose entries are theirs and not its own.
#[derive(Debug, Clone)]
struct Scope {
    root: PathBuf,
    name: String,
    inner: Vec<PathBuf>,
}

impl Scope {
    fn owns(&self, path: &Path) -> bool {
        path.starts_with(&self.root) && !self.inner.iter().any(|i| path.starts_with(i))
    }
}

/// Rows the screen keeps for itself above the list: the view bar, a blank row
/// and the heading.
const CHROME: usize = 3;

/// Where a line of keys begins in an empty body, so it is drawn quieter than
/// the facts above it.
const KEYS_LEAD: &str = "Keys: ";

/// How far a page key moves in a body of `rows`: the entries the window shows,
/// which is the body less what is above them. A body no frame has drawn yet has
/// no window, and a page of none would leave the key doing nothing, so it is one.
fn page(rows: usize) -> usize {
    rows.saturating_sub(CHROME).max(1)
}

/// The artifact directory's name, which is what says what kind of thing it is.
fn kind(path: &Path) -> &OsStr {
    path.file_name().unwrap_or(path.as_os_str())
}

/// The candidates screen.
///
/// The safe and the blocked are kept in separate collections, and the cursor is
/// an index into the safe one. That is the whole mechanism: a blocked entry has
/// no index, so no key, and no sequence of keys, can address it. Skipping over
/// blocked entries in a single list would leave the wrong path reachable and
/// merely unvisited, one logic error away from being visited.
#[derive(Debug)]
pub struct Candidates {
    selectable: Vec<Candidate>,
    blocked: Vec<Blocked>,
    order: Order,
    descending: bool,
    cursor: usize,
    /// Keyed by path, not by row. Rows move when the list is reordered; a mark
    /// that named a row would then name whatever moved into it, and the marks
    /// are what the plan is built from.
    marked: BTreeSet<PathBuf>,
    /// What `c` last cleared, for `c` to put back. By path, for the same reason
    /// as `marked`. Any change to the marks drops it: restoring over a selection
    /// the user has since made differently would re-mark what they just passed
    /// over.
    cleared: Option<BTreeSet<PathBuf>>,
    /// The project the screen was opened on, once it was opened on one.
    project: Option<Scope>,
    /// Whether `Tab` has widened the screen from that project to every project.
    /// The scope is a filter over the same entries and the same marks: nothing
    /// is copied, so nothing can drift.
    widened: bool,
    /// Where something can be rebuilt, in words, for the body that has nothing
    /// to list. Said by whoever knows the projects; this screen does not.
    elsewhere: Vec<String>,
}

impl Candidates {
    /// Sort the entries by what can be proved about them.
    ///
    /// Tier decides, not the caller. A candidate handed in among the offerable
    /// ones that turns out to be unproven or protected is moved across rather
    /// than trusted, so a mistake upstream cannot put an unsafe entry within
    /// reach of the cursor.
    pub fn new(candidates: Vec<Candidate>, rejected: Vec<Rejected>) -> Self {
        let (selectable, unsafe_ones): (Vec<_>, Vec<_>) = candidates
            .into_iter()
            .partition(|c| c.safety.is_selectable());

        let mut blocked: Vec<Blocked> = unsafe_ones
            .into_iter()
            .map(|c| Blocked {
                reason: describe(&c.safety),
                path: c.path,
            })
            .collect();
        blocked.extend(rejected.into_iter().map(|r| Blocked {
            path: r.path,
            reason: r.because,
        }));

        // Largest first: this is the screen where what to remove is decided,
        // and it should open on what is worst rather than on whatever sorts
        // first under `/Users`. The blocked list keeps the order it came in;
        // there is nothing to act on there.
        let mut screen = Self {
            selectable,
            blocked,
            order: Order::Size,
            descending: true,
            cursor: 0,
            marked: BTreeSet::new(),
            cleared: None,
            project: None,
            widened: false,
            elsewhere: Vec::new(),
        };
        screen.apply_sort();
        screen
    }

    /// Say where something can be rebuilt, for the body to name when there is
    /// nothing here.
    pub fn set_elsewhere(&mut self, lines: Vec<String>) {
        self.elsewhere = lines;
    }

    /// Back to the view the screen opens on: largest first, in the project that
    /// was opened, the cursor at the top. Only the view: marks are not its.
    pub fn reset_view(&mut self) {
        self.order = Order::Size;
        self.descending = true;
        self.widened = false;
        self.apply_sort();
        self.cursor = 0;
    }

    /// The order in force, in words: what `r` puts back, said after it did.
    pub fn ordering(&self) -> String {
        format!("{}, {}", self.order.name(), self.order.way(self.descending))
    }

    /// Let go of what `c` would restore. Marks belong to one visit: coming back
    /// to the screen later must not resurrect a selection from before.
    pub fn forget_cleared(&mut self) {
        self.cleared = None;
    }

    /// Every offerable entry of every project, whatever the screen shows.
    pub fn selectable(&self) -> &[Candidate] {
        &self.selectable
    }

    /// Every entry held back, of every project.
    pub fn blocked(&self) -> &[Blocked] {
        &self.blocked
    }

    /// The scope in force, when it is a project: the screen's own words for it.
    pub fn scope_name(&self) -> Option<&str> {
        self.scope().map(|s| s.name.as_str())
    }

    fn scope(&self) -> Option<&Scope> {
        self.project.as_ref().filter(|_| !self.widened)
    }

    /// Show only what belongs to the project at `root`. `inner` are the roots of
    /// projects nested inside it, which keep their own entries.
    pub fn scope_to(&mut self, root: &Path, name: &str, inner: Vec<PathBuf>) {
        self.project = Some(Scope {
            root: root.to_path_buf(),
            name: name.to_string(),
            inner,
        });
        self.widened = false;
        self.cursor = 0;
    }

    /// Widen to every project, or narrow back to the one that was opened. The
    /// cursor stays on its entry when the new scope still shows it. `false`
    /// when no project was ever opened, so there is nothing to widen from.
    pub fn toggle_scope(&mut self) -> bool {
        if self.project.is_none() {
            return false;
        }
        let under = self.selected().map(|c| c.path.clone());
        self.widened = !self.widened;
        self.cursor = under.and_then(|p| self.position_of(&p)).unwrap_or(0);
        true
    }

    /// Whether the screen shows every project although one was opened.
    pub fn is_widened(&self) -> bool {
        self.project.is_some() && self.widened
    }

    /// Indices into `selectable` of what the scope shows, in the screen's order.
    fn view(&self) -> Vec<usize> {
        let scope = self.scope();
        (0..self.selectable.len())
            .filter(|&i| scope.is_none_or(|s| s.owns(&self.selectable[i].path)))
            .collect()
    }

    fn position_of(&self, path: &Path) -> Option<usize> {
        self.visible().iter().position(|c| c.path == path)
    }

    /// The offerable entries the scope shows, in the order the screen shows them.
    pub fn visible(&self) -> Vec<&Candidate> {
        self.view()
            .into_iter()
            .map(|i| &self.selectable[i])
            .collect()
    }

    /// The entries held back that the scope shows.
    pub fn visible_blocked(&self) -> Vec<&Blocked> {
        let scope = self.scope();
        self.blocked
            .iter()
            .filter(|b| scope.is_none_or(|s| s.owns(&b.path)))
            .collect()
    }

    /// Index of the entry under the cursor, among the entries shown.
    pub fn cursor(&self) -> usize {
        self.cursor
    }

    /// The entry under the cursor, which is always one that may be purged.
    pub fn selected(&self) -> Option<&Candidate> {
        let at = *self.view().get(self.cursor)?;
        self.selectable.get(at)
    }

    /// What the marks say about the project at `root`.
    pub fn tally(&self, root: &Path, inner: &[PathBuf]) -> Tally {
        let scope = Scope {
            root: root.to_path_buf(),
            name: String::new(),
            inner: inner.to_vec(),
        };
        let mut tally = Tally::default();
        for c in self.selectable.iter().filter(|c| scope.owns(&c.path)) {
            tally.offered.0 += 1;
            tally.offered.1 += c.bytes;
            if self.marked.contains(&c.path) {
                tally.marked.0 += 1;
                tally.marked.1 += c.bytes;
            }
        }
        tally
    }

    /// What the project at `root` holds back, by reason, with how many each.
    pub fn held_back(&self, root: &Path, inner: &[PathBuf]) -> Vec<(&str, usize)> {
        let scope = Scope {
            root: root.to_path_buf(),
            name: String::new(),
            inner: inner.to_vec(),
        };
        let mut reasons: Vec<(&str, usize)> = Vec::new();
        for b in self.blocked.iter().filter(|b| scope.owns(&b.path)) {
            match reasons.iter_mut().find(|(r, _)| *r == b.reason) {
                Some((_, n)) => *n += 1,
                None => reasons.push((&b.reason, 1)),
            }
        }
        reasons
    }

    /// `Space` on a project of the table: mark everything it offers, or, when
    /// all of it is marked already, unmark it. Only that project's entries move.
    pub fn toggle_project(&mut self, root: &Path, inner: &[PathBuf]) -> ProjectMarking {
        let scope = Scope {
            root: root.to_path_buf(),
            name: String::new(),
            inner: inner.to_vec(),
        };
        let own: Vec<(PathBuf, u64)> = self
            .selectable
            .iter()
            .filter(|c| scope.owns(&c.path))
            .map(|c| (c.path.clone(), c.bytes))
            .collect();
        if own.is_empty() {
            return ProjectMarking::NothingOffered;
        }
        self.cleared = None;
        let (n, bytes) = (own.len(), own.iter().map(|(_, b)| b).sum());
        if own.iter().all(|(p, _)| self.marked.contains(p)) {
            for (p, _) in &own {
                self.marked.remove(p);
            }
            ProjectMarking::Unmarked(n, bytes)
        } else {
            self.marked.extend(own.into_iter().map(|(p, _)| p));
            ProjectMarking::Marked(n, bytes)
        }
    }

    /// Move the cursor to the first offerable entry under `root`, and say where
    /// it went. Nothing offerable under it leaves the cursor where it was.
    ///
    /// Found by path, in the order the screen shows now, and searched in
    /// `selectable` alone: a blocked entry has no index to land on.
    pub fn focus(&mut self, root: &Path) -> Option<usize> {
        let at = self
            .visible()
            .iter()
            .position(|c| c.path.starts_with(root))?;
        self.cursor = at;
        Some(at)
    }

    /// Everything marked for the plan, in the order the screen shows it.
    pub fn marked(&self) -> Vec<&Candidate> {
        self.selectable
            .iter()
            .filter(|c| self.marked.contains(&c.path))
            .collect()
    }

    /// Apply `key`, and say what it did to the marks, when it did anything. `rows`
    /// is what the last frame gave this screen, which is what a page key moves by.
    pub fn press(&mut self, key: Key, rows: usize) -> Option<Marking> {
        // What the keys act on is what the screen shows, so the marking keys
        // cannot reach a row the user cannot see.
        let view = self.view();
        let last = view.len().saturating_sub(1);
        match key {
            Key::Up => self.cursor = self.cursor.saturating_sub(1),
            Key::Down => self.cursor = (self.cursor + 1).min(last),
            Key::Top => self.cursor = 0,
            Key::Bottom => self.cursor = last,
            Key::PageUp => self.cursor = self.cursor.saturating_sub(page(rows)),
            Key::PageDown => self.cursor = (self.cursor + page(rows)).min(last),
            Key::Toggle => {
                let Some(c) = view.get(self.cursor).map(|&i| &self.selectable[i]) else {
                    return Some(Marking::NothingToMark);
                };
                let entry = (c.path.clone(), c.bytes);
                self.cleared = None;
                return Some(if self.marked.insert(c.path.clone()) {
                    Marking::Marked(entry.0, entry.1)
                } else {
                    self.marked.remove(&c.path);
                    Marking::Unmarked(entry.0, entry.1)
                });
            }
            Key::MarkAll => {
                if view.is_empty() {
                    return Some(Marking::NothingToMark);
                }
                self.cleared = None;
                let shown = || view.iter().map(|&i| &self.selectable[i]);
                let bytes = shown().map(|c| c.bytes).sum();
                let paths: Vec<PathBuf> = shown().map(|c| c.path.clone()).collect();
                self.marked.extend(paths);
                return Some(Marking::MarkedAll(view.len(), bytes));
            }
            Key::ClearMarks => {
                let shown = |p: &PathBuf| view.iter().any(|&i| &self.selectable[i].path == p);
                let here: BTreeSet<PathBuf> =
                    self.marked.iter().filter(|p| shown(p)).cloned().collect();
                if here.is_empty() {
                    // Restored through `selectable`, never from the stash
                    // directly: a path that is not offerable is not marked. And
                    // only what the screen shows: what was cleared in another
                    // scope stays in the stash for that scope.
                    let stash = self.cleared.take().unwrap_or_default();
                    let (back, rest): (BTreeSet<PathBuf>, BTreeSet<PathBuf>) = stash
                        .into_iter()
                        .filter(|p| self.selectable.iter().any(|c| &c.path == p))
                        .partition(|p| shown(p));
                    let (n, bytes) = self.count(&back);
                    self.cleared = (!rest.is_empty()).then_some(rest);
                    self.marked.extend(back);
                    return Some(if n == 0 {
                        Marking::Cleared(0, 0)
                    } else {
                        Marking::Restored(n, bytes)
                    });
                }
                let (n, bytes) = self.count(&here);
                for p in &here {
                    self.marked.remove(p);
                }
                self.cleared = Some(here);
                return Some(Marking::Cleared(n, bytes));
            }
            Key::Sort(order) => self.sort_by(order),
        }
        None
    }

    /// How many of `paths` are offerable entries, and their bytes.
    fn count(&self, paths: &BTreeSet<PathBuf>) -> (usize, u64) {
        let found: Vec<&Candidate> = self
            .selectable
            .iter()
            .filter(|c| paths.contains(&c.path))
            .collect();
        (found.len(), found.iter().map(|c| c.bytes).sum())
    }

    /// Order by `order`, reversing if it is already the one in use.
    ///
    /// Coming back to an order later starts from its own default again, as
    /// `Projects::sort_by` does, so a digit always means the same thing the
    /// first time it is pressed.
    fn sort_by(&mut self, order: Order) {
        if self.order == order {
            self.descending = !self.descending;
        } else {
            self.order = order;
            self.descending = order.starts_descending();
        }
        // The cursor follows its entry, as the marks do: the user was looking
        // at a directory, not at a row number.
        let under = self.selected().map(|c| c.path.clone());
        self.apply_sort();
        self.cursor = under.and_then(|path| self.position_of(&path)).unwrap_or(0);
    }

    /// Ties fall back to the path whichever way the order runs, so two entries
    /// of one size stand in the same relation to each other in both directions
    /// and the order is the same on every run.
    fn apply_sort(&mut self) {
        let (order, descending) = (self.order, self.descending);
        self.selectable.sort_by(|a, b| {
            let primary = match order {
                Order::Path => a.path.cmp(&b.path),
                Order::Size => a.bytes.cmp(&b.bytes),
                Order::Kind => kind(&a.path).cmp(kind(&b.path)),
            };
            if descending {
                primary.reverse()
            } else {
                primary
            }
            .then_with(|| a.path.cmp(&b.path))
        });
    }

    /// What the view is, in one line that fits `width`: the sort in words with
    /// its arrow, the scope with how many entries it shows, and the key that
    /// puts it all back. And whether it is anything but the view it opens on.
    fn view_bar(&self, shown: usize, width: usize) -> (String, bool) {
        let arrow = if self.descending { "▼" } else { "▲" };
        let (name, way) = (self.order.name(), self.order.way(self.descending));
        let total = self.selectable.len();
        let entries = |n: usize| if n == 1 { "entry" } else { "entries" };
        let (scope, short) = match self.scope() {
            Some(_) => (
                format!("this project ({shown} of {total} {})", entries(total)),
                format!("this project ({shown} of {total})"),
            ),
            None => (
                format!("all projects ({shown} {})", entries(shown)),
                format!("all projects ({shown})"),
            ),
        };
        let tries = [
            format!("view  sort {name} {arrow} {way} · scope {scope} · r reset"),
            format!("view  sort {name} {arrow} · {short} · r reset"),
        ];
        let line = tries
            .iter()
            .find(|t| t.chars().count() <= width)
            .unwrap_or(&tries[1]);
        let default = self.order == Order::Size && self.descending && !self.is_widened();
        (elide_tail(line, width), !default)
    }

    /// What a list with nothing to show says in its place: what is true, where
    /// something is, and the keys that get there.
    fn empty_body(&self) -> Vec<String> {
        let mut lines = Vec::new();
        match self.scope() {
            Some(s) => lines.push(format!("Nothing to rebuild in {}.", s.name)),
            None => lines.push("Nothing can be rebuilt in any project.".to_string()),
        }
        lines.extend(self.elsewhere.iter().cloned());
        lines.push(match (self.scope(), self.project.is_some()) {
            (Some(_), _) => format!("{KEYS_LEAD}Tab all projects · r reset view · Esc projects"),
            (None, true) => format!("{KEYS_LEAD}Tab this project · r reset view · Esc projects"),
            (None, false) => format!("{KEYS_LEAD}Esc projects"),
        });
        lines
    }

    pub fn render(&self, theme: &Theme, area: Rect, buf: &mut Buffer) {
        let left = area.x + 1;

        // Columns are measured from the area and from their own content, never
        // fixed. Real paths run far longer than any fixture suggests, and a
        // description written at a fixed offset lands in the middle of one,
        // leaving a row that reads as a path which does not exist.
        let width = area.width.saturating_sub(2) as usize;
        let mut y = area.y;

        // The window follows the cursor, so a key that moves it always moves
        // something on screen. The blocked list below gets whatever is left.
        let shown = self.visible();
        let (bar, lit) = self.view_bar(shown.len(), width);
        buf.set_string(left, y, bar, if lit { theme.head } else { theme.text });
        y += 2;
        let height = area.height.saturating_sub(CHROME as u16) as usize;
        let start = window_start(self.cursor, shown.len(), height);
        let visible = &shown[start..(start + height).min(shown.len())];

        let order = self.order.words(self.descending);
        let at = showing(start, visible.len(), shown.len());
        let heading = match (&self.project, self.scope()) {
            (None, _) => format!("Can be rebuilt  ({})  {order}  {at}", shown.len()),
            (Some(_), scope) => {
                let name = scope.map_or("All projects", |s| s.name.as_str());
                if shown.is_empty() {
                    format!("{name} · nothing can be rebuilt here")
                } else {
                    let bytes: u64 = shown.iter().map(|c| c.bytes).sum();
                    format!(
                        "{name} · {} can be rebuilt · {}  {order}  {at}",
                        shown.len(),
                        human(bytes)
                    )
                }
            }
        };
        y = section(buf, theme, left, y, width, &heading);
        if shown.is_empty() {
            for line in self.empty_body() {
                if y >= area.bottom() {
                    break;
                }
                let style = if line.starts_with(KEYS_LEAD) {
                    theme.muted
                } else {
                    theme.text
                };
                buf.set_string(left, y, elide_tail(&line, width), style);
                y += 1;
            }
        }

        let descriptions: Vec<String> = shown.iter().map(|c| describe(&c.safety)).collect();
        let longest = widest(shown.iter().map(|c| c.path.as_path()));
        let (path_w, desc_x, desc_w) = columns(left, width, 18, longest, &descriptions);
        for (i, c) in (start..).zip(visible) {
            let mark = if self.marked.contains(&c.path) {
                'x'
            } else {
                ' '
            };
            put(
                buf,
                left,
                y,
                &[
                    (
                        format!("[{mark}]"),
                        if mark == 'x' { theme.head } else { theme.text },
                    ),
                    (" ".to_string(), theme.text),
                    (c.safety.symbol().to_string(), theme.violet),
                    (format!(" {:>10}", human(c.bytes)), theme.size(c.bytes)),
                ],
            );
            buf.set_string(
                left + 18,
                y,
                elide_path(&c.path.display().to_string(), path_w),
                theme.text,
            );
            buf.set_string(desc_x, y, elide_tail(&descriptions[i], desc_w), theme.safe);
            // Across the whole row, the command included: it is part of what
            // the cursor is on.
            if i == self.cursor {
                buf.set_style(Rect::new(area.x, y, area.width, 1), theme.selected);
            }
            y += 1;
        }

        let blocked = self.visible_blocked();
        if blocked.is_empty() {
            return;
        }
        y += 1;
        if y >= area.bottom() {
            return;
        }
        // Shown, explained, and out of reach. A user who cannot see why a
        // directory is missing has no way to act on it, and silence reads as
        // there having been nothing there.
        y = section(
            buf,
            theme,
            left,
            y,
            width,
            &format!("Not offered  ({})", blocked.len()),
        );

        // The reason is the only thing on a blocked row that can be acted on,
        // so it is sized first and the path takes what is left.
        let reasons: Vec<String> = blocked.iter().map(|b| b.reason.clone()).collect();
        let longest = widest(blocked.iter().map(|b| b.path.as_path()));
        let (blocked_path_w, reason_x, reason_w) = columns(left, width, 4, longest, &reasons);
        for (b, reason) in blocked.iter().zip(&reasons) {
            if y >= area.bottom() {
                return;
            }
            buf.set_string(left, y, "  !", theme.blocked);
            buf.set_string(
                left + 4,
                y,
                elide_path(&b.path.display().to_string(), blocked_path_w),
                theme.blocked,
            );
            buf.set_string(reason_x, y, elide_tail(reason, reason_w), theme.blocked);
            y += 1;
        }
    }
}
