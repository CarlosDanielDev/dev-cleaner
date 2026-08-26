//! What a directory is, which ecosystem owns it, and whether its project is alive.

mod activity;
mod artifact;
mod cache;
mod project;

pub use activity::Activity;
pub use artifact::{ArtifactKind, Ecosystem, artifact_for, artifact_kinds, artifact_root};
pub use cache::{CacheEntry, CacheKind, cache_kinds, probe_caches};
pub use project::{Project, ProjectIndex};
