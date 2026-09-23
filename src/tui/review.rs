//! Reading the plan before it can be carried out.
//!
//! The screen owns no candidates of its own: it is a window onto the plan the
//! router is holding, and every draw reads it again. A copy taken when the
//! screen was built would keep showing a set the plan no longer holds, which is
//! the one thing review must never do.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};

use super::keymap::Motion;
use super::row::{columns, describe, elide_path};
use crate::bytes::human;
use crate::safety::{Candidate, Plan, Reviewed};

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
const CHROME: usize = 4;

impl Review {
    pub fn new() -> Self {
        Self::default()
    }

    /// The rows on screen, always a full window where there are enough items.
    pub fn visible<'a>(&self, plan: &'a Plan<Reviewed>, height: usize) -> &'a [Candidate] {
        let items = plan.items();
        let start = self.top.min(items.len().saturating_sub(height));
        &items[start..(start + height).min(items.len())]
    }

    pub fn scroll(&mut self, motion: Motion, plan: &Plan<Reviewed>, height: usize) {
        let last = plan.items().len().saturating_sub(height);
        self.top = match motion {
            Motion::Up => self.top.saturating_sub(1),
            Motion::Down => (self.top + 1).min(last),
            Motion::Top => 0,
            Motion::Bottom => last,
            Motion::PageUp => self.top.saturating_sub(height),
            Motion::PageDown => (self.top + height).min(last),
        };
    }

    pub fn render(&self, plan: &Plan<Reviewed>, area: Rect, buf: &mut Buffer) {
        let left = area.x + 1;
        let head = Style::new().add_modifier(Modifier::BOLD);
        let dim = Style::new().fg(Color::DarkGray);
        let width = area.width.saturating_sub(2) as usize;

        buf.set_string(
            left,
            area.y,
            format!(
                "The plan  ({} items, {})",
                plan.items().len(),
                human(plan.total_bytes())
            ),
            head,
        );

        let rows = (area.height as usize).saturating_sub(CHROME);
        let visible = self.visible(plan, rows);

        // Every row carries the command that brings it back. Nothing without
        // one can be in a plan at all — `Plan::<Draft>::add` refuses the tiers
        // that have none — so a blank here is a bug rather than a row to draw.
        let commands: Vec<String> = visible.iter().map(|c| describe(&c.safety)).collect();
        let (path_w, command_x) = columns(left, width, 14, &commands);

        let mut y = area.y + 2;
        for (c, command) in visible.iter().zip(&commands) {
            if y >= area.bottom() {
                break;
            }
            buf.set_string(
                left,
                y,
                format!("{} {:>10}", c.safety.symbol(), human(c.bytes)),
                Style::new(),
            );
            buf.set_string(
                left + 14,
                y,
                elide_path(&c.path.display().to_string(), path_w),
                Style::new(),
            );
            buf.set_string(command_x, y, command, dim);
            y += 1;
        }

        // Said once, at the bottom, where the eye lands after the list: the
        // right-hand column above is a promise, and this is what it means.
        if y < area.bottom() {
            buf.set_string(
                left,
                y + 1,
                "Each line names the command that rebuilds it. Esc to change the plan.",
                dim,
            );
        }
    }
}
