//! Territory: which paths the tool may look at, and which it must never touch.
//!
//! This is the outermost safety boundary. A path outside every root, or inside
//! any denylist entry, is unreachable regardless of what later stages decide.

use std::path::{Component, Path, PathBuf};
use std::time::Duration;

use serde::Deserialize;

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(default)]
pub struct Config {
    /// Directories to scan for projects.
    pub roots: Vec<PathBuf>,
    /// Ecosystem cache registries to include, by name.
    pub caches: Vec<String>,
    /// Paths that must never be offered, whatever else concludes.
    pub denylist: Vec<PathBuf>,
}

impl Default for Config {
    fn default() -> Self {
        // An empty root set would scan nothing while reporting success, so the
        // default points somewhere real.
        Self {
            roots: vec![home().join("projects")],
            caches: vec![
                "npm".into(),
                "cargo".into(),
                "go".into(),
                "xcode".into(),
                "gradle".into(),
                "cocoapods".into(),
                "pnpm".into(),
            ],
            denylist: Vec::new(),
        }
    }
}

impl Config {
    /// Load from disk. A missing file yields defaults rather than an error:
    /// a first run should work without setup.
    pub fn load(path: &Path) -> Result<Self, toml::de::Error> {
        let Ok(text) = std::fs::read_to_string(path) else {
            return Ok(Self::default());
        };
        let mut cfg: Config = toml::from_str(&text)?;
        cfg.roots = cfg.roots.iter().map(|p| expand_tilde(p)).collect();
        cfg.denylist = cfg.denylist.iter().map(|p| expand_tilde(p)).collect();
        Ok(cfg)
    }

    /// Whether `path` falls inside any denylist entry.
    ///
    /// Both sides are canonicalised first, so `a/../denied/x` is recognised as
    /// the denied location it actually resolves to.
    pub fn is_denied(&self, path: &Path) -> bool {
        let target = canonical(path);
        self.denylist
            .iter()
            .map(|d| canonical(d))
            .any(|denied| target.starts_with(&denied))
    }

    /// A denylist test for paths a walk of `roots` produced, resolved once.
    ///
    /// [`Config::is_denied`] canonicalises the path and every denylist entry on
    /// every call, a dozen `lstat`s per file: minutes over a million entries.
    /// The walker never follows a symlink and only yields regular files, so
    /// below its root a path has no symlink component, and its canonical form
    /// is the root's canonical form plus the relative part. Resolve the roots
    /// and the denylist once and each test is a prefix comparison.
    ///
    /// Anything else (a path under no root, or with `.`/`..` below its root)
    /// goes through [`Config::is_denied`]'s own resolution, so the answer is
    /// the same for every path, only the walker's are cheap.
    pub fn denier(&self, roots: &[PathBuf]) -> Denier {
        let mut roots: Vec<(PathBuf, PathBuf)> =
            roots.iter().map(|r| (r.clone(), canonical(r))).collect();
        // Most specific first, so a root inside another root resolves through itself.
        roots.sort_by_key(|(root, _)| std::cmp::Reverse(root.components().count()));
        Denier {
            denylist: self.denylist.iter().map(|d| canonical(d)).collect(),
            roots,
        }
    }

    /// Configured roots that are not present on disk. Reported rather than
    /// skipped, so a typo in the config does not look like a clean scan.
    pub fn missing_roots(&self) -> Vec<PathBuf> {
        self.roots.iter().filter(|r| !r.exists()).cloned().collect()
    }
}

/// The `theme = "..."` of the config at `path`, if there is a readable file with
/// a string under that key. Read on its own, so the theme is a setting of the
/// interface and never a field every `Config` has to be built with.
pub fn theme_setting(path: &Path) -> Option<String> {
    let table: toml::Table = toml::from_str(&std::fs::read_to_string(path).ok()?).ok()?;
    table.get("theme")?.as_str().map(str::to_string)
}

/// How long the last complete scan of the same roots must have taken before a
/// scan again asks first. Under it the scan simply starts: a dialog on every
/// cheap action teaches the owner to press `Enter` without reading.
pub const CONFIRM_RESCAN_AFTER: Duration = Duration::from_secs(10);

/// Whether a scan started from inside the interface asks first, and from what
/// cost on. The command line never asks, whatever this says.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RescanPolicy {
    /// `confirm_rescan` in the config; on by default.
    pub confirm: bool,
    /// `confirm_rescan_after_secs` in the config.
    pub threshold: Duration,
}

impl Default for RescanPolicy {
    fn default() -> Self {
        Self {
            confirm: true,
            threshold: CONFIRM_RESCAN_AFTER,
        }
    }
}

/// The rescan keys of the config at `path`. Read on its own, like
/// [`theme_setting`]: they are settings of the interface and no `Config` has to
/// be built with them. A missing file, or a key of the wrong type, is the
/// default: a typo must not switch off the question.
pub fn rescan_policy(path: &Path) -> RescanPolicy {
    let table: toml::Table = std::fs::read_to_string(path)
        .ok()
        .and_then(|text| toml::from_str(&text).ok())
        .unwrap_or_default();
    let mut policy = RescanPolicy::default();
    if let Some(confirm) = table.get("confirm_rescan").and_then(toml::Value::as_bool) {
        policy.confirm = confirm;
    }
    if let Some(secs) = table
        .get("confirm_rescan_after_secs")
        .and_then(toml::Value::as_integer)
        .and_then(|secs| u64::try_from(secs).ok())
    {
        policy.threshold = Duration::from_secs(secs);
    }
    policy
}

fn home() -> PathBuf {
    PathBuf::from(std::env::var("HOME").unwrap_or_else(|_| "/".into()))
}

fn expand_tilde(p: &Path) -> PathBuf {
    match p.strip_prefix("~") {
        Ok(rest) => home().join(rest),
        Err(_) => p.to_path_buf(),
    }
}

/// See [`Config::denier`].
pub struct Denier {
    denylist: Vec<PathBuf>,
    /// Each root as given, with its canonical form; longest root first.
    roots: Vec<(PathBuf, PathBuf)>,
}

impl Denier {
    /// Same answer as [`Config::is_denied`], with no syscall for a walked path.
    pub fn is_denied(&self, path: &Path) -> bool {
        if self.denylist.is_empty() {
            return false;
        }
        let resolved = self.roots.iter().find_map(|(root, canon)| {
            let rel = path.strip_prefix(root).ok()?;
            rel.components()
                .all(|c| matches!(c, Component::Normal(_)))
                .then(|| canon.join(rel))
        });
        let target = resolved.unwrap_or_else(|| canonical(path));
        self.denylist
            .iter()
            .any(|denied| target.starts_with(denied))
    }
}

/// Canonicalise where possible. A path that does not exist cannot be
/// canonicalised, so fall back to the literal form rather than silently
/// treating it as unmatched.
fn canonical(p: &Path) -> PathBuf {
    p.canonicalize().unwrap_or_else(|_| p.to_path_buf())
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::os::unix::fs::symlink;

    use super::*;
    use crate::scan::Walker;

    /// A tree behind a symlinked root, with a denied directory, a denylist entry
    /// that is itself a symlink, and a way back in through `..`.
    struct Tree {
        _dir: tempfile::TempDir,
        link_root: PathBuf,
        real_root: PathBuf,
    }

    fn tree() -> Tree {
        let dir = tempfile::tempdir().unwrap();
        let real_root = dir.path().canonicalize().unwrap().join("real");
        for f in [
            "keep/a.txt",
            "keep/sub/b.txt",
            "denied/c.txt",
            "denied/sub/d.txt",
            "hidden/e.txt",
            "app/app2/f.txt",
            "app2/g.txt",
        ] {
            let p = real_root.join(f);
            fs::create_dir_all(p.parent().unwrap()).unwrap();
            fs::write(p, b"x").unwrap();
        }
        symlink(real_root.join("hidden"), real_root.join("alias")).unwrap();
        symlink(real_root.join("denied"), real_root.join("keep/into-denied")).unwrap();
        let link_root = dir.path().join("link");
        symlink(&real_root, &link_root).unwrap();
        Tree {
            _dir: dir,
            link_root,
            real_root,
        }
    }

    fn config(denylist: Vec<PathBuf>) -> Config {
        Config {
            roots: Vec::new(),
            caches: Vec::new(),
            denylist,
        }
    }

    /// Every denylist shape worth distinguishing, for a root named `base`.
    fn denylists(base: &Path) -> Vec<Vec<PathBuf>> {
        vec![
            vec![],
            vec![base.join("denied")],
            vec![base.join("denied/")],
            vec![base.join("alias")],
            vec![base.join("nope")],
            vec![base.join("keep/sub/../../denied")],
            vec![base.join("app")],
            vec![base.join("denied"), base.join("alias"), base.join("nope")],
        ]
    }

    #[test]
    fn denier_agrees_with_is_denied_on_every_walked_path() {
        let t = tree();
        for base in [&t.link_root, &t.real_root] {
            let roots = vec![base.clone()];
            let walked: Vec<PathBuf> = Walker::new(&roots)
                .walk()
                .files
                .into_iter()
                .map(|f| f.path)
                .collect();
            assert!(walked.len() >= 7, "the walk must see the fixture");
            for list in denylists(base) {
                let cfg = config(list.clone());
                let denier = cfg.denier(&roots);
                for p in &walked {
                    assert_eq!(
                        denier.is_denied(p),
                        cfg.is_denied(p),
                        "{} under denylist {list:?}",
                        p.display()
                    );
                }
            }
        }
    }

    #[test]
    fn denier_agrees_with_is_denied_on_paths_the_walk_does_not_produce() {
        // Not covered, by design: a path with a symlink component below its
        // root (`root/alias/e.txt`). The walker never yields one, and telling
        // it apart from a real directory is the syscall `Denier` exists to skip.
        let t = tree();
        let roots = vec![t.link_root.clone()];
        let outside = tempfile::tempdir().unwrap();
        fs::write(outside.path().join("o.txt"), b"x").unwrap();
        for base in [&t.link_root, &t.real_root] {
            let paths = [
                base.join("keep/sub/../../denied/c.txt"),
                base.join("keep/../denied/sub/d.txt"),
                base.join("./denied/c.txt"),
                base.join("denied/"),
                base.join("denied"),
                base.join("keep/a.txt"),
                base.join("app/app2/f.txt"),
                base.join("app2/g.txt"),
                base.to_path_buf(),
                outside.path().join("o.txt"),
                outside.path().join("missing.txt"),
                PathBuf::from("/"),
            ];
            for list in denylists(base) {
                let cfg = config(list.clone());
                let denier = cfg.denier(&roots);
                for p in &paths {
                    assert_eq!(
                        denier.is_denied(p),
                        cfg.is_denied(p),
                        "{} under denylist {list:?}",
                        p.display()
                    );
                }
            }
        }
    }

    #[test]
    fn denier_prefers_the_most_specific_root() {
        // A root that is a symlink inside another root: its files must resolve
        // through it, not through the outer root's spelling.
        let t = tree();
        let inner = t.link_root.join("keep/into-denied");
        let roots = vec![t.link_root.clone(), inner.clone()];
        let cfg = config(vec![t.real_root.join("denied")]);
        let denier = cfg.denier(&roots);
        let p = inner.join("c.txt");
        assert!(cfg.is_denied(&p), "oracle: the file is in the denied tree");
        assert!(denier.is_denied(&p));
    }

    #[test]
    fn denier_decides_without_touching_the_filesystem() {
        let t = tree();
        let roots = vec![t.link_root.clone()];
        let cfg = config(vec![t.real_root.join("denied"), t.real_root.join("alias")]);
        let denier = cfg.denier(&roots);
        // Resolution happened up front: with the tree gone, a syscall per path
        // would resolve nothing and answer differently.
        fs::remove_dir_all(&t.real_root).unwrap();
        assert!(denier.is_denied(&t.link_root.join("denied/c.txt")));
        assert!(denier.is_denied(&t.link_root.join("hidden/e.txt")));
        assert!(!denier.is_denied(&t.link_root.join("keep/a.txt")));
    }
}
