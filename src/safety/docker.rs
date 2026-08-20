use super::{BlockReason, RegenCommand, Safety};

/// What a Docker prune can and cannot reclaim.
///
/// Docker reports volumes among its reclaimable space, which is true of the
/// bytes and false of the consequences. On the reference machine the
/// "reclaimable" volumes held a running Postgres database and a Qdrant vector
/// store. No command rebuilds those, so they are Protected and no code path
/// here emits a flag that would remove them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DockerTarget {
    Images,
    BuildCache,
    StoppedContainers,
    Volumes,
}

impl DockerTarget {
    pub fn all() -> [DockerTarget; 4] {
        use DockerTarget::*;
        [Images, BuildCache, StoppedContainers, Volumes]
    }

    pub fn safety(&self) -> Safety {
        use DockerTarget::*;
        match self {
            Images => Safety::Regenerable {
                regen: RegenCommand::new("docker pull, or rebuild from a Dockerfile")
                    .expect("non-empty"),
            },
            BuildCache => Safety::Cache {
                refills_on: "the next docker build",
            },
            StoppedContainers => Safety::Regenerable {
                regen: RegenCommand::new("docker run, or docker compose up").expect("non-empty"),
            },
            // Deliberately unreachable in the UI.
            Volumes => Safety::Protected {
                reason: BlockReason::DockerVolume,
            },
        }
    }

    /// The arguments to run for this target, or `None` when no command may
    /// exist for it.
    ///
    /// `--volumes` appears nowhere, and a test asserts that over every variant
    /// so a later addition cannot reintroduce it.
    pub fn prune_args(&self) -> Option<&'static [&'static str]> {
        use DockerTarget::*;
        match self {
            // -a removes unused images. Images still referenced by a running
            // container are untouched by docker itself.
            Images => Some(&["system", "prune", "-a", "-f"]),
            BuildCache => Some(&["builder", "prune", "-f"]),
            StoppedContainers => Some(&["container", "prune", "-f"]),
            Volumes => None,
        }
    }
}
