//! How much room the disk holding a path actually has.

use std::path::Path;

/// Capacity and free space on one mounted volume.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Volume {
    pub total: u64,
    pub free: u64,
}

impl Volume {
    /// Measure the volume holding `path`, or `None` if it cannot be queried.
    ///
    /// Takes a path rather than assuming the root filesystem, for two reasons.
    ///
    /// A scanned root may sit on a different volume entirely, such as an
    /// external disk or a separate mount, in which case the root filesystem's
    /// numbers say nothing about the space a purge there would return.
    ///
    /// And on modern macOS `/` is a sealed system volume whose reported usage
    /// is misleading: during this project's design it showed 24 GiB used while
    /// the data volume holding the user's files was at 349 GiB and 94% full.
    /// The two share an APFS container, so free space happens to agree, but
    /// reasoning from the sealed volume is a habit worth not forming.
    pub fn of(path: &Path) -> Option<Self> {
        use std::ffi::CString;
        use std::os::unix::ffi::OsStrExt;

        let c_path = CString::new(path.as_os_str().as_bytes()).ok()?;
        // SAFETY: c_path is a valid NUL-terminated string that outlives the
        // call, and statvfs only writes into the zeroed struct we hand it.
        let stat = unsafe {
            let mut stat: libc::statvfs = std::mem::zeroed();
            (libc::statvfs(c_path.as_ptr(), &mut stat) == 0).then_some(stat)?
        };
        let block = stat.f_frsize as u64;
        Some(Self {
            total: stat.f_blocks as u64 * block,
            // f_bavail, not f_bfree: blocks available to an unprivileged user,
            // which is the space this tool can actually give back.
            free: stat.f_bavail as u64 * block,
        })
    }

    /// Everything that is not free.
    ///
    /// Reclaimable space is a part of this, never a part of `free`. Adding it
    /// to the free side would report room the user does not have yet.
    pub fn used(&self) -> u64 {
        self.total.saturating_sub(self.free)
    }
}
