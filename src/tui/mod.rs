//! The terminal interface: where the guards become visible without becoming
//! reachable.
//!
//! Routing lives apart from drawing so the flow can be tested with no terminal
//! attached. What the interface refuses is a property of the state machine, not
//! of how a screen happens to be painted.

mod app;
mod dashboard;

pub use app::{App, Screen};
pub use dashboard::{Consumer, Dashboard, Trend};
