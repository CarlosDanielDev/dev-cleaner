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
