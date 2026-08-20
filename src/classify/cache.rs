use std::path::{Path, PathBuf};

use crate::scan::{Usage, Walker};

/// A toolchain cache that lives outside any project.
///
/// These hold the larger share of recoverable space: on the reference machine
/// they accounted for roughly 8 GB against 3 GB inside projects.
#[derive(Debug, Clone, Copy)]
pub struct CacheKind {
    pub name: &'static str,
    /// Location relative to the user's home directory.
    pub rel_path: &'static str,
    /// How the cache comes back. Never empty.
    pub regen: &'static str,
    /// The toolchain's own cleanup command, where one exists.
    ///
    /// Preferred over deleting paths ourselves: the tool that created the cache
    /// understands its own layout, and its cleanup path is already designed to
    /// be safe.
    pub cleanup: Option<&'static str>,
}

impl CacheKind {
    const fn new(
        name: &'static str,
        rel_path: &'static str,
        regen: &'static str,
        cleanup: Option<&'static str>,
    ) -> Self {
        assert!(!name.is_empty(), "cache kind must be named");
        assert!(!rel_path.is_empty(), "cache kind must declare a location");
        assert!(
            !regen.is_empty(),
            "every cache kind must declare how it is regenerated"
        );
        Self {
            name,
            rel_path,
            regen,
            cleanup,
        }
    }
}

/// The one place a new cache is added. Sizes noted are what the reference
/// machine held before its first clean.
const REGISTRY: &[CacheKind] = &[
    // 2.9 GB. Fails with "directory not empty" when the cache is read-only,
    // so write permissions must be restored before the command will work.
    CacheKind::new(
        "go",
        "go/pkg/mod",
        "re-downloaded on next build",
        Some("go clean -modcache"),
    ),
    // 1.2 GB
    CacheKind::new(
        "npm",
        ".npm/_cacache",
        "re-downloaded on next install",
        Some("npm cache clean --force"),
    ),
    // 0.9 GB
    CacheKind::new(
        "cargo",
        ".cargo/registry",
        "re-downloaded on next build",
        None,
    ),
    CacheKind::new(
        "pnpm",
        "Library/pnpm/store",
        "re-downloaded on next install",
        Some("pnpm store prune"),
    ),
    // 0.3 GB
    CacheKind::new(
        "xcode-derived",
        "Library/Developer/Xcode/DerivedData",
        "rebuilt by Xcode",
        None,
    ),
    // 11.1 GB across two near-identical iOS versions.
    CacheKind::new(
        "xcode-devicesupport",
        "Library/Developer/Xcode/iOS DeviceSupport",
        "regenerated when a device is next connected",
        None,
    ),
    // 5.3 GB
    CacheKind::new(
        "xcode-previews",
        "Library/Developer/Xcode/UserData/Previews",
        "regenerated on next SwiftUI preview",
        None,
    ),
    CacheKind::new(
        "gradle",
        ".gradle/caches",
        "re-downloaded on next build",
        None,
    ),
    CacheKind::new("cocoapods", "Library/Caches/CocoaPods", "pod install", None),
    CacheKind::new(
        "playwright",
        "Library/Caches/ms-playwright",
        "npx playwright install",
        None,
    ),
    // 0.1 GB
    CacheKind::new(
        "homebrew",
        "Library/Caches/Homebrew",
        "re-downloaded on next install",
        Some("brew cleanup -s"),
    ),
];

pub fn cache_kinds() -> &'static [CacheKind] {
    REGISTRY
}

/// A cache that is actually present on this machine.
#[derive(Debug, Clone)]
pub struct CacheEntry {
    pub kind: &'static CacheKind,
    pub path: PathBuf,
}

impl CacheEntry {
    /// Measure what this cache currently occupies.
    pub fn usage(&self) -> Usage {
        Usage::of(&Walker::new([&self.path]).walk().files)
    }
}

/// Locate the enabled caches under `home`.
///
/// A cache that is not installed is simply absent from the result. A machine
/// without Go or Xcode is normal, not an error condition.
pub fn probe_caches(home: &Path, enabled: &[String]) -> Vec<CacheEntry> {
    REGISTRY
        .iter()
        .filter(|k| enabled.iter().any(|e| e == k.name))
        .filter_map(|kind| {
            let path = home.join(kind.rel_path);
            path.exists().then_some(CacheEntry { kind, path })
        })
        .collect()
}
