//! Fixture trees for scanner tests.
#![allow(dead_code)]

use std::fs;
use std::path::{Path, PathBuf};
use tempfile::TempDir;

pub struct Fixture {
    dir: TempDir,
}

impl Default for Fixture {
    fn default() -> Self {
        Self::new()
    }
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

/// Git fixtures. These shell out to `git` deliberately: the constraint that the
/// tool must not shell out applies to production code, not to test setup.
impl Fixture {
    pub fn git(&self, rel: &str, args: &[&str]) -> String {
        let dir = self.dir.path().join(rel);
        let out = std::process::Command::new("git")
            .current_dir(&dir)
            .args(args)
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_SYSTEM", "/dev/null")
            .output()
            .expect("git");
        assert!(
            out.status.success(),
            "git {args:?} failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8_lossy(&out.stdout).trim().to_string()
    }

    /// A repository with one commit, dated `days_ago`.
    pub fn git_repo(&self, rel: &str, days_ago: u64) -> PathBuf {
        self.file(&format!("{rel}/README.md"), b"hello");
        let dir = self.dir.path().join(rel);
        self.git(rel, &["init", "-q", "-b", "main"]);
        self.git(rel, &["config", "user.email", "t@example.com"]);
        self.git(rel, &["config", "user.name", "Test"]);
        self.git(rel, &["add", "-A"]);

        let when = format!("{} -0000", epoch_days_ago(days_ago));
        let out = std::process::Command::new("git")
            .current_dir(&dir)
            .args(["commit", "-q", "-m", "initial"])
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_SYSTEM", "/dev/null")
            .env("GIT_AUTHOR_DATE", &when)
            .env("GIT_COMMITTER_DATE", &when)
            .output()
            .expect("git commit");
        assert!(out.status.success(), "commit failed");
        dir
    }

    /// Point a remote-tracking ref at HEAD, so the repo looks fully pushed.
    pub fn mark_pushed(&self, rel: &str) {
        let head = self.git(rel, &["rev-parse", "HEAD"]);
        self.git(
            rel,
            &["remote", "add", "origin", "git@example.com:me/repo.git"],
        );
        self.git(rel, &["update-ref", "refs/remotes/origin/main", &head]);
    }

    /// Add a commit that the remote-tracking ref does not contain, dated
    /// `days_ago` so that recency cannot mask the pushed check.
    pub fn commit_unpushed(&self, rel: &str, days_ago: u64) {
        self.file(&format!("{rel}/WIP.md"), b"unpushed work");
        self.git(rel, &["add", "-A"]);

        let when = format!("{} -0000", epoch_days_ago(days_ago));
        let out = std::process::Command::new("git")
            .current_dir(self.dir.path().join(rel))
            .args(["commit", "-q", "-m", "wip"])
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_SYSTEM", "/dev/null")
            .env("GIT_AUTHOR_DATE", &when)
            .env("GIT_COMMITTER_DATE", &when)
            .output()
            .expect("git commit");
        assert!(out.status.success(), "commit failed");

        // The reflog records when the ref moved, which is now regardless of the
        // commit date. Rewrite it so the fixture reflects an old, idle repo.
        let log = self.dir.path().join(rel).join(".git/logs/HEAD");
        let text = std::fs::read_to_string(&log).expect("reflog");
        let stamped = text
            .lines()
            .map(|l| match l.split_once('\t') {
                Some((head, msg)) => {
                    let mut f: Vec<&str> = head.split_whitespace().collect();
                    let ts = epoch_days_ago(days_ago).to_string();
                    let n = f.len();
                    f[n - 2] = &ts;
                    format!("{}\t{msg}", f.join(" "))
                }
                None => l.to_string(),
            })
            .collect::<Vec<_>>()
            .join("\n");
        std::fs::write(&log, format!("{stamped}\n")).expect("rewrite reflog");
    }
}

fn epoch_days_ago(days: u64) -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("epoch")
        .as_secs()
        - days * 86_400
}
