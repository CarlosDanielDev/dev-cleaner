use std::path::PathBuf;
use std::time::Duration;

use crate::classify::Ecosystem;
use crate::store::TrendRow;
use crate::volume::Volume;

/// One thing taking up room, measured both ways.
#[derive(Debug, Clone)]
pub struct Consumer {
    pub label: String,
    pub bytes: u64,
    /// File count. The largest directory is often not the one with the most
    /// files in it, and it is the file count that makes every backup and every
    /// Spotlight pass slow.
    pub inodes: u64,
}

/// What can be said about change since the last scan.
///
/// A first run has no comparison, which is ordinary rather than missing. Saying
/// so once here keeps it out of every field that would otherwise be optional.
#[derive(Debug, Clone, Default)]
pub enum Trend {
    #[default]
    FirstScan,
    Since(Vec<TrendRow>),
    /// The history could not be read, so nothing can be said about change.
    ///
    /// Deliberately not folded into `FirstScan`. Announcing "recorded as the
    /// first scan of these roots" because the database would not open is a
    /// claim about the disk made out of a failure to reach a file, and the next
    /// run would contradict it.
    Unavailable(String),
}

/// What one step forward would let the user do.
///
/// Counted from the objects the candidates screen and the projects table are
/// built from, never from the scan again, so the opening screen cannot
/// disagree with the screens it points at.
#[derive(Debug, Clone, Default)]
pub struct Now {
    /// Entries every guard cleared.
    pub offerable: usize,
    /// What they add up to, summed as the plan sums them: the figure the
    /// confirm screen would ask the user to approve if every one were marked.
    /// The gauge above measures the union instead and can read lower where
    /// directories hardlink into each other, as the plan already does (#45).
    pub offerable_bytes: u64,
    /// Why entries were held back, in the words the candidates screen uses,
    /// each with how many it held. In no particular order; the screen ranks.
    pub blocked: Vec<(String, usize)>,
    /// Projects the table calls dead.
    pub dead: usize,
    /// The build output measured inside them, per project as the table
    /// measures it. Not the projects themselves: the tool does not offer those.
    pub dead_reclaimable: u64,
}

/// One kind of artifact directory, and what it holds across every project.
///
/// `bytes` of every group add up to the reclaimable total, exactly: a file
/// reachable from two kinds is counted in the larger one only, the way the
/// total counts it once, so the breakdown and the gauge cannot disagree.
#[derive(Debug, Clone)]
pub struct Group {
    /// The directory name that identifies the kind: `node_modules`, `target`.
    pub label: String,
    pub ecosystem: Ecosystem,
    /// The command that brings it back.
    pub regen: String,
    pub bytes: u64,
    /// How many directories of this kind were found.
    pub dirs: usize,
    /// What the guards cleared to offer, summed as the plan sums it.
    pub offerable_bytes: u64,
    pub offerable_dirs: usize,
}

/// What the scan looked at: the context for every other number on the screen.
#[derive(Debug, Clone, Default)]
pub struct Analysed {
    pub projects: usize,
    /// Projects with build output inside them.
    pub with_rebuild: usize,
    /// Directory entries the walk saw.
    pub entries: u64,
    /// Artifact directories measured.
    pub measured: usize,
    pub elapsed: Duration,
    pub roots: Vec<PathBuf>,
}

/// The opening screen: the state of the disk, what the scan looked at, where
/// the rebuildable bytes are, and what is worth doing first.
#[derive(Debug, Clone, Default)]
pub struct Dashboard {
    /// `None` when the volume could not be measured. The rest of the scan's
    /// answers are still worth showing.
    pub volume: Option<Volume>,
    pub reclaimable: u64,
    pub trend: Trend,
    pub consumers: Vec<Consumer>,
    pub now: Now,
    /// What each recent scan of these roots found reclaimable, oldest first.
    /// `None` is a scan recorded before the total was kept. Empty when the
    /// history could not be read.
    pub history: Vec<Option<u64>>,
    /// Reclaimable bytes by kind of directory, in no particular order; the
    /// screen ranks them.
    pub groups: Vec<Group>,
    pub analysed: Analysed,
}

impl Dashboard {
    pub fn top_by_bytes(&self, n: usize) -> Vec<&Consumer> {
        let mut all: Vec<&Consumer> = self.consumers.iter().collect();
        all.sort_by_key(|c| std::cmp::Reverse(c.bytes));
        all.into_iter().take(n).collect()
    }

    pub fn top_by_inodes(&self, n: usize) -> Vec<&Consumer> {
        let mut all: Vec<&Consumer> = self.consumers.iter().collect();
        all.sort_by_key(|c| std::cmp::Reverse(c.inodes));
        all.into_iter().take(n).collect()
    }

    /// The groups, the heaviest first. Ties break on the name so the order is
    /// the same on every draw.
    pub fn ranked_groups(&self) -> Vec<&Group> {
        let mut all: Vec<&Group> = self.groups.iter().collect();
        all.sort_by(|a, b| b.bytes.cmp(&a.bytes).then_with(|| a.label.cmp(&b.label)));
        all
    }

    /// The group with the most the guards cleared to offer, if any.
    pub fn biggest_win(&self) -> Option<&Group> {
        self.groups
            .iter()
            .filter(|g| g.offerable_bytes > 0)
            .max_by(|a, b| {
                a.offerable_bytes
                    .cmp(&b.offerable_bytes)
                    .then_with(|| b.label.cmp(&a.label))
            })
    }
}
