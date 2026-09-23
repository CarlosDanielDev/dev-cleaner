use std::collections::BTreeSet;
use std::path::PathBuf;

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};

use super::row::{columns, describe, elide_path};
use crate::bytes::human;
use crate::safety::{Candidate, Rejected};

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
        ]
    }
}

/// How far a page key moves.
const PAGE: usize = 10;

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
    cursor: usize,
    marked: BTreeSet<usize>,
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

        Self {
            selectable,
            blocked,
            cursor: 0,
            marked: BTreeSet::new(),
        }
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

    /// Everything marked for the plan.
    pub fn marked(&self) -> Vec<&Candidate> {
        self.marked
            .iter()
            .filter_map(|i| self.selectable.get(*i))
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
                if self.cursor < self.selectable.len() && !self.marked.insert(self.cursor) {
                    self.marked.remove(&self.cursor);
                }
            }
            Key::MarkAll => self.marked = (0..self.selectable.len()).collect(),
            Key::ClearMarks => self.marked.clear(),
        }
    }

    pub fn render(&self, area: Rect, buf: &mut Buffer) {
        let left = area.x + 1;
        let head = Style::new().add_modifier(Modifier::BOLD);
        let dim = Style::new().fg(Color::DarkGray);

        // Columns are measured from the area and from their own content, never
        // fixed. Real paths run far longer than any fixture suggests, and a
        // description written at a fixed offset lands in the middle of one,
        // leaving a row that reads as a path which does not exist.
        let width = area.width.saturating_sub(2) as usize;
        let mut y = area.y;

        buf.set_string(
            left,
            y,
            format!("Can be rebuilt  ({})", self.selectable.len()),
            head,
        );
        y += 1;

        let descriptions: Vec<String> = self
            .selectable
            .iter()
            .map(|c| describe(&c.safety))
            .collect();
        let (path_w, desc_x) = columns(left, width, 18, &descriptions);
        for (i, c) in self.selectable.iter().enumerate() {
            if y >= area.bottom() {
                return;
            }
            let style = if i == self.cursor {
                Style::new().add_modifier(Modifier::REVERSED)
            } else {
                Style::new()
            };
            let mark = if self.marked.contains(&i) { 'x' } else { ' ' };
            buf.set_string(
                left,
                y,
                format!("[{mark}] {} {:>10}", c.safety.symbol(), human(c.bytes)),
                style,
            );
            buf.set_string(
                left + 18,
                y,
                elide_path(&c.path.display().to_string(), path_w),
                style,
            );
            buf.set_string(desc_x, y, &descriptions[i], dim);
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
            head,
        );
        y += 1;

        // The reason is the only thing on a blocked row that can be acted on,
        // so it is sized first and the path takes what is left.
        let reasons: Vec<String> = self.blocked.iter().map(|b| b.reason.clone()).collect();
        let (blocked_path_w, reason_x) = columns(left, width, 4, &reasons);
        for (b, reason) in self.blocked.iter().zip(&reasons) {
            if y >= area.bottom() {
                return;
            }
            buf.set_string(left, y, "  !", dim);
            buf.set_string(
                left + 4,
                y,
                elide_path(&b.path.display().to_string(), blocked_path_w),
                dim,
            );
            buf.set_string(reason_x, y, reason, dim);
            y += 1;
        }
    }
}
