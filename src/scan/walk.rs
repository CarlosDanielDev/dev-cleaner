use jwalk::WalkDirGeneric;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

/// One file observed during a walk.
#[derive(Debug, Clone)]
pub struct FileMeta {
    pub path: PathBuf,
    /// `st_size`: the logical length. Sparse files and APFS clones make this a lie.
    pub bytes_apparent: u64,
    /// `st_blocks * 512`: what the file actually occupies. This is the number
    /// the user gets back on deletion, so it is the one we report.
    pub bytes_actual: u64,
    /// Device and inode, used to avoid counting hardlinked content twice.
    pub dev: u64,
    pub ino: u64,
    /// Last modification. Feeds activity classification, which uses the newest
    /// source file in a project as evidence of recent work.
    pub mtime: SystemTime,
}

/// Everything a single walk produced, including non-fatal errors.
#[derive(Debug, Default)]
pub struct WalkResult {
    pub files: Vec<FileMeta>,
    pub errors: Vec<String>,
}

/// Per-entry state carried through jwalk's parallel pipeline.
type Sized = Option<(u64, u64, u64, u64, SystemTime)>;

/// Walks registered roots. Deliberately blind to `.gitignore`.
///
/// Every crate in this space is gitignore-aware by default, and the bytes worth
/// reclaiming are exactly the ones `.gitignore` hides. `jwalk` does no gitignore
/// filtering, and a test pins that behaviour.
pub struct Walker {
    roots: Vec<PathBuf>,
}

impl Walker {
    pub fn new<P: AsRef<Path>>(roots: impl IntoIterator<Item = P>) -> Self {
        Self {
            roots: roots
                .into_iter()
                .map(|p| p.as_ref().to_path_buf())
                .collect(),
        }
    }

    pub fn walk(&self) -> WalkResult {
        let mut out = WalkResult::default();

        for root in &self.roots {
            // Stat inside process_read_dir so it runs on jwalk's rayon pool.
            // Doing it in the consuming iterator instead leaves the walk
            // syscall-bound on a single core.
            let walker = WalkDirGeneric::<((), Sized)>::new(root)
                .skip_hidden(false)
                .follow_links(false)
                .process_read_dir(|_depth, _path, _state, children| {
                    for child in children.iter_mut().flatten() {
                        if !child.file_type().is_file() {
                            continue;
                        }
                        if let Ok(md) = std::fs::symlink_metadata(child.path()) {
                            child.client_state = Some((
                                md.size(),
                                md.blocks() * 512,
                                md.dev(),
                                md.ino(),
                                md.modified().unwrap_or(UNIX_EPOCH),
                            ));
                        }
                    }
                });

            for entry in walker {
                match entry {
                    Ok(mut e) => {
                        // A directory we could not descend into surfaces here rather
                        // than as an Err, so an unreadable subtree would otherwise be
                        // silently reported as empty.
                        if let Some(err) = e.read_children_error.take() {
                            out.errors.push(format!("{}: {err}", e.path().display()));
                        }
                        if !e.file_type().is_file() {
                            continue;
                        }
                        match e.client_state {
                            Some((bytes_apparent, bytes_actual, dev, ino, mtime)) => {
                                out.files.push(FileMeta {
                                    path: e.path(),
                                    bytes_apparent,
                                    bytes_actual,
                                    dev,
                                    ino,
                                    mtime,
                                })
                            }
                            None => out
                                .errors
                                .push(format!("{}: could not stat", e.path().display())),
                        }
                    }
                    Err(e) => out.errors.push(e.to_string()),
                }
            }
        }
        out
    }
}
