use std::collections::{BTreeMap, HashMap};
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
    /// Sorted by path.
    projects: Vec<Project>,
    /// Position in `projects` by root, so an owner is found by walking a path's
    /// ancestors rather than testing every project.
    by_root: HashMap<PathBuf, usize>,
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

        let projects: Vec<Project> = found
            .into_iter()
            .map(|(root, ecosystems)| Project { root, ecosystems })
            .collect();
        let by_root = projects
            .iter()
            .enumerate()
            .map(|(i, p)| (p.root.clone(), i))
            .collect();
        Self { projects, by_root }
    }

    /// The innermost project containing `path`, if any.
    pub fn owner_of(&self, path: &Path) -> Option<&Project> {
        // `ancestors` yields the path itself, then each parent, so the first
        // project root met is the innermost. O(depth), not O(projects).
        path.ancestors()
            .find_map(|a| self.by_root.get(a))
            .map(|&i| &self.projects[i])
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
pub fn is_inside_artifact(path: &Path) -> bool {
    path.components()
        .filter_map(|c| c.as_os_str().to_str())
        .any(|c| artifact_for(c).is_some())
}

/// Whether a file of this name marks its directory as a project.
///
/// The walk asks this to count projects as it goes; [`ProjectIndex`] asks the
/// same table when it builds, so the two cannot disagree about what a marker is.
pub fn is_project_marker(name: &str) -> bool {
    MARKERS.iter().any(|(m, _)| *m == name)
}

#[cfg(test)]
mod tests {
    use std::time::UNIX_EPOCH;

    use super::*;

    fn file(path: &str) -> FileMeta {
        FileMeta {
            path: PathBuf::from(path),
            bytes_apparent: 0,
            bytes_actual: 0,
            dev: 0,
            ino: 0,
            mtime: UNIX_EPOCH,
        }
    }

    /// The lookup this replaced: every project, longest matching root wins.
    fn owner_of_linear<'a>(index: &'a ProjectIndex, path: &Path) -> Option<&'a Project> {
        index
            .projects()
            .filter(|p| path.starts_with(&p.root))
            .max_by_key(|p| p.root.as_os_str().len())
    }

    /// Deterministic pseudo-random numbers; no dependency for a test.
    struct Lcg(u64);
    impl Lcg {
        fn next(&mut self, n: usize) -> usize {
            self.0 = self
                .0
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            ((self.0 >> 33) as usize) % n
        }
    }

    #[test]
    fn owner_by_ancestor_agrees_with_the_linear_scan() {
        // Few names, so roots nest and share string prefixes (`app`, `app2`).
        const NAMES: &[&str] = &[
            "app",
            "app2",
            "a",
            "ab",
            "src",
            "ios",
            "lib",
            "node_modules",
        ];
        let mut rng = Lcg(7);
        for round in 0..200 {
            let dir = |rng: &mut Lcg| {
                let depth = 1 + rng.next(5);
                let mut p = String::from("/r");
                for _ in 0..depth {
                    p.push('/');
                    p.push_str(NAMES[rng.next(NAMES.len())]);
                }
                p
            };
            let markers = ["package.json", "Cargo.toml", "go.mod"];
            let mut files: Vec<FileMeta> = (0..1 + rng.next(8))
                .map(|_| file(&format!("{}/{}", dir(&mut rng), markers[rng.next(3)])))
                .collect();
            files.extend((0..30).map(|_| file(&format!("{}/f.txt", dir(&mut rng)))));
            let index = ProjectIndex::from_files(&files);

            let mut probes: Vec<PathBuf> = files.iter().map(|f| f.path.clone()).collect();
            // A project root itself, a sibling sharing its string prefix, and
            // paths outside every project.
            for p in index.projects() {
                probes.push(p.root.clone());
                probes.push(PathBuf::from(format!("{}2/x", p.root.display())));
                probes.push(PathBuf::from(format!("{}/", p.root.display())));
                probes.push(p.root.join("deeper/still/x"));
            }
            probes.push(PathBuf::from("/r"));
            probes.push(PathBuf::from("/elsewhere/app/x"));
            probes.push(PathBuf::from("relative/app/x"));

            for path in &probes {
                assert_eq!(
                    index.owner_of(path),
                    owner_of_linear(&index, path),
                    "round {round}: {}",
                    path.display()
                );
            }
        }
    }

    #[test]
    fn nested_projects_keep_the_innermost_owner() {
        let index = ProjectIndex::from_files(&[
            file("/r/app/package.json"),
            file("/r/app/ios/Package.swift"),
            file("/r/app2/Cargo.toml"),
        ]);
        let owner = |p: &str| index.owner_of(Path::new(p)).map(|p| p.root.clone());
        assert_eq!(owner("/r/app/ios/x.swift"), Some("/r/app/ios".into()));
        assert_eq!(owner("/r/app/src/x.js"), Some("/r/app".into()));
        assert_eq!(owner("/r/app2/x.rs"), Some("/r/app2".into()));
        assert_eq!(owner("/r/apple/x"), None);
    }
}
