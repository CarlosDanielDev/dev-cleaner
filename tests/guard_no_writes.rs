//! The dirty-tree guard must read a repository, never write to it.
//!
//! `git status` refreshes the index and rewrites `.git/index` when its stat
//! cache is stale (after a build, a checkout, another machine). The guard runs
//! it on the user's own repositories, so it passes `--no-optional-locks`. These
//! tests pin that, and pin that the flag does not change what the guard decides.

mod common;

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, SystemTime};

use common::Fixture;
use dev_cleaner::safety::{BlockReason, Guards};

fn guards(fx: &Fixture) -> Guards {
    Guards::new(vec![fx.root().to_path_buf()], Vec::new())
}

/// Change every tracked file's mtime without changing its content, so git's
/// stat cache no longer matches and a plain `git status` wants to refresh it.
fn make_stat_cache_stale(repo: &Path) {
    let later = SystemTime::now() + Duration::from_secs(60);
    for name in ["README.md", "src/lib.rs"] {
        std::fs::File::open(repo.join(name))
            .expect("open")
            .set_modified(later)
            .expect("set mtime");
    }
}

fn repo_with_src(fx: &Fixture) -> PathBuf {
    fx.file("r/src/lib.rs", b"fn main() {}");
    fx.file("r/.gitignore", b"node_modules\n");
    fx.git_repo("r", 30)
}

fn status(repo: &Path, candidate: &Path, flag: bool) -> String {
    let mut c = Command::new("git");
    c.current_dir(repo);
    if flag {
        c.arg("--no-optional-locks");
    }
    let out = c
        .args(["status", "--porcelain", "--untracked-files=normal", "--"])
        .arg(candidate)
        .output()
        .expect("git");
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).into_owned()
}

#[test]
fn checking_a_candidate_does_not_rewrite_the_git_index() {
    let fx = Fixture::new();
    let repo = repo_with_src(&fx);
    make_stat_cache_stale(&repo);
    let index = repo.join(".git/index");
    let bytes = std::fs::read(&index).expect("index");
    let mtime = std::fs::metadata(&index).unwrap().modified().unwrap();
    // Let a rewrite show up as a new mtime even on coarse clocks.
    std::thread::sleep(Duration::from_millis(1100));

    let _ = guards(&fx).check(&repo.join("src"));

    assert!(
        std::fs::read(&index).unwrap() == bytes,
        "index bytes changed: the guard rewrote .git/index"
    );
    assert_eq!(
        std::fs::metadata(&index).unwrap().modified().unwrap(),
        mtime,
        "index mtime changed: the guard wrote to the repository"
    );
}

/// Run each scenario against the old command, the new command and the guard.
fn differential(
    name: &str,
    setup: impl Fn(&Fixture, &Path) -> PathBuf,
    want: Result<(), BlockReason>,
) {
    let fx = Fixture::new();
    let repo = repo_with_src(&fx);
    let candidate = setup(&fx, &repo);
    let candidate = candidate.canonicalize().expect("candidate");
    let repo = repo.canonicalize().expect("repo");
    let new = status(&repo, &candidate, true);
    let old = status(&repo, &candidate, false);
    assert_eq!(old, new, "{name}: the flag changed what git reports");
    assert_eq!(guards(&fx).check(&candidate), want, "{name}");
}

#[test]
fn the_flag_does_not_change_the_verdict() {
    differential(
        "clean subdir",
        |fx, r| {
            fx.file("r/node_modules/x", b"1");
            r.join("src")
        },
        Ok(()),
    );
    differential(
        "modified tracked file",
        |fx, r| {
            fx.file("r/src/lib.rs", b"changed");
            r.join("src")
        },
        Err(BlockReason::DirtyWorktree),
    );
    differential(
        "untracked inside candidate",
        |fx, r| {
            fx.file("r/src/new.rs", b"1");
            r.join("src")
        },
        Err(BlockReason::UntrackedSource),
    );
    differential(
        "untracked outside candidate",
        |fx, r| {
            fx.file("r/other/new.rs", b"1");
            r.join("src")
        },
        Ok(()),
    );
    differential(
        "ignored directory",
        |fx, r| {
            fx.file("r/node_modules/dep/a.js", b"1");
            r.join("node_modules")
        },
        Ok(()),
    );
    differential(
        "staged change",
        |fx, r| {
            fx.file("r/src/lib.rs", b"staged");
            fx.git("r", &["add", "src/lib.rs"]);
            r.join("src")
        },
        Err(BlockReason::DirtyWorktree),
    );
    differential(
        "stash present, candidate is a subdir",
        |fx, r| {
            fx.file("r/src/lib.rs", b"stashed");
            fx.git("r", &["stash"]);
            r.join("src")
        },
        Ok(()),
    );
    differential(
        "stash present, candidate is the root",
        |fx, r| {
            fx.file("r/src/lib.rs", b"stashed");
            fx.git("r", &["stash"]);
            r.to_path_buf()
        },
        Err(BlockReason::StashEntries),
    );
    differential("clean repo root", |_, r| r.to_path_buf(), Ok(()));
    differential(
        "dirty repo root",
        |fx, r| {
            fx.file("r/src/lib.rs", b"changed");
            r.to_path_buf()
        },
        Err(BlockReason::DirtyWorktree),
    );
}
