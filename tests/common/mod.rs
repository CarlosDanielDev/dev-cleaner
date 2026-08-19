//! Fixture trees for scanner tests.
#![allow(dead_code)]

use std::fs;
use std::path::{Path, PathBuf};
use tempfile::TempDir;

pub struct Fixture {
    dir: TempDir,
}

impl Fixture {
    pub fn new() -> Self {
        Self {
            dir: TempDir::new().expect("tempdir"),
        }
    }

    pub fn root(&self) -> &Path {
        self.dir.path()
    }

    /// Write a file, creating parent directories as needed.
    pub fn file(&self, rel: &str, contents: &[u8]) -> PathBuf {
        let p = self.dir.path().join(rel);
        fs::create_dir_all(p.parent().expect("parent")).expect("mkdir");
        fs::write(&p, contents).expect("write");
        p
    }

    /// A directory that `.gitignore` hides, holding one file.
    pub fn gitignored_artifact(&self, dir: &str) -> PathBuf {
        self.file(".gitignore", format!("{dir}\n").as_bytes());
        self.file(&format!("{dir}/pkg/index.js"), b"console.log(1)");
        self.dir.path().join(dir)
    }
}

impl Fixture {
    /// A symlink inside the root pointing at `target`, which lives outside it.
    pub fn symlink_to(&self, rel: &str, target: &Path) -> PathBuf {
        let link = self.dir.path().join(rel);
        fs::create_dir_all(link.parent().expect("parent")).expect("mkdir");
        std::os::unix::fs::symlink(target, &link).expect("symlink");
        link
    }
}

impl Fixture {
    /// A sparse file: `logical` bytes long, but almost no blocks allocated.
    pub fn sparse_file(&self, rel: &str, logical: u64) -> PathBuf {
        use std::io::{Seek, SeekFrom, Write};
        let p = self.dir.path().join(rel);
        fs::create_dir_all(p.parent().expect("parent")).expect("mkdir");
        let mut f = fs::File::create(&p).expect("create");
        f.seek(SeekFrom::Start(logical - 1)).expect("seek");
        f.write_all(b"\0").expect("write");
        f.sync_all().expect("sync");
        p
    }
}

impl Fixture {
    /// A hardlink at `rel` pointing at the same inode as `target`.
    pub fn hardlink(&self, rel: &str, target: &Path) -> PathBuf {
        let p = self.dir.path().join(rel);
        fs::create_dir_all(p.parent().expect("parent")).expect("mkdir");
        fs::hard_link(target, &p).expect("hardlink");
        p
    }
}
