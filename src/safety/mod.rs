//! Deciding what may be deleted, and making the wrong answer unreachable.

mod docker;
mod guard;
mod plan;
mod tier;

pub use docker::DockerTarget;
pub use guard::Guards;
pub use plan::{Candidate, Confirmed, Draft, Plan, Rejected, Reviewed};
pub use tier::{BlockReason, RegenCommand, Safety};
