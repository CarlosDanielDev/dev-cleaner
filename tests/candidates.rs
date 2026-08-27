pub mod common;

use common::Fixture;
use dev_cleaner::candidates::from_scan;
use dev_cleaner::safety::{Guards, Safety};
use dev_cleaner::scan::Walker;

fn build(fx: &Fixture) -> dev_cleaner::candidates::Build {
    let files = Walker::new([fx.root()]).walk().files;
    from_scan(&files, &Guards::new(vec![fx.root().to_path_buf()], vec![]))
}

#[test]
fn files_under_one_artifact_directory_become_a_single_candidate() {
    let fx = Fixture::new();
    fx.file("app/package.json", b"{}");
    fx.file("app/node_modules/react/index.js", &vec![b'x'; 8192]);
    fx.file("app/node_modules/lodash/index.js", &vec![b'y'; 8192]);

    let built = build(&fx);

    assert_eq!(built.candidates.len(), 1, "one directory, one candidate");
    let c = &built.candidates[0];
    assert!(c.path.ends_with("app/node_modules"));
    assert!(
        c.bytes >= 16384,
        "the candidate carries the whole tree's size: {}",
        c.bytes
    );
}

#[test]
fn nested_artifact_directories_roll_up_to_the_outermost() {
    // node_modules contains node_modules. Offering the inner one separately
    // would double-count and let a user delete half a tree.
    let fx = Fixture::new();
    fx.file("app/package.json", b"{}");
    fx.file("app/node_modules/vite/index.js", &vec![b'x'; 4096]);
    fx.file(
        "app/node_modules/vite/node_modules/esbuild/bin.js",
        &vec![b'y'; 4096],
    );

    let built = build(&fx);

    assert_eq!(built.candidates.len(), 1);
    assert!(built.candidates[0].path.ends_with("app/node_modules"));
}

#[test]
fn a_candidate_carries_the_command_that_rebuilds_it() {
    let fx = Fixture::new();
    fx.file("app/package.json", b"{}");
    fx.file("app/node_modules/react/index.js", b"x");

    let built = build(&fx);

    match &built.candidates[0].safety {
        Safety::Regenerable { regen } => assert_eq!(regen.as_str(), "npm install"),
        other => panic!("expected Regenerable, got {other:?}"),
    }
}

#[test]
fn artifacts_in_a_dirty_repository_are_rejected_with_the_reason() {
    let fx = Fixture::new();
    fx.git_repo("app", 10);
    fx.file("app/package.json", b"{}");
    fx.file("app/node_modules/react/index.js", b"x");

    let built = build(&fx);

    assert!(
        built.candidates.is_empty(),
        "nothing in an unclean repository may be offered"
    );
    assert_eq!(built.rejected.len(), 1);
    assert!(
        built.rejected[0]
            .because
            .to_lowercase()
            .contains("untracked"),
        "the reason must be specific: {}",
        built.rejected[0].because
    );
}

#[test]
fn source_files_are_never_candidates() {
    let fx = Fixture::new();
    fx.file("app/package.json", b"{}");
    fx.file("app/src/index.js", &vec![b'x'; 100_000]);

    assert!(
        build(&fx).candidates.is_empty(),
        "only registered artifact directories are ever offered"
    );
}

/// pnpm, uv and cargo all hardlink: one inode reachable from several paths.
/// Counting it once per path promises space that deletion cannot return, which
/// is the same rule the scan-wide total already follows.
#[test]
fn a_hardlinked_inode_inside_one_artifact_directory_counts_once() {
    let fx = Fixture::new();
    fx.file("app/package.json", b"{}");
    let real = fx.file(
        "app/node_modules/.store/react/index.js",
        &vec![b'x'; 400_000],
    );
    fx.hardlink("app/node_modules/react/index.js", &real);
    fx.hardlink("app/node_modules/nested/react/index.js", &real);

    let built = build(&fx);
    let c = &built.candidates[0];

    assert!(
        c.bytes < 600_000,
        "one 400 KB inode reachable three times was counted as {} bytes",
        c.bytes
    );
    assert!(
        c.bytes >= 400_000,
        "the inode still occupies the disk once: {}",
        c.bytes
    );
}

/// The independent check. `du -sk` deduplicates hardlinks the same way, so a
/// candidate that disagrees with it is a candidate promising the wrong number.
#[test]
fn a_candidate_agrees_with_du() {
    let fx = Fixture::new();
    fx.file("app/package.json", b"{}");
    let real = fx.file("app/node_modules/.store/pkg/blob.bin", &vec![b'x'; 300_000]);
    fx.hardlink("app/node_modules/a/blob.bin", &real);
    fx.hardlink("app/node_modules/b/blob.bin", &real);
    fx.file("app/node_modules/c/other.js", &vec![b'y'; 100_000]);

    let built = build(&fx);
    let c = &built.candidates[0];

    let out = std::process::Command::new("du")
        .args(["-sk", &c.path.to_string_lossy()])
        .output()
        .expect("du");
    let kb: u64 = String::from_utf8_lossy(&out.stdout)
        .split_whitespace()
        .next()
        .expect("du output")
        .parse()
        .expect("kilobytes");

    // du counts the directories themselves; the walk only counts files, so it
    // can read a little lower but never higher.
    let du_bytes = kb * 1024;
    assert!(
        c.bytes <= du_bytes && du_bytes - c.bytes < 64 * 1024,
        "candidate says {} bytes, du says {du_bytes}",
        c.bytes
    );
}
