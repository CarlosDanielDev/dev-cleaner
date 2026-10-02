//! What a path is called: its project, its checkout, and where it is inside.

use super::super::palette::Theme;
use super::super::project_label::{fit_name, squeeze};
use super::super::row::elide_path;
use ratatui::style::Style;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// One project, named the way the projects table names it.
#[derive(Debug, Clone)]
pub struct Located {
    pub root: PathBuf,
    /// Its label: the shortest suffix of its path no other project shares.
    pub label: String,
    /// What its checkout adds: `⎇ issue-942`, `◆ main`.
    pub note: Option<String>,
    /// What it is, in words, for the selected row.
    pub facts: String,
}

/// The projects, by root, so an entry can be named by the one it is in.
#[derive(Debug, Clone, Default)]
pub struct Locator {
    by_root: HashMap<PathBuf, Located>,
}

/// An entry's project and its place inside it.
#[derive(Debug, Clone, Copy)]
pub(in crate::tui) struct Site<'a> {
    pub project: &'a Located,
    /// The path from the project's root to the entry: `app/ios/Pods`.
    pub inside: &'a Path,
}

impl Locator {
    pub fn new(projects: impl IntoIterator<Item = Located>) -> Self {
        Self {
            by_root: projects.into_iter().map(|p| (p.root.clone(), p)).collect(),
        }
    }

    /// The innermost project `path` is in, with the path inside it.
    pub(in crate::tui) fn site<'a>(&'a self, path: &'a Path) -> Option<Site<'a>> {
        path.ancestors().find_map(|a| {
            let project = self.by_root.get(a)?;
            Some(Site {
                project,
                inside: path.strip_prefix(a).ok()?,
            })
        })
    }
}

/// How an entry is written in a row: its project, the checkout's note and the
/// path inside the project. The absolute path is not here: it is the selected
/// row's, in the detail line.
#[derive(Debug, Clone)]
pub(in crate::tui) struct Where {
    label: String,
    note: Option<String>,
    /// The path inside the project; the whole path where there is no project.
    inside: String,
}

/// Columns between the project and the path inside it.
const BETWEEN: usize = 2;

impl Where {
    pub(in crate::tui) fn of(locator: &Locator, path: &Path) -> Self {
        match locator.site(path) {
            Some(site) => Self {
                label: site.project.label.clone(),
                note: site.project.note.clone(),
                inside: site.inside.display().to_string(),
            },
            None => Self {
                label: String::new(),
                note: None,
                inside: path.display().to_string(),
            },
        }
    }

    /// How wide it would like to be.
    pub(in crate::tui) fn width(&self) -> usize {
        let note = self
            .note
            .as_ref()
            .map_or(0, |n| BETWEEN + n.chars().count());
        let inside = if self.label.is_empty() || self.inside.is_empty() {
            self.inside.chars().count()
        } else {
            BETWEEN + self.inside.chars().count()
        };
        self.label.chars().count() + note + inside
    }

    /// Only the path inside the project, for a row that sits under the
    /// project's own head.
    pub(in crate::tui) fn inside_runs(&self, theme: &Theme, width: usize) -> Vec<(String, Style)> {
        vec![(squeeze(&self.inside, width), theme.text)]
    }

    /// Runs of text and the style each is drawn in, no wider than `width`.
    ///
    /// The project is what identifies the row and the end of the path is what
    /// the entry is, so the path is cut from the middle, keeping both ends, and
    /// the project and its note give way only to leave the path a little room.
    pub(in crate::tui) fn runs(&self, theme: &Theme, width: usize) -> Vec<(String, Style)> {
        if self.label.is_empty() {
            // No project to say: the end of the path is what identifies it.
            return vec![(elide_path(&self.inside, width), theme.text)];
        }
        let (label, note, inside) = if self.width() <= width {
            let note = self.note.as_ref().map(|n| format!("{:BETWEEN$}{n}", ""));
            (
                self.label.clone(),
                note.unwrap_or_default(),
                self.inside.clone(),
            )
        } else {
            let keep = self.inside.chars().count().min(12);
            let (label, note) = fit_name(
                &self.label,
                self.note.as_deref(),
                width.saturating_sub(keep + BETWEEN).max(1),
            );
            let used = label.chars().count() + note.chars().count();
            let room = width.saturating_sub(used + BETWEEN);
            let inside = if room > 0 {
                squeeze(&self.inside, room)
            } else {
                String::new()
            };
            (label, note, inside)
        };
        let mut runs = vec![(label, theme.accent), (note, theme.violet)];
        if !inside.is_empty() {
            runs.push((" ".repeat(BETWEEN), theme.text));
            runs.push((inside, theme.text));
        }
        runs
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn located(label: &str, note: Option<&str>) -> Where {
        Where {
            label: label.to_string(),
            note: note.map(str::to_string),
            inside: "packages/some-very-long-name/another-long-segment/Pods".to_string(),
        }
    }

    fn drawn(runs: &[(String, Style)]) -> String {
        runs.iter().map(|(t, _)| t.as_str()).collect()
    }

    #[test]
    fn a_row_that_fits_says_everything() {
        let w = located("kyte-app", Some("⎇ issue-942"));
        let text = drawn(&w.runs(&Theme::ansi(), 200));
        assert_eq!(
            text,
            "kyte-app  ⎇ issue-942  packages/some-very-long-name/another-long-segment/Pods"
        );
    }

    #[test]
    fn a_path_that_does_not_fit_is_cut_in_the_middle_and_keeps_both_ends() {
        let w = located("kyte-app", None);
        for width in [30, 40, 60] {
            let text = drawn(&w.runs(&Theme::ansi(), width));
            assert!(text.chars().count() <= width, "{width}: {text:?}");
            assert!(text.starts_with("kyte-app"), "{width}: {text:?}");
            assert!(text.ends_with("/Pods"), "{width}: {text:?}");
            assert!(text.contains('…'), "{width}: {text:?}");
        }
    }

    #[test]
    fn an_entry_in_no_project_keeps_the_end_of_its_path() {
        let w = Where {
            label: String::new(),
            note: None,
            inside: "/very/long/path/to/some/place/node_modules".to_string(),
        };
        let text = drawn(&w.runs(&Theme::ansi(), 20));
        assert_eq!(text.chars().count(), 20);
        assert!(
            text.starts_with('…') && text.ends_with("node_modules"),
            "{text:?}"
        );
    }

    #[test]
    fn the_innermost_project_names_an_entry() {
        let project = |root: &str, label: &str| Located {
            root: PathBuf::from(root),
            label: label.to_string(),
            note: None,
            facts: String::new(),
        };
        let locator = Locator::new([
            project("/w/outer", "outer"),
            project("/w/outer/inner", "inner"),
        ]);
        let site = locator
            .site(Path::new("/w/outer/inner/target"))
            .expect("inside a project");
        assert_eq!(site.project.label, "inner");
        assert_eq!(site.inside, Path::new("target"));
        assert!(locator.site(Path::new("/elsewhere/target")).is_none());
    }
}
