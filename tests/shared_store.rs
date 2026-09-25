//! What a shared content-addressed store would recover, and what it would not.
//!
//! Every number this suite asserts is an estimate: nobody has run a migration
//! and measured the disk afterwards. The two ways such a number turns into a
//! lie are the same two the duplicate report faces, plus one of its own.
//!
//! Counting all N copies instead of N-1 promises back a package that has to
//! stay. Counting copies that already share an inode promises back blocks a
//! store has already collapsed. And counting a project that is *already* in a
//! store promises a migration that has already happened.

pub mod common;

use common::Fixture;
use dev_cleaner::scan::Walker;
use dev_cleaner::shared_store::{Estimate, Exclusion, Reason, estimate};

/// An npm v3 lockfile naming exactly these packages.
fn npm_lock(packages: &[(&str, &str)]) -> String {
    let entries: Vec<String> = packages
        .iter()
        .map(|(name, version)| {
            format!("    \"node_modules/{name}\": {{ \"version\": \"{version}\" }}")
        })
        .collect();
    format!(
        "{{\n  \"lockfileVersion\": 3,\n  \"packages\": {{\n    \"\": {{ \"version\": \"1.0.0\" }},\n{}\n  }}\n}}",
        entries.join(",\n")
    )
}

/// A pnpm v9 lockfile naming exactly these packages.
fn pnpm_lock(packages: &[(&str, &str)]) -> String {
    let entries: Vec<String> = packages
        .iter()
        .map(|(name, version)| {
            format!("  {name}@{version}:\n    resolution: {{integrity: sha512-x}}")
        })
        .collect();
    format!(
        "lockfileVersion: '9.0'\n\npackages:\n\n{}\n",
        entries.join("\n")
    )
}

/// The `[[package]]` array that Cargo, poetry and uv all write.
fn toml_lock(packages: &[(&str, &str)]) -> String {
    packages
        .iter()
        .map(|(name, version)| format!("[[package]]\nname = \"{name}\"\nversion = \"{version}\"\n"))
        .collect::<Vec<_>>()
        .join("\n")
}

/// A project whose lockfiles are exactly `locks`, with nothing installed yet.
fn project(fx: &Fixture, app: &str, locks: &[(&str, String)]) {
    fx.file(&format!("{app}/package.json"), b"{}");
    for (name, text) in locks {
        fx.file(&format!("{app}/{name}"), text.as_bytes());
    }
}

/// A project holding `bytes` of `name` under its own `node_modules`.
fn installs(fx: &Fixture, app: &str, name: &str, bytes: usize) -> std::path::PathBuf {
    fx.file(
        &format!("{app}/node_modules/{name}/index.js"),
        &vec![b'x'; bytes],
    )
}

fn run(fx: &Fixture) -> Estimate {
    estimate(&Walker::new([fx.root()]).walk().files)
}

fn excluded<'a>(e: &'a Estimate, app: &str) -> Option<&'a Exclusion> {
    e.excluded.iter().find(|x| x.project.ends_with(app))
}

/// One copy has to stay, in a store as much as in a project. An estimate that
/// adds up all N copies promises back a package the store still holds.
#[test]
fn a_package_in_three_projects_estimates_two_copies_recovered_not_three() {
    let fx = Fixture::new();
    for app in ["a", "b", "c"] {
        project(
            &fx,
            app,
            &[("package-lock.json", npm_lock(&[("react", "18.3.1")]))],
        );
        installs(&fx, app, "react", 400_000);
    }

    let e = run(&fx);

    assert_eq!(e.projects(), 3, "all three can migrate");
    assert!(
        e.excluded.is_empty(),
        "nothing to exclude: {:?}",
        e.excluded
    );
    assert!(
        (800_000..1_200_000).contains(&e.bytes()),
        "three independent 400 KB copies collapse to one, recovering two: {} bytes",
        e.bytes()
    );
}

/// The test this estimate exists to pass. Where the copies are already one
/// inode, a store has nothing left to collapse and the honest estimate is zero.
/// Anything else offers the user space that pnpm already gave them.
#[test]
fn copies_already_hardlinked_together_estimate_nothing() {
    let fx = Fixture::new();
    project(
        &fx,
        "a",
        &[("package-lock.json", npm_lock(&[("react", "18.3.1")]))],
    );
    let real = installs(&fx, "a", "react", 400_000);

    for app in ["b", "c"] {
        project(
            &fx,
            app,
            &[("package-lock.json", npm_lock(&[("react", "18.3.1")]))],
        );
        fx.hardlink(&format!("{app}/node_modules/react/index.js"), &real);
    }

    let e = run(&fx);

    assert_eq!(e.projects(), 3, "all three still count as migratable");
    assert_eq!(
        e.bytes(),
        0,
        "one 400 KB inode reachable from three projects occupies the disk once; \
         a store cannot recover blocks that are already shared"
    );
    assert_eq!(e.inferred_bytes(), 0, "nothing was inferred either");
}

/// A project that already keeps a `pnpm-lock.yaml` is in the store this
/// estimate is recommending. Counting it would sell a migration that happened.
#[test]
fn a_pnpm_project_is_already_in_a_store_and_leaves_the_estimate() {
    let fx = Fixture::new();
    for app in ["a", "b"] {
        project(
            &fx,
            app,
            &[("package-lock.json", npm_lock(&[("react", "18.3.1")]))],
        );
        installs(&fx, app, "react", 400_000);
    }
    project(
        &fx,
        "c",
        &[("pnpm-lock.yaml", pnpm_lock(&[("react", "18.3.1")]))],
    );
    installs(&fx, "c", "react", 400_000);

    let e = run(&fx);

    assert_eq!(
        e.projects(),
        2,
        "only the two npm projects have a store to move to"
    );
    assert!(
        matches!(
            excluded(&e, "c").map(|x| &x.reason),
            Some(Reason::AlreadyStored("pnpm"))
        ),
        "c should be excluded as already stored: {:?}",
        e.excluded
    );
    assert!(
        (300_000..600_000).contains(&e.bytes()),
        "two independent copies recover one, and c's copy is not in the arithmetic: {} bytes",
        e.bytes()
    );
}

/// uv installs into its own cache for the same reason and gets the same answer.
#[test]
fn a_uv_project_is_already_in_a_cache_and_leaves_the_estimate() {
    let fx = Fixture::new();
    project(
        &fx,
        "a",
        &[("package-lock.json", npm_lock(&[("react", "18.3.1")]))],
    );
    installs(&fx, "a", "react", 400_000);
    project(&fx, "py", &[("uv.lock", toml_lock(&[("httpx", "0.27.0")]))]);

    let e = run(&fx);

    assert!(
        matches!(
            excluded(&e, "py").map(|x| &x.reason),
            Some(Reason::AlreadyStored("uv"))
        ),
        "py should be excluded as already cached: {:?}",
        e.excluded
    );
}

/// Two lockfiles from two package managers describe two different installs and
/// neither one is authoritative. Picking one would be arbitration dressed as
/// detection, so the project leaves the estimate and the report names it.
#[test]
fn a_project_between_two_lockfiles_is_excluded_and_says_which_two() {
    let fx = Fixture::new();
    for app in ["a", "b"] {
        project(
            &fx,
            app,
            &[("package-lock.json", npm_lock(&[("react", "18.3.1")]))],
        );
        installs(&fx, app, "react", 400_000);
    }
    project(
        &fx,
        "mid",
        &[
            ("package-lock.json", npm_lock(&[("react", "18.3.1")])),
            ("pnpm-lock.yaml", pnpm_lock(&[("react", "18.3.1")])),
        ],
    );
    installs(&fx, "mid", "react", 400_000);

    let e = run(&fx);
    let x = excluded(&e, "mid").expect("mid is excluded");

    assert!(
        matches!(x.reason, Reason::Migrating),
        "a half-migrated project is its own case, not 'already stored': {:?}",
        x.reason
    );
    assert!(
        x.lockfiles.contains(&"package-lock.json") && x.lockfiles.contains(&"pnpm-lock.yaml"),
        "the report has to name both lockfiles rather than pick one: {:?}",
        x.lockfiles
    );
    assert_eq!(e.projects(), 2, "mid is out of the arithmetic entirely");
}

/// Cargo hardlinks into `~/.cargo` by default. There is no store to move to
/// and no command to print, so a Rust project is not an opportunity at all.
#[test]
fn a_cargo_project_is_out_of_scope_rather_than_an_opportunity() {
    let fx = Fixture::new();
    project(
        &fx,
        "a",
        &[("package-lock.json", npm_lock(&[("react", "18.3.1")]))],
    );
    installs(&fx, "a", "react", 400_000);
    project(
        &fx,
        "rs",
        &[("Cargo.lock", toml_lock(&[("serde", "1.0.210")]))],
    );

    let e = run(&fx);

    assert!(
        matches!(
            excluded(&e, "rs").map(|x| &x.reason),
            Some(Reason::OutOfScope)
        ),
        "rs should be out of scope: {:?}",
        e.excluded
    );
}

/// The whole premise of this tool is that deletion is provably reversible.
/// Rewriting a project's dependency layout on a guess earns none of that, so
/// the migration is printed and never invoked. `git` is the one subprocess the
/// crate is allowed, and it only ever reads.
#[test]
fn no_code_path_can_run_a_package_manager() {
    let src = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut pending = vec![src];
    let mut checked = 0;

    while let Some(dir) = pending.pop() {
        for entry in std::fs::read_dir(&dir).expect("read src") {
            let path = entry.expect("entry").path();
            if path.is_dir() {
                pending.push(path);
                continue;
            }
            if path.extension().is_none_or(|e| e != "rs") {
                continue;
            }
            let text = std::fs::read_to_string(&path).expect("read source");
            checked += 1;
            for (n, line) in text.lines().enumerate() {
                let Some((head, rest)) = line.split_once("Command::new(") else {
                    continue;
                };
                // `RegenCommand::new` is a different name that ends in the same
                // letters, and it only ever stores a string for printing.
                if head.ends_with(|c: char| c.is_alphanumeric() || c == '_') {
                    continue;
                }
                assert!(
                    rest.starts_with("\"git\""),
                    "{}:{} spawns something other than git: {}",
                    path.display(),
                    n + 1,
                    line.trim()
                );
            }
        }
    }
    assert!(
        checked > 10,
        "the walk should have found the crate: {checked} files"
    );
}

/// Two package names over one inode. npm hardlinks the platform binary it
/// downloaded into the wrapper package that selects it, so `esbuild` and
/// `@esbuild/darwin-arm64` are two rows describing one set of blocks. Each row
/// is right on its own; adding them up promises the binary back twice.
///
/// Found on the reference corpus, where the pair was 10.11 MB of an 118.49 MB
/// sum. A store collapses inodes, not names, so the estimate has to as well.
#[test]
fn two_package_names_over_one_inode_are_counted_once() {
    let fx = Fixture::new();
    let packages = &[("esbuild", "0.28.2"), ("@esbuild/darwin-arm64", "0.28.2")];

    for app in ["a", "b"] {
        project(&fx, app, &[("package-lock.json", npm_lock(packages))]);
        let binary = fx.file(
            &format!("{app}/node_modules/@esbuild/darwin-arm64/bin/esbuild"),
            &vec![b'x'; 400_000],
        );
        fx.hardlink(&format!("{app}/node_modules/esbuild/bin/esbuild"), &binary);
    }

    let e = run(&fx);
    let per_row: u64 = e.duplicates.rows.iter().map(|d| d.bytes).sum();

    assert_eq!(
        e.duplicates.rows.len(),
        2,
        "both names duplicate across a and b"
    );
    assert!(
        (300_000..600_000).contains(&e.bytes()),
        "two projects hold one 400 KB inode each, so a store recovers one of them: {} bytes",
        e.bytes()
    );
    assert!(
        per_row > e.bytes(),
        "the per-row sum should be the larger, wrong number this guards against: \
         rows {per_row}, estimate {}",
        e.bytes()
    );
}
