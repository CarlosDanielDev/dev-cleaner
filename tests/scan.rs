mod common;

use common::Fixture;
use dev_cleaner::scan::Walker;

#[test]
fn walks_into_gitignored_directories() {
    let fx = Fixture::new();
    fx.gitignored_artifact("node_modules");

    let result = Walker::new([fx.root()]).walk();

    let found = result
        .files
        .iter()
        .any(|f| f.path.ends_with("node_modules/pkg/index.js"));

    assert!(
        found,
        "walker must not honour .gitignore; saw {:?}",
        result.files.iter().map(|f| &f.path).collect::<Vec<_>>()
    );
}

#[test]
fn does_not_follow_symlinks_out_of_the_root() {
    let outside = Fixture::new();
    outside.file("secret/treasure.txt", b"do not walk me");

    let fx = Fixture::new();
    fx.file("real.txt", b"walk me");
    fx.symlink_to("escape", outside.root());

    let result = Walker::new([fx.root()]).walk();

    let escaped = result
        .files
        .iter()
        .any(|f| f.path.to_string_lossy().contains("treasure.txt"));

    assert!(!escaped, "walker followed a symlink out of the root");
    assert!(
        result.files.iter().any(|f| f.path.ends_with("real.txt")),
        "walker should still see real files"
    );
}

#[test]
fn reports_allocated_blocks_not_apparent_length() {
    let fx = Fixture::new();
    // 64 MiB long, one byte written. Apparent size lies; st_blocks tells the truth.
    fx.sparse_file("disk.raw", 64 * 1024 * 1024);

    let result = Walker::new([fx.root()]).walk();
    let meta = result
        .files
        .iter()
        .find(|f| f.path.ends_with("disk.raw"))
        .expect("sparse file not found");

    assert_eq!(
        meta.bytes_apparent,
        64 * 1024 * 1024,
        "apparent size should be the logical length"
    );
    assert!(
        meta.bytes_actual < meta.bytes_apparent / 100,
        "actual should be a tiny fraction of apparent; got actual={} apparent={}",
        meta.bytes_actual,
        meta.bytes_apparent
    );
}

#[test]
fn counts_hardlinked_content_once() {
    const MIB: u64 = 1024 * 1024;
    let fx = Fixture::new();
    // One megabyte of real content, reachable under two paths - the pnpm store shape.
    let store = fx.file("store/react@18.2.0/index.js", &vec![b'x'; MIB as usize]);
    fx.hardlink("project-a/node_modules/react/index.js", &store);
    fx.hardlink("project-b/node_modules/react/index.js", &store);

    let usage = dev_cleaner::scan::Usage::of(&Walker::new([fx.root()]).walk().files);

    assert!(
        usage.bytes_apparent >= 3 * MIB,
        "apparent should count every path: got {}",
        usage.bytes_apparent
    );
    assert!(
        usage.bytes_unique < 2 * MIB,
        "unique should count the inode once: got {}",
        usage.bytes_unique
    );
}

#[test]
fn inode_count_is_distinct_from_path_count() {
    let fx = Fixture::new();
    let a = fx.file("a.txt", b"content");
    fx.file("b.txt", b"other");
    fx.hardlink("c.txt", &a);

    let usage = dev_cleaner::scan::Usage::of(&Walker::new([fx.root()]).walk().files);

    assert_eq!(usage.files, 3, "three directory entries exist");
    assert_eq!(usage.inodes, 2, "but only two distinct inodes");
}

#[test]
fn unreadable_directories_are_reported_not_fatal() {
    use std::os::unix::fs::PermissionsExt;

    let fx = Fixture::new();
    fx.file("readable.txt", b"fine");
    fx.file("locked/hidden.txt", b"nope");
    let locked = fx.root().join("locked");
    fs_set_mode(&locked, 0o000);

    let result = Walker::new([fx.root()]).walk();

    // Restore before assertions so the tempdir can clean up even on failure.
    fs_set_mode(&locked, 0o755);

    assert!(
        result
            .files
            .iter()
            .any(|f| f.path.ends_with("readable.txt")),
        "walk continued past the unreadable directory"
    );
    assert!(
        !result.errors.is_empty(),
        "the permission failure should be recorded, not swallowed"
    );

    fn fs_set_mode(p: &std::path::Path, mode: u32) {
        let mut perms = std::fs::metadata(p).expect("metadata").permissions();
        perms.set_mode(mode);
        std::fs::set_permissions(p, perms).expect("chmod");
    }
}

#[test]
fn records_modification_time_for_activity_classification() {
    let fx = Fixture::new();
    fx.file("recent.txt", b"just written");

    let result = Walker::new([fx.root()]).walk();
    let meta = result
        .files
        .iter()
        .find(|f| f.path.ends_with("recent.txt"))
        .expect("file");

    let age = std::time::SystemTime::now()
        .duration_since(meta.mtime)
        .expect("mtime must not be in the future");
    assert!(age.as_secs() < 60, "a just-written file should look recent");
}

#[test]
fn progress_matches_what_the_walk_returns() {
    use dev_cleaner::scan::Progress;
    use std::sync::Arc;
    use std::sync::atomic::Ordering;

    // Two roots, so a counter reset between roots would show up as a shortfall.
    let a = Fixture::new();
    let b = Fixture::new();
    for i in 0..12u64 {
        a.file(
            &format!("d{}/f{i}", i % 3),
            &vec![b'a'; (i as usize + 1) * 1024],
        );
        b.file(&format!("f{i}"), &vec![b'b'; 4096]);
    }
    a.file("empty", b"");

    let progress = Arc::new(Progress::default());
    let result = Walker::new([a.root(), b.root()]).walk_with(&progress);

    assert_eq!(
        progress.entries.load(Ordering::Relaxed),
        result.files.len() as u64,
        "entries counted during the walk should equal the files it returned"
    );
    assert_eq!(
        progress.bytes.load(Ordering::Relaxed),
        result.files.iter().map(|f| f.bytes_actual).sum::<u64>(),
        "bytes counted during the walk should equal the sum of bytes_actual"
    );
}

#[test]
fn progress_never_goes_backwards_while_the_walk_runs() {
    use dev_cleaner::scan::Progress;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::time::Duration;

    const FILES: u64 = 5_000;
    let fx = Fixture::new();
    for i in 0..FILES {
        fx.file(&format!("d{:02}/f{i}", i % 50), b"x");
    }

    let progress = Arc::new(Progress::default());
    let done = AtomicBool::new(false);

    let (result, samples) = std::thread::scope(|s| {
        let sampler = s.spawn(|| {
            let mut samples = Vec::new();
            while !done.load(Ordering::Relaxed) {
                samples.push((
                    progress.entries.load(Ordering::Relaxed),
                    progress.bytes.load(Ordering::Relaxed),
                ));
                std::thread::sleep(Duration::from_millis(1));
            }
            samples
        });
        let result = Walker::new([fx.root()]).walk_with(&progress);
        done.store(true, Ordering::Relaxed);
        (result, sampler.join().expect("sampler thread"))
    });

    for pair in samples.windows(2) {
        let (before, after) = (pair[0], pair[1]);
        assert!(
            after.0 >= before.0 && after.1 >= before.1,
            "counters went backwards: {before:?} then {after:?}"
        );
    }
    assert_eq!(result.files.len() as u64, FILES);
    assert_eq!(progress.entries.load(Ordering::Relaxed), FILES);
}
