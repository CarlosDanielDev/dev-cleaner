//! The last screen: holding a key long enough to mean it.
//!
//! The screen holds no plan and cannot reach one. Arming is a fact about how
//! long a key has been down, and the caller turns that fact into `App::confirm`
//! — so releasing early cancels by nothing ever having been called. There is no
//! partially confirmed state to unwind, because there is no state but a stopwatch.

use std::time::Duration;

use super::keymap::PURGE;
use super::palette::{BLOCKED, DANGER, DEFAULT, HEAD};
use crate::bytes::human;
use crate::safety::{Plan, Reviewed};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;

/// Gauge cells, told apart by shape rather than by colour.
const FILLED: char = '█';
const EMPTY: char = '·';

/// The hold-to-arm gauge.
#[derive(Debug, Default)]
pub struct Confirm {
    held: Duration,
    presses: u32,
    armed: bool,
    /// A hold was under way and stopped short. Shown until the next press.
    lapsed: bool,
}

impl Confirm {
    /// How long the key must be down.
    ///
    /// Long enough to be a decision, short enough not to feel broken. A slip
    /// lasts one key repeat; this is more than a dozen at the default macOS rate.
    pub const HOLD: Duration = Duration::from_millis(1500);

    /// The longest gap between two events of one hold.
    ///
    /// A plain terminal reports no key-release event, so a hold that stopped
    /// shows up as a repeat that never arrived. The window has to clear the gap
    /// before the *first* repeat, which macOS defaults to around 375 ms, and the
    /// terminal loop releases the hold once the key has been quiet this long.
    ///
    /// ponytail: a constant, not the terminal's own repeat rate, which no
    /// terminal reports. A key whose first repeat or repeat interval is slower
    /// than this cannot fill the gauge, and the lapse notice names the shell's
    /// route for that user. Read real release events from the kitty keyboard
    /// protocol if that ever turns up.
    pub const GRACE: Duration = Duration::from_millis(600);

    /// The fewest events that can make a hold.
    ///
    /// The gauge measures the clock, so a few presses under the grace window
    /// could otherwise add up to the threshold. Ten is two thirds of what the
    /// default macOS repeat delivers in `HOLD`, and every repeat rate macOS
    /// offers below the grace window reaches it within about three seconds.
    const MIN_PRESSES: u32 = 10;

    pub fn new() -> Self {
        Self::default()
    }

    /// Account for another event of the key, `delta` after the one before.
    ///
    /// The gauge is wall-clock time since the hold began, not a count of
    /// events: at a real repeat rate, counting capped steps filled the bar at
    /// half the speed of the clock. What stops a stall arming it — a laptop
    /// waking, a debugger stopping the world — is that a gap longer than
    /// [`Self::GRACE`] is a key that came up, so the event after it begins a new
    /// hold rather than carrying the old one; and that arming also takes
    /// [`Self::MIN_PRESSES`] events, which only come from a key that is down.
    ///
    /// Returns true exactly once: on the event that completes the hold. The
    /// caller confirms on that, so holding past the threshold cannot purge
    /// twice.
    pub fn hold(&mut self, delta: Duration) -> bool {
        if self.armed {
            return false;
        }
        if delta > Self::GRACE {
            *self = Self::new();
        } else {
            self.held += delta;
        }
        self.presses += 1;
        self.lapsed = false;
        self.armed = self.held >= Self::HOLD && self.presses >= Self::MIN_PRESSES;
        self.armed
    }

    /// The key came up. Whatever had been held counts for nothing.
    ///
    /// ponytail: a plain terminal reports no key-release event, so the screen is
    /// told about the release rather than seeing it. The terminal loop (#54)
    /// calls this when a repeat fails to arrive within [`Self::GRACE`].
    pub fn release(&mut self) {
        let lapsed = !self.armed && self.presses > 0;
        *self = Self {
            lapsed,
            ..Self::new()
        };
    }

    pub fn is_armed(&self) -> bool {
        self.armed
    }

    /// How far through the hold, from 0.0 to 1.0.
    pub fn progress(&self) -> f32 {
        // The slower of the two conditions, so a full bar always means armed.
        let time = self.held.as_secs_f32() / Self::HOLD.as_secs_f32();
        let presses = self.presses as f32 / Self::MIN_PRESSES as f32;
        time.min(presses).min(1.0)
    }

    pub fn render(&self, plan: &Plan<Reviewed>, area: Rect, buf: &mut Buffer) {
        let left = area.x + 1;
        let mut y = area.y;

        buf.set_string(
            left,
            y,
            format!(
                "Hold  {PURGE}  to purge {} items, {}",
                plan.items().len(),
                human(plan.total_bytes())
            ),
            HEAD,
        );
        y += 2;

        let width = area.width.saturating_sub(2).max(10) as usize;
        let filled = (width as f32 * self.progress()).round() as usize;
        let bar: String = std::iter::repeat_n(FILLED, filled.min(width))
            .chain(std::iter::repeat_n(EMPTY, width.saturating_sub(filled)))
            .collect();
        buf.set_string(left, y, bar, DANGER);
        y += 2;

        // What is about to happen and how to stop it, side by side. A screen
        // that only says how to proceed reads as having no way out.
        buf.set_string(
            left,
            y,
            "Everything in the plan goes to the Trash, with a manifest saying how \
             to put it back.",
            DEFAULT,
        );
        buf.set_string(left, y + 1, "Release the key to cancel.", DEFAULT);

        if self.lapsed {
            // Said at the moment the bar empties, because an empty bar with
            // nothing said reads as the interface having broken.
            let note = BLOCKED;
            buf.set_string(
                left,
                y + 3,
                "The hold lapsed before the bar filled: the key stopped repeating.",
                note,
            );
            buf.set_string(
                left,
                y + 4,
                "If holding never fills it, your key repeat is too slow for this screen.",
                note,
            );
            buf.set_string(
                left,
                y + 5,
                "The shell does the same: dev-cleaner purge --execute --confirm",
                note,
            );
        }
    }
}
