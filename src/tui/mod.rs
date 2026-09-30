//! The terminal interface: where the guards become visible without becoming
//! reachable.
//!
//! Routing lives apart from drawing so the flow can be tested with no terminal
//! attached. What the interface refuses is a property of the state machine, not
//! of how a screen happens to be painted.

mod app;
mod candidates;
mod confirm;
mod dashboard;
mod data;
mod keymap;
pub mod palette;
mod projects;
mod result;
mod review;
mod row;
mod run;
mod terminal;

pub use app::{App, Screen};
pub use candidates::{Blocked, Candidates, Key, Order};
pub use confirm::Confirm;
pub use dashboard::{Consumer, Dashboard, Now, Trend};
pub use data::{Screens, collect};
pub use keymap::{
    Action, Binding, Effect, KeyPress, Motion, PURGE, adjacent, bindings, bindings_for,
};
pub use projects::{Column, ProjectSummary, Projects};
pub use result::Report;
pub use review::Review;
pub use run::{NOTICE_TTL, Step, Tui, footer, run, wayfinding};
pub use terminal::install_panic_hook;

/// Index of the first row of a window `height` tall that keeps `cursor` in it.
///
/// ponytail: derived from the cursor rather than kept as a scroll offset, so
/// moving past the bottom edge jumps the view by a row instead of following
/// smoothly. Keep an offset if the jumpiness shows.
fn window_start(cursor: usize, len: usize, height: usize) -> usize {
    (cursor + 1)
        .saturating_sub(height)
        .min(len.saturating_sub(height))
}

/// Where a list is, said on every list whether or not it fits.
///
/// A key that moves nothing on a list that already shows all of itself is
/// only readable as "complete" rather than "broken" if the list says so.
fn showing(start: usize, shown: usize, total: usize) -> String {
    if shown == 0 {
        return format!("showing 0 of {total}");
    }
    format!("showing {}-{} of {total}", start + 1, start + shown)
}
