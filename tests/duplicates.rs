//! The cross-project duplicate report.
//!
//! The number that matters here is what collapsing every copy into one would
//! free, so most of these tests are about the two ways that number turns into a
//! lie: counting all N copies instead of N-1, and counting hardlinked copies
//! that already share their blocks.

pub mod common;

use common::Fixture;
use dev_cleaner::duplicates::{Duplicate, Report, report};
use dev_cleaner::scan::Walker;

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

/// A node project whose lockfile names `packages`, with no install on disk.
fn project(fx: &Fixture, app: &str, packages: &[(&str, &str)]) {
    fx.file(&format!("{app}/package.json"), b"{}");
    fx.file(
        &format!("{app}/package-lock.json"),
        npm_lock(packages).as_bytes(),
    );
}

fn run(fx: &Fixture) -> Report {
    report(&Walker::new([fx.root()]).walk().files)
}

fn row<'a>(r: &'a Report, name: &str) -> Option<&'a Duplicate> {
    r.rows.iter().find(|d| d.name == name)
}

/// One copy has to stay. A report that adds up all N copies promises the whole
/// package back, which deleting duplicates never returns.
#[test]
fn duplicated_bytes_are_the_copies_that_could_go_not_every_copy() {
    let fx = Fixture::new();
    for app in ["a", "b", "c"] {
        project(&fx, app, &[("react", "18.3.1")]);
        fx.file(
            &format!("{app}/node_modules/react/index.js"),
            &vec![b'x'; 400_000],
        );
    }

    let r = run(&fx);
    let d = row(&r, "react").expect("react is installed in three projects");

    assert_eq!(d.projects, 3);
    assert_eq!(d.measured, 3);
    assert!(!d.estimated, "every copy was on disk");
    assert!(
        (800_000..1_200_000).contains(&d.bytes),
        "three copies of 400 KB duplicate two of them, not three: {} bytes",
        d.bytes
    );
}

/// The test this report exists to pass. pnpm, uv and cargo hardlink into a
/// shared store, so the same inode is reachable from every project that holds
/// the package and the blocks are already occupied exactly once. Reporting
/// that as duplicated is space the disk will never return.
#[test]
fn a_package_hardlinked_between_projects_is_not_reported_as_duplicated() {
    let fx = Fixture::new();
    project(&fx, "a", &[("react", "18.3.1")]);
    let real = fx.file("a/node_modules/react/index.js", &vec![b'x'; 400_000]);

    for app in ["b", "c"] {
        project(&fx, app, &[("react", "18.3.1")]);
        fx.hardlink(&format!("{app}/node_modules/react/index.js"), &real);
    }

    let r = run(&fx);

    assert!(
        row(&r, "react").is_none(),
        "one 400 KB inode reachable from three projects occupies the disk once, \
         so nothing is duplicated; got {:?}",
        r.rows
    );
    // Measured and found to duplicate nothing, not skipped for want of a size.
    assert_eq!(
        r.sized, 1,
        "all three copies were on disk and were measured"
    );
    assert_eq!(r.already_shared(), 1);
    assert_eq!(r.unmeasurable(), 0);
}

/// Half shared, half not: a report that treats sharing as all-or-nothing reads
/// one of these two packages wrong.
#[test]
fn a_shared_package_and_an_unshared_one_are_told_apart_in_the_same_scan() {
    let fx = Fixture::new();
    project(&fx, "a", &[("react", "18.3.1"), ("lodash", "4.17.21")]);
    let shared = fx.file("a/node_modules/react/index.js", &vec![b'x'; 400_000]);
    fx.file("a/node_modules/lodash/index.js", &vec![b'y'; 300_000]);

    project(&fx, "b", &[("react", "18.3.1"), ("lodash", "4.17.21")]);
    fx.hardlink("b/node_modules/react/index.js", &shared);
    fx.file("b/node_modules/lodash/index.js", &vec![b'y'; 300_000]);

    let r = run(&fx);

    assert!(
        row(&r, "react").is_none(),
        "react is hardlinked, not duplicated"
    );
    let lodash = row(&r, "lodash").expect("lodash is two independent copies");
    assert!(
        (300_000..500_000).contains(&lodash.bytes),
        "one of two independent 300 KB copies could go: {} bytes",
        lodash.bytes
    );
}

/// The store layout pnpm actually writes: the real files live under
/// `.pnpm/<name>@<version>/node_modules/<name>` and the top-level entry is a
/// symlink. A reader that only knows the flat layout measures nothing here.
#[test]
fn a_package_in_the_pnpm_store_layout_is_found() {
    let fx = Fixture::new();
    for app in ["a", "b"] {
        project(&fx, app, &[("react", "18.3.1")]);
        fx.file(
            &format!("{app}/node_modules/.pnpm/react@18.3.1/node_modules/react/index.js"),
            &vec![b'x'; 400_000],
        );
    }

    let r = run(&fx);
    let d = row(&r, "react").expect("the store layout holds the real files");

    assert_eq!(d.measured, 2);
    assert!(
        (400_000..600_000).contains(&d.bytes),
        "one of two copies could go: {} bytes",
        d.bytes
    );
}

/// A scoped name is two path components, and pnpm escapes the slash in its
/// store directory. Both have to resolve to the same package.
#[test]
fn a_scoped_package_keeps_its_scope() {
    let fx = Fixture::new();
    project(&fx, "a", &[("@babel/core", "7.29.7")]);
    fx.file("a/node_modules/@babel/core/index.js", &vec![b'x'; 200_000]);

    project(&fx, "b", &[("@babel/core", "7.29.7")]);
    fx.file(
        "b/node_modules/.pnpm/@babel+core@7.29.7/node_modules/@babel/core/index.js",
        &vec![b'y'; 200_000],
    );

    let r = run(&fx);
    let d = row(&r, "@babel/core").expect("@babel/core is installed twice");

    assert_eq!(d.measured, 2, "both layouts name the same package");
    assert!((200_000..300_000).contains(&d.bytes), "{} bytes", d.bytes);
}

/// Two versions are two packages. Collapsing them would claim a project can
/// drop a copy it does not have.
#[test]
fn the_same_name_at_two_versions_is_not_a_duplicate() {
    let fx = Fixture::new();
    project(&fx, "a", &[("react", "18.3.1")]);
    fx.file("a/node_modules/react/index.js", &vec![b'x'; 400_000]);
    project(&fx, "b", &[("react", "17.0.2")]);
    fx.file("b/node_modules/react/index.js", &vec![b'y'; 400_000]);

    assert!(
        run(&fx).rows.is_empty(),
        "two versions share a name and nothing else"
    );
}

/// A lockfile is a list of what was resolved, not of what is on disk. A project
/// that never installed holds no bytes, so counting it as a copy would offer
/// space that is not there.
#[test]
fn a_lockfile_without_an_install_holds_no_bytes() {
    let fx = Fixture::new();
    project(&fx, "a", &[("react", "18.3.1")]);
    fx.file("a/node_modules/react/index.js", &vec![b'x'; 400_000]);
    project(&fx, "b", &[("react", "18.3.1")]);

    assert!(
        run(&fx).rows.is_empty(),
        "only one project actually holds react"
    );
}

/// A copy that is on disk but whose bytes cannot be attributed to one version:
/// the project resolved the same name twice, and one set of files covers both.
/// The copy is certainly there, so its size is inferred and the row says so.
#[test]
fn a_copy_that_cannot_be_attributed_to_one_version_is_inferred_and_marked() {
    let fx = Fixture::new();
    for app in ["a", "b"] {
        project(&fx, app, &[("react", "18.3.1")]);
        fx.file(
            &format!("{app}/node_modules/react/index.js"),
            &vec![b'x'; 400_000],
        );
    }
    // c resolved react twice. Its node_modules/react is real, but which of the
    // two versions those bytes belong to is not recorded anywhere here.
    fx.file("c/package.json", b"{}");
    fx.file(
        "c/package-lock.json",
        br#"{
  "lockfileVersion": 3,
  "packages": {
    "": { "version": "1.0.0" },
    "node_modules/react": { "version": "18.3.1" },
    "node_modules/legacy/node_modules/react": { "version": "17.0.2" }
  }
}"#,
    );
    fx.file("c/node_modules/react/index.js", &vec![b'z'; 400_000]);

    let r = run(&fx);
    let d = row(&r, "react").expect("react is held by three projects");

    assert!(d.estimated, "one of the three copies could not be measured");
    assert_eq!(d.measured, 2);
    assert_eq!(d.projects, 3);
    assert!(
        (800_000..1_200_000).contains(&d.bytes),
        "two of three copies could go: {} bytes",
        d.bytes
    );
    assert_eq!(
        r.measured_bytes(),
        0,
        "an estimated row is never added to the measured total"
    );
    assert_eq!(r.estimated_bytes(), d.bytes);
}

/// The correction the real corpus forced. A lockfile records what was resolved,
/// not what was installed: optional and platform-specific packages are resolved
/// everywhere and installed almost nowhere. Inferring a copy from the lockfile
/// alone invented three copies of fsevents that were never on the disk.
#[test]
fn a_package_resolved_but_never_installed_is_not_a_copy() {
    let fx = Fixture::new();
    for app in ["a", "b"] {
        project(&fx, app, &[("react", "18.3.1"), ("fsevents", "2.3.3")]);
        fx.file(
            &format!("{app}/node_modules/react/index.js"),
            &vec![b'x'; 400_000],
        );
        fx.file(
            &format!("{app}/node_modules/fsevents/index.js"),
            &vec![b'f'; 200_000],
        );
    }
    // c installed react but not fsevents, which its lockfile still names.
    project(&fx, "c", &[("react", "18.3.1"), ("fsevents", "2.3.3")]);
    fx.file("c/node_modules/react/index.js", &vec![b'y'; 400_000]);

    let r = run(&fx);

    let fsevents = row(&r, "fsevents").expect("two projects really hold fsevents");
    assert_eq!(
        fsevents.projects, 2,
        "three lockfiles name fsevents and two directories exist; the third is \
         not a copy and must not be estimated as one"
    );
    assert!(!fsevents.estimated);
    assert!(
        (200_000..300_000).contains(&fsevents.bytes),
        "one of two copies could go: {} bytes",
        fsevents.bytes
    );

    let react = row(&r, "react").expect("three projects hold react");
    assert_eq!(react.projects, 3);
    assert!(!react.estimated);
}

/// Cargo, poetry and bundler install outside the project, so no copy of theirs
/// is measurable from a project root. Omitted, and said out loud.
#[test]
fn a_package_that_cannot_be_measured_anywhere_is_omitted_not_counted_as_zero() {
    let fx = Fixture::new();
    for app in ["a", "b"] {
        fx.file(
            &format!("{app}/Cargo.lock"),
            b"[[package]]\nname = \"serde\"\nversion = \"1.0.0\"\n",
        );
    }

    let r = run(&fx);

    assert!(row(&r, "serde").is_none(), "no copy of serde was measured");
    assert_eq!(
        r.unmeasurable(),
        1,
        "a package that could not be sized is named in the summary, never \
         rendered as zero duplicated bytes"
    );
    assert_eq!(r.shared, 1);
    assert_eq!(r.sized, 0);
}

/// A lockfile that will not parse is a project missing from the report. Silence
/// reads as "there was nothing there".
#[test]
fn a_lockfile_that_will_not_parse_is_named_in_a_warning() {
    let fx = Fixture::new();
    fx.file("broken/package.json", b"{}");
    fx.file("broken/package-lock.json", b"{ not json");

    let r = run(&fx);

    assert_eq!(r.warnings.len(), 1, "one unreadable lockfile, one warning");
    assert!(
        r.warnings[0].contains("broken/package-lock.json"),
        "the warning names the project: {}",
        r.warnings[0]
    );
}

#[test]
fn rows_are_sorted_by_duplicated_bytes_descending() {
    let fx = Fixture::new();
    for app in ["a", "b"] {
        project(&fx, app, &[("small", "1.0.0"), ("big", "1.0.0")]);
        fx.file(
            &format!("{app}/node_modules/small/index.js"),
            &vec![b'x'; 100_000],
        );
        fx.file(
            &format!("{app}/node_modules/big/index.js"),
            &vec![b'y'; 900_000],
        );
    }

    let r = run(&fx);
    let order: Vec<&str> = r.rows.iter().map(|d| d.name.as_str()).collect();

    assert_eq!(order, ["big", "small"], "largest duplication first");
}

/// The independent check. `du -sk` deduplicates hardlinks the same way, so the
/// copy that could go has to weigh what `du` says one copy weighs.
#[test]
fn the_duplicated_total_agrees_with_du() {
    let fx = Fixture::new();
    for app in ["a", "b"] {
        project(&fx, app, &[("react", "18.3.1")]);
        fx.file(
            &format!("{app}/node_modules/react/index.js"),
            &vec![b'x'; 300_000],
        );
        fx.file(
            &format!("{app}/node_modules/react/cjs/react.js"),
            &vec![b'y'; 120_000],
        );
    }

    let r = run(&fx);
    let d = row(&r, "react").expect("react is installed twice");

    let out = std::process::Command::new("du")
        .args([
            "-sk",
            &fx.root().join("a/node_modules/react").to_string_lossy(),
        ])
        .output()
        .expect("du");
    let kb: u64 = String::from_utf8_lossy(&out.stdout)
        .split_whitespace()
        .next()
        .expect("du output")
        .parse()
        .expect("kilobytes");
    let du_bytes = kb * 1024;

    // du counts the directories themselves; the walk only counts files, so the
    // report can read a little lower but never higher.
    assert!(
        d.bytes <= du_bytes && du_bytes - d.bytes < 64 * 1024,
        "one redundant copy is {} bytes, du says the copy is {du_bytes}",
        d.bytes
    );
}
