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
mod keymap;
mod projects;
mod result;
mod review;
mod row;

pub use app::{App, Screen};
pub use candidates::{Blocked, Candidates, Key};
pub use confirm::Confirm;
pub use dashboard::{Consumer, Dashboard, Trend};
pub use keymap::{
    Action, Binding, Effect, KeyPress, Motion, PURGE, adjacent, bindings, bindings_for,
};
pub use projects::{Column, ProjectSummary, Projects};
pub use result::Report;
pub use review::Review;
