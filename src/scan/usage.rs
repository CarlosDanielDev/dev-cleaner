use std::collections::HashSet;

use super::FileMeta;

/// Aggregate disk usage over a set of files.
///
/// `bytes_unique` is the number a user actually recovers by deleting these
/// paths. Package managers like pnpm and uv hardlink into a shared store, so
/// the same inode is reachable from many projects; counting it once per path
/// would promise space that deletion cannot return.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Usage {
    /// Sum of `st_size` across every path, hardlinks included.
    pub bytes_apparent: u64,
    /// Sum of allocated blocks, counting each inode exactly once.
    pub bytes_unique: u64,
    /// Number of distinct inodes. Inode pressure drives the daily lag that
    /// byte totals alone do not explain.
    pub inodes: u64,
    /// Number of directory entries seen, hardlinks included.
    pub files: u64,
}

impl Usage {
    /// Takes anything that yields file metadata, so a caller can measure a
    /// borrowed subset of a walk without copying it.
    pub fn of<'a>(files: impl IntoIterator<Item = &'a FileMeta>) -> Self {
        let mut seen: HashSet<(u64, u64)> = HashSet::new();
        let mut usage = Usage::default();

        for f in files {
            usage.files += 1;
            usage.bytes_apparent += f.bytes_apparent;
            if seen.insert((f.dev, f.ino)) {
                usage.bytes_unique += f.bytes_actual;
                usage.inodes += 1;
            }
        }
        usage
    }
}
