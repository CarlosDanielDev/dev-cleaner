use std::collections::BTreeMap;
use std::path::PathBuf;

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};

use crate::bytes::human;
use crate::classify::Activity;

/// One project, measured every way the table can order it.
#[derive(Debug, Clone)]
pub struct ProjectSummary {
    pub path: PathBuf,
    /// Sum of `st_size`.
    pub bytes_apparent: u64,
    /// Allocated blocks with each inode counted once: what deletion returns.
    pub bytes_unique: u64,
    pub inodes: u64,
    pub activity: Activity,
    /// The part of this project that is build output and could be rebuilt.
    pub reclaimable: u64,
}

impl ProjectSummary {
    pub fn name(&self) -> &str {
        self.path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or_else(|| self.path.to_str().unwrap_or("?"))
    }

    /// The apparent size, when it says something the unique size does not.
    ///
    /// They differ only where a project hardlinks into a shared store, and that
    /// gap is the difference between what the directory looks like and what
    /// deleting it would give back. Where they agree, printing both twice on a
    /// row is noise that hides the rows where they do not.
    fn apparent_if_different(&self) -> String {
        if self.bytes_apparent == self.bytes_unique {
            String::new()
        } else {
            human(self.bytes_apparent)
        }
    }
}

/// A column the table can be ordered by.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Column {
    Name,
    Apparent,
    Unique,
    Inodes,
    Activity,
    Reclaimable,
}

impl Column {
    fn header(&self) -> &'static str {
        match self {
            Column::Name => "project",
            Column::Apparent => "apparent",
            Column::Unique => "unique",
            Column::Inodes => "inodes",
            Column::Activity => "activity",
            Column::Reclaimable => "reclaimable",
        }
    }

    /// Which way round this column is most useful first.
    ///
    /// Sizes and counts answer "what is worst", so they start at the largest.
    /// Names answer "where is X", so they start at A.
    fn starts_descending(&self) -> bool {
        !matches!(self, Column::Name | Column::Activity)
    }
}

/// The projects table.
#[derive(Debug)]
pub struct Projects {
    rows: Vec<ProjectSummary>,
    /// Display name per project, qualified where a bare name would be ambiguous.
    labels: BTreeMap<PathBuf, String>,
    sort: Column,
    descending: bool,
    cursor: usize,
}

impl Projects {
    pub fn new(rows: Vec<ProjectSummary>) -> Self {
        let mut table = Self {
            labels: disambiguate(&rows),
            rows,
            sort: Column::Unique,
            descending: true,
            cursor: 0,
        };
        table.apply_sort();
        table
    }

    pub fn rows(&self) -> &[ProjectSummary] {
        &self.rows
    }

    /// Order by `column`, reversing if it is already the one in use.
    ///
    /// Coming back to a column later starts from its own default again rather
    /// than from whatever direction it was left in, so a column always means
    /// the same thing the first time it is pressed.
    pub fn sort_by(&mut self, column: Column) {
        if self.sort == column {
            self.descending = !self.descending;
        } else {
            self.sort = column;
            self.descending = column.starts_descending();
        }
        self.apply_sort();
    }

    fn apply_sort(&mut self) {
        match self.sort {
            Column::Name => self.rows.sort_by(|a, b| a.name().cmp(b.name())),
            Column::Apparent => self.rows.sort_by_key(|r| r.bytes_apparent),
            Column::Unique => self.rows.sort_by_key(|r| r.bytes_unique),
            Column::Inodes => self.rows.sort_by_key(|r| r.inodes),
            Column::Reclaimable => self.rows.sort_by_key(|r| r.reclaimable),
            // Most alive first: a project still in use is the one you most want
            // to recognise before acting anywhere near it.
            Column::Activity => self.rows.sort_by_key(|r| match r.activity {
                Activity::Active => 0,
                Activity::Dormant => 1,
                Activity::Dead => 2,
            }),
        }
        if self.descending {
            self.rows.reverse();
        }
    }

    /// What to call this project on screen.
    pub fn label<'a>(&'a self, row: &'a ProjectSummary) -> &'a str {
        self.labels
            .get(&row.path)
            .map(String::as_str)
            .unwrap_or_else(|| row.name())
    }

    pub fn selected(&self) -> Option<&ProjectSummary> {
        self.rows.get(self.cursor)
    }

    /// Move down a row. Stops at the last rather than wrapping: wrapping from
    /// the end to the start moves the selection somewhere the user was not
    /// looking.
    pub fn down(&mut self) {
        self.cursor = (self.cursor + 1).min(self.rows.len().saturating_sub(1));
    }

    pub fn up(&mut self) {
        self.cursor = self.cursor.saturating_sub(1);
    }

    /// The rows that fit on screen, always including the selected one.
    ///
    /// Drawing is bounded by the window rather than by the number of rows,
    /// which is what keeps a few hundred projects responsive.
    ///
    /// ponytail: the window is derived from the cursor rather than kept as a
    /// scroll offset, so moving past the bottom edge jumps the view by a row
    /// instead of following smoothly. Keep an offset if the jumpiness shows.
    pub fn visible(&self, height: usize) -> &[ProjectSummary] {
        let start = self.window_start(height);
        let end = (start + height).min(self.rows.len());
        &self.rows[start..end]
    }

    /// Index of the first visible row, chosen so the cursor is always in view.
    ///
    /// Shared with `render` so the row it highlights is the row `visible`
    /// returns; computing the window twice is how a table comes to highlight
    /// the wrong line.
    fn window_start(&self, height: usize) -> usize {
        if height == 0 || self.rows.is_empty() {
            return 0;
        }
        let last_start = self.rows.len().saturating_sub(height);
        (self.cursor + 1).saturating_sub(height).min(last_start)
    }

    pub fn render(&self, area: Rect, buf: &mut Buffer) {
        let left = area.x + 1;
        let head = Style::new().add_modifier(Modifier::BOLD);
        let dim = Style::new().fg(Color::DarkGray);

        let columns = [
            (Column::Name, 0u16),
            (Column::Unique, 26),
            (Column::Apparent, 38),
            (Column::Inodes, 50),
            (Column::Reclaimable, 60),
            (Column::Activity, 74),
        ];
        for (column, x) in columns {
            let marker = if column == self.sort {
                if self.descending { " v" } else { " ^" }
            } else {
                ""
            };
            let style = if column == self.sort { head } else { dim };
            buf.set_string(
                left + x,
                area.y,
                format!("{}{marker}", column.header()),
                style,
            );
        }

        let height = area.height.saturating_sub(1) as usize;
        let start = self.window_start(height);
        for (i, row) in self.visible(height).iter().enumerate() {
            let y = area.y + 1 + i as u16;
            let style = if start + i == self.cursor {
                Style::new().add_modifier(Modifier::REVERSED)
            } else {
                Style::new()
            };
            buf.set_string(left, y, truncate(self.label(row), 24), style);
            buf.set_string(left + 26, y, human(row.bytes_unique), style);
            buf.set_string(left + 38, y, row.apparent_if_different(), dim);
            buf.set_string(left + 50, y, row.inodes.to_string(), style);
            buf.set_string(left + 60, y, human(row.reclaimable), style);
            buf.set_string(
                left + 74,
                y,
                format!("{} {}", row.activity.symbol(), describe(row.activity)),
                style,
            );
        }
    }
}

/// Give every project a name that identifies it.
///
/// Directory names repeat: a corpus holds several projects called `web`, and
/// git worktrees multiply them further. Two rows reading the same thing with
/// near-identical figures cannot be acted on, so a name that collides is
/// qualified with the directory above it.
///
/// ponytail: one parent, not as many as it takes. Two projects at `x/a/web` and
/// `y/a/web` would still read alike; go further up only if that shows up.
fn disambiguate(rows: &[ProjectSummary]) -> BTreeMap<PathBuf, String> {
    let mut seen: BTreeMap<&str, usize> = BTreeMap::new();
    for row in rows {
        *seen.entry(row.name()).or_default() += 1;
    }
    rows.iter()
        .map(|row| {
            let unique = seen.get(row.name()).copied().unwrap_or(1) == 1;
            let label = match row.path.parent().and_then(|p| p.file_name()) {
                Some(parent) if !unique => {
                    format!("{}/{}", parent.to_string_lossy(), row.name())
                }
                _ => row.name().to_string(),
            };
            (row.path.clone(), label)
        })
        .collect()
}

fn describe(activity: Activity) -> &'static str {
    match activity {
        Activity::Active => "active",
        Activity::Dormant => "dormant",
        Activity::Dead => "dead",
    }
}

/// Keep a long name inside its column, marking that it was cut.
fn truncate(name: &str, width: usize) -> String {
    if name.chars().count() <= width {
        return name.to_string();
    }
    let kept: String = name.chars().take(width.saturating_sub(1)).collect();
    format!("{kept}…")
}
