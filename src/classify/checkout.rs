use std::path::{Path, PathBuf};

/// How far above a project's root to look for the repository it lives in. A
/// project is usually a subdirectory of a checkout (`app/ios`), but a `.git`
/// found many levels up belongs to something else, such as a dotfiles repo.
///
/// ponytail: a fixed ceiling rather than the scan root, which this layer does
/// not know. Raise it if a layout ever nests projects deeper.
const CLIMB: usize = 5;

/// What kind of checkout a project sits in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Kind {
    /// No repository, or one this reader cannot make sense of.
    #[default]
    Plain,
    /// The checkout that owns a repository's `.git` directory.
    Main,
    /// A linked worktree: its `.git` is a file pointing into the main repository.
    Worktree,
    /// A linked worktree whose repository no longer lists it, so its pointer
    /// leads nowhere. `git worktree prune` would remove the record it lacks.
    Orphan,
}

/// The checkout a project belongs to, read from the files git leaves on disk.
///
/// Read without running `git`: the layout is a stable on-disk format, and the
/// scanner stays dependency-free and fast over a hundred thousand entries.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Checkout {
    pub kind: Kind,
    /// The main repository's working directory (its `.git` directory's parent,
    /// or the bare repository itself). Shared by every worktree of it.
    pub repo: Option<PathBuf>,
    /// The worktree's own name: its directory under `.git/worktrees/`.
    pub worktree: Option<String>,
    /// The checked-out branch, or a short hash where HEAD is detached.
    pub branch: Option<String>,
    /// For a main checkout, how many linked worktrees its repository lists.
    pub linked: usize,
    /// Where this checkout's own `HEAD` and `logs/` live.
    pub git_dir: Option<PathBuf>,
    /// Where the objects and `refs/` shared by all worktrees live.
    pub common_dir: Option<PathBuf>,
}

impl Checkout {
    /// The checkout the project at `root` is in.
    pub fn of(root: &Path) -> Self {
        for dir in root.ancestors().take(CLIMB + 1) {
            let dot_git = dir.join(".git");
            let Ok(meta) = std::fs::symlink_metadata(&dot_git) else {
                continue;
            };
            return if meta.is_dir() {
                Self::main(dir, dot_git)
            } else {
                Self::linked(dir, &dot_git)
            };
        }
        Self::default()
    }

    fn main(dir: &Path, git: PathBuf) -> Self {
        let linked = std::fs::read_dir(git.join("worktrees"))
            .map(|d| d.flatten().filter(|e| e.path().is_dir()).count())
            .unwrap_or(0);
        Self {
            kind: Kind::Main,
            repo: Some(clean(dir)),
            branch: branch_of(&git),
            linked,
            git_dir: Some(git.clone()),
            common_dir: Some(git),
            ..Self::default()
        }
    }

    /// A `.git` file: `gitdir: <repo>/.git/worktrees/<name>`.
    fn linked(dir: &Path, dot_git: &Path) -> Self {
        let Some(pointer) = std::fs::read_to_string(dot_git).ok().and_then(|t| {
            t.trim()
                .strip_prefix("gitdir:")
                .map(|p| p.trim().to_string())
        }) else {
            return Self::default();
        };
        let gitdir = dir.join(pointer);
        // A submodule points at `.git/modules/<name>`: a repository, but not a
        // worktree of one, so it is not classified as either.
        let in_worktrees = gitdir
            .parent()
            .is_some_and(|p| p.file_name().is_some_and(|n| n == "worktrees"));
        let (Some(name), true) = (gitdir.file_name(), in_worktrees) else {
            return Self::default();
        };
        let name = name.to_string_lossy().into_owned();

        let common = std::fs::read_to_string(gitdir.join("commondir"))
            .ok()
            .map(|c| gitdir.join(c.trim()))
            .or_else(|| gitdir.parent()?.parent().map(Path::to_path_buf));
        let common = common.map(|c| clean(&c));
        let repo = common.as_deref().map(|c| match c.file_name() {
            Some(n) if n == ".git" => c
                .parent()
                .map_or_else(|| c.to_path_buf(), Path::to_path_buf),
            _ => c.to_path_buf(),
        });

        if !gitdir.is_dir() {
            return Self {
                kind: Kind::Orphan,
                repo,
                worktree: Some(name),
                ..Self::default()
            };
        }
        Self {
            kind: Kind::Worktree,
            repo,
            worktree: Some(name),
            branch: branch_of(&gitdir),
            git_dir: Some(gitdir),
            common_dir: common,
            ..Self::default()
        }
    }
}

/// `path` without `..` and symlinks where it can be resolved; as given otherwise.
fn clean(path: &Path) -> PathBuf {
    path.canonicalize().unwrap_or_else(|_| path.to_path_buf())
}

/// The branch `HEAD` names, or the first seven characters of a detached one.
fn branch_of(git: &Path) -> Option<String> {
    let head = std::fs::read_to_string(git.join("HEAD")).ok()?;
    let head = head.trim();
    match head.strip_prefix("ref: ") {
        Some(r) => Some(r.strip_prefix("refs/heads/").unwrap_or(r).to_string()),
        None if head.is_empty() => None,
        None => Some(head.chars().take(7).collect()),
    }
}
