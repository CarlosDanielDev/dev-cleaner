use std::path::{Path, PathBuf};
use std::process::Command;

use super::BlockReason;

/// The non-overridable checks a candidate must survive.
///
/// These are blocks, not warnings. A candidate that fails any of them is
/// `Protected`, which means the selection cursor cannot reach it. There is no
/// keystroke, flag, or configuration that overrides one.
pub struct Guards {
    roots: Vec<PathBuf>,
    denylist: Vec<PathBuf>,
}

impl Guards {
    pub fn new(roots: Vec<PathBuf>, denylist: Vec<PathBuf>) -> Self {
        Self { roots, denylist }
    }

    /// Check a candidate, naming the first rule that rejects it.
    pub fn check(&self, path: &Path) -> Result<(), BlockReason> {
        // Compare twice. Lexical normalisation resolves `..` without touching
        // the filesystem; canonicalisation additionally follows symlinks. A
        // path that is lexically inside a root but resolves outside it got
        // there through a link, which is worth naming separately from a path
        // that was never in scope to begin with.
        let as_written = lexical(path);
        let resolved = resolve(path);

        let looked_inside = self
            .roots
            .iter()
            .any(|r| as_written.starts_with(lexical(r)));
        let really_inside = self.within_roots(&resolved);

        if !really_inside {
            return Err(if looked_inside {
                BlockReason::SymlinkEscape
            } else {
                BlockReason::OutsideRoots
            });
        }
        if self.denied(&resolved) {
            return Err(BlockReason::Denylisted);
        }
        if let Some(repo) = repo_root(&resolved) {
            check_repository(&repo)?;
        }
        Ok(())
    }

    fn within_roots(&self, resolved: &Path) -> bool {
        self.roots.iter().any(|r| resolved.starts_with(resolve(r)))
    }

    fn denied(&self, resolved: &Path) -> bool {
        self.denylist
            .iter()
            .any(|d| resolved.starts_with(resolve(d)))
    }
}

/// Whether a repository holds work that exists nowhere else.
///
/// Delegated to git rather than reimplemented. Deciding "clean" correctly means
/// parsing the binary index, applying gitignore semantics and handling
/// submodules; a subtle bug there reports clean and costs the user their
/// uncommitted work. Git is the authority on its own state, and this runs only
/// on candidates entering a plan, not during a scan.
///
/// Any failure to obtain an answer blocks. Not being able to prove a repository
/// is clean is treated exactly like knowing it is dirty.
fn check_repository(repo: &Path) -> Result<(), BlockReason> {
    if has_stash(repo) {
        return Err(BlockReason::StashEntries);
    }

    let output = Command::new("git")
        .current_dir(repo)
        .args(["status", "--porcelain", "--untracked-files=normal"])
        .output();

    let Ok(output) = output else {
        // git missing or not executable: assume the worst.
        return Err(BlockReason::DirtyWorktree);
    };
    if !output.status.success() {
        return Err(BlockReason::DirtyWorktree);
    }

    let text = String::from_utf8_lossy(&output.stdout);
    let mut untracked = false;
    for line in text.lines().filter(|l| !l.trim().is_empty()) {
        if line.starts_with("??") {
            untracked = true;
        } else {
            // Modified or staged content is the stronger signal, so report it
            // even if untracked files are also present.
            return Err(BlockReason::DirtyWorktree);
        }
    }
    if untracked {
        return Err(BlockReason::UntrackedSource);
    }
    Ok(())
}

fn has_stash(repo: &Path) -> bool {
    let git = repo.join(".git");
    if git.join("refs/stash").exists() || git.join("logs/refs/stash").exists() {
        return true;
    }
    std::fs::read_to_string(git.join("packed-refs"))
        .map(|s| s.lines().any(|l| l.ends_with("refs/stash")))
        .unwrap_or(false)
}

/// The nearest enclosing repository, searching upwards.
fn repo_root(path: &Path) -> Option<PathBuf> {
    path.ancestors()
        .find(|a| a.join(".git").exists())
        .map(Path::to_path_buf)
}

/// Normalise `.` and `..` without consulting the filesystem.
///
/// Deliberately does not follow symlinks: the difference between this and
/// `resolve` is exactly what identifies a link that leaves the roots.
fn lexical(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for part in path.components() {
        match part {
            std::path::Component::ParentDir => {
                out.pop();
            }
            std::path::Component::CurDir => {}
            other => out.push(other),
        }
    }
    out
}

/// Canonicalise as far as the filesystem allows.
///
/// A path that does not exist cannot be canonicalised, so fall back to a
/// lexical normalisation rather than giving up: `..` must still be resolved, or
/// a non-existent path could be used to step outside the roots.
fn resolve(path: &Path) -> PathBuf {
    path.canonicalize().unwrap_or_else(|_| lexical(path))
}
