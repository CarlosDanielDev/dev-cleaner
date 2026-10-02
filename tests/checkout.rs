//! What kind of checkout a project is: a main repository, a linked git
//! worktree, a worktree whose repository is gone, or no repository at all.

mod common;

use common::Fixture;
use dev_cleaner::classify::{Activity, Checkout, Kind};
use std::path::PathBuf;
use std::time::SystemTime;

fn real(p: &std::path::Path) -> PathBuf {
    p.canonicalize().expect("canonical")
}

/// A repository with three linked worktrees, one orphaned worktree and a plain
/// folder: every kind in one tree.
fn tree() -> Fixture {
    let fx = Fixture::new();
    fx.git_repo("kyte", 200);
    fx.git_worktree("kyte", "wt/issue-942", "feat/942");
    fx.git_worktree("kyte", "wt/hotfix", "hotfix");
    fx.git_worktree_detached("kyte", "wt/detached");
    fx.git_worktree("kyte", "wt/gone", "gone");
    // The repository forgets the worktree, as `git worktree prune` does once
    // the directory is deleted; the directory is left holding a dangling pointer.
    std::fs::remove_dir_all(fx.root().join("kyte/.git/worktrees/gone")).expect("forget");
    fx.file("loose/main.py", b"print(1)");
    fx
}

#[test]
fn a_main_checkout_knows_its_branch_and_how_many_worktrees_hang_off_it() {
    let fx = tree();
    let c = Checkout::of(&fx.root().join("kyte"));
    assert_eq!(c.kind, Kind::Main);
    assert_eq!(c.branch.as_deref(), Some("main"));
    assert_eq!(
        c.repo.as_deref(),
        Some(real(&fx.root().join("kyte")).as_path())
    );
    assert_eq!(c.linked, 3, "the forgotten one is no longer linked");
}

#[test]
fn a_linked_worktree_names_its_repository_its_own_name_and_its_branch() {
    let fx = tree();
    let c = Checkout::of(&fx.root().join("wt/issue-942"));
    assert_eq!(c.kind, Kind::Worktree);
    assert_eq!(
        c.repo.as_deref(),
        Some(real(&fx.root().join("kyte")).as_path())
    );
    assert_eq!(c.worktree.as_deref(), Some("issue-942"));
    assert_eq!(c.branch.as_deref(), Some("feat/942"));
}

#[test]
fn a_detached_worktree_shows_a_short_hash_in_place_of_a_branch() {
    let fx = tree();
    let head = fx.git("kyte", &["rev-parse", "--short=7", "HEAD"]);
    let c = Checkout::of(&fx.root().join("wt/detached"));
    assert_eq!(c.kind, Kind::Worktree);
    assert_eq!(c.branch.as_deref(), Some(head.as_str()));
}

#[test]
fn a_worktree_whose_repository_forgot_it_is_an_orphan() {
    let fx = tree();
    let c = Checkout::of(&fx.root().join("wt/gone"));
    assert_eq!(c.kind, Kind::Orphan);
    assert_eq!(c.worktree.as_deref(), Some("gone"));
    assert_eq!(
        c.repo.as_deref(),
        Some(real(&fx.root().join("kyte")).as_path())
    );
    assert_eq!(c.branch, None);
}

#[test]
fn a_folder_with_no_git_is_plain() {
    let fx = tree();
    let c = Checkout::of(&fx.root().join("loose"));
    assert_eq!(c.kind, Kind::Plain);
    assert_eq!((c.repo, c.worktree, c.branch), (None, None, None));
}

#[test]
fn an_unreadable_git_file_is_plain_not_a_guess() {
    let fx = Fixture::new();
    fx.file("odd/.git", b"not a pointer");
    assert_eq!(Checkout::of(&fx.root().join("odd")).kind, Kind::Plain);
}

#[test]
fn a_project_inside_a_worktree_is_classified_by_the_worktree_around_it() {
    // `app/ios` is the project; the `.git` pointer is two directories up.
    let fx = tree();
    let nested = fx
        .file("wt/issue-942/app/ios/Podfile", b"")
        .parent()
        .unwrap()
        .to_path_buf();
    let c = Checkout::of(&nested);
    assert_eq!(c.kind, Kind::Worktree);
    assert_eq!(c.worktree.as_deref(), Some("issue-942"));
}

#[test]
fn a_worktree_is_dated_by_its_own_reflog_not_by_the_fallback() {
    // The repository is 200 days old; the worktree was created just now. The
    // old read looked for `<worktree>/.git/logs/HEAD`, found a file where it
    // expected a directory, and fell back to "no date".
    let fx = tree();
    let wt = fx.root().join("wt/hotfix");
    assert_eq!(Activity::of(&wt, None, SystemTime::now()), Activity::Active);
}

#[test]
fn a_long_idle_worktree_whose_head_is_pushed_is_dead_by_its_own_facts() {
    let fx = tree();
    fx.mark_pushed("kyte");
    let wt = fx.root().join("wt/hotfix");
    let log = fx.root().join("kyte/.git/worktrees/hotfix/logs/HEAD");
    fx.age_reflog(&log, 200);
    assert_eq!(
        Activity::of(&wt, None, SystemTime::now()),
        Activity::Dead,
        "HEAD is read from the worktree's gitdir, the remote refs from the common one"
    );
}

#[test]
fn an_unpushed_worktree_is_never_dead() {
    let fx = tree();
    // No remote-tracking refs at all.
    let wt = fx.root().join("wt/hotfix");
    fx.age_reflog(&fx.root().join("kyte/.git/worktrees/hotfix/logs/HEAD"), 200);
    assert_ne!(Activity::of(&wt, None, SystemTime::now()), Activity::Dead);
}
