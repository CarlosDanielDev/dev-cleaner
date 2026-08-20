mod common;

use common::Fixture;
use dev_cleaner::config::Config;

#[test]
fn a_missing_config_still_yields_roots() {
    let fx = Fixture::new();
    let absent = fx.root().join("nope/config.toml");

    let cfg = Config::load(&absent).expect("a missing config is not an error");

    assert!(
        !cfg.roots.is_empty(),
        "defaults must never leave the root set empty; an empty set silently scans nothing"
    );
}

#[test]
fn tilde_expands_to_the_home_directory() {
    let fx = Fixture::new();
    let path = fx.file(
        "config.toml",
        b"roots = [\"~/projects\"]\ndenylist = [\"~/kyte\"]\n",
    );

    let cfg = Config::load(&path).expect("load");
    let home = std::env::var("HOME").expect("HOME");

    assert_eq!(cfg.roots[0], std::path::Path::new(&home).join("projects"));
    assert_eq!(cfg.denylist[0], std::path::Path::new(&home).join("kyte"));
}

#[test]
fn denylist_survives_traversal_in_the_candidate_path() {
    let fx = Fixture::new();
    fx.file("kyte/work.txt", b"payroll");
    fx.file("projects/toy.txt", b"toy");

    let cfg = Config {
        roots: vec![fx.root().to_path_buf()],
        denylist: vec![fx.root().join("kyte")],
        ..Config::default()
    };

    // The obvious form, and the same location reached by traversal.
    let direct = fx.root().join("kyte/work.txt");
    let sneaky = fx.root().join("projects/../kyte/work.txt");

    assert!(cfg.is_denied(&direct), "direct path must be denied");
    assert!(
        cfg.is_denied(&sneaky),
        "a denied path reached via .. must still be denied"
    );
    assert!(
        !cfg.is_denied(&fx.root().join("projects/toy.txt")),
        "unrelated paths must not be denied"
    );
}

#[test]
fn roots_that_do_not_exist_are_reported() {
    let fx = Fixture::new();
    fx.file("real/keep.txt", b"x");

    let cfg = Config {
        roots: vec![fx.root().join("real"), fx.root().join("ghost")],
        ..Config::default()
    };

    let missing = cfg.missing_roots();

    assert_eq!(missing, vec![fx.root().join("ghost")]);
}
