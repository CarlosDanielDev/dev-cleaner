/// Which toolchain owns a directory.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ecosystem {
    Node,
    Rust,
    Python,
    Swift,
    Ruby,
    Go,
    Java,
    Php,
    Embedded,
}

/// A directory that a build tool creates and can recreate.
#[derive(Debug, Clone, Copy)]
pub struct ArtifactKind {
    pub dir_name: &'static str,
    pub ecosystem: Ecosystem,
    /// The command that brings this directory back. Never empty: a kind whose
    /// regeneration is unknown must not exist, because the safety tiers use
    /// this field to decide what may be offered for deletion.
    pub regen: &'static str,
}

impl ArtifactKind {
    const fn new(dir_name: &'static str, ecosystem: Ecosystem, regen: &'static str) -> Self {
        // Evaluated at compile time because REGISTRY is a const: a kind added
        // without a regeneration command fails the build rather than reaching
        // a user.
        assert!(
            !regen.is_empty(),
            "every artifact kind must declare how it is regenerated"
        );
        Self {
            dir_name,
            ecosystem,
            regen,
        }
    }
}

use Ecosystem::*;

/// The one place a new artifact kind is added.
const REGISTRY: &[ArtifactKind] = &[
    ArtifactKind::new("node_modules", Node, "npm install"),
    ArtifactKind::new(".next", Node, "next build"),
    ArtifactKind::new(".turbo", Node, "turbo run build"),
    ArtifactKind::new(".parcel-cache", Node, "parcel build"),
    ArtifactKind::new(".svelte-kit", Node, "vite build"),
    ArtifactKind::new("target", Rust, "cargo build"),
    ArtifactKind::new(
        ".venv",
        Python,
        "python -m venv .venv && pip install -r requirements.txt",
    ),
    ArtifactKind::new(
        "venv",
        Python,
        "python -m venv venv && pip install -r requirements.txt",
    ),
    ArtifactKind::new("__pycache__", Python, "regenerated on next import"),
    ArtifactKind::new("Pods", Swift, "pod install"),
    ArtifactKind::new("DerivedData", Swift, "rebuild in Xcode"),
    ArtifactKind::new(".gradle", Java, "gradle build"),
    ArtifactKind::new("vendor", Php, "composer install"),
    // Covers both .pio/build and .pio/libdeps. Vendored libraries under
    // libdeps ship their own example projects with platformio.ini, so
    // registering the parent keeps them out of project detection too.
    ArtifactKind::new(".pio", Embedded, "pio run"),
];

/// Every registered kind. Used by tests and by the candidate builder.
pub fn artifact_kinds() -> &'static [ArtifactKind] {
    REGISTRY
}

/// Look a directory name up in the registry.
///
/// Returns `None` for anything unrecognised, which keeps unknown directories in
/// the Unproven tier instead of guessing at them.
pub fn artifact_for(dir_name: &str) -> Option<&'static ArtifactKind> {
    REGISTRY.iter().find(|k| k.dir_name == dir_name)
}

/// The outermost artifact directory on `path`, with the kind that claimed it.
///
/// Outermost, not innermost: `node_modules` regularly contains further
/// `node_modules`. Offering an inner one as its own candidate would both
/// double-count its bytes and let a user delete half a dependency tree.
pub fn artifact_root(
    path: &std::path::Path,
) -> Option<(std::path::PathBuf, &'static ArtifactKind)> {
    let mut prefix = std::path::PathBuf::new();
    for component in path.components() {
        prefix.push(component);
        if let Some(name) = component.as_os_str().to_str()
            && let Some(kind) = artifact_for(name)
        {
            return Some((prefix, kind));
        }
    }
    None
}
