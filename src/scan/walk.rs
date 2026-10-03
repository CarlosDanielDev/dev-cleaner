use jwalk::WalkDirGeneric;
use std::collections::BTreeMap;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU8, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{SystemTime, UNIX_EPOCH};

use super::gauge::{Baseline, Phase, Reading, TopReading};
use crate::classify::{is_inside_artifact, is_project_marker};

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
    /// Whether the walk was asked to stop. What it found is then a part.
    pub cancelled: bool,
}

/// Why a directory or file could not be read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnreadableKind {
    /// Not allowed: a mode, or on macOS a privacy prompt that was refused.
    Permission,
    /// There when the parent was listed and gone when it was read.
    Vanished,
    /// A directory that contains itself.
    Loop,
    Other,
}

impl UnreadableKind {
    pub fn of_io(kind: std::io::ErrorKind) -> Self {
        match kind {
            std::io::ErrorKind::PermissionDenied => Self::Permission,
            std::io::ErrorKind::NotFound => Self::Vanished,
            _ => Self::Other,
        }
    }

    fn of(err: &jwalk::Error) -> Self {
        if err.loop_ancestor().is_some() {
            return Self::Loop;
        }
        err.io_error()
            .map_or(Self::Other, |e| Self::of_io(e.kind()))
    }

    const fn slot(self) -> usize {
        self as usize
    }
}

/// How many things could not be read, by why.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Unreadable {
    pub permission: u64,
    pub vanished: u64,
    pub looped: u64,
    pub other: u64,
}

impl Unreadable {
    pub fn total(&self) -> u64 {
        self.permission + self.vanished + self.looped + self.other
    }
}

/// A top-level folder, or the files directly in a root, as the walk counts it.
#[derive(Debug, Default)]
struct Top {
    entries: u64,
    /// Directories discovered inside it and not yet read, itself included.
    pending: i64,
    folder: bool,
}

#[derive(Debug, Default)]
struct Folders {
    by: BTreeMap<PathBuf, Top>,
    current: Option<PathBuf>,
}

/// What a walk has seen so far, readable from another thread while it runs.
///
/// Bumped on jwalk's pool as each directory is read, so the numbers move with
/// the disk rather than with the consumer. `Relaxed` is enough: a reader wants
/// a figure that never goes backwards, not a snapshot of both together.
///
/// The folder table is behind a lock, taken once per directory read and never
/// by a reader that would have to wait for it: [`Progress::read`] skips it
/// when it is busy, so a frame is never held up by the disk.
#[derive(Debug, Default)]
pub struct Progress {
    pub entries: AtomicU64,
    pub bytes: AtomicU64,
    projects: AtomicU64,
    cancelled: AtomicBool,
    unreadable: [AtomicU64; 4],
    phase: AtomicU8,
    phase_done: AtomicU64,
    phase_total: AtomicU64,
    folders: Mutex<Folders>,
    baseline: Mutex<Option<Arc<Baseline>>>,
}

impl Progress {
    /// Ask the scan to stop. It does, at the next entry it would have read.
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::Relaxed);
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Relaxed)
    }

    /// What could not be read so far, by why.
    pub fn unreadable(&self) -> Unreadable {
        let n = |k: UnreadableKind| self.unreadable[k.slot()].load(Ordering::Relaxed);
        Unreadable {
            permission: n(UnreadableKind::Permission),
            vanished: n(UnreadableKind::Vanished),
            looped: n(UnreadableKind::Loop),
            other: n(UnreadableKind::Other),
        }
    }

    fn note_unreadable(&self, kind: UnreadableKind) {
        self.unreadable[kind.slot()].fetch_add(1, Ordering::Relaxed);
    }

    /// Move to `phase`, which has `total` things to do, or `0` where that is not known.
    pub fn set_phase(&self, phase: Phase, total: u64) {
        self.phase_done.store(0, Ordering::Relaxed);
        self.phase_total.store(total, Ordering::Relaxed);
        self.phase.store(phase.code(), Ordering::Relaxed);
    }

    /// One more thing of the current phase is done.
    pub fn tick(&self) {
        self.tick_by(1);
    }

    /// `n` more things of the current phase are done.
    pub fn tick_by(&self, n: u64) {
        self.phase_done.fetch_add(n, Ordering::Relaxed);
    }

    /// What the last complete scan of these roots held, once it has been read.
    pub fn set_baseline(&self, baseline: Option<Baseline>) {
        *self.baseline.lock().unwrap_or_else(PoisonError::into_inner) = baseline.map(Arc::new);
    }

    pub fn baseline(&self) -> Option<Arc<Baseline>> {
        self.baseline
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// Every counter at once, without ever waiting for the walk.
    pub fn read(&self) -> Reading {
        let folders = self.folders.try_lock().ok();
        Reading {
            entries: self.entries.load(Ordering::Relaxed),
            bytes: self.bytes.load(Ordering::Relaxed),
            projects: self.projects.load(Ordering::Relaxed),
            unreadable: self.unreadable().total(),
            phase: Phase::from_code(self.phase.load(Ordering::Relaxed)),
            phase_done: self.phase_done.load(Ordering::Relaxed),
            phase_total: self.phase_total.load(Ordering::Relaxed),
            current: folders.as_ref().and_then(|f| f.current.clone()),
            tops: folders.map(|f| {
                f.by.iter()
                    .map(|(path, top)| TopReading {
                        path: path.clone(),
                        entries: top.entries,
                        folder: top.folder,
                        done: top.pending <= 0,
                    })
                    .collect()
            }),
        }
    }

    /// What each top-level folder held, for the store to keep as the next scan's baseline.
    pub fn shape(&self) -> Vec<(PathBuf, u64)> {
        let folders = self.folders.lock().unwrap_or_else(PoisonError::into_inner);
        folders
            .by
            .iter()
            .map(|(path, top)| (path.clone(), top.entries))
            .collect()
    }

    /// One directory was read: `files` were measured in it and `subdirs` found.
    ///
    /// `new_folders` is only given for a root, whose directories are the
    /// top-level folders everything else is attributed to.
    fn dir_read(
        &self,
        root: &Path,
        dir: &Path,
        files: u64,
        subdirs: i64,
        new_folders: Vec<PathBuf>,
    ) {
        let mut folders = self.folders.lock().unwrap_or_else(PoisonError::into_inner);
        if dir == root {
            folders.by.entry(root.to_path_buf()).or_default().entries += files;
            for path in new_folders {
                folders.by.insert(
                    path,
                    Top {
                        entries: 0,
                        pending: 1,
                        folder: true,
                    },
                );
            }
            return;
        }
        let key = top_of(root, dir);
        let top = folders.by.entry(key.clone()).or_default();
        top.entries += files;
        top.pending += subdirs - 1;
        folders.current = Some(key);
    }

    /// A directory that was listed and could not be read: it will never report in.
    fn dir_failed(&self, root: &Path, dir: &Path) {
        let mut folders = self.folders.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(top) = folders.by.get_mut(&top_of(root, dir)) {
            top.pending -= 1;
        }
    }

    /// The walk is over: whatever it never got to read is not pending any more.
    fn walk_over(&self) {
        let mut folders = self.folders.lock().unwrap_or_else(PoisonError::into_inner);
        for top in folders.by.values_mut() {
            top.pending = 0;
        }
    }
}

/// The top-level folder of `root` that `dir` is in or is.
fn top_of(root: &Path, dir: &Path) -> PathBuf {
    match dir
        .strip_prefix(root)
        .ok()
        .and_then(|rel| rel.components().next())
    {
        Some(first) => root.join(first),
        None => root.to_path_buf(),
    }
}

/// Per-entry state carried through jwalk's parallel pipeline.
type Sized = Option<(u64, u64, u64, u64, SystemTime)>;

/// Whether a path is to be left out of the walk altogether.
type Skip = Arc<dyn Fn(&Path) -> bool + Send + Sync>;

/// Walks registered roots. Deliberately blind to `.gitignore`.
///
/// Every crate in this space is gitignore-aware by default, and the bytes worth
/// reclaiming are exactly the ones `.gitignore` hides. `jwalk` does no gitignore
/// filtering, and a test pins that behaviour.
pub struct Walker {
    roots: Vec<PathBuf>,
    skip: Option<Skip>,
}

impl Walker {
    pub fn new<P: AsRef<Path>>(roots: impl IntoIterator<Item = P>) -> Self {
        Self {
            roots: roots
                .into_iter()
                .map(|p| p.as_ref().to_path_buf())
                .collect(),
            skip: None,
        }
    }

    /// Leave out every path `skip` says yes to, and everything under it.
    ///
    /// Decided as each directory is read, so a skipped tree is never entered,
    /// never counted and never stat'd: the denylist is a boundary, and a walk
    /// that crosses it only to discard what it found has still looked.
    pub fn skipping(mut self, skip: impl Fn(&Path) -> bool + Send + Sync + 'static) -> Self {
        self.skip = Some(Arc::new(skip));
        self
    }

    pub fn walk(&self) -> WalkResult {
        self.walk_with(&Arc::new(Progress::default()))
    }

    /// Walk, counting into `progress` as entries are measured.
    ///
    /// Shared rather than borrowed because jwalk keeps the read-dir closure
    /// for `'static`, so it has to own its handle on the counters.
    pub fn walk_with(&self, progress: &Arc<Progress>) -> WalkResult {
        let mut out = WalkResult::default();

        for root in &self.roots {
            if progress.is_cancelled() {
                break;
            }
            let progress = Arc::clone(progress);
            let skip = self.skip.clone();
            let walk_root = root.clone();
            // Stat inside process_read_dir so it runs on jwalk's rayon pool.
            // Doing it in the consuming iterator instead leaves the walk
            // syscall-bound on a single core.
            let counting = Arc::clone(&progress);
            let walker = WalkDirGeneric::<((), Sized)>::new(root)
                .skip_hidden(false)
                .follow_links(false)
                .process_read_dir(move |depth, path, _state, children| {
                    // The call for the root entry itself, which has no directory
                    // of its own to read.
                    if depth.is_none() {
                        return;
                    }
                    if counting.is_cancelled() {
                        children.clear();
                        return;
                    }
                    let mut stopped = false;
                    if let Some(skip) = &skip {
                        children.retain(|child| {
                            stopped = stopped || counting.is_cancelled();
                            !stopped && child.as_ref().map_or(true, |c| !skip(&c.path()))
                        });
                    }

                    let (mut files, mut subdirs, mut marker) = (0u64, 0i64, false);
                    let mut new_folders = Vec::new();
                    for child in children.iter_mut().flatten() {
                        if stopped || counting.is_cancelled() {
                            stopped = true;
                            break;
                        }
                        let kind = child.file_type();
                        if kind.is_dir() {
                            subdirs += 1;
                            if path == walk_root {
                                new_folders.push(child.path());
                            }
                            continue;
                        }
                        if !kind.is_file() {
                            continue;
                        }
                        if let Ok(md) = std::fs::symlink_metadata(child.path()) {
                            let bytes_actual = md.blocks() * 512;
                            counting.entries.fetch_add(1, Ordering::Relaxed);
                            counting.bytes.fetch_add(bytes_actual, Ordering::Relaxed);
                            files += 1;
                            marker =
                                marker || is_project_marker(&child.file_name().to_string_lossy());
                            child.client_state = Some((
                                md.size(),
                                bytes_actual,
                                md.dev(),
                                md.ino(),
                                md.modified().unwrap_or(UNIX_EPOCH),
                            ));
                        }
                    }
                    if stopped {
                        children.clear();
                        return;
                    }
                    // A marker inside build output belongs to a dependency, not
                    // to a project; one directory is one project however many
                    // markers it holds.
                    if marker && !is_inside_artifact(path) {
                        counting.projects.fetch_add(1, Ordering::Relaxed);
                    }
                    counting.dir_read(&walk_root, path, files, subdirs, new_folders);
                });

            for entry in walker {
                if progress.is_cancelled() {
                    break;
                }
                match entry {
                    Ok(mut e) => {
                        // A directory we could not descend into surfaces here rather
                        // than as an Err, so an unreadable subtree would otherwise be
                        // silently reported as empty.
                        if let Some(err) = e.read_children_error.take() {
                            progress.note_unreadable(UnreadableKind::of(&err));
                            progress.dir_failed(root, &e.path());
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
                            None => {
                                progress.note_unreadable(UnreadableKind::Other);
                                out.errors
                                    .push(format!("{}: could not stat", e.path().display()))
                            }
                        }
                    }
                    Err(e) => {
                        progress.note_unreadable(UnreadableKind::of(&e));
                        out.errors.push(e.to_string())
                    }
                }
            }
            progress.walk_over();
        }
        out.cancelled = progress.is_cancelled();
        out
    }
}
