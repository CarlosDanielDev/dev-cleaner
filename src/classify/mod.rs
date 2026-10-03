//! What a directory is, which ecosystem owns it, and whether its project is alive.

mod activity;
mod artifact;
mod cache;
mod checkout;
mod lockfile;
mod project;

pub use activity::{Activity, last_activity};
pub use artifact::{ArtifactKind, Ecosystem, artifact_for, artifact_kinds, artifact_root};
pub use cache::{CacheEntry, CacheKind, cache_kinds, probe_caches};
pub use checkout::{Checkout, Kind};
pub use lockfile::{
    Lockfile, LockfileKind, Package, lockfile_for, lockfile_kinds, lockfiles_in, parse_lockfile,
    read_lockfile,
};
pub use project::{Project, ProjectIndex, is_git_metadata, is_inside_artifact, is_project_marker};
