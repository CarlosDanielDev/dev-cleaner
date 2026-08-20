use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use super::{Ecosystem, artifact_for};
use crate::scan::FileMeta;

/// A directory identified as a project by the marker files it contains.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Project {
    pub root: PathBuf,
    pub ecosystems: Vec<Ecosystem>,
}

/// Marker file to the ecosystem it implies.
const MARKERS: &[(&str, Ecosystem)] = &[
    ("package.json", Ecosystem::Node),
    ("Cargo.toml", Ecosystem::Rust),
    ("go.mod", Ecosystem::Go),
    ("pyproject.toml", Ecosystem::Python),
    ("requirements.txt", Ecosystem::Python),
    ("Package.swift", Ecosystem::Swift),
    ("Podfile", Ecosystem::Swift),
    ("build.gradle", Ecosystem::Java),
    ("build.gradle.kts", Ecosystem::Java),
    ("pom.xml", Ecosystem::Java),
    ("Gemfile", Ecosystem::Ruby),
    ("composer.json", Ecosystem::Php),
    ("platformio.ini", Ecosystem::Embedded),
];

/// Projects discovered in a scan, queryable by path.
#[derive(Debug, Default)]
pub struct ProjectIndex {
    /// Sorted by path so the innermost owner is the last matching prefix.
    projects: Vec<Project>,
}

impl ProjectIndex {
    pub fn from_files(files: &[FileMeta]) -> Self {
        let mut found: BTreeMap<PathBuf, Vec<Ecosystem>> = BTreeMap::new();

        for file in files {
            // A marker inside a build artifact belongs to a dependency, not to
            // a project. Every package under node_modules carries its own
            // package.json, so without this the corpus inflates from 103
            // projects to several thousand.
            if is_inside_artifact(&file.path) {
                continue;
            }
            let Some(name) = file.path.file_name().and_then(|n| n.to_str()) else {
                continue;
            };
            let Some((_, eco)) = MARKERS.iter().find(|(m, _)| *m == name) else {
                continue;
            };
            let Some(root) = file.path.parent() else {
                continue;
            };
            let entry = found.entry(root.to_path_buf()).or_default();
            if !entry.contains(eco) {
                entry.push(*eco);
            }
        }

        Self {
            projects: found
                .into_iter()
                .map(|(root, ecosystems)| Project { root, ecosystems })
                .collect(),
        }
    }

    /// The innermost project containing `path`, if any.
    pub fn owner_of(&self, path: &Path) -> Option<&Project> {
        self.projects
            .iter()
            .filter(|p| path.starts_with(&p.root))
            .max_by_key(|p| p.root.as_os_str().len())
    }

    pub fn projects(&self) -> impl Iterator<Item = &Project> {
        self.projects.iter()
    }

    pub fn len(&self) -> usize {
        self.projects.len()
    }

    pub fn is_empty(&self) -> bool {
        self.projects.is_empty()
    }
}

/// Whether any component of `path` is a registered build artifact directory.
fn is_inside_artifact(path: &Path) -> bool {
    path.components()
        .filter_map(|c| c.as_os_str().to_str())
        .any(|c| artifact_for(c).is_some())
}
