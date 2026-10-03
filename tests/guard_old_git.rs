//! An old or missing git must still block. `--no-optional-locks` needs git
//! 2.15 (2017); an older git errors on the unknown option, and the guard must
//! read that as "cannot prove clean", never as "clean".
//!
//! Own test binary: it edits `PATH`, which would race with other tests' git.

mod common;

use std::os::unix::fs::PermissionsExt;

use common::Fixture;
use dev_cleaner::safety::{BlockReason, Guards};

#[test]
fn a_git_that_rejects_the_flag_or_is_absent_still_blocks() {
    let fx = Fixture::new();
    fx.file("r/src/lib.rs", b"fn main() {}");
    let repo = fx.git_repo("r", 30);
    let candidate = repo.join("src").canonicalize().unwrap();
    let guards = Guards::new(vec![fx.root().to_path_buf()], Vec::new());
    assert_eq!(guards.check(&candidate), Ok(()), "real git: clean subdir");

    let real = String::from_utf8(
        std::process::Command::new("sh")
            .args(["-c", "command -v git"])
            .output()
            .unwrap()
            .stdout,
    )
    .unwrap();
    let bin = fx.root().join("fakebin");
    std::fs::create_dir(&bin).unwrap();
    let fake = bin.join("git");
    std::fs::write(
        &fake,
        format!(
            "#!/bin/sh\ncase \"$1\" in --no-optional-locks) echo 'unknown option' >&2; exit 129;; esac\nexec {} \"$@\"\n",
            real.trim()
        ),
    )
    .unwrap();
    std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).unwrap();

    let old_path = std::env::var_os("PATH").unwrap();
    let mut paths = vec![bin.clone()];
    paths.extend(std::env::split_paths(&old_path));
    // SAFETY: single test in this binary, no other thread reads the environment.
    unsafe { std::env::set_var("PATH", std::env::join_paths(paths).unwrap()) };
    let old_git = guards.check(&candidate);

    // No git at all: PATH holds only an empty directory.
    let empty = fx.root().join("emptybin");
    std::fs::create_dir(&empty).unwrap();
    unsafe { std::env::set_var("PATH", &empty) };
    let no_git = guards.check(&candidate);
    unsafe { std::env::set_var("PATH", old_path) };

    assert_eq!(old_git, Err(BlockReason::DirtyWorktree));
    assert_eq!(no_git, Err(BlockReason::DirtyWorktree));
}
