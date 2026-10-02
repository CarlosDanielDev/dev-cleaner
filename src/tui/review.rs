//! Reading the plan before it can be carried out.
//!
//! The screen owns no candidates of its own: it is a window onto the plan the
//! router is holding, and every draw reads it again. A copy taken when the
//! screen was built would keep showing a set the plan no longer holds, which is
//! the one thing review must never do.

use super::keymap::Motion;
use super::palette::Theme;
use super::row::{heading, plan_rows};
use super::showing;
use crate::bytes::human;
use crate::safety::{Candidate, Plan, Reviewed};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Modifier;

/// Where the list has been scrolled to.
///
/// Taking `Plan<Reviewed>` on every call rather than holding one is what makes
/// the type do the work: a draft cannot be reviewed, because `Plan<Draft>` will
/// not go through this door.
#[derive(Debug, Default)]
pub struct Review {
    top: usize,
}

/// Rows of heading above the list, and of footer below it.
pub(super) const CHROME: usize = 4;

impl Review {
    pub fn new() -> Self {
        Self::default()
    }

    /// The rows on screen, always a full window where there are enough items.
    pub fn visible<'a>(&self, plan: &'a Plan<Reviewed>, height: usize) -> &'a [Candidate] {
        let items = plan.items();
        let start = self.start(items.len(), height);
        &items[start..(start + height).min(items.len())]
    }

    /// First row on screen, for a list of `len` in a window of `height`.
    pub(super) fn offset(&self, len: usize, height: usize) -> usize {
        self.start(len, height)
    }

    fn start(&self, len: usize, height: usize) -> usize {
        self.top.min(len.saturating_sub(height))
    }

    /// Scroll the list, which is `height` rows tall.
    ///
    /// There is no cursor: nothing here acts on a single row, and a cursor that
    /// selects nothing is a promise the screen does not keep. What moves is the
    /// window, and the heading says where it is.
    pub fn scroll(&mut self, motion: Motion, plan: &Plan<Reviewed>, height: usize) {
        let last = plan.items().len().saturating_sub(height);
        // Clamped first, so a window that grew since the last scroll does not
        // leave `Up` spending presses on rows that are already in view.
        self.top = self.top.min(last);
        self.top = match motion {
            Motion::Up => self.top.saturating_sub(1),
            Motion::Down => (self.top + 1).min(last),
            Motion::Top => 0,
            Motion::Bottom => last,
            Motion::PageUp => self.top.saturating_sub(height),
            Motion::PageDown => (self.top + height).min(last),
        };
    }

    pub fn render(&self, theme: &Theme, plan: &Plan<Reviewed>, area: Rect, buf: &mut Buffer) {
        let left = area.x + 1;
        let width = area.width.saturating_sub(2) as usize;

        let rows = (area.height as usize).saturating_sub(CHROME);
        let visible = self.visible(plan, rows);
        let len = plan.items().len();

        heading(
            buf,
            theme,
            left,
            area.y,
            width,
            &[
                (format!("The plan  ({len} items, "), theme.head),
                (
                    human(plan.total_bytes()),
                    theme.size(plan.total_bytes()).add_modifier(Modifier::BOLD),
                ),
                (
                    format!(")  {}", showing(self.start(len, rows), visible.len(), len)),
                    theme.head,
                ),
            ],
        );

        let rows: Vec<&Candidate> = visible.iter().collect();
        let y = plan_rows(theme, &rows, left, width, area.y + 2, area.bottom(), buf);

        // Said once, at the bottom, where the eye lands after the list: the
        // right-hand column above is a promise, and this is what it means.
        if y < area.bottom() {
            buf.set_string(
                left,
                y + 1,
                "Each line names the command that rebuilds it. Esc to change the plan.",
                theme.muted,
            );
        }
    }
}
