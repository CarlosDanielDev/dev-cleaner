use std::collections::BTreeSet;
use std::ffi::OsStr;
use std::path::{Path, PathBuf};

use super::palette::{BLOCKED, DEFAULT, HEAD, SAFE, SELECTED};
use super::row::{columns, describe, elide_path, elide_tail};
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

/// How far a page key moves.
const PAGE: usize = 10;

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
        };
        screen.apply_sort();
        screen
    }

    pub fn selectable(&self) -> &[Candidate] {
        &self.selectable
    }

    pub fn blocked(&self) -> &[Blocked] {
        &self.blocked
    }

    /// The entry under the cursor, which is always one that may be purged.
    pub fn selected(&self) -> Option<&Candidate> {
        self.selectable.get(self.cursor)
    }

    /// Everything marked for the plan, in the order the screen shows it.
    pub fn marked(&self) -> Vec<&Candidate> {
        self.selectable
            .iter()
            .filter(|c| self.marked.contains(&c.path))
            .collect()
    }

    pub fn press(&mut self, key: Key) {
        let last = self.selectable.len().saturating_sub(1);
        match key {
            Key::Up => self.cursor = self.cursor.saturating_sub(1),
            Key::Down => self.cursor = (self.cursor + 1).min(last),
            Key::Top => self.cursor = 0,
            Key::Bottom => self.cursor = last,
            Key::PageUp => self.cursor = self.cursor.saturating_sub(PAGE),
            Key::PageDown => self.cursor = (self.cursor + PAGE).min(last),
            Key::Toggle => {
                if let Some(c) = self.selectable.get(self.cursor)
                    && !self.marked.insert(c.path.clone())
                {
                    self.marked.remove(&c.path);
                }
            }
            Key::MarkAll => self.marked = self.selectable.iter().map(|c| c.path.clone()).collect(),
            Key::ClearMarks => self.marked.clear(),
            Key::Sort(order) => self.sort_by(order),
        }
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
        self.cursor = under
            .and_then(|path| self.selectable.iter().position(|c| c.path == path))
            .unwrap_or(0);
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

    pub fn render(&self, area: Rect, buf: &mut Buffer) {
        let left = area.x + 1;

        // Columns are measured from the area and from their own content, never
        // fixed. Real paths run far longer than any fixture suggests, and a
        // description written at a fixed offset lands in the middle of one,
        // leaving a row that reads as a path which does not exist.
        let width = area.width.saturating_sub(2) as usize;
        let mut y = area.y;

        // The window follows the cursor, so a key that moves it always moves
        // something on screen. The blocked list below gets whatever is left.
        let height = area.height.saturating_sub(1) as usize;
        let start = window_start(self.cursor, self.selectable.len(), height);
        let visible = &self.selectable[start..(start + height).min(self.selectable.len())];

        buf.set_string(
            left,
            y,
            format!(
                "Can be rebuilt  ({})  {}  {}",
                self.selectable.len(),
                self.order.words(self.descending),
                showing(start, visible.len(), self.selectable.len())
            ),
            HEAD,
        );
        y += 1;

        let descriptions: Vec<String> = self
            .selectable
            .iter()
            .map(|c| describe(&c.safety))
            .collect();
        let (path_w, desc_x, desc_w) = columns(left, width, 18, &descriptions);
        for (i, c) in (start..).zip(visible) {
            let mark = if self.marked.contains(&c.path) {
                'x'
            } else {
                ' '
            };
            buf.set_string(
                left,
                y,
                format!("[{mark}] {} {:>10}", c.safety.symbol(), human(c.bytes)),
                DEFAULT,
            );
            buf.set_string(
                left + 18,
                y,
                elide_path(&c.path.display().to_string(), path_w),
                DEFAULT,
            );
            buf.set_string(desc_x, y, elide_tail(&descriptions[i], desc_w), SAFE);
            // Across the whole row, the command included: it is part of what
            // the cursor is on.
            if i == self.cursor {
                buf.set_style(Rect::new(area.x, y, area.width, 1), SELECTED);
            }
            y += 1;
        }

        if self.blocked.is_empty() {
            return;
        }
        y += 1;
        if y >= area.bottom() {
            return;
        }
        // Shown, explained, and out of reach. A user who cannot see why a
        // directory is missing has no way to act on it, and silence reads as
        // there having been nothing there.
        buf.set_string(
            left,
            y,
            format!("Not offered  ({})", self.blocked.len()),
            HEAD,
        );
        y += 1;

        // The reason is the only thing on a blocked row that can be acted on,
        // so it is sized first and the path takes what is left.
        let reasons: Vec<String> = self.blocked.iter().map(|b| b.reason.clone()).collect();
        let (blocked_path_w, reason_x, reason_w) = columns(left, width, 4, &reasons);
        for (b, reason) in self.blocked.iter().zip(&reasons) {
            if y >= area.bottom() {
                return;
            }
            buf.set_string(left, y, "  !", BLOCKED);
            buf.set_string(
                left + 4,
                y,
                elide_path(&b.path.display().to_string(), blocked_path_w),
                BLOCKED,
            );
            buf.set_string(reason_x, y, elide_tail(reason, reason_w), BLOCKED);
            y += 1;
        }
    }
}
