//! The last screen: holding a key long enough to mean it.
//!
//! The screen holds no plan and cannot reach one. Arming is a fact about how
//! long a key has been down, and the caller turns that fact into `App::confirm`
//! — so releasing early cancels by nothing ever having been called. There is no
//! partially confirmed state to unwind, because there is no state but a stopwatch.

use std::time::Duration;

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};

use super::keymap::PURGE;
use crate::bytes::human;
use crate::safety::{Plan, Reviewed};

/// Gauge cells, told apart by shape rather than by colour.
const FILLED: char = '█';
const EMPTY: char = '·';

/// The hold-to-arm gauge.
#[derive(Debug, Default)]
pub struct Confirm {
    held: Duration,
    armed: bool,
}

impl Confirm {
    /// How long the key must be down.
    ///
    /// Long enough to be a decision, short enough not to feel broken. A slip
    /// lasts one key repeat; this is roughly thirty of them.
    pub const HOLD: Duration = Duration::from_millis(1500);

    /// The most a single event may contribute.
    ///
    /// A stalled frame, a laptop waking, a debugger stopping the world: any of
    /// them hands the screen a delta longer than the whole threshold. Without a
    /// cap, one keypress arriving after a stall would purge — which is the slip
    /// holding a key exists to prevent. Capped, arming always takes many events,
    /// and many events only come from a key that is still down.
    const MAX_STEP: Duration = Duration::from_millis(50);

    pub fn new() -> Self {
        Self::default()
    }

    /// Account for `delta` more of the key being held.
    ///
    /// Returns true exactly once: on the event that completes the hold. The
    /// caller confirms on that, so holding past the threshold cannot purge
    /// twice.
    pub fn hold(&mut self, delta: Duration) -> bool {
        if self.armed {
            return false;
        }
        self.held += delta.min(Self::MAX_STEP);
        self.armed = self.held >= Self::HOLD;
        self.armed
    }

    /// The key came up. Whatever had been held counts for nothing.
    ///
    /// ponytail: a plain terminal reports no key-release event, so the screen is
    /// told about the release rather than seeing it. The terminal loop (#54)
    /// calls this when a repeat fails to arrive in time; the grace window is its
    /// decision, not this screen's.
    pub fn release(&mut self) {
        *self = Self::new();
    }

    pub fn is_armed(&self) -> bool {
        self.armed
    }

    /// How far through the hold, from 0.0 to 1.0.
    pub fn progress(&self) -> f32 {
        (self.held.as_secs_f32() / Self::HOLD.as_secs_f32()).min(1.0)
    }

    pub fn render(&self, plan: &Plan<Reviewed>, area: Rect, buf: &mut Buffer) {
        let left = area.x + 1;
        let head = Style::new().add_modifier(Modifier::BOLD);
        let dim = Style::new().fg(Color::DarkGray);
        let mut y = area.y;

        buf.set_string(
            left,
            y,
            format!(
                "Hold  {PURGE}  to purge {} items, {}",
                plan.items().len(),
                human(plan.total_bytes())
            ),
            head,
        );
        y += 2;

        let width = area.width.saturating_sub(2).max(10) as usize;
        let filled = (width as f32 * self.progress()).round() as usize;
        let bar: String = std::iter::repeat_n(FILLED, filled.min(width))
            .chain(std::iter::repeat_n(EMPTY, width.saturating_sub(filled)))
            .collect();
        buf.set_string(left, y, bar, Style::new().fg(Color::Cyan));
        y += 2;

        // What is about to happen and how to stop it, side by side. A screen
        // that only says how to proceed reads as having no way out.
        buf.set_string(
            left,
            y,
            "Everything in the plan goes to the Trash, with a manifest saying how \
             to put it back.",
            dim,
        );
        buf.set_string(left, y + 1, "Release the key to cancel.", dim);
    }
}
