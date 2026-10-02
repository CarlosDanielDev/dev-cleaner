use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use super::candidates::Tally;
use super::palette::Theme;
use super::project_label::{annotation, fit_name, glyph, unique_suffixes};
use super::row::elide_path;
use super::{showing, window_start};
use crate::bytes::human;
use crate::classify::{Activity, Checkout, Kind};
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
    /// The git checkout this project sits in, if any.
    pub checkout: Checkout,
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
    /// The kind-of-checkout badge. Drawn, but ordered by [`Column::Repo`].
    Kind,
    /// Which repository a project belongs to. Not a column of its own: it
    /// orders the table so the worktrees of one repository sit together, the
    /// main checkout first, and the badge column carries it.
    Repo,
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
            Column::Kind => "kind",
            Column::Repo => "repo",
        }
    }

    /// Which way round this column is most useful first.
    ///
    /// Sizes and counts answer "what is worst", so they start at the largest.
    /// Names answer "where is X", so they start at A.
    fn starts_descending(&self) -> bool {
        !matches!(
            self,
            Column::Name | Column::Activity | Column::Repo | Column::Kind
        )
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
    /// What is marked in each project, once the screen that owns the marks has
    /// said. Until then the table has no mark column: it does not know.
    marks: Option<BTreeMap<PathBuf, Tally>>,
}

/// The mark column's width, gap included, and the narrowest area that has room
/// for it. Under that the column goes whole, as the others do.
const MARK_W: u16 = 3;
const MARKS_MIN_WIDTH: u16 = 80;

impl Projects {
    pub fn new(rows: Vec<ProjectSummary>) -> Self {
        let mut table = Self {
            labels: unique_suffixes(rows.iter().map(|r| r.path.as_path())),
            rows,
            sort: Column::Unique,
            descending: true,
            cursor: 0,
            marks: None,
        };
        table.apply_sort();
        table
    }

    pub fn rows(&self) -> &[ProjectSummary] {
        &self.rows
    }

    /// Say what is marked in each project, for the table to show.
    pub fn set_marks(&mut self, marks: BTreeMap<PathBuf, Tally>) {
        self.marks = Some(marks);
    }

    /// The project an entry at `path` belongs to: the innermost one it is under.
    pub fn owner_of(&self, path: &Path) -> Option<&Path> {
        self.rows
            .iter()
            .filter(|r| path.starts_with(&r.path))
            .max_by_key(|r| r.path.components().count())
            .map(|r| r.path.as_path())
    }

    /// The roots of the projects inside the one at `root`: their entries are
    /// theirs, not the outer project's.
    pub fn inner_roots(&self, root: &Path) -> Vec<PathBuf> {
        self.rows
            .iter()
            .filter(|r| r.path != root && r.path.starts_with(root))
            .map(|r| r.path.clone())
            .collect()
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
            (Column::Repo, false) => "each repository's worktrees together, main first",
            (Column::Repo, true) => "each repository's worktrees together, main last",
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
            // A repository's checkouts together: the main one first, then its
            // worktrees by path. A project with no repository is its own group.
            Column::Repo | Column::Kind => self.rows.sort_by(|a, b| {
                let key = |r: &ProjectSummary| {
                    let rank = match r.checkout.kind {
                        Kind::Main => 0,
                        Kind::Worktree | Kind::Orphan => 1,
                        Kind::Plain => 2,
                    };
                    (
                        r.checkout.repo.clone().unwrap_or_else(|| r.path.clone()),
                        rank,
                        r.path.clone(),
                    )
                };
                key(a).cmp(&key(b))
            }),
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

    /// What a row's checkout adds to its label: the branch of a worktree, `main`
    /// for the checkout the worktrees hang off, `orphan` for a dangling one.
    pub fn note(&self, row: &ProjectSummary) -> Option<String> {
        annotation(&row.checkout)
    }

    /// The label of the project that owns the entry at `path`, with the path
    /// inside it and the checkout's note: how the dashboard names a directory
    /// so it reads as the table names its project.
    pub fn name_inside(&self, path: &Path) -> Option<String> {
        let owner = self.owner_of(path)?;
        let row = self.rows.iter().find(|r| r.path == owner)?;
        let inside = path.strip_prefix(owner).ok()?.display().to_string();
        let mut name = self.label(row).to_string();
        if !inside.is_empty() {
            name = format!("{name}/{inside}");
        }
        if let Some(note) = self.note(row) {
            name = format!("{name}  {note}");
        }
        Some(name)
    }

    /// The row under the cursor in full, so that telling projects apart never
    /// depends on how wide a column was: what kind of checkout, of which
    /// repository, on which branch, then the whole path, cut from the left only
    /// when even that cannot fit.
    pub fn detail(&self, row: &ProjectSummary, width: usize) -> String {
        let c = &row.checkout;
        let repo = c
            .repo
            .as_deref()
            .and_then(Path::file_name)
            .map(|n| n.to_string_lossy().into_owned());
        let of = repo.map(|r| format!(" of {r}")).unwrap_or_default();
        let branch = c
            .branch
            .as_ref()
            .map(|b| format!(" · {b}"))
            .unwrap_or_default();
        let facts = match c.kind {
            Kind::Plain => "plain folder".to_string(),
            Kind::Main if c.linked > 0 => {
                format!(
                    "{} main checkout{of}, {} worktrees{branch}",
                    glyph(c.kind),
                    c.linked
                )
            }
            Kind::Main => format!("{} main checkout{of}{branch}", glyph(c.kind)),
            Kind::Worktree => format!(
                "{} worktree {}{of}{branch}",
                glyph(c.kind),
                c.worktree.as_deref().unwrap_or("?")
            ),
            Kind::Orphan => format!(
                "{} orphan worktree {}{of}: its repository no longer lists it",
                glyph(c.kind),
                c.worktree.as_deref().unwrap_or("?")
            ),
        };
        let path = row.path.display().to_string();
        let room = width.saturating_sub(facts.chars().count() + 3);
        format!("{facts} · {}", elide_path(&path, room.max(8)))
    }

    pub fn selected(&self) -> Option<&ProjectSummary> {
        self.rows.get(self.cursor)
    }

    /// Put the cursor on the project at `root`, without touching the order.
    /// `false`, and nothing moved, when the table has no such project.
    pub fn focus(&mut self, root: &Path) -> bool {
        let Some(at) = self.rows.iter().position(|r| r.path == root) else {
            return false;
        };
        self.cursor = at;
        true
    }

    /// Index of the row under the cursor.
    pub fn cursor(&self) -> usize {
        self.cursor
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
        // The mark column leads, and only where it fits whole.
        let marks = self
            .marks
            .as_ref()
            .filter(|_| area.width >= MARKS_MIN_WIDTH);
        let lead = if marks.is_some() { MARK_W } else { 0 };
        let left = area.x + 1;
        let (drawn, hidden, name_w) = fit(
            area.width.saturating_sub(1 + lead),
            marks.is_some(),
            self.has_repos(),
            self.widest_name(),
        );
        let at = left + lead;

        for (column, x) in &drawn {
            let style = if self.sorted_by(*column) {
                theme.head
            } else if *column == Column::Kind {
                // A shape, not a word: it is the badges' key, so not skippable.
                theme.violet
            } else {
                theme.muted
            };
            let text = match column {
                // One cell wide: the glyph says what the column is, and the
                // footer says what its shapes are.
                Column::Kind => "⎇".to_string(),
                _ => format!("{}{}", column.header(), self.marker(*column)),
            };
            buf.set_string(
                at + x,
                area.y,
                aligned(*column, &text, marks.is_some()),
                style,
            );
        }

        // A row of headers above the table, then the selected project in full
        // and the position below it.
        let height = area.height.saturating_sub(FRAME) as usize;
        let start = self.window_start(height);
        let visible = self.visible(height);
        for (i, row) in visible.iter().enumerate() {
            let y = area.y + 1 + i as u16;
            let tally = marks.map(|m| m.get(&row.path).copied().unwrap_or_default());
            if let Some(tally) = tally {
                let (glyph, style) = mark_glyph(theme, &tally);
                buf.set_string(left, y, glyph, style);
            }
            for (column, x) in &drawn {
                if *column == Column::Name {
                    let (label, note) = self.name_cell(row, name_w as usize - 2);
                    buf.set_string(at + x, y, &label, Self::ink(theme, row, *column));
                    let label_w = label.chars().count() as u16;
                    buf.set_string(at + x + label_w, y, note, theme.violet);
                    continue;
                }
                let cell = match (column, tally) {
                    (Column::Reclaimable, Some(t)) if t.marked.0 > 0 => marked_cell(&t),
                    _ => self.cell(row, *column),
                };
                buf.set_string(
                    at + x,
                    y,
                    aligned(*column, &cell, marks.is_some()),
                    Self::ink(theme, row, *column),
                );
            }
            // Across the whole row, gaps included: highlighted cell by cell it
            // reads as separate blocks rather than as one line under a cursor.
            if start + i == self.cursor {
                buf.set_style(Rect::new(area.x, y, area.width, 1), theme.selected);
            }
        }
        if area.height > FRAME
            && let Some(row) = self.selected()
        {
            let line = self.detail(row, area.width.saturating_sub(1) as usize);
            buf.set_string(left, area.bottom() - 2, line, theme.text);
        }
        if area.height >= 2 {
            let mut line = showing(start, visible.len(), self.rows.len());
            // The glyphs say it by shape, and the words say what the shapes are.
            if marks.is_some() {
                line = format!("{line} · {LEGEND}");
            }
            // A column that is not drawn is still there to sort by, so the
            // sorted one's header, marker and all, moves down here.
            let sorted_hidden: Vec<String> = hidden
                .iter()
                .map(|c| match c {
                    Column::Kind => format!("kind{}", self.marker(Column::Repo)),
                    _ => format!("{}{}", c.header(), self.marker(*c)),
                })
                .collect();
            if !sorted_hidden.is_empty() {
                line = format!("{line} · {} hidden at this width", listed(&sorted_hidden));
            }
            // Last, so a narrow line loses the legend before the order.
            if drawn.iter().any(|(c, _)| *c == Column::Kind) {
                line = format!("{line} · {KINDS}");
            }
            buf.set_string(
                left,
                area.bottom() - 1,
                truncate(&line, area.width.saturating_sub(1) as usize),
                theme.text,
            );
        }
    }

    /// Whether any project is in a repository, which is when the badge column
    /// and its legend have something to say.
    fn has_repos(&self) -> bool {
        self.rows.iter().any(|r| r.checkout.kind != Kind::Plain)
    }

    /// Whether the table is ordered by what `column` shows. The badge column is
    /// the repo order's face.
    fn sorted_by(&self, column: Column) -> bool {
        column == self.sort || (column == Column::Kind && self.sort == Column::Repo)
    }

    /// The label and its note, laid out for a name column `width` wide.
    fn name_cell(&self, row: &ProjectSummary, width: usize) -> (String, String) {
        fit_name(self.label(row), self.note(row).as_deref(), width)
    }

    /// How wide the name column would like to be: the longest label with its
    /// note and the gap after them.
    fn widest_name(&self) -> u16 {
        self.rows
            .iter()
            .map(|r| {
                let note = self.note(r).map_or(0, |n| 2 + n.chars().count());
                self.label(r).chars().count() + note
            })
            .max()
            .map_or(0, |n| (n + 2) as u16)
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
            Column::Kind | Column::Repo => match row.checkout.kind {
                Kind::Orphan => theme.blocked,
                _ => theme.violet,
            },
            Column::Unique => theme.size(row.bytes_unique),
            Column::Apparent => theme.size(row.bytes_apparent),
            Column::Inodes => theme.accent,
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
            Column::Name => self.label(row).to_string(),
            Column::Kind | Column::Repo => glyph(row.checkout.kind).to_string(),
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

/// What the mark glyphs mean, in words: the glyph is the state's first carrier
/// and the colour only the second.
const LEGEND: &str = "● all marked  ◐ some marked  · none marked";

/// What the badge column's shapes mean. Not the mark legend's `●`: that one is
/// a state of a project, and these are what it is.
const KINDS: &str = "◆ main checkout  ⎇ worktree  ⌀ orphan";

/// How many lines of the area are not rows: the headers above, the selected
/// project in full and the position below. The paging keys use it too, so a
/// page is exactly what is on screen.
pub const FRAME: u16 = 3;

/// The name column's width where it is not given more, gap included.
const NAME_MIN: u16 = 26;

/// Each column's width, gap included, in the order they are drawn. The
/// reclaimable one is wider where the mark column is, for `124 MB of 538 MB`.
fn layout(marks: bool) -> [(Column, u16); 7] {
    [
        (Column::Kind, 3),
        (Column::Name, NAME_MIN),
        (Column::Unique, 12),
        (Column::Apparent, 12),
        (Column::Inodes, 11),
        (Column::Reclaimable, if marks { 25 } else { 15 }),
        (Column::Activity, 10),
    ]
}

/// The glyph for a project's marks, with the ink it is drawn in. Nothing
/// offerable is blank: there is nothing there to be marked or not.
fn mark_glyph(theme: &Theme, tally: &Tally) -> (&'static str, Style) {
    match (tally.offered.0, tally.marked.0) {
        (0, _) => (" ", theme.text),
        (_, 0) => ("·", theme.text),
        (all, some) if some == all => ("●", theme.safe),
        _ => ("◐", theme.head),
    }
}

/// What is marked of what is offered: `124 MB of 538 MB`, or `x / x` when all.
fn marked_cell(tally: &Tally) -> String {
    let (marked, offered) = (human(tally.marked.1), human(tally.offered.1));
    if tally.marked.0 == tally.offered.0 {
        format!("{marked} / {offered}")
    } else {
        format!("{marked} of {offered}")
    }
}

/// `text` laid in its column: a figure ends where the one above it does, so
/// the digits line up and a longer number is visibly a bigger one; a name or a
/// word starts where the one above it does. Two gap columns follow each.
fn aligned(column: Column, text: &str, marks: bool) -> String {
    let width = layout(marks)
        .iter()
        .find(|(c, _)| *c == column)
        .map_or(0, |(_, w)| *w as usize - 2);
    match column {
        Column::Name | Column::Activity | Column::Kind | Column::Repo => text.to_string(),
        _ => format!("{text:>width$}"),
    }
}

/// The order columns are kept in when the area is narrower than all of them.
///
/// What deletion gives back and what could be rebuilt are the two figures a
/// decision rests on; the apparent size only says something on a hardlinked
/// project, so it is the first to go. The badge is the narrowest and what makes
/// worktrees tell apart, so it goes after the figures but before the name.
const PRIORITY: [Column; 7] = [
    Column::Name,
    Column::Kind,
    Column::Unique,
    Column::Reclaimable,
    Column::Activity,
    Column::Inodes,
    Column::Apparent,
];

/// The columns that fit in `width`, each with the x it starts at, the ones that
/// did not, most important first, and how wide the name column ended up.
///
/// Columns are dropped whole, least important first, the way the key bar
/// drops entries: a header cut mid-word reads as a column that was never
/// there, and a cell run into its neighbour reads as a number nobody measured.
/// What is left over goes to the name, up to `wanted`: it is the column whose
/// cut loses what tells two rows apart.
fn fit(
    width: u16,
    marks: bool,
    kinds: bool,
    wanted: u16,
) -> (Vec<(Column, u16)>, Vec<Column>, u16) {
    let mut kept: Vec<(Column, u16)> = layout(marks).to_vec();
    // Nothing is a repository: the badge would be an empty column of blanks.
    kept.retain(|(c, _)| kinds || *c != Column::Kind);
    for column in PRIORITY.iter().rev() {
        if kept.iter().map(|(_, w)| w).sum::<u16>() <= width {
            break;
        }
        kept.retain(|(c, _)| c != column);
    }
    let used: u16 = kept.iter().map(|(_, w)| w).sum();
    let name_w = NAME_MIN.max(wanted.min(NAME_MIN + width.saturating_sub(used)));
    let hidden = PRIORITY
        .iter()
        .copied()
        .filter(|c| (kinds || *c != Column::Kind) && !kept.iter().any(|(k, _)| k == c))
        .collect();
    let mut x = 0;
    let drawn = kept
        .into_iter()
        .map(|(column, w)| {
            let at = x;
            x += if column == Column::Name { name_w } else { w };
            (column, at)
        })
        .collect();
    (drawn, hidden, name_w)
}

/// `a`, `a and b`, `a, b and c`.
fn listed(names: &[String]) -> String {
    match names {
        [] => String::new(),
        [one] => one.clone(),
        [init @ .., last] => format!("{} and {last}", init.join(", ")),
    }
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
