//! What a project installed, read from the lockfile that already recorded it.
//!
//! The duplicate that costs real space is the same package at the same version
//! installed into many projects. Hashing file contents to find it means reading
//! a million files to learn what seven text files already state exactly, so the
//! lockfiles are the cheap evidence.
//!
//! Nothing here becomes a `Candidate`. Every package named below already sits
//! inside the artifact directory that is offered for deletion, so offering both
//! would count the same bytes twice in a plan total.

use std::path::{Path, PathBuf};

use serde::Deserialize;

use super::Ecosystem;
use super::project::is_inside_artifact;
use crate::scan::FileMeta;

/// One package a lockfile resolved.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Package {
    pub name: String,
    pub version: String,
    /// The size the registry recorded for the distribution, where the format
    /// records one at all.
    ///
    /// `None` is not zero. Only uv writes a size, so a consumer has to measure
    /// the disk for every other ecosystem; reading an absent size as `Some(0)`
    /// would claim the package weighs nothing.
    pub bytes: Option<u64>,
}

/// Every package a single lockfile names.
#[derive(Debug, Clone)]
pub struct Lockfile {
    pub path: PathBuf,
    pub ecosystem: Ecosystem,
    pub packages: Vec<Package>,
}

/// How a lockfile is laid out. Several names share one layout.
#[derive(Debug, Clone, Copy)]
enum Format {
    /// npm, whose three lockfile versions are all JSON.
    Npm,
    /// cargo, poetry and uv all write an array of `[[package]]` tables.
    Toml,
    Pnpm,
    Yarn,
    Gemfile,
}

/// A lockfile name and the toolchain that writes it.
#[derive(Debug, Clone, Copy)]
pub struct LockfileKind {
    pub file_name: &'static str,
    pub ecosystem: Ecosystem,
    format: Format,
}

impl LockfileKind {
    const fn new(file_name: &'static str, ecosystem: Ecosystem, format: Format) -> Self {
        assert!(!file_name.is_empty(), "a lockfile kind must be named");
        Self {
            file_name,
            ecosystem,
            format,
        }
    }
}

use Ecosystem::*;

/// The one place a new lockfile format is added.
const REGISTRY: &[LockfileKind] = &[
    LockfileKind::new("package-lock.json", Node, Format::Npm),
    LockfileKind::new("pnpm-lock.yaml", Node, Format::Pnpm),
    LockfileKind::new("yarn.lock", Node, Format::Yarn),
    LockfileKind::new("Cargo.lock", Rust, Format::Toml),
    LockfileKind::new("poetry.lock", Python, Format::Toml),
    LockfileKind::new("uv.lock", Python, Format::Toml),
    LockfileKind::new("Gemfile.lock", Ruby, Format::Gemfile),
];

/// Every registered kind.
pub fn lockfile_kinds() -> &'static [LockfileKind] {
    REGISTRY
}

/// Look a file name up in the registry.
///
/// Returns `None` for a manifest such as `package.json`: a manifest names
/// ranges, and a range is not evidence that anything was installed.
pub fn lockfile_for(file_name: &str) -> Option<&'static LockfileKind> {
    REGISTRY.iter().find(|k| k.file_name == file_name)
}

/// Parse `text` as the lockfile `path` names.
///
/// Every failure is an error carrying the path, never a panic. A malformed or
/// half-written lockfile has to cost a warning: a scan that dies on one bad
/// file is worse than one that steps over it.
pub fn parse_lockfile(path: &Path, text: &str) -> Result<Lockfile, String> {
    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or_default();
    let Some(kind) = lockfile_for(name) else {
        return Err(format!("{}: not a known lockfile", path.display()));
    };

    let parsed = match kind.format {
        Format::Npm => npm(text),
        Format::Toml => toml_packages(text),
        Format::Pnpm => Ok(pnpm(text)),
        Format::Yarn => Ok(yarn(text)),
        Format::Gemfile => Ok(gemfile(text)),
    };
    let mut packages = parsed.map_err(|err| format!("{}: {err}", path.display()))?;

    // ponytail: one entry per name and version. npm records a hoisted package
    // at every path it was installed to, so the raw list counts one dependency
    // several times. A project that really holds two physical copies of the
    // same version is rare and reads as one here; split them out if the
    // duplicate report ever needs per-path counts.
    packages.sort_by(|a, b| (&a.name, &a.version).cmp(&(&b.name, &b.version)));
    packages.dedup_by(|a, b| a.name == b.name && a.version == b.version);

    Ok(Lockfile {
        path: path.to_path_buf(),
        ecosystem: kind.ecosystem,
        packages,
    })
}

/// Read and parse a lockfile from disk.
pub fn read_lockfile(path: &Path) -> Result<Lockfile, String> {
    let text = std::fs::read_to_string(path).map_err(|err| format!("{}: {err}", path.display()))?;
    parse_lockfile(path, &text)
}

/// Every lockfile a walk passed over, with a warning for each one that failed.
///
/// Lockfiles inside artifact directories are skipped on exactly the grounds
/// `ProjectIndex` skips marker files there: a lockfile under `node_modules`
/// describes a dependency's own dependencies, and reading it would report one
/// project's install as hundreds of separate projects.
pub fn lockfiles_in(files: &[FileMeta]) -> (Vec<Lockfile>, Vec<String>) {
    let mut found = Vec::new();
    let mut warnings = Vec::new();

    for file in files {
        if is_inside_artifact(&file.path) {
            continue;
        }
        let Some(name) = file.path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        if lockfile_for(name).is_none() {
            continue;
        }
        match read_lockfile(&file.path) {
            Ok(lock) => found.push(lock),
            Err(err) => warnings.push(err),
        }
    }

    (found, warnings)
}

/// `package-lock.json`, all three lockfile versions.
fn npm(text: &str) -> Result<Vec<Package>, String> {
    let root: serde_json::Value = serde_json::from_str(text).map_err(|err| err.to_string())?;

    // Versions 2 and 3 key `packages` by install path. Version 2 also carries
    // the version 1 tree for older clients, describing the same install, so
    // reading both would report every dependency twice.
    if let Some(tree) = root.get("packages").and_then(|v| v.as_object()) {
        return Ok(tree
            .iter()
            .filter_map(|(install_path, entry)| {
                // "" is the project itself and a bare "packages/ui" is a
                // workspace member. Neither was installed from a registry.
                let (_, name) = install_path.rsplit_once("node_modules/")?;
                Some(Package {
                    name: name.to_string(),
                    version: version_of(entry)?,
                    bytes: None,
                })
            })
            .collect());
    }

    // Version 1 nests dependencies inside dependencies. Walked with an explicit
    // stack rather than recursion so depth is the input's problem, not the
    // stack's.
    let mut out = Vec::new();
    let mut pending: Vec<&serde_json::Value> = root.get("dependencies").into_iter().collect();
    while let Some(node) = pending.pop() {
        let Some(tree) = node.as_object() else {
            continue;
        };
        for (name, entry) in tree {
            if let Some(version) = version_of(entry) {
                out.push(Package {
                    name: name.clone(),
                    version,
                    bytes: None,
                });
            }
            if let Some(nested) = entry.get("dependencies") {
                pending.push(nested);
            }
        }
    }
    Ok(out)
}

fn version_of(entry: &serde_json::Value) -> Option<String> {
    Some(entry.get("version")?.as_str()?.to_string())
}

/// `Cargo.lock`, `poetry.lock` and `uv.lock`: an array of `[[package]]` tables.
#[derive(Deserialize, Default)]
#[serde(default)]
struct TomlLock {
    package: Vec<TomlPackage>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct TomlPackage {
    name: Option<String>,
    version: Option<String>,
    /// Only uv records a size, and only for the source distribution. The wheel
    /// list is per platform, so no single entry there is the size this machine
    /// installed.
    sdist: Option<TomlDist>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct TomlDist {
    size: Option<u64>,
}

fn toml_packages(text: &str) -> Result<Vec<Package>, String> {
    let lock: TomlLock = toml::from_str(text).map_err(|err| err.to_string())?;
    Ok(lock
        .package
        .into_iter()
        .filter_map(|p| {
            Some(Package {
                name: p.name?,
                version: p.version?,
                bytes: p.sdist.and_then(|d| d.size),
            })
        })
        .collect())
}

/// `pnpm-lock.yaml`: one key per package under the top-level `packages` map.
///
/// ponytail: a line scanner over the layout pnpm generates, not a YAML parser.
/// It assumes two-space indentation and one package per key, which is what pnpm
/// has written since version 5. A hand-edited file with different indentation
/// yields fewer packages, never a panic. Add a YAML crate if that stops holding.
fn pnpm(text: &str) -> Vec<Package> {
    let mut out = Vec::new();
    let mut in_packages = false;

    for line in text.lines() {
        // A column-zero key ends the previous block. `snapshots` repeats the
        // same packages with their dependency graphs and would double them.
        if !line.starts_with(' ') && !line.trim().is_empty() {
            in_packages = line.trim_end() == "packages:";
            continue;
        }
        if !in_packages {
            continue;
        }
        let Some(key) = line.strip_prefix("  ") else {
            continue;
        };
        // Deeper indentation is a field of the package above, not a package.
        if key.starts_with(' ') {
            continue;
        }
        let Some(key) = key.trim_end().strip_suffix(':') else {
            continue;
        };
        if let Some(package) = pnpm_key(key) {
            out.push(package);
        }
    }
    out
}

fn pnpm_key(key: &str) -> Option<Package> {
    let key = key.trim_matches(['\'', '"']);
    // A peer suffix, `react-dom@18.3.1(react@18.3.1)`, is not part of the
    // version and holds an @ that would be mistaken for the separator.
    let key = key.split('(').next()?;
    // pnpm 5 wrote /name/version, 6 wrote /name@version, 9 dropped the slash.
    let key = key.strip_prefix('/').unwrap_or(key);

    // A leading @ is the scope, never the separator.
    let (name, version) = match key.rfind('@').filter(|&at| at > 0) {
        Some(at) => (&key[..at], &key[at + 1..]),
        None => key.rsplit_once('/')?,
    };
    (!name.is_empty() && !version.is_empty()).then(|| Package {
        name: name.to_string(),
        version: version.to_string(),
        bytes: None,
    })
}

/// `yarn.lock`, both the v1 text format and the YAML-shaped Berry one.
///
/// ponytail: a line scanner. v1 is not YAML at all, and Berry's subset is
/// regular enough that pairing each column-zero header with the `version` field
/// under it covers both. Ceiling: an entry whose version field is missing is
/// dropped rather than guessed at.
fn yarn(text: &str) -> Vec<Package> {
    let mut out = Vec::new();
    let mut current: Option<String> = None;

    for line in text.lines() {
        if line.starts_with('#') || line.trim().is_empty() {
            continue;
        }
        if !line.starts_with(' ') {
            current = line.trim_end().strip_suffix(':').and_then(yarn_name);
            continue;
        }
        let Some(name) = &current else {
            continue;
        };
        // v1 writes `version "1.2.3"`, Berry writes `version: 1.2.3`.
        let Some(rest) = line.trim().strip_prefix("version") else {
            continue;
        };
        let version = rest.trim_start_matches([':', ' ']).trim_matches('"');
        if !version.is_empty() {
            out.push(Package {
                name: name.clone(),
                version: version.to_string(),
                bytes: None,
            });
        }
        current = None;
    }
    out
}

fn yarn_name(header: &str) -> Option<String> {
    // One entry can satisfy several specs: `"a@^1.0", "a@^1.2":`.
    let first = header.split(',').next()?.trim().trim_matches('"');
    // Berry's `__metadata` block has no @ and is not a package.
    let at = first.rfind('@').filter(|&at| at > 0)?;
    Some(first[..at].to_string())
}

/// `Gemfile.lock`: the four-space `name (version)` lines under `specs:`.
///
/// ponytail: a line scanner over the shape Bundler writes. A gem's own
/// requirements sit one level deeper and the DEPENDENCIES block sits one level
/// shallower, so indentation alone separates resolutions from ranges. A
/// reindented file yields fewer gems, never a panic.
fn gemfile(text: &str) -> Vec<Package> {
    text.lines()
        .filter_map(|line| {
            let spec = line.strip_prefix("    ")?;
            // Six spaces: a requirement of the gem above, named without a
            // resolved version.
            if spec.starts_with(' ') {
                return None;
            }
            let (name, version) = spec.trim_end().strip_suffix(')')?.split_once(" (")?;
            (!name.is_empty() && !version.is_empty()).then(|| Package {
                name: name.to_string(),
                version: version.to_string(),
                bytes: None,
            })
        })
        .collect()
}
