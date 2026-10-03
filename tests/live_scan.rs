//! The walk as the interface lives with it: stoppable, countable, and
//! indifferent to what it was told to skip.

mod common;

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use common::Fixture;
use dev_cleaner::config::Config;
use dev_cleaner::scan::{Progress, Unreadable, UnreadableKind, Walker};

/// `n` empty files in one directory, which the walker reads and stats in one call.
fn one_huge_directory(fx: &Fixture, n: usize) {
    let dir = fx.root().join("huge");
    fs::create_dir_all(&dir).expect("mkdir");
    for i in 0..n {
        fs::write(dir.join(format!("f{i}")), b"").expect("write");
    }
}

#[test]
fn cancel_stops_a_walk_inside_one_huge_directory_in_a_tenth_of_a_second() {
    let fx = Fixture::new();
    one_huge_directory(&fx, 100_000);
    let progress = Arc::new(Progress::default());

    let handle = {
        let progress = Arc::clone(&progress);
        let roots = fx.root().to_path_buf();
        std::thread::spawn(move || Walker::new([roots]).walk_with(&progress))
    };

    // Let the walk get into the directory, then ask it to stop.
    let deadline = Instant::now() + Duration::from_secs(60);
    while progress.entries.load(Ordering::Relaxed) < 2_000 {
        assert!(Instant::now() < deadline, "the walk never started");
        std::thread::sleep(Duration::from_millis(1));
    }
    let asked = Instant::now();
    progress.cancel();
    let result = handle.join().expect("the walk");
    let took = asked.elapsed();

    assert!(result.cancelled, "the result says it was cut short");
    assert!(
        result.files.len() < 100_000,
        "a cancelled walk must not have finished the directory ({} files)",
        result.files.len()
    );
    assert!(
        took < Duration::from_millis(100),
        "stopped {took:?} after the request"
    );
}

#[test]
fn a_walk_nobody_cancels_is_not_marked_cancelled() {
    let fx = Fixture::new();
    fx.file("a/one", b"1");
    let result = Walker::new([fx.root()]).walk();
    assert!(!result.cancelled);
    assert_eq!(result.files.len(), 1);
}

/// The old way: walk everything, then drop what the denylist names.
fn after_the_walk(fx: &Fixture, cfg: &Config) -> Vec<PathBuf> {
    let mut found: Vec<PathBuf> = Walker::new([fx.root()])
        .walk()
        .files
        .into_iter()
        .filter(|f| !cfg.is_denied(&f.path))
        .map(|f| f.path)
        .collect();
    found.sort();
    found
}

#[test]
fn a_denylist_applied_by_the_walker_leaves_what_the_old_filter_left() {
    let fx = Fixture::new();
    fx.file("keep/a.rs", b"a");
    fx.file("keep/deep/b.rs", b"b");
    fx.file("secret/x", b"x");
    fx.file("secret/deep/y", b"y");
    fx.file("also-secret.txt", b"z");
    fx.file("keep/nope/inner", b"i");
    // A symlink into a denied directory is not followed, so it adds nothing.
    fx.symlink_to("keep/link", &fx.root().join("secret"));
    // A name that merely starts like a denied one is not denied.
    fx.file("secretary/ok", b"ok");

    let cfg = Config {
        roots: vec![fx.root().to_path_buf()],
        caches: Vec::new(),
        denylist: vec![
            fx.root().join("secret"),
            fx.root().join("also-secret.txt"),
            fx.root().join("keep/nope"),
            // Spelled the long way round: it resolves to the same place.
            fx.root().join("keep/../keep/deep/../nope"),
        ],
    };

    let old = after_the_walk(&fx, &cfg);
    let denylist = cfg.clone();
    let mut new: Vec<PathBuf> = Walker::new([fx.root()])
        .skipping(move |p| denylist.is_denied(p))
        .walk()
        .files
        .into_iter()
        .map(|f| f.path)
        .collect();
    new.sort();

    assert_eq!(new, old);
    assert!(new.iter().any(|p| p.ends_with("secretary/ok")));
    assert!(!new.iter().any(|p| p.ends_with("secret/x")));
}

#[test]
fn a_denied_directory_is_not_entered_at_all() {
    let fx = Fixture::new();
    fx.file("keep/a", b"a");
    for i in 0..50 {
        fx.file(&format!("secret/f{i}"), b"x");
    }
    let progress = Arc::new(Progress::default());
    let denied = fx.root().join("secret");
    Walker::new([fx.root()])
        .skipping(move |p| p.starts_with(&denied))
        .walk_with(&progress);

    assert_eq!(
        progress.entries.load(Ordering::Relaxed),
        1,
        "a denied tree must not even be counted: it would be stat'd, and read"
    );
}

#[test]
fn an_unreadable_folder_is_counted_by_kind_and_does_not_stop_the_walk() {
    let fx = Fixture::new();
    fx.file("open/a", b"a");
    fx.file("locked/inner/b", b"b");
    let locked = fx.root().join("locked");
    fs::set_permissions(&locked, fs::Permissions::from_mode(0o000)).expect("chmod");

    let progress = Arc::new(Progress::default());
    let result = Walker::new([fx.root()]).walk_with(&progress);
    // Given back so the fixture can be removed.
    fs::set_permissions(&locked, fs::Permissions::from_mode(0o755)).expect("chmod");

    assert_eq!(result.files.len(), 1, "the readable half is still walked");
    let counted = progress.read().unreadable;
    assert_eq!(counted, 1, "one folder could not be read");
    assert_eq!(
        progress.unreadable(),
        Unreadable {
            permission: 1,
            ..Unreadable::default()
        }
    );
}

#[test]
fn errors_are_sorted_into_kinds() {
    use std::io::ErrorKind::*;
    assert_eq!(
        UnreadableKind::of_io(PermissionDenied),
        UnreadableKind::Permission
    );
    assert_eq!(UnreadableKind::of_io(NotFound), UnreadableKind::Vanished);
    assert_eq!(UnreadableKind::of_io(Other), UnreadableKind::Other);
}

#[test]
fn the_walk_keeps_a_count_per_top_level_folder_and_says_when_each_is_done() {
    let fx = Fixture::new();
    fx.file("top.txt", b"loose");
    fx.file("a/one", b"1");
    fx.file("a/deep/er/two", b"2");
    fx.file("b/three", b"3");
    fx.file("c/empty-dir-below/.keep", b"");
    let progress = Arc::new(Progress::default());

    Walker::new([fx.root()]).walk_with(&progress);
    let reading = progress.read();
    let tops = reading.tops.expect("nobody holds the lock after the walk");

    let folders: Vec<_> = tops.iter().filter(|t| t.folder).collect();
    assert_eq!(folders.len(), 3);
    assert!(folders.iter().all(|t| t.done), "{folders:?}");
    let by = |name: &str| {
        tops.iter()
            .find(|t| t.path == fx.root().join(name))
            .map_or(0, |t| t.entries)
    };
    assert_eq!((by("a"), by("b"), by("c")), (2, 1, 1));
    // Files directly in the root are a bucket of their own, not a folder.
    let loose = tops
        .iter()
        .find(|t| !t.folder)
        .expect("the root's own files");
    assert_eq!((loose.path.as_path(), loose.entries), (fx.root(), 1));
    assert_eq!(reading.entries, 5);
    assert_eq!(
        progress.shape().iter().map(|(_, n)| n).sum::<u64>(),
        5,
        "what the store keeps adds up to what was counted"
    );
}

#[test]
fn project_markers_are_counted_once_per_directory_and_not_inside_build_output() {
    let fx = Fixture::new();
    fx.file("app/package.json", b"{}");
    fx.file("app/Cargo.toml", b"");
    fx.file("app/src/main.rs", b"");
    fx.file("lib/go.mod", b"");
    // A dependency's own manifest is not a project.
    fx.file("app/node_modules/dep/package.json", b"{}");
    let progress = Arc::new(Progress::default());

    Walker::new([fx.root()]).walk_with(&progress);

    assert_eq!(progress.read().projects, 2);
}
