use std::path::{Path, PathBuf};
use std::time::SystemTime;

use crate::safety::{Confirmed, Plan};

/// Moves a path out of the filesystem.
///
/// An abstraction with exactly two implementors: [`TrashRemover`] in
/// production, and recording doubles in tests. Tests must never put anything in
/// the user's Trash, and production must never do anything else.
pub trait Remover {
    /// Move `path` away, returning where it went.
    fn remove(&self, path: &Path) -> std::io::Result<PathBuf>;

    /// Whether removal returns the space straight away.
    ///
    /// Trashing does not: the Trash sits on the same disk, so free space is
    /// unchanged until the user empties it. Sanctioned cleanup commands such as
    /// `docker system prune` or `go clean -modcache` do delete immediately.
    ///
    /// The distinction decides whether a free-space measurement means anything.
    /// Treating a trashed run as if it should have freed space would report
    /// normal behaviour as a near-total shortfall.
    fn frees_space_immediately(&self) -> bool {
        false
    }
}

/// The only remover used in production.
///
/// Routes to the macOS Trash so every deletion is reversible from Finder.
/// Nothing here unlinks: there is no `remove_dir_all` in this crate's
/// user-facing paths, which is what makes "nothing irreversible" true rather
/// than merely intended.
pub struct TrashRemover;

impl Remover for TrashRemover {
    fn remove(&self, path: &Path) -> std::io::Result<PathBuf> {
        trash::delete(path)
            .map_err(|e| std::io::Error::other(format!("could not move to Trash: {e}")))?;
        // The Trash names entries itself, and will rename on collision, so the
        // recorded destination is the directory rather than a guessed filename.
        Ok(trash_dir())
    }
}

fn trash_dir() -> PathBuf {
    PathBuf::from(std::env::var("HOME").unwrap_or_else(|_| "/".into())).join(".Trash")
}

/// What happened to one candidate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    Removed { trashed_to: PathBuf },
    Failed { error: String },
}

/// One line of the record.
#[derive(Debug, Clone)]
pub struct PurgeItem {
    pub path: PathBuf,
    pub bytes: u64,
    pub regen: String,
    pub result: Outcome,
}

/// The record of a run.
///
/// `bytes_expected` is what the plan predicted. `bytes_actual` is what the disk
/// gave back, filled in by the caller after measuring. They are stored
/// separately and reported separately, because presenting a prediction as a
/// result is how a tool loses the user's trust.
#[derive(Debug)]
pub struct Manifest {
    pub executed_at: SystemTime,
    pub items: Vec<PurgeItem>,
    pub bytes_expected: u64,
    pub bytes_actual: Option<u64>,
    /// Whether the remover used here returns space at once.
    pub freed_immediately: bool,
}

impl Manifest {
    pub fn removed(&self) -> impl Iterator<Item = &PurgeItem> {
        self.items
            .iter()
            .filter(|i| matches!(i.result, Outcome::Removed { .. }))
    }

    pub fn failed(&self) -> impl Iterator<Item = &PurgeItem> {
        self.items
            .iter()
            .filter(|i| matches!(i.result, Outcome::Failed { .. }))
    }

    /// Bytes belonging to items that actually moved. Never includes an item
    /// that failed.
    pub fn bytes_moved(&self) -> u64 {
        self.removed().map(|i| i.bytes).sum()
    }

    pub fn is_complete(&self) -> bool {
        self.failed().next().is_none()
    }

    /// Record what the disk actually returned.
    pub fn record_actual(&mut self, freed: u64) {
        self.bytes_actual = Some(freed);
    }

    /// Bytes sitting in the Trash: moved, reversible, and still occupying the
    /// disk until the user empties it.
    pub fn pending_in_trash(&self) -> u64 {
        if self.freed_immediately {
            0
        } else {
            self.bytes_moved()
        }
    }

    /// Whether the disk gave back materially less than the plan predicted.
    ///
    /// Only meaningful when removal frees space at once. After a trashed run
    /// the free-space delta is expected to be near zero, and calling that a
    /// shortfall would raise an alarm about the tool working correctly.
    pub fn shortfall(&self) -> Option<f64> {
        if !self.freed_immediately {
            return None;
        }
        let actual = self.bytes_actual?;
        let moved = self.bytes_moved();
        if moved == 0 {
            return None;
        }
        let ratio = 1.0 - (actual as f64 / moved as f64);
        (ratio > 0.10).then_some(ratio)
    }
}

/// Carry out a confirmed plan.
///
/// Takes `Plan<Confirmed>` by value. That signature is the enforcement: there
/// is no way to call this with a draft or an unreviewed plan, so the review and
/// confirmation steps cannot be skipped by any caller, now or later.
///
/// A failing item is recorded and the run continues. One unreadable directory
/// must not strand the rest of a plan the user already approved.
pub fn execute(plan: Plan<Confirmed>, remover: &dyn Remover) -> Manifest {
    let items = plan.into_items();
    let bytes_expected = items.iter().map(|c| c.bytes).sum();
    let mut recorded = Vec::with_capacity(items.len());

    for candidate in items {
        let result = match remover.remove(&candidate.path) {
            Ok(trashed_to) => Outcome::Removed { trashed_to },
            Err(error) => Outcome::Failed {
                error: error.to_string(),
            },
        };
        recorded.push(PurgeItem {
            path: candidate.path,
            bytes: candidate.bytes,
            regen: regen_of(&candidate.safety),
            result,
        });
    }

    Manifest {
        executed_at: SystemTime::now(),
        items: recorded,
        bytes_expected,
        bytes_actual: None,
        freed_immediately: remover.frees_space_immediately(),
    }
}

fn regen_of(safety: &crate::safety::Safety) -> String {
    use crate::safety::Safety::*;
    match safety {
        Cache { refills_on } => format!("refills on {refills_on}"),
        Regenerable { regen } => regen.to_string(),
        // Neither can enter a plan, so neither can reach this point.
        Unproven { .. } | Protected { .. } => String::new(),
    }
}

/// Free bytes on the volume holding `path`, or `None` if it cannot be queried.
///
/// Takes a path rather than assuming the root filesystem, for two reasons.
///
/// A scanned root may sit on a different volume entirely, such as an external
/// disk or a separate mount, in which case the root filesystem's numbers say
/// nothing about the space a purge there would return.
///
/// And on modern macOS `/` is a sealed system volume whose reported usage is
/// misleading: during this project's design it showed 24 GiB used while the
/// data volume holding the user's files was at 349 GiB and 94% full. The two
/// share an APFS container, so free space happens to agree, but reasoning from
/// the sealed volume is a habit worth not forming.
pub fn free_bytes(path: &Path) -> Option<u64> {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;

    let c_path = CString::new(path.as_os_str().as_bytes()).ok()?;
    // SAFETY: c_path is a valid NUL-terminated string that outlives the call,
    // and statvfs only writes into the zeroed struct we hand it.
    let stat = unsafe {
        let mut stat: libc::statvfs = std::mem::zeroed();
        (libc::statvfs(c_path.as_ptr(), &mut stat) == 0).then_some(stat)?
    };
    // f_bavail, not f_bfree: blocks available to an unprivileged user, which is
    // the space this tool can actually give back.
    Some(stat.f_bavail as u64 * stat.f_frsize as u64)
}
