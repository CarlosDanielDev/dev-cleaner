use std::collections::BTreeMap;
use std::path::PathBuf;

use super::palette::Theme;
use super::{showing, window_start};
use crate::bytes::human;
use crate::classify::Activity;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Style;

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
        // The cursor follows its project, as it does on the candidates screen:
        // the user was looking at a project, not at a row number, and a key
        // that is not a move must not change what is selected.
        let under = self.selected().map(|r| r.path.clone());
        self.apply_sort();
        if let Some(i) = under.and_then(|path| self.rows.iter().position(|r| r.path == path)) {
            self.cursor = i;
        }
    }

    /// The order in force, in words: the column and which end comes first.
    pub fn ordering(&self) -> String {
        let way = match (self.sort, self.descending) {
            (Column::Name, false) => "A to Z",
            (Column::Name, true) => "Z to A",
            (Column::Inodes, true) => "most first",
            (Column::Inodes, false) => "fewest first",
            (Column::Activity, false) => "most active first",
            (Column::Activity, true) => "least active first",
            (_, true) => "largest first",
            (_, false) => "smallest first",
        };
        let column = match self.sort {
            Column::Name => "name",
            other => other.header(),
        };
        format!("{column}, {way}")
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

    pub fn top(&mut self) {
        self.cursor = 0;
    }

    pub fn bottom(&mut self) {
        self.cursor = self.rows.len().saturating_sub(1);
    }

    /// Move a window of `rows` at once, clamped like `down` and `up`. From the
    /// top a page lands on the first row past the window, so nothing on screen
    /// is skipped and nothing is read twice.
    pub fn page_down(&mut self, rows: usize) {
        self.cursor = (self.cursor + rows).min(self.rows.len().saturating_sub(1));
    }

    pub fn page_up(&mut self, rows: usize) {
        self.cursor = self.cursor.saturating_sub(rows);
    }

    /// The rows that fit on screen, always including the selected one.
    ///
    /// Drawing is bounded by the window rather than by the number of rows,
    /// which is what keeps a few hundred projects responsive.
    ///
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
        window_start(self.cursor, self.rows.len(), height)
    }

    pub fn render(&self, theme: &Theme, area: Rect, buf: &mut Buffer) {
        let left = area.x + 1;
        let (drawn, hidden) = fit(area.width.saturating_sub(1));

        for (column, x) in &drawn {
            let style = if *column == self.sort {
                theme.head
            } else {
                theme.muted
            };
            let text = format!("{}{}", column.header(), self.marker(*column));
            buf.set_string(left + x, area.y, text, style);
        }

        // A row of headers above the table, and the position below it.
        let height = area.height.saturating_sub(2) as usize;
        let start = self.window_start(height);
        let visible = self.visible(height);
        for (i, row) in visible.iter().enumerate() {
            let y = area.y + 1 + i as u16;
            for (column, x) in &drawn {
                buf.set_string(
                    left + x,
                    y,
                    self.cell(row, *column),
                    Self::ink(theme, row, *column),
                );
            }
            // Across the whole row, gaps included: highlighted cell by cell it
            // reads as separate blocks rather than as one line under a cursor.
            if start + i == self.cursor {
                buf.set_style(Rect::new(area.x, y, area.width, 1), theme.selected);
            }
        }
        if area.height >= 2 {
            let mut line = showing(start, visible.len(), self.rows.len());
            // A column that is not drawn is still there to sort by, so the
            // sorted one's header, marker and all, moves down here.
            if !hidden.is_empty() {
                let names: Vec<String> = hidden
                    .iter()
                    .map(|c| format!("{}{}", c.header(), self.marker(*c)))
                    .collect();
                line = format!("{line} · {} hidden at this width", listed(&names));
            }
            buf.set_string(
                left,
                area.bottom() - 1,
                truncate(&line, area.width.saturating_sub(1) as usize),
                theme.text,
            );
        }
    }

    /// The sort direction, on the column that is sorted by; nothing elsewhere.
    fn marker(&self, column: Column) -> &'static str {
        if column != self.sort {
            ""
        } else if self.descending {
            " v"
        } else {
            " ^"
        }
    }

    /// The role `row`'s `column` is drawn in: a name is the accent, a size is
    /// on the size ramp, and the activity is told apart by hue as well as by
    /// its glyph and its word.
    fn ink(theme: &Theme, row: &ProjectSummary, column: Column) -> Style {
        match column {
            Column::Name => theme.accent,
            Column::Unique => theme.size(row.bytes_unique),
            Column::Apparent => theme.size(row.bytes_apparent),
            Column::Inodes => theme.text,
            Column::Reclaimable => theme.size(row.reclaimable),
            Column::Activity => match row.activity {
                Activity::Active => theme.accent,
                Activity::Dormant => theme.violet,
                Activity::Dead => theme.head,
            },
        }
    }

    /// What `row` says under `column`.
    fn cell(&self, row: &ProjectSummary, column: Column) -> String {
        match column {
            Column::Name => truncate(self.label(row), 24),
            Column::Unique => human(row.bytes_unique),
            Column::Apparent => row.apparent_if_different(),
            Column::Inodes => row.inodes.to_string(),
            Column::Reclaimable => human(row.reclaimable),
            Column::Activity => {
                format!("{} {}", row.activity.symbol(), describe(row.activity))
            }
        }
    }
}

/// Each column's width, gap included, in the order they are drawn.
const LAYOUT: [(Column, u16); 6] = [
    (Column::Name, 26),
    (Column::Unique, 12),
    (Column::Apparent, 12),
    (Column::Inodes, 10),
    (Column::Reclaimable, 14),
    (Column::Activity, 10),
];

/// The order columns are kept in when the area is narrower than all of them.
///
/// What deletion gives back and what could be rebuilt are the two figures a
/// decision rests on; the apparent size only says something on a hardlinked
/// project, so it is the first to go.
const PRIORITY: [Column; 6] = [
    Column::Name,
    Column::Unique,
    Column::Reclaimable,
    Column::Activity,
    Column::Inodes,
    Column::Apparent,
];

/// The columns that fit in `width`, each with the x it starts at, and the
/// ones that did not, most important first.
///
/// Columns are dropped whole, least important first, the way the key bar
/// drops entries: a header cut mid-word reads as a column that was never
/// there, and a cell run into its neighbour reads as a number nobody measured.
fn fit(width: u16) -> (Vec<(Column, u16)>, Vec<Column>) {
    let mut kept: Vec<(Column, u16)> = LAYOUT.to_vec();
    for column in PRIORITY.iter().rev() {
        if kept.iter().map(|(_, w)| w).sum::<u16>() <= width {
            break;
        }
        kept.retain(|(c, _)| c != column);
    }
    let hidden = PRIORITY
        .iter()
        .copied()
        .filter(|c| !kept.iter().any(|(k, _)| k == c))
        .collect();
    let mut x = 0;
    let drawn = kept
        .into_iter()
        .map(|(column, w)| {
            let at = x;
            x += w;
            (column, at)
        })
        .collect();
    (drawn, hidden)
}

/// `a`, `a and b`, `a, b and c`.
fn listed(names: &[String]) -> String {
    match names {
        [] => String::new(),
        [one] => one.clone(),
        [init @ .., last] => format!("{} and {last}", init.join(", ")),
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
pub(super) fn truncate(name: &str, width: usize) -> String {
    if name.chars().count() <= width {
        return name.to_string();
    }
    let kept: String = name.chars().take(width.saturating_sub(1)).collect();
    format!("{kept}…")
}
