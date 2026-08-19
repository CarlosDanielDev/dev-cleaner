//! Territory: which paths the tool may look at, and which it must never touch.
//!
//! This is the outermost safety boundary. A path outside every root, or inside
//! any denylist entry, is unreachable regardless of what later stages decide.

use std::path::{Path, PathBuf};

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

    /// Configured roots that are not present on disk. Reported rather than
    /// skipped, so a typo in the config does not look like a clean scan.
    pub fn missing_roots(&self) -> Vec<PathBuf> {
        self.roots.iter().filter(|r| !r.exists()).cloned().collect()
    }
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

/// Canonicalise where possible. A path that does not exist cannot be
/// canonicalised, so fall back to the literal form rather than silently
/// treating it as unmatched.
fn canonical(p: &Path) -> PathBuf {
    p.canonicalize().unwrap_or_else(|_| p.to_path_buf())
}
