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
